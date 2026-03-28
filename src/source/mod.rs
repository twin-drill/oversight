pub mod providers;
pub mod types;

use types::{ContextSummary, TypedTurn};
use crate::error::Result;
use crate::healing_loop::discovery::Candidate;
use crate::state::LoopState;
use std::path::PathBuf;

use providers::claude_code::ClaudeCodeSource;
use providers::codex::CodexSource;
use providers::crush::CrushSource;
use providers::gemini::GeminiSource;
use providers::opencode::OpenCodeSource;

pub enum TranscriptSource {
    ClaudeCode(ClaudeCodeSource),
    Codex(CodexSource),
    Crush(CrushSource),
    Gemini(GeminiSource),
    OpenCode(OpenCodeSource),
}

impl TranscriptSource {
    pub async fn discover_candidates(
        &self,
        state: &LoopState,
        limit: u32,
    ) -> Result<Vec<Candidate>> {
        match self {
            TranscriptSource::ClaudeCode(src) => src.discover_candidates(state, limit),
            TranscriptSource::Codex(src) => src.discover_candidates(state, limit),
            TranscriptSource::Crush(src) => src.discover_candidates(state, limit),
            TranscriptSource::Gemini(src) => src.discover_candidates(state, limit),
            TranscriptSource::OpenCode(src) => src.discover_candidates(state, limit),
        }
    }

    pub async fn get_turns(&self, candidate: &Candidate) -> Result<Vec<TypedTurn>> {
        match self {
            TranscriptSource::ClaudeCode(src) => src.get_turns(candidate),
            TranscriptSource::Codex(src) => src.get_turns(candidate),
            TranscriptSource::Crush(src) => src.get_turns(candidate),
            TranscriptSource::Gemini(src) => src.get_turns(candidate),
            TranscriptSource::OpenCode(src) => src.get_turns(candidate),
        }
    }

    pub fn source_name(&self) -> &'static str {
        match self {
            TranscriptSource::ClaudeCode(_) => "claude-code",
            TranscriptSource::Codex(_) => "codex",
            TranscriptSource::Crush(_) => "crush",
            TranscriptSource::Gemini(_) => "gemini",
            TranscriptSource::OpenCode(_) => "opencode",
        }
    }

    /// Return directories/files to watch for filesystem change notification.
    ///
    /// These are the root paths where the provider stores conversation data.
    /// The watcher should use recursive mode on directories.
    pub fn watch_paths(&self) -> Vec<PathBuf> {
        match self {
            TranscriptSource::ClaudeCode(src) => vec![src.root_dir().to_path_buf()],
            TranscriptSource::Codex(src) => vec![src.root_dir().to_path_buf()],
            TranscriptSource::Crush(src) => {
                let mut paths = vec![src.manifest_path().to_path_buf()];
                if let Ok(db_dirs) = src.database_dirs() {
                    paths.extend(db_dirs);
                }
                paths
            }
            TranscriptSource::Gemini(src) => vec![src.root_dir().to_path_buf()],
            TranscriptSource::OpenCode(src) => src.database_paths(),
        }
    }
}

pub fn context_id_from_uuid(uuid: &str) -> u64 {
    use sha2::{Digest, Sha256};

    let hash = Sha256::digest(uuid.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hash[..8]);
    u64::from_le_bytes(bytes)
}

pub fn make_context_summary(
    context_id: u64,
    head_turn_id: u64,
    title: Option<String>,
    client_tag: Option<String>,
) -> ContextSummary {
    ContextSummary {
        context_id,
        head_turn_id: Some(head_turn_id),
        head_depth: None,
        created_at_unix_ms: None,
        title,
        client_tag,
        is_live: false,
    }
}
