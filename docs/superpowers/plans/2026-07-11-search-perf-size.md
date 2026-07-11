# Search Performance & Binary Size Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Shrink release binary to ≤1.5MB (stretch ≤1MB) and speed full-text search ≥10× while keeping nwg UI and all current features.

**Architecture:** Keep `main.rs` UI mostly unchanged. Rewrite search into a modular pipeline (walk → worker pool → match/decode) with batched UI notifications. Reduce binary size via release profile (LTO/strip/opt-z) and dependency slimming (`walkdir`/`crossbeam` out, unused nwg features off).

**Tech Stack:** Rust 2024, `native-windows-gui`, `windows` 0.58, `regex`, `encoding_rs`, `serde`/`toml`, std threads + `std::sync::mpsc`

**Spec:** `docs/superpowers/specs/2026-07-11-search-perf-size-design.md`

---

## File map

| Path | Responsibility |
|------|----------------|
| `Cargo.toml` | release profile, deps, features |
| `src/search.rs` | public API: types + `run_search` |
| `src/search/glob_mask.rs` | glob → matcher, file/dir mask rules |
| `src/search/decode.rs` | read file, binary detect, encoding |
| `src/search/match_engine.rs` | literal / regex match + column |
| `src/search/walk.rs` | recursive directory enumeration |
| `src/search/pool.rs` | worker pool, batching, progress |
| `src/main.rs` | channel type + `handle_notice` borrow fix only |
| `src/bin/bench_search.rs` | CLI baseline/after benchmark (optional binary) |
| `tests/search_unit.rs` | unit tests for mask/match/decode |

---

### Task 1: Baseline metrics + release profile

**Files:**
- Modify: `Cargo.toml`
- Create: `docs/superpowers/plans/baseline-metrics.md` (local notes; can overwrite)

- [ ] **Step 1: Record current binary size**

```bash
cargo build --release
# PowerShell or ls:
ls -la target/release/JGrep3.exe
```

Expected: ~3_050_496 bytes (record exact).

- [ ] **Step 2: Add release profile to `Cargo.toml`**

Append:

```toml
[profile.release]
lto = true
codegen-units = 1
panic = "abort"
strip = true
opt-level = "z"
```

- [ ] **Step 3: Rebuild and record size delta**

```bash
cargo build --release
ls -la target/release/JGrep3.exe
```

Write both sizes into a short note at top of working memory / commit message.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "build: optimize release profile for size (LTO, strip, opt-z)"
```

---

### Task 2: Unit test harness for search helpers

**Files:**
- Create: `tests/search_unit.rs`
- Modify: `src/search.rs` (make helpers `pub(crate)` or `pub` as needed later)

- [ ] **Step 1: Add failing tests for glob and literal match (API we will implement)**

Create `tests/search_unit.rs`:

```rust
use JGrep3::search::glob_mask::{parse_file_masks, matches_file_masks, parse_dir_masks, DirMaskSet};
use JGrep3::search::match_engine::{Matcher, MatchHit};

#[test]
fn file_mask_glob_star() {
    let masks = parse_file_masks("*.rs;*.toml").unwrap();
    assert!(matches_file_masks("main.rs", &masks));
    assert!(matches_file_masks("Cargo.toml", &masks));
    assert!(!matches_file_masks("readme.md", &masks));
}

#[test]
fn dir_mask_exclude_git() {
    let set = parse_dir_masks("**;!.git;!node_modules").unwrap();
    assert!(!set.allow_dir(".git"));
    assert!(!set.allow_dir("node_modules"));
    assert!(set.allow_dir("src"));
}

#[test]
fn literal_case_insensitive_finds_column() {
    let m = Matcher::literal("Foo", false).unwrap();
    let hit = m.find_in_line("hello foo bar").expect("hit");
    assert_eq!(hit.column_chars, 7); // 1-based char column of 'f'
}

#[test]
fn literal_case_sensitive_no_match() {
    let m = Matcher::literal("Foo", true).unwrap();
    assert!(m.find_in_line("hello foo bar").is_none());
}
```

Note: crate name in Cargo.toml is `JGrep3` — library export may be missing. If package is binary-only, either:

1. Add `[lib]` with `path = "src/lib.rs"` that re-exports modules, **or**
2. Put unit tests in `src/search/*.rs` with `#[cfg(test)]`.

**Prefer approach 2** to avoid large package restructure:

Create tests inside modules instead (see Task 3). If using approach 2, delete `tests/search_unit.rs` and put equivalent `#[cfg(test)] mod tests` in each module.

- [ ] **Step 2: Choose approach 2 (in-module tests) and skip external tests crate for now**

Document in commit: unit tests live next to modules.

- [ ] **Step 3: Commit scaffolding only if files added**

```bash
git add -A
git commit -m "test: prepare search unit test locations" || true
```

---

### Task 3: `glob_mask` module (TDD)

**Files:**
- Create: `src/search/glob_mask.rs`
- Modify: `src/search.rs` (module tree + re-exports if needed)

- [ ] **Step 1: Create module with tests first**

`src/search/glob_mask.rs`:

```rust
use regex::{Regex, RegexBuilder};

#[derive(Debug, Clone)]
pub struct FileMasks {
    patterns: Vec<Regex>,
}

#[derive(Debug, Clone)]
pub struct DirMaskSet {
    includes: Vec<Regex>,
    excludes: Vec<Regex>,
}

fn glob_to_regex(pattern: &str) -> Result<Regex, regex::Error> {
    let mut regex_str = String::from("^");
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
        .case_insensitive(true)
        .build()
}

fn split_masks(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c| c == ';' || c == ',' || c == ' ')
        .map(str::trim)
        .filter(|p| !p.is_empty())
}

pub fn parse_file_masks(s: &str) -> Result<FileMasks, regex::Error> {
    let mut patterns = Vec::new();
    for part in split_masks(s) {
        patterns.push(glob_to_regex(part)?);
    }
    Ok(FileMasks { patterns })
}

pub fn matches_file_masks(filename: &str, masks: &FileMasks) -> bool {
    if masks.patterns.is_empty() {
        return true;
    }
    masks.patterns.iter().any(|re| re.is_match(filename))
}

pub fn parse_dir_masks(s: &str) -> Result<DirMaskSet, regex::Error> {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for part in split_masks(s) {
        if part == "**" {
            continue;
        }
        if let Some(ex) = part.strip_prefix('!') {
            let ex = ex.trim();
            if !ex.is_empty() {
                excludes.push(glob_to_regex(ex)?);
            }
        } else {
            includes.push(glob_to_regex(part)?);
        }
    }
    Ok(DirMaskSet { includes, excludes })
}

impl DirMaskSet {
    /// Whether a directory *name* (not full path) may be entered.
    pub fn allow_dir(&self, name: &str) -> bool {
        if !self.excludes.is_empty() && self.excludes.iter().any(|re| re.is_match(name)) {
            return false;
        }
        if !self.includes.is_empty() && !self.includes.iter().any(|re| re.is_match(name)) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_mask_glob_star() {
        let masks = parse_file_masks("*.rs;*.toml").unwrap();
        assert!(matches_file_masks("main.rs", &masks));
        assert!(!matches_file_masks("readme.md", &masks));
    }

    #[test]
    fn dir_mask_exclude() {
        let set = parse_dir_masks("**;!.git;!node_modules").unwrap();
        assert!(!set.allow_dir(".git"));
        assert!(set.allow_dir("src"));
    }
}
```

- [ ] **Step 2: Wire module in `src/search.rs`**

Replace monolithic body gradually. For this task, make `search` a directory module:

1. Move current `src/search.rs` → temporarily keep as facade OR convert:
   - Create `src/search/mod.rs` with current public types + `mod glob_mask;`
   - Delete old `src/search.rs` after move

**Concrete steps:**

```bash
mkdir -p src/search
# Move existing search.rs content into src/search/mod.rs (keep run_search working)
```

At top of `src/search/mod.rs`:

```rust
pub mod glob_mask;
// later: decode, match_engine, walk, pool

// keep existing SearchResultItem, SearchStatus, run_search for now
```

In `src/main.rs` the `mod search;` already works with `src/search/mod.rs`.

- [ ] **Step 3: Run unit tests**

```bash
cargo test --lib 2>&1 || cargo test glob_mask -- --nocapture
```

If no lib target, use:

```bash
cargo test --bin JGrep3
```

If tests not discovered from binary package, add to `Cargo.toml`:

```toml
[lib]
name = "jgrep3"
path = "src/lib.rs"
```

And create `src/lib.rs`:

```rust
pub mod drives;
pub mod search;
```

Keep `src/main.rs` as binary:

```rust
// remove `mod search; mod drives;` if moved to lib, use:
use jgrep3::search;
use jgrep3::drives;
// OR keep mods in main only — simplest path:

// SIMPLEST: keep binary-only package and use `cargo test` with:
// #[cfg(test)] in modules under search/ — but rustc only compiles them via:
```

**Required package layout (lock this in):**

```toml
# Cargo.toml
[lib]
name = "jgrep3"
path = "src/lib.rs"

[[bin]]
name = "JGrep3"
path = "src/main.rs"
```

`src/lib.rs`:

```rust
pub mod drives;
pub mod search;
```

`src/main.rs`: remove `mod search;` and `mod drives;`, add:

```rust
use jgrep3::search;
use jgrep3::drives;
// or use jgrep3::{search, drives};
```

Update any `mod search` / paths accordingly.

- [ ] **Step 4: Run tests**

```bash
cargo test -q
```

Expected: glob_mask tests PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs src/search src/drives.rs
git commit -m "refactor: extract search glob_mask module with unit tests"
```

---

### Task 4: `match_engine` module (TDD) — kill per-line to_lowercase

**Files:**
- Create: `src/search/match_engine.rs`
- Modify: `src/search/mod.rs`

- [ ] **Step 1: Implement Matcher with tests**

```rust
// src/search/match_engine.rs
use regex::{Regex, RegexBuilder};

#[derive(Debug, Clone)]
pub struct MatchHit {
    /// 1-based character column of first match
    pub column_chars: usize,
    /// byte offset of match start in line
    pub byte_start: usize,
    pub byte_end: usize,
}

pub enum Matcher {
    Literal {
        needle: String,
        case_sensitive: bool,
        /// lowercase needle when !case_sensitive
        needle_lower: Option<String>,
    },
    Regex(Regex),
}

impl Matcher {
    pub fn literal(query: &str, case_sensitive: bool) -> Result<Self, String> {
        if query.is_empty() {
            return Err("empty query".into());
        }
        let needle_lower = if case_sensitive {
            None
        } else {
            Some(query.to_lowercase())
        };
        Ok(Self::Literal {
            needle: query.to_string(),
            case_sensitive,
            needle_lower,
        })
    }

    pub fn regex(query: &str, case_sensitive: bool) -> Result<Self, regex::Error> {
        Ok(Self::Regex(
            RegexBuilder::new(query)
                .case_insensitive(!case_sensitive)
                .build()?,
        ))
    }

    pub fn find_in_line(&self, line: &str) -> Option<MatchHit> {
        match self {
            Self::Literal {
                needle,
                case_sensitive,
                needle_lower,
            } => {
                if *case_sensitive {
                    line.find(needle.as_str()).map(|byte_start| {
                        let byte_end = byte_start + needle.len();
                        MatchHit {
                            column_chars: line[..byte_start].chars().count() + 1,
                            byte_start,
                            byte_end,
                        }
                    })
                } else {
                    // Avoid full-line to_lowercase allocation when ASCII-only
                    let needle_l = needle_lower.as_ref().unwrap();
                    if line.is_ascii() && needle_l.is_ascii() {
                        let line_bytes = line.as_bytes();
                        let n = needle_l.as_bytes();
                        let pos = line_bytes.windows(n.len()).position(|w| {
                            w.iter()
                                .zip(n.iter())
                                .all(|(a, b)| a.to_ascii_lowercase() == *b)
                        })?;
                        Some(MatchHit {
                            column_chars: pos + 1, // ASCII: byte == char
                            byte_start: pos,
                            byte_end: pos + n.len(),
                        })
                    } else {
                        let lower = line.to_lowercase();
                        let byte_start_l = lower.find(needle_l.as_str())?;
                        // Map lowercase byte offset back carefully: rebuild via char iteration
                        // Safe approach: scan original with eq_ignore_ascii_case for ASCII;
                        // for non-ASCII use lower and map by char index.
                        let char_idx = lower[..byte_start_l].chars().count();
                        let mut byte_start = 0;
                        for (i, ch) in line.char_indices() {
                            if i == char_idx {
                                // wrong: char_idx is count not index
                                break;
                            }
                            let _ = ch;
                        }
                        // Correct mapping by char count:
                        let mut chars = line.char_indices();
                        let mut count = 0;
                        let mut start_byte = 0;
                        while let Some((i, _ch)) = chars.next() {
                            if count == char_idx {
                                start_byte = i;
                                break;
                            }
                            count += 1;
                        }
                        let matched_chars = needle_l.chars().count();
                        let end_byte = line
                            .char_indices()
                            .nth(char_idx + matched_chars)
                            .map(|(i, _)| i)
                            .unwrap_or(line.len());
                        Some(MatchHit {
                            column_chars: char_idx + 1,
                            byte_start: start_byte,
                            byte_end: end_byte,
                        })
                    }
                }
            }
            Self::Regex(re) => re.find(line).map(|m| MatchHit {
                column_chars: line[..m.start()].chars().count() + 1,
                byte_start: m.start(),
                byte_end: m.end(),
            }),
        }
    }

    pub fn is_match_line(&self, line: &str) -> bool {
        self.find_in_line(line).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_case_insensitive() {
        let m = Matcher::literal("Foo", false).unwrap();
        let hit = m.find_in_line("xxFOOyy").unwrap();
        assert_eq!(hit.column_chars, 3);
    }

    #[test]
    fn case_sensitive() {
        let m = Matcher::literal("Foo", true).unwrap();
        assert!(m.find_in_line("foo").is_none());
        assert!(m.find_in_line("Foo").is_some());
    }

    #[test]
    fn regex_case_insensitive() {
        let m = Matcher::regex(r"f.o", false).unwrap();
        assert!(m.is_match_line("FxO"));
    }
}
```

- [ ] **Step 2: `mod match_engine;` in `src/search/mod.rs`**

- [ ] **Step 3: Run tests**

```bash
cargo test match_engine -q
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/search/match_engine.rs src/search/mod.rs
git commit -m "feat(search): fast literal matcher without per-line to_lowercase"
```

---

### Task 5: `decode` module — binary skip + encoding

**Files:**
- Create: `src/search/decode.rs`
- Modify: `src/search/mod.rs`

- [ ] **Step 1: Implement read/decode with binary detection**

```rust
// src/search/decode.rs
use std::fs::File;
use std::io::Read;
use std::path::Path;

const BINARY_CHECK_BYTES: usize = 8192;

pub fn looks_binary(bytes: &[u8]) -> bool {
    let n = bytes.len().min(BINARY_CHECK_BYTES);
    if n == 0 {
        return false;
    }
    // NUL in first chunk => binary
    if bytes[..n].contains(&0) {
        return true;
    }
    false
}

pub fn read_and_decode_file(path: &Path, auto_detect: bool) -> Result<Option<String>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    if looks_binary(&buffer) {
        return Ok(None); // skip
    }

    if auto_detect {
        if let Ok(s) = std::str::from_utf8(&buffer) {
            return Ok(Some(s.to_string()));
        }
        if buffer.starts_with(&[0xEF, 0xBB, 0xBF]) {
            if let Ok(s) = std::str::from_utf8(&buffer[3..]) {
                return Ok(Some(s.to_string()));
            }
        }
        let (res, _, has_errors) = encoding_rs::SHIFT_JIS.decode(&buffer);
        if !has_errors {
            return Ok(Some(res.into_owned()));
        }
        let (res, _, has_errors) = encoding_rs::EUC_JP.decode(&buffer);
        if !has_errors {
            return Ok(Some(res.into_owned()));
        }
        Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
    } else {
        Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_nul_binary() {
        assert!(looks_binary(b"abc\0def"));
        assert!(!looks_binary(b"hello world"));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test decode -q
```

- [ ] **Step 3: Commit**

```bash
git add src/search/decode.rs src/search/mod.rs
git commit -m "feat(search): decode module with binary early-skip"
```

---

### Task 6: `walk` module — replace walkdir with FindFirstFileW

**Files:**
- Create: `src/search/walk.rs`
- Modify: `Cargo.toml` (remove `walkdir` after switch)
- Modify: `src/search/mod.rs`

- [ ] **Step 1: Implement recursive walk**

```rust
// src/search/walk.rs
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
/// Reading/matching is parallelized by the pool.
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
                    walk_dir(&path, cfg, cancel, out);
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
```

- [ ] **Step 2: Ensure `Win32_Storage_FileSystem` includes FindFirstFile (already in Cargo.toml features)**

- [ ] **Step 3: Manual smoke — call `collect_files` from a quick test with repo root**

```rust
#[cfg(test)]
mod tests {
    // only if we can run without full cancel plumbing
}
```

- [ ] **Step 4: Commit walk module (still not wired to run_search)**

```bash
git add src/search/walk.rs src/search/mod.rs
git commit -m "feat(search): Win32 FindFirstFile directory walk"
```

---

### Task 7: `pool` + rewrite `run_search`

**Files:**
- Create: `src/search/pool.rs`
- Modify: `src/search/mod.rs` (new `run_search`)
- Modify: `src/main.rs` (mpsc Receiver, handle_notice)
- Modify: `Cargo.toml` (remove `walkdir`, `crossbeam-channel`)

- [ ] **Step 1: Define channel type as `std::sync::mpsc`**

In `src/search/mod.rs`:

```rust
use std::sync::mpsc::Sender;
use native_windows_gui::NoticeSender;
// ...

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
```

- [ ] **Step 2: Implement pool**

```rust
// src/search/pool.rs
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc::Sender};
use std::thread;
use std::time::Instant;

use native_windows_gui::NoticeSender;

use super::{SearchResultItem, SearchStatus};
use super::decode::read_and_decode_file;
use super::glob_mask::{parse_dir_masks, parse_file_masks};
use super::match_engine::Matcher;
use super::walk::{collect_files, WalkConfig};

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
    notice_sender: NoticeSender,
) {
    let start = Instant::now();

    let matcher = if is_regex {
        match Matcher::regex(&search_query, case_sensitive) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(format!("Invalid regex: {}", e)));
                notice_sender.notice();
                return;
            }
        }
    } else {
        match Matcher::literal(&search_query, case_sensitive) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                let _ = sender.send(SearchStatus::Error(e));
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

    let mut handles = Vec::new();
    for _ in 0..workers {
        let files = Arc::clone(&files);
        let next_index = Arc::clone(&next_index);
        let matcher = Arc::clone(&matcher);
        let cancel = Arc::clone(&cancellation_token);
        let sender = sender.clone();
        let notice_sender = notice_sender.clone();
        let match_count = Arc::clone(&match_count);
        let scanned = Arc::clone(&scanned);
        let query_for_trunc = search_query.clone();

        handles.push(thread::spawn(move || {
            let mut batch: Vec<SearchResultItem> = Vec::with_capacity(MATCH_BATCH);
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
                    notice_sender.notice();
                }

                let content = match read_and_decode_file(path, auto_detect_encoding) {
                    Ok(Some(s)) => s,
                    _ => continue,
                };

                for (line_idx, line) in content.lines().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(hit) = matcher.find_in_line(line) else { continue };
                    let line_num = line_idx + 1;
                    let line_content = truncate_line(line, &query_for_trunc, matcher.as_ref());
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
    notice: &NoticeSender,
    batch: &mut Vec<SearchResultItem>,
) {
    if batch.is_empty() {
        return;
    }
    for item in batch.drain(..) {
        let _ = sender.send(SearchStatus::Match(item));
    }
    notice.notice();
}

fn finish(
    sender: &Sender<SearchStatus>,
    notice: &NoticeSender,
    start: Instant,
    total_scanned: usize,
    match_count: usize,
) {
    let _ = sender.send(SearchStatus::Completed {
        elapsed_ms: start.elapsed().as_millis() as u64,
        total_scanned,
        match_count,
    });
    notice.notice();
}

fn truncate_line(line: &str, _query: &str, matcher: &Matcher) -> String {
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
```

Note: `NoticeSender` must be `Clone` (nwg NoticeSender is Clone). `Sender` from mpsc is Clone.

- [ ] **Step 3: Fix `main.rs` for mpsc**

```rust
// remove: use crossbeam_channel::Receiver;
use std::sync::mpsc::{self, Receiver};

// handle_search:
let (tx, rx) = mpsc::channel();

// handle_notice — cannot clone Receiver:
fn handle_notice(&self) {
    let mut drained = Vec::new();
    {
        let mut state = self.search_state.borrow_mut();
        if let Some(s) = state.as_mut() {
            while let Ok(status) = s.receiver.try_recv() {
                drained.push(status);
            }
        }
    }
    for status in drained {
        // same match arms as before
    }
}
```

- [ ] **Step 4: Remove deps from Cargo.toml**

```toml
# delete walkdir and crossbeam-channel lines
```

- [ ] **Step 5: Remove unused nwg flexbox feature**

```toml
native-windows-gui = { version = "1.0.12", features = ["combobox", "list-view", "tree-view", "menu", "status-bar", "image-list", "cursor", "file-dialog", "frame", "high-dpi", "progress-bar"] }
```

- [ ] **Step 6: Build and fix compile errors**

```bash
cargo build --release 2>&1
```

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/search src/main.rs src/lib.rs
git commit -m "feat(search): parallel worker pool, mpsc, drop walkdir/crossbeam"
```

---

### Task 8: Bench harness + measure speedup

**Files:**
- Create: `src/bin/bench_search.rs` OR `examples/bench_search.rs`

- [ ] **Step 1: Add example binary**

`examples/bench_search.rs`:

```rust
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::Instant;

// Use internal API — if run_search needs NoticeSender, add a bench entrypoint:

// Prefer adding to search module:
// pub fn run_search_bench(...) -> (u64, usize, usize) without UI notice
```

Add to `src/search/mod.rs`:

```rust
/// Headless search for benchmarks (no NoticeSender).
pub fn run_search_headless(
    search_dir: PathBuf,
    search_query: String,
    file_mask_str: String,
    dir_mask_str: String,
    recursive: bool,
    case_sensitive: bool,
    is_regex: bool,
    auto_detect_encoding: bool,
) -> (u64, usize, usize) {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    // Implement pool path that accepts Option<NoticeSender> OR dummy no-op notice.
    // Simplest: pass a black-hole by making notice optional in pool::run.
    ...
}
```

**Cleaner:** change `pool::run` to take `Option<NoticeSender>` and only call `notice()` when `Some`.

`run_search` passes `Some(notice_sender)`; headless passes `None`.

- [ ] **Step 2: Example**

```rust
// examples/bench_search.rs
fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let query = std::env::args().nth(2).unwrap_or_else(|| "fn ".into());
    let t0 = Instant::now();
    let (ms, scanned, matches) = jgrep3::search::run_search_headless(
        PathBuf::from(dir),
        query,
        "*.rs".into(),
        "**;!.git;!target".into(),
        true,
        true,
        false,
        false,
    );
    eprintln!("elapsed_ms={ms} scanned={scanned} matches={matches} wall={:?}", t0.elapsed());
}
```

- [ ] **Step 3: Run bench on repo**

```bash
cargo run --release --example bench_search -- . "fn " 
```

Record results in `docs/superpowers/specs/2026-07-11-search-perf-size-design.md` appendix or commit message.

- [ ] **Step 4: Commit**

```bash
git add examples/bench_search.rs src/search
git commit -m "chore: add headless search benchmark example"
```

---

### Task 9: Size audit + final polish

**Files:**
- Modify: `Cargo.toml` (trim `windows` features if safe)
- Modify: `README.md` (note performance/size if needed)

- [ ] **Step 1: Measure binary**

```bash
cargo build --release
ls -la target/release/JGrep3.exe
```

Target: ≤1_572_864 (1.5MB). Stretch: ≤1_048_576.

- [ ] **Step 2: If still >1.5MB, try in order**

1. Confirm `strip = true` and LTO applied (`cargo clean && cargo build --release`)
2. Remove any remaining unused `windows` features (grep `windows::` usage)
3. `opt-level = "s"` vs `"z"` comparison
4. Do **not** remove features required by design

- [ ] **Step 3: Manual regression checklist**

- [ ] Literal search works
- [ ] Case-insensitive works
- [ ] Regex works
- [ ] File mask `*.rs`
- [ ] Dir exclude `!.git`
- [ ] Esc cancel
- [ ] Progress updates
- [ ] Maximize still lays out (prior fix)
- [ ] Settings / theme still work

- [ ] **Step 4: Update design doc with measured numbers**

Append section:

```markdown
## Appendix: Measured results (YYYY-MM-DD)

| Metric | Before | After | Ratio |
|--------|--------|-------|-------|
| Binary | ... | ... | ... |
| Bench literal | ... ms | ... ms | ...× |
```

- [ ] **Step 5: Final commit**

```bash
git add README.md docs/superpowers/specs/2026-07-11-search-perf-size-design.md Cargo.toml
git commit -m "docs: record search perf and binary size results"
```

---

## Spec coverage checklist

| Spec requirement | Task |
|------------------|------|
| Release LTO/strip/opt-z | Task 1 |
| Remove walkdir → FindFirstFile | Task 6 |
| Remove crossbeam → mpsc | Task 7 |
| Parallel workers ≤8 | Task 7 |
| Batch match notices | Task 7 |
| No per-line to_lowercase | Task 4 |
| No per-match Regex rebuild | Task 4 |
| Binary early skip | Task 5 |
| encoding_rs keep | Task 5 |
| Mask semantics | Task 3 |
| API compatibility SearchStatus | Task 7 |
| nwg flexbox remove | Task 7 |
| Bench ≥10× | Task 8 |
| Size ≤1.5MB | Task 9 |
| Feature parity regression | Task 9 |

## Self-review notes

- Fixed `Receiver::clone` issue for mpsc via drain pattern in Task 7.
- Package needs `[lib]` for unit tests — locked in Task 3.
- Walk logic must be cleaned of draft duplication when coding Task 6.
- `NoticeSender` optional for headless bench — Task 8.
- No TBD left for core path; mmap deferred (chunk read first is enough for v1).
