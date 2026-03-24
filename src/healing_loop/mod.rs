pub mod daemon;
pub mod dedupe;
pub mod discovery;
pub mod merge;
pub mod policy;
pub mod runner;
pub mod transcript;

pub use runner::Runner;
pub use dedupe::MergeOutcome;
pub use policy::{DedupePolicy, Regime, TitleMatchMode};
