use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// User-facing creation regime that shifts the balance between creating
/// new topics and appending to existing ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Regime {
    /// Prefer creating new topics; stricter matching.
    Aggressive,
    /// Current default behavior.
    #[default]
    Balanced,
    /// Prefer appending to existing topics; looser matching.
    Conservative,
}

impl fmt::Display for Regime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Regime::Aggressive => write!(f, "aggressive"),
            Regime::Balanced => write!(f, "balanced"),
            Regime::Conservative => write!(f, "conservative"),
        }
    }
}

impl FromStr for Regime {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "aggressive" => Ok(Regime::Aggressive),
            "balanced" => Ok(Regime::Balanced),
            "conservative" => Ok(Regime::Conservative),
            other => Err(format!(
                "Invalid regime '{}'. Valid options: aggressive, balanced, conservative",
                other
            )),
        }
    }
}

impl Serialize for Regime {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Regime {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Regime::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// How titles are compared for matching existing topics.
#[derive(Debug, Clone, PartialEq)]
pub enum TitleMatchMode {
    /// Only match when slugified titles are identical.
    Exact,
    /// Match when one title contains the other (current behavior).
    Contains,
    /// Match based on Jaccard similarity of slug tokens.
    FuzzyTokenJaccard { min_similarity: f64 },
}

/// Controls how dedupe matches learnings to existing topics.
#[derive(Debug, Clone)]
pub struct DedupePolicy {
    /// The active regime.
    pub regime: Regime,
    /// Word-overlap ratio to consider a learning "covered" (0.0-1.0).
    pub coverage_threshold: f64,
    /// Minimum tag overlap count for tag-based matching.
    pub tag_overlap_minimum: usize,
    /// Whether tag-overlap matching also requires slug substring affinity.
    pub require_slug_affinity: bool,
    /// How titles are compared for matching.
    pub title_match_mode: TitleMatchMode,
    /// Minimum tag Jaccard similarity for semantic duplicate detection (0.0-1.0).
    /// Set to 1.0 to disable.
    pub tag_jaccard_threshold: f64,
}

impl Default for DedupePolicy {
    fn default() -> Self {
        Self::balanced()
    }
}

impl DedupePolicy {
    /// Return the preset for the Aggressive regime.
    pub fn aggressive() -> Self {
        DedupePolicy {
            regime: Regime::Aggressive,
            coverage_threshold: 0.95,
            tag_overlap_minimum: 3,
            require_slug_affinity: true,
            title_match_mode: TitleMatchMode::Exact,
            tag_jaccard_threshold: 1.0,
        }
    }

    /// Return the preset for the Balanced regime (matches current hard-coded behavior).
    pub fn balanced() -> Self {
        DedupePolicy {
            regime: Regime::Balanced,
            coverage_threshold: 0.8,
            tag_overlap_minimum: 2,
            require_slug_affinity: true,
            title_match_mode: TitleMatchMode::Contains,
            tag_jaccard_threshold: 0.5,
        }
    }

    /// Return the preset for the Conservative regime.
    pub fn conservative() -> Self {
        DedupePolicy {
            regime: Regime::Conservative,
            coverage_threshold: 0.6,
            tag_overlap_minimum: 1,
            require_slug_affinity: false,
            title_match_mode: TitleMatchMode::FuzzyTokenJaccard { min_similarity: 0.5 },
            tag_jaccard_threshold: 0.35,
        }
    }

    /// Construct a DedupePolicy from a named regime.
    pub fn from_regime(regime: Regime) -> Self {
        match regime {
            Regime::Aggressive => Self::aggressive(),
            Regime::Balanced => Self::balanced(),
            Regime::Conservative => Self::conservative(),
        }
    }

    /// Apply partial overrides from config on top of the current policy.
    pub fn with_overrides(
        mut self,
        coverage_threshold: Option<f64>,
        tag_overlap_minimum: Option<usize>,
        require_slug_affinity: Option<bool>,
        title_match_mode: Option<String>,
    ) -> std::result::Result<Self, String> {
        if let Some(ct) = coverage_threshold {
            if !(0.0..=1.0).contains(&ct) {
                return Err(format!(
                    "coverage_threshold must be between 0.0 and 1.0, got {ct}"
                ));
            }
            self.coverage_threshold = ct;
        }
        if let Some(tom) = tag_overlap_minimum {
            if tom < 1 {
                return Err(format!(
                    "tag_overlap_minimum must be >= 1, got {tom}"
                ));
            }
            self.tag_overlap_minimum = tom;
        }
        if let Some(rsa) = require_slug_affinity {
            self.require_slug_affinity = rsa;
        }
        if let Some(mode_str) = title_match_mode {
            self.title_match_mode = parse_title_match_mode(&mode_str)?;
        }
        Ok(self)
    }

    /// Return a human-readable summary of this policy.
    pub fn policy_summary(&self) -> String {
        let title_mode = match &self.title_match_mode {
            TitleMatchMode::Exact => "exact".to_string(),
            TitleMatchMode::Contains => "contains".to_string(),
            TitleMatchMode::FuzzyTokenJaccard { min_similarity } => {
                format!("fuzzy({:.2})", min_similarity)
            }
        };
        let slug_part = if self.require_slug_affinity {
            "+slug"
        } else {
            ""
        };
        format!(
            "{} (title={}, tags>={}{}, coverage>={:.2})",
            self.regime, title_mode, self.tag_overlap_minimum, slug_part, self.coverage_threshold
        )
    }
}

/// Compute Jaccard similarity between two sets of hyphen-split slug tokens.
///
/// Returns 0.0 if both sets are empty.
pub fn jaccard_similarity(slug_a: &str, slug_b: &str) -> f64 {
    let tokens_a: std::collections::HashSet<&str> = slug_a.split('-').filter(|t| !t.is_empty()).collect();
    let tokens_b: std::collections::HashSet<&str> = slug_b.split('-').filter(|t| !t.is_empty()).collect();

    if tokens_a.is_empty() && tokens_b.is_empty() {
        return 0.0;
    }

    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Parse a title match mode string (from config) into a TitleMatchMode.
fn parse_title_match_mode(s: &str) -> std::result::Result<TitleMatchMode, String> {
    match s.to_lowercase().as_str() {
        "exact" => Ok(TitleMatchMode::Exact),
        "contains" => Ok(TitleMatchMode::Contains),
        "fuzzy" => Ok(TitleMatchMode::FuzzyTokenJaccard { min_similarity: 0.5 }),
        other => Err(format!(
            "Invalid title_match_mode '{}'. Valid options: exact, contains, fuzzy",
            other
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regime_from_str() {
        assert_eq!(Regime::from_str("aggressive").unwrap(), Regime::Aggressive);
        assert_eq!(Regime::from_str("Balanced").unwrap(), Regime::Balanced);
        assert_eq!(Regime::from_str("CONSERVATIVE").unwrap(), Regime::Conservative);
        assert!(Regime::from_str("invalid").is_err());
    }

    #[test]
    fn test_regime_display() {
        assert_eq!(Regime::Aggressive.to_string(), "aggressive");
        assert_eq!(Regime::Balanced.to_string(), "balanced");
        assert_eq!(Regime::Conservative.to_string(), "conservative");
    }

    #[test]
    fn test_regime_serde_roundtrip() {
        let regime = Regime::Aggressive;
        let json = serde_json::to_string(&regime).unwrap();
        assert_eq!(json, r#""aggressive""#);
        let parsed: Regime = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Regime::Aggressive);
    }

    #[test]
    fn test_default_regime_is_balanced() {
        assert_eq!(Regime::default(), Regime::Balanced);
    }

    #[test]
    fn test_dedupe_policy_default_is_balanced() {
        let policy = DedupePolicy::default();
        assert_eq!(policy.regime, Regime::Balanced);
        assert!((policy.coverage_threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(policy.tag_overlap_minimum, 2);
        assert!(policy.require_slug_affinity);
        assert_eq!(policy.title_match_mode, TitleMatchMode::Contains);
    }

    #[test]
    fn test_aggressive_preset() {
        let policy = DedupePolicy::aggressive();
        assert_eq!(policy.regime, Regime::Aggressive);
        assert!((policy.coverage_threshold - 0.95).abs() < f64::EPSILON);
        assert_eq!(policy.tag_overlap_minimum, 3);
        assert!(policy.require_slug_affinity);
        assert_eq!(policy.title_match_mode, TitleMatchMode::Exact);
    }

    #[test]
    fn test_conservative_preset() {
        let policy = DedupePolicy::conservative();
        assert_eq!(policy.regime, Regime::Conservative);
        assert!((policy.coverage_threshold - 0.6).abs() < f64::EPSILON);
        assert_eq!(policy.tag_overlap_minimum, 1);
        assert!(!policy.require_slug_affinity);
        assert!(matches!(
            policy.title_match_mode,
            TitleMatchMode::FuzzyTokenJaccard { min_similarity } if (min_similarity - 0.5).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn test_from_regime() {
        let p = DedupePolicy::from_regime(Regime::Aggressive);
        assert_eq!(p.regime, Regime::Aggressive);
        let p = DedupePolicy::from_regime(Regime::Balanced);
        assert_eq!(p.regime, Regime::Balanced);
        let p = DedupePolicy::from_regime(Regime::Conservative);
        assert_eq!(p.regime, Regime::Conservative);
    }

    #[test]
    fn test_with_overrides() {
        let policy = DedupePolicy::balanced()
            .with_overrides(Some(0.85), Some(3), None, None)
            .unwrap();
        assert!((policy.coverage_threshold - 0.85).abs() < f64::EPSILON);
        assert_eq!(policy.tag_overlap_minimum, 3);
        // Other fields unchanged
        assert!(policy.require_slug_affinity);
        assert_eq!(policy.title_match_mode, TitleMatchMode::Contains);
    }

    #[test]
    fn test_with_overrides_title_match_mode() {
        let policy = DedupePolicy::balanced()
            .with_overrides(None, None, None, Some("exact".to_string()))
            .unwrap();
        assert_eq!(policy.title_match_mode, TitleMatchMode::Exact);
    }

    #[test]
    fn test_with_overrides_validation() {
        assert!(DedupePolicy::balanced()
            .with_overrides(Some(1.5), None, None, None)
            .is_err());
        assert!(DedupePolicy::balanced()
            .with_overrides(Some(-0.1), None, None, None)
            .is_err());
        assert!(DedupePolicy::balanced()
            .with_overrides(None, Some(0), None, None)
            .is_err());
        assert!(DedupePolicy::balanced()
            .with_overrides(None, None, None, Some("bogus".to_string()))
            .is_err());
    }

    #[test]
    fn test_policy_summary() {
        let summary = DedupePolicy::balanced().policy_summary();
        assert!(summary.contains("balanced"));
        assert!(summary.contains("title=contains"));
        assert!(summary.contains("tags>=2+slug"));
        assert!(summary.contains("coverage>=0.80"));
    }

    #[test]
    fn test_policy_summary_conservative() {
        let summary = DedupePolicy::conservative().policy_summary();
        assert!(summary.contains("conservative"));
        assert!(summary.contains("title=fuzzy(0.50)"));
        // No +slug since require_slug_affinity is false
        assert!(summary.contains("tags>=1,"));
    }

    #[test]
    fn test_jaccard_similarity_identical() {
        assert!((jaccard_similarity("gh-cli-auth", "gh-cli-auth") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        // "gh-cli" vs "gh-cli-auth": tokens {gh, cli} and {gh, cli, auth}
        // intersection = 2, union = 3 => 2/3 ~= 0.667
        let sim = jaccard_similarity("gh-cli", "gh-cli-auth");
        assert!((sim - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        assert!((jaccard_similarity("gh-cli", "docker-compose") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_similarity_empty() {
        assert!((jaccard_similarity("", "") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_title_match_mode() {
        assert_eq!(
            parse_title_match_mode("exact").unwrap(),
            TitleMatchMode::Exact
        );
        assert_eq!(
            parse_title_match_mode("contains").unwrap(),
            TitleMatchMode::Contains
        );
        assert!(matches!(
            parse_title_match_mode("fuzzy").unwrap(),
            TitleMatchMode::FuzzyTokenJaccard { .. }
        ));
        assert!(parse_title_match_mode("invalid").is_err());
    }
}
