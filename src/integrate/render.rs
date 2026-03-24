use crate::integrate::markers;
use crate::integrate::targets::{InstructionStyle, IntegrationTarget};
use crate::kb::types::TopicSummary;

/// Default maximum number of topic slugs to show in the preview.
const DEFAULT_TOPIC_PREVIEW_LIMIT: usize = 20;

/// Render a complete managed block for the given target and topic summaries.
///
/// The block is wrapped in begin/end markers and stays compact (under ~15 lines).
pub fn render_managed_block(
    target: &IntegrationTarget,
    topics: &[TopicSummary],
    preview_limit: Option<usize>,
) -> String {
    let content = render_section_content(target, topics, preview_limit);
    markers::wrap_block(&target.identifier, &content)
}

/// Render the inner content of the managed section (without markers).
fn render_section_content(
    target: &IntegrationTarget,
    topics: &[TopicSummary],
    preview_limit: Option<usize>,
) -> String {
    let limit = preview_limit.unwrap_or(DEFAULT_TOPIC_PREVIEW_LIMIT);
    let title = &target.rendering_hints.section_title;

    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("## {title}"));
    lines.push(String::new());

    // Instructions vary by target style
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

    // Topic list
    if topics.is_empty() {
        lines.push("No topics yet. Run `oversight add` to create one, or continue normally.".to_string());
    } else {
        let slugs: Vec<&str> = topics.iter().map(|t| t.slug.as_str()).collect();
        let display_count = slugs.len().min(limit);
        let displayed: Vec<&str> = slugs[..display_count].to_vec();
        let topic_line = format!("Current topics: {}", displayed.join(", "));
        lines.push(topic_line);

        if slugs.len() > limit {
            let remaining = slugs.len() - limit;
            lines.push(format!(
                "...and {remaining} more. Run `oversight topics` for full list."
            ));
        }
    }

    lines.push(String::new());
    lines.push(
        "If no topics are listed or Oversight is not installed, continue normally.".to_string(),
    );

    let mut content = lines.join("\n");
    content.push('\n');
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrate::targets::IntegrationTarget;

    fn make_topics(slugs: &[&str]) -> Vec<TopicSummary> {
        slugs
            .iter()
            .map(|s| TopicSummary {
                slug: s.to_string(),
                title: s.to_string(),
                aliases: Vec::new(),
                tags: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn test_render_empty_kb() {
        let target = IntegrationTarget::claude_code();
        let block = render_managed_block(&target, &[], None);
        assert!(block.contains("oversight:begin target=claude-code"));
        assert!(block.contains("No topics yet"));
        assert!(block.contains("oversight:end"));
    }

    #[test]
    fn test_render_with_topics() {
        let target = IntegrationTarget::claude_code();
        let topics = make_topics(&["gh-cli", "aws-sso", "docker-local"]);
        let block = render_managed_block(&target, &topics, None);
        assert!(block.contains("Current topics: gh-cli, aws-sso, docker-local"));
        assert!(block.contains("oversight:begin"));
        assert!(block.contains("oversight:end"));
    }

    #[test]
    fn test_render_respects_preview_limit() {
        let target = IntegrationTarget::claude_code();
        let slugs: Vec<String> = (0..25).map(|i| format!("topic-{i:03}")).collect();
        let topics = make_topics(
            &slugs.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
        let block = render_managed_block(&target, &topics, Some(5));
        assert!(block.contains("topic-000"));
        assert!(block.contains("topic-004"));
        assert!(!block.contains("topic-005"));
        assert!(block.contains("...and 20 more"));
    }

    #[test]
    fn test_render_at_exact_limit() {
        let target = IntegrationTarget::claude_code();
        let topics = make_topics(&["a", "b", "c"]);
        let block = render_managed_block(&target, &topics, Some(3));
        assert!(block.contains("Current topics: a, b, c"));
        assert!(!block.contains("...and"));
    }

    #[test]
    fn test_render_over_limit_by_one() {
        let target = IntegrationTarget::claude_code();
        let topics = make_topics(&["a", "b", "c", "d"]);
        let block = render_managed_block(&target, &topics, Some(3));
        assert!(block.contains("Current topics: a, b, c"));
        assert!(block.contains("...and 1 more"));
    }

    #[test]
    fn test_render_stays_compact() {
        let target = IntegrationTarget::claude_code();
        let topics = make_topics(&["gh-cli", "aws-sso", "docker-local"]);
        let block = render_managed_block(&target, &topics, None);
        let line_count = block.lines().count();
        assert!(
            line_count <= 20,
            "Block should be compact, got {line_count} lines"
        );
    }

    #[test]
    fn test_render_claude_code_instructions() {
        let target = IntegrationTarget::claude_code();
        let block = render_managed_block(&target, &[], None);
        assert!(block.contains("RULE: Your first action for ANY task"));
    }

    #[test]
    fn test_render_generic_instructions() {
        let target = IntegrationTarget::generic_agents_md(None);
        let block = render_managed_block(&target, &[], None);
        assert!(block.contains("RULE: Your first action for ANY task"));
    }
}
