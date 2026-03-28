use crate::error::Result;
use crate::healing_loop::patterns::PatternConfig;
use crate::healing_loop::policy::{DedupePolicy, Regime};
use crate::llm::client::LlmProvider;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Configuration for dedupe threshold overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DedupeConfig {
    /// Word-overlap ratio to consider a learning "covered" (0.0-1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_threshold: Option<f64>,
    /// Minimum tag overlap count for tag-based matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_overlap_minimum: Option<usize>,
    /// Whether tag-overlap matching also requires slug substring affinity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_slug_affinity: Option<bool>,
    /// Title match mode: "exact", "contains", or "fuzzy".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_match_mode: Option<String>,
}

/// Configuration for the healing loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopConfig {
    /// Polling interval in seconds for daemon mode.
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    /// Maximum number of sessions to scan per poll.
    #[serde(default = "default_context_limit")]
    pub context_limit: u32,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
    /// Maximum transcript length (characters) sent to LLM.
    #[serde(default = "default_max_transcript_len")]
    pub max_transcript_len: usize,
    /// Topic creation regime: "aggressive", "balanced", or "conservative".
    #[serde(default)]
    pub regime: Regime,
    /// Optional dedupe threshold overrides applied on top of the regime preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe: Option<DedupeConfig>,
    /// Cross-conversation pattern detection settings.
    #[serde(default)]
    pub patterns: PatternConfig,
}

fn default_context_limit() -> u32 {
    20
}

fn default_interval_secs() -> u64 {
    300
}

fn default_confidence_threshold() -> f64 {
    0.7
}

fn default_max_transcript_len() -> usize {
    30000
}

impl Default for LoopConfig {
    fn default() -> Self {
        LoopConfig {
            context_limit: default_context_limit(),
            interval_secs: default_interval_secs(),
            confidence_threshold: default_confidence_threshold(),
            max_transcript_len: default_max_transcript_len(),
            regime: Regime::default(),
            dedupe: None,
            patterns: PatternConfig::default(),
        }
    }
}

impl LoopConfig {
    /// Build a DedupePolicy from the configured regime + any overrides.
    /// Accepts an optional CLI-level regime override (highest priority).
    pub fn build_dedupe_policy(
        &self,
        cli_regime: Option<&Regime>,
    ) -> std::result::Result<DedupePolicy, String> {
        let regime = cli_regime.copied().unwrap_or(self.regime);
        let policy = DedupePolicy::from_regime(regime);

        if let Some(ref dc) = self.dedupe {
            policy.with_overrides(
                dc.coverage_threshold,
                dc.tag_overlap_minimum,
                dc.require_slug_affinity,
                dc.title_match_mode.clone(),
            )
        } else {
            Ok(policy)
        }
    }
}

/// Configuration for the LLM used by the healing loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: LlmProvider,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_model() -> String {
    LlmProvider::default().default_model().to_string()
}

fn default_max_tokens() -> u32 {
    4096
}

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            provider: LlmProvider::default(),
            model: default_model(),
            max_tokens: default_max_tokens(),
        }
    }
}


/// Global configuration for the oversight system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Root directory for the knowledge base (contains topics/, index.json).
    pub kb_path: PathBuf,
    /// Healing loop configuration.
    #[serde(default, rename = "loop")]
    pub loop_config: LoopConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

impl Default for Config {
    fn default() -> Self {
        let home = home_dir();
        Config {
            kb_path: home.join(".oversight").join("kb"),
            loop_config: LoopConfig::default(),
            llm: LlmConfig::default(),
        }
    }
}

impl Config {
    /// Resolve configuration with precedence: cli_override > env > config file > default.
    pub fn resolve(cli_override: Option<&Path>) -> Result<Config> {
        // Start with base config from file or default
        let mut config = {
            let config_path = Self::config_file_path();
            if config_path.exists() {
                Self::from_file(&config_path)?
            } else {
                Config::default()
            }
        };

        // Environment variable overrides kb_path
        if let Ok(env_path) = std::env::var("OVERSIGHT_KB_PATH") {
            config.kb_path = PathBuf::from(env_path);
        }

        // CLI flag takes highest priority for kb_path
        if let Some(path) = cli_override {
            config.kb_path = path.to_path_buf();
        }

        Ok(config)
    }

    /// Return the path to the state file (~/.oversight/state.json).
    pub fn state_path() -> PathBuf {
        Self::oversight_root().join("state.json")
    }

    /// Returns the path to the global config file (~/.oversight/config.toml).
    pub fn config_file_path() -> PathBuf {
        let home = home_dir();
        home.join(".oversight").join("config.toml")
    }

    /// Returns the path to the oversight root directory (~/.oversight/).
    pub fn oversight_root() -> PathBuf {
        let home = home_dir();
        home.join(".oversight")
    }

    /// Load config from a TOML file.
    pub fn from_file(path: &Path) -> Result<Config> {
        let contents = std::fs::read_to_string(path)?;
        let file_config: FileConfig = toml::from_str(&contents)?;

        let mut config = Config::default();
        if let Some(kb_path) = file_config.kb_path {
            config.kb_path = PathBuf::from(expand_tilde(&kb_path));
        }
        if let Some(loop_cfg) = file_config.loop_config {
            if let Some(limit) = loop_cfg.context_limit {
                config.loop_config.context_limit = limit;
            }
            if let Some(interval) = loop_cfg.interval_secs {
                config.loop_config.interval_secs = interval;
            }
            if let Some(threshold) = loop_cfg.confidence_threshold {
                config.loop_config.confidence_threshold = threshold;
            }
            if let Some(max_len) = loop_cfg.max_transcript_len {
                config.loop_config.max_transcript_len = max_len;
            }
            if let Some(regime) = loop_cfg.regime {
                config.loop_config.regime = regime;
            }
            if let Some(dedupe_cfg) = loop_cfg.dedupe {
                config.loop_config.dedupe = Some(DedupeConfig {
                    coverage_threshold: dedupe_cfg.coverage_threshold,
                    tag_overlap_minimum: dedupe_cfg.tag_overlap_minimum,
                    require_slug_affinity: dedupe_cfg.require_slug_affinity,
                    title_match_mode: dedupe_cfg.title_match_mode,
                });
            }
            if let Some(pat_cfg) = loop_cfg.patterns {
                if let Some(enabled) = pat_cfg.enabled {
                    config.loop_config.patterns.enabled = enabled;
                }
                if let Some(threshold) = pat_cfg.similarity_threshold {
                    config.loop_config.patterns.similarity_threshold = threshold;
                }
                if let Some(min_occ) = pat_cfg.min_occurrences {
                    config.loop_config.patterns.min_occurrences = min_occ;
                }
                if let Some(max_clusters) = pat_cfg.max_clusters_per_pass {
                    config.loop_config.patterns.max_clusters_per_pass = max_clusters;
                }
            }
        }
        if let Some(llm_cfg) = file_config.llm {
            let mut provider_changed = false;
            if let Some(provider) = llm_cfg.provider {
                config.llm.provider = provider;
                provider_changed = true;
            }
            if let Some(model) = llm_cfg.model {
                config.llm.model = model;
            } else if provider_changed {
                config.llm.model = config.llm.provider.default_model().to_string();
            }
            if let Some(max_tokens) = llm_cfg.max_tokens {
                config.llm.max_tokens = max_tokens;
            }
        }
        Ok(config)
    }

    /// Write config to a TOML file.
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let file_config = FileConfig {
            kb_path: Some(self.kb_path.to_string_lossy().to_string()),
            loop_config: Some(FileLoopConfig {
                context_limit: Some(self.loop_config.context_limit),
                interval_secs: Some(self.loop_config.interval_secs),
                confidence_threshold: Some(self.loop_config.confidence_threshold),
                max_transcript_len: Some(self.loop_config.max_transcript_len),
                regime: Some(self.loop_config.regime),
                dedupe: self.loop_config.dedupe.as_ref().map(|d| FileDedupeConfig {
                    coverage_threshold: d.coverage_threshold,
                    tag_overlap_minimum: d.tag_overlap_minimum,
                    require_slug_affinity: d.require_slug_affinity,
                    title_match_mode: d.title_match_mode.clone(),
                }),
                patterns: Some(FilePatternConfig {
                    enabled: Some(self.loop_config.patterns.enabled),
                    similarity_threshold: Some(self.loop_config.patterns.similarity_threshold),
                    min_occurrences: Some(self.loop_config.patterns.min_occurrences),
                    max_clusters_per_pass: Some(self.loop_config.patterns.max_clusters_per_pass),
                }),
            }),
            llm: Some(FileLlmConfig {
                provider: Some(self.llm.provider),
                model: Some(self.llm.model.clone()),
                max_tokens: Some(self.llm.max_tokens),
            }),
        };
        let contents = toml::to_string_pretty(&file_config)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp_name = path
            .file_name()
            .map(|name| format!("{}.tmp", name.to_string_lossy()))
            .unwrap_or_else(|| "config.toml.tmp".to_string());
        let temp_path = path.with_file_name(temp_name);
        std::fs::write(&temp_path, contents)?;
        if let Err(err) = std::fs::rename(&temp_path, path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err.into());
        }
        Ok(())
    }

    /// Return the topics directory path.
    pub fn topics_dir(&self) -> PathBuf {
        self.kb_path.join("topics")
    }

    /// Return the index file path.
    pub fn index_path(&self) -> PathBuf {
        self.kb_path.join("index.json")
    }

    /// Build the transcript source based on config.
    ///
    /// Defaults to Claude Code local logs. Set `OVERSIGHT_SOURCE` to
    /// `codex`, `crush`, or `gemini` to use other providers.
    pub fn build_source(&self) -> crate::source::TranscriptSource {
        let source_env = std::env::var("OVERSIGHT_SOURCE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match source_env.as_str() {
            "codex" => crate::source::TranscriptSource::Codex(
                crate::source::providers::codex::CodexSource::new(
                    crate::source::providers::codex::CodexSource::default_sessions_dir(),
                ),
            ),
            "crush" => crate::source::TranscriptSource::Crush(
                crate::source::providers::crush::CrushSource::new(
                    crate::source::providers::crush::CrushSource::default_projects_json(),
                ),
            ),
            "gemini" => crate::source::TranscriptSource::Gemini(
                crate::source::providers::gemini::GeminiSource::new(
                    crate::source::providers::gemini::GeminiSource::default_tmp_dir(),
                ),
            ),
            "opencode" => crate::source::TranscriptSource::OpenCode(
                crate::source::providers::opencode::OpenCodeSource::new(
                    crate::source::providers::opencode::OpenCodeSource::default_config_dir(),
                ),
            ),
            _ => crate::source::TranscriptSource::ClaudeCode(
                crate::source::providers::claude_code::ClaudeCodeSource::new(
                    crate::source::providers::claude_code::ClaudeCodeSource::default_projects_dir(),
                ),
            ),
        }
    }
}

/// Intermediate struct for TOML deserialization (allows partial config).
#[derive(Debug, Deserialize, Serialize)]
struct FileConfig {
    kb_path: Option<String>,
    #[serde(default, rename = "loop")]
    loop_config: Option<FileLoopConfig>,
    #[serde(default)]
    llm: Option<FileLlmConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileLoopConfig {
    context_limit: Option<u32>,
    interval_secs: Option<u64>,
    confidence_threshold: Option<f64>,
    max_transcript_len: Option<usize>,
    regime: Option<Regime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dedupe: Option<FileDedupeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patterns: Option<FilePatternConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileDedupeConfig {
    coverage_threshold: Option<f64>,
    tag_overlap_minimum: Option<usize>,
    require_slug_affinity: Option<bool>,
    title_match_mode: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileLlmConfig {
    provider: Option<LlmProvider>,
    model: Option<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FilePatternConfig {
    enabled: Option<bool>,
    similarity_threshold: Option<f64>,
    min_occurrences: Option<usize>,
    max_clusters_per_pass: Option<usize>,
}


/// Return the user's home directory, falling back to "." if unavailable.
fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Expand ~ at the start of a path string.
fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.display().to_string();
        }
    } else if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.kb_path.to_string_lossy().contains(".oversight"));
        assert!(config.kb_path.to_string_lossy().ends_with("kb"));
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/foo/bar");
        assert!(!expanded.starts_with("~/"));
        assert!(expanded.ends_with("/foo/bar"));
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        let expanded = expand_tilde("/absolute/path");
        assert_eq!(expanded, "/absolute/path");
    }

    #[test]
    fn test_expand_tilde_lone() {
        let expanded = expand_tilde("~");
        assert!(!expanded.starts_with('~'));
        assert!(!expanded.is_empty());
    }

    #[test]
    fn test_from_file_provider_without_model_uses_provider_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[llm]
provider = "openai"
"#,
        )
        .unwrap();

        let config = Config::from_file(&path).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::OpenAI);
        assert_eq!(config.llm.model, LlmProvider::OpenAI.default_model());
    }
}
