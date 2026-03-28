use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::source::types::TypedTurn;
use crate::error::{Error, Result};
use crate::healing_loop::discovery::Candidate;
use crate::source::{context_id_from_uuid, make_context_summary};
use crate::state::LoopState;

pub struct ClaudeCodeSource {
    projects_dir: PathBuf,
}

impl ClaudeCodeSource {
    pub fn new(projects_dir: PathBuf) -> Self {
        ClaudeCodeSource { projects_dir }
    }

    pub fn default_projects_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .join("projects")
    }

    pub fn root_dir(&self) -> &std::path::Path {
        &self.projects_dir
    }

    pub fn discover_candidates(
        &self,
        state: &LoopState,
        limit: u32,
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        if !self.projects_dir.exists() {
            return Ok(candidates);
        }

        let mut sessions = self.collect_sessions()?;
        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
        sessions.truncate(limit as usize);

        for session in sessions {
            let ctx_id = context_id_from_uuid(&session.session_id);
            let head_turn_id = session.file_size;

            if state.is_processed(ctx_id, head_turn_id) {
                continue;
            }

            let title = session.project_name.clone();
            let context = make_context_summary(
                ctx_id,
                head_turn_id,
                Some(title),
                Some("claude-code".to_string()),
            );
            candidates.push(Candidate {
                context,
                head_turn_id,
                source_path: Some(session.path.clone()),
                project_path: Some(session.project_name.clone()),
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

        let entries = fs::read_dir(&self.projects_dir).map_err(|e| {
            Error::Config(format!(
                "Failed to read Claude Code projects dir {}: {e}",
                self.projects_dir.display()
            ))
        })?;

        for entry in entries.flatten() {
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }

            let project_name = project_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .replace('-', "/");

            let jsonl_files = fs::read_dir(&project_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "jsonl")
                        .unwrap_or(false)
                });

            for file_entry in jsonl_files {
                let path = file_entry.path();
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
                    project_name: project_name.clone(),
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
            "Session file not found for context_id {ctx_id}"
        )))
    }
}

struct SessionInfo {
    session_id: String,
    project_name: String,
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
            "assistant" => {
                let message = match entry.get("message") {
                    Some(m) => m,
                    None => continue,
                };
                let blocks = match message.get("content").and_then(|c| c.as_array()) {
                    Some(b) => b,
                    None => continue,
                };

                for block in blocks {
                    let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match block_type {
                        "tool_use" => {
                            turn_id += 1;
                            let name = block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let input = block.get("input").cloned().unwrap_or(json!({}));
                            let args = serde_json::to_string(&input).unwrap_or_default();

                            turns.push(TypedTurn {
                                turn_id: Some(turn_id),
                                depth: None,
                                data: json!({
                                    "item_type": "tool_call",
                                    "tool_call": {
                                        "call_id": block.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                        "name": name,
                                        "args": args
                                    }
                                }),
                                declared_type: None,
                            });
                        }
                        "text" => {
                            let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                            if text.is_empty() {
                                continue;
                            }
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
                        _ => {}
                    }
                }
            }
            "user" => {
                let message = match entry.get("message") {
                    Some(m) => m,
                    None => {
                        let content = entry
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
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
                        continue;
                    }
                };

                let content = message.get("content");

                if let Some(serde_json::Value::String(text)) = content {
                    if !text.is_empty() {
                        turn_id += 1;
                        turns.push(TypedTurn {
                            turn_id: Some(turn_id),
                            depth: None,
                            data: json!({
                                "item_type": "user_input",
                                "user_input": { "text": text }
                            }),
                            declared_type: None,
                        });
                    }
                } else if let Some(serde_json::Value::Array(blocks)) = content {
                    for block in blocks {
                        let block_type =
                            block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if block_type == "tool_result" {
                            turn_id += 1;
                            let result_content = match block.get("content") {
                                Some(serde_json::Value::String(s)) => s.clone(),
                                Some(serde_json::Value::Array(arr)) => arr
                                    .iter()
                                    .filter_map(|b| {
                                        if b.get("type").and_then(|v| v.as_str()) == Some("text")
                                        {
                                            b.get("text").and_then(|v| v.as_str())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                _ => String::new(),
                            };
                            let is_error = block
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            turns.push(TypedTurn {
                                turn_id: Some(turn_id),
                                depth: None,
                                data: json!({
                                    "item_type": "tool_result",
                                    "tool_result": {
                                        "call_id": block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or(""),
                                        "content": result_content,
                                        "is_error": is_error
                                    }
                                }),
                                declared_type: None,
                            });
                        } else if block_type == "text" {
                            let text =
                                block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                            if !text.is_empty() {
                                turn_id += 1;
                                turns.push(TypedTurn {
                                    turn_id: Some(turn_id),
                                    depth: None,
                                    data: json!({
                                        "item_type": "user_input",
                                        "user_input": { "text": text }
                                    }),
                                    declared_type: None,
                                });
                            }
                        }
                    }
                }
            }
            "system" => {
                let content = entry
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let subtype = entry
                    .get("subtype")
                    .and_then(|v| v.as_str())
                    .unwrap_or("info");
                if content.is_empty() {
                    continue;
                }
                turn_id += 1;
                turns.push(TypedTurn {
                    turn_id: Some(turn_id),
                    depth: None,
                    data: json!({
                        "item_type": "system",
                        "system": {
                            "kind": subtype,
                            "content": content
                        }
                    }),
                    declared_type: None,
                });
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
    fn test_parse_tool_use_and_result() {
        let jsonl = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"file1.txt\nfile2.txt","is_error":false}]}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].turn_type(), TurnType::ToolCall);
        assert_eq!(turns[0].tool_name(), Some("Bash".to_string()));
        assert_eq!(turns[1].turn_type(), TurnType::ToolResult);
        assert!(!turns[1].is_error());
        assert!(turns[1].text_content().contains("file1.txt"));
    }

    #[test]
    fn test_parse_tool_result_error() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_02","content":"command not found","is_error":true}]}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::ToolResult);
        assert!(turns[0].is_error());
    }

    #[test]
    fn test_parse_user_text() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"fix the build"}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::UserInput);
        assert_eq!(turns[0].text_content(), "fix the build");
    }

    #[test]
    fn test_parse_assistant_text() {
        let jsonl = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I'll fix the error by updating the config."}]}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::Assistant);
        assert!(turns[0].text_content().contains("fix the error"));
    }

    #[test]
    fn test_parse_system_message() {
        let jsonl = r#"{"type":"system","subtype":"local_command","content":"model switched to opus"}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::System);
    }

    #[test]
    fn test_skips_progress_and_snapshots() {
        let jsonl = r#"{"type":"progress","content":""}
{"type":"file-history-snapshot","messageId":"abc","snapshot":{}}
{"type":"user","message":{"role":"user","content":"hello"}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::UserInput);
    }

    #[test]
    fn test_skips_thinking_blocks() {
        let jsonl = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"let me think..."},{"type":"text","text":"Here is my answer."}]}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_type(), TurnType::Assistant);
        assert!(turns[0].text_content().contains("my answer"));
    }

    #[test]
    fn test_sequential_turn_ids() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"hello"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#;

        let turns = parse_jsonl(jsonl).unwrap();
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].id(), 1);
        assert_eq!(turns[1].id(), 2);
        assert_eq!(turns[2].id(), 3);
    }

    #[test]
    fn test_discover_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = ClaudeCodeSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_discover_finds_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path().join("-test-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            project_dir.join("abc-123.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":"hello"}}"#,
        )
        .unwrap();

        let src = ClaudeCodeSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].context.client_tag.as_deref(),
            Some("claude-code")
        );
    }

    #[test]
    fn test_discover_skips_processed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path().join("-test-project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_file = project_dir.join("abc-123.jsonl");
        let content = r#"{"type":"user","message":{"role":"user","content":"hello"}}"#;
        fs::write(&session_file, content).unwrap();

        let ctx_id = context_id_from_uuid("abc-123");
        let file_size = fs::metadata(&session_file).unwrap().len();

        let mut state = LoopState::default();
        state.mark_processed(ctx_id, file_size);

        let src = ClaudeCodeSource::new(tmp.path().to_path_buf());
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }
}
