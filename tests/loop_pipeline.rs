//! Integration tests for the healing loop pipeline components.
//!
//! These tests exercise the pipeline stages (transcript, extraction, dedupe, merge)
//! with mock data, without requiring network or Anthropic API access.

use oversight::config::Config;
use oversight::healing_loop::dedupe::{self, MergeOutcome};
use oversight::healing_loop::merge;
use oversight::healing_loop::policy::DedupePolicy;
use oversight::healing_loop::transcript;
use oversight::llm::extractor::{self, Learning};
use oversight::state::LoopState;
use oversight::{KBService, Topic};
use tempfile::TempDir;

fn setup_kb() -> (TempDir, KBService) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        kb_path: dir.path().to_path_buf(),
        ..Config::default()
    };
    let service = KBService::new(config);
    service.init().unwrap();
    (dir, service)
}

fn make_learning(hint: &str, title: &str, summary: &str, tags: &[&str]) -> Learning {
    Learning {
        topic_hint: hint.to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
        evidence: vec!["test evidence".to_string()],
        tags: tags.iter().map(|s| s.to_string()).collect(),
        confidence: 0.9,
    }
}

// -- Extraction parsing tests --

#[test]
fn test_extraction_parsing_valid() {
    let json = r#"{
        "context_id": 42,
        "learnings": [
            {
                "topic_hint": "gh-cli",
                "title": "Unset GITHUB_TOKEN before gh commands",
                "summary": "Environment token overrides keychain auth.",
                "evidence": ["gh auth failed with read-only token"],
                "tags": ["cli", "github"],
                "confidence": 0.92
            }
        ]
    }"#;

    let resp = extractor::parse_extraction_response(json, 42).unwrap();
    assert_eq!(resp.learnings.len(), 1);
    assert_eq!(resp.learnings[0].topic_hint, "gh-cli");
}

#[test]
fn test_extraction_confidence_filter() {
    let learnings = vec![
        make_learning("a", "High confidence", "summary", &[]),
        Learning {
            confidence: 0.3,
            ..make_learning("b", "Low confidence", "summary", &[])
        },
        Learning {
            confidence: 0.7,
            ..make_learning("c", "Threshold", "summary", &[])
        },
    ];

    let filtered = extractor::filter_by_confidence(learnings, 0.7);
    assert_eq!(filtered.len(), 2);
}

// -- Dedup tests --

#[test]
fn test_dedupe_create_when_no_match() {
    let learning = make_learning("new-tool", "New Tool Tip", "A brand new tip.", &["tool"]);
    let existing: Vec<Topic> = vec![];

    let outcomes = dedupe::deduplicate(&[learning], &existing, &DedupePolicy::balanced());
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], MergeOutcome::CreateTopic { .. }));
}

#[test]
fn test_dedupe_append_when_slug_matches() {
    let (_dir, service) = setup_kb();
    service
        .add_topic("GH CLI", "# GH CLI\n\nBasic usage.\n", vec!["cli".into()], vec![])
        .unwrap();

    let existing = vec![service.get_topic("gh-cli").unwrap()];
    let learning = make_learning("gh-cli", "Auth workaround", "Unset env var.", &["cli"]);

    let outcomes = dedupe::deduplicate(&[learning], &existing, &DedupePolicy::balanced());
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(outcomes[0], MergeOutcome::AppendInsight { .. }),
        "Expected AppendInsight, got: {}",
        outcomes[0].summary()
    );
}

#[test]
fn test_dedupe_noop_when_already_covered() {
    let topic = Topic::new(
        "GH CLI".to_string(),
        "gh-cli".to_string(),
        "# GH CLI\n\n## Key Insights\n\n### Unset GITHUB_TOKEN\n\nThe env var overrides keychain auth.\n".to_string(),
    );

    let learning = make_learning(
        "gh-cli",
        "Unset GITHUB_TOKEN",
        "The env var overrides keychain auth.",
        &[],
    );

    let outcomes = dedupe::deduplicate(&[learning], &[topic], &DedupePolicy::balanced());
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(outcomes[0], MergeOutcome::NoOpDuplicate { .. }),
        "Expected NoOpDuplicate, got: {}",
        outcomes[0].summary()
    );
}

// -- Merge tests --

#[test]
fn test_merge_create_topic() {
    let (_dir, service) = setup_kb();
    let learning = make_learning("docker-tips", "Use docker compose", "Run docker compose up for local dev.", &["docker"]);

    let outcomes = vec![MergeOutcome::CreateTopic {
        learning,
    }];

    let result = merge::apply_merges(&service, &outcomes, 42).unwrap();
    assert_eq!(result.topics_created, 1);
    assert_eq!(result.topics_updated, 0);

    let topic = service.get_topic("docker-tips").unwrap();
    assert!(topic.body.contains("docker compose up"));
    assert_eq!(topic.source.as_deref(), Some("healing-loop"));
    assert_eq!(topic.source_context_id, Some(42));
    assert!(topic.extracted_at.is_some());
}

#[test]
fn test_merge_append_insight() {
    let (_dir, service) = setup_kb();
    service
        .add_topic("GH CLI", "# GH CLI\n\nBasic usage.\n", vec!["cli".into()], vec![])
        .unwrap();

    let existing = service.get_topic("gh-cli").unwrap();
    let learning = make_learning("gh-cli", "Auth workaround", "Unset GITHUB_TOKEN env var.", &["auth"]);

    let outcomes = vec![MergeOutcome::AppendInsight {
        learning,
        existing_topic: existing,
    }];

    let result = merge::apply_merges(&service, &outcomes, 99).unwrap();
    assert_eq!(result.topics_updated, 1);

    let updated = service.get_topic("gh-cli").unwrap();
    assert!(updated.body.contains("## Key Insights"));
    assert!(updated.body.contains("Auth workaround"));
    assert!(updated.body.contains("Unset GITHUB_TOKEN env var."));
    assert!(updated.body.contains("context 99"));
    // Tags should be merged
    assert!(updated.tags.contains(&"cli".to_string()));
    assert!(updated.tags.contains(&"auth".to_string()));
}

#[test]
fn test_merge_noop_duplicate() {
    let learning = make_learning("test", "Test", "Already exists.", &[]);
    let outcomes = vec![MergeOutcome::NoOpDuplicate {
        learning,
        existing_slug: "test".into(),
        reason: "Already covered".into(),
    }];

    let (_dir, service) = setup_kb();
    let result = merge::apply_merges(&service, &outcomes, 1).unwrap();
    assert_eq!(result.duplicates_skipped, 1);
    assert_eq!(result.topics_created, 0);
    assert_eq!(result.topics_updated, 0);
}

#[test]
fn test_merge_provenance_metadata() {
    let (_dir, service) = setup_kb();
    let learning = make_learning("provenance-test", "Test provenance", "Check metadata.", &[]);

    let outcomes = vec![MergeOutcome::CreateTopic { learning }];
    merge::apply_merges(&service, &outcomes, 777).unwrap();

    let topic = service.get_topic("provenance-test").unwrap();
    assert_eq!(topic.source_context_id, Some(777));
    assert!(topic.extracted_at.is_some());
    assert_eq!(topic.source.as_deref(), Some("healing-loop"));
}

#[test]
fn test_merge_duplicate_create_slug_fails_before_writing() {
    let (_dir, service) = setup_kb();
    let outcomes = vec![
        MergeOutcome::CreateTopic {
            learning: make_learning("shared-hint", "First", "First summary.", &["cli"]),
        },
        MergeOutcome::CreateTopic {
            learning: make_learning("shared-hint", "Second", "Second summary.", &["cli"]),
        },
    ];

    let err = merge::apply_merges(&service, &outcomes, 555).unwrap_err();
    assert!(err.to_string().contains("multiple create outcomes"));
    assert!(service.get_topic("shared-hint").is_err());
}

// -- Dry run tests --

#[test]
fn test_dry_run_summary() {
    let learning = make_learning("test", "Test", "Summary.", &[]);
    let topic = Topic::new("Existing".into(), "existing".into(), "Body".into());

    let outcomes = vec![
        MergeOutcome::CreateTopic {
            learning: learning.clone(),
        },
        MergeOutcome::AppendInsight {
            learning: learning.clone(),
            existing_topic: topic,
        },
        MergeOutcome::NoOpDuplicate {
            learning,
            existing_slug: "existing".into(),
            reason: "Dup".into(),
        },
    ];

    let summary = merge::dry_run_summary(&outcomes);
    assert!(summary.contains("1 create"));
    assert!(summary.contains("1 append"));
    assert!(summary.contains("1 skip"));
}

// -- State persistence tests --

#[test]
fn test_state_persistence() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("state.json");

    let mut state = LoopState::default();
    state.mark_processed(1, 100);
    state.mark_processed(2, 200);
    state.add_extraction_hash("abc123".to_string());
    state.last_poll_at = Some(chrono::Utc::now());

    state.save(&state_path).unwrap();

    let loaded = LoopState::load(&state_path).unwrap();
    assert!(loaded.is_processed(1, 100));
    assert!(loaded.is_processed(2, 200));
    assert!(loaded.has_extraction_hash("abc123"));
    assert!(loaded.last_poll_at.is_some());
}

#[test]
fn test_state_fresh_start() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("nonexistent.json");

    let state = LoopState::load(&state_path).unwrap();
    assert!(state.processed.is_empty());
    assert!(state.last_poll_at.is_none());
}

// -- Transcript reduction tests --

#[test]
fn test_transcript_reduction() {
    use oversight::source::types::TypedTurn;
    use serde_json::json;

    let turns = vec![
        TypedTurn {
            turn_id: Some(1),
            depth: None,
            data: json!({"item_type": "tool_call", "tool_call": {"name": "bash", "call_id": "tc_1", "args": "gh pr list"}}),
            declared_type: None,
        },
        TypedTurn {
            turn_id: Some(2),
            depth: None,
            data: json!({"item_type": "tool_result", "tool_result": {"call_id": "tc_1", "content": "authentication required", "is_error": true}}),
            declared_type: None,
        },
        TypedTurn {
            turn_id: Some(3),
            depth: None,
            data: json!({"item_type": "assistant", "assistant": {"text": "The error suggests we need to fix auth by unsetting GITHUB_TOKEN."}}),
            declared_type: None,
        },
        TypedTurn {
            turn_id: Some(4),
            depth: None,
            data: json!({"item_type": "tool_call", "tool_call": {"name": "bash", "call_id": "tc_2", "args": "unset GITHUB_TOKEN && gh pr list"}}),
            declared_type: None,
        },
        TypedTurn {
            turn_id: Some(5),
            depth: None,
            data: json!({"item_type": "tool_result", "tool_result": {"call_id": "tc_2", "content": "Showing 3 of 3 open pull requests", "is_error": false}}),
            declared_type: None,
        },
        // This user input should be filtered (no error keywords)
        TypedTurn {
            turn_id: Some(6),
            depth: None,
            data: json!({"item_type": "user_input", "user_input": {"text": "Great, thanks!"}}),
            declared_type: None,
        },
    ];

    let transcript = transcript::reduce_transcript(&turns, 10000);
    assert!(transcript.contains("TOOL_CALL: bash"));
    assert!(transcript.contains("authentication required"));
    assert!(transcript.contains("GITHUB_TOKEN"));
    // The user "Great, thanks!" should be filtered
    assert!(!transcript.contains("Great, thanks!"));
}

// -- Content hash dedup tests --

#[test]
fn test_learning_content_hash_stability() {
    let l1 = make_learning("gh-cli", "Test Title", "Test summary", &[]);
    let l2 = make_learning("gh-cli", "Test Title", "Test summary", &[]);

    assert_eq!(l1.content_hash(), l2.content_hash());
}

#[test]
fn test_learning_content_hash_case_insensitive() {
    let l1 = make_learning("GH-CLI", "Test Title", "Test summary", &[]);
    let l2 = make_learning("gh-cli", "test title", "test summary", &[]);

    assert_eq!(l1.content_hash(), l2.content_hash());
}

#[test]
fn test_learning_content_hash_different() {
    let l1 = make_learning("gh-cli", "Title A", "Summary A", &[]);
    let l2 = make_learning("docker", "Title B", "Summary B", &[]);

    assert_ne!(l1.content_hash(), l2.content_hash());
}

// -- End-to-end pipeline test with mocks --

#[test]
fn test_full_pipeline_create_and_append() {
    let (_dir, service) = setup_kb();

    // Pre-populate with one topic
    service
        .add_topic("GH CLI", "# GH CLI\n\nBasic usage guide.\n", vec!["cli".into()], vec![])
        .unwrap();

    // Simulate learnings extracted by LLM
    let learnings = vec![
        // This should APPEND to existing gh-cli topic
        make_learning("gh-cli", "Auth token workaround", "Unset GITHUB_TOKEN env var before using gh.", &["cli", "auth"]),
        // This should CREATE a new topic
        make_learning("docker-compose", "Use docker compose v2", "The v2 plugin replaces docker-compose binary.", &["docker"]),
    ];

    // Load all existing topics for dedup
    let summaries = service.list_topics().unwrap();
    let mut existing_topics = Vec::new();
    for s in &summaries {
        if let Ok(t) = service.get_topic(&s.slug) {
            existing_topics.push(t);
        }
    }

    // Dedup
    let outcomes = dedupe::deduplicate(&learnings, &existing_topics, &DedupePolicy::balanced());
    assert_eq!(outcomes.len(), 2);

    // Verify classification
    assert!(
        matches!(&outcomes[0], MergeOutcome::AppendInsight { .. }),
        "Expected AppendInsight for gh-cli learning"
    );
    assert!(
        matches!(&outcomes[1], MergeOutcome::CreateTopic { .. }),
        "Expected CreateTopic for docker-compose learning"
    );

    // Apply merges
    let result = merge::apply_merges(&service, &outcomes, 100).unwrap();
    assert_eq!(result.topics_created, 1);
    assert_eq!(result.topics_updated, 1);

    // Verify the results
    let gh_topic = service.get_topic("gh-cli").unwrap();
    assert!(gh_topic.body.contains("Key Insights"));
    assert!(gh_topic.body.contains("Auth token workaround"));
    assert!(gh_topic.body.contains("context 100"));
    assert!(gh_topic.tags.contains(&"auth".to_string()));

    let docker_topic = service.get_topic("docker-compose").unwrap();
    assert!(docker_topic.body.contains("docker compose v2"));
    assert_eq!(docker_topic.source.as_deref(), Some("healing-loop"));
    assert_eq!(docker_topic.source_context_id, Some(100));

    // Verify topic list grew
    let all_topics = service.list_topics().unwrap();
    assert_eq!(all_topics.len(), 2);
}
