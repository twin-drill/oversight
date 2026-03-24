use serde::{Deserialize, Deserializer, Serialize};

/// Helper to deserialize a u64 that may be encoded as a string.
fn deserialize_string_u64<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("expected u64")),
        serde_json::Value::String(s) => s
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom(format!("invalid u64 string: {s}"))),
        _ => Err(serde::de::Error::custom("expected number or string")),
    }
}

fn deserialize_option_string_u64<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => Ok(n.as_u64()),
        Some(serde_json::Value::String(s)) => {
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<u64>()
                    .map(Some)
                    .map_err(|_| serde::de::Error::custom(format!("invalid u64 string: {s}")))
            }
        }
        _ => Err(serde::de::Error::custom("expected number, string, or null")),
    }
}

/// Summary of a conversation context (session).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSummary {
    /// Unique identifier for this context.
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub context_id: u64,
    /// ID of the most recent turn.
    #[serde(default, deserialize_with = "deserialize_option_string_u64")]
    pub head_turn_id: Option<u64>,
    /// Number of turns (depth) in the context.
    #[serde(default)]
    pub head_depth: Option<u64>,
    /// Creation timestamp in unix milliseconds.
    #[serde(default)]
    pub created_at_unix_ms: Option<u64>,
    /// Title from context metadata.
    #[serde(default)]
    pub title: Option<String>,
    /// Client tag (e.g., "claude", "codex").
    #[serde(default)]
    pub client_tag: Option<String>,
    /// Whether the context has an active session.
    #[serde(default)]
    pub is_live: bool,
}

impl ContextSummary {
    /// Convenience accessor matching the old `id` field name.
    pub fn id(&self) -> u64 {
        self.context_id
    }

    /// Convenience accessor for a human-readable label.
    pub fn label(&self) -> Option<&str> {
        self.title
            .as_deref()
            .or(self.client_tag.as_deref())
    }
}

/// Wrapper for the contexts list API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextsResponse {
    #[serde(default)]
    pub contexts: Vec<ContextSummary>,
    #[serde(default)]
    pub count: Option<u64>,
}

/// A single turn in a conversation transcript.
///
/// The turn type is embedded inside `data.item_type`. The `data` field
/// contains nested sub-objects like `tool_call`, `tool_result`, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedTurn {
    /// Turn sequence ID.
    #[serde(default, deserialize_with = "deserialize_option_string_u64")]
    pub turn_id: Option<u64>,
    /// Depth in the conversation tree.
    #[serde(default)]
    pub depth: Option<u64>,
    /// Projected JSON data for this turn.
    #[serde(default)]
    pub data: serde_json::Value,
    /// Declared type information.
    #[serde(default)]
    pub declared_type: Option<DeclaredType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclaredType {
    #[serde(default)]
    pub type_id: Option<String>,
    #[serde(default)]
    pub type_version: Option<u32>,
}

/// Wrapper for the typed turns API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedTurnsResponse {
    #[serde(default)]
    pub turns: Vec<TypedTurn>,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

/// The type of a conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TurnType {
    UserInput,
    AssistantTurn,
    ToolCall,
    ToolResult,
    System,
    Assistant,
    Handoff,
    #[default]
    #[serde(other)]
    Unknown,
}

impl TypedTurn {
    /// Convenience accessor for turn ID.
    pub fn id(&self) -> u64 {
        self.turn_id.unwrap_or(0)
    }

    /// Derive the turn type from `data.item_type`.
    pub fn turn_type(&self) -> TurnType {
        self.data
            .get("item_type")
            .and_then(|v| v.as_str())
            .map(|s| match s {
                "user_input" => TurnType::UserInput,
                "assistant_turn" => TurnType::AssistantTurn,
                "tool_call" => TurnType::ToolCall,
                "tool_result" => TurnType::ToolResult,
                "system" => TurnType::System,
                "assistant" => TurnType::Assistant,
                "handoff" => TurnType::Handoff,
                _ => TurnType::Unknown,
            })
            .unwrap_or(TurnType::Unknown)
    }

    /// Extract a text summary from the turn's data.
    pub fn text_content(&self) -> String {
        let item_type = self.turn_type();

        match item_type {
            TurnType::UserInput => {
                if let Some(ui) = self.data.get("user_input") {
                    if let Some(text) = ui.get("text").and_then(|v| v.as_str()) {
                        return text.to_string();
                    }
                }
            }
            TurnType::ToolCall => {
                if let Some(tc) = self.data.get("tool_call") {
                    let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let args = tc.get("args").and_then(|v| v.as_str()).unwrap_or("");
                    return format!("{name}: {args}");
                }
            }
            TurnType::ToolResult => {
                if let Some(tr) = self.data.get("tool_result") {
                    if let Some(content) = tr.get("content").and_then(|v| v.as_str()) {
                        let is_error = tr.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                        if is_error {
                            return format!("ERROR: {content}");
                        }
                        return content.to_string();
                    }
                }
            }
            TurnType::Assistant => {
                if let Some(a) = self.data.get("assistant") {
                    if let Some(text) = a.get("text").and_then(|v| v.as_str()) {
                        return text.to_string();
                    }
                }
            }
            TurnType::AssistantTurn => {
                if let Some(t) = self.data.get("turn") {
                    if let Some(text) = t.get("text").and_then(|v| v.as_str()) {
                        return text.to_string();
                    }
                }
            }
            TurnType::System => {
                if let Some(s) = self.data.get("system") {
                    if let Some(content) = s.get("content").and_then(|v| v.as_str()) {
                        let kind = s.get("kind").and_then(|v| v.as_str()).unwrap_or("info");
                        return format!("[{kind}] {content}");
                    }
                }
            }
            _ => {}
        }

        // Fallback: try flat fields for backwards compat
        if let Some(text) = self.data.get("text").and_then(|v| v.as_str()) {
            return text.to_string();
        }
        if let Some(content) = self.data.get("content").and_then(|v| v.as_str()) {
            return content.to_string();
        }
        if let Some(output) = self.data.get("output").and_then(|v| v.as_str()) {
            return output.to_string();
        }
        if let Some(error) = self.data.get("error").and_then(|v| v.as_str()) {
            return format!("ERROR: {error}");
        }
        if let Some(s) = self.data.as_str() {
            return s.to_string();
        }
        if !self.data.is_null() {
            return serde_json::to_string(&self.data).unwrap_or_default();
        }
        String::new()
    }

    /// Extract the tool name from a tool_call turn.
    pub fn tool_name(&self) -> Option<String> {

        if let Some(tc) = self.data.get("tool_call") {
            if let Some(name) = tc.get("name").and_then(|v| v.as_str()) {
                return Some(name.to_string());
            }
        }
        // Fallback: flat fields
        self.data
            .get("tool")
            .or_else(|| self.data.get("name"))
            .or_else(|| self.data.get("tool_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Check if this turn represents an error.
    pub fn is_error(&self) -> bool {

        if let Some(tr) = self.data.get("tool_result") {
            if tr.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
                return true;
            }
        }

        if let Some(s) = self.data.get("system") {
            if let Some(kind) = s.get("kind").and_then(|v| v.as_str()) {
                if kind == "error" {
                    return true;
                }
            }
        }
        // Fallback: flat fields
        if self.data.get("error").is_some() {
            return true;
        }
        if let Some(status) = self.data.get("status").and_then(|v| v.as_str()) {
            if status == "error" || status == "failed" {
                return true;
            }
        }
        if let Some(exit_code) = self.data.get("exit_code").and_then(|v| v.as_i64()) {
            return exit_code != 0;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_summary_string_ids() {
        let json = r#"{"context_id": "42", "head_turn_id": "100", "head_depth": 10, "created_at_unix_ms": 1700000000000, "is_live": false}"#;
        let ctx: ContextSummary = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.id(), 42);
        assert_eq!(ctx.head_turn_id, Some(100));
        assert_eq!(ctx.head_depth, Some(10));
    }

    #[test]
    fn test_context_summary_numeric_ids() {
        let json = r#"{"context_id": 42, "head_turn_id": 100, "head_depth": 10}"#;
        let ctx: ContextSummary = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.id(), 42);
        assert_eq!(ctx.head_turn_id, Some(100));
    }

    #[test]
    fn test_context_summary_with_title() {
        let json = r#"{"context_id": "1", "title": "my-project", "client_tag": "claude"}"#;
        let ctx: ContextSummary = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.label(), Some("my-project"));
    }

    #[test]
    fn test_context_summary_label_fallback_to_tag() {
        let json = r#"{"context_id": "1", "client_tag": "codex"}"#;
        let ctx: ContextSummary = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.label(), Some("codex"));
    }

    #[test]
    fn test_contexts_response() {
        let json = r#"{"contexts": [{"context_id": "1", "head_turn_id": "5", "head_depth": 3}], "count": 1}"#;
        let resp: ContextsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.contexts.len(), 1);
        assert_eq!(resp.contexts[0].id(), 1);
    }

    #[test]
    fn test_typed_turn_nested_format() {
        let json = r#"{
            "turn_id": "2",
            "depth": 1,
            "data": {
                "item_type": "tool_call",
                "status": "complete",
                "tool_call": { "call_id": "tc_1", "name": "bash", "args": "{\"command\": \"ls\"}" }
            }
        }"#;
        let turn: TypedTurn = serde_json::from_str(json).unwrap();
        assert_eq!(turn.id(), 2);
        assert_eq!(turn.turn_type(), TurnType::ToolCall);
        assert_eq!(turn.tool_name(), Some("bash".to_string()));
    }

    #[test]
    fn test_typed_turn_tool_result_error() {
        let json = r#"{
            "turn_id": "3",
            "data": {
                "item_type": "tool_result",
                "tool_result": { "call_id": "tc_1", "content": "auth failed", "is_error": true }
            }
        }"#;
        let turn: TypedTurn = serde_json::from_str(json).unwrap();
        assert_eq!(turn.turn_type(), TurnType::ToolResult);
        assert!(turn.is_error());
        assert_eq!(turn.text_content(), "ERROR: auth failed");
    }

    #[test]
    fn test_typed_turn_assistant() {
        let json = r#"{
            "turn_id": "4",
            "data": {
                "item_type": "assistant",
                "assistant": { "text": "I'll fix the auth issue." }
            }
        }"#;
        let turn: TypedTurn = serde_json::from_str(json).unwrap();
        assert_eq!(turn.turn_type(), TurnType::Assistant);
        assert_eq!(turn.text_content(), "I'll fix the auth issue.");
    }

    #[test]
    fn test_typed_turn_user_input() {
        let json = r#"{
            "turn_id": "1",
            "data": {
                "item_type": "user_input",
                "user_input": { "text": "List PRs" }
            }
        }"#;
        let turn: TypedTurn = serde_json::from_str(json).unwrap();
        assert_eq!(turn.turn_type(), TurnType::UserInput);
        assert_eq!(turn.text_content(), "List PRs");
    }

    #[test]
    fn test_typed_turn_system_error() {
        let json = r#"{
            "turn_id": "5",
            "data": {
                "item_type": "system",
                "system": { "kind": "error", "title": "", "content": "connection refused" }
            }
        }"#;
        let turn: TypedTurn = serde_json::from_str(json).unwrap();
        assert_eq!(turn.turn_type(), TurnType::System);
        assert!(turn.is_error());
        assert_eq!(turn.text_content(), "[error] connection refused");
    }

    #[test]
    fn test_turns_response() {
        let json = r#"{"turns": [{"turn_id": "1", "data": {"item_type": "user_input"}}], "meta": {"context_id": "1"}}"#;
        let resp: TypedTurnsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.turns.len(), 1);
    }
}
