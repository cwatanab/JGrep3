//! 並列ワーカプール。
//!
//! ファイルリストを論理CPU数（最大8）のワーカで並列処理。
//! バッチ通知（32件ごと）と進捗更新（50ファイルごと）でUI負荷を抑制。
//! memchr SIMDによる高速リテラル検索とバッファ再利用を含む。

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc::Sender, Arc, Mutex};
use std::thread;
use std::time::Instant;

use memchr::memmem;

use native_windows_gui::NoticeSender;

fn notify(notice: &Option<NoticeSender>) {
    if let Some(n) = notice {
        n.notice();
    }
}

use super::decode::{looks_binary, read_and_decode_file};
use super::gitignore::SearchFilter;
use super::match_engine::Matcher;
use super::walk::{collect_files_parallel, WalkConfig};
use super::{SearchResultItem, SearchStatus};

fn search_bytes(
    buf: &[u8],
    matcher: &Matcher,
    path: &Path,
    batch: &mut Vec<SearchResultItem>,
    match_count: &AtomicUsize,
) -> bool {
    let (needle, case_sensitive, needle_lower) = match matcher {
        Matcher::Literal {
            needle,
            case_sensitive,
            needle_lower,
        } => (needle, case_sensitive, needle_lower),
        _ => return false,
    };

    let needle_bytes = needle.as_bytes();
    if needle_bytes.iter().any(|&b| b >= 128) {
        return false;
    }

    let file_path: Arc<str> = Arc::from(path.to_string_lossy().as_ref());

    if *case_sensitive {
        let finder = memmem::Finder::new(needle_bytes);
        let mut matches = finder.find_iter(buf).peekable();
        if matches.peek().is_none() {
            return true;
        }

        let mut line_idx: usize = 0;
        let mut pos = 0;
        loop {
            let line_start = pos;
            let mut line_end = buf.len();

            if let Some(offset) = memchr::memchr(b'\n', &buf[pos..]) {
                line_end = pos + offset;
                pos = line_end + 1;
            } else {
                pos = buf.len();
            }

            let content_end = if line_end > 0 && buf[line_end.saturating_sub(1)] == b'\r' {
                line_end.saturating_sub(1)
            } else {
                line_end
            };

            line_idx += 1;

            if let Some(first_match) = matches.peek()
                && *first_match < line_end {
                    let content_bytes = &buf[line_start..content_end];
                    let column = finder.find(content_bytes).map(|c| c + 1).unwrap_or(1);
                    let lossy = String::from_utf8_lossy(content_bytes);
                    let line_content = truncate_line(&lossy, matcher);
                    batch.push(SearchResultItem {
                        file_path: Arc::clone(&file_path),
                        line_number: line_idx,
                        column_number: column,
                        line_content,
                    });
                    match_count.fetch_add(1, Ordering::Relaxed);
                    while matches.next_if(|&m| m < line_end).is_some() {}
                }

            if pos >= buf.len() {
                break;
            }
        }
    } else {
        let needle_lower_bytes = needle_lower.as_ref().map(|s| s.as_bytes());
        let nb = needle_lower_bytes.unwrap_or(needle_bytes);
        let n = nb.len();

        let first_byte = nb[0];
        let first_upper = first_byte.to_ascii_uppercase();
        let use_memchr2 = first_byte != first_upper;

        let finder_lowercase = |haystack: &[u8]| -> Option<usize> {
            if haystack.len() < n {
                return None;
            }
            let mut search_pos = 0;
            let limit = haystack.len() - n;
            while search_pos <= limit {
                let found = if use_memchr2 {
                    memchr::memchr2(first_byte, first_upper, &haystack[search_pos..=limit])
                } else {
                    memchr::memchr(first_byte, &haystack[search_pos..=limit])
                };
                
                if let Some(offset) = found {
                    let hit_idx = search_pos + offset;
                    let mut ok = true;
                    for j in 1..n {
                        if haystack[hit_idx + j].to_ascii_lowercase() != nb[j] {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        return Some(hit_idx);
                    }
                    search_pos = hit_idx + 1;
                } else {
                    break;
                }
            }
            None
        };

        let mut line_idx: usize = 0;
        let mut pos = 0;
        while pos < buf.len() {
            let line_start = pos;
            let mut line_end = buf.len();

            if let Some(offset) = memchr::memchr(b'\n', &buf[pos..]) {
                line_end = pos + offset;
                pos = line_end + 1;
            } else {
                pos = buf.len();
            }

            let content_end = if line_end > 0 && buf[line_end.saturating_sub(1)] == b'\r' {
                line_end.saturating_sub(1)
            } else {
                line_end
            };

            line_idx += 1;

            let content_bytes = &buf[line_start..content_end];

            let hit_pos = finder_lowercase(content_bytes);

            if let Some(column) = hit_pos {
                let lossy = String::from_utf8_lossy(content_bytes);
                let line_content = truncate_line(&lossy, matcher);
                batch.push(SearchResultItem {
                    file_path: Arc::clone(&file_path),
                    line_number: line_idx,
                    column_number: column + 1,
                    line_content,
                });
                match_count.fetch_add(1, Ordering::Relaxed);
            }

            if pos >= buf.len() {
                break;
            }
        }
    }

    true
}

const MATCH_BATCH: usize = 32;
const PROGRESS_EVERY: usize = 50;

pub fn run(
    search_dir: PathBuf,
    search_query: String,
    mask_str: String,
    recursive: bool,
    case_sensitive: bool,
    is_regex: bool,
    auto_detect_encoding: bool,
    apply_ignore_files: bool,
    cancellation_token: Arc<AtomicBool>,
    sender: Sender<SearchStatus>,
    notice_sender: Option<NoticeSender>,
) {
    let start = Instant::now();

    let matcher = if is_regex {
        match Matcher::regex(&search_query, case_sensitive) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(format!("Invalid regex: {}", e)));
                notify(&notice_sender);
                return;
            }
        }
    } else {
        match Matcher::literal(&search_query, case_sensitive) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(format!("Invalid query: {}", e)));
                notify(&notice_sender);
                return;
            }
        }
    };

    let filter = SearchFilter::new(&mask_str);

    let walk_cfg = WalkConfig {
        recursive,
        filter,
        apply_ignore_files,
    };

    let (path_tx, path_rx) = std::sync::mpsc::channel::<PathBuf>();
    let path_rx = Arc::new(Mutex::new(path_rx));
    let walk_cfg_clone = walk_cfg;
    let cancel_clone = Arc::clone(&cancellation_token);

    thread::spawn(move || {
        collect_files_parallel(&search_dir, walk_cfg_clone, cancel_clone, path_tx);
    });

    if cancellation_token.load(Ordering::Relaxed) {
        finish(&sender, &notice_sender, start, 0, 0);
        return;
    }

    let workers = thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .max(1);

    let match_count = Arc::new(AtomicUsize::new(0));
    let scanned = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let path_rx = Arc::clone(&path_rx);
        let matcher = Arc::clone(&matcher);
        let cancel = Arc::clone(&cancellation_token);
        let sender = sender.clone();
        let notice_sender = notice_sender.clone();
        let match_count = Arc::clone(&match_count);
        let scanned = Arc::clone(&scanned);

        handles.push(thread::spawn(move || {
            let mut batch: Vec<SearchResultItem> = Vec::with_capacity(MATCH_BATCH);
            let mut buf: Vec<u8> = Vec::new();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let path = {
                    let rx = path_rx.lock().unwrap();
                    match rx.recv() {
                        Ok(p) => p,
                        Err(_) => break,
                    }
                };
                let n = scanned.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(PROGRESS_EVERY) {
                    let _ = sender.send(SearchStatus::Progress { scanned_files: n });
                    notify(&notice_sender);
                }

                buf.clear();
                let mut file = match File::open(&path) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                if file.read_to_end(&mut buf).is_err() {
                    continue;
                }
                if looks_binary(&buf) {
                    continue;
                }

                if search_bytes(
                    &buf,
                    &matcher,
                    &path,
                    &mut batch,
                    &match_count,
                ) {
                    if batch.len() >= MATCH_BATCH {
                        flush_batch(&sender, &notice_sender, &mut batch);
                    }
                    continue;
                }

                let content = match read_and_decode_file(&path, auto_detect_encoding) {
                    Ok(Some(s)) => s,
                    _ => continue,
                };

                let file_path: Arc<str> = Arc::from(path.to_string_lossy().as_ref());

                for (line_idx, line) in content.lines().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(hit) = matcher.find_in_line(line) else {
                        continue;
                    };
                    let line_num = line_idx + 1;
                    let line_content = truncate_line(line, matcher.as_ref());
                    batch.push(SearchResultItem {
                        file_path: Arc::clone(&file_path),
                        line_number: line_num,
                        column_number: hit.column_chars,
                        line_content,
                    });
                    match_count.fetch_add(1, Ordering::Relaxed);
                    if batch.len() >= MATCH_BATCH {
                        flush_batch(&sender, &notice_sender, &mut batch);
                    }
                }
            }
            flush_batch(&sender, &notice_sender, &mut batch);
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    finish(
        &sender,
        &notice_sender,
        start,
        scanned.load(Ordering::Relaxed),
        match_count.load(Ordering::Relaxed),
    );
}

fn flush_batch(
    sender: &Sender<SearchStatus>,
    notice: &Option<NoticeSender>,
    batch: &mut Vec<SearchResultItem>,
) {
    if batch.is_empty() {
        return;
    }
    let items = std::mem::take(batch);
    let _ = sender.send(SearchStatus::Matches(items));
    notify(notice);
}

fn finish(
    sender: &Sender<SearchStatus>,
    notice: &Option<NoticeSender>,
    start: Instant,
    total_scanned: usize,
    match_count: usize,
) {
    let _ = sender.send(SearchStatus::Completed {
        elapsed_ms: start.elapsed().as_millis() as u64,
        total_scanned,
        match_count,
    });
    notify(notice);
}

fn truncate_line(line: &str, matcher: &Matcher) -> String {
    let line_trimmed = line.trim();
    let max_len = 120;
    if line_trimmed.chars().count() <= max_len {
        return line_trimmed.to_string();
    }
    let match_char_idx = matcher
        .find_in_line(line_trimmed)
        .map(|h| h.column_chars.saturating_sub(1))
        .unwrap_or(0);
    let chars: Vec<char> = line_trimmed.chars().collect();
    let total = chars.len();
    let start = match_char_idx.saturating_sub(40);
    let end = (match_char_idx + 60).min(total);
    let mut out = String::new();
    if start > 0 {
        out.push_str("...");
    }
    out.extend(chars[start..end].iter().copied());
    if end < total {
        out.push_str("...");
    }
    out
}
