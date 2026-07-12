//! .gitignore のパースとマッチング。
//!
//! Git の仕様に基づくパターンマッチングをサポートする。
//! 空行、コメント、否定パターン（!）、ディレクトリ限定（/末尾）、ワイルドカード（*, ?, **）に対応。

use std::path::{Path, PathBuf};
use regex::{Regex, RegexBuilder};

#[derive(Clone, Debug)]
pub struct GitIgnoreRule {
    pub regex: Regex,
    pub is_negation: bool,
    pub is_dir_only: bool,
    pub raw_pattern: String,
}

#[derive(Clone, Debug)]
pub struct GitIgnore {
    pub base_dir: PathBuf,
    pub rules: Vec<GitIgnoreRule>,
}

/// 指定されたディレクトリ直下にある無視ファイル (.gitignore, .ignore, .rgignore) を読み込んでロードする。
pub fn load_ignore_files(dir: &Path) -> Vec<GitIgnore> {
    let mut ignores = Vec::new();
    let names = [".gitignore", ".ignore", ".rgignore"];
    for name in &names {
        let path = dir.join(name);
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                ignores.push(GitIgnore::new(dir.to_path_buf(), &content));
            }
        }
    }
    ignores
}

impl GitIgnore {
    /// 新しい GitIgnore インスタンスを構築する。
    pub fn new(base_dir: PathBuf, content: &str) -> Self {
        let mut rules = Vec::new();
        for line in content.lines() {
            if let Some(parsed) = parse_line(line) {
                if let Some((regex, is_negation, is_dir_only)) = gitignore_to_regex(parsed) {
                    rules.push(GitIgnoreRule {
                        regex,
                        is_negation,
                        is_dir_only,
                        raw_pattern: line.to_string(),
                    });
                }
            }
        }
        GitIgnore { base_dir, rules }
    }

    /// 指定されたパスが無視されるべきかを判定する。
    /// `path` は絶対パス、または `base_dir` からの相対パス。
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        let rel_path = if path.is_absolute() {
            match path.strip_prefix(&self.base_dir) {
                Ok(p) => p,
                Err(_) => return false,
            }
        } else {
            path
        };

        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");
        if rel_path_str.is_empty() {
            return false;
        }

        let mut ignored = false;
        for rule in &self.rules {
            if rule.is_dir_only && !is_dir {
                continue;
            }
            if rule.regex.is_match(&rel_path_str) {
                ignored = !rule.is_negation;
            }
        }
        ignored
    }
}

fn parse_line(line: &str) -> Option<&str> {
    let trimmed_start = line.trim_start();
    if trimmed_start.is_empty() {
        return None;
    }
    if line.starts_with('#') {
        return None;
    }

    // エスケープされていない末尾のスペースを取り除く
    let mut end = line.len();
    let bytes = line.as_bytes();
    while end > 0 {
        if bytes[end - 1] == b' ' {
            let mut backslashes = 0;
            let mut idx = end - 2;
            while idx > 0 && bytes[idx] == b'\\' {
                backslashes += 1;
                idx -= 1;
            }
            if idx == 0 && bytes[0] == b'\\' {
                backslashes += 1;
            }
            if backslashes % 2 == 0 {
                end -= 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    let s = &line[..end];
    if s.is_empty() {
        return None;
    }
    Some(s)
}

fn gitignore_to_regex(mut pattern: &str) -> Option<(Regex, bool, bool)> {
    if pattern.is_empty() {
        return None;
    }

    let mut is_negation = false;
    if pattern.starts_with('!') {
        is_negation = true;
        pattern = &pattern[1..];
    }

    let mut is_dir_only = false;
    if pattern.ends_with('/') && !pattern.ends_with("\\/") {
        is_dir_only = true;
        pattern = &pattern[..pattern.len() - 1];
    }

    let mut is_anchored = false;
    if pattern.starts_with('/') {
        is_anchored = true;
        pattern = &pattern[1..];
    } else {
        let mut has_slash = false;
        let mut chars = pattern.chars().enumerate();
        while let Some((i, c)) = chars.next() {
            if c == '/' {
                if i == 0 || pattern.as_bytes()[i - 1] != b'\\' {
                    has_slash = true;
                    break;
                }
            }
        }
        if has_slash {
            is_anchored = true;
        }
    }

    let mut re_str = String::new();
    if is_anchored {
        re_str.push('^');
    } else {
        re_str.push_str("^(?:.*/)?");
    }

    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&next_c) = chars.peek() {
                    match next_c {
                        ' ' | '#' | '!' | '\\' | '/' | '*' | '?' => {
                            re_str.push('\\');
                            re_str.push(next_c);
                        }
                        _ => {
                            re_str.push_str("\\\\");
                            re_str.push(next_c);
                        }
                    }
                    chars.next();
                } else {
                    re_str.push_str("\\\\");
                }
            }
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    let next_is_slash = chars.peek() == Some(&'/');
                    if next_is_slash {
                        chars.next();
                        re_str.push_str("(?:.*/)?");
                    } else {
                        re_str.push_str(".*");
                    }
                } else {
                    re_str.push_str("[^/]*");
                }
            }
            '?' => {
                re_str.push_str("[^/]");
            }
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                re_str.push('\\');
                re_str.push(c);
            }
            _ => {
                re_str.push(c);
            }
        }
    }
    re_str.push('$');

    let re = RegexBuilder::new(&re_str)
        .case_insensitive(true)
        .build()
        .ok()?;

    Some((re, is_negation, is_dir_only))
}

#[derive(Clone, Debug, Default)]
pub struct SearchFilter {
    pub include_rules: Vec<GitIgnoreRule>,
    pub exclude_rules: Vec<GitIgnoreRule>,
}

impl SearchFilter {
    pub fn new(mask_str: &str) -> Self {
        let mut include_rules = Vec::new();
        let mut exclude_rules = Vec::new();

        for part in split_masks(mask_str) {
            if let Some(ex) = part.strip_prefix('!') {
                if let Some((regex, _, is_dir_only)) = gitignore_to_regex(ex) {
                    exclude_rules.push(GitIgnoreRule {
                        regex,
                        is_negation: false,
                        is_dir_only,
                        raw_pattern: part.to_string(),
                    });
                }
            } else {
                if let Some((regex, _, is_dir_only)) = gitignore_to_regex(part) {
                    include_rules.push(GitIgnoreRule {
                        regex,
                        is_negation: false,
                        is_dir_only,
                        raw_pattern: part.to_string(),
                    });
                }
            }
        }

        SearchFilter {
            include_rules,
            exclude_rules,
        }
    }

    pub fn allow_dir(&self, rel_path: &Path) -> bool {
        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");
        for rule in &self.exclude_rules {
            if rule.regex.is_match(&rel_path_str) {
                return false;
            }

            // 親ディレクトリのチェック
            let mut parent = rel_path.parent();
            while let Some(p) = parent {
                let p_str = p.to_string_lossy().replace('\\', "/");
                if !p_str.is_empty() && rule.regex.is_match(&p_str) {
                    return false;
                }
                parent = p.parent();
            }
        }
        true
    }

    pub fn allow_file(&self, rel_path: &Path) -> bool {
        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");

        for rule in &self.exclude_rules {
            if rule.is_dir_only {
                // 親ディレクトリがこのルールにマッチするかチェック
                let mut parent = rel_path.parent();
                let mut matched_parent = false;
                while let Some(p) = parent {
                    let p_str = p.to_string_lossy().replace('\\', "/");
                    if !p_str.is_empty() && rule.regex.is_match(&p_str) {
                        matched_parent = true;
                        break;
                    }
                    parent = p.parent();
                }
                if matched_parent {
                    return false;
                }
            } else {
                if rule.regex.is_match(&rel_path_str) {
                    return false;
                }
            }
        }

        if !self.include_rules.is_empty() {
            let mut matched = false;
            for rule in &self.include_rules {
                if rule.is_dir_only {
                    continue;
                }
                if rule.regex.is_match(&rel_path_str) {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return false;
            }
        }

        true
    }
}

pub fn split_masks(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c| c == ';' || c == ',' || c == ' ')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    const BASE: &str = "C:\\project";
    #[cfg(not(windows))]
    const BASE: &str = "/project";

    fn p(rel: &str) -> PathBuf {
        let mut base = PathBuf::from(BASE);
        for part in rel.split('/') {
            if !part.is_empty() {
                base.push(part);
            }
        }
        base
    }

    #[test]
    fn test_basic_ignore() {
        let base_path = PathBuf::from(BASE);
        let gi = GitIgnore::new(base_path, "*.log\n/temp/\n!important.log");
        
        // *.log
        assert!(gi.is_ignored(&p("a.log"), false));
        assert!(gi.is_ignored(&p("src/b.log"), false));
        
        // !important.log
        assert!(!gi.is_ignored(&p("important.log"), false));
        assert!(!gi.is_ignored(&p("src/important.log"), false));
        
        // /temp/ (dir only)
        assert!(gi.is_ignored(&p("temp"), true));
        assert!(!gi.is_ignored(&p("temp"), false));
        assert!(!gi.is_ignored(&p("src/temp"), true)); // Anchored to root
    }

    #[test]
    fn test_wildcard_gitignore() {
        let base_path = PathBuf::from(BASE);
        let gi = GitIgnore::new(base_path, "**/build/\nsrc/**/test.rs");

        // **/build/
        assert!(gi.is_ignored(&p("build"), true));
        assert!(gi.is_ignored(&p("foo/build"), true));
        assert!(!gi.is_ignored(&p("build"), false)); // dir only

        // src/**/test.rs
        assert!(gi.is_ignored(&p("src/test.rs"), false));
        assert!(gi.is_ignored(&p("src/foo/test.rs"), false));
        assert!(gi.is_ignored(&p("src/foo/bar/test.rs"), false));
        assert!(!gi.is_ignored(&p("test.rs"), false));
    }

    #[test]
    fn test_search_filter() {
        let sf = SearchFilter::new("*.rs; !target/; !node_modules/");

        // Dir tests
        assert!(sf.allow_dir(&p("src")));
        assert!(!sf.allow_dir(&p("target")));
        assert!(!sf.allow_dir(&p("foo/node_modules"))); // nested exclusion

        // File tests
        assert!(sf.allow_file(&p("src/main.rs")));
        assert!(!sf.allow_file(&p("src/README.md"))); // Not matching *.rs
        assert!(!sf.allow_file(&p("target/main.rs"))); // In target (dir is ignored, but if evaluated directly as file)
    }
}
