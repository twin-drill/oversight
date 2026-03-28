//! Tests for the KB topic regime feature (Sprint 004).
//!
//! Validates that:
//! - `balanced` regime produces identical outcomes to pre-regime behavior
//! - `aggressive` creates more topics than `balanced`
//! - `conservative` creates fewer topics (more appends/skips) than `balanced`
//! - Coverage threshold, title match mode, and config parsing all work correctly

use oversight::config::{Config, DedupeConfig, LoopConfig};
use oversight::healing_loop::dedupe::{self, MergeOutcome};
use oversight::healing_loop::policy::{jaccard_similarity, DedupePolicy, Regime, TitleMatchMode};
use oversight::llm::extractor::{self, Learning};
use oversight::kb::types::Topic;

// -- Helpers --

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

/// A shared fixture: a set of existing topics and learnings that exercises
/// multiple matching paths (slug, title, tags, coverage).
fn fixture() -> (Vec<Topic>, Vec<Learning>) {
    let topics = vec![
        make_topic(
            "gh-cli",
            "GitHub CLI",
            &["github cli"],
            &["cli", "github", "auth"],
            "# GitHub CLI\n\n## Key Insights\n\nUse gh auth login for authentication.\nThe GITHUB_TOKEN env var overrides keychain auth.\n",
        ),
        make_topic(
            "docker-compose",
            "Docker Compose",
            &[],
            &["docker", "containers"],
            "# Docker Compose\n\nBasic docker compose up usage.\n",
        ),
        make_topic(
            "aws-sso",
            "AWS SSO Login",
            &[],
            &["aws", "sso", "auth"],
            "# AWS SSO\n\nUse aws sso login --profile to authenticate.\n",
        ),
    ];

    let learnings = vec![
        // L0: Exact slug match to gh-cli, title appears in body -> NoOpDuplicate (all regimes)
        make_learning(
            "gh-cli",
            "GITHUB_TOKEN env var",
            "The GITHUB_TOKEN env var overrides keychain auth when using gh cli commands.",
            &["cli", "github"],
        ),
        // L1: Slug match to docker-compose, but novel content -> AppendInsight (all regimes)
        make_learning(
            "docker-compose",
            "Docker compose v2 plugin",
            "The docker compose v2 plugin replaces the standalone binary.",
            &["docker"],
        ),
        // L2: No existing topic for kubectl -> CreateTopic (all regimes)
        make_learning(
            "kubectl",
            "Kubectl context switching",
            "Use kubectl config use-context to switch clusters.",
            &["kubernetes", "cli"],
        ),
        // L3: 3 tag overlap with gh-cli (cli, github, auth) + slug affinity ("gh-cli-auth" contains "gh-cli")
        //     Balanced: 3 >= 2 -> match -> AppendInsight
        //     Aggressive: 3 >= 3 -> match -> AppendInsight
        make_learning(
            "gh-cli-auth",
            "gh auth refresh token",
            "Refresh tokens can be used to renew expired sessions automatically.",
            &["cli", "github", "auth"],
        ),
        // L4: 2 tag overlap with aws-sso (aws, auth), no slug affinity
        //     Balanced: 2 >= 2 but require_slug_affinity=true, slugs unrelated -> no match -> CreateTopic
        //     Conservative: 2 >= 1, require_slug_affinity=false -> match -> AppendInsight
        make_learning(
            "aws-credentials",
            "AWS credential file location",
            "Credentials are stored in ~/.aws/credentials by default.",
            &["aws", "auth"],
        ),
        // L5: 2 tag overlap with gh-cli (cli, github), slug unrelated ("npm-auth" doesn't contain "gh-cli")
        //     Balanced: 2 >= 2 but require_slug_affinity=true, slugs unrelated -> no match -> CreateTopic
        //     Aggressive: 2 < 3 -> no tag match -> CreateTopic
        //     Conservative: 2 >= 1, require_slug_affinity=false -> match -> AppendInsight
        make_learning(
            "npm-auth",
            "NPM auth tokens",
            "NPM uses auth tokens stored in .npmrc for registry authentication.",
            &["cli", "github"],
        ),
    ];

    (topics, learnings)
}

fn count_outcomes(outcomes: &[MergeOutcome]) -> (usize, usize, usize) {
    let creates = outcomes
        .iter()
        .filter(|o| matches!(o, MergeOutcome::CreateTopic { .. }))
        .count();
    let appends = outcomes
        .iter()
        .filter(|o| matches!(o, MergeOutcome::AppendInsight { .. }))
        .count();
    let skips = outcomes
        .iter()
        .filter(|o| matches!(o, MergeOutcome::NoOpDuplicate { .. }))
        .count();
    (creates, appends, skips)
}

// -- Backward-compat fixture test --

#[test]
fn test_balanced_matches_pre_regime_behavior() {
    let (topics, learnings) = fixture();
    let policy = DedupePolicy::balanced();
    let outcomes = dedupe::deduplicate(&learnings, &topics, &policy);

    assert_eq!(outcomes.len(), 6);

    // L0: slug matches gh-cli, title appears in body -> NoOpDuplicate
    assert!(
        matches!(&outcomes[0], MergeOutcome::NoOpDuplicate { .. }),
        "L0: Expected NoOpDuplicate, got: {}",
        outcomes[0].summary()
    );

    // L1: slug matches docker-compose, novel content -> AppendInsight
    assert!(
        matches!(&outcomes[1], MergeOutcome::AppendInsight { .. }),
        "L1: Expected AppendInsight, got: {}",
        outcomes[1].summary()
    );

    // L2: no match for kubectl -> CreateTopic
    assert!(
        matches!(&outcomes[2], MergeOutcome::CreateTopic { .. }),
        "L2: Expected CreateTopic, got: {}",
        outcomes[2].summary()
    );

    // L3: 3 tag overlap with gh-cli + slug affinity -> AppendInsight
    assert!(
        matches!(&outcomes[3], MergeOutcome::AppendInsight { .. }),
        "L3: Expected AppendInsight under balanced (tag match + slug affinity), got: {}",
        outcomes[3].summary()
    );

    // L4: tags {aws, auth} vs aws-sso {aws, sso, auth} — Jaccard 2/3=0.67 >= 0.5 -> AppendInsight
    assert!(
        matches!(&outcomes[4], MergeOutcome::AppendInsight { .. }),
        "L4: Expected AppendInsight under balanced (tag Jaccard match), got: {}",
        outcomes[4].summary()
    );

    // L5: tags {cli, github} vs gh-cli {cli, github, auth} — Jaccard 2/3=0.67 >= 0.5 -> AppendInsight
    assert!(
        matches!(&outcomes[5], MergeOutcome::AppendInsight { .. }),
        "L5: Expected AppendInsight under balanced (tag Jaccard match), got: {}",
        outcomes[5].summary()
    );
}

// -- Regime delta tests --

#[test]
fn test_aggressive_creates_more_than_balanced() {
    // Targeted fixture: a topic with 2 tag overlap and slug affinity.
    // Under balanced (tag_min=2), this matches. Under aggressive (tag_min=3), it doesn't.
    let topics = vec![
        make_topic(
            "gh-cli",
            "GH CLI",
            &[],
            &["cli", "github"],
            "# GH CLI\n\nBasic usage.\n",
        ),
    ];

    let learnings = vec![
        // 2 tags overlap (cli, github) + slug affinity ("gh-cli-auth" contains "gh-cli")
        // Balanced: 2 >= 2 + slug affinity -> match -> AppendInsight
        // Aggressive: 2 < 3 -> no tag match, no title match (Exact: "gh-cli" != "gh-cli-auth") -> CreateTopic
        make_learning(
            "gh-cli-auth",
            "gh auth refresh token",
            "Refresh tokens can be used to renew expired sessions automatically.",
            &["cli", "github"],
        ),
        // No match at all -> CreateTopic (both)
        make_learning(
            "kubectl",
            "Kubectl context switching",
            "Use kubectl config use-context to switch clusters.",
            &["kubernetes"],
        ),
    ];

    let balanced_outcomes = dedupe::deduplicate(&learnings, &topics, &DedupePolicy::balanced());
    let aggressive_outcomes = dedupe::deduplicate(&learnings, &topics, &DedupePolicy::aggressive());

    let (bal_creates, _, _) = count_outcomes(&balanced_outcomes);
    let (agg_creates, _, _) = count_outcomes(&aggressive_outcomes);

    assert!(
        agg_creates > bal_creates,
        "Aggressive should create strictly more topics than balanced. Aggressive={}, Balanced={}",
        agg_creates,
        bal_creates
    );

    // Balanced: L0 matches via tag overlap -> AppendInsight
    assert!(
        matches!(&balanced_outcomes[0], MergeOutcome::AppendInsight { .. }),
        "L0 under balanced: Expected AppendInsight (2 tags >= 2 + slug affinity). Got: {}",
        balanced_outcomes[0].summary()
    );

    // Aggressive: L0 does not match (2 < 3 tag min, Exact title mismatch) -> CreateTopic
    assert!(
        matches!(&aggressive_outcomes[0], MergeOutcome::CreateTopic { .. }),
        "L0 under aggressive: Expected CreateTopic (2 < 3 tag min). Got: {}",
        aggressive_outcomes[0].summary()
    );
}

#[test]
fn test_conservative_creates_fewer_than_balanced() {
    // Topic with 4 tags
    let topics = vec![make_topic(
        "deploy-pipeline",
        "Deploy Pipeline",
        &[],
        &["ci", "deploy", "aws", "docker"],
        "# Deploy\n\nPipeline details.\n",
    )];

    let learnings = vec![
        // 1 tag overlap (ci) out of union of 6 -> Jaccard 1/6=0.17
        // Neither balanced nor conservative catches this -> CreateTopic (both)
        make_learning(
            "ci-config",
            "CI config",
            "CI config for tests.",
            &["ci", "testing", "github-actions"],
        ),
        // 2 tag overlap (ci, aws) out of union of 5 -> Jaccard 2/5=0.4
        // Conservative (0.35): match -> AppendInsight
        // Balanced (0.5): no match -> CreateTopic
        make_learning(
            "cloud-deploy",
            "Cloud deployment",
            "Deploy to cloud infra.",
            &["ci", "aws", "terraform"],
        ),
    ];

    let balanced_outcomes = dedupe::deduplicate(&learnings, &topics, &DedupePolicy::balanced());
    let conservative_outcomes =
        dedupe::deduplicate(&learnings, &topics, &DedupePolicy::conservative());

    let (bal_creates, _, _) = count_outcomes(&balanced_outcomes);
    let (con_creates, _, _) = count_outcomes(&conservative_outcomes);

    assert!(
        con_creates < bal_creates,
        "Conservative should create strictly fewer topics than balanced. Conservative={}, Balanced={}",
        con_creates,
        bal_creates
    );
}

// Balanced L4/L5 now match via tag Jaccard — verify the old assertions are updated
#[test]
fn test_balanced_catches_semantic_duplicates_via_tag_jaccard() {
    let (topics, learnings) = fixture();
    let outcomes = dedupe::deduplicate(&learnings, &topics, &DedupePolicy::balanced());

    // L4: aws-credentials {aws, auth} vs aws-sso {aws, sso, auth} -> Jaccard 0.67
    assert!(
        matches!(&outcomes[4], MergeOutcome::AppendInsight { .. }),
        "L4 should match aws-sso via tag Jaccard, got: {}",
        outcomes[4].summary()
    );

    // L5: npm-auth {cli, github} vs gh-cli {cli, github, auth} -> Jaccard 0.67
    assert!(
        matches!(&outcomes[5], MergeOutcome::AppendInsight { .. }),
        "L5 should match gh-cli via tag Jaccard, got: {}",
        outcomes[5].summary()
    );
}

// -- Coverage threshold test --

#[test]
fn test_coverage_threshold_sensitivity() {
    // Create a topic with known body content
    let topic = make_topic(
        "test-tool",
        "Test Tool",
        &[],
        &[],
        "# Test Tool\n\nUse alpha beta gamma delta epsilon zeta eta theta iota kappa lambda.\n",
    );

    // Summary: 11 significant words (>= 4 chars), 9 match the body.
    // Matching: alpha, beta, gamma, delta, epsilon, zeta, theta, kappa, lambda = 9
    // Not matching: something, fresh = 2
    // Ratio: 9/11 ~= 0.818
    let learning = make_learning(
        "test-tool",
        "Novel title not in body",
        "Use alpha beta gamma delta epsilon zeta theta kappa lambda for something fresh.",
        &[],
    );

    // Conservative threshold=0.6: 81.8% > 60% -> covered (NoOpDuplicate)
    let conservative_outcomes = dedupe::deduplicate(std::slice::from_ref(&learning), std::slice::from_ref(&topic), &DedupePolicy::conservative());
    assert!(
        matches!(&conservative_outcomes[0], MergeOutcome::NoOpDuplicate { .. }),
        "Under conservative (threshold=0.6), ~82% overlap should be covered. Got: {}",
        conservative_outcomes[0].summary()
    );

    // Aggressive threshold=0.95: 81.8% < 95% -> not covered (AppendInsight)
    let aggressive_outcomes = dedupe::deduplicate(std::slice::from_ref(&learning), std::slice::from_ref(&topic), &DedupePolicy::aggressive());
    assert!(
        matches!(&aggressive_outcomes[0], MergeOutcome::AppendInsight { .. }),
        "Under aggressive (threshold=0.95), ~82% overlap should NOT be covered. Got: {}",
        aggressive_outcomes[0].summary()
    );

    // Balanced threshold=0.8: 81.8% > 80% -> covered (NoOpDuplicate)
    let balanced_outcomes = dedupe::deduplicate(&[learning], &[topic], &DedupePolicy::balanced());
    assert!(
        matches!(&balanced_outcomes[0], MergeOutcome::NoOpDuplicate { .. }),
        "Under balanced (threshold=0.8), ~82% overlap should be covered. Got: {}",
        balanced_outcomes[0].summary()
    );
}

// -- Title match mode tests --

#[test]
fn test_title_match_fuzzy_vs_exact() {
    // Use a topic whose title, when slugified, gives "gh-cli"
    // and a learning whose hint slug is "gh-cli-auth".
    // They share tokens {gh, cli} but differ in full slug.
    let topic = make_topic("gh-cli", "GH CLI", &[], &[], "# GH CLI\n\nBasic usage.\n");

    // Hint "gh-cli-auth" -> slug "gh-cli-auth", hint_normalized = "gh cli auth"
    // Topic title "GH CLI" -> title_normalized = "gh cli", title_slug = "gh-cli"
    let learning = make_learning(
        "gh-cli-auth",
        "gh auth refresh",
        "Completely novel auth refresh mechanism.",
        &[],
    );

    // Exact mode: title_slug "gh-cli" != hint_slug "gh-cli-auth" -> no title match -> CreateTopic
    let exact_policy = DedupePolicy {
        title_match_mode: TitleMatchMode::Exact,
        ..DedupePolicy::balanced()
    };
    let outcomes = dedupe::deduplicate(std::slice::from_ref(&learning), std::slice::from_ref(&topic), &exact_policy);
    assert!(
        matches!(&outcomes[0], MergeOutcome::CreateTopic { .. }),
        "Exact title match should not match 'gh-cli-auth' to 'gh-cli'. Got: {}",
        outcomes[0].summary()
    );

    // FuzzyTokenJaccard mode: title_slug "gh-cli" vs hint_slug "gh-cli-auth"
    // Jaccard = |{gh,cli} intersect {gh,cli,auth}| / |{gh,cli} union {gh,cli,auth}| = 2/3 ~= 0.67 >= 0.5
    let fuzzy_policy = DedupePolicy {
        title_match_mode: TitleMatchMode::FuzzyTokenJaccard { min_similarity: 0.5 },
        ..DedupePolicy::balanced()
    };
    let outcomes = dedupe::deduplicate(std::slice::from_ref(&learning), std::slice::from_ref(&topic), &fuzzy_policy);
    assert!(
        matches!(&outcomes[0], MergeOutcome::AppendInsight { .. }),
        "Fuzzy title match should match 'gh-cli-auth' to 'gh-cli' (Jaccard ~0.67). Got: {}",
        outcomes[0].summary()
    );

    // Contains mode (default balanced): hint_normalized "gh cli auth" contains
    // title_normalized.replace(' ', '-') = "gh-cli"? "gh cli auth".contains("gh-cli") -> No.
    // But title_normalized "gh cli" is contained in hint_normalized "gh cli auth"? No because
    // we check hint_normalized.contains(&title_normalized.replace(' ', "-")) = "gh cli auth".contains("gh-cli") -> No.
    // And title_normalized.contains(hint_normalized) = "gh cli".contains("gh cli auth") -> No.
    // And title_normalized == hint_normalized? No.
    // So Contains mode also doesn't match here (only slug/alias/tag paths would).
    // With balanced policy, tag_overlap_minimum=2 but no tags -> CreateTopic
    let contains_policy = DedupePolicy::balanced();
    let outcomes = dedupe::deduplicate(&[learning], &[topic], &contains_policy);
    assert!(
        matches!(&outcomes[0], MergeOutcome::CreateTopic { .. }),
        "Contains title match should not match 'gh-cli-auth' to 'GH CLI' (no substring match). Got: {}",
        outcomes[0].summary()
    );
}

// -- Config parsing tests --

#[test]
fn test_config_regime_parsing() {
    let toml = r#"
kb_path = "/tmp/test-kb"

[loop]
regime = "aggressive"
confidence_threshold = 0.8

[loop.dedupe]
coverage_threshold = 0.85
tag_overlap_minimum = 3
"#;

    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(&config_path, toml).unwrap();

    let config = Config::from_file(&config_path).unwrap();
    assert_eq!(config.loop_config.regime, Regime::Aggressive);

    let dedupe = config.loop_config.dedupe.as_ref().unwrap();
    assert_eq!(dedupe.coverage_threshold, Some(0.85));
    assert_eq!(dedupe.tag_overlap_minimum, Some(3));
}

#[test]
fn test_config_regime_with_overrides_builds_policy() {
    let toml = r#"
kb_path = "/tmp/test-kb"

[loop]
regime = "conservative"

[loop.dedupe]
coverage_threshold = 0.75
require_slug_affinity = true
title_match_mode = "exact"
"#;

    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(&config_path, toml).unwrap();

    let config = Config::from_file(&config_path).unwrap();
    let policy = config.loop_config.build_dedupe_policy(None).unwrap();

    // Base is conservative, but overrides should be applied
    assert_eq!(policy.regime, Regime::Conservative);
    assert!((policy.coverage_threshold - 0.75).abs() < f64::EPSILON);
    assert!(policy.require_slug_affinity); // overridden from false to true
    assert_eq!(policy.title_match_mode, TitleMatchMode::Exact); // overridden from fuzzy
}

#[test]
fn test_config_cli_regime_override() {
    let config = LoopConfig::default();
    assert_eq!(config.regime, Regime::Balanced);

    // CLI override should take precedence
    let policy = config.build_dedupe_policy(Some(&Regime::Aggressive)).unwrap();
    assert_eq!(policy.regime, Regime::Aggressive);
}

#[test]
fn test_config_default_regime_is_balanced() {
    let config = LoopConfig::default();
    let policy = config.build_dedupe_policy(None).unwrap();
    assert_eq!(policy.regime, Regime::Balanced);
    assert!((policy.coverage_threshold - 0.8).abs() < f64::EPSILON);
}

// -- CLI flag test --

#[test]
fn test_regime_from_str_valid() {
    assert_eq!("aggressive".parse::<Regime>().unwrap(), Regime::Aggressive);
    assert_eq!("balanced".parse::<Regime>().unwrap(), Regime::Balanced);
    assert_eq!("conservative".parse::<Regime>().unwrap(), Regime::Conservative);
    assert_eq!("AGGRESSIVE".parse::<Regime>().unwrap(), Regime::Aggressive);
}

#[test]
fn test_regime_from_str_invalid() {
    let result = "invalid".parse::<Regime>();
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("aggressive"));
    assert!(err.contains("balanced"));
    assert!(err.contains("conservative"));
}

// -- Prompt modifier tests --

#[test]
fn test_prompt_modifier_aggressive() {
    let modifier = extractor::regime_prompt_modifier(&Regime::Aggressive);
    assert!(modifier.is_some());
    assert!(modifier.unwrap().contains("fine-grained"));
}

#[test]
fn test_prompt_modifier_balanced_is_none() {
    let modifier = extractor::regime_prompt_modifier(&Regime::Balanced);
    assert!(modifier.is_none());
}

#[test]
fn test_prompt_modifier_conservative() {
    let modifier = extractor::regime_prompt_modifier(&Regime::Conservative);
    assert!(modifier.is_some());
    assert!(modifier.unwrap().contains("Consolidate"));
}

// -- Jaccard similarity edge cases --

#[test]
fn test_jaccard_shared_prefix() {
    // "docker-compose" vs "docker-compose-v2"
    let sim = jaccard_similarity("docker-compose", "docker-compose-v2");
    // {docker, compose} vs {docker, compose, v2} -> 2/3 ~= 0.667
    assert!(sim > 0.6 && sim < 0.7);
}

#[test]
fn test_jaccard_single_token() {
    assert!((jaccard_similarity("docker", "docker") - 1.0).abs() < f64::EPSILON);
    assert!((jaccard_similarity("docker", "kubectl") - 0.0).abs() < f64::EPSILON);
}

// -- Config write/read roundtrip --

#[test]
fn test_config_write_read_regime_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("config.toml");

    let mut config = Config::default();
    config.loop_config.regime = Regime::Aggressive;
    config.loop_config.dedupe = Some(DedupeConfig {
        coverage_threshold: Some(0.9),
        tag_overlap_minimum: Some(4),
        require_slug_affinity: Some(false),
        title_match_mode: Some("fuzzy".to_string()),
    });

    config.write_to_file(&config_path).unwrap();

    let loaded = Config::from_file(&config_path).unwrap();
    assert_eq!(loaded.loop_config.regime, Regime::Aggressive);

    let dedupe = loaded.loop_config.dedupe.unwrap();
    assert_eq!(dedupe.coverage_threshold, Some(0.9));
    assert_eq!(dedupe.tag_overlap_minimum, Some(4));
    assert_eq!(dedupe.require_slug_affinity, Some(false));
    assert_eq!(dedupe.title_match_mode.as_deref(), Some("fuzzy"));
}
