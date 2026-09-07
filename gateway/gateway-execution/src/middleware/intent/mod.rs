//! Intent routing: the intent agent reasons about a request and produces
//! an orchestration decision via submit_intent.

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
pub use prompt::{
    load_intent_analysis_prompt, INTENT_AGENT_PROMPT as DEFAULT_INTENT_ANALYSIS_PROMPT,
};
pub use router::analyze_intent;
