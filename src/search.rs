use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use regex::{Regex, RegexBuilder};
use walkdir::WalkDir;
use encoding_rs;

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

// Convert a glob pattern (e.g. *.txt) to a Regex
fn glob_to_regex(pattern: &str) -> Result<Regex, regex::Error> {
    let mut regex_str = String::new();
    regex_str.push('^');
    for c in pattern.chars() {
        match c {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' | '|' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
    }
    regex_str.push('$');
    RegexBuilder::new(&regex_str)
        .case_insensitive(true) // File masks are usually case-insensitive on Windows
        .build()
}

// Check if a filename matches any of the semicolon/space separated file masks
fn matches_masks(filename: &str, masks: &[Regex]) -> bool {
    if masks.is_empty() {
        return true;
    }
    masks.iter().any(|mask| mask.is_match(filename))
}

// Helper to read file and decode into String with SJS/EUC-JP fallbacks
pub fn read_and_decode_file(path: &Path, auto_detect: bool) -> Result<String, std::io::Error> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    if auto_detect {
        // Try UTF-8 first
        if let Ok(s) = std::str::from_utf8(&buffer) {
            return Ok(s.to_string());
        }
        // Check for UTF-8 BOM
        if buffer.starts_with(&[0xEF, 0xBB, 0xBF]) {
            if let Ok(s) = std::str::from_utf8(&buffer[3..]) {
                return Ok(s.to_string());
            }
        }
        // Fallback to Shift_JIS (Japanese Windows default)
        let (res, _encoding, has_errors) = encoding_rs::SHIFT_JIS.decode(&buffer);
        if !has_errors {
            return Ok(res.into_owned());
        }
        // Fallback to EUC-JP
        let (res, _encoding, has_errors) = encoding_rs::EUC_JP.decode(&buffer);
        if !has_errors {
            return Ok(res.into_owned());
        }
        // Last resort: UTF-8 lossy
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    } else {
        // Default decoding: UTF-8 lossy
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }
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

    // Prepare search query regex or simple matcher
    let query_regex = if is_regex {
        match RegexBuilder::new(&search_query)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(re) => Some(re),
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(format!("Invalid regex: {}", e)));
                notice_sender.notice();
                return;
            }
        }
    } else {
        None
    };

    let query_literal = if !is_regex {
        if case_sensitive {
            Some(search_query.clone())
        } else {
            Some(search_query.to_lowercase())
        }
    } else {
        None
    };

    // Parse file masks (separated by semicolon, comma or space)
    let file_masks: Vec<Regex> = file_mask_str
        .split(|c| c == ';' || c == ',' || c == ' ')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| glob_to_regex(s).ok())
        .collect();

    // Parse directory masks:
    //   include:  src;lib   (empty / ** = all directories)
    //   exclude:  !.git;!node_modules;!target
    let mut dir_includes: Vec<Regex> = Vec::new();
    let mut dir_excludes: Vec<Regex> = Vec::new();
    for part in dir_mask_str.split(|c| c == ';' || c == ',' || c == ' ') {
        let s = part.trim();
        if s.is_empty() || s == "**" {
            continue;
        }
        if let Some(ex) = s.strip_prefix('!') {
            let ex = ex.trim();
            if !ex.is_empty() {
                if let Ok(re) = glob_to_regex(ex) {
                    dir_excludes.push(re);
                }
            }
        } else if let Ok(re) = glob_to_regex(s) {
            dir_includes.push(re);
        }
    }

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
            // Exclude wins (e.g. !.git, !node_modules)
            if !dir_excludes.is_empty() && matches_masks(name, &dir_excludes) {
                return false;
            }
            // Include filter (if any): only enter matching dirs
            if !dir_includes.is_empty() && !matches_masks(name, &dir_includes) {
                return false;
            }
            true
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
            if !matches_masks(file_name, &file_masks) {
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
        match read_and_decode_file(path, auto_detect_encoding) {
            Ok(content) => {
                let mut line_num = 1;
                for line in content.lines() {
                    let is_match = if let Some(ref re) = query_regex {
                        re.is_match(line)
                    } else if let Some(ref lit) = query_literal {
                        if case_sensitive {
                            line.contains(lit)
                        } else {
                            line.to_lowercase().contains(lit)
                        }
                    } else {
                        false
                    };

                    if is_match {
                        match_count += 1;

                        // 1-based character column of the first match
                        let column_number = if let Some(ref re) = query_regex {
                            re.find(line)
                                .map(|m| line[..m.start()].chars().count() + 1)
                                .unwrap_or(1)
                        } else if let Some(ref lit) = query_literal {
                            let escaped = regex::escape(lit);
                            RegexBuilder::new(&escaped)
                                .case_insensitive(!case_sensitive)
                                .build()
                                .ok()
                                .and_then(|re| re.find(line))
                                .map(|m| line[..m.start()].chars().count() + 1)
                                .unwrap_or(1)
                        } else {
                            1
                        };
                        
                        // Plain line text — UI custom-draws match highlights
                        // If the line is very long, extract context around the match to make it visible in UI.
                        let line_trimmed = line.trim();
                        let max_len = 120;
                        let line_content = if line_trimmed.chars().count() > max_len {
                            let query_lower = search_query.to_lowercase();
                            let trimmed_lower = line_trimmed.to_lowercase();
                            let match_char_idx = if is_regex {
                                if let Some(ref re) = query_regex {
                                    re.find(&line_trimmed)
                                        .map(|m| line_trimmed[..m.start()].chars().count())
                                        .unwrap_or(0)
                                } else {
                                    0
                                }
                            } else {
                                trimmed_lower.find(&query_lower)
                                    .map(|bytes_idx| line_trimmed[..bytes_idx].chars().count())
                                    .unwrap_or(0)
                            };

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
            Err(_) => {
                // Silently skip files that fail to read (locked files, system files, etc.)
            }
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
