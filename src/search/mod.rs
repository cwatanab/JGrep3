pub mod glob_mask;
mod decode;
mod match_engine;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use walkdir::WalkDir;

use decode::read_and_decode_file;
use glob_mask::{matches_file_masks, parse_dir_masks, parse_file_masks};
use match_engine::Matcher;

#[derive(Debug, Clone)]
pub struct SearchResultItem {
    pub file_path: String,
    pub line_number: usize,
    pub column_number: usize,
    pub line_content: String,
}

#[derive(Debug, Clone)]
pub enum SearchStatus {
    Match(SearchResultItem),
    Progress { scanned_files: usize },
    Completed { elapsed_ms: u64, total_scanned: usize, match_count: usize },
    Error(String),
}

pub fn run_search(
    search_dir: PathBuf,
    search_query: String,
    file_mask_str: String,
    dir_mask_str: String,
    recursive: bool,
    case_sensitive: bool,
    is_regex: bool,
    auto_detect_encoding: bool,
    cancellation_token: Arc<AtomicBool>,
    sender: crossbeam_channel::Sender<SearchStatus>,
    notice_sender: native_windows_gui::NoticeSender,
) {
    let start_time = Instant::now();

    let matcher = if is_regex {
        match Matcher::regex(&search_query, case_sensitive) {
            Ok(m) => m,
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(format!("Invalid regex: {}", e)));
                notice_sender.notice();
                return;
            }
        }
    } else {
        match Matcher::literal(&search_query, case_sensitive) {
            Ok(m) => m,
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(format!("Invalid query: {}", e)));
                notice_sender.notice();
                return;
            }
        }
    };

    let file_masks = match parse_file_masks(&file_mask_str) {
        Ok(m) => m,
        Err(e) => {
            let _ = sender.send(SearchStatus::Error(format!("Invalid file mask: {}", e)));
            notice_sender.notice();
            return;
        }
    };

    let dir_masks = match parse_dir_masks(&dir_mask_str) {
        Ok(m) => m,
        Err(e) => {
            let _ = sender.send(SearchStatus::Error(format!("Invalid dir mask: {}", e)));
            notice_sender.notice();
            return;
        }
    };

    let mut scanned_files = 0;
    let mut match_count = 0;

    let walk_depth = if recursive { usize::MAX } else { 1 };
    let walker = WalkDir::new(&search_dir)
        .max_depth(walk_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Always enter the search root
            if e.depth() == 0 {
                return true;
            }
            if !e.file_type().is_dir() {
                return true;
            }
            let name = match e.file_name().to_str() {
                Some(n) => n,
                None => return false,
            };
            dir_masks.allow_dir(name)
        });

    for entry in walker.filter_map(|e| e.ok()) {
        // Check cancellation
        if cancellation_token.load(Ordering::Relaxed) {
            break;
        }

        let path = entry.path();
        if path.is_dir() {
            continue;
        }

        // Apply file mask check
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if !matches_file_masks(file_name, &file_masks) {
                continue;
            }
        } else {
            continue;
        }

        scanned_files += 1;

        // Periodic progress update (every 50 files scanned)
        if scanned_files % 50 == 0 {
            let _ = sender.send(SearchStatus::Progress { scanned_files });
            notice_sender.notice();
        }

        // Read and search inside file
        let content = match read_and_decode_file(path, auto_detect_encoding) {
            Ok(Some(c)) => c,
            Ok(None) | Err(_) => continue,
        };

        let mut line_num = 1;
        for line in content.lines() {
            if let Some(hit) = matcher.find_in_line(line) {
                match_count += 1;
                let column_number = hit.column_chars;

                // Plain line text — UI custom-draws match highlights
                // If the line is very long, extract context around the match to make it visible in UI.
                let line_trimmed = line.trim();
                let max_len = 120;
                let line_content = if line_trimmed.chars().count() > max_len {
                    let match_char_idx = matcher
                        .find_in_line(line_trimmed)
                        .map(|h| h.column_chars.saturating_sub(1))
                        .unwrap_or(0);

                    let chars: Vec<char> = line_trimmed.chars().collect();
                    let total_chars = chars.len();
                    let context_before = 40;
                    let context_after = 60;

                    let start = if match_char_idx > context_before {
                        match_char_idx - context_before
                    } else {
                        0
                    };
                    let end = (match_char_idx + context_after).min(total_chars);

                    let mut truncated = String::new();
                    if start > 0 {
                        truncated.push_str("...");
                    }
                    truncated.push_str(&chars[start..end].iter().collect::<String>());
                    if end < total_chars {
                        truncated.push_str("...");
                    }
                    truncated
                } else {
                    line_trimmed.to_string()
                };

                let item = SearchResultItem {
                    file_path: path.to_string_lossy().to_string(),
                    line_number: line_num,
                    column_number,
                    line_content,
                };
                let _ = sender.send(SearchStatus::Match(item));
                notice_sender.notice();
            }
            line_num += 1;
        }
    }

    let elapsed = start_time.elapsed().as_millis() as u64;
    let _ = sender.send(SearchStatus::Completed {
        elapsed_ms: elapsed,
        total_scanned: scanned_files,
        match_count,
    });
    notice_sender.notice();
}
