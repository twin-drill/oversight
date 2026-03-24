use crate::error::{Error, Result};
use crate::kb::frontmatter;
use crate::kb::index::IndexManager;
use crate::kb::slug;
use crate::kb::types::{Topic, TopicSummary};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Filesystem-backed topic CRUD operations.
pub struct KBStore {
    /// Root of the KB (e.g. ~/.oversight/kb/).
    root: PathBuf,
    index_manager: IndexManager,
}

impl KBStore {
    /// Create a new KBStore with the given root directory.
    pub fn new(root: PathBuf) -> Self {
        let index_manager = IndexManager::new(root.clone());
        KBStore {
            root,
            index_manager,
        }
    }

    /// Return the root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the topics directory path.
    pub fn topics_dir(&self) -> PathBuf {
        self.root.join("topics")
    }

    /// Return the file path for a given slug.
    fn topic_path(&self, slug: &str) -> PathBuf {
        self.topics_dir().join(format!("{slug}.md"))
    }

    /// Ensure the KB directory structure exists.
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.topics_dir())?;
        Ok(())
    }

    /// Add a new topic. Errors on slug or alias collision.
    pub fn add(&self, topic: &Topic) -> Result<()> {
        slug::validate(&topic.slug)?;

        // Check slug collision
        let path = self.topic_path(&topic.slug);
        if path.exists() {
            return Err(Error::SlugCollision(topic.slug.clone()));
        }

        // Check alias collisions against all existing topics
        self.check_alias_collisions(topic, None)?;

        // Write the topic file atomically
        self.write_topic_atomic(topic)?;

        // Rebuild index
        self.rebuild_index()?;

        Ok(())
    }

    /// Update an existing topic. The slug must already exist.
    pub fn update(&self, slug: &str, topic: &Topic) -> Result<()> {
        slug::validate(slug)?;
        slug::validate(&topic.slug)?;

        let path = self.topic_path(slug);
        if !path.exists() {
            return Err(Error::TopicNotFound(slug.to_string()));
        }

        // If slug changed, check for collision with new slug
        if slug != topic.slug {
            let new_path = self.topic_path(&topic.slug);
            if new_path.exists() {
                return Err(Error::SlugCollision(topic.slug.clone()));
            }
        }

        // Check alias collisions (excluding the topic being updated)
        self.check_alias_collisions(topic, Some(slug))?;

        // If slug changed, remove old file
        if slug != topic.slug {
            fs::remove_file(&path)?;
        }

        // Write the updated topic
        self.write_topic_atomic(topic)?;

        // Rebuild index
        self.rebuild_index()?;

        Ok(())
    }

    /// Add or update a topic.
    pub fn upsert(&self, topic: &Topic) -> Result<()> {
        let path = self.topic_path(&topic.slug);
        if path.exists() {
            self.update(&topic.slug, topic)
        } else {
            self.add(topic)
        }
    }

    /// Get a topic by slug or alias.
    pub fn get(&self, slug_or_alias: &str) -> Result<Topic> {
        // Try direct slug lookup first
        let normalized = slug::normalize(slug_or_alias);
        let path = self.topic_path(&normalized);
        if path.exists() {
            return self.read_topic_file(&path);
        }

        // Try alias lookup: search through all topics
        let query_lower = slug_or_alias.to_lowercase();
        for topic in self.load_all_topics()? {
            if topic.slug == normalized {
                return Ok(topic);
            }
            for alias in &topic.aliases {
                if alias.to_lowercase() == query_lower {
                    return Ok(topic);
                }
            }
        }

        Err(Error::TopicNotFound(slug_or_alias.to_string()))
    }

    /// Delete a topic by slug.
    pub fn delete(&self, slug: &str) -> Result<()> {
        slug::validate(slug)?;

        let path = self.topic_path(slug);
        if !path.exists() {
            return Err(Error::TopicNotFound(slug.to_string()));
        }

        fs::remove_file(path)?;

        // Rebuild index
        self.rebuild_index()?;

        Ok(())
    }

    /// List all topics as summaries (from the index).
    pub fn list(&self) -> Result<Vec<TopicSummary>> {
        // Regenerate index if missing
        let index_path = self.root.join("index.json");
        if !index_path.exists() {
            self.rebuild_index()?;
        }

        let index = self.index_manager.load()?;
        Ok(index.topics)
    }

    /// Rebuild the index from the topic files on disk.
    pub fn rebuild_index(&self) -> Result<()> {
        let topics = self.load_all_topics()?;
        let summaries: Vec<TopicSummary> = topics.iter().map(|t| t.to_summary()).collect();
        self.index_manager.save(&summaries)?;
        Ok(())
    }

    /// Load all topics from disk.
    pub fn load_all_topics(&self) -> Result<Vec<Topic>> {
        let topics_dir = self.topics_dir();
        if !topics_dir.exists() {
            return Ok(Vec::new());
        }

        let mut topics = Vec::new();
        for entry in WalkDir::new(&topics_dir)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                topics.push(self.read_topic_file(path)?);
            }
        }

        // Sort by slug for deterministic ordering
        topics.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(topics)
    }

    /// Read and parse a single topic file.
    fn read_topic_file(&self, path: &Path) -> Result<Topic> {
        let content = fs::read_to_string(path)?;
        frontmatter::parse(&content)
    }

    /// Write a topic file atomically (write to temp, then rename).
    fn write_topic_atomic(&self, topic: &Topic) -> Result<()> {
        let content = frontmatter::serialize(topic)?;
        let target = self.topic_path(&topic.slug);

        // Write to a temp file in the same directory, then rename
        let temp_path = self.topics_dir().join(format!(".{}.tmp", topic.slug));
        fs::write(&temp_path, &content)?;
        fs::rename(&temp_path, &target)?;

        Ok(())
    }

    /// Check that none of the topic's aliases collide with existing slugs or aliases.
    /// `exclude_slug` allows skipping the topic being updated.
    fn check_alias_collisions(
        &self,
        topic: &Topic,
        exclude_slug: Option<&str>,
    ) -> Result<()> {
        let existing_topics = self.load_all_topics()?;

        for alias in &topic.aliases {
            let alias_lower = alias.to_lowercase();
            let alias_as_slug = slug::normalize(alias);

            for existing in &existing_topics {
                // Skip the topic being updated
                if let Some(exc) = exclude_slug {
                    if existing.slug == exc {
                        continue;
                    }
                }

                // Check if alias matches an existing slug
                if existing.slug == alias_as_slug {
                    return Err(Error::AliasCollision {
                        alias: alias.clone(),
                        existing_slug: existing.slug.clone(),
                    });
                }

                // Check if alias matches any existing alias
                for existing_alias in &existing.aliases {
                    if existing_alias.to_lowercase() == alias_lower {
                        return Err(Error::AliasCollision {
                            alias: alias.clone(),
                            existing_slug: existing.slug.clone(),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, KBStore) {
        let dir = TempDir::new().unwrap();
        let store = KBStore::new(dir.path().to_path_buf());
        store.ensure_dirs().unwrap();
        (dir, store)
    }

    #[test]
    fn test_add_and_get() {
        let (_dir, store) = setup();

        let topic = Topic::new(
            "Test Topic".to_string(),
            "test-topic".to_string(),
            "# Test\n\nContent here.\n".to_string(),
        );

        store.add(&topic).unwrap();

        let retrieved = store.get("test-topic").unwrap();
        assert_eq!(retrieved.title, "Test Topic");
        assert_eq!(retrieved.slug, "test-topic");
        assert!(retrieved.body.contains("Content here."));
    }

    #[test]
    fn test_slug_collision() {
        let (_dir, store) = setup();

        let topic1 = Topic::new("A".to_string(), "same-slug".to_string(), "Body 1".to_string());
        let topic2 = Topic::new("B".to_string(), "same-slug".to_string(), "Body 2".to_string());

        store.add(&topic1).unwrap();
        let err = store.add(&topic2).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_alias_collision() {
        let (_dir, store) = setup();

        let mut topic1 = Topic::new("A".to_string(), "topic-a".to_string(), "Body".to_string());
        topic1.aliases = vec!["my alias".to_string()];
        store.add(&topic1).unwrap();

        let mut topic2 = Topic::new("B".to_string(), "topic-b".to_string(), "Body".to_string());
        topic2.aliases = vec!["my alias".to_string()];
        let err = store.add(&topic2).unwrap_err();
        assert!(err.to_string().contains("collides"));
    }

    #[test]
    fn test_delete() {
        let (_dir, store) = setup();

        let topic = Topic::new("Del".to_string(), "del-me".to_string(), "Body".to_string());
        store.add(&topic).unwrap();
        assert!(store.get("del-me").is_ok());

        store.delete("del-me").unwrap();
        assert!(store.get("del-me").is_err());
    }

    #[test]
    fn test_list() {
        let (_dir, store) = setup();

        let t1 = Topic::new("A".to_string(), "aaa".to_string(), "Body".to_string());
        let t2 = Topic::new("B".to_string(), "bbb".to_string(), "Body".to_string());

        store.add(&t1).unwrap();
        store.add(&t2).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_load_all_topics_fails_on_invalid_topic_file() {
        let (_dir, store) = setup();
        let bad_path = store.topics_dir().join("broken.md");
        fs::write(&bad_path, "not frontmatter").unwrap();

        let err = store.load_all_topics().unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
    }

    #[test]
    fn test_get_by_alias() {
        let (_dir, store) = setup();

        let mut topic = Topic::new(
            "GitHub CLI".to_string(),
            "gh-cli".to_string(),
            "Use gh.".to_string(),
        );
        topic.aliases = vec!["github cli".to_string()];
        store.add(&topic).unwrap();

        let found = store.get("github cli").unwrap();
        assert_eq!(found.slug, "gh-cli");
    }

    #[test]
    fn test_update() {
        let (_dir, store) = setup();

        let topic = Topic::new("Original".to_string(), "my-topic".to_string(), "V1".to_string());
        store.add(&topic).unwrap();

        let mut updated = topic;
        updated.body = "V2".to_string();
        updated.title = "Updated".to_string();
        store.update("my-topic", &updated).unwrap();

        let retrieved = store.get("my-topic").unwrap();
        assert_eq!(retrieved.title, "Updated");
        assert!(retrieved.body.contains("V2"));
    }

    #[test]
    fn test_upsert_add() {
        let (_dir, store) = setup();

        let topic = Topic::new("New".to_string(), "new-topic".to_string(), "Body".to_string());
        store.upsert(&topic).unwrap();

        let retrieved = store.get("new-topic").unwrap();
        assert_eq!(retrieved.title, "New");
    }

    #[test]
    fn test_upsert_update() {
        let (_dir, store) = setup();

        let topic = Topic::new("V1".to_string(), "upsert-me".to_string(), "Body 1".to_string());
        store.add(&topic).unwrap();

        let mut updated = topic;
        updated.body = "Body 2".to_string();
        store.upsert(&updated).unwrap();

        let retrieved = store.get("upsert-me").unwrap();
        assert!(retrieved.body.contains("Body 2"));
    }

    #[test]
    fn test_empty_list() {
        let (_dir, store) = setup();
        let list = store.list().unwrap();
        assert!(list.is_empty());
    }
}
