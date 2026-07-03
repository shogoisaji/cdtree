use crate::config::{Config, RgbColor, Theme};
use crate::history::History;
use rand::Rng;
use ratatui::widgets::ListState;
use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
        let mut result = Vec::new();
        let mut is_last_stack = Vec::new();
        result.push(("".to_string(), &self.root));

        if self.root.expanded {
            if let Some(children) = &self.root.children {
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(App::find_node(&app.root, Path::new("/root/a")).unwrap().expanded);
        app.toggle_current();
        assert!(!App::find_node(&app.root, Path::new("/root/a")).unwrap().expanded);
    }

    #[test]
    fn toggle_current_expands_collapsed_dir() {
        let mut app = sample_app();
        // give b pre-loaded children so toggle doesn't hit the filesystem
        if let Some(children) = app.root.children.as_mut() {
            children[1].children = Some(vec![]);
        }
        app.selected_path = PathBuf::from("/root/b");
        assert!(!App::find_node(&app.root, Path::new("/root/b")).unwrap().expanded);
        app.toggle_current();
        assert!(App::find_node(&app.root, Path::new("/root/b")).unwrap().expanded);
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
