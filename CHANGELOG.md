# Changelog

## [0.4.0] - 2026-11-01

### Changed
- `f` searches directory names under the tree root for a continuous, case-insensitive substring after typing pauses (`abc` matches `XabcY` and not `ZBXC`), then shows matches as an expanded tree (ancestors stay visible, other entries are hidden) with matched characters highlighted
- Search hits appear as soon as they are found. The walk continues in the background, and the match count keeps updating until it finishes
- Search advances during navigation events and yields between filesystem operations after a short time slice, including inside large directories
- Partial results are labeled even with zero matches, including depth-limit omissions and directory read errors

### Fixed
- Returning to an earlier query after editing no longer treats an unfinished search snapshot as a completed result
- A user-selected deep result stays selected while a revised query is still scanning shallower branches
- Leaving search with Esc or clearing the query preserves the selected result even when the live root was collapsed before searching

## [0.3.0] - 2026-09-05

### Added
- `f` enters name search mode and highlights case-insensitive partial matches in currently visible file/folder names
- Title bar shows a right-aligned `Find [query_]` input on the top border while searching
- `Esc` exits search and returns to normal navigation (`q` / letters type into the query)

### Changed
- File visibility toggle moved from `f` to `v`

### Fixed
- Search field truncation uses terminal display width so CJK queries do not overflow the title
- Name matching case-folds expanding characters such as `ß`

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
