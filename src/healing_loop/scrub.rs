use regex::Regex;
use std::sync::LazyLock;

struct SecretPattern {
    regex: Regex,
    label: &'static str,
}

static SECRET_PATTERNS: LazyLock<Vec<SecretPattern>> = LazyLock::new(|| {
    vec![
        SecretPattern {
            regex: Regex::new(r"(?i)(AKIA[0-9A-Z]{16})").unwrap(),
            label: "[REDACTED:aws-access-key]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(aws[_\-]?secret[_\-]?access[_\-]?key\s*[=:]\s*)[^\s'"]+"##).unwrap(),
            label: "${1}[REDACTED:aws-secret]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(bearer\s+)[A-Za-z0-9\-._~+/]+=*"##).unwrap(),
            label: "${1}[REDACTED:bearer-token]",
        },
        SecretPattern {
            regex: Regex::new(r##"eyJ[A-Za-z0-9\-_]+\.eyJ[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+"##).unwrap(),
            label: "[REDACTED:jwt]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(anthropic[_\-]?api[_\-]?key\s*[=:]\s*)[^\s'"]+"##).unwrap(),
            label: "${1}[REDACTED:anthropic-key]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(openai[_\-]?api[_\-]?key\s*[=:]\s*)[^\s'"]+"##).unwrap(),
            label: "${1}[REDACTED:openai-key]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(gemini[_\-]?api[_\-]?key\s*[=:]\s*)[^\s'"]+"##).unwrap(),
            label: "${1}[REDACTED:gemini-key]",
        },
        SecretPattern {
            regex: Regex::new(r"sk-ant-[A-Za-z0-9\-]{20,}").unwrap(),
            label: "[REDACTED:anthropic-key]",
        },
        SecretPattern {
            regex: Regex::new(r"sk-[A-Za-z0-9]{20,}").unwrap(),
            label: "[REDACTED:api-key]",
        },
        SecretPattern {
            regex: Regex::new(r"ghp_[A-Za-z0-9]{36,}").unwrap(),
            label: "[REDACTED:github-pat]",
        },
        SecretPattern {
            regex: Regex::new(r"gho_[A-Za-z0-9]{36,}").unwrap(),
            label: "[REDACTED:github-oauth]",
        },
        SecretPattern {
            regex: Regex::new(r"github_pat_[A-Za-z0-9_]{22,}").unwrap(),
            label: "[REDACTED:github-pat]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(github[_\-]?token\s*[=:]\s*)[^\s'"]+"##).unwrap(),
            label: "${1}[REDACTED:github-token]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(password|passwd|pwd)\s*[=:]\s*[^\s'"]+"##).unwrap(),
            label: "[REDACTED:password]",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(mongodb(?:\+srv)?://)[^\s@]+@").unwrap(),
            label: "${1}[REDACTED:credentials]@",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(postgres(?:ql)?://)[^\s@]+@").unwrap(),
            label: "${1}[REDACTED:credentials]@",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(mysql://)[^\s@]+@").unwrap(),
            label: "${1}[REDACTED:credentials]@",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(redis://)[^\s@]+@").unwrap(),
            label: "${1}[REDACTED:credentials]@",
        },
        SecretPattern {
            regex: Regex::new(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----").unwrap(),
            label: "[REDACTED:private-key]",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(api[_\-]?key\s*[=:]\s*)[A-Za-z0-9\-._]{20,}").unwrap(),
            label: "${1}[REDACTED:api-key]",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(secret[_\-]?key\s*[=:]\s*)[A-Za-z0-9\-._]{20,}").unwrap(),
            label: "${1}[REDACTED:secret-key]",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)(access[_\-]?token\s*[=:]\s*)[A-Za-z0-9\-._]{20,}").unwrap(),
            label: "${1}[REDACTED:access-token]",
        },
        SecretPattern {
            regex: Regex::new(r"xox[bpoas]-[A-Za-z0-9\-]{10,}").unwrap(),
            label: "[REDACTED:slack-token]",
        },
        SecretPattern {
            regex: Regex::new(r##"(?i)(npm[_\-]?token\s*[=:]\s*)[^\s'"]+"##).unwrap(),
            label: "${1}[REDACTED:npm-token]",
        },
    ]
});

/// Scrub sensitive secrets from transcript text.
///
/// Applies a series of regex patterns to detect and replace API keys, tokens,
/// passwords, connection strings, and other credentials with redaction markers.
pub fn scrub_secrets(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in SECRET_PATTERNS.iter() {
        result = pattern.regex.replace_all(&result, pattern.label).to_string();
    }
    result
}

/// Returns true if the text contains any detected secrets.
pub fn contains_secrets(text: &str) -> bool {
    SECRET_PATTERNS.iter().any(|p| p.regex.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrub_aws_access_key() {
        let input = "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:aws-access-key]"));
        assert!(!scrubbed.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn test_scrub_aws_secret_key() {
        let input = "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:aws-secret]"), "got: {scrubbed}");
        assert!(!scrubbed.contains("wJalrXUtnFEMI"));
    }

    #[test]
    fn test_scrub_bearer_token() {
        let input = "Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.abc.def";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("eyJhbGciOiJSUzI1NiI"));
    }

    #[test]
    fn test_scrub_github_pat() {
        let input = "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("ghp_"));
    }

    #[test]
    fn test_scrub_sk_key() {
        let input = "OPENAI_API_KEY=sk-proj-abc123def456ghi789jklmnopqrst";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("sk-proj-abc123"));
    }

    #[test]
    fn test_scrub_password() {
        let input = "password=SuperSecret123!";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:password]"), "got: {scrubbed}");
        assert!(!scrubbed.contains("SuperSecret123"));
    }

    #[test]
    fn test_scrub_postgres_connection_string() {
        let input = "DATABASE_URL=postgresql://user:password@localhost:5432/mydb";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:credentials]@"));
        assert!(!scrubbed.contains("user:password@"));
    }

    #[test]
    fn test_scrub_mongodb_connection_string() {
        let input = "MONGO_URI=mongodb+srv://admin:s3cret@cluster.mongodb.net/db";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:credentials]@"));
        assert!(!scrubbed.contains("admin:s3cret@"));
    }

    #[test]
    fn test_scrub_private_key() {
        let input = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpA...\n-----END RSA PRIVATE KEY-----";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:private-key]"));
    }

    #[test]
    fn test_scrub_slack_token() {
        let input = "SLACK_TOKEN=xoxb-123456789012-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("xoxb-"));
    }

    #[test]
    fn test_no_scrub_normal_text() {
        let input = "Running cargo test in the project directory. Found 42 passing tests.";
        let scrubbed = scrub_secrets(input);
        assert_eq!(input, scrubbed);
    }

    #[test]
    fn test_contains_secrets_positive() {
        assert!(contains_secrets("key=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop"));
    }

    #[test]
    fn test_contains_secrets_negative() {
        assert!(!contains_secrets("just normal conversation text"));
    }

    #[test]
    fn test_scrub_jwt() {
        let input = "token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let scrubbed = scrub_secrets(input);
        assert!(scrubbed.contains("[REDACTED:jwt]"));
        assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiI"));
    }

    #[test]
    fn test_scrub_anthropic_key() {
        let input = "ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("sk-ant-api03"));
    }

    #[test]
    fn test_scrub_multiple_secrets() {
        let input = "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI and GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("wJalrXUtnFEMI"), "got: {scrubbed}");
        assert!(!scrubbed.contains("ghp_"), "got: {scrubbed}");
    }
}
