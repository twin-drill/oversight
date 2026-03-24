use crate::error::Result;
use crate::healing_loop::dedupe::MergeOutcome;
use crate::kb::service::KBService;
use crate::kb::slug;
use crate::kb::types::Topic;
use crate::llm::extractor::Learning;
use chrono::Utc;

/// Result of applying a merge plan.
#[derive(Debug)]
pub struct MergeResult {
    pub topics_created: u32,
    pub topics_updated: u32,
    pub duplicates_skipped: u32,
}

/// Apply a set of merge outcomes to the KB.
pub fn apply_merges(
    service: &KBService,
    outcomes: &[MergeOutcome],
    context_id: u64,
) -> Result<MergeResult> {
    validate_create_outcomes(outcomes)?;

    let mut result = MergeResult {
        topics_created: 0,
        topics_updated: 0,
        duplicates_skipped: 0,
    };

    for outcome in outcomes {
        match outcome {
            MergeOutcome::CreateTopic { learning } => {
                create_topic_from_learning(service, learning, context_id)?;
                result.topics_created += 1;
            }
            MergeOutcome::AppendInsight {
                learning,
                existing_topic,
            } => {
                append_insight_to_topic(service, learning, existing_topic, context_id)?;
                result.topics_updated += 1;
            }
            MergeOutcome::NoOpDuplicate { .. } => {
                result.duplicates_skipped += 1;
            }
        }
    }

    Ok(result)
}

fn validate_create_outcomes(outcomes: &[MergeOutcome]) -> Result<()> {
    let mut seen_slugs = std::collections::HashSet::new();

    for outcome in outcomes {
        if let MergeOutcome::CreateTopic { learning } = outcome {
            let topic_slug = slug::normalize(&learning.topic_hint);
            if !seen_slugs.insert(topic_slug.clone()) {
                return Err(crate::error::Error::MergeConflict(format!(
                    "multiple create outcomes target slug '{topic_slug}'"
                )));
            }
        }
    }

    Ok(())
}

/// Create a new KB topic from a learning.
fn create_topic_from_learning(
    service: &KBService,
    learning: &Learning,
    context_id: u64,
) -> Result<()> {
    let topic_slug = slug::normalize(&learning.topic_hint);
    let now = Utc::now();

    let body = format_new_topic_body(learning);

    let mut topic = Topic::new(learning.title.clone(), topic_slug, body);
    topic.tags = learning.tags.clone();
    topic.source = Some("healing-loop".to_string());
    topic.confidence = Some(format!("{:.2}", learning.confidence));
    topic.source_context_id = Some(context_id);
    topic.extracted_at = Some(now);

    service.create_topic(&topic)?;
    Ok(())
}

/// Append a new insight to an existing topic.
fn append_insight_to_topic(
    service: &KBService,
    learning: &Learning,
    existing_topic: &Topic,
    context_id: u64,
) -> Result<()> {
    let mut updated = existing_topic.clone();
    let now = Utc::now();

    // Build the insight section to append
    let insight_section = format_insight_section(learning, context_id);

    // Append to the body - find or create "Key Insights" section
    updated.body = append_to_insights_section(&updated.body, &insight_section);
    updated.updated = Some(now);

    // Merge tags (add new ones)
    for tag in &learning.tags {
        if !updated.tags.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            updated.tags.push(tag.clone());
        }
    }

    // Update provenance to reflect most recent extraction
    updated.source_context_id = Some(context_id);
    updated.extracted_at = Some(now);

    service.upsert_topic(&updated)?;
    Ok(())
}

/// Format the body for a brand new topic created from a learning.
fn format_new_topic_body(learning: &Learning) -> String {
    let mut body = format!("# {}\n\n", learning.title);
    body.push_str(&learning.summary);
    body.push_str("\n\n");

    if !learning.evidence.is_empty() {
        body.push_str("## Evidence\n\n");
        for ev in &learning.evidence {
            body.push_str(&format!("- {ev}\n"));
        }
        body.push('\n');
    }

    body
}

/// Format an insight section for appending to an existing topic.
fn format_insight_section(learning: &Learning, context_id: u64) -> String {
    let mut section = format!("### {}\n\n", learning.title);
    section.push_str(&learning.summary);
    section.push('\n');

    if !learning.evidence.is_empty() {
        section.push('\n');
        for ev in &learning.evidence {
            section.push_str(&format!("- {ev}\n"));
        }
    }

    section.push_str(&format!(
        "\n_Extracted from context {} at {}_\n",
        context_id,
        Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    section
}

/// Append an insight section to the topic body, inserting into or creating
/// a "Key Insights" section.
fn append_to_insights_section(body: &str, insight: &str) -> String {
    // Look for an existing "## Key Insights" section
    if let Some(pos) = body.find("## Key Insights") {
        // Find the end of this section (next ## header or end of body)
        let after_header = &body[pos..];
        let section_end = after_header[1..] // skip past the first ##
            .find("\n## ")
            .map(|p| pos + 1 + p)
            .unwrap_or(body.len());

        let mut result = String::new();
        result.push_str(&body[..section_end]);
        // Ensure there's a newline before the new insight
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push('\n');
        result.push_str(insight);
        result.push_str(&body[section_end..]);
        result
    } else {
        // No "Key Insights" section exists - create one at the end
        let mut result = body.to_string();
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("\n## Key Insights\n\n");
        result.push_str(insight);
        result
    }
}

/// Generate a dry-run summary of proposed merge outcomes.
pub fn dry_run_summary(outcomes: &[MergeOutcome]) -> String {
    if outcomes.is_empty() {
        return "No changes proposed.".to_string();
    }

    let mut creates = 0u32;
    let mut appends = 0u32;
    let mut skips = 0u32;
    let mut lines = Vec::new();

    for outcome in outcomes {
        match outcome {
            MergeOutcome::CreateTopic { .. } => {
                creates += 1;
                lines.push(format!("  + {}", outcome.summary()));
            }
            MergeOutcome::AppendInsight { .. } => {
                appends += 1;
                lines.push(format!("  ~ {}", outcome.summary()));
            }
            MergeOutcome::NoOpDuplicate { .. } => {
                skips += 1;
                lines.push(format!("  = {}", outcome.summary()));
            }
        }
    }

    let mut result = format!(
        "Proposed changes: {} create, {} append, {} skip\n",
        creates, appends, skips
    );
    for line in lines {
        result.push_str(&line);
        result.push('\n');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_new_topic_body() {
        let learning = Learning {
            topic_hint: "gh-cli".into(),
            title: "Unset GITHUB_TOKEN".into(),
            summary: "The env var overrides keychain auth.".into(),
            evidence: vec!["gh auth failed".into(), "after unset, it worked".into()],
            tags: vec!["cli".into()],
            confidence: 0.9,
        };

        let body = format_new_topic_body(&learning);
        assert!(body.contains("# Unset GITHUB_TOKEN"));
        assert!(body.contains("The env var overrides keychain auth."));
        assert!(body.contains("## Evidence"));
        assert!(body.contains("- gh auth failed"));
    }

    #[test]
    fn test_append_to_existing_insights() {
        let body = "# Topic\n\nIntro.\n\n## Key Insights\n\n### Old insight\n\nOld content.\n";
        let insight = "### New insight\n\nNew content.\n";
        let result = append_to_insights_section(body, insight);

        assert!(result.contains("### Old insight"));
        assert!(result.contains("### New insight"));
    }

    #[test]
    fn test_append_creates_insights_section() {
        let body = "# Topic\n\nJust intro content.\n";
        let insight = "### New insight\n\nContent.\n";
        let result = append_to_insights_section(body, insight);

        assert!(result.contains("## Key Insights"));
        assert!(result.contains("### New insight"));
    }

    #[test]
    fn test_dry_run_summary_empty() {
        let summary = dry_run_summary(&[]);
        assert_eq!(summary, "No changes proposed.");
    }

    #[test]
    fn test_dry_run_summary_mixed() {
        let learning1 = Learning {
            topic_hint: "gh-cli".into(),
            title: "Auth fix".into(),
            summary: "Fix auth.".into(),
            evidence: vec![],
            tags: vec![],
            confidence: 0.9,
        };
        let learning2 = learning1.clone();
        let learning3 = learning1.clone();

        let topic = Topic::new("GH CLI".into(), "gh-cli".into(), "Body".into());

        let outcomes = vec![
            MergeOutcome::CreateTopic {
                learning: learning1,
            },
            MergeOutcome::AppendInsight {
                learning: learning2,
                existing_topic: topic,
            },
            MergeOutcome::NoOpDuplicate {
                learning: learning3,
                existing_slug: "gh-cli".into(),
                reason: "Duplicate".into(),
            },
        ];

        let summary = dry_run_summary(&outcomes);
        assert!(summary.contains("1 create"));
        assert!(summary.contains("1 append"));
        assert!(summary.contains("1 skip"));
    }

    #[test]
    fn test_format_insight_section() {
        let learning = Learning {
            topic_hint: "test".into(),
            title: "New finding".into(),
            summary: "Something interesting.".into(),
            evidence: vec!["saw this happen".into()],
            tags: vec![],
            confidence: 0.8,
        };

        let section = format_insight_section(&learning, 42);
        assert!(section.contains("### New finding"));
        assert!(section.contains("Something interesting."));
        assert!(section.contains("- saw this happen"));
        assert!(section.contains("context 42"));
    }
}
