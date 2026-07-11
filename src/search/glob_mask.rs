use regex::{Regex, RegexBuilder};

pub struct FileMasks {
    patterns: Vec<Regex>,
}

pub struct DirMaskSet {
    includes: Vec<Regex>,
    excludes: Vec<Regex>,
}

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
        .case_insensitive(true)
        .build()
}

fn split_masks(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c| c == ';' || c == ',' || c == ' ')
        .map(|p| p.trim())
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
    masks.patterns.iter().any(|mask| mask.is_match(filename))
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

    #[test]
    fn file_mask_empty_matches_all() {
        let masks = parse_file_masks("").unwrap();
        assert!(matches_file_masks("anything.txt", &masks));
    }

    #[test]
    fn file_mask_case_insensitive() {
        let masks = parse_file_masks("*.RS").unwrap();
        assert!(matches_file_masks("main.rs", &masks));
    }

    #[test]
    fn dir_mask_include_filter() {
        let set = parse_dir_masks("src;lib").unwrap();
        assert!(set.allow_dir("src"));
        assert!(!set.allow_dir("target"));
    }
}
