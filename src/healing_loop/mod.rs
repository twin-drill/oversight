pub mod daemon;
pub mod dedupe;
pub mod discovery;
pub mod merge;
pub mod patterns;
pub mod policy;
pub mod runner;
pub mod scrub;
pub mod transcript;

pub use runner::Runner;
pub use dedupe::MergeOutcome;
pub use patterns::{PatternCluster, PatternConfig};
pub use policy::{DedupePolicy, Regime, TitleMatchMode};
