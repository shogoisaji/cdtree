//! Directory-name search under the tree root.
//!
//! A query matches only when those characters occur together, in order, so
//! `abc` hits `abc` and `XabcY` and misses `ZBXC`.
//!
//! The walk is sliced so the UI can keep reading keys. Heavy directories are
//! visited after ordinary ones, so a match under a project is found before the
//! budget is spent inside `Library`, `node_modules`, or build output.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long typing must stay idle before a walk starts.
pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// Maximum directories completed in one turn of the event loop.
pub(crate) const DIRS_PER_SLICE: usize = 400;
/// Yield between filesystem operations after this much UI-thread time.
const SEARCH_SLICE_TIME: Duration = Duration::from_millis(8);

const MAX_SEARCH_DIRS: usize = 40_000;
const MAX_SEARCH_MATCHES: usize = 400;
const MAX_SEARCH_DEPTH: usize = 32;

/// Directory names whose contents are searched after the rest of the tree.
/// The directory itself can still match; only descent is postponed.
const HEAVY_DIR_NAMES: &[&str] = &[
    ".cargo",
    ".git",
    "Caches",
    "DerivedData",
    "Library",
    "Pods",
    "node_modules",
    "target",
    "vendor",
];

const START_BONUS: i32 = 16;
const BOUNDARY_BONUS: i32 = 16;

#[derive(Clone, Copy)]
pub(crate) struct ScanLimits {
    pub max_dirs: usize,
    pub max_matches: usize,
    pub max_depth: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_dirs: MAX_SEARCH_DIRS,
            max_matches: MAX_SEARCH_MATCHES,
            max_depth: MAX_SEARCH_DEPTH,
        }
    }
}

pub(crate) enum SearchPhase {
    Idle,
    Waiting { since: std::time::Instant },
    Scanning(Box<SearchScan>),
}

pub(crate) struct SearchScan {
    query: String,
    query_folded: Vec<char>,
    show_hidden: bool,
    limits: ScanLimits,
    /// Ordinary directories still to open. `(path, depth)`.
    queue: VecDeque<(PathBuf, usize)>,
    /// Heavy subtrees, opened only after `queue` is empty.
    deferred: VecDeque<(PathBuf, usize)>,
    active: Option<DirectoryRead>,
    matches: Vec<PathBuf>,
    visited: usize,
    truncated: bool,
}

/// Keep a large directory resumable, including its sorted child processing.
struct DirectoryRead {
    entries: Option<fs::ReadDir>,
    children: BTreeMap<String, (PathBuf, bool)>,
    depth: usize,
    heavy: bool,
}

impl SearchScan {
    pub(crate) fn new(root: &Path, query: &str, show_hidden: bool) -> Self {
        Self::with_limits(root, query, show_hidden, ScanLimits::default())
    }

    pub(crate) fn with_limits(
        root: &Path,
        query: &str,
        show_hidden: bool,
        limits: ScanLimits,
    ) -> Self {
        let query_folded = fold_chars(query);
        let mut scan = Self {
            query: query.to_string(),
            query_folded,
            show_hidden,
            limits,
            queue: VecDeque::new(),
            deferred: VecDeque::new(),
            active: None,
            matches: Vec::new(),
            visited: 0,
            truncated: false,
        };
        if !scan.query_folded.is_empty() {
            scan.queue.push_back((root.to_path_buf(), 0));
        }
        scan
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// Complete up to `dir_budget` directories, yielding after a time slice.
    /// Individual blocking OS calls cannot be interrupted by this budget.
    ///
    /// Returns true when the walk is finished (the tree is complete, possibly
    /// incomplete because of limits or read errors).
    pub(crate) fn pump(&mut self, dir_budget: usize) -> bool {
        let started = Instant::now();
        self.pump_until(dir_budget, || started.elapsed() >= SEARCH_SLICE_TIME)
    }

    fn pump_until(&mut self, dir_budget: usize, mut should_yield: impl FnMut() -> bool) -> bool {
        let mut completed = 0;
        while completed < dir_budget && !self.finished() {
            if should_yield() {
                return false;
            }
            if self.active.is_none() {
                if self.visited >= self.limits.max_dirs
                    || self.matches.len() >= self.limits.max_matches
                {
                    self.stop();
                    return true;
                }
                let Some((path, depth, heavy)) = self.pop() else {
                    return true;
                };
                self.visited += 1;
                match fs::read_dir(path) {
                    Ok(entries) => {
                        self.active = Some(DirectoryRead {
                            entries: Some(entries),
                            children: BTreeMap::new(),
                            depth,
                            heavy,
                        })
                    }
                    Err(_) => {
                        self.truncated = true;
                        completed += 1;
                    }
                }
            } else if self.visit_step() {
                completed += 1;
            }
        }
        self.finished()
    }

    fn finished(&self) -> bool {
        self.active.is_none() && self.queue.is_empty() && self.deferred.is_empty()
    }

    fn stop(&mut self) {
        self.truncated = true;
        self.active = None;
        self.queue.clear();
        self.deferred.clear();
    }

    pub(crate) fn into_result(self) -> (Vec<PathBuf>, bool) {
        (self.matches, self.truncated)
    }

    pub(crate) fn matches(&self) -> &[PathBuf] {
        &self.matches
    }

    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }

    fn pop(&mut self) -> Option<(PathBuf, usize, bool)> {
        if let Some((path, depth)) = self.queue.pop_front() {
            Some((path, depth, false))
        } else {
            self.deferred
                .pop_front()
                .map(|(path, depth)| (path, depth, true))
        }
    }

    /// Read or process one entry, allowing a yield even inside a directory.
    /// Returns true once the current directory is complete.
    fn visit_step(&mut self) -> bool {
        let active = self.active.as_mut().expect("directory is open");
        if let Some(entries) = &mut active.entries {
            let entry = match entries.next() {
                Some(Ok(entry)) => entry,
                Some(Err(_)) => {
                    self.truncated = true;
                    return false;
                }
                None => {
                    active.entries = None;
                    return false;
                }
            };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                self.truncated = true;
                return false;
            };
            if name.is_empty() || (!self.show_hidden && name.starts_with('.')) {
                return false;
            }
            let Ok(file_type) = entry.file_type() else {
                self.truncated = true;
                return false;
            };
            let child = entry.path();
            let is_symlink = file_type.is_symlink();
            let is_dir = if is_symlink {
                child.is_dir()
            } else {
                file_type.is_dir()
            };
            if is_dir {
                active.children.insert(name, (child, is_symlink));
            }
            return false;
        }
        let Some((name, (child, is_symlink))) = active.children.pop_first() else {
            self.active = None;
            return true;
        };
        let child_depth = active.depth + 1;
        let heavy = active.heavy;
        if self.record_match(&name, &child) {
            return true;
        }
        if !is_symlink {
            if child_depth > self.limits.max_depth {
                self.truncated = true;
                return false;
            }
            if heavy || is_heavy_dir(&name) {
                self.deferred.push_back((child, child_depth));
            } else {
                self.queue.push_back((child, child_depth));
            }
        }
        false
    }

    /// Record `path` when `name` matches. Returns true when the match cap is hit.
    fn record_match(&mut self, name: &str, path: &Path) -> bool {
        if !contains_substring(name, &self.query_folded) {
            return false;
        }
        self.matches.push(path.to_path_buf());
        if self.matches.len() >= self.limits.max_matches {
            self.stop();
            return true;
        }
        false
    }
}

fn is_heavy_dir(name: &str) -> bool {
    HEAVY_DIR_NAMES.contains(&name)
}

/// Contiguous, case-insensitive occurrence of `query` inside `name`.
///
/// The characters have to sit next to each other. `abc` matches `XabcY` and
/// does not match `ZBXC`. The returned ranges are UTF-8 byte spans of the
/// original `name`. An empty query matches nothing.
pub(crate) fn fuzzy_match_ranges(name: &str, query: &str) -> Option<Vec<(usize, usize)>> {
    if query.is_empty() {
        return None;
    }
    let (name_folded, name_map) = fold_with_orig_ranges(name);
    let query_folded = fold_chars(query);
    let (_, start) = best_substring(&name_folded, &query_folded)?;
    let end = start + query_folded.len();
    let ranges = (start..end).map(|index| name_map[index]).collect();
    Some(merge_ranges(ranges))
}

/// Higher scores are clearer matches: the same text at the start of the name,
/// or just after `_`, `-`, `.`, or a space, beats a later copy.
/// `None` when `query` is not a contiguous substring.
pub(crate) fn fuzzy_match_score(name: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return None;
    }
    let (name_folded, _) = fold_with_orig_ranges(name);
    let query_folded = fold_chars(query);
    best_substring(&name_folded, &query_folded).map(|(score, _)| score)
}

/// Best contiguous alignment. The returned index is into `name`.
fn best_substring(name: &[char], query: &[char]) -> Option<(i32, usize)> {
    let m = query.len();
    if m == 0 || name.len() < m {
        return None;
    }
    let mut best: Option<(i32, usize)> = None;
    for start in 0..=name.len() - m {
        if &name[start..start + m] != query {
            continue;
        }
        let mut score = -(start as i32);
        if start == 0 {
            score += START_BONUS;
        } else if is_boundary(name[start - 1]) {
            score += BOUNDARY_BONUS;
        }
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, start));
        }
    }
    best
}

fn contains_substring(name: &str, query: &[char]) -> bool {
    if query.is_empty() {
        return false;
    }
    let folded = fold_chars(name);
    folded.windows(query.len()).any(|window| window == query)
}

fn fold_chars(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_uppercase).collect()
}

/// Uppercase `s` and, for each produced character, the original byte range it came from.
fn fold_with_orig_ranges(s: &str) -> (Vec<char>, Vec<(usize, usize)>) {
    let mut folded = Vec::new();
    let mut map = Vec::new();
    for (byte_idx, ch) in s.char_indices() {
        let byte_end = byte_idx + ch.len_utf8();
        for uc in ch.to_uppercase() {
            folded.push(uc);
            map.push((byte_idx, byte_end));
        }
    }
    (folded, map)
}

fn is_boundary(ch: char) -> bool {
    matches!(ch, '_' | '-' | '.' | ' ')
}

fn merge_ranges(ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = out.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
            continue;
        }
        out.push((start, end));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            path.push(format!("{prefix}_{}_{nanos}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn create_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    fn create_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"test").unwrap();
    }

    fn search_all(root: &Path, query: &str, show_hidden: bool) -> (Vec<PathBuf>, bool) {
        search_limited(root, query, show_hidden, ScanLimits::default())
    }

    fn search_limited(
        root: &Path,
        query: &str,
        show_hidden: bool,
        limits: ScanLimits,
    ) -> (Vec<PathBuf>, bool) {
        let mut scan = SearchScan::with_limits(root, query, show_hidden, limits);
        for _ in 0..10_000 {
            if scan.pump(DIRS_PER_SLICE) {
                return scan.into_result();
            }
        }
        panic!("search did not finish");
    }

    #[test]
    fn fuzzy_match_rejects_empty_and_impossible_queries() {
        assert!(fuzzy_match_ranges("ReadMe.md", "").is_none());
        assert!(fuzzy_match_ranges("foo", "bar").is_none());
        assert!(fuzzy_match_ranges("ab", "abcdef").is_none());
    }

    #[test]
    fn fuzzy_match_finds_partial_substrings_case_insensitively() {
        assert_eq!(
            fuzzy_match_ranges("ReadMe.md", "readme"),
            Some(vec![(0, 6)])
        );
        assert_eq!(fuzzy_match_ranges("ReadMe.md", "MD"), Some(vec![(7, 9)]));
        assert_eq!(
            fuzzy_match_ranges("alpha-target", "target"),
            Some(vec![(6, 12)])
        );
        assert_eq!(fuzzy_match_ranges("Desktop", "desk"), Some(vec![(0, 4)]));
    }

    #[test]
    fn fuzzy_match_highlights_one_best_occurrence() {
        assert_eq!(fuzzy_match_ranges("FooBar", "o"), Some(vec![(1, 2)]));
        assert_eq!(fuzzy_match_ranges("ababab", "ab"), Some(vec![(0, 2)]));
        // xxabc-abc: the copy after '-' outranks the earlier buried copy.
        assert_eq!(fuzzy_match_ranges("xxabc-abc", "abc"), Some(vec![(6, 9)]));
    }

    #[test]
    fn fuzzy_score_prefers_tighter_matches() {
        let exact = fuzzy_match_score("target", "target").unwrap();
        let partial = fuzzy_match_score("alpha-target", "target").unwrap();
        assert!(exact > partial);
        assert!(fuzzy_match_score("skip", "desk").is_none());
        assert!(fuzzy_match_score("Desktop", "dsk").is_none());
    }

    #[test]
    fn fuzzy_match_requires_contiguous_characters() {
        assert!(fuzzy_match_ranges("Desktop", "dsk").is_none());
        assert!(fuzzy_match_ranges("Desktop", "DSK").is_none());
        assert!(fuzzy_match_ranges("ZBXC", "abc").is_none());
        assert!(fuzzy_match_ranges("ZBXC", "bc").is_none());
        assert_eq!(fuzzy_match_ranges("ZBXC", "zbx"), Some(vec![(0, 3)]));
        assert_eq!(fuzzy_match_ranges("XabcY", "abc"), Some(vec![(1, 4)]));
    }

    #[test]
    fn fuzzy_match_handles_multibyte_and_expanding_case() {
        // フ(0..3) ァ(3..6) イ(6..9) ル(9..12)
        assert_eq!(fuzzy_match_ranges("ファイル", "ァイ"), Some(vec![(3, 9)]));
        assert!(fuzzy_match_ranges("ファイル", "file").is_none());
        assert_eq!(
            fuzzy_match_ranges("Straße", "STRASSE"),
            Some(vec![(0, "Straße".len())])
        );
        assert_eq!(fuzzy_match_ranges("Straße", "ss"), Some(vec![(4, 6)]));
    }

    #[test]
    fn search_finds_contiguous_directory_names_only() {
        let temp = TempDir::new("cdtree_fuzzy_names");
        let root = temp.path.join("root");
        create_dir(&root.join("alpha-target"));
        create_dir(&root.join("Desktop"));
        create_dir(&root.join("ZBXC"));
        create_dir(&root.join("other"));
        create_file(&root.join("notes-target.txt"));

        let (matches, truncated) = search_all(&root, "target", true);
        assert!(!truncated);
        assert_eq!(matches, vec![root.join("alpha-target")]);

        let (matches, _) = search_all(&root, "desk", true);
        assert_eq!(matches, vec![root.join("Desktop")]);

        let (matches, _) = search_all(&root, "dsk", true);
        assert!(matches.is_empty());
        let (matches, _) = search_all(&root, "abc", true);
        assert!(matches.is_empty());
        let (matches, _) = search_all(&root, "zbx", true);
        assert_eq!(matches, vec![root.join("ZBXC")]);

        let (matches, _) = search_all(&root, "zzz", true);
        assert!(matches.is_empty());
    }

    #[test]
    fn search_skips_hidden_directories_until_enabled() {
        let temp = TempDir::new("cdtree_fuzzy_hidden");
        let root = temp.path.join("root");
        create_dir(&root.join(".secret").join("hidden-target"));
        create_dir(&root.join("visible").join("hidden-target"));

        let (matches, _) = search_all(&root, "hidden", false);
        assert_eq!(matches, vec![root.join("visible").join("hidden-target")]);

        let (matches, _) = search_all(&root, "hidden", true);
        assert_eq!(matches.len(), 2);
        assert!(
            matches
                .iter()
                .any(|path| path == &root.join(".secret").join("hidden-target"))
        );
    }

    #[test]
    fn search_finds_nested_names_and_skips_unrelated_branches() {
        let temp = TempDir::new("cdtree_fuzzy_nested");
        let root = temp.path.join("root");
        create_dir(&root.join("keep").join("alpha-target"));
        create_dir(&root.join("keep").join("other"));
        create_dir(&root.join("skip").join("nope"));

        let (matches, truncated) = search_all(&root, "target", true);
        assert!(!truncated);
        assert_eq!(matches, vec![root.join("keep").join("alpha-target")]);
    }

    #[test]
    fn search_opens_ordinary_directories_before_heavy_ones() {
        let temp = TempDir::new("cdtree_fuzzy_heavy");
        let root = temp.path.join("root");
        create_dir(&root.join("Library").join("deep").join("needle"));
        create_dir(&root.join("src").join("needle"));

        // root + src fit in the budget and src's child is recorded on sight.
        // Library is deferred, so a non-deferred walk would open Library first
        // and stop before src.
        let limits = ScanLimits {
            max_dirs: 2,
            max_matches: 10,
            max_depth: 8,
        };
        let (matches, truncated) = search_limited(&root, "needle", true, limits);
        assert!(truncated);
        assert_eq!(matches, vec![root.join("src").join("needle")]);
    }

    #[test]
    fn large_directory_read_yields_and_resumes_without_losing_matches() {
        let temp = TempDir::new("cdtree_search_slice");
        for i in 0..64 {
            create_dir(&temp.path.join(format!("needle-{i:02}")));
        }
        let mut scan = SearchScan::new(&temp.path, "needle", true);
        let mut steps = 0;
        assert!(!scan.pump_until(400, || {
            steps += 1;
            steps > 12
        }));
        assert!(scan.active.as_ref().unwrap().entries.is_some());
        assert!(!scan.is_truncated());

        // Yield while processing the sorted children, too.
        while scan.active.as_ref().unwrap().entries.is_some() {
            let mut steps = 0;
            assert!(!scan.pump_until(400, || {
                steps += 1;
                steps > 1
            }));
        }
        let mut steps = 0;
        assert!(!scan.pump_until(400, || {
            steps += 1;
            steps > 5
        }));
        assert_eq!(scan.matches().len(), 5);
        assert!(!scan.pump_until(400, || true));
        assert_eq!(scan.matches().len(), 5);
        assert!(scan.pump_until(400, || false));
        let (matches, partial) = scan.into_result();
        assert!(!partial);
        let expected: Vec<_> = (0..64)
            .map(|i| temp.path.join(format!("needle-{i:02}")))
            .collect();
        assert_eq!(matches, expected);
    }

    #[test]
    fn depth_omissions_are_partial_but_other_branches_still_get_searched() {
        let temp = TempDir::new("cdtree_search_depth");
        create_dir(&temp.path.join("a/deep/needle"));
        create_dir(&temp.path.join("z/needle"));
        let (matches, partial) = search_limited(
            &temp.path,
            "needle",
            true,
            ScanLimits {
                max_depth: 1,
                ..ScanLimits::default()
            },
        );
        assert!(partial);
        assert_eq!(matches, vec![temp.path.join("z/needle")]);
    }

    #[test]
    fn zero_hits_at_directory_or_depth_limit_are_partial() {
        let temp = TempDir::new("cdtree_search_limits");
        create_dir(&temp.path.join("a/deep/needle"));
        for limits in [
            ScanLimits {
                max_dirs: 1,
                ..ScanLimits::default()
            },
            ScanLimits {
                max_depth: 0,
                ..ScanLimits::default()
            },
        ] {
            let (matches, partial) = search_limited(&temp.path, "needle", true, limits);
            assert!(matches.is_empty());
            assert!(partial);
        }
    }

    #[test]
    fn finishing_exactly_at_directory_limit_is_complete() {
        let temp = TempDir::new("cdtree_search_exact_limit");
        let (_, partial) = search_limited(
            &temp.path,
            "needle",
            true,
            ScanLimits {
                max_dirs: 1,
                ..ScanLimits::default()
            },
        );
        assert!(!partial);
    }

    #[test]
    fn match_limit_stops_a_resumable_directory() {
        let temp = TempDir::new("cdtree_search_match_limit");
        for name in ["needle-a", "needle-b", "needle-c"] {
            create_dir(&temp.path.join(name));
        }
        let (matches, partial) = search_limited(
            &temp.path,
            "needle",
            true,
            ScanLimits {
                max_matches: 2,
                ..ScanLimits::default()
            },
        );
        assert!(partial);
        assert_eq!(
            matches,
            vec![temp.path.join("needle-a"), temp.path.join("needle-b")]
        );
    }

    #[test]
    fn unreadable_branch_is_partial_and_does_not_stop_other_branches() {
        let temp = TempDir::new("cdtree_search_missing");
        create_dir(&temp.path.join("a_missing"));
        create_dir(&temp.path.join("b_keep/needle"));
        let mut scan = SearchScan::new(&temp.path, "needle", true);
        assert!(!scan.pump_until(1, || false));
        fs::remove_dir(temp.path.join("a_missing")).unwrap();
        assert!(scan.pump_until(400, || false));
        let (matches, partial) = scan.into_result();
        assert!(partial);
        assert_eq!(matches, vec![temp.path.join("b_keep/needle")]);
    }

    #[cfg(unix)]
    #[test]
    fn search_does_not_descend_into_symlinks() {
        let temp = TempDir::new("cdtree_fuzzy_link");
        let root = temp.path.join("root");
        let outside = temp.path.join("outside");
        create_dir(&outside.join("loop-target"));
        create_dir(&root);
        std::os::unix::fs::symlink(&outside, root.join("link-dir")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("self-link")).unwrap();

        let (matches, truncated) = search_all(&root, "loop", true);
        assert!(!truncated);
        assert!(matches.is_empty());

        let (matches, truncated) = search_all(&root, "link", true);
        assert!(!truncated);
        let mut matches = matches;
        matches.sort();
        assert_eq!(matches, vec![root.join("link-dir"), root.join("self-link")]);
    }
}
