use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::error::{Error, Result};
use crate::healing_loop::discovery::Candidate;
use crate::source::types::TypedTurn;
use crate::source::{context_id_from_uuid, make_context_summary};
use crate::state::LoopState;

pub struct OpenCodeSource {
    config_dir: PathBuf,
}

impl OpenCodeSource {
    pub fn new(config_dir: PathBuf) -> Self {
        OpenCodeSource { config_dir }
    }

    pub fn default_config_dir() -> PathBuf {
        std::env::var("OPENCODE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
                    .join("opencode")
            })
    }

    pub fn database_paths(&self) -> Vec<PathBuf> {
        let dbs = self.find_databases();
        dbs.into_iter().map(|db| db.parent().unwrap_or(&db).to_path_buf()).collect()
    }

    pub fn discover_candidates(
        &self,
        state: &LoopState,
        limit: u32,
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        let db_paths = self.find_databases();
        if db_paths.is_empty() {
            return Ok(candidates);
        }

        for db_path in db_paths {
            let sessions = match self.list_sessions(&db_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for session in sessions {
                let ctx_id = context_id_from_uuid(&session.id);
                let head_turn_id = session.time_updated;

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
                    Some("opencode".to_string()),
                );
                candidates.push(Candidate {
                    context,
                    head_turn_id,
                    source_path: Some(db_path.clone()),
                    project_path: None,
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
        .map_err(|e| {
            Error::Config(format!(
                "Failed to open opencode DB {}: {e}",
                db_path.display()
            ))
        })?;

        let mut msg_stmt = conn
            .prepare(
                "SELECT id, data FROM message WHERE session_id = ?1 ORDER BY time_created",
            )
            .map_err(|e| Error::Config(format!("Failed to query opencode messages: {e}")))?;

        let mut part_stmt = conn
            .prepare(
                "SELECT data FROM part WHERE message_id = ?1 ORDER BY time_created",
            )
            .map_err(|e| Error::Config(format!("Failed to query opencode parts: {e}")))?;

        let mut turns = Vec::new();
        let mut turn_id: u64 = 0;

        let msg_rows = msg_stmt
            .query_map([&session_id], |row| {
                let id: String = row.get(0)?;
                let data: String = row.get(1)?;
                Ok((id, data))
            })
            .map_err(|e| Error::Config(format!("Failed to read opencode messages: {e}")))?;

        for msg_row in msg_rows {
            let (msg_id, msg_data_str) = match msg_row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let msg_data: serde_json::Value = match serde_json::from_str(&msg_data_str) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let role = msg_data
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let part_rows = part_stmt
                .query_map([&msg_id], |row| {
                    let data: String = row.get(0)?;
                    Ok(data)
                })
                .map_err(|e| Error::Config(format!("Failed to read opencode parts: {e}")))?;

            for part_row in part_rows {
                let part_data_str = match part_row {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let part: serde_json::Value = match serde_json::from_str(&part_data_str) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match (role, part_type) {
                    (_, "text") => {
                        let text = part
                            .get("text")
                            .or_else(|| part.get("content"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if text.is_empty() {
                            continue;
                        }
                        turn_id += 1;
                        let item_type = if role == "user" {
                            "user_input"
                        } else {
                            "assistant"
                        };
                        turns.push(TypedTurn {
                            turn_id: Some(turn_id),
                            depth: None,
                            data: json!({
                                "item_type": item_type,
                                item_type: { "text": text }
                            }),
                            declared_type: None,
                        });
                    }
                    (_, "tool") => {
                        let tool_name = part
                            .get("tool")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let call_id = part
                            .get("callID")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let state_obj = part.get("state").cloned().unwrap_or(json!({}));
                        let status = state_obj
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let input = state_obj
                            .get("input")
                            .cloned()
                            .unwrap_or(json!({}));
                        let args = serde_json::to_string(&input).unwrap_or_default();

                        turn_id += 1;
                        turns.push(TypedTurn {
                            turn_id: Some(turn_id),
                            depth: None,
                            data: json!({
                                "item_type": "tool_call",
                                "tool_call": {
                                    "call_id": call_id,
                                    "name": tool_name,
                                    "args": args
                                }
                            }),
                            declared_type: None,
                        });

                        if status == "completed" || status == "error" {
                            let content = if status == "error" {
                                state_obj
                                    .get("error")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string()
                            } else {
                                state_obj
                                    .get("output")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string()
                            };

                            turn_id += 1;
                            turns.push(TypedTurn {
                                turn_id: Some(turn_id),
                                depth: None,
                                data: json!({
                                    "item_type": "tool_result",
                                    "tool_result": {
                                        "call_id": call_id,
                                        "content": content,
                                        "is_error": status == "error"
                                    }
                                }),
                                declared_type: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(turns)
    }

    fn find_databases(&self) -> Vec<PathBuf> {
        let mut db_paths = Vec::new();

        // Check the config dir itself
        let global_db = self.config_dir.join("opencode.db");
        if global_db.exists() {
            db_paths.push(global_db);
        }

        // Scan home directory for project-level .opencode/ dirs
        if let Some(home) = dirs::home_dir() {
            let docs = home.join("Documents");
            if docs.is_dir() {
                if let Ok(entries) = fs::read_dir(&docs) {
                    for entry in entries.flatten() {
                        let db = entry.path().join(".opencode").join("opencode.db");
                        if db.exists() {
                            db_paths.push(db);
                        }
                    }
                }
            }
        }

        db_paths
    }

    fn list_sessions(&self, db_path: &PathBuf) -> Result<Vec<SessionInfo>> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| {
            Error::Config(format!(
                "Failed to open opencode DB {}: {e}",
                db_path.display()
            ))
        })?;

        // Check if session table exists
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !table_exists {
            return Ok(Vec::new());
        }

        let mut stmt = conn
            .prepare(
                "SELECT id, title, time_updated FROM session \
                 WHERE time_archived IS NULL \
                 ORDER BY time_updated DESC",
            )
            .map_err(|e| Error::Config(format!("Failed to query opencode sessions: {e}")))?;

        let sessions = stmt
            .query_map([], |row| {
                Ok(SessionInfo {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    time_updated: row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(|e| Error::Config(format!("Failed to read opencode sessions: {e}")))?
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
            "OpenCode session not found for context_id {ctx_id}"
        )))
    }
}

struct SessionInfo {
    id: String,
    title: String,
    time_updated: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_no_databases() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = OpenCodeSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_discover_finds_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("opencode.db");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE project (id TEXT PRIMARY KEY);
             INSERT INTO project (id) VALUES ('proj-1');
             CREATE TABLE session (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                workspace_id TEXT,
                parent_id TEXT,
                slug TEXT NOT NULL,
                directory TEXT NOT NULL,
                title TEXT NOT NULL,
                version TEXT NOT NULL,
                share_url TEXT,
                summary_additions INTEGER,
                summary_deletions INTEGER,
                summary_files INTEGER,
                summary_diffs TEXT,
                revert TEXT,
                permission TEXT,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                time_compacting INTEGER,
                time_archived INTEGER
             );
             INSERT INTO session (id, project_id, slug, directory, title, version, time_created, time_updated)
             VALUES ('sess-1', 'proj-1', 'test', '/tmp', 'Test session', '1', 1000, 2000);",
        )
        .unwrap();

        let src = OpenCodeSource::new(tmp.path().to_path_buf());
        let state = LoopState::default();
        let candidates = src.discover_candidates(&state, 10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].context.client_tag.as_deref(),
            Some("opencode")
        );
    }
}
