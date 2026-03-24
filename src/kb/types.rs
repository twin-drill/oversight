use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A full topic with metadata and body content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub title: String,
    pub slug: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    /// Context ID from the source that generated this topic (machine provenance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_context_id: Option<u64>,
    /// Timestamp when this was extracted by the healing loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_at: Option<DateTime<Utc>>,
    /// The markdown body content (not part of frontmatter).
    #[serde(skip)]
    pub body: String,
}

impl Topic {
    /// Create a new topic with minimal required fields.
    pub fn new(title: String, slug: String, body: String) -> Self {
        let now = Utc::now();
        Topic {
            title,
            slug,
            aliases: Vec::new(),
            tags: Vec::new(),
            created: Some(now),
            updated: Some(now),
            source: Some("manual".to_string()),
            confidence: Some("high".to_string()),
            source_context_id: None,
            extracted_at: None,
            body,
        }
    }

    /// Convert this topic to a lightweight summary (no body).
    pub fn to_summary(&self) -> TopicSummary {
        TopicSummary {
            slug: self.slug.clone(),
            title: self.title.clone(),
            aliases: self.aliases.clone(),
            tags: self.tags.clone(),
        }
    }
}

/// A lightweight summary of a topic for index/listing purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicSummary {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Index file structure containing all topic summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicIndex {
    pub version: u32,
    pub generated: DateTime<Utc>,
    pub topics: Vec<TopicSummary>,
}

impl TopicIndex {
    pub fn new(topics: Vec<TopicSummary>) -> Self {
        TopicIndex {
            version: 1,
            generated: Utc::now(),
            topics,
        }
    }
}
