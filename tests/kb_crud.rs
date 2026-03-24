use oversight::{Config, KBService, Topic};
use tempfile::TempDir;

fn setup() -> (TempDir, KBService) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        kb_path: dir.path().to_path_buf(),
        ..Config::default()
    };
    let service = KBService::new(config);
    service.init().unwrap();
    (dir, service)
}

#[test]
fn test_add_topic() {
    let (_dir, service) = setup();

    let topic = service
        .add_topic(
            "GitHub CLI",
            "# GitHub CLI\n\nUse gh for commands.",
            vec!["cli".into(), "git".into()],
            vec!["gh command".into()],
        )
        .unwrap();

    assert_eq!(topic.slug, "github-cli");
    assert_eq!(topic.title, "GitHub CLI");
    assert_eq!(topic.tags, vec!["cli", "git"]);
    assert_eq!(topic.aliases, vec!["gh command"]);
}

#[test]
fn test_add_and_read_by_slug() {
    let (_dir, service) = setup();

    service
        .add_topic("Docker Local", "# Docker\n\nUse docker compose.", vec![], vec![])
        .unwrap();

    let topic = service.get_topic("docker-local").unwrap();
    assert_eq!(topic.title, "Docker Local");
    assert!(topic.body.contains("docker compose"));
}

#[test]
fn test_add_and_read_by_alias() {
    let (_dir, service) = setup();

    service
        .add_topic(
            "AWS SSO Login",
            "# AWS SSO\n\nLogin with SSO.",
            vec!["aws".into()],
            vec!["aws login".into()],
        )
        .unwrap();

    let topic = service.get_topic("aws login").unwrap();
    assert_eq!(topic.slug, "aws-sso-login");
}

#[test]
fn test_slug_collision_rejected() {
    let (_dir, service) = setup();

    service
        .add_topic("My Topic", "Body 1", vec![], vec![])
        .unwrap();

    let err = service
        .add_topic("My Topic", "Body 2", vec![], vec![])
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn test_alias_collision_rejected() {
    let (_dir, service) = setup();

    service
        .add_topic("Topic A", "Body", vec![], vec!["shared alias".into()])
        .unwrap();

    let err = service
        .add_topic("Topic B", "Body", vec![], vec!["shared alias".into()])
        .unwrap_err();
    assert!(err.to_string().contains("collides"));
}

#[test]
fn test_alias_collides_with_slug() {
    let (_dir, service) = setup();

    service
        .add_topic("Existing Topic", "Body", vec![], vec![])
        .unwrap();

    // Try to add an alias that normalizes to "existing-topic" (which is already a slug)
    let err = service
        .add_topic("Other Topic", "Body", vec![], vec!["existing topic".into()])
        .unwrap_err();
    assert!(err.to_string().contains("collides"));
}

#[test]
fn test_update_topic() {
    let (_dir, service) = setup();

    service
        .add_topic("Updatable", "Original content.", vec![], vec![])
        .unwrap();

    let updated = service
        .update_topic("updatable", "Updated content.")
        .unwrap();

    assert!(updated.body.contains("Updated content."));
    assert_eq!(updated.title, "Updatable");
}

#[test]
fn test_delete_topic() {
    let (_dir, service) = setup();

    service
        .add_topic("Delete Me", "Body", vec![], vec![])
        .unwrap();

    let list = service.list_topics().unwrap();
    assert_eq!(list.len(), 1);

    service.delete_topic("delete-me").unwrap();

    let list = service.list_topics().unwrap();
    assert!(list.is_empty());
}

#[test]
fn test_delete_nonexistent_topic() {
    let (_dir, service) = setup();

    let err = service.delete_topic("nonexistent").unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn test_list_topics() {
    let (_dir, service) = setup();

    service.add_topic("Alpha", "Body", vec!["a".into()], vec![]).unwrap();
    service.add_topic("Beta", "Body", vec!["b".into()], vec![]).unwrap();
    service.add_topic("Gamma", "Body", vec!["g".into()], vec![]).unwrap();

    let topics = service.list_topics().unwrap();
    assert_eq!(topics.len(), 3);

    // Should be sorted by slug
    assert_eq!(topics[0].slug, "alpha");
    assert_eq!(topics[1].slug, "beta");
    assert_eq!(topics[2].slug, "gamma");
}

#[test]
fn test_index_regenerates_after_mutations() {
    let (dir, service) = setup();

    service.add_topic("First", "Body", vec![], vec![]).unwrap();

    // Verify index exists and has one topic
    let index_path = dir.path().join("index.json");
    assert!(index_path.exists());

    let index_content = std::fs::read_to_string(&index_path).unwrap();
    let index: serde_json::Value = serde_json::from_str(&index_content).unwrap();
    assert_eq!(index["topics"].as_array().unwrap().len(), 1);

    // Add another
    service.add_topic("Second", "Body", vec![], vec![]).unwrap();

    let index_content = std::fs::read_to_string(&index_path).unwrap();
    let index: serde_json::Value = serde_json::from_str(&index_content).unwrap();
    assert_eq!(index["topics"].as_array().unwrap().len(), 2);

    // Delete one
    service.delete_topic("first").unwrap();

    let index_content = std::fs::read_to_string(&index_path).unwrap();
    let index: serde_json::Value = serde_json::from_str(&index_content).unwrap();
    assert_eq!(index["topics"].as_array().unwrap().len(), 1);
}

#[test]
fn test_upsert_new_topic() {
    let (_dir, service) = setup();

    let topic = Topic::new("Upsert New".to_string(), "upsert-new".to_string(), "Body".to_string());
    service.upsert_topic(&topic).unwrap();

    let retrieved = service.get_topic("upsert-new").unwrap();
    assert_eq!(retrieved.title, "Upsert New");
}

#[test]
fn test_upsert_existing_topic() {
    let (_dir, service) = setup();

    service.add_topic("Upsert Me", "V1", vec![], vec![]).unwrap();

    let mut topic = service.get_topic("upsert-me").unwrap();
    topic.body = "V2".to_string();
    service.upsert_topic(&topic).unwrap();

    let retrieved = service.get_topic("upsert-me").unwrap();
    assert!(retrieved.body.contains("V2"));
}

#[test]
fn test_topic_roundtrip_preserves_metadata() {
    let (_dir, service) = setup();

    service
        .add_topic(
            "Roundtrip Test",
            "# Roundtrip\n\nBody content here.\n",
            vec!["tag1".into(), "tag2".into()],
            vec!["alias one".into(), "alias two".into()],
        )
        .unwrap();

    let topic = service.get_topic("roundtrip-test").unwrap();
    assert_eq!(topic.title, "Roundtrip Test");
    assert_eq!(topic.slug, "roundtrip-test");
    assert_eq!(topic.tags, vec!["tag1", "tag2"]);
    assert_eq!(topic.aliases, vec!["alias one", "alias two"]);
    assert!(topic.body.contains("Body content here."));
    assert!(topic.source.is_some());
    assert!(topic.created.is_some());
    assert!(topic.updated.is_some());
}
