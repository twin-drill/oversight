use crate::error::{Error, Result};
use crate::healing_loop::patterns::PatternCluster;
use crate::llm::client::LlmClient;
use crate::llm::extractor::Learning;
use serde::{Deserialize, Serialize};

/// A directive synthesized from a cluster of repeated user patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directive {
    pub topic_hint: String,
    pub title: String,
    pub summary: String,
    pub evidence: Vec<String>,
    pub tags: Vec<String>,
    pub confidence: f64,
    #[serde(default)]
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SynthesisResponse {
    directives: Vec<Directive>,
}

impl Directive {
    pub fn into_learning(self) -> Learning {
        Learning {
            topic_hint: self.topic_hint,
            title: self.title,
            summary: self.summary,
            evidence: self.evidence,
            tags: self.tags,
            confidence: self.confidence,
            project_path: self.projects.first().cloned(),
        }
    }
}

const SYNTHESIS_SYSTEM_PROMPT: &str = r#"You are a pattern synthesizer for an AI coding assistant knowledge base. You receive clusters of repeated user messages found across multiple conversations. Your job is to distill each cluster into a clear, actionable directive that can be stored in the assistant's knowledge base to prevent the user from having to repeat themselves.

For each cluster you will receive:
- The cluster type: "repeated-instruction" (user keeps telling the agent the same thing) or "repeated-correction" (user keeps correcting the same mistake)
- A representative message and all occurrences with their project context

For each cluster, produce a directive with:
- topic_hint: short kebab-case identifier for the tool/workflow involved (e.g., "build-system", "package-manager", "testing")
- title: concise imperative statement of what the agent should do/remember (e.g., "Always use pnpm instead of npm", "Run make gen before testing")
- summary: 1-3 sentences explaining the directive, why it matters, and any project-specific context
- evidence: 2-3 brief excerpts from the user's messages demonstrating the pattern
- tags: relevant categorization tags
- confidence: 0.85-0.95 for clear patterns, 0.7-0.85 for less certain ones
- projects: list of project paths where this was observed (empty if it's a global preference)

SECURITY — absolutely critical:
- NEVER include API keys, tokens, passwords, secrets, or credentials in any field.
- If evidence mentions secrets, paraphrase without the actual values.

PROJECT ATTRIBUTION:
- If the pattern is project-specific (only seen in one project), include the project path in projects[] and note it in the summary.
- If it's a general preference (seen across multiple projects), leave projects[] empty and describe it as a global preference.
- For project-specific details like pod names, service names, or internal URLs, always prefix them with the project name.

Rules:
- Each directive should be a STANDING INSTRUCTION the agent can follow in future conversations.
- Focus on the user's intent, not the exact wording — synthesize the pattern into a clear rule.
- If a cluster is too vague or just general conversation, return an empty directives array.
- Corrections are higher priority than repeated instructions — they indicate the agent is actively doing the wrong thing.

You MUST respond with valid JSON matching this exact schema:
{
  "directives": [
    {
      "topic_hint": "<string>",
      "title": "<string>",
      "summary": "<string>",
      "evidence": ["<string>", ...],
      "tags": ["<string>", ...],
      "confidence": <number>,
      "projects": ["<string>", ...]
    }
  ]
}

Respond ONLY with the JSON object. No markdown fences, no explanation."#;

/// Synthesize pattern clusters into directives using the LLM.
pub async fn synthesize_patterns(
    client: &LlmClient,
    clusters: &[PatternCluster],
    existing_tags: &[String],
) -> Result<Vec<Directive>> {
    if clusters.is_empty() {
        return Ok(Vec::new());
    }

    let user_prompt = build_synthesis_prompt(clusters);

    let mut system_prompt = SYNTHESIS_SYSTEM_PROMPT.to_string();

    if !existing_tags.is_empty() {
        system_prompt.push_str("\n\nReuse these existing tags when applicable (do not invent synonyms): ");
        system_prompt.push_str(&existing_tags.join(", "));
    }

    let response_text = client
        .complete(Some(&system_prompt), &user_prompt)
        .await?;

    parse_synthesis_response(&response_text)
}

fn build_synthesis_prompt(clusters: &[PatternCluster]) -> String {
    let mut prompt = String::from(
        "Synthesize the following repeated user patterns into standing directives:\n\n",
    );

    for (i, cluster) in clusters.iter().enumerate() {
        prompt.push_str(&format!("## Cluster {} ({})\n\n", i + 1, cluster.cluster_type));
        prompt.push_str(&format!(
            "Representative message: \"{}\"\n\n",
            cluster.representative
        ));
        prompt.push_str("Occurrences:\n");
        for occ in &cluster.occurrences {
            let project_tag = occ
                .project_path
                .as_deref()
                .map(|p| format!(" [project: {p}]"))
                .unwrap_or_default();
            prompt.push_str(&format!(
                "- (context {}{}) \"{}\"\n",
                occ.context_id, project_tag, occ.text
            ));
        }
        prompt.push('\n');
    }

    prompt
}

pub fn parse_synthesis_response(response_text: &str) -> Result<Vec<Directive>> {
    let json_str = extract_json_from_text(response_text);

    let resp: SynthesisResponse = serde_json::from_str(json_str).map_err(|e| {
        Error::Extraction(format!(
            "Failed to parse synthesis response as JSON: {e}\nResponse: {}",
            truncate_str(response_text, 500)
        ))
    })?;

    let directives = resp
        .directives
        .into_iter()
        .filter(|d| {
            !d.topic_hint.is_empty()
                && !d.title.is_empty()
                && !d.summary.is_empty()
                && d.confidence >= 0.0
                && d.confidence <= 1.0
        })
        .collect();

    Ok(directives)
}

fn extract_json_from_text(text: &str) -> &str {
    let trimmed = text.trim();

    if let Some(start) = trimmed.find("```json") {
        let after_fence = &trimmed[start + 7..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }

    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }

    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }

    trimmed
}

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
    use crate::healing_loop::patterns::{ClusterType, PatternOccurrence};

    #[test]
    fn test_parse_valid_synthesis() {
        let json = r#"{
            "directives": [
                {
                    "topic_hint": "package-manager",
                    "title": "Always use pnpm instead of npm",
                    "summary": "The user consistently prefers pnpm over npm.",
                    "evidence": ["no, use pnpm", "I said pnpm not npm"],
                    "tags": ["package-manager", "preference"],
                    "confidence": 0.9,
                    "projects": []
                }
            ]
        }"#;

        let directives = parse_synthesis_response(json).unwrap();
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].topic_hint, "package-manager");
        assert_eq!(directives[0].title, "Always use pnpm instead of npm");
    }

    #[test]
    fn test_parse_empty_directives() {
        let json = r#"{"directives": []}"#;
        let directives = parse_synthesis_response(json).unwrap();
        assert!(directives.is_empty());
    }

    #[test]
    fn test_parse_filters_invalid() {
        let json = r#"{
            "directives": [
                {
                    "topic_hint": "",
                    "title": "Missing hint",
                    "summary": "No hint.",
                    "evidence": [],
                    "tags": [],
                    "confidence": 0.9,
                    "projects": []
                },
                {
                    "topic_hint": "valid",
                    "title": "Valid directive",
                    "summary": "This is valid.",
                    "evidence": [],
                    "tags": [],
                    "confidence": 0.85,
                    "projects": []
                }
            ]
        }"#;

        let directives = parse_synthesis_response(json).unwrap();
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].topic_hint, "valid");
    }

    #[test]
    fn test_parse_with_markdown_fences() {
        let text = r#"```json
{"directives": []}
```"#;
        let directives = parse_synthesis_response(text).unwrap();
        assert!(directives.is_empty());
    }

    #[test]
    fn test_directive_into_learning() {
        let directive = Directive {
            topic_hint: "build-system".into(),
            title: "Run make gen before testing".into(),
            summary: "Always run make gen first.".into(),
            evidence: vec!["user said so".into()],
            tags: vec!["build".into()],
            confidence: 0.9,
            projects: vec!["/my/project".into()],
        };

        let learning = directive.into_learning();
        assert_eq!(learning.topic_hint, "build-system");
        assert_eq!(learning.project_path, Some("/my/project".into()));
    }

    #[test]
    fn test_build_synthesis_prompt() {
        let clusters = vec![PatternCluster {
            representative: "always use pnpm".into(),
            occurrences: vec![
                PatternOccurrence {
                    text: "use pnpm please".into(),
                    context_id: 1,
                    project_path: Some("/proj".into()),
                },
                PatternOccurrence {
                    text: "pnpm not npm".into(),
                    context_id: 2,
                    project_path: None,
                },
            ],
            cluster_type: ClusterType::RepeatedCorrection,
            content_hash: "abc".into(),
        }];

        let prompt = build_synthesis_prompt(&clusters);
        assert!(prompt.contains("Cluster 1"));
        assert!(prompt.contains("repeated-correction"));
        assert!(prompt.contains("always use pnpm"));
        assert!(prompt.contains("[project: /proj]"));
    }
}
