use oversight::{Config, KBService};
use tempfile::TempDir;

#[test]
fn test_init_creates_structure() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        kb_path: dir.path().to_path_buf(),
        ..Config::default()
    };
    let service = KBService::new(config);

    service.init().unwrap();

    // Topics directory exists
    assert!(dir.path().join("topics").exists());
    // Index file exists
    assert!(dir.path().join("index.json").exists());
}

#[test]
fn test_init_idempotent() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        kb_path: dir.path().to_path_buf(),
        ..Config::default()
    };
    let service = KBService::new(config);

    service.init().unwrap();
    // Running init again should not fail
    service.init().unwrap();

    assert!(dir.path().join("topics").exists());
    assert!(dir.path().join("index.json").exists());
}

#[test]
fn test_init_empty_index() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        kb_path: dir.path().to_path_buf(),
        ..Config::default()
    };
    let service = KBService::new(config);
    service.init().unwrap();

    let topics = service.list_topics().unwrap();
    assert!(topics.is_empty());
}

#[test]
fn test_operations_fail_without_init() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        kb_path: dir.path().join("nonexistent"),
        ..Config::default()
    };
    let service = KBService::new(config);

    let err = service.list_topics().unwrap_err();
    assert!(err.to_string().contains("not initialized"));

    let err = service.get_topic("anything").unwrap_err();
    assert!(err.to_string().contains("not initialized"));
}
