use crate::error::{Error, Result};

/// Maximum allowed slug length.
const MAX_SLUG_LENGTH: usize = 128;

/// Normalize a human-readable name into a valid slug.
///
/// Rules:
/// - Lowercase
/// - Alphanumeric characters and hyphens only
/// - Spaces and underscores become hyphens
/// - Consecutive hyphens collapsed to one
/// - Leading/trailing hyphens removed
///
/// Examples:
/// - "GitHub CLI" -> "github-cli"
/// - "AWS SSO Login" -> "aws-sso-login"
/// - "docker--local" -> "docker-local"
pub fn normalize(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else {
                '-'
            }
        })
        .collect();

    // Collapse consecutive hyphens
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // Trim leading/trailing hyphens
    result.trim_matches('-').to_string()
}

/// Validate that a slug is well-formed.
///
/// A valid slug:
/// - Is non-empty
/// - Contains only lowercase alphanumeric chars and hyphens
/// - Does not contain path traversal characters (., /)
/// - Does not exceed MAX_SLUG_LENGTH
/// - Does not start or end with a hyphen
pub fn validate(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(Error::InvalidSlug("Slug cannot be empty".to_string()));
    }

    if slug.len() > MAX_SLUG_LENGTH {
        return Err(Error::InvalidSlug(format!(
            "Slug exceeds maximum length of {MAX_SLUG_LENGTH} characters"
        )));
    }

    if slug.contains('.') || slug.contains('/') || slug.contains('\\') {
        return Err(Error::InvalidSlug(
            "Slug cannot contain path traversal characters (., /, \\)".to_string(),
        ));
    }

    if slug.starts_with('-') || slug.ends_with('-') {
        return Err(Error::InvalidSlug(
            "Slug cannot start or end with a hyphen".to_string(),
        ));
    }

    for c in slug.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
            return Err(Error::InvalidSlug(format!(
                "Slug contains invalid character: '{c}'. Only lowercase alphanumeric and hyphens allowed."
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_basic() {
        assert_eq!(normalize("GitHub CLI"), "github-cli");
        assert_eq!(normalize("AWS SSO Login"), "aws-sso-login");
        assert_eq!(normalize("docker-local"), "docker-local");
    }

    #[test]
    fn test_normalize_special_chars() {
        assert_eq!(normalize("foo_bar"), "foo-bar");
        assert_eq!(normalize("foo  bar"), "foo-bar");
        assert_eq!(normalize("foo--bar"), "foo-bar");
        assert_eq!(normalize("  leading  "), "leading");
    }

    #[test]
    fn test_normalize_already_valid() {
        assert_eq!(normalize("simple"), "simple");
        assert_eq!(normalize("a-b-c"), "a-b-c");
    }

    #[test]
    fn test_validate_valid() {
        assert!(validate("gh-cli").is_ok());
        assert!(validate("a").is_ok());
        assert!(validate("test-123").is_ok());
    }

    #[test]
    fn test_validate_empty() {
        assert!(validate("").is_err());
    }

    #[test]
    fn test_validate_traversal() {
        assert!(validate("../etc/passwd").is_err());
        assert!(validate("foo/bar").is_err());
        assert!(validate("foo.bar").is_err());
    }

    #[test]
    fn test_validate_uppercase() {
        assert!(validate("UpperCase").is_err());
    }

    #[test]
    fn test_validate_leading_trailing_hyphen() {
        assert!(validate("-leading").is_err());
        assert!(validate("trailing-").is_err());
    }

    #[test]
    fn test_validate_too_long() {
        let long_slug = "a".repeat(MAX_SLUG_LENGTH + 1);
        assert!(validate(&long_slug).is_err());
    }
}
