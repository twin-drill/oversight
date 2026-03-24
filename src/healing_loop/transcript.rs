use crate::source::types::{TurnType, TypedTurn};

/// Maximum lines to keep from verbose tool output.
const MAX_OUTPUT_LINES: usize = 30;

/// Reduce a set of typed turns into a compact transcript string for LLM extraction.
///
/// This filters to tool-relevant events and truncates verbose output to fit
/// within a token budget.
pub fn reduce_transcript(turns: &[TypedTurn], max_len: usize) -> String {
    let relevant_turns = filter_relevant_turns(turns);
    let mut transcript = String::new();

    for turn in &relevant_turns {
        let entry = format_turn(turn);
        if entry.is_empty() {
            continue;
        }

        // Check if adding this entry would exceed our budget
        if transcript.len() + entry.len() + 2 > max_len {
            transcript.push_str("\n[... transcript truncated ...]\n");
            break;
        }

        transcript.push_str(&entry);
        transcript.push('\n');
    }

    transcript
}

/// Filter turns to tool-relevant events.
///
/// Keeps: tool_call, tool_result, assistant_turn (if tool-related), system (if error).
/// Drops: user_input (unless it references a tool), unknown types.
fn filter_relevant_turns(turns: &[TypedTurn]) -> Vec<&TypedTurn> {
    turns
        .iter()
        .filter(|turn| match turn.turn_type() {
            TurnType::ToolCall => true,
            TurnType::ToolResult => true,
            TurnType::System => turn.is_error(), // Only keep error system messages
            TurnType::AssistantTurn | TurnType::Assistant => {
                // Keep assistant turns that reference tools
                let text = turn.text_content().to_lowercase();
                text.contains("error")
                    || text.contains("failed")
                    || text.contains("fix")
                    || text.contains("workaround")
                    || text.contains("instead")
                    || text.contains("solution")
                    || turn.data.get("tool").is_some()
                    || turn.data.get("tool_use").is_some()
            }
            TurnType::UserInput => {
                // Keep user input only if it references error/fix
                let text = turn.text_content().to_lowercase();
                text.contains("error")
                    || text.contains("failed")
                    || text.contains("fix")
                    || text.contains("not working")
            }
            TurnType::Unknown | TurnType::Handoff => false,
        })
        .collect()
}

/// Format a single turn into a transcript line.
fn format_turn(turn: &TypedTurn) -> String {
    let prefix = match turn.turn_type() {
        TurnType::ToolCall => {
            let tool = turn.tool_name().unwrap_or_else(|| "unknown".to_string());
            format!("[TOOL_CALL: {tool}]")
        }
        TurnType::ToolResult => "[TOOL_RESULT]".to_string(),
        TurnType::AssistantTurn | TurnType::Assistant => "[ASSISTANT]".to_string(),
        TurnType::System => "[SYSTEM]".to_string(),
        TurnType::UserInput => "[USER]".to_string(),
        TurnType::Unknown | TurnType::Handoff => return String::new(),
    };

    let content = turn.text_content();
    let truncated = truncate_output(&content, MAX_OUTPUT_LINES);

    if truncated.is_empty() {
        return prefix;
    }

    format!("{prefix} {truncated}")
}

/// Truncate verbose output, keeping the first and last N lines.
fn truncate_output(output: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= max_lines {
        return output.to_string();
    }

    let keep_start = max_lines / 2;
    let keep_end = max_lines - keep_start;
    let omitted = lines.len() - max_lines;

    let mut result = String::new();
    for line in &lines[..keep_start] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&format!("[... {omitted} lines omitted ...]\n"));
    for (i, line) in lines[lines.len() - keep_end..].iter().enumerate() {
        result.push_str(line);
        if i < keep_end - 1 {
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_turn(turn_type: TurnType, data: serde_json::Value) -> TypedTurn {
        // Build a data object with item_type set
        let item_type_str = match turn_type {
            TurnType::ToolCall => "tool_call",
            TurnType::ToolResult => "tool_result",
            TurnType::AssistantTurn => "assistant_turn",
            TurnType::Assistant => "assistant",
            TurnType::System => "system",
            TurnType::UserInput => "user_input",
            TurnType::Handoff => "handoff",
            TurnType::Unknown => "unknown",
        };
        let mut merged = data;
        if let Some(obj) = merged.as_object_mut() {
            obj.insert("item_type".to_string(), json!(item_type_str));
        }
        TypedTurn {
            turn_id: Some(1),
            depth: None,
            data: merged,
            declared_type: None,
        }
    }

    #[test]
    fn test_filter_keeps_tool_calls() {
        let turns = vec![
            make_turn(TurnType::ToolCall, json!({"tool_call": {"name": "bash", "call_id": "tc_1", "args": "ls"}})),
            make_turn(
                TurnType::ToolResult,
                json!({"tool_result": {"call_id": "tc_1", "content": "file1.txt\nfile2.txt", "is_error": false}}),
            ),
            make_turn(TurnType::UserInput, json!({"user_input": {"text": "hello"}})), // no error keywords -> filtered
        ];

        let filtered = filter_relevant_turns(&turns);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_keeps_error_system() {
        let turns = vec![
            make_turn(TurnType::System, json!({"system": {"kind": "error", "content": "connection refused"}})),
            make_turn(TurnType::System, json!({"system": {"kind": "info", "content": "session started"}})),
        ];

        let filtered = filter_relevant_turns(&turns);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_keeps_error_user_input() {
        let turns = vec![
            make_turn(
                TurnType::UserInput,
                json!({"user_input": {"text": "This command failed, can you fix it?"}}),
            ),
            make_turn(
                TurnType::UserInput,
                json!({"user_input": {"text": "Looks good, thanks!"}}),
            ),
        ];

        let filtered = filter_relevant_turns(&turns);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_reduce_transcript_basic() {
        let turns = vec![
            make_turn(
                TurnType::ToolCall,
                json!({"tool_call": {"name": "bash", "call_id": "tc_1", "args": "gh pr list"}}),
            ),
            make_turn(
                TurnType::ToolResult,
                json!({"tool_result": {"call_id": "tc_1", "content": "auth required", "is_error": true}}),
            ),
            make_turn(
                TurnType::Assistant,
                json!({"assistant": {"text": "The error indicates we need to fix authentication."}}),
            ),
            make_turn(
                TurnType::ToolCall,
                json!({"tool_call": {"name": "bash", "call_id": "tc_2", "args": "unset GITHUB_TOKEN && gh pr list"}}),
            ),
            make_turn(
                TurnType::ToolResult,
                json!({"tool_result": {"call_id": "tc_2", "content": "#1 Fix bug\n#2 Add feature", "is_error": false}}),
            ),
        ];

        let transcript = reduce_transcript(&turns, 10000);
        assert!(transcript.contains("TOOL_CALL: bash"));
        assert!(transcript.contains("TOOL_RESULT"));
        assert!(transcript.contains("error") || transcript.contains("ERROR"));
    }

    #[test]
    fn test_reduce_transcript_truncation() {
        let turns = vec![make_turn(
            TurnType::ToolCall,
            json!({"tool_call": {"name": "bash", "call_id": "tc_1", "args": "a".repeat(5000)}}),
        )];

        let transcript = reduce_transcript(&turns, 100);
        assert!(transcript.len() <= 200); // some overhead for truncation message
    }

    #[test]
    fn test_truncate_output_short() {
        let output = "line1\nline2\nline3";
        let result = truncate_output(output, 30);
        assert_eq!(result, output);
    }

    #[test]
    fn test_truncate_output_long() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let output = lines.join("\n");
        let result = truncate_output(&output, 10);
        assert!(result.contains("line 0"));
        assert!(result.contains("line 99"));
        assert!(result.contains("omitted"));
    }

    #[test]
    fn test_format_turn_tool_call() {
        let turn = make_turn(
            TurnType::ToolCall,
            json!({"tool_call": {"name": "bash", "call_id": "tc_1", "args": "ls -la"}}),
        );
        let formatted = format_turn(&turn);
        assert!(formatted.contains("[TOOL_CALL: bash]"));
        assert!(formatted.contains("ls -la"));
    }

    #[test]
    fn test_format_turn_unknown_skipped() {
        let turn = make_turn(TurnType::Unknown, json!({"text": "something"}));
        let formatted = format_turn(&turn);
        assert!(formatted.is_empty());
    }
}
