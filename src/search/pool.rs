use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc::Sender, Arc};
use std::thread;
use std::time::Instant;

use native_windows_gui::NoticeSender;

fn notify(notice: &Option<NoticeSender>) {
    if let Some(n) = notice {
        n.notice();
    }
}

use super::decode::{looks_binary, read_and_decode_file};
use super::glob_mask::{parse_dir_masks, parse_file_masks};
use super::match_engine::Matcher;
use super::walk::{collect_files, WalkConfig};
use super::{SearchResultItem, SearchStatus};

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_subsequence_case_insensitive(haystack: &[u8], needle_lower: &[u8]) -> Option<usize> {
    let n = needle_lower.len();
    haystack.windows(n).position(|w| {
        w.iter()
            .zip(needle_lower.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

fn search_bytes(
    buf: &[u8],
    matcher: &Matcher,
    path: &Path,
    _auto_detect: bool,
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

    let needle_lower_bytes = needle_lower.as_ref().map(|s| s.as_bytes());

    let mut line_idx: usize = 0;
    let mut pos = 0;
    while pos < buf.len() {
        let line_start = pos;
        let mut line_end = buf.len();
        let mut found_newline = false;

        for i in pos..buf.len() {
            if buf[i] == b'\n' {
                line_end = i;
                pos = i + 1;
                found_newline = true;
                break;
            }
        }
        if !found_newline {
            pos = buf.len();
        }

        let content_end = if line_end > 0 && buf[line_end.saturating_sub(1)] == b'\r' {
            line_end.saturating_sub(1)
        } else {
            line_end
        };

        line_idx += 1;

        let content_bytes = &buf[line_start..content_end];

        let hit_pos = if *case_sensitive {
            find_subsequence(content_bytes, needle_bytes)
        } else {
            find_subsequence_case_insensitive(content_bytes, needle_lower_bytes.unwrap_or(needle_bytes))
        };

        if let Some(column) = hit_pos {
            let lossy = String::from_utf8_lossy(content_bytes);
            let line_content = truncate_line(&lossy, matcher);
            batch.push(SearchResultItem {
                file_path: path.to_string_lossy().into_owned(),
                line_number: line_idx,
                column_number: column + 1,
                line_content,
            });
            match_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    true
}

const MATCH_BATCH: usize = 32;
const PROGRESS_EVERY: usize = 50;

pub fn run(
    search_dir: PathBuf,
    search_query: String,
    file_mask_str: String,
    dir_mask_str: String,
    recursive: bool,
    case_sensitive: bool,
    is_regex: bool,
    auto_detect_encoding: bool,
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

    let file_masks = match parse_file_masks(&file_mask_str) {
        Ok(m) => m,
        Err(e) => {
            let _ = sender.send(SearchStatus::Error(format!("Invalid file mask: {}", e)));
            notify(&notice_sender);
            return;
        }
    };
    let dir_masks = match parse_dir_masks(&dir_mask_str) {
        Ok(m) => m,
        Err(e) => {
            let _ = sender.send(SearchStatus::Error(format!("Invalid dir mask: {}", e)));
            notify(&notice_sender);
            return;
        }
    };

    let walk_cfg = WalkConfig {
        recursive,
        file_masks,
        dir_masks,
    };

    let files = collect_files(&search_dir, &walk_cfg, &cancellation_token);
    if cancellation_token.load(Ordering::Relaxed) {
        finish(&sender, &notice_sender, start, 0, 0);
        return;
    }

    let workers = thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .max(1);

    let files = Arc::new(files);
    let next_index = Arc::new(AtomicUsize::new(0));
    let match_count = Arc::new(AtomicUsize::new(0));
    let scanned = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let files = Arc::clone(&files);
        let next_index = Arc::clone(&next_index);
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
                let i = next_index.fetch_add(1, Ordering::Relaxed);
                if i >= files.len() {
                    break;
                }
                let path = &files[i];
                let n = scanned.fetch_add(1, Ordering::Relaxed) + 1;
                if n % PROGRESS_EVERY == 0 {
                    let _ = sender.send(SearchStatus::Progress { scanned_files: n });
                    notify(&notice_sender);
                }

                buf.clear();
                let mut file = match File::open(path) {
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
                    path,
                    auto_detect_encoding,
                    &mut batch,
                    &match_count,
                ) {
                    if batch.len() >= MATCH_BATCH {
                        flush_batch(&sender, &notice_sender, &mut batch);
                    }
                    continue;
                }

                let content = match read_and_decode_file(path, auto_detect_encoding) {
                    Ok(Some(s)) => s,
                    _ => continue,
                };

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
                        file_path: path.to_string_lossy().into_owned(),
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
