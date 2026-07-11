//! 検索マッチャ。
//!
//! リテラル（大小区別/無視）と正規表現に対応。
//! ASCII リテラルは行の全小文字化を回避しバイト列比較で高速化。

use regex::RegexBuilder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchHit {
    pub column_chars: usize, // 1-based
    pub byte_start: usize,
    pub byte_end: usize,
}

pub enum Matcher {
    Literal {
        needle: String,
        case_sensitive: bool,
        needle_lower: Option<String>,
    },
    Regex(regex::Regex),
}

impl Matcher {
    pub fn literal(query: &str, case_sensitive: bool) -> Result<Self, String> {
        if query.is_empty() {
            return Err("empty query".to_string());
        }
        let needle_lower = if case_sensitive {
            None
        } else {
            Some(query.to_lowercase())
        };
        Ok(Matcher::Literal {
            needle: query.to_string(),
            case_sensitive,
            needle_lower,
        })
    }

    pub fn regex(query: &str, case_sensitive: bool) -> Result<Self, regex::Error> {
        let re = RegexBuilder::new(query)
            .case_insensitive(!case_sensitive)
            .build()?;
        Ok(Matcher::Regex(re))
    }

    pub fn find_in_line(&self, line: &str) -> Option<MatchHit> {
        match self {
            Matcher::Literal {
                needle,
                case_sensitive,
                needle_lower,
            } => {
                if *case_sensitive {
                    let byte_start = line.find(needle.as_str())?;
                    let byte_end = byte_start + needle.len();
                    Some(MatchHit {
                        column_chars: column_from_byte(line, byte_start),
                        byte_start,
                        byte_end,
                    })
                } else {
                    let needle_lower = needle_lower.as_ref()?;
                    find_literal_case_insensitive(line, needle, needle_lower)
                }
            }
            Matcher::Regex(re) => {
                let m = re.find(line)?;
                Some(MatchHit {
                    column_chars: column_from_byte(line, m.start()),
                    byte_start: m.start(),
                    byte_end: m.end(),
                })
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_match_line(&self, line: &str) -> bool {
        self.find_in_line(line).is_some()
    }
}

fn column_from_byte(line: &str, byte_start: usize) -> usize {
    if line.is_ascii() {
        byte_start + 1
    } else {
        line[..byte_start].chars().count() + 1
    }
}

/// Case-insensitive literal search. ASCII lines avoid allocation via window compare.
fn find_literal_case_insensitive(
    line: &str,
    needle: &str,
    needle_lower: &str,
) -> Option<MatchHit> {
    let nlen = needle_lower.len();
    if nlen == 0 {
        return Some(MatchHit {
            column_chars: 1,
            byte_start: 0,
            byte_end: 0,
        });
    }
    if line.len() < nlen {
        return None;
    }

    if line.is_ascii() && needle.is_ascii() {
        let hay = line.as_bytes();
        let needle_bytes = needle_lower.as_bytes();
        for i in 0..=(hay.len() - nlen) {
            let window = &hay[i..i + nlen];
            if window
                .iter()
                .zip(needle_bytes.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
            {
                return Some(MatchHit {
                    column_chars: i + 1,
                    byte_start: i,
                    byte_end: i + nlen,
                });
            }
        }
        return None;
    }

    // Non-ASCII fallback: walk chars, compare lowercased windows (one path, no full-line alloc for small needles)
    find_literal_case_insensitive_unicode(line, needle_lower)
}

fn find_literal_case_insensitive_unicode(line: &str, needle_lower: &str) -> Option<MatchHit> {
    let needle_chars: Vec<char> = needle_lower.chars().collect();
    let nlen = needle_chars.len();
    if nlen == 0 {
        return Some(MatchHit {
            column_chars: 1,
            byte_start: 0,
            byte_end: 0,
        });
    }

    let line_chars: Vec<(usize, char)> = line.char_indices().collect();
    if line_chars.len() < nlen {
        return None;
    }

    for i in 0..=(line_chars.len() - nlen) {
        let mut matched = true;
        for j in 0..nlen {
            let lower: String = line_chars[i + j].1.to_lowercase().collect();
            if lower != needle_chars[j].to_string() {
                matched = false;
                break;
            }
        }
        if matched {
            let byte_start = line_chars[i].0;
            let last = &line_chars[i + nlen - 1];
            let byte_end = last.0 + last.1.len_utf8();
            return Some(MatchHit {
                column_chars: i + 1,
                byte_start,
                byte_end,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_case_insensitive_finds_correct_column() {
        let m = Matcher::literal("Foo", false).unwrap();
        let hit = m.find_in_line("xxFOOyy").unwrap();
        assert_eq!(hit.column_chars, 3);
        assert_eq!(hit.byte_start, 2);
        assert_eq!(hit.byte_end, 5);
        assert!(m.is_match_line("xxFOOyy"));
    }

    #[test]
    fn case_sensitive_no_match_on_wrong_case() {
        let m = Matcher::literal("Foo", true).unwrap();
        assert!(m.find_in_line("foo").is_none());
        assert!(!m.is_match_line("foo"));
        let hit = m.find_in_line("xxFooyy").unwrap();
        assert_eq!(hit.column_chars, 3);
    }

    #[test]
    fn regex_works_case_insensitive() {
        let m = Matcher::regex(r"f.o", false).unwrap();
        let hit = m.find_in_line("xxFAOyy").unwrap();
        assert_eq!(hit.column_chars, 3);
        assert!(m.is_match_line("xxFAOyy"));
        assert!(!m.is_match_line("xxFByy"));
    }
}
