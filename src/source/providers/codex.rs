use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::source::types::TypedTurn;
use crate::error::{Error, Result};
use crate::healing_loop::discovery::Candidate;
use crate::source::{context_id_from_uuid, make_context_summary};
use crate::state::LoopState;

pub struct CodexSource {
    sessions_dir: PathBuf,
}

impl CodexSource {
    pub fn new(sessions_dir: PathBuf) -> Self {
        CodexSource { sessions_dir }
    }

    pub fn default_sessions_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
            .join("sessions")
    }

    pub fn discover_candidates(
        &self,
        state: &LoopState,
        limit: u32,
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        if !self.sessions_dir.exists() {
            return Ok(candidates);
        }

        let mut sessions = self.collect_sessions()?;
        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));

        for session in sessions {
            let ctx_id = context_id_from_uuid(&session.session_id);
            let head_turn_id = session.file_size;

            if state.is_processed(ctx_id, head_turn_id) {
                continue;
            }

            let context = make_context_summary(
                ctx_id,
                head_turn_id,
                Some(session.session_id.clone()),
                Some("codex".to_string()),
            );
            candidates.push(Candidate {
                context,
                head_turn_id,
                source_path: Some(session.path.clone()),
            });

            if candidates.len() >= limit as usize {
                break;
            }
        }

        Ok(candidates)
    }

    pub fn get_turns(&self, candidate: &Candidate) -> Result<Vec<TypedTurn>> {
        let path = match &candidate.source_path {
            Some(p) => p.clone(),
            None => self.find_session_file(candidate.context.id())?,
        };
        let content = fs::read_to_string(&path).map_err(|e| {
            Error::Config(format!("Failed to read session file {}: {e}", path.display()))
        })?;
        parse_jsonl(&content)
    }

    fn collect_sessions(&self) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();

        let entries = fs::read_dir(&self.sessions_dir).map_err(|e| {
            Error::Config(format!(
                "Failed to read Codex sessions dir {}: {e}",
                self.sessions_dir.display()
            ))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "jsonl") != Some(true) {
                continue;
            }

            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if session_id.is_empty() {
                continue;
            }

            let metadata = fs::metadata(&path).ok();
            let file_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = metadata
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            if file_size == 0 {
                continue;
            }

            sessions.push(SessionInfo {
                session_id,
                path,
                file_size,
                modified,
            });
        }

        Ok(sessions)
    }

    fn find_session_file(&self, ctx_id: u64) -> Result<PathBuf> {
        let sessions = self.collect_sessions()?;
        for session in &sessions {
            if context_id_from_uuid(&session.session_id) == ctx_id {
                return Ok(session.path.clone());
            }
        }
        Err(Error::Config(format!(
            "Codex session file not found for context_id {ctx_id}"
        )))
    }
}

struct SessionInfo {
    session_id: String,
    path: PathBuf,
    file_size: u64,
    modified: std::time::SystemTime,
}

fn parse_jsonl(content: &str) -> Result<Vec<TypedTurn>> {
    let mut turns = Vec::new();
    let mut turn_id: u64 = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match entry_type {
            "function_call_requested" => {
                let data = match entry.get("data") {
                    Some(d) => d,
                    None => continue,
                };
                turn_id += 1;
                let name = data
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let call_id = data
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = data.get("arguments").cloned().unwrap_or(json!({}));
                let args_str = serde_json::to_string(&args).unwrap_or_default();

                turns.push(TypedTurn {
                    turn_id: Some(turn_id),
                    depth: None,
                    data: json!({
                        "item_type": "tool_call",
                        "tool_call": {
                            "call_id": call_id,
                            "name": name,
                            "args": args_str
                        }
                    }),
                    declared_type: None,
                });
            }
            "function_call_completed" => {
                let data = match entry.get("data") {
                    Some(d) => d,
                    None => continue,
                };
                turn_id += 1;
                let call_id = data
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let output = data
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                turns.push(TypedTurn {
                    turn_id: Some(turn_id),
                    depth: None,
                    data: json!({
                        "item_type": "tool_result",
                        "tool_result": {
                            "call_id": call_id,
                            "content": output,
                            "is_error": false
                        }
                    }),
                    declared_type: None,
                });
            }
            "error" => {
                let data = entry.get("data").cloned().unwrap_or(json!({}));
                let message = data
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                turn_id += 1;
                turns.push(TypedTurn {
                    turn_id: Some(turn_id),
                    depth: None,
                    data: json!({
                        "item_type": "system",
                        "system": {
                            "kind": "error",
                            "content": message
                        }
                    }),
                    declared_type: None,
                });
            }
            "response.completed" => {
                let data = match entry.get("data") {
                    Some(d) => d,
                    None => continue,
                };
                let output = match data.get("output").and_then(|v| v.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };
                for item in output {
                    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if item_type == "message" {
                        let Some(blocks) = item.get("content").and_then(|c| c.as_array()) else {
                            continue;
                        };
                        for block in blocks {
                            let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if block_type == "output_text" {
                                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                if !text.is_empty() {
                                    turn_id += 1;
                                    turns.push(TypedTurn {
                                        turn_id: Some(turn_id),
                                        depth: None,
                                        data: json!({
                                            "item_type": "assistant",
                                            "assistant": { "text": text }
                                        }),
                                        declared_type: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(turns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::types::TurnType;

    #[test]
    fn test_parse_function_call_and_result() {
        let jsonl = r#"{"type":"function_call_requested","data":{"name":"shell","call_id":"call_1","arguments":{"command":["bash","-c","ls"]}}}
{"type":"function_call_completed","data":{"call_id":"call_1","output":"file1.txt\nfile2.txt"}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].turn_type(), TurnType::ToolCall);
        assert_eq!(turns[0].tool_name(), Some("shell".to_string()));
        assert_eq!(turns[1].turn_type(), TurnType::ToolResult);
        assert!(turns[1].text_content().contains("file1.txt"));
    }

    #[test]
    fn test_parse_error() {
        let jsonl = r#"{"type":"error","data":{"message":"context deadline exceeded"}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::System);
        assert!(turns[0].is_error());
        assert!(turns[0].text_content().contains("deadline exceeded"));
    }

    #[test]
    fn test_parse_response_completed_with_text() {
        let jsonl = r#"{"type":"response.completed","data":{"output":[{"type":"message","content":[{"type":"output_text","text":"I fixed the issue."}]}]}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::Assistant);
        assert!(turns[0].text_content().contains("fixed the issue"));
    }

    #[test]
    fn test_skips_delta_events() {
        let jsonl = r#"{"type":"response.reasoning_summary_text.delta","data":{}}
{"type":"response.output_text.delta","data":{}}
{"type":"response.function_call_arguments.delta","data":{}}
{"type":"function_call_requested","data":{"name":"shell","call_id":"c1","arguments":{"command":["ls"]}}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::ToolCall);
    }

    #[test]
    fn test_discover_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = CodexSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_discover_finds_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(
            tmp.path().join("test-session-abc.jsonl"),
            r#"{"type":"session_configured","data":{}}"#,
        )
        .unwrap();

        let src = CodexSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].context.client_tag.as_deref(), Some("codex"));
    }
}
