//! Intent contract — the typed decision a router returns.
//!
//! The LLM judges only what it uniquely knows (posture, domain, implicit
//! requirements). Skills/agents/MCPs come from retrieval, enforcement from
//! the runtime — neither is re-narrated here.

use agent_primitives::event::AgentCapabilityAssignment;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The routed decision for one user request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct IntentAnalysis {
    /// Concise kebab-case or short phrase describing the user's main goal.
    pub primary_intent: String,
    /// Actionable implicit requirements the user expects but didn't state.
    pub hidden_intents: Vec<String>,
    /// Retrieved skill candidates (embedding search, not LLM judgment).
    pub recommended_skills: Vec<String>,
    /// Retrieved agent candidates (embedding search, not LLM judgment).
    pub recommended_agents: Vec<String>,
    /// Capability recommendations grouped by the exact agent that may use
    /// them. MCP IDs are validated again at execution time.
    #[serde(default)]
    pub recommended_capabilities: Vec<AgentCapabilityAssignment>,
    /// Reusable domain category for the work (never task-specific).
    pub ward_recommendation: WardRecommendation,
    /// Orchestration posture.
    pub execution_strategy: ExecutionStrategy,
    /// Server-computed: a proven procedure whose name the request matched.
    /// Never requested from the LLM; carries the home ward so the directive
    /// can route a cross-ward macro invocation.
    #[serde(skip)]
    pub pinned_procedure: Option<PinnedProcedure>,
}

/// A deterministic name-match hit against the global procedure index.
#[derive(Debug, Clone, PartialEq)]
pub struct PinnedProcedure {
    pub name: String,
    /// Home ward of the procedure; the invocation executes with its context
    /// regardless of the session's current ward.
    pub ward_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WardAction {
    UseExisting,
    CreateNew,
}

impl WardAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UseExisting => "use_existing",
            Self::CreateNew => "create_new",
        }
    }
}

impl std::fmt::Display for WardAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WardRecommendation {
    pub action: WardAction,
    pub ward_name: String,
    #[serde(default)]
    pub subdirectory: Option<String>,
    #[serde(default)]
    pub structure: HashMap<String, serde_json::Value>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionApproach {
    Simple,
    Graph,
}

impl ExecutionApproach {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Graph => "graph",
        }
    }
}

impl std::fmt::Display for ExecutionApproach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ExecutionStrategy {
    pub approach: ExecutionApproach,
    /// One-line classifier rationale; surfaced as the approach note.
    #[serde(default)]
    pub explanation: String,
}
