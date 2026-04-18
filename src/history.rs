use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub fn load() -> io::Result<Self> {
        let path = Self::history_path();
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let history: History = serde_json::from_str(&content).unwrap_or_default();
        Ok(history)
    }

    pub fn save(&self) -> io::Result<()> {
        let path = Self::history_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn upsert_entry(&mut self, path: PathBuf) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.entries.retain(|e| e.path != path);
        self.entries.insert(0, HistoryEntry { path, timestamp });
        self.entries.truncate(MAX_ENTRIES);
    }

    pub fn record(&mut self, path: PathBuf) {
        self.upsert_entry(path);
        let _ = self.save();
    }

    /// Add entry without persisting (for testing use only)
    #[cfg(test)]
    fn add_entry(&mut self, path: PathBuf) {
        self.upsert_entry(path);
    }

    fn history_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Path::new(&home).join(".config").join("cdtree").join("history.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_record_adds_entry() {
        let mut history = History::default();
        history.add_entry(PathBuf::from("/tmp/test1"));
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].path, PathBuf::from("/tmp/test1"));
    }

    #[test]
    fn test_record_deduplicates() {
        let mut history = History::default();
        history.add_entry(PathBuf::from("/tmp/test1"));
        history.add_entry(PathBuf::from("/tmp/test2"));
        history.add_entry(PathBuf::from("/tmp/test1")); // re-visit
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].path, PathBuf::from("/tmp/test1")); // moved to front
    }

    #[test]
    fn test_record_moves_to_front() {
        let mut history = History::default();
        history.add_entry(PathBuf::from("/tmp/a"));
        history.add_entry(PathBuf::from("/tmp/b"));
        history.add_entry(PathBuf::from("/tmp/c"));
        history.add_entry(PathBuf::from("/tmp/a")); // re-visit a
        assert_eq!(history.entries[0].path, PathBuf::from("/tmp/a"));
        assert_eq!(history.entries[1].path, PathBuf::from("/tmp/c"));
        assert_eq!(history.entries[2].path, PathBuf::from("/tmp/b"));
    }

    #[test]
    fn test_record_enforces_max() {
        let mut history = History::default();
        for i in 0..(MAX_ENTRIES + 10) {
            history.add_entry(PathBuf::from(format!("/tmp/dir{}", i)));
        }
        assert_eq!(history.entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("cdtree_test_history_roundtrip");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("history.json");

        let mut history = History::default();
        history.add_entry(PathBuf::from("/tmp/test_roundtrip"));

        let content = serde_json::to_string_pretty(&history).unwrap();
        fs::write(&file_path, &content).unwrap();

        let loaded_content = fs::read_to_string(&file_path).unwrap();
        let loaded: History = serde_json::from_str(&loaded_content).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].path, PathBuf::from("/tmp/test_roundtrip"));

        let _ = fs::remove_dir_all(&dir);
    }
}
