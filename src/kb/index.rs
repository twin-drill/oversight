use crate::error::Result;
use crate::kb::types::{TopicIndex, TopicSummary};
use std::fs;
use std::path::PathBuf;

/// Manages the index.json file for lightweight topic discovery.
pub struct IndexManager {
    /// Root of the KB (e.g. ~/.oversight/kb/).
    root: PathBuf,
}

impl IndexManager {
    pub fn new(root: PathBuf) -> Self {
        IndexManager { root }
    }

    /// Path to the index file.
    pub fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    /// Load the topic index from disk.
    pub fn load(&self) -> Result<TopicIndex> {
        let path = self.index_path();
        if !path.exists() {
            // Return empty index if file doesn't exist
            return Ok(TopicIndex::new(Vec::new()));
        }

        let content = fs::read_to_string(&path)?;
        let index: TopicIndex = serde_json::from_str(&content)?;
        Ok(index)
    }

    /// Save the topic index to disk (atomic write).
    pub fn save(&self, topics: &[TopicSummary]) -> Result<()> {
        let index = TopicIndex::new(topics.to_vec());
        let content = serde_json::to_string_pretty(&index)?;

        let path = self.index_path();
        let temp_path = self.root.join(".index.json.tmp");

        fs::write(&temp_path, &content)?;
        fs::rename(&temp_path, &path)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let mgr = IndexManager::new(dir.path().to_path_buf());

        let summaries = vec![
            TopicSummary {
                slug: "gh-cli".to_string(),
                title: "GitHub CLI".to_string(),
                aliases: vec!["github cli".to_string()],
                tags: vec!["cli".to_string()],
            },
            TopicSummary {
                slug: "aws-sso".to_string(),
                title: "AWS SSO".to_string(),
                aliases: vec![],
                tags: vec!["aws".to_string()],
            },
        ];

        mgr.save(&summaries).unwrap();

        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.topics.len(), 2);
        assert_eq!(loaded.topics[0].slug, "gh-cli");
        assert_eq!(loaded.topics[1].slug, "aws-sso");
    }

    #[test]
    fn test_load_missing() {
        let dir = TempDir::new().unwrap();
        let mgr = IndexManager::new(dir.path().to_path_buf());

        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.topics.len(), 0);
    }
}
