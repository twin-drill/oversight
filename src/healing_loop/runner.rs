use crate::config::Config;
use crate::error::Result;
use crate::healing_loop::dedupe::{self, MergeOutcome};
use crate::healing_loop::discovery::Candidate;
use crate::healing_loop::merge;
use crate::healing_loop::patterns::{self, PatternResult};
use crate::healing_loop::policy::DedupePolicy;
use crate::healing_loop::transcript;
use crate::kb::service::KBService;
use crate::llm::client::LlmClient;
use crate::llm::extractor::{self, Learning};
use crate::llm::synthesizer;
use crate::source::TranscriptSource;
use crate::state::LoopState;
use chrono::Utc;
use std::path::Path;

/// Result of a single run pass.
#[derive(Debug)]
pub struct RunResult {
    pub contexts_discovered: u32,
    pub learnings_extracted: u32,
    pub topics_created: u32,
    pub topics_updated: u32,
    pub duplicates_skipped: u32,
    pub pattern_result: Option<PatternResult>,
    pub errors: Vec<String>,
}

impl RunResult {
    /// Return a human-readable summary.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "Contexts discovered: {}\nLearnings extracted: {}\nTopics created: {}\nTopics updated: {}\nDuplicates skipped: {}",
            self.contexts_discovered,
            self.learnings_extracted,
            self.topics_created,
            self.topics_updated,
            self.duplicates_skipped,
        );
        if let Some(ref pr) = self.pattern_result {
            s.push_str(&format!("\n{}", pr.summary()));
        }
        if !self.errors.is_empty() {
            s.push_str(&format!("\nErrors: {}", self.errors.len()));
            for err in &self.errors {
                s.push_str(&format!("\n  - {err}"));
            }
        }
        s
    }
}

/// The pipeline runner that orchestrates a single discover -> reduce -> extract -> dedup -> merge cycle.
pub struct Runner {
    config: Config,
    state_path: Option<std::path::PathBuf>,
}

/// Services and configuration needed to process a candidate.
struct ProcessContext<'a> {
    source: &'a TranscriptSource,
    kb: &'a KBService,
    llm: Option<&'a LlmClient>,
    policy: &'a DedupePolicy,
}

impl Runner {
    /// Create a new runner with the given config.
    pub fn new(config: Config) -> Self {
        Runner {
            config,
            state_path: None,
        }
    }

    /// Override the state file path (for testing).
    pub fn with_state_path(mut self, path: std::path::PathBuf) -> Self {
        self.state_path = Some(path);
        self
    }

    fn state_path(&self) -> std::path::PathBuf {
        self.state_path
            .clone()
            .unwrap_or_else(Config::state_path)
    }

    /// Run a single pass of the healing loop.
    ///
    /// If `dry_run` is true, no KB writes or state updates are made.
    pub async fn run_once(&self, dry_run: bool) -> Result<RunResult> {
        let mut result = RunResult {
            contexts_discovered: 0,
            learnings_extracted: 0,
            topics_created: 0,
            topics_updated: 0,
            duplicates_skipped: 0,
            pattern_result: None,
            errors: Vec::new(),
        };

        // Construct DedupePolicy from config (CLI override already applied to config)
        let policy = self
            .config
            .loop_config
            .build_dedupe_policy(None)
            .map_err(crate::error::Error::Config)?;

        eprintln!("Active regime: {}", policy.policy_summary());

        // Load state
        let state_path = self.state_path();
        let mut state = LoopState::load(&state_path)?;

        // Initialize services
        let source = self.config.build_source();
        let kb = KBService::new(self.config.clone());

        // Ensure KB is initialized
        if !kb.is_initialized() {
            kb.init()?;
            eprintln!("Initialized KB at {}", self.config.kb_path.display());
        }

        // Phase 1: Discover candidates
        eprintln!("Discovering contexts from {}...", source.source_name());
        let candidates = source
            .discover_candidates(&state, self.config.loop_config.context_limit)
            .await?;
        result.contexts_discovered = candidates.len() as u32;
        eprintln!("Found {} candidate context(s)", candidates.len());

        if candidates.is_empty() {
            state.last_poll_at = Some(Utc::now());
            if !dry_run {
                state.save(&state_path)?;
            }
            return Ok(result);
        }

        // Set up LLM client
        let llm = match LlmClient::from_config(self.config.llm.clone()) {
            Ok(client) => Some(client),
            Err(e) => {
                eprintln!("Warning: {} client not available: {e}", self.config.llm.provider);
                eprintln!("Extraction will be skipped.");
                None
            }
        };

        // Phase 2-5: Process each candidate and collect user messages for pattern detection
        let mut all_user_messages: Vec<patterns::UserMessage> = Vec::new();
        let mut all_corrections: Vec<patterns::CorrectionSequence> = Vec::new();
        let ctx = ProcessContext {
            source: &source,
            kb: &kb,
            llm: llm.as_ref(),
            policy: &policy,
        };
        for candidate in &candidates {
            // Collect raw turns for pattern detection before processing
            if self.config.loop_config.patterns.enabled {
                if let Ok(turns) = ctx.source.get_turns(candidate).await {
                    let ctx_id = candidate.context.id();
                    let project = candidate.project_path.clone();
                    let msgs = patterns::extract_user_messages(&turns, ctx_id, project.clone());
                    all_user_messages.extend(msgs);
                    let corrs = patterns::detect_corrections(&turns, ctx_id, project);
                    all_corrections.extend(corrs);
                }
            }

            match self
                .process_candidate(
                    &ctx,
                    &mut state,
                    candidate,
                    dry_run,
                )
                .await
            {
                Ok(candidate_result) => {
                    result.learnings_extracted += candidate_result.learnings_extracted;
                    result.topics_created += candidate_result.topics_created;
                    result.topics_updated += candidate_result.topics_updated;
                    result.duplicates_skipped += candidate_result.duplicates_skipped;
                }
                Err(e) => {
                    let msg = format!(
                        "Error processing context {}: {e}",
                        candidate.context.id()
                    );
                    eprintln!("{msg}");
                    result.errors.push(msg);
                }
            }
        }

        // Phase 6: Cross-conversation pattern detection
        if self.config.loop_config.patterns.enabled {
            let pattern_result = self.run_pattern_detection(
                &ctx,
                &mut state,
                &all_user_messages,
                &all_corrections,
                dry_run,
            ).await;
            result.pattern_result = Some(pattern_result);
        }

        // Save state
        state.last_poll_at = Some(Utc::now());
        if !dry_run {
            state.save(&state_path)?;
        }

        Ok(result)
    }

    /// Process a single candidate context through the pipeline.
    async fn process_candidate(
        &self,
        ctx: &ProcessContext<'_>,
        state: &mut LoopState,
        candidate: &Candidate,
        dry_run: bool,
    ) -> Result<RunResult> {
        let mut result = RunResult {
            contexts_discovered: 0,
            learnings_extracted: 0,
            topics_created: 0,
            topics_updated: 0,
            duplicates_skipped: 0,
            pattern_result: None,
            errors: Vec::new(),
        };

        let ctx_id = candidate.context.id();
        eprintln!(
            "Processing context {} (head_turn: {})...",
            ctx_id, candidate.head_turn_id
        );

        // Phase 2: Fetch and reduce transcript
        let turns = ctx.source.get_turns(candidate).await?;
        if turns.is_empty() {
            eprintln!("  No turns found, skipping.");
            if !dry_run {
                state.mark_processed(ctx_id, candidate.head_turn_id);
            }
            return Ok(result);
        }

        let mut reduced = transcript::reduce_transcript(
            &turns,
            self.config.loop_config.max_transcript_len,
        );
        if reduced.trim().is_empty() {
            eprintln!("  No tool-relevant content, skipping.");
            if !dry_run {
                state.mark_processed(ctx_id, candidate.head_turn_id);
            }
            return Ok(result);
        }

        if let Some(ref project) = candidate.project_path {
            reduced = format!("[PROJECT: {project}]\n\n{reduced}");
        }

        // Phase 3: Extract learnings via LLM
        let llm_client = match ctx.llm {
            Some(c) => c,
            None => {
                eprintln!("  Skipping extraction (no LLM client).");
                if !dry_run {
                    state.mark_processed(ctx_id, candidate.head_turn_id);
                }
                return Ok(result);
            }
        };

        // Collect existing tags for the LLM prompt
        let all_topics = ctx.kb.load_topics()?;
        let existing_tags: Vec<String> = {
            let mut tags: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for topic in &all_topics {
                for tag in &topic.tags {
                    tags.insert(tag.to_lowercase());
                }
            }
            tags.into_iter().collect()
        };

        let extraction = extractor::extract_learnings(llm_client, ctx_id, &reduced, &ctx.policy.regime, &existing_tags).await?;
        let learnings = extractor::filter_by_confidence(
            extraction.learnings,
            self.config.loop_config.confidence_threshold,
        );

        // Filter out already-seen extraction hashes
        let novel_learnings: Vec<Learning> = learnings
            .into_iter()
            .filter(|l| {
                let hash = l.content_hash();
                !state.has_extraction_hash(&hash)
            })
            .map(|l| l.with_project(candidate.project_path.clone()))
            .collect();

        result.learnings_extracted = novel_learnings.len() as u32;
        eprintln!("  Extracted {} novel learning(s)", novel_learnings.len());

        if novel_learnings.is_empty() {
            if !dry_run {
                state.mark_processed(ctx_id, candidate.head_turn_id);
            }
            return Ok(result);
        }

        // Phase 4: Deduplicate against KB
        let outcomes = dedupe::deduplicate(&novel_learnings, &all_topics, ctx.policy);

        // Phase 5: Merge or dry-run
        if dry_run {
            let summary = merge::dry_run_summary(&outcomes);
            eprintln!("{summary}");

            for outcome in &outcomes {
                match outcome {
                    MergeOutcome::CreateTopic { .. } => result.topics_created += 1,
                    MergeOutcome::AppendInsight { .. } => result.topics_updated += 1,
                    MergeOutcome::NoOpDuplicate { .. } => result.duplicates_skipped += 1,
                }
            }
        } else {
            let merge_result = merge::apply_merges(ctx.kb, &outcomes, ctx_id)?;
            result.topics_created += merge_result.topics_created;
            result.topics_updated += merge_result.topics_updated;
            result.duplicates_skipped += merge_result.duplicates_skipped;

            // Record extraction hashes
            for learning in &novel_learnings {
                state.add_extraction_hash(learning.content_hash());
            }

            state.mark_processed(ctx_id, candidate.head_turn_id);
        }

        Ok(result)
    }

    /// Run cross-conversation pattern detection and synthesis.
    async fn run_pattern_detection(
        &self,
        ctx: &ProcessContext<'_>,
        state: &mut LoopState,
        user_messages: &[patterns::UserMessage],
        corrections: &[patterns::CorrectionSequence],
        dry_run: bool,
    ) -> PatternResult {
        let mut pr = PatternResult::empty();
        pr.messages_scanned = user_messages.len();
        pr.corrections_found = corrections.len();

        eprintln!(
            "Pattern detection: {} messages, {} corrections",
            user_messages.len(),
            corrections.len()
        );

        let pattern_config = &self.config.loop_config.patterns;
        let clusters = patterns::detect_patterns(
            user_messages,
            corrections,
            pattern_config,
            state.pattern_hashes(),
        );
        pr.clusters_detected = clusters.len();

        if clusters.is_empty() {
            eprintln!("  No new pattern clusters detected.");
            return pr;
        }

        eprintln!("  Found {} pattern cluster(s)", clusters.len());
        for (i, cluster) in clusters.iter().enumerate() {
            eprintln!(
                "  Cluster {}: {} ({} occurrences across {} contexts)",
                i + 1,
                cluster.cluster_type,
                cluster.occurrences.len(),
                cluster
                    .occurrences
                    .iter()
                    .map(|o| o.context_id)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
            );
        }

        let llm_client = match ctx.llm {
            Some(c) => c,
            None => {
                eprintln!("  Skipping pattern synthesis (no LLM client).");
                return pr;
            }
        };

        let all_topics = match ctx.kb.load_topics() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  Failed to load topics for pattern synthesis: {e}");
                return pr;
            }
        };
        let existing_tags: Vec<String> = {
            let mut tags: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for topic in &all_topics {
                for tag in &topic.tags {
                    tags.insert(tag.to_lowercase());
                }
            }
            tags.into_iter().collect()
        };

        let directives = match synthesizer::synthesize_patterns(
            llm_client,
            &clusters,
            &existing_tags,
        )
        .await
        {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  Pattern synthesis failed: {e}");
                return pr;
            }
        };

        pr.clusters_synthesized = directives.len();
        eprintln!("  Synthesized {} directive(s)", directives.len());

        let learnings: Vec<Learning> = directives
            .into_iter()
            .map(|d| d.into_learning())
            .collect();

        let policy = &ctx.policy;
        let outcomes = dedupe::deduplicate(&learnings, &all_topics, policy);

        if dry_run {
            let summary = merge::dry_run_summary(&outcomes);
            eprintln!("  Pattern directives (dry run):\n{summary}");
            for outcome in &outcomes {
                match outcome {
                    MergeOutcome::CreateTopic { .. } => pr.topics_created += 1,
                    MergeOutcome::AppendInsight { .. } => pr.topics_updated += 1,
                    MergeOutcome::NoOpDuplicate { .. } => pr.duplicates_skipped += 1,
                }
            }
        } else {
            match merge::apply_merges(ctx.kb, &outcomes, 0) {
                Ok(merge_result) => {
                    pr.topics_created += merge_result.topics_created;
                    pr.topics_updated += merge_result.topics_updated;
                    pr.duplicates_skipped += merge_result.duplicates_skipped;
                }
                Err(e) => {
                    eprintln!("  Failed to merge pattern directives: {e}");
                    return pr;
                }
            }

            for cluster in &clusters {
                state.add_pattern_hash(cluster.content_hash.clone());
            }
        }

        pr
    }
}

/// Run a single pass (convenience function for CLI).
pub async fn run_once(config: Config, dry_run: bool) -> Result<RunResult> {
    let runner = Runner::new(config);
    runner.run_once(dry_run).await
}

/// Show the current loop status.
pub fn show_status(state_path: Option<&Path>) -> Result<String> {
    let path = state_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(Config::state_path);
    let state = LoopState::load(&path)?;
    Ok(state.summary())
}
