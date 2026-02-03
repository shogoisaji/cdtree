use std::path::{Path, PathBuf};
use std::fs;
use std::cmp::Ordering;
use std::io;
use ratatui::widgets::ListState;

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

        let mut app = Self {
            root,
            selected_path,
            startup_path: current_dir.clone(),
            show_files: false,
            show_hidden: false,
            list_state: ListState::default(),
        };

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
        self.refresh_after_toggle();
    }
    
    pub fn toggle_show_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
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
}
