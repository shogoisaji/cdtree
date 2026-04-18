use std::path::{Path, PathBuf};
use std::fs;
use std::cmp::Ordering;
use std::io;
use ratatui::widgets::ListState;
use crate::config::{Config, Theme, RgbColor};
use crate::history::History;
use rand::Rng;
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
            AppMode::Cd => "_CD",
            AppMode::Open => "_OPEN",
            AppMode::Code => "_CODE",
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
}

impl FileNode {
    pub fn new(path: PathBuf, is_dir: bool) -> Self {
        Self {
            path,
            is_dir,
            children: None,
            expanded: false,
        }
    }

    pub fn name(&self) -> String {
        self.path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
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

        entries.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => a.path.file_name().cmp(&b.path.file_name()),
            }
        });

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
        
        let mut root = FileNode::new(home_dir.clone(), true);
        root.load_children(false, false)?; 
        root.expanded = true;

        let selected_path = if current_dir.starts_with(&home_dir) {
            current_dir.clone()
        } else {
            home_dir.clone()
        };

        let config = Config::load().unwrap_or_default();
        let show_files = config.show_files;
        let show_hidden = config.show_hidden;
        let history = History::load().unwrap_or_default();

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
        if let Some(pos) = app.get_visible_nodes().iter().position(|(_, node)| node.path == app.selected_path) {
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

    fn collect_visible_nodes<'a>(node: &'a FileNode, is_last_stack: &mut Vec<bool>, result: &mut Vec<(String, &'a FileNode)>) {
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
        if visible.is_empty() { return; }

        let current_idx = visible.iter().position(|(_, node)| node.path == self.selected_path);
        
        if let Some(idx) = current_idx {
            let max_idx = (visible.len() - 1) as i32;
            let new_idx = (idx as i32 + delta).clamp(0, max_idx) as usize;
            self.selected_path = visible[new_idx].1.path.clone();
            self.update_list_state();
        }
    }

    pub fn update_list_state(&mut self) {
        let visible = self.get_visible_nodes();
        if let Some(pos) = visible.iter().position(|(_, node)| node.path == self.selected_path) {
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
    where F: Fn(&mut FileNode) + Copy {
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
        self.reload_all();
        let selected = self.selected_path.clone();
        self.expand_to_path(selected.as_path());
        self.ensure_valid_selection();
        self.update_list_state();
    }
    
    fn ensure_valid_selection(&mut self) {
        let visible = self.get_visible_nodes();
        if visible.is_empty() { return; }

        // Check if current selection is still visible
        if visible.iter().any(|(_, node)| node.path == self.selected_path) {
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
    
    fn traverse_and_reload(node: &mut FileNode, show_files: bool, show_hidden: bool) {
         if node.is_dir && node.expanded {
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
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use crate::test_support::{env_lock, CurrentDirGuard, EnvGuard};

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
        let names: Vec<_> = node.children.as_ref().unwrap().iter().map(|n| n.name()).collect();
        assert_eq!(names, vec!["a_dir", "b_dir"]);

        node.load_children(true, false).unwrap();
        let names: Vec<_> = node.children.as_ref().unwrap().iter().map(|n| n.name()).collect();
        assert_eq!(names, vec!["a_dir", "b_dir", "a.txt", "z.txt"]);

        node.load_children(true, true).unwrap();
        let names: Vec<_> = node.children.as_ref().unwrap().iter().map(|n| n.name()).collect();
        assert_eq!(
            names,
            vec![".hdir", "a_dir", "b_dir", ".hidden", "a.txt", "z.txt"]
        );
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
    fn select_from_history_returns_cd_mode() {
        let mut app = sample_app();
        app.mode = AppMode::Open;
        app.history.entries.push(crate::history::HistoryEntry {
            path: PathBuf::from("/root/a"),
            timestamp: 1,
        });
        app.history_list_state.select(Some(0));

        let result = app.select_from_history();
        assert_eq!(result, Some(("/root/a".to_string(), AppMode::Cd)));
        // Should re-record (move to front)
        assert_eq!(app.history.entries.len(), 1);
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
        Some((path_str, AppMode::Cd))
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
