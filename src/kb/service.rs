use crate::config::Config;
use crate::error::{Error, Result};
use crate::kb::search::{self, SearchResult};
use crate::kb::slug;
use crate::kb::store::KBStore;
use crate::kb::types::{Topic, TopicSummary};
use std::path::Path;

/// High-level knowledge base service.
///
/// This is the primary API for both the CLI and the future healing loop daemon.
/// It wraps KBStore with config resolution and convenience methods.
pub struct KBService {
    store: KBStore,
    config: Config,
}

impl KBService {
    /// Create a new KBService from a resolved config.
    pub fn new(config: Config) -> Self {
        let store = KBStore::new(config.kb_path.clone());
        KBService { store, config }
    }

    /// Create a KBService with default config resolution.
    pub fn from_defaults(cli_kb_path: Option<&Path>) -> Result<Self> {
        let config = Config::resolve(cli_kb_path)?;
        Ok(Self::new(config))
    }

    /// Initialize a new KB directory structure.
    ///
    /// Creates: config.toml, kb/topics/, kb/index.json
    pub fn init(&self) -> Result<()> {
        self.store.ensure_dirs()?;

        let config_path = Config::config_file_path();
        if !config_path.exists() {
            if let Err(e) = self.config.write_to_file(&config_path) {
                eprintln!("Warning: could not write config to {}: {e}", config_path.display());
            }
        }

        self.store.rebuild_index()?;

        Ok(())
    }

    /// Check if the KB is initialized.
    pub fn is_initialized(&self) -> bool {
        self.store.topics_dir().exists()
    }

    /// Ensure the KB is initialized before performing operations.
    fn require_initialized(&self) -> Result<()> {
        if !self.is_initialized() {
            return Err(Error::NotInitialized);
        }
        Ok(())
    }

    /// Add a new topic.
    pub fn add_topic(
        &self,
        name: &str,
        body: &str,
        tags: Vec<String>,
        aliases: Vec<String>,
    ) -> Result<Topic> {
        self.require_initialized()?;

        let topic_slug = slug::normalize(name);
        slug::validate(&topic_slug)?;

        let mut topic = Topic::new(name.to_string(), topic_slug, body.to_string());
        topic.tags = tags;
        topic.aliases = aliases;

        self.store.add(&topic)?;
        Ok(topic)
    }

    /// Update an existing topic's body content.
    pub fn update_topic(&self, slug_or_alias: &str, body: &str) -> Result<Topic> {
        self.require_initialized()?;

        let mut topic = self.store.get(slug_or_alias)?;
        let original_slug = topic.slug.clone();
        topic.body = body.to_string();
        topic.updated = Some(chrono::Utc::now());

        self.store.update(&original_slug, &topic)?;
        Ok(topic)
    }

    /// Create a topic from a fully-formed Topic struct.
    pub fn create_topic(&self, topic: &Topic) -> Result<()> {
        self.require_initialized()?;
        self.store.add(topic)
    }

    /// Upsert a topic (add if new, update if exists).
    pub fn upsert_topic(&self, topic: &Topic) -> Result<()> {
        self.require_initialized()?;
        self.store.upsert(topic)
    }

    /// Get a topic by slug or alias.
    pub fn get_topic(&self, slug_or_alias: &str) -> Result<Topic> {
        self.require_initialized()?;
        self.store.get(slug_or_alias)
    }

    /// Delete a topic by slug.
    pub fn delete_topic(&self, slug: &str) -> Result<()> {
        self.require_initialized()?;

        // Resolve alias to slug if needed
        let topic = self.store.get(slug)?;
        self.store.delete(&topic.slug)
    }

    /// List all topics as lightweight summaries.
    pub fn list_topics(&self) -> Result<Vec<TopicSummary>> {
        self.require_initialized()?;
        self.store.list()
    }

    /// Load all topics with full body content.
    pub fn load_topics(&self) -> Result<Vec<Topic>> {
        self.require_initialized()?;
        self.store.load_all_topics()
    }

    /// Search topics by keyword query.
    pub fn search_topics(&self, query: &str) -> Result<Vec<SearchResult>> {
        let topics = self.load_topics()?;
        Ok(search::search(&topics, query))
    }

    /// Rebuild the topic index from disk.
    pub fn rebuild_index(&self) -> Result<()> {
        self.require_initialized()?;
        self.store.rebuild_index()
    }

    /// Get a reference to the underlying config.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, KBService) {
        let dir = TempDir::new().unwrap();
        let config = Config {
            kb_path: dir.path().to_path_buf(),
            ..Config::default()
        };
        let service = KBService::new(config);
        service.init().unwrap();
        (dir, service)
    }

    #[test]
    fn test_init() {
        let dir = TempDir::new().unwrap();
        let config = Config {
            kb_path: dir.path().to_path_buf(),
            ..Config::default()
        };
        let service = KBService::new(config);

        assert!(!service.is_initialized());
        service.init().unwrap();
        assert!(service.is_initialized());
        assert!(dir.path().join("topics").exists());
        assert!(dir.path().join("index.json").exists());
    }

    #[test]
    fn test_full_lifecycle() {
        let (_dir, service) = setup();

        // Add
        let topic = service
            .add_topic("Docker Local", "# Docker\n\nRun locally.", vec!["docker".into()], vec!["docker for local".into()])
            .unwrap();
        assert_eq!(topic.slug, "docker-local");

        // List
        let list = service.list_topics().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].slug, "docker-local");

        // Read
        let read = service.get_topic("docker-local").unwrap();
        assert_eq!(read.title, "Docker Local");

        let created = service.get_topic("docker-local").unwrap();
        service.create_topic(&created).unwrap_err();

        let full_topics = service.load_topics().unwrap();
        assert_eq!(full_topics.len(), 1);
        assert_eq!(full_topics[0].slug, "docker-local");

        // Read by alias
        let read = service.get_topic("docker for local").unwrap();
        assert_eq!(read.slug, "docker-local");

        // Update
        let updated = service.update_topic("docker-local", "# Docker\n\nNew content.").unwrap();
        assert!(updated.body.contains("New content."));

        // Search
        let results = service.search_topics("docker").unwrap();
        assert!(!results.is_empty());

        // Delete
        service.delete_topic("docker-local").unwrap();
        let list = service.list_topics().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_not_initialized_error() {
        let dir = TempDir::new().unwrap();
        let config = Config {
            kb_path: dir.path().join("nonexistent"),
            ..Config::default()
        };
        let service = KBService::new(config);

        let err = service.list_topics().unwrap_err();
        assert!(err.to_string().contains("not initialized"));
    }
}
