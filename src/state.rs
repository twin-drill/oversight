use crate::config::Config;
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Legacy format for backward-compatible deserialization.
#[derive(Deserialize)]
struct LegacyProcessedEntry {
    context_id: u64,
    head_turn_id: u64,
}

fn serialize_processed<S>(
    map: &HashMap<u64, u64>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(k, _)| *k);
    for (context_id, head_turn_id) in entries {
        #[derive(Serialize)]
        struct Entry {
            context_id: u64,
            head_turn_id: u64,
        }
        seq.serialize_element(&Entry {
            context_id: *context_id,
            head_turn_id: *head_turn_id,
        })?;
    }
    seq.end()
}

fn deserialize_processed<'de, D>(
    deserializer: D,
) -> std::result::Result<HashMap<u64, u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let entries: Vec<LegacyProcessedEntry> = Vec::deserialize(deserializer)?;
    Ok(entries.into_iter().map(|e| (e.context_id, e.head_turn_id)).collect())
}

/// Persistent state for the healing loop.
///
/// Tracks the last poll timestamp, which contexts have been processed,
/// and hashes of previously extracted learnings to avoid duplicates.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoopState {
    /// When the loop last polled for new sessions.
    #[serde(default)]
    pub last_poll_at: Option<DateTime<Utc>>,

    /// Contexts that have already been processed, keyed by context_id -> head_turn_id.
    #[serde(
        default,
        serialize_with = "serialize_processed",
        deserialize_with = "deserialize_processed"
    )]
    pub processed: HashMap<u64, u64>,

    /// SHA-256 hashes of previously extracted learnings (for dedup).
    #[serde(default)]
    pub extraction_hashes: HashSet<String>,

    /// SHA-256 hashes of pattern clusters already synthesized.
    #[serde(default)]
    pub pattern_hashes: HashSet<String>,
}

impl LoopState {
    /// Load state from the default state file path.
    pub fn load_default() -> Result<Self> {
        let path = Config::state_path();
        Self::load(&path)
    }

    /// Load state from a file. Returns default state if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(LoopState::default());
        }

        let content = fs::read_to_string(path).map_err(|e| {
            Error::State(format!("Failed to read state file {}: {e}", path.display()))
        })?;

        serde_json::from_str(&content).map_err(|e| {
            Error::State(format!(
                "Failed to parse state file {}: {e}",
                path.display()
            ))
        })
    }

    /// Save state to the default state file path.
    pub fn save_default(&self) -> Result<()> {
        let path = Config::state_path();
        self.save(&path)
    }

    /// Save state atomically to a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self).map_err(|e| {
            Error::State(format!("Failed to serialize state: {e}"))
        })?;

        let temp_name = path
            .file_name()
            .map(|name| format!("{}.tmp", name.to_string_lossy()))
            .unwrap_or_else(|| "state.tmp".to_string());
        let temp_path = path.with_file_name(temp_name);
        fs::write(&temp_path, &content).map_err(|e| {
            Error::State(format!("Failed to write temp state file {}: {e}", temp_path.display()))
        })?;
        fs::rename(&temp_path, path).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            Error::State(format!(
                "Failed to rename temp state file {} -> {}: {e}",
                temp_path.display(),
                path.display()
            ))
        })?;

        Ok(())
    }

    /// Check if a context has been processed (at the given turn level).
    pub fn is_processed(&self, context_id: u64, head_turn_id: u64) -> bool {
        self.processed.get(&context_id) == Some(&head_turn_id)
    }

    /// Record that a context has been processed.
    pub fn mark_processed(&mut self, context_id: u64, head_turn_id: u64) {
        self.processed.insert(context_id, head_turn_id);
    }

    /// Record an extraction hash.
    pub fn add_extraction_hash(&mut self, hash: String) {
        self.extraction_hashes.insert(hash);
    }

    /// Check if an extraction hash has been seen.
    pub fn has_extraction_hash(&self, hash: &str) -> bool {
        self.extraction_hashes.contains(hash)
    }

    pub fn add_pattern_hash(&mut self, hash: String) {
        self.pattern_hashes.insert(hash);
    }

    pub fn pattern_hashes(&self) -> &HashSet<String> {
        &self.pattern_hashes
    }

    /// Return a summary string for display.
    pub fn summary(&self) -> String {
        let last_poll = self
            .last_poll_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "never".to_string());
        format!(
            "Last poll: {}\nContexts processed: {}\nExtraction hashes: {}\nPattern hashes: {}",
            last_poll,
            self.processed.len(),
            self.extraction_hashes.len(),
            self.pattern_hashes.len(),
        )
    }

    /// Return the state file path for a given override or default.
    pub fn state_path(override_path: Option<&Path>) -> PathBuf {
        match override_path {
            Some(p) => p.to_path_buf(),
            None => Config::state_path(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_state() {
        let state = LoopState::default();
        assert!(state.last_poll_at.is_none());
        assert!(state.processed.is_empty());
        assert!(state.extraction_hashes.is_empty());
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");

        let mut state = LoopState {
            last_poll_at: Some(Utc::now()),
            ..LoopState::default()
        };
        state.mark_processed(1, 100);
        state.mark_processed(2, 200);
        state.add_extraction_hash("abc123".to_string());

        state.save(&path).unwrap();

        let loaded = LoopState::load(&path).unwrap();
        assert!(loaded.last_poll_at.is_some());
        assert_eq!(loaded.processed.len(), 2);
        assert!(loaded.has_extraction_hash("abc123"));
        assert!(!loaded.has_extraction_hash("xyz"));
    }

    #[test]
    fn test_load_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = LoopState::load(&path).unwrap();
        assert!(state.processed.is_empty());
    }

    #[test]
    fn test_is_processed() {
        let mut state = LoopState::default();
        assert!(!state.is_processed(1, 100));
        state.mark_processed(1, 100);
        assert!(state.is_processed(1, 100));
        assert!(!state.is_processed(1, 200));
    }

    #[test]
    fn test_mark_processed_updates_head() {
        let mut state = LoopState::default();
        state.mark_processed(1, 100);
        assert!(state.is_processed(1, 100));

        state.mark_processed(1, 200);
        assert!(state.is_processed(1, 200));
        assert!(!state.is_processed(1, 100));
        assert_eq!(
            state.processed.keys().filter(|k| **k == 1).count(),
            1
        );
    }

    #[test]
    fn test_summary() {
        let state = LoopState::default();
        let summary = state.summary();
        assert!(summary.contains("Last poll: never"));
        assert!(summary.contains("Contexts processed: 0"));
    }

    #[test]
    fn test_load_legacy_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");

        let legacy_json = r#"{
            "last_poll_at": null,
            "processed": [
                {"context_id": 1, "head_turn_id": 100},
                {"context_id": 2, "head_turn_id": 200}
            ],
            "extraction_hashes": []
        }"#;
        fs::write(&path, legacy_json).unwrap();

        let state = LoopState::load(&path).unwrap();
        assert_eq!(state.processed.len(), 2);
        assert!(state.is_processed(1, 100));
        assert!(state.is_processed(2, 200));
    }
}
