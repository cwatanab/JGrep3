use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use super::glob_mask::{matches_file_masks, DirMaskSet, FileMasks};

pub struct WalkConfig {
    pub recursive: bool,
    pub file_masks: FileMasks,
    pub dir_masks: DirMaskSet,
}

/// Collect matching file paths under root (single-threaded).
pub fn collect_files(root: &Path, cfg: &WalkConfig, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_dir(root, cfg, cancel, &mut out);
    out
}

fn walk_dir(dir: &Path, cfg: &WalkConfig, cancel: &AtomicBool, out: &mut Vec<PathBuf>) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let pattern = dir.join("*");
    let wide = path_to_wide(&pattern);
    let mut data = WIN32_FIND_DATAW::default();

    let handle = match unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data) } {
        Ok(h) if h != INVALID_HANDLE_VALUE => h,
        _ => return,
    };

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = wide_to_string(&data.cFileName);
        if name != "." && name != ".." {
            let is_dir = (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
            let path = dir.join(&name);
            if is_dir {
                // non-recursive: never enter subdirectories (same as WalkDir max_depth(1))
                if cfg.recursive && cfg.dir_masks.allow_dir(&name) {
                    walk_dir(&path, cfg, cancel, &mut *out);
                }
            } else if matches_file_masks(&name, &cfg.file_masks) {
                out.push(path);
            }
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::glob_mask::{parse_dir_masks, parse_file_masks};
    use std::fs;
    use std::sync::atomic::AtomicBool;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        let mut v: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn collects_root_files_non_recursive() {
        let root = std::env::temp_dir().join("jgrep_walk_nonrec");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("a.txt"), "a");
        write_file(&root.join("b.rs"), "b");
        write_file(&root.join("sub").join("c.txt"), "c");

        let cfg = WalkConfig {
            recursive: false,
            file_masks: parse_file_masks("*.txt").unwrap(),
            dir_masks: parse_dir_masks("").unwrap(),
        };
        let cancel = AtomicBool::new(false);
        let files = collect_files(&root, &cfg, &cancel);
        assert_eq!(names(&files), vec!["a.txt".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recursive_respects_dir_masks() {
        let root = std::env::temp_dir().join("jgrep_walk_rec");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("root.txt"), "r");
        write_file(&root.join("src").join("main.rs"), "m");
        write_file(&root.join(".git").join("config"), "g");
        write_file(&root.join("src").join("nested").join("lib.rs"), "l");

        let cfg = WalkConfig {
            recursive: true,
            file_masks: parse_file_masks("").unwrap(),
            dir_masks: parse_dir_masks("**;!.git").unwrap(),
        };
        let cancel = AtomicBool::new(false);
        let files = collect_files(&root, &cfg, &cancel);
        let mut paths: Vec<String> = files
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "root.txt".to_string(),
                "src/main.rs".to_string(),
                "src/nested/lib.rs".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cancel_stops_early() {
        let root = std::env::temp_dir().join("jgrep_walk_cancel");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("a.txt"), "a");

        let cfg = WalkConfig {
            recursive: true,
            file_masks: parse_file_masks("").unwrap(),
            dir_masks: parse_dir_masks("").unwrap(),
        };
        let cancel = AtomicBool::new(true);
        let files = collect_files(&root, &cfg, &cancel);
        assert!(files.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
