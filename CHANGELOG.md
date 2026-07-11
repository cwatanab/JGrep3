# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-07-11

### Added
- 外部エディタの引数に、カーソル位置からプレースホルダー（`%FILENAME%`, `%LINE%`, `%COL%`）を自動挿入できるクリックボタンを追加
- 引数テンプレートのプレースホルダー置換機能に対するユニットテストを追加

### Changed
- 設定ダイアログのデザインを legend 風の凡例見出しに刷新し、ラベル幅とコントロールの配置・余白を調整
- 設定ウィンドウの横幅を 600px に拡張し、各種コンボボックスやテキスト入力領域の幅を拡張
- エディタ設定フレーム内のプレースホルダーボタンの下部に余白（25px）を追加
- 引数テンプレートのプレースホルダーを従来の `$f`, `$l`, `$c` から `%FILENAME%`, `%LINE%`, `%COL%` へバッチ変数形式に変更

### Removed
- 旧プレースホルダー（ドル記法 `$f`, `$l`, `$c`）のサポートを削除

### Fixed
- 設定画面でのフォント変更時、見出しラベルがグループ枠線の背面に隠れてしまうZ-orderのバグを修正

[0.1.2]: https://github.com/cwatanab/JGrep3/compare/v0.1.1...v0.1.2
