use std::path::PathBuf;

/// Domain-specific error types for the oversight system.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("KB root not initialized. Run `oversight init` first.")]
    NotInitialized,

    #[error("KB root already exists at {0}")]
    AlreadyInitialized(PathBuf),

    #[error("Topic not found: {0}")]
    TopicNotFound(String),

    #[error("Topic already exists with slug: {0}")]
    SlugCollision(String),

    #[error("Alias '{alias}' collides with existing topic '{existing_slug}'")]
    AliasCollision {
        alias: String,
        existing_slug: String,
    },

    #[error("Invalid slug: {0}")]
    InvalidSlug(String),

    #[error("Invalid frontmatter: {0}")]
    InvalidFrontmatter(String),

    #[error("Missing required frontmatter field: {0}")]
    MissingField(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("LLM API error: {0}")]
    LlmApi(String),

    #[error("LLM API key not set. Set {0} in your environment.")]
    LlmKeyMissing(String),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("Loop state error: {0}")]
    State(String),

    #[error("Merge conflict: {0}")]
    MergeConflict(String),

    #[error("Unknown integration target: {0}")]
    UnknownTarget(String),

    #[error("Integration error: {0}")]
    Integration(String),

    #[error("Malformed managed block in {path}: {detail}")]
    MalformedBlock { path: String, detail: String },
}

pub type Result<T> = std::result::Result<T, Error>;
