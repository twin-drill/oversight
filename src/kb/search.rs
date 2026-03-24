use crate::kb::types::Topic;

/// Result of a search operation, pairing a topic with its relevance score.
#[derive(Debug)]
pub struct SearchResult {
    pub topic: Topic,
    pub score: u32,
}

/// Search topics by keyword query.
///
/// Matches against slug, title, aliases, tags, and body.
/// Returns results sorted by relevance score (highest first).
pub fn search(topics: &[Topic], query: &str) -> Vec<SearchResult> {
    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();

    if terms.is_empty() {
        return Vec::new();
    }

    let mut results: Vec<SearchResult> = topics
        .iter()
        .filter_map(|topic| {
            let score = score_topic(topic, &terms);
            if score > 0 {
                Some(SearchResult {
                    topic: topic.clone(),
                    score,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending, then by slug for determinism
    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.topic.slug.cmp(&b.topic.slug))
    });

    results
}

/// Score how well a topic matches the search terms.
/// Higher score = better match.
fn score_topic(topic: &Topic, terms: &[&str]) -> u32 {
    let mut score = 0u32;

    for term in terms {
        // Slug match (high value)
        if topic.slug.contains(term) {
            score += 10;
        }

        // Title match (high value)
        if topic.title.to_lowercase().contains(term) {
            score += 10;
        }

        // Alias match (medium value)
        for alias in &topic.aliases {
            if alias.to_lowercase().contains(term) {
                score += 7;
            }
        }

        // Tag match (medium value)
        for tag in &topic.tags {
            if tag.to_lowercase().contains(term) {
                score += 5;
            }
        }

        // Body match (lower value, but still useful)
        if topic.body.to_lowercase().contains(term) {
            score += 2;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::types::Topic;

    fn make_topic(slug: &str, title: &str, aliases: &[&str], tags: &[&str], body: &str) -> Topic {
        let mut t = Topic::new(title.to_string(), slug.to_string(), body.to_string());
        t.aliases = aliases.iter().map(|s| s.to_string()).collect();
        t.tags = tags.iter().map(|s| s.to_string()).collect();
        t
    }

    #[test]
    fn test_search_by_slug() {
        let topics = vec![
            make_topic("docker-local", "Docker Local", &[], &["docker"], "Run docker."),
            make_topic("gh-cli", "GitHub CLI", &[], &["cli"], "Use gh."),
        ];

        let results = search(&topics, "docker");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].topic.slug, "docker-local");
    }

    #[test]
    fn test_search_by_title() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &[], "Content"),
        ];

        let results = search(&topics, "github");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].topic.slug, "gh-cli");
    }

    #[test]
    fn test_search_by_alias() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &["gh command"], &[], "Content"),
        ];

        let results = search(&topics, "command");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_by_body() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &[], "Run unset GITHUB_TOKEN before gh."),
        ];

        let results = search(&topics, "GITHUB_TOKEN");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_no_match() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &[], "Content"),
        ];

        let results = search(&topics, "kubernetes");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_case_insensitive() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &[], "Content"),
        ];

        let results = search(&topics, "GITHUB");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_ranking() {
        let topics = vec![
            make_topic("docker-local", "Docker Local", &[], &["docker"], "Docker content"),
            make_topic("gh-cli", "GitHub CLI", &[], &[], "Use docker sometimes"),
        ];

        let results = search(&topics, "docker");
        assert!(!results.is_empty());
        // docker-local should rank higher (matches slug, title, tag, body)
        assert_eq!(results[0].topic.slug, "docker-local");
    }

    #[test]
    fn test_search_empty_query() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &[], "Content"),
        ];

        let results = search(&topics, "");
        assert!(results.is_empty());

        let results = search(&topics, "   ");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_by_tag() {
        let topics = vec![
            make_topic("gh-cli", "GitHub CLI", &[], &["cli", "git"], "Content"),
            make_topic("docker-local", "Docker", &[], &["docker"], "Content"),
        ];

        let results = search(&topics, "cli");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].topic.slug, "gh-cli");
    }
}
