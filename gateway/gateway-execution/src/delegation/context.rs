//! # Delegation Context
//!
//! Types for tracking delegation relationships between agents.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;

/// Execution posture for a delegated child agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationMode {
    /// Exact-output standalone artifact work; write first, verify, return paths.
    DirectArtifact,
    /// Implementation work that depends on existing ward context.
    WardBackedBuild,
    /// Execute a planned/spec step with acceptance criteria.
    StepExecutor,
}

impl DelegationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectArtifact => "direct_artifact",
            Self::WardBackedBuild => "ward_backed_build",
            Self::StepExecutor => "step_executor",
        }
    }

    pub fn as_state_value(self) -> serde_json::Value {
        serde_json::Value::String(self.as_str().to_string())
    }
}

impl FromStr for DelegationMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "direct_artifact" => Ok(Self::DirectArtifact),
            "ward_backed_build" => Ok(Self::WardBackedBuild),
            "step_executor" => Ok(Self::StepExecutor),
            other => Err(format!("unknown delegation mode '{other}'")),
        }
    }
}

/// Resolve a child delegation's execution posture.
///
/// Explicit caller intent wins. Inference is intentionally conservative:
/// ambiguous implementation work uses ward-backed context instead of the lean
/// direct-artifact path.
pub fn infer_delegation_mode(
    child_agent_id: &str,
    task: &str,
    explicit_mode: Option<DelegationMode>,
) -> DelegationMode {
    if let Some(mode) = explicit_mode {
        return mode;
    }

    let lower = task.to_lowercase();

    if looks_like_step_executor(&lower) {
        return DelegationMode::StepExecutor;
    }
    if looks_like_direct_artifact(child_agent_id, &lower) {
        return DelegationMode::DirectArtifact;
    }

    DelegationMode::WardBackedBuild
}

fn looks_like_step_executor(task_lower: &str) -> bool {
    (task_lower.contains("## goal")
        && (task_lower.contains("## acceptance") || task_lower.contains("acceptance criteria")))
        || task_lower.contains("steps/step")
        || task_lower.contains("step_")
}

fn looks_like_direct_artifact(child_agent_id: &str, task_lower: &str) -> bool {
    let builder_like = child_agent_id == "builder-agent"
        || child_agent_id.contains("builder")
        || child_agent_id.contains("code");
    if !builder_like {
        return false;
    }

    let exact_output = task_lower.contains("create a single file")
        || task_lower.contains("single file:")
        || task_lower.contains("exact output")
        || task_lower.contains("exact path")
        || task_lower.contains("write_file to create")
        || task_lower.contains("create `")
        || task_lower.contains("create ");
    let self_contained = task_lower.contains("self-contained")
        || task_lower.contains("no dependencies")
        || task_lower.contains("no build step")
        || task_lower.contains("all html")
        || task_lower.contains("standalone");
    let artifact_path = task_lower.contains(".html")
        || task_lower.contains(".css")
        || task_lower.contains(".js")
        || task_lower.contains(".md")
        || task_lower.contains(".json")
        || task_lower.contains(".wav")
        || task_lower.contains(".ogg")
        || task_lower.contains(".txt");

    artifact_path && (self_contained || exact_output)
}

// ============================================================================
// DELEGATION REQUEST
// ============================================================================

/// Request to spawn a delegated subagent.
///
/// This is sent from the parent agent's execution to spawn a child agent
/// that will handle a delegated task.
///
/// The `child_execution_id` is created synchronously when the delegation is
/// requested, ensuring the execution record exists before `try_complete_session()`
/// is called. This prevents a race condition where the session could be marked
/// COMPLETED before the subagent execution exists.
#[derive(Debug, Clone)]
pub struct DelegationRequest {
    /// ID of the parent agent initiating the delegation
    pub parent_agent_id: String,
    /// Session ID (shared across the entire conversation tree)
    pub session_id: String,
    /// Execution ID of the parent (for linking child to parent)
    pub parent_execution_id: String,
    /// Conversation ID of the parent agent (used for routing realtime events
    /// to the parent's WebSocket subscription scope).
    pub parent_conversation_id: String,
    /// ID of the child agent to spawn
    pub child_agent_id: String,
    /// Pre-created execution ID for the child agent.
    ///
    /// This execution is created synchronously when the delegation is requested,
    /// with status QUEUED. The spawn handler will transition it to RUNNING.
    pub child_execution_id: String,
    /// Task description for the child agent
    pub task: String,
    /// Optional explicit execution posture for the child agent.
    pub mode: Option<DelegationMode>,
    /// Optional context to pass to the child agent
    pub context: Option<Value>,
    /// Optional max iterations for the child agent execution loop.
    /// Defaults to 25 if not specified.
    pub max_iterations: Option<u32>,
    /// Optional JSON Schema the child agent's response must conform to.
    ///
    /// When provided, the child's system prompt is augmented with an output
    /// contract requiring a JSON response matching this schema.
    pub output_schema: Option<Value>,

    /// Skills to pre-load for the subagent.
    pub skills: Vec<String>,

    /// Explicit dynamic capabilities requested for this child. The spawn path
    /// validates and resolves these IDs before building its executor.
    pub capability_assignment: Option<agent_primitives::event::AgentCapabilityAssignment>,

    /// Host-owned safe catalog made available only to a planner or ward-agent.
    pub planning_capability_catalog: Option<Value>,

    /// Task complexity level ("S", "M", "L", "XL") for budget enforcement.
    pub complexity: Option<String>,

    /// Whether to run in parallel (bypass per-session queue).
    pub parallel: bool,
}

// ============================================================================
// DELEGATION CONTEXT
// ============================================================================

/// Context for delegated task execution.
///
/// When a parent agent delegates to a subagent, this context tracks
/// the relationship and enables callbacks on completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationContext {
    /// Session ID (shared across the entire conversation tree).
    pub session_id: String,

    /// Execution ID of the parent agent.
    pub parent_execution_id: String,

    /// ID of the parent agent that initiated delegation.
    pub parent_agent_id: String,

    /// Conversation ID of the parent agent (legacy, for backward compatibility).
    pub parent_conversation_id: String,

    /// Task-scoped context passed from parent.
    #[serde(default)]
    pub task_context: Option<Value>,

    /// Whether to send a callback message on completion.
    #[serde(default = "default_callback")]
    pub callback_on_complete: bool,

    /// Conversation ID of the child agent (for routing events back).
    #[serde(default)]
    pub child_conversation_id: Option<String>,

    /// Optional JSON Schema the child's response should conform to.
    ///
    /// Stored here so the callback handler can validate the child's response
    /// at completion time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,

    /// Execution posture for the delegated child, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DelegationMode>,
}

fn default_callback() -> bool {
    true
}

impl DelegationContext {
    /// Create a new delegation context.
    pub fn new(
        session_id: impl Into<String>,
        parent_execution_id: impl Into<String>,
        parent_agent_id: impl Into<String>,
        parent_conversation_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            parent_execution_id: parent_execution_id.into(),
            parent_agent_id: parent_agent_id.into(),
            parent_conversation_id: parent_conversation_id.into(),
            task_context: None,
            callback_on_complete: true,
            child_conversation_id: None,
            output_schema: None,
            mode: None,
        }
    }

    /// Set task-scoped context.
    pub fn with_context(mut self, context: Value) -> Self {
        self.task_context = Some(context);
        self
    }

    /// Set the child conversation ID.
    pub fn with_child_conversation_id(mut self, id: String) -> Self {
        self.child_conversation_id = Some(id);
        self
    }

    /// Disable callback on completion.
    pub fn without_callback(mut self) -> Self {
        self.callback_on_complete = false;
        self
    }

    /// Set the output schema for response validation.
    pub fn with_output_schema(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Set the delegation mode for this context.
    pub fn with_mode(mut self, mode: DelegationMode) -> Self {
        self.mode = Some(mode);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delegation_context() {
        let ctx = DelegationContext::new("sess-123", "exec-456", "parent-agent", "parent-conv")
            .with_context(serde_json::json!({"key": "value"}));

        assert_eq!(ctx.session_id, "sess-123");
        assert_eq!(ctx.parent_execution_id, "exec-456");
        assert_eq!(ctx.parent_agent_id, "parent-agent");
        assert_eq!(ctx.parent_conversation_id, "parent-conv");
        assert!(ctx.callback_on_complete);
        assert!(ctx.task_context.is_some());
    }

    #[test]
    fn delegation_mode_parses_wire_values() {
        assert_eq!(
            "direct_artifact".parse::<DelegationMode>().unwrap(),
            DelegationMode::DirectArtifact
        );
        assert_eq!(
            "ward_backed_build".parse::<DelegationMode>().unwrap(),
            DelegationMode::WardBackedBuild
        );
        assert_eq!(
            "step_executor".parse::<DelegationMode>().unwrap(),
            DelegationMode::StepExecutor
        );
        assert!("unknown".parse::<DelegationMode>().is_err());
    }

    #[test]
    fn explicit_delegation_mode_wins() {
        assert_eq!(
            infer_delegation_mode(
                "builder-agent",
                "Create a single file: index.html",
                Some(DelegationMode::WardBackedBuild)
            ),
            DelegationMode::WardBackedBuild
        );
    }

    #[test]
    fn infers_step_executor_for_step_specs() {
        assert_eq!(
            infer_delegation_mode(
                "builder-agent",
                "## Goal\nBuild it\n## Inputs\nx\n## Acceptance\npasses",
                None
            ),
            DelegationMode::StepExecutor
        );
        assert_eq!(
            infer_delegation_mode("builder-agent", "Execute specs/foo/steps/step2.md", None),
            DelegationMode::StepExecutor
        );
    }

    #[test]
    fn infers_direct_artifact_for_exact_self_contained_outputs() {
        assert_eq!(
            infer_delegation_mode(
                "builder-agent",
                "Create a single file: pomodoro-timer/index.html. All HTML, CSS, and JS inline.",
                None
            ),
            DelegationMode::DirectArtifact
        );
    }

    #[test]
    fn infers_ward_backed_build_for_ambiguous_builder_work() {
        assert_eq!(
            infer_delegation_mode("builder-agent", "Build the data import module", None),
            DelegationMode::WardBackedBuild
        );
    }
}
