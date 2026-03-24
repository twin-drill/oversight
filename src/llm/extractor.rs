use crate::error::{Error, Result};
use crate::healing_loop::policy::Regime;
use crate::llm::client::LlmClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single learning extracted from a conversation context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Learning {
    /// Hint for which KB topic this relates to (will be slugified).
    pub topic_hint: String,
    /// Title for the learning.
    pub title: String,
    /// Summary description of what was learned.
    pub summary: String,
    /// Evidence strings from the conversation.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Confidence score from the LLM (0.0 to 1.0).
    #[serde(default)]
    pub confidence: f64,
}

/// The full extraction response for one context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResponse {
    /// The context ID this extraction is for.
    pub context_id: u64,
    /// Extracted learnings.
    #[serde(default)]
    pub learnings: Vec<Learning>,
}

impl Learning {
    /// Compute a stable SHA-256 hash of this learning for deduplication.
    ///
    /// The hash is based on the topic_hint + title + summary (normalized).
    pub fn content_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.topic_hint.to_lowercase().as_bytes());
        hasher.update(b"|");
        hasher.update(self.title.to_lowercase().as_bytes());
        hasher.update(b"|");
        hasher.update(self.summary.to_lowercase().as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// The system prompt for the extraction LLM call.
const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are a knowledge extractor that finds environment-specific quirks and non-obvious fixes from conversations between a developer and an AI coding assistant.

You are looking for things a skilled developer would NOT already know — surprises, gotchas, and workarounds specific to THIS user's machine, project, or infrastructure.

Extract:
- Error-recovery sequences: a command failed due to a non-obvious reason, and the fix was specific to the user's setup (e.g., an env var that shadows auth, a port conflict, a version mismatch)
- Project-specific configuration: flags, env vars, config overrides, or file paths that are unique to this codebase or environment — not standard tool usage
- Workarounds for broken or surprising behavior: things that shouldn't be necessary but are (e.g., "must unset X before running Y", "need --legacy-peer-deps because of Z")
- Infrastructure quirks: CI pipeline ordering issues, deploy steps with undocumented dependencies, service startup ordering

Do NOT extract:
- How standard tools work (e.g., "cargo test runs tests", "git push pushes to remote")
- Generic best practices any developer would know
- The existence or basic usage of a tool (e.g., "Docker Compose is used for container orchestration")
- Descriptions of what a project is or how its code is structured
- Learnings about the AI assistant itself or its capabilities

Rules:
- Only extract concrete, actionable learnings that would save time if encountered again.
- Each learning must describe a SPECIFIC fix, workaround, or gotcha — not general knowledge.
- Set confidence between 0.0 and 1.0. Use 0.9+ only when the transcript shows a clear failure-then-fix sequence.
- The topic_hint should be a short kebab-case identifier for the tool or system involved (e.g., "gh-cli", "docker-compose", "aws-sso").
- Evidence should be brief excerpts or paraphrases, NOT raw tool output.
- If nothing in the transcript is surprising or environment-specific, return an empty learnings array. An empty array is the correct answer for most conversations.

You MUST respond with valid JSON matching this exact schema:
{
  "context_id": <number>,
  "learnings": [
    {
      "topic_hint": "<string>",
      "title": "<string>",
      "summary": "<string>",
      "evidence": ["<string>", ...],
      "tags": ["<string>", ...],
      "confidence": <number between 0.0 and 1.0>
    }
  ]
}

Respond ONLY with the JSON object. No markdown fences, no explanation."#;

/// Return the regime-specific prompt modifier paragraph, or None for Balanced.
pub fn regime_prompt_modifier(regime: &Regime) -> Option<&'static str> {
    match regime {
        Regime::Aggressive => Some(
            "Extract separate, fine-grained learnings. Prefer creating distinct entries for each specific tool behavior, flag, or workaround — even if they relate to the same tool."
        ),
        Regime::Balanced => None,
        Regime::Conservative => Some(
            "Consolidate related learnings into fewer, broader entries. Group tool-specific details under a single topic when they share a tool or workflow."
        ),
    }
}

/// Extract learnings from a transcript using the configured LLM provider.
///
/// The `regime` parameter influences the extraction prompt to encourage
/// fine-grained or consolidated learnings.
pub async fn extract_learnings(
    client: &LlmClient,
    context_id: u64,
    transcript: &str,
    regime: &Regime,
) -> Result<ExtractionResponse> {
    let user_prompt = format!(
        "Extract tool knowledge learnings from this conversation transcript (context ID: {context_id}):\n\n{transcript}"
    );

    let system_prompt = match regime_prompt_modifier(regime) {
        Some(modifier) => format!("{}\n\n{}", EXTRACTION_SYSTEM_PROMPT, modifier),
        None => EXTRACTION_SYSTEM_PROMPT.to_string(),
    };

    let response_text = client
        .complete(Some(&system_prompt), &user_prompt)
        .await?;

    parse_extraction_response(&response_text, context_id)
}

/// Parse the LLM's JSON response into an ExtractionResponse.
///
/// This is public so it can be tested with mock responses.
pub fn parse_extraction_response(
    response_text: &str,
    expected_context_id: u64,
) -> Result<ExtractionResponse> {
    // Try to find JSON in the response (handles cases where LLM adds markdown fences)
    let json_str = extract_json_from_text(response_text);

    let mut resp: ExtractionResponse = serde_json::from_str(json_str).map_err(|e| {
        Error::Extraction(format!(
            "Failed to parse extraction response as JSON: {e}\nResponse: {}",
            truncate_str(response_text, 500)
        ))
    })?;

    // Validate context_id matches
    if resp.context_id != expected_context_id {
        // Fix it rather than failing - the LLM sometimes gets this wrong
        resp.context_id = expected_context_id;
    }

    // Validate and sanitize learnings
    resp.learnings.retain(|l| {
        !l.topic_hint.is_empty()
            && !l.title.is_empty()
            && !l.summary.is_empty()
            && l.confidence >= 0.0
            && l.confidence <= 1.0
    });

    Ok(resp)
}

/// Filter learnings by confidence threshold.
pub fn filter_by_confidence(learnings: Vec<Learning>, threshold: f64) -> Vec<Learning> {
    learnings
        .into_iter()
        .filter(|l| l.confidence >= threshold)
        .collect()
}

/// Extract JSON from text that might have markdown fences or other wrapping.
fn extract_json_from_text(text: &str) -> &str {
    let trimmed = text.trim();

    // Check for ```json ... ``` wrapping
    if let Some(start) = trimmed.find("```json") {
        let after_fence = &trimmed[start + 7..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }

    // Check for ``` ... ``` wrapping
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }

    // Find first { and last }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }

    trimmed
}

/// Truncate a string for error messages.
fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_response() {
        let json = r#"{
            "context_id": 42,
            "learnings": [
                {
                    "topic_hint": "gh-cli",
                    "title": "Unset GITHUB_TOKEN before gh commands",
                    "summary": "The GITHUB_TOKEN env var overrides keychain auth.",
                    "evidence": ["gh auth failed with read-only token"],
                    "tags": ["cli", "github"],
                    "confidence": 0.92
                }
            ]
        }"#;

        let resp = parse_extraction_response(json, 42).unwrap();
        assert_eq!(resp.context_id, 42);
        assert_eq!(resp.learnings.len(), 1);
        assert_eq!(resp.learnings[0].topic_hint, "gh-cli");
        assert_eq!(resp.learnings[0].confidence, 0.92);
    }

    #[test]
    fn test_parse_with_markdown_fences() {
        let text = r#"Here's the extraction:

```json
{
    "context_id": 10,
    "learnings": []
}
```"#;

        let resp = parse_extraction_response(text, 10).unwrap();
        assert_eq!(resp.context_id, 10);
        assert!(resp.learnings.is_empty());
    }

    #[test]
    fn test_parse_filters_invalid_learnings() {
        let json = r#"{
            "context_id": 1,
            "learnings": [
                {
                    "topic_hint": "valid",
                    "title": "Valid learning",
                    "summary": "This is valid.",
                    "evidence": [],
                    "tags": [],
                    "confidence": 0.8
                },
                {
                    "topic_hint": "",
                    "title": "Invalid - empty topic hint",
                    "summary": "This should be filtered.",
                    "evidence": [],
                    "tags": [],
                    "confidence": 0.5
                },
                {
                    "topic_hint": "valid2",
                    "title": "Also valid",
                    "summary": "Another valid learning.",
                    "evidence": [],
                    "tags": [],
                    "confidence": 1.5
                }
            ]
        }"#;

        let resp = parse_extraction_response(json, 1).unwrap();
        assert_eq!(resp.learnings.len(), 1);
        assert_eq!(resp.learnings[0].topic_hint, "valid");
    }

    #[test]
    fn test_parse_fixes_wrong_context_id() {
        let json = r#"{"context_id": 999, "learnings": []}"#;
        let resp = parse_extraction_response(json, 42).unwrap();
        assert_eq!(resp.context_id, 42);
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = parse_extraction_response("not json at all", 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_by_confidence() {
        let learnings = vec![
            Learning {
                topic_hint: "a".into(),
                title: "A".into(),
                summary: "S".into(),
                evidence: vec![],
                tags: vec![],
                confidence: 0.9,
            },
            Learning {
                topic_hint: "b".into(),
                title: "B".into(),
                summary: "S".into(),
                evidence: vec![],
                tags: vec![],
                confidence: 0.3,
            },
            Learning {
                topic_hint: "c".into(),
                title: "C".into(),
                summary: "S".into(),
                evidence: vec![],
                tags: vec![],
                confidence: 0.7,
            },
        ];

        let filtered = filter_by_confidence(learnings, 0.7);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].topic_hint, "a");
        assert_eq!(filtered[1].topic_hint, "c");
    }

    #[test]
    fn test_learning_content_hash() {
        let l1 = Learning {
            topic_hint: "gh-cli".into(),
            title: "Test Title".into(),
            summary: "Test summary".into(),
            evidence: vec![],
            tags: vec![],
            confidence: 0.9,
        };

        let l2 = Learning {
            topic_hint: "GH-CLI".into(),
            title: "Test Title".into(),
            summary: "test summary".into(),
            evidence: vec!["different evidence".into()],
            tags: vec!["extra-tag".into()],
            confidence: 0.5,
        };

        // Same content (case-insensitive) should produce same hash
        assert_eq!(l1.content_hash(), l2.content_hash());

        // Different content should produce different hash
        let l3 = Learning {
            topic_hint: "docker".into(),
            title: "Different".into(),
            summary: "Different".into(),
            evidence: vec![],
            tags: vec![],
            confidence: 0.9,
        };
        assert_ne!(l1.content_hash(), l3.content_hash());
    }

    #[test]
    fn test_extract_json_from_text() {
        assert_eq!(
            extract_json_from_text(r#"```json
{"a": 1}
```"#),
            r#"{"a": 1}"#
        );

        assert_eq!(
            extract_json_from_text(r#"Some text {"a": 1} more text"#),
            r#"{"a": 1}"#
        );

        assert_eq!(extract_json_from_text(r#"{"a": 1}"#), r#"{"a": 1}"#);
    }

    #[test]
    fn test_regime_prompt_modifier_aggressive() {
        let modifier = regime_prompt_modifier(&Regime::Aggressive);
        assert!(modifier.is_some());
        let text = modifier.unwrap();
        assert!(text.contains("fine-grained"));
        assert!(text.contains("distinct entries"));
    }

    #[test]
    fn test_regime_prompt_modifier_balanced() {
        let modifier = regime_prompt_modifier(&Regime::Balanced);
        assert!(modifier.is_none());
    }

    #[test]
    fn test_regime_prompt_modifier_conservative() {
        let modifier = regime_prompt_modifier(&Regime::Conservative);
        assert!(modifier.is_some());
        let text = modifier.unwrap();
        assert!(text.contains("Consolidate"));
        assert!(text.contains("fewer"));
    }
}
