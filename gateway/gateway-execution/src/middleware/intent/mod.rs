//! Intent routing: the intent agent searches the index and submits an analysis.

pub mod agent;
pub mod contract;
pub mod inject;
pub mod prompt;
pub mod router;

pub use contract::{
    ExecutionApproach, ExecutionStrategy, IntentAnalysis, PinnedProcedure, WardAction,
    WardRecommendation,
};
pub use inject::{format_intent_injection, format_planner_task};
pub use prompt::INTENT_AGENT_PROMPT as DEFAULT_INTENT_ANALYSIS_PROMPT;
pub use router::{analyze_intent, load_intent_analysis_prompt};
