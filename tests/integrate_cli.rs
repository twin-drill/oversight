use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn oversight_cmd() -> Command {
    Command::cargo_bin("oversight").unwrap()
}

/// Helper: create an initialized KB and return (dir, kb_path).
fn setup_kb() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    (dir, kb_path)
}

/// Helper: add a topic to the KB.
fn add_topic(kb_path: &std::path::Path, name: &str, body: &str) {
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", name])
        .write_stdin(body)
        .assert()
        .success();
}

// --- Install tests ---

#[test]
fn test_cli_integrate_install_creates_file() {
    let (dir, kb_path) = setup_kb();
    add_topic(&kb_path, "GH CLI", "# GitHub CLI\nUse gh.\n");

    let target_path = dir.path().join("claude-config").join("CLAUDE.md");

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--target",
            "claude-code",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created file with managed block"));

    assert!(target_path.exists());
    let content = fs::read_to_string(&target_path).unwrap();
    assert!(content.contains("oversight:begin target=claude-code"));
    assert!(content.contains("gh-cli"));
    assert!(content.contains("oversight:end"));
}

#[test]
fn test_cli_integrate_install_idempotent() {
    let (dir, kb_path) = setup_kb();
    add_topic(&kb_path, "Docker", "Docker stuff.\n");

    let target_path = dir.path().join("CLAUDE.md");

    // First install
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Second install should report no changes
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes needed"));
}

#[test]
fn test_cli_integrate_install_empty_kb() {
    let (dir, kb_path) = setup_kb();
    let target_path = dir.path().join("CLAUDE.md");

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = fs::read_to_string(&target_path).unwrap();
    assert!(content.contains("No topics yet"));
}

#[test]
fn test_cli_integrate_install_dry_run() {
    let (dir, kb_path) = setup_kb();
    let target_path = dir.path().join("CLAUDE.md");

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--path",
            target_path.to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would:"));

    assert!(!target_path.exists(), "Dry run should not create file");
}

// --- Refresh tests ---

#[test]
fn test_cli_integrate_refresh_updates_topics() {
    let (dir, kb_path) = setup_kb();
    let target_path = dir.path().join("CLAUDE.md");

    add_topic(&kb_path, "GH CLI", "GitHub CLI.\n");

    // Install
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Add another topic
    add_topic(&kb_path, "AWS SSO", "AWS SSO.\n");

    // Refresh with --path so it operates on our custom file
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "refresh",
            "--target",
            "claude-code",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = fs::read_to_string(&target_path).unwrap();
    assert!(content.contains("aws-sso"), "Refreshed block should contain new topic");
    assert!(content.contains("gh-cli"), "Refreshed block should still contain original topic");
}

// --- Remove tests ---

#[test]
fn test_cli_integrate_remove() {
    let (dir, kb_path) = setup_kb();
    let target_path = dir.path().join("CLAUDE.md");

    // Write existing content + install
    fs::write(&target_path, "# My Config\n").unwrap();

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify block is present
    let content = fs::read_to_string(&target_path).unwrap();
    assert!(content.contains("oversight:begin"));

    // Remove
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "remove",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed managed block"));

    let content = fs::read_to_string(&target_path).unwrap();
    assert!(!content.contains("oversight:begin"));
    assert!(content.contains("# My Config"));
}

#[test]
fn test_cli_integrate_remove_idempotent() {
    let (dir, kb_path) = setup_kb();
    let target_path = dir.path().join("CLAUDE.md");

    // Remove on nonexistent file
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "remove",
            "--path",
            target_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes needed"));
}

// --- Status tests ---

#[test]
fn test_cli_integrate_status() {
    let (_dir, kb_path) = setup_kb();

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "status",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Target: claude-code"));
}

// --- Unknown target ---

#[test]
fn test_cli_integrate_unknown_target() {
    let (_dir, kb_path) = setup_kb();

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "integrate",
            "install",
            "--target",
            "nonexistent-target",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown integration target"));
}
