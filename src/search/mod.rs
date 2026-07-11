pub mod glob_mask;
mod decode;
mod match_engine;
mod pool;
mod walk;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use native_windows_gui::NoticeSender;

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
    Completed {
        elapsed_ms: u64,
        total_scanned: usize,
        match_count: usize,
    },
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
    sender: Sender<SearchStatus>,
    notice_sender: NoticeSender,
) {
    pool::run(
        search_dir,
        search_query,
        file_mask_str,
        dir_mask_str,
        recursive,
        case_sensitive,
        is_regex,
        auto_detect_encoding,
        cancellation_token,
        sender,
        notice_sender,
    );
}
