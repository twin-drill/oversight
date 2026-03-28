use crate::healing_loop::policy::{self, DedupePolicy, TitleMatchMode};
use crate::kb::slug;
use crate::kb::types::Topic;
use crate::llm::extractor::Learning;

/// The outcome of deduplication for a single learning.
#[derive(Debug, Clone)]
pub enum MergeOutcome {
    /// No matching topic exists - create a new one.
    CreateTopic {
        learning: Learning,
    },
    /// Matching topic exists, but this insight is novel - append to it.
    AppendInsight {
        learning: Learning,
        existing_topic: Topic,
    },
    /// Matching topic exists and this insight is already covered - skip.
    NoOpDuplicate {
        learning: Learning,
        existing_slug: String,
        reason: String,
    },
}

impl MergeOutcome {
    /// Return a human-readable summary for display.
    pub fn summary(&self) -> String {
        match self {
            MergeOutcome::CreateTopic { learning } => {
                format!(
                    "CREATE topic '{}': {}",
                    learning.topic_hint, learning.title
                )
            }
            MergeOutcome::AppendInsight {
                learning,
                existing_topic,
            } => {
                format!(
                    "APPEND to '{}': {}",
                    existing_topic.slug, learning.title
                )
            }
            MergeOutcome::NoOpDuplicate {
                learning,
                existing_slug,
                reason,
            } => {
                format!(
                    "SKIP '{}' (matches '{}'): {}",
                    learning.title, existing_slug, reason
                )
            }
        }
    }
}

/// Deduplicate learnings against existing KB topics.
///
/// For each learning, determines whether to create, append, or skip.
/// Uses the provided `DedupePolicy` for all matching decisions.
pub fn deduplicate(
    learnings: &[Learning],
    existing_topics: &[Topic],
    policy: &DedupePolicy,
) -> Vec<MergeOutcome> {
    learnings
        .iter()
        .map(|learning| classify_learning(learning, existing_topics, policy))
        .collect()
}

/// Classify a single learning against existing topics.
fn classify_learning(
    learning: &Learning,
    existing_topics: &[Topic],
    policy: &DedupePolicy,
) -> MergeOutcome {
    let hint_slug = slug::normalize(&learning.topic_hint);

    // Try to find a matching topic
    let matched_topic = find_matching_topic(&hint_slug, learning, existing_topics, policy);

    match matched_topic {
        Some(topic) => {
            // Check if this specific insight is already covered
            if is_insight_covered(learning, topic, policy) {
                MergeOutcome::NoOpDuplicate {
                    learning: learning.clone(),
                    existing_slug: topic.slug.clone(),
                    reason: "Insight already covered in existing topic body".to_string(),
                }
            } else {
                MergeOutcome::AppendInsight {
                    learning: learning.clone(),
                    existing_topic: topic.clone(),
                }
            }
        }
        None => MergeOutcome::CreateTopic {
            learning: learning.clone(),
        },
    }
}

/// Find a topic that matches the learning by slug, alias, title, or tag similarity.
fn find_matching_topic<'a>(
    hint_slug: &str,
    learning: &Learning,
    topics: &'a [Topic],
    policy: &DedupePolicy,
) -> Option<&'a Topic> {
    // 1. Direct slug match
    if let Some(topic) = topics.iter().find(|t| t.slug == hint_slug) {
        return Some(topic);
    }

    // 2. Alias match
    let hint_lower = learning.topic_hint.to_lowercase();
    if let Some(topic) = topics.iter().find(|t| {
        t.aliases
            .iter()
            .any(|a| a.to_lowercase() == hint_lower)
    }) {
        return Some(topic);
    }

    // 3. Title similarity (policy-driven)
    let hint_normalized = hint_slug.replace('-', " ");
    if let Some(topic) = topics.iter().find(|t| {
        match_title(&t.title, &hint_normalized, hint_slug, &policy.title_match_mode)
    }) {
        return Some(topic);
    }

    // 4. Tag overlap with slug affinity (policy-driven thresholds)
    if !learning.tags.is_empty() {
        let learning_tags: std::collections::HashSet<String> = learning
            .tags
            .iter()
            .map(|t| t.to_lowercase())
            .collect();

        // First pass: tag overlap + slug affinity (stricter)
        for topic in topics {
            let overlap = count_tag_overlap(&learning_tags, &topic.tags);
            if overlap >= policy.tag_overlap_minimum {
                if policy.require_slug_affinity {
                    if topic.slug.contains(hint_slug) || hint_slug.contains(&topic.slug) {
                        return Some(topic);
                    }
                } else {
                    return Some(topic);
                }
            }
        }

        // Second pass: high tag Jaccard similarity (catches semantic duplicates
        // with different slugs, e.g. "oversight-ledger" vs "sprint-ledger")
        if policy.tag_jaccard_threshold < 1.0 {
            let mut best_match: Option<(f64, &Topic)> = None;
            for topic in topics {
                if topic.tags.is_empty() {
                    continue;
                }
                let topic_tags: std::collections::HashSet<String> = topic
                    .tags
                    .iter()
                    .map(|t| t.to_lowercase())
                    .collect();
                let intersection = learning_tags.intersection(&topic_tags).count();
                let union = learning_tags.union(&topic_tags).count();
                if union == 0 {
                    continue;
                }
                let jaccard = intersection as f64 / union as f64;
                if jaccard >= policy.tag_jaccard_threshold {
                    if best_match.is_none() || jaccard > best_match.unwrap().0 {
                        best_match = Some((jaccard, topic));
                    }
                }
            }
            if let Some((_, topic)) = best_match {
                return Some(topic);
            }
        }
    }

    None
}

fn count_tag_overlap(learning_tags: &std::collections::HashSet<String>, topic_tags: &[String]) -> usize {
    topic_tags
        .iter()
        .filter(|t| learning_tags.contains(&t.to_lowercase()))
        .count()
}

/// Match a topic title against the hint using the specified mode.
fn match_title(
    topic_title: &str,
    hint_normalized: &str,
    hint_slug: &str,
    mode: &TitleMatchMode,
) -> bool {
    let title_normalized = topic_title.to_lowercase();

    match mode {
        TitleMatchMode::Exact => {
            // Only match when slugified titles are identical
            let title_slug = slug::normalize(&title_normalized);
            title_slug == hint_slug
        }
        TitleMatchMode::Contains => {
            // Match when one contains the other (current behavior)
            title_normalized == hint_normalized
                || title_normalized.contains(hint_normalized)
                || hint_normalized.contains(&title_normalized.replace(' ', "-"))
        }
        TitleMatchMode::FuzzyTokenJaccard { min_similarity } => {
            // First check contains (superset of exact)
            if title_normalized == hint_normalized
                || title_normalized.contains(hint_normalized)
                || hint_normalized.contains(&title_normalized.replace(' ', "-"))
            {
                return true;
            }
            // Then check Jaccard similarity of slug tokens
            let title_slug = slug::normalize(&title_normalized);
            policy::jaccard_similarity(&title_slug, hint_slug) >= *min_similarity
        }
    }
}

/// Check if a learning's insight is already covered by the topic's body.
fn is_insight_covered(learning: &Learning, topic: &Topic, policy: &DedupePolicy) -> bool {
    let body_lower = topic.body.to_lowercase();
    let title_lower = learning.title.to_lowercase();
    let summary_lower = learning.summary.to_lowercase();

    // Check if the title of the learning appears in the body
    if body_lower.contains(&title_lower) {
        return true;
    }

    // Check if the core summary content appears in the body
    // Split summary into significant words (>= 4 chars) and check overlap
    let summary_words: Vec<&str> = summary_lower
        .split_whitespace()
        .filter(|w| w.len() >= 4)
        .collect();

    if summary_words.is_empty() {
        return false;
    }

    let matched_words = summary_words
        .iter()
        .filter(|w| body_lower.contains(**w))
        .count();

    let ratio = matched_words as f64 / summary_words.len() as f64;

    // Use policy-driven coverage threshold
    ratio > policy.coverage_threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_topic(slug: &str, title: &str, aliases: &[&str], tags: &[&str], body: &str) -> Topic {
        let mut t = Topic::new(title.to_string(), slug.to_string(), body.to_string());
        t.aliases = aliases.iter().map(|s| s.to_string()).collect();
        t.tags = tags.iter().map(|s| s.to_string()).collect();
        t
    }

    fn make_learning(hint: &str, title: &str, summary: &str, tags: &[&str]) -> Learning {
        Learning {
            topic_hint: hint.to_string(),
            title: title.to_string(),
            summary: summary.to_string(),
            evidence: vec!["test evidence".to_string()],
            tags: tags.iter().map(|s| s.to_string()).collect(),
            confidence: 0.9,
            project_path: None,
        }
    }

    #[test]
    fn test_create_topic_no_match() {
        let existing = vec![
            make_topic("docker-compose", "Docker Compose", &[], &["docker"], "Docker stuff."),
        ];

        let learning = make_learning("gh-cli", "Use gh auth login", "Run gh auth login to authenticate.", &["cli"]);

        let outcomes = deduplicate(&[learning], &existing, &DedupePolicy::balanced());
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], MergeOutcome::CreateTopic { .. }));
    }

    #[test]
    fn test_append_insight_slug_match() {
        let existing = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &["cli"], "# GitHub CLI\n\nBasic usage."),
        ];

        let learning = make_learning(
            "gh-cli",
            "Unset GITHUB_TOKEN",
            "Environment token overrides keychain auth.",
            &["cli"],
        );

        let outcomes = deduplicate(&[learning], &existing, &DedupePolicy::balanced());
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], MergeOutcome::AppendInsight { .. }));
    }

    #[test]
    fn test_noop_duplicate() {
        let existing = vec![
            make_topic(
                "gh-cli",
                "GitHub CLI",
                &[],
                &["cli"],
                "# GitHub CLI\n\n## Key Insights\n\n- Unset GITHUB_TOKEN before gh commands\n\nEnvironment token overrides keychain auth.\n",
            ),
        ];

        let learning = make_learning(
            "gh-cli",
            "Unset GITHUB_TOKEN before gh commands",
            "Environment token overrides keychain auth.",
            &["cli"],
        );

        let outcomes = deduplicate(&[learning], &existing, &DedupePolicy::balanced());
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], MergeOutcome::NoOpDuplicate { .. }));
    }

    #[test]
    fn test_alias_match() {
        let existing = vec![
            make_topic(
                "gh-cli",
                "GitHub CLI",
                &["github cli", "gh command"],
                &["cli"],
                "# GitHub CLI\n\nBasic usage.",
            ),
        ];

        let learning = make_learning(
            "github cli",
            "New insight about gh",
            "Some completely novel finding about authentication.",
            &["cli"],
        );

        let outcomes = deduplicate(&[learning], &existing, &DedupePolicy::balanced());
        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0], MergeOutcome::AppendInsight { .. }),
            "Expected AppendInsight, got: {:?}",
            outcomes[0].summary()
        );
    }

    #[test]
    fn test_outcome_summary() {
        let learning = make_learning("gh-cli", "Test", "Summary", &[]);
        let outcome = MergeOutcome::CreateTopic {
            learning,
        };
        let summary = outcome.summary();
        assert!(summary.contains("CREATE"));
        assert!(summary.contains("gh-cli"));
    }

    #[test]
    fn test_is_insight_covered_exact_title() {
        let topic = make_topic("test", "Test", &[], &[], "# Test\n\nUnset GITHUB_TOKEN for auth.\n");
        let learning = make_learning("test", "Unset GITHUB_TOKEN for auth", "Other summary", &[]);
        let policy = DedupePolicy::balanced();
        // The title appears in the body (case-insensitive), so it should be covered
        assert!(is_insight_covered(&learning, &topic, &policy));
    }

    #[test]
    fn test_is_insight_not_covered() {
        let topic = make_topic("test", "Test", &[], &[], "# Test\n\nBasic docker usage.\n");
        let learning = make_learning(
            "test",
            "New auth pattern",
            "A completely different authentication mechanism discovered.",
            &[],
        );
        let policy = DedupePolicy::balanced();
        assert!(!is_insight_covered(&learning, &topic, &policy));
    }

    #[test]
    fn test_multiple_learnings() {
        let existing = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &[], "# GitHub CLI\n\nBasic usage."),
        ];

        let learnings = vec![
            make_learning("gh-cli", "Auth fix", "Fix auth issues.", &[]),
            make_learning("docker", "Docker tip", "Run docker compose up.", &[]),
        ];

        let outcomes = deduplicate(&learnings, &existing, &DedupePolicy::balanced());
        assert_eq!(outcomes.len(), 2);
        assert!(matches!(outcomes[0], MergeOutcome::AppendInsight { .. }));
        assert!(matches!(outcomes[1], MergeOutcome::CreateTopic { .. }));
    }
}
