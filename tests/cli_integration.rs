use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn oversight_cmd() -> Command {
    Command::cargo_bin("oversight").unwrap()
}

#[test]
fn test_cli_init() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Knowledge base initialized"));

    assert!(kb_path.join("topics").exists());
    assert!(kb_path.join("index.json").exists());
}

#[test]
fn test_cli_topics_empty() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    // Init first
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    // List topics (empty)
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "topics"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No topics found"));
}

#[test]
fn test_cli_add_and_read() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    // Init
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    // Add topic
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "add",
            "Docker Local",
            "--tag",
            "docker",
            "--alias",
            "docker dev",
        ])
        .write_stdin("# Docker\n\nUse docker compose for local dev.\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Added topic: Docker Local (docker-local)"));

    // Read by slug
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "read", "docker-local"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docker compose"));

    // Read by alias
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "read", "docker dev"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docker compose"));
}

#[test]
fn test_cli_read_raw() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", "Test Raw"])
        .write_stdin("Body content here.\n")
        .assert()
        .success();

    // Read with --raw should include frontmatter
    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "read",
            "test-raw",
            "--raw",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("---"))
        .stdout(predicate::str::contains("title: Test Raw"))
        .stdout(predicate::str::contains("slug: test-raw"));
}

#[test]
fn test_cli_topics_list() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "add",
            "GitHub CLI",
            "--tag",
            "cli",
            "--alias",
            "gh command",
        ])
        .write_stdin("# GH CLI\n")
        .assert()
        .success();

    // Topics listing
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "topics"])
        .assert()
        .success()
        .stdout(predicate::str::contains("github-cli"))
        .stdout(predicate::str::contains("[gh command]"))
        .stdout(predicate::str::contains("#cli"));
}

#[test]
fn test_cli_topics_json() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", "JSON Test"])
        .write_stdin("Body\n")
        .assert()
        .success();

    // Topics JSON output
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "topics", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"slug\": \"json-test\""));
}

#[test]
fn test_cli_search() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args([
            "--kb-path",
            kb_path.to_str().unwrap(),
            "add",
            "Docker Local",
            "--tag",
            "docker",
        ])
        .write_stdin("# Docker\n\nUse docker compose.\n")
        .assert()
        .success();

    // Search
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "search", "docker"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docker-local"));
}

#[test]
fn test_cli_search_no_match() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "search", "nonexistent"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No topics found"));
}

#[test]
fn test_cli_delete() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", "Delete Me"])
        .write_stdin("Body\n")
        .assert()
        .success();

    // Verify it exists
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "topics"])
        .assert()
        .success()
        .stdout(predicate::str::contains("delete-me"));

    // Delete
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "delete", "delete-me"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted topic: delete-me"));

    // Verify empty
    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "topics"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No topics found"));
}

#[test]
fn test_cli_read_nonexistent_fails() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "read", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_cli_add_collision_fails() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", "Collision"])
        .write_stdin("Body 1\n")
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", "Collision"])
        .write_stdin("Body 2\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_cli_not_initialized_fails() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("nonexistent-kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "topics"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_cli_update() {
    let dir = TempDir::new().unwrap();
    let kb_path = dir.path().join("kb");

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "init"])
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "add", "Update Me"])
        .write_stdin("Original content.\n")
        .assert()
        .success();

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "update", "update-me"])
        .write_stdin("Updated content.\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated topic"));

    oversight_cmd()
        .args(["--kb-path", kb_path.to_str().unwrap(), "read", "update-me"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated content."));
}
