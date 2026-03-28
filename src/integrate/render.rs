use crate::integrate::markers;
use crate::integrate::targets::{InstructionStyle, IntegrationTarget};

/// Render a complete managed block for the given target.
///
/// The block is wrapped in begin/end markers and stays compact.
/// It does not include a topic list — agents query the CLI directly.
pub fn render_managed_block(target: &IntegrationTarget) -> String {
    let content = render_section_content(target);
    markers::wrap_block(&target.identifier, &content)
}

/// Render the inner content of the managed section (without markers).
fn render_section_content(target: &IntegrationTarget) -> String {
    let title = &target.rendering_hints.section_title;

    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("## {title}"));
    lines.push(String::new());

    match target.rendering_hints.instruction_style {
        InstructionStyle::ClaudeCode => {
            lines.push("You have access to a local knowledge base called Oversight that contains solutions to previously encountered tool errors and workflow issues.".to_string());
            lines.push(String::new());
            lines.push("RULE: Your first action for ANY task that involves CLI tools (git, gh, docker, npm, cargo, etc.) MUST be to check this knowledge base. Do not run commands or answer directly until you have checked for relevant entries.".to_string());
            lines.push(String::new());
            lines.push("Steps (run these as bash commands):".to_string());
            lines.push("1. `oversight topics` — list what is in the KB.".to_string());
            lines.push("2. `oversight search <query>` — find entries matching the tools/task.".to_string());
            lines.push("3. `oversight read <topic>` — read the full entry before proceeding.".to_string());
        }
        InstructionStyle::Generic => {
            lines.push("You have access to a local knowledge base called Oversight that contains solutions to previously encountered tool errors and workflow issues.".to_string());
            lines.push(String::new());
            lines.push("RULE: Your first action for ANY task that involves CLI tools (git, gh, docker, npm, cargo, etc.) MUST be to check this knowledge base. Do not run commands or answer directly until you have checked for relevant entries.".to_string());
            lines.push(String::new());
            lines.push("Steps:".to_string());
            lines.push("1. `oversight topics` — list what is in the KB.".to_string());
            lines.push("2. `oversight search <query>` — find entries matching the tools/task.".to_string());
            lines.push("3. `oversight read <topic>` — read the full entry before proceeding.".to_string());
        }
    }

    lines.push(String::new());
    lines.push(
        "If Oversight is not installed, continue normally.".to_string(),
    );

    let mut content = lines.join("\n");
    content.push('\n');
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrate::targets::IntegrationTarget;

    #[test]
    fn test_render_block_structure() {
        let target = IntegrationTarget::claude_code();
        let block = render_managed_block(&target);
        assert!(block.contains("oversight:begin target=claude-code"));
        assert!(block.contains("oversight:end"));
        assert!(block.contains("oversight topics"));
        assert!(block.contains("oversight search"));
        assert!(block.contains("oversight read"));
    }

    #[test]
    fn test_render_stays_compact() {
        let target = IntegrationTarget::claude_code();
        let block = render_managed_block(&target);
        let line_count = block.lines().count();
        assert!(
            line_count <= 20,
            "Block should be compact, got {line_count} lines"
        );
    }

    #[test]
    fn test_render_claude_code_instructions() {
        let target = IntegrationTarget::claude_code();
        let block = render_managed_block(&target);
        assert!(block.contains("RULE: Your first action for ANY task"));
    }

    #[test]
    fn test_render_generic_instructions() {
        let target = IntegrationTarget::generic_agents_md(None);
        let block = render_managed_block(&target);
        assert!(block.contains("RULE: Your first action for ANY task"));
    }
}
