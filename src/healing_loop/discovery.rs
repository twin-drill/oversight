use crate::source::types::ContextSummary;
use crate::state::LoopState;
use std::path::PathBuf;

/// A candidate context that should be processed by the loop.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub context: ContextSummary,
    /// The head turn ID at the time of discovery.
    pub head_turn_id: u64,
    /// Path to the source file, cached from discovery to avoid re-scanning.
    pub source_path: Option<PathBuf>,
    /// The project directory this conversation took place in.
    pub project_path: Option<String>,
}

pub fn filter_candidates(contexts: Vec<ContextSummary>, state: &LoopState) -> Vec<Candidate> {
    contexts
        .into_iter()
        .filter_map(|ctx| {
            let head_turn_id = ctx.head_turn_id.unwrap_or(0);
            if head_turn_id == 0 {
                return None;
            }
            if state.is_processed(ctx.id(), head_turn_id) {
                return None;
            }
            Some(Candidate {
                context: ctx,
                head_turn_id,
                source_path: None,
                project_path: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context(context_id: u64, head_turn_id: u64, head_depth: u64) -> ContextSummary {
        ContextSummary {
            context_id,
            head_turn_id: Some(head_turn_id),
            head_depth: Some(head_depth),
            created_at_unix_ms: None,
            title: None,
            client_tag: None,
            is_live: false,
        }
    }

    #[test]
    fn test_candidate_filtering() {
        let mut state = LoopState::default();
        state.mark_processed(1, 100);
        state.mark_processed(2, 200);

        let contexts = vec![
            make_context(1, 100, 5),  // Already processed at this version
            make_context(2, 300, 10), // Updated since last processing
            make_context(3, 50, 3),   // Never processed
            make_context(4, 0, 0),    // Empty context
        ];

        let candidates = filter_candidates(contexts, &state);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].context.id(), 2); // updated
        assert_eq!(candidates[1].context.id(), 3); // new
    }
}
