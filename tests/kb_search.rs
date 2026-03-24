use oversight::{Config, KBService};
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

fn populate(service: &KBService) {
    service
        .add_topic(
            "GitHub CLI",
            "# GitHub CLI\n\nRun `unset GITHUB_TOKEN` before any `gh` command.",
            vec!["cli".into(), "git".into(), "github".into()],
            vec!["gh command".into()],
        )
        .unwrap();

    service
        .add_topic(
            "AWS SSO Login",
            "# AWS SSO\n\nSSO login with `aws sso login`.",
            vec!["cli".into(), "aws".into()],
            vec!["aws login".into()],
        )
        .unwrap();

    service
        .add_topic(
            "Docker Local Testing",
            "# Docker\n\nUse `docker compose` for local dev.",
            vec!["docker".into(), "testing".into()],
            vec!["docker local".into()],
        )
        .unwrap();
}

#[test]
fn test_search_by_slug_match() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("docker").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].topic.slug, "docker-local-testing");
}

#[test]
fn test_search_by_tag() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("aws").unwrap();
    assert!(!results.is_empty());

    let slugs: Vec<&str> = results.iter().map(|r| r.topic.slug.as_str()).collect();
    assert!(slugs.contains(&"aws-sso-login"));
}

#[test]
fn test_search_by_body_content() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("GITHUB_TOKEN").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].topic.slug, "github-cli");
}

#[test]
fn test_search_no_results() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("kubernetes").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_case_insensitive() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("github").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].topic.slug, "github-cli");
}

#[test]
fn test_search_multi_word() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("docker compose").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].topic.slug, "docker-local-testing");
}

#[test]
fn test_search_returns_ranked_results() {
    let (_dir, service) = setup();
    populate(&service);

    // "cli" matches github-cli (slug, title, tag) and aws-sso-login (tag only)
    let results = service.search_topics("cli").unwrap();
    assert!(results.len() >= 2);

    // github-cli should rank higher because "cli" appears in slug and title
    assert_eq!(results[0].topic.slug, "github-cli");
}

#[test]
fn test_search_empty_query() {
    let (_dir, service) = setup();
    populate(&service);

    let results = service.search_topics("").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_empty_kb() {
    let (_dir, service) = setup();

    let results = service.search_topics("anything").unwrap();
    assert!(results.is_empty());
}
