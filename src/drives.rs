//! 論理ドライブ列挙とサブディレクトリ一覧取得。

use std::path::{Path, PathBuf};
use std::fs;
use windows::Win32::Storage::FileSystem::GetLogicalDriveStringsW;

/// Fetches all logical drives present on the Windows machine (e.g. "C:\", "D:\").
pub fn get_logical_drives() -> Vec<String> {
    let mut buffer = [0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) };
    if len == 0 {
        return vec!["C:\\".to_string()];
    }

    let mut drives = Vec::new();
    let mut current = &buffer[..len as usize];
    
    while !current.is_empty() {
        if let Some(null_idx) = current.iter().position(|&c| c == 0) {
            let drive_str = String::from_utf16(&current[..null_idx]).unwrap_or_default();
            if !drive_str.is_empty() {
                drives.push(drive_str);
            }
            current = &current[null_idx + 1..];
        } else {
            break;
        }
    }

    if drives.is_empty() {
        drives.push("C:\\".to_string());
    }
    
    drives
}

/// Lists immediate subdirectories for a given path, ignoring system and hidden directories.
pub fn list_subdirectories(path: &Path) -> Vec<(String, PathBuf)> {
    let mut dirs = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            // Check if it is a directory and not a symlink
            if p.is_dir() && !p.is_symlink() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    // Skip hidden/system directories
                    if name.starts_with('.') || name.starts_with('$') || name.eq_ignore_ascii_case("System Volume Information") {
                        continue;
                    }
                    dirs.push((name.to_string(), p));
                }
            }
        }
    }
    // Sort directories alphabetically (case insensitive)
    dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    dirs
}
