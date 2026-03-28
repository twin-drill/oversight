use oversight::integrate::manager::{self, IntegrationAction, MarkerHealth};
use oversight::integrate::targets::IntegrationTarget;
use oversight::kb::types::TopicSummary;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

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

fn target_in(dir: &TempDir) -> (IntegrationTarget, PathBuf) {
    let path = dir.path().join("CLAUDE.md");
    let mut target = IntegrationTarget::claude_code();
    target.default_path = path.clone();
    (target, path)
}

// --- Install tests ---

#[test]
fn test_install_when_file_absent_creates_file() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let topics = make_topics(&["gh-cli", "aws-sso"]);

    let result = manager::install(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::Created);
    assert!(path.exists());

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("oversight:begin target=claude-code"));
    assert!(content.contains("oversight topics"));
    assert!(content.contains("oversight:end"));
}

#[test]
fn test_install_into_existing_file_with_other_content() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);

    let original = "# My Global Config\n\nDo not modify this manually.\n\n## Important Rules\n\n- Rule 1\n- Rule 2\n";
    fs::write(&path, original).unwrap();

    let topics = make_topics(&["docker-local"]);
    let result = manager::install(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::Inserted);

    let content = fs::read_to_string(&path).unwrap();
    // Original content preserved
    assert!(content.contains("# My Global Config"));
    assert!(content.contains("Do not modify this manually."));
    assert!(content.contains("## Important Rules"));
    assert!(content.contains("- Rule 1"));
    // Managed block present
    assert!(content.contains("oversight:begin target=claude-code"));
    assert!(content.contains("oversight topics"));
}

#[test]
fn test_rerun_install_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let topics = make_topics(&["gh-cli"]);

    // First install
    manager::install(&target, None, false).unwrap();
    let content_v1 = fs::read_to_string(&path).unwrap();

    // Second install (same topics) should be NoChange
    let result = manager::install(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);

    let content_v2 = fs::read_to_string(&path).unwrap();
    assert_eq!(content_v1, content_v2, "Content should be identical after idempotent install");
}

#[test]
fn test_install_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let (target, _path) = target_in(&dir);

    manager::install(&target, None, false).unwrap();
    let result = manager::install(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);
}

// --- Refresh tests ---

#[test]
fn test_refresh_is_no_change_with_static_block() {
    let dir = TempDir::new().unwrap();
    let (target, _path) = target_in(&dir);

    manager::install(&target, None, false).unwrap();
    let result = manager::refresh(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);
}

#[test]
fn test_refresh_preserves_surrounding_content() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);

    let original = "# Header\n\nSome instructions.\n";
    fs::write(&path, original).unwrap();

    // Install
    let topics_v1 = make_topics(&["gh-cli"]);
    manager::install(&target, None, false).unwrap();

    // Refresh
    let topics_v2 = make_topics(&["gh-cli", "aws-sso"]);
    manager::refresh(&target, None, false).unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("# Header"));
    assert!(content.contains("Some instructions."));
    assert!(content.contains("oversight topics"));
}

#[test]
fn test_refresh_no_block_returns_no_change() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    fs::write(&path, "# No block\n").unwrap();

    let topics = make_topics(&["gh-cli"]);
    let result = manager::refresh(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);

    // Content unchanged
    assert_eq!(fs::read_to_string(&path).unwrap(), "# No block\n");
}

#[test]
fn test_refresh_file_absent_returns_no_change() {
    let dir = TempDir::new().unwrap();
    let (target, _path) = target_in(&dir);
    let topics = make_topics(&["gh-cli"]);
    let result = manager::refresh(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);
}

// --- Remove tests ---

#[test]
fn test_remove_cleans_only_managed_block() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);

    let original = "# Header\n\nImportant stuff.\n";
    fs::write(&path, original).unwrap();

    let topics = make_topics(&["gh-cli"]);
    manager::install(&target, None, false).unwrap();

    // Verify block was added
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("oversight:begin"));

    // Remove
    let result = manager::remove(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::Removed);

    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.contains("oversight:begin"));
    assert!(!content.contains("oversight:end"));
    assert!(content.contains("# Header"));
    assert!(content.contains("Important stuff."));
}

#[test]
fn test_remove_deletes_file_when_only_managed() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let topics = make_topics(&["gh-cli"]);

    manager::install(&target, None, false).unwrap();
    assert!(path.exists());

    let result = manager::remove(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::FileRemoved);
    assert!(!path.exists());
}

#[test]
fn test_remove_idempotent_when_no_block() {
    let dir = TempDir::new().unwrap();
    let (target, _) = target_in(&dir);

    let result = manager::remove(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);
}

#[test]
fn test_remove_idempotent_after_removal() {
    let dir = TempDir::new().unwrap();
    let (target, _) = target_in(&dir);
    let topics = make_topics(&["gh-cli"]);

    manager::install(&target, None, false).unwrap();
    manager::remove(&target, None, false).unwrap();

    let result = manager::remove(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::NoChange);
}

// --- Dry-run tests ---

#[test]
fn test_dry_run_install_no_file_changes() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let topics = make_topics(&["gh-cli"]);

    let result = manager::install(&target, None, true).unwrap();
    assert!(matches!(result.action, IntegrationAction::DryRun(_)));
    assert!(!path.exists(), "Dry run must not create the file");
}

#[test]
fn test_dry_run_refresh_no_file_changes() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let topics_v1 = make_topics(&["gh-cli"]);
    manager::install(&target, None, false).unwrap();
    let content_before = fs::read_to_string(&path).unwrap();

    let topics_v2 = make_topics(&["gh-cli", "aws-sso"]);
    let result = manager::refresh(&target, None, true).unwrap();
    assert!(matches!(result.action, IntegrationAction::DryRun(_)));

    let content_after = fs::read_to_string(&path).unwrap();
    assert_eq!(content_before, content_after, "Dry run must not modify file");
}

// --- Empty KB tests ---

#[test]
fn test_empty_kb_install_succeeds() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);

    let result = manager::install(&target, None, false).unwrap();
    assert_eq!(result.action, IntegrationAction::Created);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("oversight topics"));
    assert!(content.contains("oversight:begin"));
    assert!(content.contains("oversight:end"));
}

// --- Preview limit tests ---

#[test]
fn test_block_contains_instructions_not_topics() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);

    manager::install(&target, None, false).unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("oversight topics"));
    assert!(content.contains("oversight search"));
    assert!(content.contains("oversight read"));
    assert!(!content.contains("Current topics:"));
}

// --- Status tests ---

#[test]
fn test_status_not_installed() {
    let dir = TempDir::new().unwrap();
    let (target, _) = target_in(&dir);
    let st = manager::status(&target, None);
    assert!(!st.installed);
    assert!(!st.file_exists);
    assert_eq!(st.marker_health, MarkerHealth::FileAbsent);
}

#[test]
fn test_status_installed_reports_topic_count() {
    let dir = TempDir::new().unwrap();
    let (target, _) = target_in(&dir);
    let topics = make_topics(&["gh-cli", "aws-sso", "docker"]);
    manager::install(&target, None, false).unwrap();

    let st = manager::status(&target, None);
    assert!(st.installed);
    assert!(st.file_exists);
    assert_eq!(st.marker_health, MarkerHealth::Healthy);
    assert!(st.installed);
}

#[test]
fn test_status_file_exists_no_block() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    fs::write(&path, "# Just config\n").unwrap();

    let st = manager::status(&target, None);
    assert!(!st.installed);
    assert!(st.file_exists);
    assert_eq!(st.marker_health, MarkerHealth::NotInstalled);
}

// --- Malformed markers tests ---

#[test]
fn test_malformed_missing_end_marker_error() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let malformed = "# Config\n\n<!-- oversight:begin target=claude-code -->\nOrphan block\n";
    fs::write(&path, malformed).unwrap();

    let err = manager::install(&target, None, false).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("no matching end marker"),
        "Expected actionable error, got: {msg}"
    );
}

#[test]
fn test_malformed_duplicate_blocks_error() {
    let dir = TempDir::new().unwrap();
    let (target, path) = target_in(&dir);
    let malformed = "\
<!-- oversight:begin target=claude-code -->
Block A
<!-- oversight:end -->
<!-- oversight:begin target=claude-code -->
Block B
<!-- oversight:end -->
";
    fs::write(&path, malformed).unwrap();

    let err = manager::install(&target, None, false).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("2 begin markers"),
        "Expected duplicate block error, got: {msg}"
    );
}
