use crate::config::Config;
use crate::error::Result;
use crate::healing_loop::dedupe::{self, MergeOutcome};
use crate::healing_loop::discovery::Candidate;
use crate::healing_loop::merge;
use crate::healing_loop::policy::DedupePolicy;
use crate::healing_loop::transcript;
use crate::kb::service::KBService;
use crate::llm::client::LlmClient;
use crate::llm::extractor::{self, Learning};
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

        // Phase 2-5: Process each candidate
        let ctx = ProcessContext {
            source: &source,
            kb: &kb,
            llm: llm.as_ref(),
            policy: &policy,
        };
        for candidate in &candidates {
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

        let reduced = transcript::reduce_transcript(
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

        let extraction = extractor::extract_learnings(llm_client, ctx_id, &reduced, &ctx.policy.regime).await?;
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
        let all_topics = ctx.kb.load_topics()?;
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
