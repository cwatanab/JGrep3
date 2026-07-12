//! ディレクトリ列挙。
//!
//! Win32 FindFirstFileW / FindNextFileW による再帰的ファイル収集。
//! ファイルマスク・ディレクトリマスクによるフィルタリング。
//! `collect_files_parallel` はトップレベルディレクトリを並列列挙する。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::sync::Arc;

use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use super::gitignore::{load_ignore_files, GitIgnore, SearchFilter};

#[derive(Clone)]
pub struct WalkConfig {
    pub recursive: bool,
    pub filter: SearchFilter,
    pub apply_ignore_files: bool,
}

/// Collect matching file paths under root (single-threaded).
#[allow(dead_code)]
pub fn collect_files(root: &Path, cfg: &WalkConfig, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut ignores = Vec::new();
    if cfg.apply_ignore_files {
        ignores = load_ignore_files(root);
    }
    walk_dir(root, root, cfg, cancel, &ignores, &mut out);
    out
}

/// Collect matching file paths under root, sending to channel as found.
/// Top-level subdirectories are processed in parallel.
pub fn collect_files_parallel(
    root: &Path,
    cfg: WalkConfig,
    cancel: Arc<AtomicBool>,
    sender: Sender<PathBuf>,
) {
    let mut root_ignores = Vec::new();
    if cfg.apply_ignore_files {
        root_ignores = load_ignore_files(root);
    }

    let pattern = root.join("*");
    let wide = path_to_wide(&pattern);
    let mut data = WIN32_FIND_DATAW::default();

    let handle = match unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data) } {
        Ok(h) if h != INVALID_HANDLE_VALUE => h,
        _ => return,
    };

    let mut handles = Vec::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = wide_to_string(&data.cFileName);
        if name != "." && name != ".." {
            let is_dir = (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
            let path = root.join(&name);
            let rel_path = path.strip_prefix(root).unwrap_or(&path);

            if !is_ignored(&path, is_dir, &root_ignores) {
                if is_dir {
                    if cfg.recursive && cfg.filter.allow_dir(rel_path) {
                        let cfg = cfg.clone();
                        let cancel = Arc::clone(&cancel);
                        let s = sender.clone();
                        let root_buf = root.to_path_buf();
                        let root_ignores_clone = root_ignores.clone();
                        handles.push(thread::spawn(move || {
                            let mut out = Vec::new();
                            walk_dir(&root_buf, &path, &cfg, &cancel, &root_ignores_clone, &mut out);
                            for p in out {
                                if s.send(p).is_err() {
                                    break;
                                }
                              }
                        }));
                    }
                } else if cfg.filter.allow_file(rel_path) {
                    if sender.send(path).is_err() {
                        break;
                    }
                }
            }
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }

    for h in handles {
        let _ = h.join();
    }
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    cfg: &WalkConfig,
    cancel: &AtomicBool,
    parent_ignores: &[GitIgnore],
    out: &mut Vec<PathBuf>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let mut current_ignores = parent_ignores.to_vec();
    if cfg.apply_ignore_files {
        let local = load_ignore_files(dir);
        current_ignores.extend(local);
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
            let rel_path = path.strip_prefix(root).unwrap_or(&path);

            if !is_ignored(&path, is_dir, &current_ignores) {
                if is_dir {
                    if cfg.recursive && cfg.filter.allow_dir(rel_path) {
                        walk_dir(root, &path, cfg, cancel, &current_ignores, out);
                    }
                } else if cfg.filter.allow_file(rel_path) {
                    out.push(path);
                }
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

fn is_ignored(path: &Path, is_dir: bool, ignores: &[GitIgnore]) -> bool {
    ignores.iter().any(|gi| gi.is_ignored(path, is_dir))
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
            filter: SearchFilter::new("*.txt"),
            apply_ignore_files: false,
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
            filter: SearchFilter::new("!.git/"),
            apply_ignore_files: false,
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
            filter: SearchFilter::new(""),
            apply_ignore_files: false,
        };
        let cancel = AtomicBool::new(true);
        let files = collect_files(&root, &cfg, &cancel);
        assert!(files.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parallel_collects_recursive() {
        let root = std::env::temp_dir().join("jgrep_walk_parallel");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("root.txt"), "r");
        write_file(&root.join("src").join("main.rs"), "m");
        write_file(&root.join(".git").join("config"), "g");
        write_file(&root.join("lib").join("helper.rs"), "h");

        let cfg = WalkConfig {
            recursive: true,
            filter: SearchFilter::new("*.rs; !.git/"),
            apply_ignore_files: false,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let cfg_clone = cfg.clone();
        let cancel_clone = Arc::clone(&cancel);
        let root_clone = root.clone();
        let h = thread::spawn(move || {
            collect_files_parallel(&root_clone, cfg_clone, cancel_clone, tx);
        });
        let mut files: Vec<PathBuf> = rx.into_iter().collect();
        h.join().unwrap();
        files.sort();
        let mut paths: Vec<String> = files
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "lib/helper.rs".to_string(),
                "src/main.rs".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recursive_respects_ignore_files() {
        let root = std::env::temp_dir().join("jgrep_walk_ignores");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("root.txt"), "r");
        write_file(&root.join("src").join("main.rs"), "m");
        write_file(&root.join("src").join("nested").join("lib.rs"), "l");
        write_file(&root.join("target").join("debug").join("app.exe"), "e");
        write_file(&root.join(".ignore"), "target/\nsrc/nested/");

        let cfg = WalkConfig {
            recursive: true,
            filter: SearchFilter::new(""),
            apply_ignore_files: true,
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
                ".ignore".to_string(),
                "root.txt".to_string(),
                "src/main.rs".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
