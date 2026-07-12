//! ファイル読込・デコード・バイナリ判定。
//!
//! UTF-8 優先、Shift_JIS / EUC-JP / ISO-2022-JP へのフォールバック。
//! NUL バイト検出によるバイナリファイルスキップ。

use std::fs::File;
use std::io::Read;
use std::path::Path;

const BINARY_CHECK_BYTES: usize = 8192;

pub fn looks_binary(bytes: &[u8]) -> bool {
    let n = bytes.len().min(BINARY_CHECK_BYTES);
    if n == 0 {
        return false;
    }
    bytes[..n].contains(&0)
}

pub fn read_and_decode_file(
    path: &Path,
    auto_detect: bool,
) -> Result<Option<String>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    if looks_binary(&buffer) {
        return Ok(None);
    }

    if auto_detect {
        if let Ok(s) = std::str::from_utf8(&buffer) {
            return Ok(Some(s.to_string()));
        }
        if buffer.starts_with(&[0xEF, 0xBB, 0xBF])
            && let Ok(s) = std::str::from_utf8(&buffer[3..]) {
                return Ok(Some(s.to_string()));
            }
        let (res, _, has_errors) = encoding_rs::SHIFT_JIS.decode(&buffer);
        if !has_errors {
            return Ok(Some(res.into_owned()));
        }
        let (res, _, has_errors) = encoding_rs::EUC_JP.decode(&buffer);
        if !has_errors {
            return Ok(Some(res.into_owned()));
        }
        let (res, _, has_errors) = encoding_rs::ISO_2022_JP.decode(&buffer);
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
