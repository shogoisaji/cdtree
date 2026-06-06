# Changelog

## [0.1.11] - 2026-06-06

### Added
- Added `--doctor` to inspect shell integration status
- Added `--uninstall` to remove shell integration

### Changed
- Manage shell integration with marked rc blocks
- Show manual setup instructions when rc files are managed or not writable, such as with Home Manager or Nix
- Harden parent-shell execution by quoting launcher paths, resolving the cdtree executable, and validating cd targets

### Fixed
- Fixed Open/Code mode success messages being written to stdout and potentially treated as cd targets
- Fixed terminal state cleanup when TUI initialization fails

## [0.1.10] - 2026-06-04

### Fixed
- 初期表示でAll状態にもかかわらず隠しファイルが表示されない問題を修正

## [0.1.9] - 2026-04-19

### Added
- History mode内でTabキーによるCd/Open/Codeモード切替をサポート
- 選択項目にモードサフィックス(_CD/_OPEN/_CODE)を表示
- History modeのガイドバーにTab Modeを追加
- キーイベントのPressのみを処理するようフィルターを追加（Spaceキーが効かない問題を解決）
