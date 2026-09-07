//! Intent routing: classify a request into an orchestration decision
//! before the engine builds. See `router.rs` for the paths, `contract.rs`
//! for the typed decision, `inject.rs` for rendering, and
//! `crate::middleware::resource_index` for the catalog write path.

mod contract;
mod inject;
mod prompt;
mod router;

pub use contract::{
    ExecutionApproach, ExecutionStrategy, IntentAnalysis, PinnedProcedure, WardAction,
    WardRecommendation,
};
pub use inject::{format_intent_injection, format_planner_task};
pub use prompt::{load_intent_analysis_prompt, DEFAULT_INTENT_ANALYSIS_PROMPT};
pub use router::analyze_intent;
