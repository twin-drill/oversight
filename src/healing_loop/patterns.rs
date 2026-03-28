use crate::healing_loop::scrub;
use crate::source::types::{TurnType, TypedTurn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Configuration for cross-conversation pattern detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternConfig {
    /// Minimum token-Jaccard similarity to consider two messages "similar" (0.0-1.0).
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f64,
    /// Minimum number of occurrences across conversations to form a cluster.
    #[serde(default = "default_min_occurrences")]
    pub min_occurrences: usize,
    /// Maximum number of pattern clusters to send to LLM per pass.
    #[serde(default = "default_max_clusters_per_pass")]
    pub max_clusters_per_pass: usize,
    /// Whether pattern detection is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_similarity_threshold() -> f64 {
    0.4
}
fn default_min_occurrences() -> usize {
    3
}
fn default_max_clusters_per_pass() -> usize {
    5
}
fn default_enabled() -> bool {
    true
}

impl Default for PatternConfig {
    fn default() -> Self {
        PatternConfig {
            similarity_threshold: default_similarity_threshold(),
            min_occurrences: default_min_occurrences(),
            max_clusters_per_pass: default_max_clusters_per_pass(),
            enabled: default_enabled(),
        }
    }
}

/// A user message extracted from a conversation, with context metadata.
#[derive(Debug, Clone)]
pub struct UserMessage {
    pub text: String,
    pub context_id: u64,
    pub project_path: Option<String>,
    tokens: Vec<String>,
}

impl UserMessage {
    pub fn new(text: String, context_id: u64, project_path: Option<String>) -> Self {
        let tokens = tokenize(&text);
        UserMessage {
            text,
            context_id,
            project_path,
            tokens,
        }
    }
}

/// A correction sequence: user says X, agent does something, user corrects.
#[derive(Debug, Clone)]
pub struct CorrectionSequence {
    pub user_correction: String,
    pub agent_action: String,
    pub context_id: u64,
    pub project_path: Option<String>,
}

/// A cluster of similar user messages found across conversations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternCluster {
    pub representative: String,
    pub occurrences: Vec<PatternOccurrence>,
    pub cluster_type: ClusterType,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternOccurrence {
    pub text: String,
    pub context_id: u64,
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClusterType {
    RepeatedInstruction,
    RepeatedCorrection,
}

impl std::fmt::Display for ClusterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClusterType::RepeatedInstruction => write!(f, "repeated-instruction"),
            ClusterType::RepeatedCorrection => write!(f, "repeated-correction"),
        }
    }
}

impl PatternCluster {
    fn compute_hash(representative: &str, cluster_type: &ClusterType) -> String {
        let mut hasher = Sha256::new();
        hasher.update(representative.to_lowercase().as_bytes());
        hasher.update(b"|");
        hasher.update(cluster_type.to_string().as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Extract user messages from turns for a given context.
pub fn extract_user_messages(
    turns: &[TypedTurn],
    context_id: u64,
    project_path: Option<String>,
) -> Vec<UserMessage> {
    turns
        .iter()
        .filter(|t| t.turn_type() == TurnType::UserInput)
        .map(|t| {
            let text = scrub::scrub_secrets(&t.text_content());
            UserMessage::new(text, context_id, project_path.clone())
        })
        .filter(|m| !m.text.trim().is_empty() && m.tokens.len() >= 3)
        .collect()
}

/// Detect correction sequences: user says something → agent responds → user corrects.
pub fn detect_corrections(
    turns: &[TypedTurn],
    context_id: u64,
    project_path: Option<String>,
) -> Vec<CorrectionSequence> {
    let correction_keywords = [
        "no,", "no.", "not that", "i said", "i told you", "i meant",
        "wrong", "that's not", "don't do that", "stop", "undo",
        "actually,", "instead,", "please don't", "i already said",
        "as i mentioned", "like i said", "again,", "remember to",
        "always ", "never ", "you forgot", "you missed", "you need to",
    ];

    let mut corrections = Vec::new();
    let mut i = 0;

    while i < turns.len() {
        let turn = &turns[i];
        if turn.turn_type() == TurnType::UserInput {
            let text = turn.text_content().to_lowercase();
            let is_correction = correction_keywords.iter().any(|kw| text.contains(kw));
            if is_correction {
                let agent_action = find_preceding_agent_action(turns, i);
                let correction_text = scrub::scrub_secrets(&turn.text_content());
                corrections.push(CorrectionSequence {
                    user_correction: correction_text,
                    agent_action: agent_action.unwrap_or_default(),
                    context_id,
                    project_path: project_path.clone(),
                });
            }
        }
        i += 1;
    }

    corrections
}

fn find_preceding_agent_action(turns: &[TypedTurn], current_idx: usize) -> Option<String> {
    if current_idx == 0 {
        return None;
    }
    for i in (0..current_idx).rev() {
        let t = &turns[i];
        match t.turn_type() {
            TurnType::Assistant | TurnType::AssistantTurn => {
                let text = t.text_content();
                if !text.trim().is_empty() {
                    let truncated = if text.len() > 200 {
                        format!("{}...", &text[..200])
                    } else {
                        text
                    };
                    return Some(scrub::scrub_secrets(&truncated));
                }
            }
            TurnType::ToolCall => {
                let text = t.text_content();
                if !text.trim().is_empty() {
                    let truncated = if text.len() > 200 {
                        format!("{}...", &text[..200])
                    } else {
                        text
                    };
                    return Some(scrub::scrub_secrets(&truncated));
                }
            }
            _ => continue,
        }
    }
    None
}

/// Detect cross-conversation patterns from collected user messages.
///
/// Uses greedy single-linkage clustering based on token-Jaccard similarity.
pub fn detect_patterns(
    messages: &[UserMessage],
    corrections: &[CorrectionSequence],
    config: &PatternConfig,
    known_hashes: &HashSet<String>,
) -> Vec<PatternCluster> {
    let mut clusters = Vec::new();

    // Stage 1: Cluster repeated instructions
    let instruction_clusters =
        cluster_by_similarity(messages, config.similarity_threshold, config.min_occurrences);
    for (representative_idx, member_indices) in instruction_clusters {
        let representative = &messages[representative_idx];
        let occurrences: Vec<PatternOccurrence> = member_indices
            .iter()
            .map(|&idx| PatternOccurrence {
                text: messages[idx].text.clone(),
                context_id: messages[idx].context_id,
                project_path: messages[idx].project_path.clone(),
            })
            .collect();

        let distinct_contexts: HashSet<u64> = occurrences.iter().map(|o| o.context_id).collect();
        if distinct_contexts.len() < config.min_occurrences {
            continue;
        }

        let hash = PatternCluster::compute_hash(
            &representative.text,
            &ClusterType::RepeatedInstruction,
        );
        if known_hashes.contains(&hash) {
            continue;
        }

        clusters.push(PatternCluster {
            representative: representative.text.clone(),
            occurrences,
            cluster_type: ClusterType::RepeatedInstruction,
            content_hash: hash,
        });
    }

    // Stage 2: Cluster repeated corrections
    let correction_messages: Vec<UserMessage> = corrections
        .iter()
        .map(|c| UserMessage::new(c.user_correction.clone(), c.context_id, c.project_path.clone()))
        .collect();

    let correction_clusters = cluster_by_similarity(
        &correction_messages,
        config.similarity_threshold,
        config.min_occurrences.max(2),
    );
    for (representative_idx, member_indices) in correction_clusters {
        let representative = &correction_messages[representative_idx];
        let occurrences: Vec<PatternOccurrence> = member_indices
            .iter()
            .map(|&idx| PatternOccurrence {
                text: corrections[idx].user_correction.clone(),
                context_id: corrections[idx].context_id,
                project_path: corrections[idx].project_path.clone(),
            })
            .collect();

        let distinct_contexts: HashSet<u64> = occurrences.iter().map(|o| o.context_id).collect();
        if distinct_contexts.len() < 2 {
            continue;
        }

        let hash = PatternCluster::compute_hash(
            &representative.text,
            &ClusterType::RepeatedCorrection,
        );
        if known_hashes.contains(&hash) {
            continue;
        }

        clusters.push(PatternCluster {
            representative: representative.text.clone(),
            occurrences,
            cluster_type: ClusterType::RepeatedCorrection,
            content_hash: hash,
        });
    }

    // Sort by number of occurrences (most frequent first), then truncate
    clusters.sort_by(|a, b| b.occurrences.len().cmp(&a.occurrences.len()));
    clusters.truncate(config.max_clusters_per_pass);

    clusters
}

/// Greedy single-linkage clustering by token-Jaccard similarity.
///
/// Returns Vec<(representative_index, Vec<member_indices>)>.
fn cluster_by_similarity(
    messages: &[UserMessage],
    threshold: f64,
    min_size: usize,
) -> Vec<(usize, Vec<usize>)> {
    if messages.is_empty() {
        return Vec::new();
    }

    let mut assigned: Vec<bool> = vec![false; messages.len()];
    let mut clusters: Vec<(usize, Vec<usize>)> = Vec::new();

    for i in 0..messages.len() {
        if assigned[i] {
            continue;
        }

        let mut members = vec![i];
        assigned[i] = true;

        for j in (i + 1)..messages.len() {
            if assigned[j] {
                continue;
            }

            let sim = token_jaccard(&messages[i].tokens, &messages[j].tokens);
            if sim >= threshold {
                members.push(j);
                assigned[j] = true;
            }
        }

        if members.len() >= min_size {
            clusters.push((i, members));
        }
    }

    clusters
}

/// Compute Jaccard similarity between two token sets.
fn token_jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Tokenize text: lowercase, split on whitespace and punctuation, filter short tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect()
}

/// Summary of pattern detection results.
#[derive(Debug)]
pub struct PatternResult {
    pub messages_scanned: usize,
    pub corrections_found: usize,
    pub clusters_detected: usize,
    pub clusters_synthesized: usize,
    pub topics_created: u32,
    pub topics_updated: u32,
    pub duplicates_skipped: u32,
}

impl PatternResult {
    pub fn empty() -> Self {
        PatternResult {
            messages_scanned: 0,
            corrections_found: 0,
            clusters_detected: 0,
            clusters_synthesized: 0,
            topics_created: 0,
            topics_updated: 0,
            duplicates_skipped: 0,
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "Patterns: {} messages scanned, {} corrections, {} clusters → {} synthesized ({} created, {} updated, {} skipped)",
            self.messages_scanned,
            self.corrections_found,
            self.clusters_detected,
            self.clusters_synthesized,
            self.topics_created,
            self.topics_updated,
            self.duplicates_skipped,
        )
    }
}

/// Collect context_ids from pattern occurrences as a map of context_id → max turn
/// (used for provenance tracking).
pub fn context_ids_from_clusters(clusters: &[PatternCluster]) -> HashMap<u64, u64> {
    let mut map = HashMap::new();
    for cluster in clusters {
        for occ in &cluster.occurrences {
            map.entry(occ.context_id).or_insert(0);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("always run make gen before testing");
        assert!(tokens.contains(&"always".to_string()));
        assert!(tokens.contains(&"run".to_string()));
        assert!(tokens.contains(&"make".to_string()));
        assert!(tokens.contains(&"gen".to_string()));
        assert!(tokens.contains(&"before".to_string()));
        assert!(tokens.contains(&"testing".to_string()));
    }

    #[test]
    fn test_tokenize_filters_short() {
        let tokens = tokenize("do it now or go");
        assert!(!tokens.contains(&"do".to_string()));
        assert!(!tokens.contains(&"it".to_string()));
        assert!(!tokens.contains(&"or".to_string()));
        assert!(!tokens.contains(&"go".to_string()));
        assert!(tokens.contains(&"now".to_string()));
    }

    #[test]
    fn test_token_jaccard_identical() {
        let a = tokenize("always run make gen");
        let b = tokenize("always run make gen");
        assert!((token_jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_token_jaccard_similar() {
        let a = tokenize("always run make gen before testing");
        let b = tokenize("remember to run make gen before tests");
        let sim = token_jaccard(&a, &b);
        assert!(sim > 0.3);
    }

    #[test]
    fn test_token_jaccard_disjoint() {
        let a = tokenize("deploy kubernetes cluster staging");
        let b = tokenize("format python code linter");
        let sim = token_jaccard(&a, &b);
        assert!(sim < 0.1, "sim was {sim}");
    }

    #[test]
    fn test_cluster_by_similarity() {
        let messages = vec![
            UserMessage::new("always run make gen before testing".into(), 1, None),
            UserMessage::new("remember to run make gen before tests".into(), 2, None),
            UserMessage::new("make sure to run make gen first".into(), 3, None),
            UserMessage::new("deploy the kubernetes cluster to staging".into(), 4, None),
        ];

        let clusters = cluster_by_similarity(&messages, 0.3, 2);
        assert!(!clusters.is_empty());

        let (_, members) = &clusters[0];
        assert!(members.len() >= 2);
        assert!(!members.contains(&3));
    }

    #[test]
    fn test_detect_patterns_min_occurrences() {
        let messages = vec![
            UserMessage::new("run make gen before testing".into(), 1, None),
            UserMessage::new("run make gen before tests".into(), 2, None),
        ];

        let config = PatternConfig {
            similarity_threshold: 0.3,
            min_occurrences: 3,
            ..PatternConfig::default()
        };

        let clusters = detect_patterns(&messages, &[], &config, &HashSet::new());
        assert!(clusters.is_empty());
    }

    #[test]
    fn test_detect_patterns_meets_threshold() {
        let messages = vec![
            UserMessage::new("always run make gen before testing".into(), 1, None),
            UserMessage::new("remember to run make gen before tests".into(), 2, None),
            UserMessage::new("make sure to run make gen first".into(), 3, None),
        ];

        let config = PatternConfig {
            similarity_threshold: 0.3,
            min_occurrences: 2,
            ..PatternConfig::default()
        };

        let clusters = detect_patterns(&messages, &[], &config, &HashSet::new());
        assert!(!clusters.is_empty());
        assert_eq!(clusters[0].cluster_type, ClusterType::RepeatedInstruction);
    }

    #[test]
    fn test_detect_patterns_skips_known_hashes() {
        let messages = vec![
            UserMessage::new("always run make gen before testing".into(), 1, None),
            UserMessage::new("remember to run make gen before tests".into(), 2, None),
            UserMessage::new("make sure to run make gen first".into(), 3, None),
        ];

        let config = PatternConfig {
            similarity_threshold: 0.3,
            min_occurrences: 2,
            ..PatternConfig::default()
        };

        let first_run = detect_patterns(&messages, &[], &config, &HashSet::new());
        assert!(!first_run.is_empty());

        let known: HashSet<String> = first_run.iter().map(|c| c.content_hash.clone()).collect();
        let second_run = detect_patterns(&messages, &[], &config, &known);
        assert!(second_run.is_empty());
    }

    #[test]
    fn test_correction_detection() {
        let turns = vec![
            make_user_turn("fix the build error"),
            make_assistant_turn("I'll run cargo build"),
            make_user_turn("no, I told you to use make build instead"),
        ];

        let corrections = detect_corrections(&turns, 1, Some("/my/project".into()));
        assert_eq!(corrections.len(), 1);
        assert!(corrections[0].user_correction.contains("make build"));
        assert_eq!(corrections[0].project_path, Some("/my/project".into()));
    }

    #[test]
    fn test_correction_cluster() {
        let corrections = vec![
            CorrectionSequence {
                user_correction: "no, always use pnpm instead of npm".into(),
                agent_action: "npm install".into(),
                context_id: 1,
                project_path: None,
            },
            CorrectionSequence {
                user_correction: "I said use pnpm not npm".into(),
                agent_action: "npm run build".into(),
                context_id: 2,
                project_path: None,
            },
            CorrectionSequence {
                user_correction: "please use pnpm instead of npm".into(),
                agent_action: "npm test".into(),
                context_id: 3,
                project_path: None,
            },
        ];

        let config = PatternConfig {
            similarity_threshold: 0.3,
            min_occurrences: 2,
            ..PatternConfig::default()
        };

        let clusters = detect_patterns(&[], &corrections, &config, &HashSet::new());
        assert!(!clusters.is_empty());
        assert_eq!(clusters[0].cluster_type, ClusterType::RepeatedCorrection);
    }

    #[test]
    fn test_pattern_cluster_hash_stable() {
        let h1 = PatternCluster::compute_hash("run make gen", &ClusterType::RepeatedInstruction);
        let h2 = PatternCluster::compute_hash("run make gen", &ClusterType::RepeatedInstruction);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_pattern_cluster_hash_differs_by_type() {
        let h1 = PatternCluster::compute_hash("run make gen", &ClusterType::RepeatedInstruction);
        let h2 = PatternCluster::compute_hash("run make gen", &ClusterType::RepeatedCorrection);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_extract_user_messages() {
        let turns = vec![
            make_user_turn("hello, fix the bug please"),
            make_assistant_turn("I'll look into it"),
            make_user_turn("also remember to always run make gen first"),
        ];

        let messages = extract_user_messages(&turns, 42, Some("/project".into()));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].context_id, 42);
        assert_eq!(messages[0].project_path, Some("/project".into()));
    }

    fn make_user_turn(text: &str) -> TypedTurn {
        TypedTurn {
            turn_id: Some(1),
            depth: None,
            data: serde_json::json!({
                "item_type": "user_input",
                "user_input": {"text": text}
            }),
            declared_type: None,
        }
    }

    fn make_assistant_turn(text: &str) -> TypedTurn {
        TypedTurn {
            turn_id: Some(2),
            depth: None,
            data: serde_json::json!({
                "item_type": "assistant",
                "assistant": {"text": text}
            }),
            declared_type: None,
        }
    }
}
