use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::error::{Error, Result};
use crate::healing_loop::discovery::Candidate;
use crate::source::types::TypedTurn;
use crate::source::{context_id_from_uuid, make_context_summary};
use crate::state::LoopState;

pub struct CrushSource {
    projects_json: PathBuf,
}

impl CrushSource {
    pub fn new(projects_json: PathBuf) -> Self {
        CrushSource { projects_json }
    }

    pub fn default_projects_json() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("share")
            .join("crush")
            .join("projects.json")
    }

    pub fn manifest_path(&self) -> &std::path::Path {
        &self.projects_json
    }

    pub fn database_dirs(&self) -> Result<Vec<PathBuf>> {
        let dbs = self.find_databases()?;
        Ok(dbs.into_iter().filter_map(|(db, _)| db.parent().map(|p| p.to_path_buf())).collect())
    }

    pub fn discover_candidates(
        &self,
        state: &LoopState,
        limit: u32,
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        let db_entries = match self.find_databases() {
            Ok(paths) => paths,
            Err(_) => return Ok(candidates),
        };

        for (db_path, project_path) in db_entries {
            let sessions = match self.list_sessions(&db_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for session in sessions {
                let ctx_id = context_id_from_uuid(&session.id);
                let head_turn_id = session.message_count;

                if head_turn_id == 0 {
                    continue;
                }

                if state.is_processed(ctx_id, head_turn_id) {
                    continue;
                }

                let context = make_context_summary(
                    ctx_id,
                    head_turn_id,
                    Some(session.title.clone()),
                    Some("crush".to_string()),
                );
                candidates.push(Candidate {
                    context,
                    head_turn_id,
                    source_path: Some(db_path.clone()),
                    project_path: project_path.clone(),
                });

                if candidates.len() >= limit as usize {
                    return Ok(candidates);
                }
            }
        }

        Ok(candidates)
    }

    pub fn get_turns(&self, candidate: &Candidate) -> Result<Vec<TypedTurn>> {
        let db_path = match &candidate.source_path {
            Some(p) => p.clone(),
            None => return Ok(Vec::new()),
        };

        let session_id = self.find_session_id(candidate.context.id(), &db_path)?;
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| Error::Config(format!("Failed to open crush DB {}: {e}", db_path.display())))?;

        let mut stmt = conn
            .prepare("SELECT role, parts FROM messages WHERE session_id = ?1 ORDER BY created_at")
            .map_err(|e| Error::Config(format!("Failed to query crush messages: {e}")))?;

        let mut turns = Vec::new();
        let mut turn_id: u64 = 0;

        let rows = stmt
            .query_map([&session_id], |row| {
                let role: String = row.get(0)?;
                let parts: String = row.get(1)?;
                Ok((role, parts))
            })
            .map_err(|e| Error::Config(format!("Failed to read crush messages: {e}")))?;

        for row in rows {
            let (role, parts_str) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let parts: Vec<serde_json::Value> = match serde_json::from_str(&parts_str) {
                Ok(p) => p,
                Err(_) => continue,
            };

            for part in &parts {
                let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let data = part.get("data").cloned().unwrap_or(json!({}));

                match (role.as_str(), part_type) {
                    ("user", "text") => {
                        let text = data.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if text.is_empty() {
                            continue;
                        }
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
                    ("assistant", "text") => {
                        let text = data.get("text").and_then(|v| v.as_str()).unwrap_or("");
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
                    ("assistant", "tool_call") => {
                        let name = data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let call_id = data
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let input = data
                            .get("input")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");

                        turn_id += 1;
                        turns.push(TypedTurn {
                            turn_id: Some(turn_id),
                            depth: None,
                            data: json!({
                                "item_type": "tool_call",
                                "tool_call": {
                                    "call_id": call_id,
                                    "name": name,
                                    "args": input
                                }
                            }),
                            declared_type: None,
                        });
                    }
                    ("tool", "tool_result") => {
                        let call_id = data
                            .get("tool_call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let content = data
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let is_error = content.contains("ERROR")
                            || content.contains("Exit code 1")
                            || content.contains("error:");

                        turn_id += 1;
                        turns.push(TypedTurn {
                            turn_id: Some(turn_id),
                            depth: None,
                            data: json!({
                                "item_type": "tool_result",
                                "tool_result": {
                                    "call_id": call_id,
                                    "content": content,
                                    "is_error": is_error
                                }
                            }),
                            declared_type: None,
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok(turns)
    }

    fn find_databases(&self) -> Result<Vec<(PathBuf, Option<String>)>> {
        if !self.projects_json.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.projects_json).map_err(|e| {
            Error::Config(format!(
                "Failed to read crush projects.json: {e}"
            ))
        })?;

        let parsed: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
            Error::Config(format!("Failed to parse crush projects.json: {e}"))
        })?;

        let projects = match parsed.get("projects").and_then(|v| v.as_array()) {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };

        let mut entries = Vec::new();
        for project in projects {
            let data_dir = project.get("data_dir").and_then(|v| v.as_str());
            let project_path = project.get("path").and_then(|v| v.as_str());
            if let Some(dd) = data_dir {
                let db_path = PathBuf::from(dd).join("crush.db");
                if db_path.exists() {
                    entries.push((db_path, project_path.map(|s| s.to_string())));
                }
            }
        }

        Ok(entries)
    }

    fn list_sessions(&self, db_path: &PathBuf) -> Result<Vec<SessionInfo>> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| Error::Config(format!("Failed to open crush DB {}: {e}", db_path.display())))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, title, message_count, updated_at FROM sessions \
                 WHERE message_count > 0 ORDER BY updated_at DESC",
            )
            .map_err(|e| Error::Config(format!("Failed to query crush sessions: {e}")))?;

        let sessions = stmt
            .query_map([], |row| {
                Ok(SessionInfo {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    message_count: row.get::<_, i64>(2)? as u64,
                    updated_at: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| Error::Config(format!("Failed to read crush sessions: {e}")))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }

    fn find_session_id(&self, ctx_id: u64, db_path: &PathBuf) -> Result<String> {
        let sessions = self.list_sessions(db_path)?;
        for session in &sessions {
            if context_id_from_uuid(&session.id) == ctx_id {
                return Ok(session.id.clone());
            }
        }
        Err(Error::Config(format!(
            "Crush session not found for context_id {ctx_id}"
        )))
    }
}

struct SessionInfo {
    id: String,
    title: String,
    message_count: u64,
    #[allow(dead_code)]
    updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_no_projects_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = CrushSource::new(tmp.path().join("nonexistent.json"));
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_discover_empty_projects() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects_json = tmp.path().join("projects.json");
        fs::write(&projects_json, r#"{"projects":[]}"#).unwrap();

        let src = CrushSource::new(projects_json);
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_discover_finds_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("project1").join(".crush");
        fs::create_dir_all(&data_dir).unwrap();

        let db_path = data_dir.join("crush.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                parent_session_id TEXT,
                title TEXT NOT NULL,
                message_count INTEGER NOT NULL DEFAULT 0,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                cost REAL NOT NULL DEFAULT 0.0,
                updated_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                summary_message_id TEXT,
                todos TEXT
            );
            INSERT INTO sessions (id, title, message_count, updated_at, created_at)
            VALUES ('sess-1', 'Test session', 5, 1000, 1000);"
        ).unwrap();

        let projects_json = tmp.path().join("projects.json");
        fs::write(
            &projects_json,
            serde_json::json!({
                "projects": [{
                    "path": tmp.path().join("project1").to_string_lossy(),
                    "data_dir": data_dir.to_string_lossy(),
                }]
            })
            .to_string(),
        )
        .unwrap();

        let src = CrushSource::new(projects_json);
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].context.client_tag.as_deref(), Some("crush"));
    }
}
