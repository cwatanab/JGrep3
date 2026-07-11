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
        let n = nlen;
        let first_byte = needle_bytes[0];
        let first_upper = first_byte.to_ascii_uppercase();
        let use_memchr2 = first_byte != first_upper;

        let mut search_pos = 0;
        let limit = hay.len() - n;
        while search_pos <= limit {
            let found = if use_memchr2 {
                memchr::memchr2(first_byte, first_upper, &hay[search_pos..=limit])
            } else {
                memchr::memchr(first_byte, &hay[search_pos..=limit])
            };
            if let Some(offset) = found {
                let hit_idx = search_pos + offset;
                let mut ok = true;
                for j in 1..n {
                    if hay[hit_idx + j].to_ascii_lowercase() != needle_bytes[j] {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    return Some(MatchHit {
                        column_chars: hit_idx + 1,
                        byte_start: hit_idx,
                        byte_end: hit_idx + n,
                    });
                }
                search_pos = hit_idx + 1;
            } else {
                break;
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

    let mut char_count = 0;
    for (byte_idx, _) in line.char_indices() {
        char_count += 1;
        let mut line_iter = line[byte_idx..].chars();
        let mut matched = true;
        let mut matched_bytes = 0;
        for &nc in &needle_chars {
            if let Some(lc) = line_iter.next() {
                if !lc.to_lowercase().eq(nc.to_lowercase()) {
                    matched = false;
                    break;
                }
                matched_bytes += lc.len_utf8();
            } else {
                matched = false;
                break;
            }
        }
        if matched {
            return Some(MatchHit {
                column_chars: char_count,
                byte_start: byte_idx,
                byte_end: byte_idx + matched_bytes,
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
