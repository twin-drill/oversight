use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::source::types::TypedTurn;
use crate::error::{Error, Result};
use crate::healing_loop::discovery::Candidate;
use crate::source::{context_id_from_uuid, make_context_summary};
use crate::state::LoopState;

pub struct GeminiSource {
    tmp_dir: PathBuf,
}

impl GeminiSource {
    pub fn new(tmp_dir: PathBuf) -> Self {
        GeminiSource { tmp_dir }
    }

    pub fn default_tmp_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gemini")
            .join("tmp")
    }

    pub fn root_dir(&self) -> &std::path::Path {
        &self.tmp_dir
    }

    pub fn discover_candidates(
        &self,
        state: &LoopState,
        limit: u32,
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        if !self.tmp_dir.exists() {
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
                Some("gemini".to_string()),
            );
            candidates.push(Candidate {
                context,
                head_turn_id,
                source_path: Some(session.path.clone()),
                    project_path: None,
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
        parse_session_json(&content)
    }

    fn collect_sessions(&self) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();

        let project_dirs = fs::read_dir(&self.tmp_dir)
            .map_err(|e| {
                Error::Config(format!(
                    "Failed to read Gemini tmp dir {}: {e}",
                    self.tmp_dir.display()
                ))
            })?;

        for project_entry in project_dirs.flatten() {
            let chats_dir = project_entry.path().join("chats");
            if !chats_dir.is_dir() {
                continue;
            }

            let chat_files = match fs::read_dir(&chats_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for file_entry in chat_files.flatten() {
                let path = file_entry.path();
                if path.extension().map(|e| e == "json") != Some(true) {
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
            "Gemini session file not found for context_id {ctx_id}"
        )))
    }
}

struct SessionInfo {
    session_id: String,
    path: PathBuf,
    file_size: u64,
    modified: std::time::SystemTime,
}

fn parse_session_json(content: &str) -> Result<Vec<TypedTurn>> {
    let data: serde_json::Value = serde_json::from_str(content).map_err(|e| {
        Error::Config(format!("Failed to parse Gemini session JSON: {e}"))
    })?;

    let messages = match data.get("messages").and_then(|m| m.as_array()) {
        Some(msgs) => msgs,
        None => return Ok(Vec::new()),
    };

    let mut turns = Vec::new();
    let mut turn_id: u64 = 0;

    for msg in messages {
        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "user" => {
                let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if !content.is_empty() {
                    turn_id += 1;
                    turns.push(TypedTurn {
                        turn_id: Some(turn_id),
                        depth: None,
                        data: json!({
                            "item_type": "user_input",
                            "user_input": { "text": content }
                        }),
                        declared_type: None,
                    });
                }
            }
            "gemini" => {
                let text = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
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

                if let Some(tool_calls) = msg.get("toolCalls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let call_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let args = tc.get("args").cloned().unwrap_or(json!({}));
                        let args_str = serde_json::to_string(&args).unwrap_or_default();

                        turn_id += 1;
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

                        if let Some(results) = tc.get("result").and_then(|v| v.as_array()) {
                            for result in results {
                                let fr = match result.get("functionResponse") {
                                    Some(fr) => fr,
                                    None => continue,
                                };
                                let output = fr
                                    .get("response")
                                    .and_then(|r| r.get("output"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");

                                turn_id += 1;
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
    fn test_parse_user_message() {
        let session = r#"{"sessionId":"test","messages":[
            {"type":"user","content":"fix the build"}
        ]}"#;

        let turns = parse_session_json(session).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::UserInput);
        assert_eq!(turns[0].text_content(), "fix the build");
    }

    #[test]
    fn test_parse_gemini_text() {
        let session = r#"{"sessionId":"test","messages":[
            {"type":"gemini","content":"I'll update the config file."}
        ]}"#;

        let turns = parse_session_json(session).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::Assistant);
        assert!(turns[0].text_content().contains("update the config"));
    }

    #[test]
    fn test_parse_tool_call_with_result() {
        let session = r#"{"sessionId":"test","messages":[
            {"type":"gemini","content":"","toolCalls":[{
                "id":"tc_1","name":"list_directory","args":{"dir_path":"src/"},
                "result":[{"functionResponse":{"id":"tc_1","name":"list_directory","response":{"output":"main.rs\nlib.rs"}}}]
            }]}
        ]}"#;

        let turns = parse_session_json(session).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].turn_type(), TurnType::ToolCall);
        assert_eq!(turns[0].tool_name(), Some("list_directory".to_string()));
        assert_eq!(turns[1].turn_type(), TurnType::ToolResult);
        assert!(turns[1].text_content().contains("main.rs"));
    }

    #[test]
    fn test_skips_info_messages() {
        let session = r#"{"sessionId":"test","messages":[
            {"type":"info","content":"Update available"},
            {"type":"user","content":"hello"}
        ]}"#;

        let turns = parse_session_json(session).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::UserInput);
    }

    #[test]
    fn test_discover_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = GeminiSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_discover_finds_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let chats_dir = tmp.path().join("abc123").join("chats");
        fs::create_dir_all(&chats_dir).unwrap();
        fs::write(
            chats_dir.join("session-2026-01-01.json"),
            r#"{"sessionId":"test","messages":[{"type":"user","content":"hi"}]}"#,
        )
        .unwrap();

        let src = GeminiSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].context.client_tag.as_deref(), Some("gemini"));
    }
}
