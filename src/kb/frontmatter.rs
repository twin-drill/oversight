use crate::error::{Error, Result};
use crate::kb::types::Topic;

const FRONTMATTER_DELIMITER: &str = "---";

/// Parse a topic file's content into a Topic struct.
///
/// Expects format:
/// ```text
/// ---
/// title: ...
/// slug: ...
/// ---
///
/// markdown body here
/// ```
pub fn parse(content: &str) -> Result<Topic> {
    let (yaml_str, body) = split_frontmatter(content)?;

    let mut topic: Topic = serde_yaml::from_str(&yaml_str).map_err(|e| {
        Error::InvalidFrontmatter(format!("Failed to parse YAML: {e}"))
    })?;

    // Validate required fields
    if topic.title.is_empty() {
        return Err(Error::MissingField("title".to_string()));
    }
    if topic.slug.is_empty() {
        return Err(Error::MissingField("slug".to_string()));
    }

    topic.body = body;
    Ok(topic)
}

/// Serialize a Topic back to a markdown file with YAML frontmatter.
pub fn serialize(topic: &Topic) -> Result<String> {
    let yaml = serde_yaml::to_string(topic)
        .map_err(|e| Error::InvalidFrontmatter(format!("Failed to serialize YAML: {e}")))?;

    // serde_yaml adds a trailing newline; trim it for consistent formatting
    let yaml = yaml.trim_end();

    let mut output = String::new();
    output.push_str(FRONTMATTER_DELIMITER);
    output.push('\n');
    output.push_str(yaml);
    output.push('\n');
    output.push_str(FRONTMATTER_DELIMITER);
    output.push('\n');

    if !topic.body.is_empty() {
        // Ensure there's a blank line between frontmatter and body
        if !topic.body.starts_with('\n') {
            output.push('\n');
        }
        output.push_str(&topic.body);
        // Ensure file ends with newline
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }

    Ok(output)
}

/// Split content into YAML frontmatter and markdown body.
fn split_frontmatter(content: &str) -> Result<(String, String)> {
    let content = content.trim_start();

    if !content.starts_with(FRONTMATTER_DELIMITER) {
        return Err(Error::InvalidFrontmatter(
            "File does not start with frontmatter delimiter (---)".to_string(),
        ));
    }

    // Find the second delimiter
    let after_first = &content[FRONTMATTER_DELIMITER.len()..];
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    let end_pos = after_first.find(&format!("\n{FRONTMATTER_DELIMITER}"));
    match end_pos {
        Some(pos) => {
            let yaml = &after_first[..pos];
            let rest = &after_first[pos + 1 + FRONTMATTER_DELIMITER.len()..];
            // Strip the leading newline(s) from the body
            let body = rest.strip_prefix('\n').unwrap_or(rest);
            // Trim leading blank line from body
            let body = body.strip_prefix('\n').unwrap_or(body);
            Ok((yaml.to_string(), body.to_string()))
        }
        None => Err(Error::InvalidFrontmatter(
            "Missing closing frontmatter delimiter (---)".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let content = r#"---
title: GitHub CLI
slug: gh-cli
---

# GitHub CLI

Some content here.
"#;
        let topic = parse(content).unwrap();
        assert_eq!(topic.title, "GitHub CLI");
        assert_eq!(topic.slug, "gh-cli");
        assert!(topic.body.contains("# GitHub CLI"));
        assert!(topic.body.contains("Some content here."));
    }

    #[test]
    fn test_parse_with_all_fields() {
        let content = r#"---
title: GitHub CLI
slug: gh-cli
aliases:
  - github cli
  - gh command
tags:
  - cli
  - git
source: manual
confidence: high
---

# GitHub CLI
"#;
        let topic = parse(content).unwrap();
        assert_eq!(topic.aliases, vec!["github cli", "gh command"]);
        assert_eq!(topic.tags, vec!["cli", "git"]);
        assert_eq!(topic.source.as_deref(), Some("manual"));
        assert_eq!(topic.confidence.as_deref(), Some("high"));
    }

    #[test]
    fn test_parse_missing_frontmatter() {
        let content = "# Just markdown\nNo frontmatter here.";
        let err = parse(content).unwrap_err();
        assert!(err.to_string().contains("frontmatter delimiter"));
    }

    #[test]
    fn test_parse_missing_title() {
        let content = "---\nslug: test\n---\n\nBody\n";
        // serde_yaml will fail because title is required
        let result = parse(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip() {
        let topic = Topic::new(
            "Test Topic".to_string(),
            "test-topic".to_string(),
            "# Test\n\nSome content.\n".to_string(),
        );
        let serialized = serialize(&topic).unwrap();
        let parsed = parse(&serialized).unwrap();
        assert_eq!(parsed.title, topic.title);
        assert_eq!(parsed.slug, topic.slug);
        assert!(parsed.body.contains("# Test"));
        assert!(parsed.body.contains("Some content."));
    }

    #[test]
    fn test_empty_body() {
        let content = "---\ntitle: Empty\nslug: empty\n---\n";
        let topic = parse(content).unwrap();
        assert_eq!(topic.title, "Empty");
        assert!(topic.body.is_empty() || topic.body.trim().is_empty());
    }
}
