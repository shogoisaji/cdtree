use crate::config::{Config, RgbColor, Theme};
use crate::history::History;
use crate::search::{
    DIRS_PER_SLICE, SEARCH_DEBOUNCE, SearchPhase, SearchScan, fuzzy_match_ranges, fuzzy_match_score,
};
use rand::Rng;
use ratatui::widgets::ListState;
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Cd,
    Open,
    Code,
}

impl AppMode {
    pub fn suffix(&self) -> &str {
        match self {
            AppMode::Cd => "🌲CD🌲",
            AppMode::Open => "🌲OPEN🌲",
            AppMode::Code => "🌲CODE🌲",
        }
    }

    pub fn toggle(&mut self) {
        *self = match self {
            AppMode::Cd => AppMode::Open,
            AppMode::Open => AppMode::Code,
            AppMode::Code => AppMode::Cd,
        };
    }
}

#[derive(Debug, Clone)]
pub struct FileNode {
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Option<Vec<FileNode>>, // None if not loaded or not a dir
    pub expanded: bool,
    pub cached_counts: Option<(usize, usize)>, // (dir_count, file_count) cached for display
    pub child_count_attempted: bool,
}

impl FileNode {
    pub fn new(path: PathBuf, is_dir: bool) -> Self {
        Self {
            path,
            is_dir,
            children: None,
            expanded: false,
            cached_counts: None,
            child_count_attempted: false,
        }
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }

    /// Returns cached (dir_count, file_count) of direct children.
    /// Returns None if counts haven't been determined yet.
    pub fn child_counts(&self) -> Option<(usize, usize)> {
        if let Some(children) = &self.children {
            let counts = children.iter().fold((0, 0), |(dirs, files), child| {
                if child.is_dir {
                    (dirs + 1, files)
                } else {
                    (dirs, files + 1)
                }
            });
            Some(counts)
        } else {
            self.cached_counts
        }
    }

    /// Lightweight count of children without building FileNode structs.
    /// Only reads directory entries and counts them, caching the result.
    pub fn load_child_counts(&mut self, show_files: bool, show_hidden: bool) {
        if !self.is_dir || self.children.is_some() || self.child_count_attempted {
            return;
        }

        self.child_count_attempted = true;

        let Ok(entries) = fs::read_dir(&self.path) else {
            return;
        };
        let (dirs, files) = entries
            .filter_map(|e| e.ok())
            .fold((0, 0), |(d, f), entry| {
                let path = entry.path();
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let is_hidden = file_name.starts_with('.');

                if !show_hidden && is_hidden {
                    return (d, f);
                }

                let Ok(file_type) = entry.file_type() else {
                    return (d, f);
                };
                let is_dir = if file_type.is_symlink() {
                    path.is_dir()
                } else {
                    file_type.is_dir()
                };

                if !show_files && !is_dir {
                    return (d, f);
                }

                if is_dir { (d + 1, f) } else { (d, f + 1) }
            });

        self.cached_counts = Some((dirs, files));
    }

    pub fn load_children(&mut self, show_files: bool, show_hidden: bool) -> io::Result<()> {
        if !self.is_dir {
            return Ok(());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let is_dir = if file_type.is_symlink() {
                path.is_dir()
            } else {
                file_type.is_dir()
            };

            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let is_hidden = file_name.starts_with('.');

            if !show_hidden && is_hidden {
                continue;
            }

            if !show_files && !is_dir {
                continue;
            }

            entries.push(FileNode::new(path, is_dir));
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.path.file_name().cmp(&b.path.file_name()),
        });

        let (dirs, files) =
            entries.iter().fold(
                (0, 0),
                |(d, f), e| {
                    if e.is_dir { (d + 1, f) } else { (d, f + 1) }
                },
            );
        self.cached_counts = Some((dirs, files));
        self.child_count_attempted = true;

        self.children = Some(entries);
        Ok(())
    }
}

pub struct App {
    pub root: FileNode,
    pub selected_path: PathBuf,
    pub startup_path: PathBuf,
    pub show_files: bool,
    pub show_hidden: bool,
    pub list_state: ListState,
    pub config: Config,
    pub current_theme: Theme,
    pub last_theme_change: Option<Instant>,
    pub mode: AppMode,
    pub history_mode: bool,
    pub history: History,
    pub history_list_state: ListState,
    pub home_dir: PathBuf,
    pub search_mode: bool,
    pub search_query: String,
    /// Query the current `search_tree` was built for.
    search_applied: String,
    /// Whether the published snapshot finished its walk (possibly with omissions).
    search_applied_complete: bool,
    /// Expanded tree of directory hits and their ancestors. The live tree is
    /// left untouched so leaving search restores it.
    search_tree: Option<FileNode>,
    search_phase: SearchPhase,
    search_truncated: bool,
    /// While this is set, newly found hits may move the cursor to a clearer
    /// match. Arrow keys, Enter's neighbors, and scrolling turn it off.
    search_follow_best: bool,
}

impl App {
    pub fn new() -> io::Result<Self> {
        let home_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        let current_dir = std::env::current_dir()?;
        let config = Config::load().unwrap_or_default();
        let show_files = config.show_files;
        let show_hidden = config.show_hidden;
        let history = History::load().unwrap_or_default();

        let mut root = FileNode::new(home_dir.clone(), true);
        root.load_children(show_files, show_hidden)?;
        root.expanded = true;

        let selected_path = if current_dir.starts_with(&home_dir) {
            current_dir.clone()
        } else {
            home_dir.clone()
        };

        let mut app = Self {
            root,
            selected_path,
            startup_path: current_dir.clone(),
            show_files,
            show_hidden,
            list_state: ListState::default(),
            config,
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history,
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: home_dir.clone(),
        };

        // Set initial theme from config
        app.current_theme = app.config.theme.clone();

        // Expand tree to current directory
        app.expand_to_path(&current_dir);

        // Ensure valid selection even when starting outside the home directory
        app.ensure_valid_selection();
        app.update_list_state();
        if let Some(pos) = app
            .get_visible_nodes()
            .iter()
            .position(|(_, node)| node.path == app.selected_path)
        {
            // Scroll the selected item to the top on startup
            *app.list_state.offset_mut() = pos;
        }

        Ok(app)
    }

    pub fn expand_to_path(&mut self, target: &Path) {
        if !target.starts_with(self.root.path.as_path()) {
            return;
        }
        let show_files = self.show_files;
        let show_hidden = self.show_hidden;

        let mut current_path = self.root.path.clone();
        let components: Vec<_> = target
            .strip_prefix(self.root.path.as_path())
            .unwrap()
            .components()
            .collect();

        for component in components {
            current_path.push(component);
            let path_to_expand = current_path.clone();
            Self::find_and_modify(&mut self.root, path_to_expand.as_path(), |node| {
                Self::expand_dir(node, show_files, show_hidden);
            });
        }
    }

    // Returns (prefix, node_reference)
    pub fn get_visible_nodes(&self) -> Vec<(String, &FileNode)> {
        let root = self.view_root();
        let mut result = Vec::new();
        let mut is_last_stack = Vec::new();
        result.push(("".to_string(), root));

        if root.expanded {
            if let Some(children) = &root.children {
                let count = children.len();
                for (i, child) in children.iter().enumerate() {
                    is_last_stack.push(i == count - 1);
                    Self::collect_visible_nodes(child, &mut is_last_stack, &mut result);
                    is_last_stack.pop();
                }
            }
        }
        result
    }

    /// Filtered search tree while a query is applied, otherwise the live tree.
    fn view_root(&self) -> &FileNode {
        self.active_search_tree().unwrap_or(&self.root)
    }

    fn active_search_tree(&self) -> Option<&FileNode> {
        if self.search_view_active() {
            self.search_tree.as_ref()
        } else {
            None
        }
    }

    /// True while search results are on screen.
    ///
    /// Stays true between keystrokes so the previous tree remains until the
    /// debounced walk finishes and replaces it.
    pub fn search_view_active(&self) -> bool {
        self.search_mode && self.search_tree.is_some() && !self.search_applied.is_empty()
    }

    /// Query whose matches should be highlighted. Follows the keys immediately,
    /// so the visible names respond before the debounced walk replaces the tree.
    pub fn search_highlight_query(&self) -> &str {
        if self.search_mode {
            self.search_query.as_str()
        } else {
            ""
        }
    }

    pub fn search_busy(&self) -> bool {
        matches!(self.search_phase, SearchPhase::Scanning(_))
    }

    /// Short status for the search guide. Empty when there is nothing to add.
    ///
    /// A count is shown only for the tree that matches the current query, so a
    /// half-typed query does not keep the previous total.
    pub fn search_status(&self) -> String {
        if !self.search_mode {
            return String::new();
        }
        if self.search_view_active() && self.search_query == self.search_applied {
            let hits = self
                .search_tree
                .as_ref()
                .map(|tree| count_search_hits(tree, &self.search_applied))
                .unwrap_or(0);
            if hits == 0 {
                if self.search_busy() || !self.search_applied_complete {
                    return "Searching...".to_string();
                }
                return if self.search_truncated {
                    "No matches (partial search)".to_string()
                } else {
                    "No matches".to_string()
                };
            }
            let mut label = if hits == 1 {
                "1 match".to_string()
            } else {
                format!("{hits} matches")
            };
            if self.search_truncated {
                label.push_str(", partial");
            }
            if self.search_busy() || !self.search_applied_complete {
                // More directories are still being opened, so the count can grow.
                label.push_str("...");
            }
            return label;
        }
        if self.search_busy() {
            return "Searching...".to_string();
        }
        String::new()
    }

    /// Idle wait before the next event. Shortens while a search is pending so
    /// the walk starts after typing pauses and then advances in slices.
    pub fn search_poll_timeout(&self) -> Duration {
        const IDLE: Duration = Duration::from_millis(250);
        match &self.search_phase {
            SearchPhase::Waiting { since } => {
                let elapsed = since.elapsed();
                if elapsed >= SEARCH_DEBOUNCE {
                    Duration::from_millis(1)
                } else {
                    (SEARCH_DEBOUNCE - elapsed).max(Duration::from_millis(1))
                }
            }
            SearchPhase::Scanning(_) => Duration::from_millis(16),
            SearchPhase::Idle => IDLE,
        }
    }

    fn collect_visible_nodes<'a>(
        node: &'a FileNode,
        is_last_stack: &mut Vec<bool>,
        result: &mut Vec<(String, &'a FileNode)>,
    ) {
        let prefix = Self::build_prefix(is_last_stack);
        result.push((prefix, node));

        if node.expanded {
            if let Some(children) = &node.children {
                let count = children.len();
                for (i, child) in children.iter().enumerate() {
                    is_last_stack.push(i == count - 1);
                    Self::collect_visible_nodes(child, is_last_stack, result);
                    is_last_stack.pop();
                }
            }
        }
    }

    /// Pre-load child counts for visible directory rows that haven't been counted yet.
    /// Uses lightweight `fs::read_dir` counting (no FileNode creation) and caches results.
    pub fn ensure_visible_child_counts(&mut self, first_row: usize, row_count: usize) {
        if self.search_view_active() {
            return;
        }
        let show_files = self.show_files;
        let show_hidden = self.show_hidden;
        let paths: Vec<PathBuf> = self
            .get_visible_nodes()
            .into_iter()
            .skip(first_row)
            .take(row_count)
            .filter(|(_, node)| {
                node.is_dir && node.children.is_none() && !node.child_count_attempted
            })
            .map(|(_, node)| node.path.clone())
            .collect();

        for path in paths {
            Self::find_and_modify(&mut self.root, path.as_path(), |node| {
                node.load_child_counts(show_files, show_hidden);
            });
        }
    }

    fn build_prefix(is_last_stack: &[bool]) -> String {
        let mut prefix = String::new();
        for (i, &is_last) in is_last_stack.iter().enumerate() {
            if i == is_last_stack.len() - 1 {
                if is_last {
                    prefix.push_str("└─ ");
                } else {
                    prefix.push_str("├─ ");
                }
            } else if is_last {
                prefix.push_str("   ");
            } else {
                prefix.push_str("│  ");
            }
        }
        prefix
    }

    pub fn move_selection(&mut self, delta: i32) {
        self.note_search_navigation();
        let visible = self.get_visible_nodes();
        if visible.is_empty() {
            return;
        }

        let current_idx = visible
            .iter()
            .position(|(_, node)| node.path == self.selected_path);

        if let Some(idx) = current_idx {
            let max_idx = (visible.len() - 1) as i32;
            let new_idx = (idx as i32 + delta).clamp(0, max_idx) as usize;
            self.selected_path = visible[new_idx].1.path.clone();
            self.update_list_state();
        }
    }

    pub fn update_list_state(&mut self) {
        let visible = self.get_visible_nodes();
        if let Some(pos) = visible
            .iter()
            .position(|(_, node)| node.path == self.selected_path)
        {
            self.list_state.select(Some(pos));
        } else {
            self.list_state.select(None);
        }
    }

    pub fn expand_current(&mut self) {
        self.note_search_navigation();
        if self.search_view_active() {
            self.select_search_child();
            return;
        }
        let show_files = self.show_files;
        let show_hidden = self.show_hidden;
        Self::find_and_modify(&mut self.root, self.selected_path.as_path(), |node| {
            Self::expand_dir(node, show_files, show_hidden);
        });
        self.update_list_state();
    }

    /// Toggle expansion of the currently selected node.
    /// Collapses expanded directories and expands collapsed ones; files are a no-op.
    pub fn toggle_current(&mut self) {
        if self.search_view_active() {
            return;
        }
        let show_files = self.show_files;
        let show_hidden = self.show_hidden;
        let selected = self.selected_path.clone();
        Self::find_and_modify(&mut self.root, selected.as_path(), |node| {
            if !node.is_dir {
                return;
            }
            if node.expanded {
                node.expanded = false;
            } else {
                Self::expand_dir(node, show_files, show_hidden);
            }
        });
        self.update_list_state();
    }

    /// Select the visible node at the given flat index (as used by the List widget).
    /// Out-of-bounds indices are ignored.
    pub fn select_visible_index(&mut self, idx: usize) {
        self.note_search_navigation();
        let visible = self.get_visible_nodes();
        if let Some((_, node)) = visible.get(idx) {
            self.selected_path = node.path.clone();
            // We already know the flat index, so set it directly instead of
            // re-walking the tree via `update_list_state`.
            self.list_state.select(Some(idx));
        }
    }

    /// Scroll the tree viewport by `delta` rows without changing the selection.
    /// The offset is clamped to >= 0; ratatui clamps the upper bound at render time.
    pub fn scroll(&mut self, delta: i32) {
        self.note_search_navigation();
        let current = self.list_state.offset() as i32;
        let new = (current + delta).max(0) as usize;
        *self.list_state.offset_mut() = new;
    }

    /// Scroll the history viewport by `delta` rows without changing the selection.
    pub fn scroll_history(&mut self, delta: i32) {
        let current = self.history_list_state.offset() as i32;
        let new = (current + delta).max(0) as usize;
        *self.history_list_state.offset_mut() = new;
    }

    fn expand_dir(node: &mut FileNode, show_files: bool, show_hidden: bool) {
        if node.is_dir {
            if !node.expanded {
                node.expanded = true;
            }
            if node.children.is_none() {
                if node.load_children(show_files, show_hidden).is_err() {
                    node.expanded = false;
                }
            }
        }
    }

    // Helper to traverse and mutate
    fn find_and_modify<F>(node: &mut FileNode, target: &Path, f: F) -> bool
    where
        F: Fn(&mut FileNode) + Copy,
    {
        if node.path.as_path() == target {
            f(node);
            return true;
        }
        if let Some(children) = &mut node.children {
            for child in children {
                if Self::find_and_modify(child, target, f) {
                    return true;
                }
            }
        }
        false
    }

    pub fn is_selected_dir(&self) -> bool {
        if let Some(tree) = self.active_search_tree() {
            return Self::find_node(tree, self.selected_path.as_path()).is_some();
        }
        Self::find_node(&self.root, self.selected_path.as_path())
            .map(|node| node.is_dir)
            .unwrap_or(false)
    }

    fn find_node<'a>(node: &'a FileNode, target: &Path) -> Option<&'a FileNode> {
        if node.path.as_path() == target {
            return Some(node);
        }
        if let Some(children) = &node.children {
            for child in children {
                if let Some(found) = Self::find_node(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    pub fn on_left(&mut self) {
        self.note_search_navigation();
        if self.search_view_active() {
            self.select_search_parent();
            return;
        }
        let selected = self.selected_path.clone();
        let parent_path = Self::find_parent_path(&self.root, selected.as_path());
        if let Some(parent) = parent_path {
            // Move to parent immediately
            self.selected_path = parent;
            // Also collapse the previously selected node if it was a directory
            Self::find_and_modify(&mut self.root, selected.as_path(), |node| {
                if node.is_dir {
                    node.expanded = false;
                }
            });
        }

        self.update_list_state();
    }

    fn find_parent_path(node: &FileNode, target: &Path) -> Option<PathBuf> {
        fn walk(node: &FileNode, target: &Path, parent: Option<&Path>) -> Option<PathBuf> {
            if node.path.as_path() == target {
                return parent.map(Path::to_path_buf);
            }
            if let Some(children) = &node.children {
                for child in children {
                    if let Some(found) = walk(child, target, Some(node.path.as_path())) {
                        return Some(found);
                    }
                }
            }
            None
        }

        walk(node, target, None)
    }

    pub fn toggle_show_files(&mut self) {
        self.show_files = !self.show_files;
        self.config.show_files = self.show_files;
        let _ = self.config.save();
        self.refresh_after_toggle();
    }

    pub fn toggle_show_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.config.show_hidden = self.show_hidden;
        let _ = self.config.save();
        self.refresh_after_toggle();
    }

    /// Enter directory search with an empty query. The live tree stays up until
    /// a query has been idle long enough to search.
    pub fn start_search(&mut self) {
        self.search_mode = true;
        self.search_query.clear();
        self.search_applied.clear();
        self.search_applied_complete = false;
        self.search_tree = None;
        self.search_truncated = false;
        self.search_phase = SearchPhase::Idle;
        self.search_follow_best = true;
    }

    pub fn exit_search(&mut self) {
        self.search_mode = false;
        self.search_query.clear();
        self.dismiss_search_view();
    }

    pub fn search_input(&mut self, c: char) {
        self.search_query.push(c);
        self.schedule_search();
    }

    pub fn search_backspace(&mut self) {
        if self.search_query.pop().is_none() {
            return;
        }
        if self.search_query.is_empty() {
            self.dismiss_search_view();
        } else {
            self.schedule_search();
        }
    }

    /// Start or continue a directory walk once typing has been idle long enough.
    pub fn poll_search(&mut self) {
        self.poll_search_at(Instant::now());
    }

    fn poll_search_at(&mut self, now: Instant) {
        self.poll_search_budget(now, DIRS_PER_SLICE);
    }

    fn poll_search_budget(&mut self, now: Instant, budget: usize) {
        if !self.search_mode || self.search_query.is_empty() {
            return;
        }
        let start = match &self.search_phase {
            SearchPhase::Scanning(_) => true,
            SearchPhase::Waiting { since } => {
                now.saturating_duration_since(*since) >= SEARCH_DEBOUNCE
            }
            SearchPhase::Idle => false,
        };
        if !start {
            return;
        }
        if matches!(self.search_phase, SearchPhase::Waiting { .. }) {
            self.begin_scan();
        }
        if matches!(self.search_phase, SearchPhase::Scanning(_)) {
            self.pump_scan_budget(budget);
        }
    }

    fn schedule_search(&mut self) {
        if !self.search_query.is_empty()
            && self.search_query == self.search_applied
            && self.search_applied_complete
            && self.search_tree.is_some()
        {
            self.search_phase = SearchPhase::Idle;
            return;
        }
        self.search_phase = SearchPhase::Waiting {
            since: Instant::now(),
        };
    }

    fn begin_scan(&mut self) {
        let scan = SearchScan::new(
            self.root.path.as_path(),
            &self.search_query,
            self.show_hidden,
        );
        self.search_phase = SearchPhase::Scanning(Box::new(scan));
    }

    fn pump_scan_budget(&mut self, budget: usize) {
        let mut scan = match std::mem::replace(&mut self.search_phase, SearchPhase::Idle) {
            SearchPhase::Scanning(scan) => scan,
            other => {
                self.search_phase = other;
                return;
            }
        };
        if scan.query() != self.search_query {
            self.schedule_search();
            return;
        }
        let matched_before = scan.matches().len();
        let finished = scan.pump(budget);
        if scan.query() != self.search_query {
            self.schedule_search();
            return;
        }
        if finished {
            let (matches, truncated) = scan.into_result();
            self.publish_search_matches(&matches, truncated, true);
            self.search_phase = SearchPhase::Idle;
            return;
        }
        // Show hits as soon as they exist. The rest of the walk keeps running.
        if scan.matches().len() != matched_before && !scan.matches().is_empty() {
            let matches = scan.matches().to_vec();
            let truncated = scan.is_truncated();
            self.publish_search_matches(&matches, truncated, false);
        }
        self.search_phase = SearchPhase::Scanning(scan);
    }

    fn publish_search_matches(&mut self, matches: &[PathBuf], truncated: bool, complete: bool) {
        // Keep a user-chosen row visible until the new walk reaches its branch
        // or finishes and confirms that the row is no longer in the results.
        if !complete
            && !self.search_follow_best
            && !matches
                .iter()
                .any(|path| path.starts_with(&self.selected_path))
        {
            return;
        }
        self.search_tree = Some(build_match_tree(self.root.path.as_path(), matches));
        self.search_applied = self.search_query.clone();
        self.search_applied_complete = complete;
        self.search_truncated = truncated;
        self.focus_published_results();
    }

    /// The cursor follows the clearest hit until the user moves it. A row they
    /// chose stays put while later hits arrive, and is given up only when that
    /// row is no longer in the result tree.
    fn focus_published_results(&mut self) {
        let query = self.search_applied.clone();
        let selected = self.selected_path.clone();
        let held = !self.search_follow_best
            && self
                .search_tree
                .as_ref()
                .is_some_and(|tree| Self::find_node(tree, selected.as_path()).is_some());
        if held {
            self.update_list_state();
            return;
        }
        self.search_follow_best = true;
        let Some(next) = self
            .search_tree
            .as_ref()
            .map(|tree| best_hit_path(tree, &query))
        else {
            return;
        };
        let jumped = next != self.selected_path;
        self.selected_path = next;
        self.update_list_state();
        // Open the result from the top. The list widget scrolls only far enough
        // to reveal the selection, which keeps the ancestor path on screen.
        if jumped {
            *self.list_state.offset_mut() = 0;
        }
    }

    fn note_search_navigation(&mut self) {
        if self.search_view_active() {
            self.search_follow_best = false;
        }
    }

    fn select_search_child(&mut self) {
        let selected = self.selected_path.clone();
        let Some(child_path) = self.active_search_tree().and_then(|tree| {
            let node = Self::find_node(tree, selected.as_path())?;
            Some(node.children.as_ref()?.first()?.path.clone())
        }) else {
            return;
        };
        self.selected_path = child_path;
        self.update_list_state();
    }

    fn select_search_parent(&mut self) {
        let selected = self.selected_path.clone();
        let Some(parent) = self
            .active_search_tree()
            .and_then(|tree| Self::find_parent_path(tree, selected.as_path()))
        else {
            return;
        };
        self.selected_path = parent;
        self.update_list_state();
    }

    /// Drop the filtered tree and show the live tree, expanded to the selection.
    fn dismiss_search_view(&mut self) {
        let selected = self.selected_path.clone();
        self.search_applied.clear();
        self.search_applied_complete = false;
        self.search_tree = None;
        self.search_truncated = false;
        self.search_phase = SearchPhase::Idle;
        self.search_follow_best = true;
        if selected != self.root.path && selected.starts_with(&self.root.path) {
            Self::expand_dir(&mut self.root, self.show_files, self.show_hidden);
        }
        self.expand_to_path(selected.as_path());
        self.selected_path = selected;
        self.ensure_valid_selection();
        self.update_list_state();
    }

    #[cfg(test)]
    fn finish_search_for_test(&mut self) {
        for _ in 0..10_000 {
            if self.advance_search_for_test(DIRS_PER_SLICE) {
                return;
            }
        }
        panic!("directory search did not finish");
    }

    /// Run one slice of the walk. Returns true when the search is idle.
    #[cfg(test)]
    fn advance_search_for_test(&mut self, budget: usize) -> bool {
        let at = match &self.search_phase {
            SearchPhase::Idle => return true,
            SearchPhase::Waiting { since } => *since + SEARCH_DEBOUNCE,
            SearchPhase::Scanning(_) => Instant::now(),
        };
        self.poll_search_budget(at, budget);
        matches!(self.search_phase, SearchPhase::Idle)
    }

    fn refresh_after_toggle(&mut self) {
        Self::clear_child_count_cache(&mut self.root);
        self.reload_all();
        let selected = self.selected_path.clone();
        self.expand_to_path(selected.as_path());
        self.ensure_valid_selection();
        self.update_list_state();
    }

    fn ensure_valid_selection(&mut self) {
        let visible = self.get_visible_nodes();
        if visible.is_empty() {
            return;
        }

        // Check if current selection is still visible
        if visible
            .iter()
            .any(|(_, node)| node.path == self.selected_path)
        {
            return;
        }

        // Try to select parent
        if let Some(parent) = self.selected_path.parent() {
            let parent_buf = parent.to_path_buf();
            if visible.iter().any(|(_, node)| node.path == parent_buf) {
                self.selected_path = parent_buf;
                return;
            }
        }

        // Fallback to first visible (root)
        if let Some((_, node)) = visible.first() {
            self.selected_path = node.path.clone();
        }
    }

    fn reload_all(&mut self) {
        let show_files = self.show_files;
        let show_hidden = self.show_hidden;
        Self::traverse_and_reload(&mut self.root, show_files, show_hidden);
    }

    fn clear_child_count_cache(node: &mut FileNode) {
        node.cached_counts = None;
        node.child_count_attempted = false;
        if let Some(children) = &mut node.children {
            for child in children {
                Self::clear_child_count_cache(child);
            }
        }
    }

    fn traverse_and_reload(node: &mut FileNode, show_files: bool, show_hidden: bool) {
        if node.is_dir && (node.expanded || node.children.is_some()) {
            if node.load_children(show_files, show_hidden).is_err() {
                node.expanded = false;
                node.children = None;
                return;
            }
            if let Some(children) = &mut node.children {
                for child in children {
                    Self::traverse_and_reload(child, show_files, show_hidden);
                }
            }
        }
    }

    pub fn change_theme_random(&mut self) {
        let mut rng = rand::rng();
        let new_theme = Theme {
            border_fg: Self::random_color(&mut rng),
            border_style: Self::random_color(&mut rng),
            border_style_soft: Self::random_color(&mut rng),
            key_highlight: Self::random_color(&mut rng),
            branch_color: Self::random_color(&mut rng),
        };
        self.current_theme = new_theme.clone();
        self.config.theme = new_theme;
        let _ = self.config.save();
    }

    pub fn reset_theme_default(&mut self) {
        let default = Theme::default();
        self.current_theme = default.clone();
        self.config.theme = default;
        let _ = self.config.save();
    }

    fn random_color(rng: &mut impl Rng) -> RgbColor {
        RgbColor {
            r: rng.random(),
            g: rng.random(),
            b: rng.random(),
        }
    }
}

/// Tree of `matches` plus the ancestors that connect them to `root`.
/// Every included directory is expanded. Branches with no hit are omitted.
fn build_match_tree(root: &Path, matches: &[PathBuf]) -> FileNode {
    let mut root_node = FileNode::new(root.to_path_buf(), true);
    root_node.expanded = true;
    root_node.children = Some(Vec::new());
    for match_path in matches {
        let Ok(rel) = match_path.strip_prefix(root) else {
            continue;
        };
        let parts: Vec<&OsStr> = rel
            .components()
            .map(|component| component.as_os_str())
            .collect();
        if parts.is_empty() {
            continue;
        }
        insert_match(&mut root_node, &parts);
    }
    sort_search_tree(&mut root_node);
    root_node
}

fn insert_match(node: &mut FileNode, parts: &[&OsStr]) {
    let Some((first, rest)) = parts.split_first() else {
        return;
    };
    let child_path = node.path.join(first);
    if node.children.is_none() {
        node.children = Some(Vec::new());
    }
    let Some(children) = node.children.as_mut() else {
        return;
    };
    if let Some(idx) = children.iter().position(|child| child.path == child_path) {
        children[idx].expanded = true;
        if !rest.is_empty() {
            insert_match(&mut children[idx], rest);
        }
    } else {
        let mut created = FileNode::new(child_path, true);
        created.expanded = true;
        if !rest.is_empty() {
            insert_match(&mut created, rest);
        }
        children.push(created);
    }
}

fn sort_search_tree(node: &mut FileNode) {
    let Some(children) = node.children.as_mut() else {
        return;
    };
    children.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    for child in children.iter_mut() {
        sort_search_tree(child);
    }
}

/// Directory hit to land on. A tighter name match wins, and a shallower path
/// wins when the scores are equal, so the cursor starts on the clearest row.
fn best_hit_path(tree: &FileNode, query: &str) -> PathBuf {
    let mut best: Option<(i32, usize, PathBuf)> = None;
    fn walk(node: &FileNode, depth: usize, query: &str, best: &mut Option<(i32, usize, PathBuf)>) {
        if depth > 0
            && let Some(score) = fuzzy_match_score(&node.name(), query)
        {
            let better = match best {
                None => true,
                Some((best_score, best_depth, _)) => {
                    score > *best_score || (score == *best_score && depth < *best_depth)
                }
            };
            if better {
                *best = Some((score, depth, node.path.clone()));
            }
        }
        if let Some(children) = &node.children {
            for child in children {
                walk(child, depth + 1, query, best);
            }
        }
    }
    walk(tree, 0, query, &mut best);
    best.map(|(_, _, path)| path)
        .unwrap_or_else(|| tree.path.clone())
}

fn count_search_hits(tree: &FileNode, query: &str) -> usize {
    let mut count = 0;
    if let Some(children) = &tree.children {
        for child in children {
            if fuzzy_match_ranges(&child.name(), query).is_some() {
                count += 1;
            }
            count += count_search_hits(child, query);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchPhase;
    use crate::test_support::{CurrentDirGuard, EnvGuard, env_lock};
    use std::fs;
    use std::path::{Path, PathBuf};
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
            path.push(format!("{}_{}_{}", prefix, std::process::id(), nanos));
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
        fs::write(path, b"test").unwrap();
    }

    fn sample_app() -> App {
        let mut root = FileNode::new(PathBuf::from("/root"), true);
        let mut a = FileNode::new(PathBuf::from("/root/a"), true);
        a.expanded = true;
        a.children = Some(vec![
            FileNode::new(PathBuf::from("/root/a/aa"), true),
            FileNode::new(PathBuf::from("/root/a/ab"), false),
        ]);
        let b = FileNode::new(PathBuf::from("/root/b"), true);
        root.expanded = true;
        root.children = Some(vec![a, b]);

        App {
            root,
            selected_path: PathBuf::from("/root"),
            startup_path: PathBuf::from("/root"),
            show_files: true,
            show_hidden: true,
            list_state: ListState::default(),
            config: Config::default(),
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history: History::default(),
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: PathBuf::from("/root"),
        }
    }

    #[test]
    fn load_children_filters_and_sorts() {
        let temp = TempDir::new("cdtree_load_children");
        let root = temp.path.join("root");
        create_dir(&root);
        create_dir(&root.join("a_dir"));
        create_dir(&root.join("b_dir"));
        create_dir(&root.join(".hdir"));
        create_file(&root.join("a.txt"));
        create_file(&root.join("z.txt"));
        create_file(&root.join(".hidden"));

        let mut node = FileNode::new(root.clone(), true);
        node.load_children(false, false).unwrap();
        let names: Vec<_> = node
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| n.name())
            .collect();
        assert_eq!(names, vec!["a_dir", "b_dir"]);

        node.load_children(true, false).unwrap();
        let names: Vec<_> = node
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| n.name())
            .collect();
        assert_eq!(names, vec!["a_dir", "b_dir", "a.txt", "z.txt"]);

        node.load_children(true, true).unwrap();
        let names: Vec<_> = node
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| n.name())
            .collect();
        assert_eq!(
            names,
            vec![".hdir", "a_dir", "b_dir", ".hidden", "a.txt", "z.txt"]
        );
    }

    #[test]
    fn load_child_counts_filters_and_caches() {
        let temp = TempDir::new("cdtree_load_child_counts");
        let root = temp.path.join("root");
        create_dir(&root);
        create_dir(&root.join("a_dir"));
        create_dir(&root.join("b_dir"));
        create_dir(&root.join(".hdir"));
        create_file(&root.join("a.txt"));
        create_file(&root.join("z.txt"));
        create_file(&root.join(".hidden"));

        let mut node = FileNode::new(root.clone(), true);
        node.load_child_counts(false, false);
        assert_eq!(node.child_counts(), Some((2, 0)));
        assert!(node.child_count_attempted);

        let mut node = FileNode::new(root.clone(), true);
        node.load_child_counts(true, false);
        assert_eq!(node.child_counts(), Some((2, 2)));

        let mut node = FileNode::new(root, true);
        node.load_child_counts(true, true);
        assert_eq!(node.child_counts(), Some((3, 3)));
    }

    #[test]
    fn ensure_visible_child_counts_only_counts_drawn_rows() {
        let temp = TempDir::new("cdtree_visible_counts");
        let root = temp.path.join("root");
        let first = root.join("first");
        let second = root.join("second");
        create_dir(&first);
        create_dir(&second);
        create_file(&first.join("file.txt"));
        create_file(&second.join("file.txt"));

        let mut root_node = FileNode::new(root.clone(), true);
        root_node.load_children(true, true).unwrap();
        root_node.expanded = true;

        let mut app = App {
            root: root_node,
            selected_path: root.clone(),
            startup_path: root.clone(),
            show_files: true,
            show_hidden: true,
            list_state: ListState::default(),
            config: Config::default(),
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history: History::default(),
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: root.clone(),
        };

        app.ensure_visible_child_counts(1, 1);

        let first_node = App::find_node(&app.root, first.as_path()).unwrap();
        let second_node = App::find_node(&app.root, second.as_path()).unwrap();
        assert_eq!(first_node.child_counts(), Some((0, 1)));
        assert_eq!(second_node.child_counts(), None);
        assert!(!second_node.child_count_attempted);
    }

    #[test]
    fn ensure_visible_child_counts_skips_collapsed_descendants() {
        let temp = TempDir::new("cdtree_collapsed_counts");
        let root = temp.path.join("root");
        let collapsed = root.join("collapsed");
        let descendant = collapsed.join("descendant");
        create_dir(&descendant);
        create_file(&descendant.join("file.txt"));

        let mut descendant_node = FileNode::new(descendant.clone(), true);
        let mut collapsed_node = FileNode::new(collapsed.clone(), true);
        collapsed_node.children = Some(vec![descendant_node.clone()]);

        let mut root_node = FileNode::new(root.clone(), true);
        root_node.expanded = true;
        root_node.children = Some(vec![collapsed_node]);

        let mut app = App {
            root: root_node,
            selected_path: root.clone(),
            startup_path: root.clone(),
            show_files: true,
            show_hidden: true,
            list_state: ListState::default(),
            config: Config::default(),
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history: History::default(),
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: root.clone(),
        };

        app.ensure_visible_child_counts(0, 100);

        descendant_node = App::find_node(&app.root, descendant.as_path())
            .unwrap()
            .clone();
        assert!(!descendant_node.child_count_attempted);
        assert_eq!(descendant_node.child_counts(), None);
    }

    #[test]
    fn failed_child_count_is_not_retried_until_cache_is_cleared() {
        let temp = TempDir::new("cdtree_failed_counts");
        let root = temp.path.join("root");
        let missing = root.join("missing");
        create_dir(&root);

        let mut root_node = FileNode::new(root.clone(), true);
        root_node.expanded = true;
        root_node.children = Some(vec![FileNode::new(missing.clone(), true)]);

        let mut app = App {
            root: root_node,
            selected_path: root.clone(),
            startup_path: root.clone(),
            show_files: true,
            show_hidden: true,
            list_state: ListState::default(),
            config: Config::default(),
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history: History::default(),
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: root.clone(),
        };

        app.ensure_visible_child_counts(1, 1);
        create_dir(&missing);
        create_file(&missing.join("file.txt"));
        app.ensure_visible_child_counts(1, 1);

        let missing_node = App::find_node(&app.root, missing.as_path()).unwrap();
        assert!(missing_node.child_count_attempted);
        assert_eq!(missing_node.child_counts(), None);

        App::clear_child_count_cache(&mut app.root);
        app.ensure_visible_child_counts(1, 1);

        let missing_node = App::find_node(&app.root, missing.as_path()).unwrap();
        assert_eq!(missing_node.child_counts(), Some((0, 1)));
    }

    #[test]
    fn get_visible_nodes_prefix_and_order() {
        let app = sample_app();
        let visible = app.get_visible_nodes();
        let got: Vec<(String, String)> = visible
            .iter()
            .map(|(prefix, node)| (prefix.clone(), node.name()))
            .collect();
        let expected = vec![
            ("".to_string(), "root".to_string()),
            ("├─ ".to_string(), "a".to_string()),
            ("│  ├─ ".to_string(), "aa".to_string()),
            ("│  └─ ".to_string(), "ab".to_string()),
            ("└─ ".to_string(), "b".to_string()),
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn move_selection_clamps_to_bounds() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a");
        app.move_selection(-10);
        assert_eq!(app.selected_path, PathBuf::from("/root"));
        app.move_selection(10);
        assert_eq!(app.selected_path, PathBuf::from("/root/b"));
    }

    #[test]
    fn ensure_valid_selection_prefers_parent() {
        let mut app = sample_app();
        if let Some(children) = app.root.children.as_mut() {
            children[0].expanded = false;
        }
        app.selected_path = PathBuf::from("/root/a/ab");
        app.ensure_valid_selection();
        assert_eq!(app.selected_path, PathBuf::from("/root/a"));
    }

    #[test]
    fn on_left_moves_to_parent_and_collapses() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a");
        app.on_left();
        assert_eq!(app.selected_path, PathBuf::from("/root"));
        let a_node = App::find_node(&app.root, Path::new("/root/a")).unwrap();
        assert!(!a_node.expanded);
    }

    #[test]
    fn toggle_current_collapses_expanded_dir() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a");
        assert!(
            App::find_node(&app.root, Path::new("/root/a"))
                .unwrap()
                .expanded
        );
        app.toggle_current();
        assert!(
            !App::find_node(&app.root, Path::new("/root/a"))
                .unwrap()
                .expanded
        );
    }

    #[test]
    fn toggle_current_expands_collapsed_dir() {
        let mut app = sample_app();
        // give b pre-loaded children so toggle doesn't hit the filesystem
        if let Some(children) = app.root.children.as_mut() {
            children[1].children = Some(vec![]);
        }
        app.selected_path = PathBuf::from("/root/b");
        assert!(
            !App::find_node(&app.root, Path::new("/root/b"))
                .unwrap()
                .expanded
        );
        app.toggle_current();
        assert!(
            App::find_node(&app.root, Path::new("/root/b"))
                .unwrap()
                .expanded
        );
    }

    #[test]
    fn toggle_current_noop_for_file() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a/ab"); // file
        app.toggle_current();
        assert_eq!(app.selected_path, PathBuf::from("/root/a/ab"));
    }

    #[test]
    fn select_visible_index_sets_selected_path() {
        let mut app = sample_app();
        app.select_visible_index(3); // /root/a/ab
        assert_eq!(app.selected_path, PathBuf::from("/root/a/ab"));
        assert_eq!(app.list_state.selected(), Some(3));
    }

    #[test]
    fn select_visible_index_out_of_bounds_is_noop() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root");
        app.select_visible_index(100);
        assert_eq!(app.selected_path, PathBuf::from("/root"));
    }

    #[test]
    fn scroll_down_increases_offset_without_changing_selection() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root");
        *app.list_state.offset_mut() = 0;
        app.scroll(1);
        assert_eq!(app.list_state.offset(), 1);
        assert_eq!(app.selected_path, PathBuf::from("/root"));
    }

    #[test]
    fn scroll_up_decreases_offset_without_changing_selection() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root");
        *app.list_state.offset_mut() = 2;
        app.scroll(-1);
        assert_eq!(app.list_state.offset(), 1);
        assert_eq!(app.selected_path, PathBuf::from("/root"));
    }

    #[test]
    fn scroll_up_at_zero_clamps_to_zero() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root");
        *app.list_state.offset_mut() = 0;
        app.scroll(-1);
        assert_eq!(app.list_state.offset(), 0);
    }

    #[test]
    fn scroll_history_changes_offset_without_changing_selection() {
        let mut app = sample_app();
        for i in 0..3 {
            app.history.entries.push(crate::history::HistoryEntry {
                path: PathBuf::from(format!("/root/dir{}", i)),
                timestamp: i as i64,
            });
        }
        app.history_list_state.select(Some(1));
        *app.history_list_state.offset_mut() = 0;
        app.scroll_history(1);
        assert_eq!(app.history_list_state.offset(), 1);
        assert_eq!(app.history_list_state.selected(), Some(1));
    }

    #[test]
    fn expand_to_path_expands_nested_dirs() {
        let temp = TempDir::new("cdtree_expand");
        let root = temp.path.join("root");
        let level1 = root.join("level1");
        let level2 = level1.join("level2");
        create_dir(&level2);

        let mut root_node = FileNode::new(root.clone(), true);
        root_node.load_children(true, true).unwrap();
        root_node.expanded = true;

        let mut app = App {
            root: root_node,
            selected_path: root.clone(),
            startup_path: root.clone(),
            show_files: true,
            show_hidden: true,
            list_state: ListState::default(),
            config: Config::default(),
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history: History::default(),
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: root.clone(),
        };

        app.expand_to_path(level2.as_path());

        let level1_node = App::find_node(&app.root, level1.as_path()).unwrap();
        assert!(level1_node.expanded);
        assert!(level1_node.children.is_some());

        let level2_node = App::find_node(&app.root, level2.as_path()).unwrap();
        assert!(level2_node.expanded);
    }

    #[test]
    fn app_new_selects_current_dir_when_inside_home() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_app_new_in");
        let home = temp.path.join("home");
        let current = home.join("projects").join("alpha");
        create_dir(&current);

        let canonical_home = home.canonicalize().unwrap();
        let canonical_current = current.canonicalize().unwrap();

        let _cwd_guard = CurrentDirGuard::set(&canonical_current);
        let _home_guard = EnvGuard::set("HOME", canonical_home.to_str().unwrap());

        let app = App::new().unwrap();

        assert_eq!(app.root.path, canonical_home);
        assert!(app.root.expanded);
        assert_eq!(app.selected_path, canonical_current);
        assert_eq!(app.startup_path, canonical_current);

        let level1 = canonical_home.join("projects");
        let level1_node = App::find_node(&app.root, level1.as_path()).unwrap();
        assert!(level1_node.expanded);
        let leaf_node = App::find_node(&app.root, canonical_current.as_path()).unwrap();
        assert!(leaf_node.expanded);
    }

    #[test]
    fn app_new_applies_visibility_config_to_initial_root_load() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_app_new_config");
        let home = temp.path.join("home");
        create_dir(&home);
        create_dir(&home.join(".hidden_dir"));
        create_file(&home.join(".hidden_file"));
        create_file(&home.join("visible_file"));

        let canonical_home = home.canonicalize().unwrap();
        let _cwd_guard = CurrentDirGuard::set(&canonical_home);
        let _home_guard = EnvGuard::set("HOME", canonical_home.to_str().unwrap());

        let mut config = Config::default();
        config.show_files = true;
        config.show_hidden = true;
        config.save().unwrap();

        let app = App::new().unwrap();
        let names: Vec<_> = app
            .root
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|node| node.name())
            .collect();

        assert!(app.show_files);
        assert!(app.show_hidden);
        assert!(names.contains(&".hidden_dir".to_string()));
        assert!(names.contains(&".hidden_file".to_string()));
        assert!(names.contains(&"visible_file".to_string()));
    }

    #[test]
    fn toggle_history_mode_resets_selection() {
        let mut app = sample_app();
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/a"),
            timestamp: 1,
        });
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/b"),
            timestamp: 2,
        });

        assert!(!app.history_mode);
        app.toggle_history_mode();
        assert!(app.history_mode);
        assert_eq!(app.history_list_state.selected(), Some(0));

        app.toggle_history_mode();
        assert!(!app.history_mode);
    }

    #[test]
    fn toggle_history_mode_empty_does_not_select() {
        let mut app = sample_app();
        assert!(app.history.entries.is_empty());
        app.toggle_history_mode();
        assert!(app.history_mode);
        assert_eq!(app.history_list_state.selected(), None);
    }

    #[test]
    fn move_history_selection_clamps() {
        let mut app = sample_app();
        for i in 0..3 {
            app.history.entries.push(crate::history::HistoryEntry {
                path: PathBuf::from(format!("/root/dir{}", i)),
                timestamp: i as i64,
            });
        }
        app.history_list_state.select(Some(0));

        app.move_history_selection(-1);
        assert_eq!(app.history_list_state.selected(), Some(0));

        app.move_history_selection(1);
        assert_eq!(app.history_list_state.selected(), Some(1));

        app.move_history_selection(10);
        assert_eq!(app.history_list_state.selected(), Some(2));

        app.move_history_selection(-1);
        assert_eq!(app.history_list_state.selected(), Some(1));
    }

    #[test]
    fn move_history_selection_empty_is_noop() {
        let mut app = sample_app();
        app.move_history_selection(1);
        assert_eq!(app.history_list_state.selected(), None);
    }

    #[test]
    fn selected_history_path_returns_correct() {
        let mut app = sample_app();
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/a"),
            timestamp: 1,
        });
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/b"),
            timestamp: 2,
        });
        app.history_list_state.select(Some(1));

        assert_eq!(app.selected_history_path(), Some(PathBuf::from("/root/b")));
    }

    #[test]
    fn select_from_history_returns_current_mode() {
        let mut app = sample_app();
        app.mode = AppMode::Open;
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/a"),
            timestamp: 1,
        });
        app.history_list_state.select(Some(0));

        let result = app.select_from_history();
        assert_eq!(result, Some(("/root/a".to_string(), AppMode::Open)));
        assert_eq!(app.history.entries.len(), 1);
    }

    #[test]
    fn select_from_history_returns_each_mode() {
        for mode in [AppMode::Cd, AppMode::Open, AppMode::Code] {
            let mut app = sample_app();
            app.mode = mode;
            app.history.entries.push(crate::history::HistoryEntry {
                path: PathBuf::from("/root/a"),
                timestamp: 1,
            });
            app.history_list_state.select(Some(0));

            let result = app.select_from_history();
            assert_eq!(result.map(|(_, m)| m), Some(mode));
        }
    }

    #[test]
    fn history_mode_toggle_cycles_modes() {
        let mut app = sample_app();
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/a"),
            timestamp: 1,
        });

        // Enter history mode, toggle mode from Cd -> Open
        app.toggle_history_mode();
        assert!(app.history_mode);
        assert_eq!(app.mode, AppMode::Cd);

        app.mode.toggle();
        assert_eq!(app.mode, AppMode::Open);

        // Select should use the current mode (Open)
        app.history_list_state.select(Some(0));
        let result = app.select_from_history();
        assert_eq!(result.map(|(_, m)| m), Some(AppMode::Open));
    }

    #[test]
    fn history_mode_preserves_mode_on_toggle() {
        let mut app = sample_app();
        app.mode = AppMode::Code;

        // Enter history mode, mode should be preserved
        app.toggle_history_mode();
        assert_eq!(app.mode, AppMode::Code);

        // Exit history mode, mode still preserved
        app.toggle_history_mode();
        assert_eq!(app.mode, AppMode::Code);
    }

    #[test]
    fn record_and_get_path_records_in_cd_mode() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a");
        app.mode = AppMode::Cd;

        let result = app.record_and_get_path();
        assert_eq!(result, Some(("/root/a".to_string(), AppMode::Cd)));
        assert_eq!(app.history.entries.len(), 1);
    }

    #[test]
    fn record_and_get_path_skips_in_open_mode() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a");
        app.mode = AppMode::Open;

        let result = app.record_and_get_path();
        assert!(result.is_some());
        assert!(app.history.entries.is_empty());
    }

    #[test]
    fn record_and_get_path_returns_none_for_file() {
        let mut app = sample_app();
        app.selected_path = PathBuf::from("/root/a/ab"); // file, not dir
        app.mode = AppMode::Cd;

        let result = app.record_and_get_path();
        assert!(result.is_none());
    }

    #[test]
    fn app_new_selects_home_when_current_dir_outside_home() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_app_new_out");
        let home = temp.path.join("home");
        let outside = temp.path.join("outside");
        create_dir(&home);
        create_dir(&outside);

        let canonical_home = home.canonicalize().unwrap();
        let canonical_outside = outside.canonicalize().unwrap();

        let _cwd_guard = CurrentDirGuard::set(&canonical_outside);
        let _home_guard = EnvGuard::set("HOME", canonical_home.to_str().unwrap());

        let app = App::new().unwrap();

        assert_eq!(app.root.path, canonical_home);
        assert_eq!(app.selected_path, canonical_home);
        assert_eq!(app.startup_path, canonical_outside);
    }

    #[test]
    fn start_search_enters_mode_with_empty_query() {
        let mut app = sample_app();
        app.search_query = "stale".to_string();
        app.start_search();
        assert!(app.search_mode);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn search_input_and_backspace() {
        let mut app = sample_app();
        app.start_search();
        app.search_input('F');
        app.search_input('o');
        assert_eq!(app.search_query, "Fo");
        app.search_backspace();
        assert_eq!(app.search_query, "F");
        app.search_backspace();
        app.search_backspace();
        assert!(app.search_query.is_empty());
        assert!(app.search_mode);
    }

    #[test]
    fn exit_search_clears_query_and_mode() {
        let mut app = sample_app();
        app.start_search();
        app.search_input('x');
        app.exit_search();
        assert!(!app.search_mode);
        assert!(app.search_query.is_empty());
    }

    fn app_at(root: &Path) -> App {
        let mut root_node = FileNode::new(root.to_path_buf(), true);
        root_node.load_children(true, true).unwrap();
        root_node.expanded = true;
        App {
            root: root_node,
            selected_path: root.to_path_buf(),
            startup_path: root.to_path_buf(),
            show_files: true,
            show_hidden: true,
            list_state: ListState::default(),
            config: Config::default(),
            current_theme: Theme::default(),
            last_theme_change: None,
            mode: AppMode::Cd,
            history_mode: false,
            history: History::default(),
            history_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            search_applied: String::new(),
            search_applied_complete: false,
            search_tree: None,
            search_phase: SearchPhase::Idle,
            search_truncated: false,
            search_follow_best: true,
            home_dir: root.to_path_buf(),
        }
    }

    fn visible_names(app: &App) -> Vec<String> {
        app.get_visible_nodes()
            .iter()
            .map(|(_, node)| node.name())
            .collect()
    }

    #[test]
    fn build_match_tree_keeps_ancestors_and_drops_other_branches() {
        let root = PathBuf::from("proj");
        let alpha = root.join("src").join("alpha-target");
        let zeta = root.join("zeta");
        let tree = build_match_tree(&root, &[zeta.clone(), alpha.clone()]);

        let names: Vec<_> = tree
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|node| node.name())
            .collect();
        assert_eq!(names, vec!["src".to_string(), "zeta".to_string()]);

        let src = &tree.children.as_ref().unwrap()[0];
        assert!(src.expanded);
        let nested = &src.children.as_ref().unwrap()[0];
        assert_eq!(nested.path, alpha);
        assert!(nested.children.is_none());
        assert!(tree.expanded);
    }

    #[test]
    fn search_waits_for_debounce_then_shows_expanded_hits_only() {
        let temp = TempDir::new("cdtree_fuzzy_tree");
        let root = temp.path.join("root");
        create_dir(&root.join("keep").join("alpha-target"));
        create_dir(&root.join("keep").join("other"));
        create_dir(&root.join("skip").join("nope"));
        create_file(&root.join("keep").join("notes-target.txt"));

        let mut app = app_at(&root);
        app.start_search();
        for ch in "target".chars() {
            app.search_input(ch);
        }

        let since = match &app.search_phase {
            SearchPhase::Waiting { since } => *since,
            _ => panic!("expected the search to wait for typing to pause"),
        };
        app.poll_search_at(since);
        let names = visible_names(&app);
        assert!(names.iter().any(|name| name == "skip"));
        assert!(!names.iter().any(|name| name == "alpha-target"));
        assert!(!app.search_view_active());

        app.finish_search_for_test();
        assert!(app.search_view_active());
        assert_eq!(
            visible_names(&app),
            vec![
                "root".to_string(),
                "keep".to_string(),
                "alpha-target".to_string()
            ]
        );
        let visible = app.get_visible_nodes();
        let leaf = visible
            .iter()
            .find(|(_, node)| node.name() == "alpha-target")
            .unwrap();
        assert_eq!(leaf.0, "   └─ ");
        assert_eq!(app.selected_path, root.join("keep").join("alpha-target"));

        app.on_left();
        assert_eq!(app.selected_path, root.join("keep"));
        app.expand_current();
        assert_eq!(app.selected_path, root.join("keep").join("alpha-target"));

        app.exit_search();
        assert!(!app.search_mode);
        assert!(app.search_query.is_empty());
        let names = visible_names(&app);
        assert!(names.iter().any(|name| name == "skip"));
        assert!(names.iter().any(|name| name == "other"));
        assert!(!app.search_view_active());
    }

    #[test]
    fn clearing_the_query_restores_the_live_tree_immediately() {
        let temp = TempDir::new("cdtree_fuzzy_clear");
        let root = temp.path.join("root");
        create_dir(&root.join("keep").join("alpha-target"));
        create_dir(&root.join("skip"));

        let mut app = app_at(&root);
        app.start_search();
        app.search_input('g');
        app.search_input('e');
        app.search_input('t');
        app.finish_search_for_test();
        assert!(app.search_view_active());

        app.search_backspace();
        assert!(app.search_view_active());
        assert_eq!(app.search_query, "ge");

        app.search_backspace();
        app.search_backspace();
        assert!(app.search_query.is_empty());
        assert!(!app.search_view_active());
        assert!(visible_names(&app).iter().any(|name| name == "skip"));
    }

    fn search_from_collapsed_live_root(root: &Path) -> App {
        let target = root.join("keep/target");
        create_dir(&target);
        let mut app = app_at(root);
        // A mouse click selects and toggles the root before entering search.
        app.select_visible_index(0);
        app.toggle_current();
        assert!(!app.root.expanded);
        app.start_search();
        for ch in "target".chars() {
            app.search_input(ch);
        }
        app.finish_search_for_test();
        assert_eq!(app.selected_path, target);
        assert!(!app.root.expanded);
        app
    }

    #[test]
    fn exiting_search_preserves_selection_under_collapsed_live_root() {
        let temp = TempDir::new("cdtree_search_exit_collapsed_root");
        let mut app = search_from_collapsed_live_root(&temp.path);
        app.exit_search();

        assert_eq!(app.selected_path, temp.path.join("keep/target"));
        assert!(app.root.expanded);
        assert!(!app.search_mode);
        assert!(!app.search_view_active());
        let visible = app.get_visible_nodes();
        assert_eq!(
            visible[app.list_state.selected().unwrap()].1.path,
            app.selected_path
        );
    }

    #[test]
    fn clearing_query_preserves_selection_under_collapsed_live_root() {
        let temp = TempDir::new("cdtree_search_clear_collapsed_root");
        let mut app = search_from_collapsed_live_root(&temp.path);
        while !app.search_query.is_empty() {
            app.search_backspace();
        }

        assert_eq!(app.selected_path, temp.path.join("keep/target"));
        assert!(app.root.expanded);
        assert!(app.search_mode);
        assert!(!app.search_view_active());
        let visible = app.get_visible_nodes();
        assert_eq!(
            visible[app.list_state.selected().unwrap()].1.path,
            app.selected_path
        );
    }

    #[test]
    fn leaving_search_with_root_selected_keeps_live_root_collapsed() {
        let temp = TempDir::new("cdtree_search_keep_root_collapsed");
        for clear_query in [false, true] {
            let mut app = search_from_collapsed_live_root(&temp.path);
            app.select_visible_index(0);
            if clear_query {
                while !app.search_query.is_empty() {
                    app.search_backspace();
                }
            } else {
                app.exit_search();
            }
            assert_eq!(app.selected_path, temp.path);
            assert!(!app.root.expanded);
            assert_eq!(app.get_visible_nodes().len(), 1);
            assert_eq!(app.list_state.selected(), Some(0));
        }
    }

    #[test]
    fn search_shows_hits_while_the_walk_continues() {
        let temp = TempDir::new("cdtree_fuzzy_live");
        let root = temp.path.join("root");
        create_dir(&root.join("keep").join("alpha-target"));
        create_dir(&root.join("middle"));
        create_dir(&root.join("zz").join("target"));

        let mut app = app_at(&root);
        app.start_search();
        for ch in "target".chars() {
            app.search_input(ch);
        }

        // Opening the root only queues its children. Nothing matches yet.
        assert!(!app.advance_search_for_test(1));
        assert!(app.search_busy());
        assert!(!app.search_view_active());
        assert_eq!(app.search_status(), "Searching...");
        assert!(visible_names(&app).iter().any(|name| name == "middle"));

        // The next directory contains a hit. Show it before the walk finishes.
        assert!(!app.advance_search_for_test(1));
        assert!(app.search_busy());
        assert!(app.search_view_active());
        assert_eq!(
            visible_names(&app),
            vec![
                "root".to_string(),
                "keep".to_string(),
                "alpha-target".to_string(),
            ]
        );
        assert_eq!(app.selected_path, root.join("keep").join("alpha-target"));
        assert_eq!(app.search_status(), "1 match...");

        // Stay on the row the user moved to once a clearer hit shows up later.
        app.on_left();
        assert_eq!(app.selected_path, root.join("keep"));
        assert!(!app.advance_search_for_test(1));
        assert!(!app.advance_search_for_test(1));
        assert!(app.search_busy());
        assert_eq!(app.search_status(), "2 matches...");
        assert_eq!(app.selected_path, root.join("keep"));
        assert!(visible_names(&app).iter().any(|name| name == "target"));
        assert!(!visible_names(&app).iter().any(|name| name == "middle"));

        app.finish_search_for_test();
        assert!(!app.search_busy());
        assert_eq!(app.search_status(), "2 matches");
        assert_eq!(app.selected_path, root.join("keep"));
    }

    #[test]
    fn returning_to_a_partially_published_query_finishes_the_search() {
        let temp = TempDir::new("cdtree_search_return_partial");
        let root = temp.path.join("root");
        create_dir(&root.join("needle-a"));
        create_dir(&root.join("z/needle-b"));
        let mut app = app_at(&root);
        app.start_search();
        for ch in "needle".chars() {
            app.search_input(ch);
        }
        assert!(!app.advance_search_for_test(1));
        assert_eq!(app.search_status(), "1 match...");

        app.search_input('x');
        app.search_backspace();
        assert_eq!(app.search_status(), "1 match...");
        app.finish_search_for_test();
        assert_eq!(app.search_status(), "2 matches");
        assert!(visible_names(&app).contains(&"needle-b".to_string()));
    }

    #[test]
    fn returning_to_a_completed_query_reuses_the_results() {
        let temp = TempDir::new("cdtree_search_return_complete");
        create_dir(&temp.path.join("needle"));
        let mut app = app_at(&temp.path);
        app.start_search();
        for ch in "needle".chars() {
            app.search_input(ch);
        }
        app.finish_search_for_test();
        app.search_input('x');
        app.search_backspace();
        assert!(matches!(app.search_phase, SearchPhase::Idle));
        assert_eq!(app.search_status(), "1 match");
    }

    #[test]
    fn search_status_distinguishes_incomplete_zero_hits_and_keeps_scanning() {
        use crate::search::ScanLimits;
        let temp = TempDir::new("cdtree_search_partial_status");
        create_dir(&temp.path.join("a/deep/needle"));
        create_dir(&temp.path.join("z/needle"));
        create_dir(&temp.path.join("zz"));
        let mut app = app_at(&temp.path);
        app.start_search();
        app.search_query = "needle".to_string();
        let scan = SearchScan::with_limits(
            &temp.path,
            "needle",
            true,
            ScanLimits {
                max_dirs: 100,
                max_matches: 100,
                max_depth: 0,
            },
        );
        app.search_phase = SearchPhase::Scanning(Box::new(scan));
        app.finish_search_for_test();
        assert_eq!(app.search_status(), "No matches (partial search)");
        let screen = render_screen(&mut app, 60, 10);
        assert!(screen_text(&screen).contains("No matches (partial search)"));

        app.search_phase = SearchPhase::Scanning(Box::new(SearchScan::with_limits(
            &temp.path,
            "needle",
            true,
            ScanLimits {
                max_dirs: 100,
                max_matches: 100,
                max_depth: 1,
            },
        )));
        assert!(!app.advance_search_for_test(3));
        assert_eq!(app.search_status(), "1 match, partial...");
        app.finish_search_for_test();
        assert_eq!(app.search_status(), "1 match, partial");
    }

    #[test]
    fn search_lands_on_the_clearer_shallow_hit() {
        let temp = TempDir::new("cdtree_fuzzy_best");
        let root = temp.path.join("root");
        create_dir(&root.join("Desktop"));
        create_dir(&root.join("nested").join("mid").join("Desktop"));
        create_dir(&root.join("skip"));

        let mut app = app_at(&root);
        app.start_search();
        for ch in "desk".chars() {
            app.search_input(ch);
        }
        app.finish_search_for_test();

        assert_eq!(app.selected_path, root.join("Desktop"));
        assert_eq!(app.search_status(), "2 matches");
        let names = visible_names(&app);
        assert_eq!(
            names,
            vec![
                "root".to_string(),
                "Desktop".to_string(),
                "nested".to_string(),
                "mid".to_string(),
                "Desktop".to_string(),
            ]
        );
        assert_eq!(app.list_state.offset(), 0);

        // A hit the user already moved to stays put when it still matches.
        app.move_selection(3);
        assert_eq!(
            app.selected_path,
            root.join("nested").join("mid").join("Desktop")
        );
        app.search_backspace();
        // The first slice finds only the shallow hit. The selected deeper hit
        // must not be discarded before the new walk reaches its branch.
        assert!(!app.advance_search_for_test(1));
        assert_eq!(app.selected_path, root.join("nested/mid/Desktop"));
        app.finish_search_for_test();
        assert_eq!(app.search_query, "des");
        assert_eq!(
            app.selected_path,
            root.join("nested").join("mid").join("Desktop")
        );
    }

    #[test]
    fn completed_search_replaces_a_selection_excluded_by_the_new_query() {
        let temp = TempDir::new("cdtree_search_excluded_selection");
        create_dir(&temp.path.join("target"));
        create_dir(&temp.path.join("targetx"));
        let mut app = app_at(&temp.path);
        app.start_search();
        for ch in "target".chars() {
            app.search_input(ch);
        }
        app.finish_search_for_test();
        app.select_visible_index(1);
        assert_eq!(app.selected_path, temp.path.join("target"));
        app.search_input('x');
        assert!(!app.advance_search_for_test(1));
        app.finish_search_for_test();
        assert_eq!(app.selected_path, temp.path.join("targetx"));
        assert_eq!(app.search_status(), "1 match");
    }

    fn render_screen(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(crate::ui::TreeWidget::new(app), frame.area());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn screen_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut lines = Vec::new();
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            lines.push(line);
        }
        lines.join("\n")
    }

    fn row_with<'a>(buf: &'a ratatui::buffer::Buffer, needle: &str) -> Option<u16> {
        (0..buf.area.height).find(|y| {
            let line: String = (0..buf.area.width).map(|x| buf[(x, *y)].symbol()).collect();
            line.contains(needle)
        })
    }

    #[test]
    fn search_screen_keeps_the_path_and_highlights_hits() {
        use ratatui::style::{Color, Modifier};

        let temp = TempDir::new("cdtree_fuzzy_screen");
        let root = temp.path.join("root");
        create_dir(&root.join("Desktop"));
        create_dir(&root.join("nested").join("mid").join("Desktop"));
        create_dir(&root.join("keep").join("alpha-target"));
        create_dir(&root.join("skip"));

        let mut app = app_at(&root);
        app.start_search();
        for ch in "desk".chars() {
            app.search_input(ch);
        }

        let pending = render_screen(&mut app, 72, 16);
        let pending_text = screen_text(&pending);
        assert!(
            pending_text.contains("skip"),
            "unfiltered tree should stay up until the debounce elapses:\n{pending_text}"
        );
        assert!(
            pending_text.contains("desk"),
            "the query is shown while typing:\n{pending_text}"
        );
        assert!(
            !pending_text.contains("matches"),
            "the result count waits for the walk:\n{pending_text}"
        );
        let desktop_row = row_with(&pending, "Desktop").unwrap();
        let desktop_yellow = (0..pending.area.width).any(|x| {
            let cell = &pending[(x, desktop_row)];
            cell.bg == Color::Yellow && cell.symbol() == "D"
        });
        assert!(
            desktop_yellow,
            "visible hits are highlighted before the tree filters"
        );

        app.finish_search_for_test();
        let shown = render_screen(&mut app, 72, 16);
        let shown_text = screen_text(&shown);
        assert!(
            !shown_text.contains("skip"),
            "non-matching branches are hidden:\n{shown_text}"
        );
        assert!(
            !shown_text.contains("alpha-target"),
            "unrelated directories stay hidden:\n{shown_text}"
        );
        assert!(
            shown_text.contains("2 matches"),
            "the guide reports the hit count:\n{shown_text}"
        );
        let root_at = row_with(&shown, "root").unwrap();
        let nested_at = row_with(&shown, "nested").unwrap();
        let mid_at = row_with(&shown, "mid").unwrap();
        let selected_at = (0..shown.area.height)
            .find(|y| {
                let line: String = (0..shown.area.width)
                    .map(|x| shown[(x, *y)].symbol())
                    .collect();
                line.contains('>') && line.contains("Desktop")
            })
            .expect("selected hit is marked");
        assert!(root_at < selected_at && selected_at < nested_at && nested_at < mid_at);
        let deep = (selected_at + 1..shown.area.height)
            .find(|y| {
                let line: String = (0..shown.area.width)
                    .map(|x| shown[(x, *y)].symbol())
                    .collect();
                line.contains("Desktop") && !line.contains('>')
            })
            .expect("the deeper same-named hit stays on screen");
        assert!(mid_at < deep);

        // The selected name keeps the cursor color. The match is underlined
        // rather than punching a yellow hole in the selection.
        let selected_row: Vec<_> = (0..shown.area.width)
            .map(|x| shown[(x, selected_at)].clone())
            .collect();
        let name_at = selected_row
            .iter()
            .position(|cell| cell.symbol() == "D")
            .unwrap();
        let matched = &selected_row[name_at];
        let unmatched = &selected_row[name_at + 4];
        assert_eq!(unmatched.symbol(), "t");
        assert_ne!(matched.bg, Color::Yellow);
        assert_eq!(matched.bg, unmatched.bg);
        assert_eq!(matched.fg, Color::Black);
        assert!(matched.modifier.contains(Modifier::UNDERLINED));
        assert!(matched.modifier.contains(Modifier::BOLD));
        // Only the matched letters are underlined, so the rest of the name
        // still reads as the cursor.
        assert!(!unmatched.modifier.contains(Modifier::UNDERLINED));

        let deep_match = (0..shown.area.width)
            .map(|x| &shown[(x, deep)])
            .find(|cell| cell.symbol() == "D")
            .unwrap();
        assert_eq!(deep_match.bg, Color::Yellow);
        assert_eq!(deep_match.fg, Color::Black);

        // Arrow keys move the cursor inside the filtered tree.
        app.move_selection(3);
        let moved = render_screen(&mut app, 72, 16);
        let moved_mid = row_with(&moved, "mid").unwrap();
        let moved_cursor = (0..moved.area.height)
            .find(|y| {
                let line: String = (0..moved.area.width)
                    .map(|x| moved[(x, *y)].symbol())
                    .collect();
                line.contains('>') && line.contains("Desktop")
            })
            .expect("the cursor follows the selection");
        assert!(moved_mid < moved_cursor);
        let returned = (0..moved.area.height)
            .find(|y| {
                let line: String = (0..moved.area.width)
                    .map(|x| moved[(x, *y)].symbol())
                    .collect();
                line.contains("Desktop") && !line.contains('>')
            })
            .unwrap();
        let returned_match = (0..moved.area.width)
            .map(|x| &moved[(x, returned)])
            .find(|cell| cell.symbol() == "D")
            .unwrap();
        assert_eq!(returned_match.bg, Color::Yellow);
    }

    #[test]
    fn short_screen_keeps_ancestors_above_a_deep_hit() {
        let temp = TempDir::new("cdtree_fuzzy_short");
        let root = temp.path.join("root");
        create_dir(&root.join("a").join("b").join("c").join("d").join("needle"));
        create_dir(&root.join("other"));

        let mut app = app_at(&root);
        app.start_search();
        for ch in "needle".chars() {
            app.search_input(ch);
        }
        app.finish_search_for_test();
        assert_eq!(
            app.selected_path,
            root.join("a").join("b").join("c").join("d").join("needle")
        );

        let shown = render_screen(&mut app, 40, 8);
        let text = screen_text(&shown);
        // The title also contains the query, so match the tree row.
        let needle_at = row_with(&shown, "└─ needle").expect(&text);
        let parent_at = row_with(&shown, "└─ d").expect(&text);
        assert!(
            parent_at < needle_at,
            "the parent stays above the hit on a short screen:\n{text}"
        );
        let needle_line: String = (0..shown.area.width)
            .map(|x| shown[(x, needle_at)].symbol())
            .collect();
        assert!(
            needle_line.contains('>'),
            "the hit row keeps the cursor:\n{text}"
        );
        assert!(
            !text.contains("other"),
            "unrelated rows stay hidden:\n{text}"
        );
        assert!(text.contains("1 match"), "a single hit is labeled:\n{text}");
    }

    #[test]
    fn search_screen_reports_no_matches() {
        let temp = TempDir::new("cdtree_fuzzy_none");
        let root = temp.path.join("root");
        create_dir(&root.join("skip"));
        create_dir(&root.join("keep"));

        let mut app = app_at(&root);
        app.start_search();
        for ch in "zzzz".chars() {
            app.search_input(ch);
        }
        app.finish_search_for_test();

        assert_eq!(app.search_status(), "No matches");
        assert_eq!(app.selected_path, root);
        let shown = render_screen(&mut app, 60, 10);
        let text = screen_text(&shown);
        assert!(text.contains("No matches"), "{text}");
        assert!(text.contains("zzzz"), "{text}");
        assert!(text.contains("root"), "{text}");
        assert!(!text.contains("skip"), "{text}");
        assert!(!text.contains("keep"), "{text}");
    }
}

// History mode methods
impl App {
    pub fn toggle_history_mode(&mut self) {
        self.history_mode = !self.history_mode;
        if self.history_mode && !self.history.entries.is_empty() {
            self.history_list_state.select(Some(0));
        }
    }

    pub fn move_history_selection(&mut self, delta: i32) {
        if self.history.entries.is_empty() {
            return;
        }
        let current = self.history_list_state.selected().unwrap_or(0);
        let max = (self.history.entries.len() - 1) as i32;
        let new = (current as i32 + delta).clamp(0, max) as usize;
        self.history_list_state.select(Some(new));
    }

    pub fn selected_history_path(&self) -> Option<PathBuf> {
        let idx = self.history_list_state.selected()?;
        self.history.entries.get(idx).map(|e| e.path.clone())
    }

    pub fn select_from_history(&mut self) -> Option<(String, AppMode)> {
        let path = self.selected_history_path()?;
        self.history.record(path.clone());
        let path_str = path.to_string_lossy().to_string();
        Some((path_str, self.mode))
    }

    pub fn record_and_get_path(&mut self) -> Option<(String, AppMode)> {
        if !self.is_selected_dir() {
            return None;
        }
        if self.mode == AppMode::Cd {
            self.history.record(self.selected_path.clone());
        }
        let path = self.selected_path.to_string_lossy().to_string();
        Some((path, self.mode))
    }
}
