//! Intent contract — the typed decision the intent agent produces.
//!
//! The intent agent reasons about the user's request using discovery tools
//! (list_skills, list_agents, search_procedures, list_wards), then calls
//! `submit_intent` with this structure. The orchestrator consumes it to
//! route, load resources, and delegate.

use agent_primitives::event::AgentCapabilityAssignment;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The routed decision for one user request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct IntentAnalysis {
    /// Concise kebab-case phrase describing the user's main goal.
    pub primary_intent: String,
    /// Actionable implicit requirements the user expects but didn't state.
    pub hidden_intents: Vec<String>,
    /// High-level steps the agent identified for solving this request.
    /// Drives the planner's first draft when approach is graph.
    #[serde(default)]
    pub solution_path: Vec<String>,
    /// Skills to load (names from list_skills tool output).
    pub recommended_skills: Vec<String>,
    /// Agents to delegate to (names from list_agents tool output).
    pub recommended_agents: Vec<String>,
    /// Procedures that could handle this request (from search_procedures).
    #[serde(default)]
    pub recommended_procedures: Vec<String>,
    /// Capability recommendations grouped by the exact agent that may use them.
    #[serde(default)]
    pub recommended_capabilities: Vec<AgentCapabilityAssignment>,
    /// Reusable domain category for the work.
    pub ward_recommendation: WardRecommendation,
    /// Orchestration posture.
    pub execution_strategy: ExecutionStrategy,
    /// Task complexity: S, M, L, or XL. Sets iteration budget.
    #[serde(default)]
    pub complexity: Option<String>,
    /// Why the agent made these choices. Surfaced in the UI.
    #[serde(default)]
    pub explanation: String,
    /// Server-computed: a proven procedure whose name the request matched.
    #[serde(skip)]
    pub pinned_procedure: Option<PinnedProcedure>,
}

/// A deterministic name-match hit against the global procedure index.
#[derive(Debug, Clone, PartialEq)]
pub struct PinnedProcedure {
    pub name: String,
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
    #[serde(default)]
    pub explanation: String,
}
