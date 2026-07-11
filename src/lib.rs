//! JGrep3 — Windows ネイティブ GUI ファイル検索ツール
//!
//! Rust + native-windows-gui で実装。並列検索エンジンによる高速全文検索、
//! ダーク/ライトテーマ、文字コード自動判別に対応。

pub mod config;
pub mod custom_draw;
pub mod drives;
pub mod search;
pub mod settings;
pub mod theme;
