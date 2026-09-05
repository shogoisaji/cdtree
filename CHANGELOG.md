# Changelog

## [Unreleased]

### Added
- `f` enters name search mode and highlights case-insensitive partial matches in currently visible file/folder names
- Title bar shows a right-aligned `Find [query_]` input on the top border while searching
- `Esc` exits search and returns to normal navigation (`q` / letters type into the query)

### Changed
- File visibility toggle moved from `f` to `v`

## [0.2.0] - 2026-07-03

### Added
- Mouse support: scroll wheel scrolls the tree/history viewport without moving the selection
- Single click selects a row and toggles directory expansion (tree mode) / selects a history entry
- Double click confirms the selection (cd/open/code), mirroring Enter behavior
- `App::toggle_current`, `select_visible_index`, `scroll`, `scroll_history` for mouse-driven interaction

### Changed
- `run_app` event loop refactored to handle `Event::Mouse` alongside `Event::Key`
- `list_content_area` mirrors the `ui.rs` list layout for accurate mouse hit-testing
- Replace `io::Error::new(io::ErrorKind::Other, ..)` with `io::Error::other` to satisfy clippy

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
