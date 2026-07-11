//! # Gateway Events
//!
//! Event bus for broadcasting events to connected clients.
//!
//! This crate provides the `EventBus` and `GatewayEvent` types used throughout
//! the AgentZero gateway for real-time event distribution.

mod broadcast;
pub mod context;

pub use broadcast::EventBus;
pub use context::{HookContext, HookType};

use agent_surfaces::WorkSurface;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Gateway event types.
///
/// All events include `session_id` and `execution_id` for routing and filtering:
/// - `session_id`: Top-level session ID (sess-xxx) - used for subscription routing
/// - `execution_id`: Specific execution ID (exec-xxx) - used for filtering (root vs subagent)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayEvent {
    /// Validated declarative surface created by an execution.
    SurfaceCreated {
        session_id: String,
        execution_id: String,
        surface: WorkSurface,
    },
    SurfaceUpdated {
        session_id: String,
        execution_id: String,
        surface: WorkSurface,
    },
    SurfaceDeleted {
        session_id: String,
        execution_id: String,
        surface_id: String,
    },
    /// A surface was rejected before client publication.
    SurfaceValidationFailed {
        session_id: String,
        execution_id: String,
        surface_id: String,
        reason: String,
    },
    /// Agent started executing.
    AgentStarted {
        agent_id: String,
        /// Session ID for subscription routing.
        session_id: String,
        /// Execution ID for filtering (root vs subagent).
        execution_id: String,
        /// Legacy conversation_id for backward compatibility.
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Agent completed execution.
    AgentCompleted {
        agent_id: String,
        session_id: String,
        execution_id: String,
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Agent was stopped by user request.
    AgentStopped {
        agent_id: String,
        session_id: String,
        execution_id: String,
        iteration: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// General error event.
    Error {
        agent_id: Option<String>,
        session_id: Option<String>,
        execution_id: Option<String>,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Streaming token from agent.
    Token {
        agent_id: String,
        session_id: String,
        execution_id: String,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Thinking/reasoning content from agent.
    Thinking {
        agent_id: String,
        session_id: String,
        execution_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Tool call started.
    ToolCall {
        agent_id: String,
        session_id: String,
        execution_id: String,
        tool_id: String,
        tool_name: String,
        args: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Tool call completed.
    ToolResult {
        agent_id: String,
        session_id: String,
        execution_id: String,
        tool_id: String,
        result: String,
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Turn complete (agent finished responding).
    TurnComplete {
        agent_id: String,
        session_id: String,
        execution_id: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Iteration update for tracking progress.
    IterationUpdate {
        agent_id: String,
        session_id: String,
        execution_id: String,
        current: u32,
        max: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Continuation prompt when max iterations reached.
    ContinuationPrompt {
        agent_id: String,
        session_id: String,
        execution_id: String,
        iteration: u32,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Response from the respond tool.
    ///
    /// This event is emitted when an agent uses the `respond` tool
    /// to send a message back to the originating hook.
    Respond {
        session_id: String,
        execution_id: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Delegation started event.
    ///
    /// Emitted when an agent delegates to a subagent.
    DelegationStarted {
        /// Session ID for subscription routing (same for parent and child).
        session_id: String,
        /// Parent execution ID.
        parent_execution_id: String,
        /// Child execution ID.
        child_execution_id: String,
        parent_agent_id: String,
        child_agent_id: String,
        task: String,
        /// Legacy fields for backward compatibility.
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        child_conversation_id: Option<String>,
    },

    /// Delegation completed event.
    ///
    /// Emitted when a delegated subagent completes.
    DelegationCompleted {
        session_id: String,
        parent_execution_id: String,
        child_execution_id: String,
        parent_agent_id: String,
        child_agent_id: String,
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        child_conversation_id: Option<String>,
    },

    /// New message added to conversation.
    ///
    /// Emitted when a message is added outside of normal streaming
    /// (e.g., delegation callbacks, system messages).
    /// Frontend should refresh the conversation to show the new message.
    MessageAdded {
        session_id: String,
        execution_id: String,
        role: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Token usage update for a session.
    ///
    /// Emitted after each LLM call with cumulative token counts.
    TokenUsage {
        session_id: String,
        execution_id: String,
        tokens_in: u64,
        tokens_out: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Execution heartbeat — signals the execution is alive during silent phases.
    Heartbeat {
        session_id: String,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// All delegations for a session have completed, continuation can proceed.
    ///
    /// Emitted when the last pending delegation completes and the session
    /// has requested continuation.
    SessionContinuationReady {
        session_id: String,
        root_agent_id: String,
        /// The execution ID that should be continued
        root_execution_id: String,
    },

    /// Agent switched to a different ward (project directory).
    WardChanged {
        session_id: String,
        execution_id: String,
        ward_id: String,
    },

    /// Agent's plan was updated via update_plan tool.
    PlanUpdate {
        session_id: String,
        execution_id: String,
        plan: Value,
        explanation: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Executor auto-extended iterations because the agent is making progress.
    IterationsExtended {
        session_id: String,
        execution_id: String,
        /// Total iterations used so far.
        iterations_used: u32,
        /// Additional iterations granted.
        iterations_added: u32,
        /// Human-readable reason for extension.
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },

    /// Session title changed by runtime title derivation or legacy log replay.
    SessionTitleChanged { session_id: String, title: String },

    /// Intent analysis started for a root session (pre-execution)
    IntentAnalysisStarted {
        session_id: String,
        execution_id: String,
    },

    /// Intent analysis completed for a root session
    IntentAnalysisComplete {
        session_id: String,
        execution_id: String,
        primary_intent: String,
        hidden_intents: Vec<String>,
        recommended_skills: Vec<String>,
        recommended_agents: Vec<String>,
        ward_recommendation: serde_json::Value,
        execution_strategy: serde_json::Value,
    },

    /// Intent analysis skipped (already analyzed in this session)
    IntentAnalysisSkipped {
        session_id: String,
        execution_id: String,
    },

    /// A markdown file in <vault>/config/ was created, modified, or deleted.
    /// Only emitted for files matching the customization allow-list (root
    /// `*.md` and `shards/*.md`). Used by the Settings → Customization tab
    /// to refresh its file list and detect external edits while a file is
    /// being edited in the UI.
    CustomizationFileChanged {
        /// Relative to <vault>/config/ (e.g., "SOUL.md" or "shards/foo.md")
        path: String,
        /// New mtime as RFC3339, or empty string if the file was deleted.
        modified_at: String,
    },

    /// Recall pipeline executed — global telemetry for the Observatory v2
    /// canvas to light up the hierarchy clusters the agent just consulted.
    ///
    /// Emitted from `recall_unified` after step 4 (graph-ANN seeds) +
    /// step 5c (LCA walk). All ID lists are pre-resolved on the server
    /// so the frontend doesn't need to look anything up.
    ///
    /// Broadcast globally (no `session_id` field) because the observatory
    /// surface watches across all conversations. When no agent is doing
    /// anything, no events fire — the canvas falls back to ambient pulses.
    RecallTrace {
        /// Agent that ran the recall.
        agent_id: String,
        /// Conversation that triggered the recall, if any.
        conversation_id: Option<String>,
        /// L0 entity ids (the raw graph-ANN seeds).
        seed_entity_ids: Vec<String>,
        /// L1 (or higher) `parent_cluster_id`s of the seeds — pre-resolved
        /// so the canvas can directly pulse the corresponding aggregates.
        seed_aggregate_ids: Vec<String>,
        /// LCA of the seed entities, if the hierarchy step found one.
        /// May be a layer-1 aggregate or higher.
        lca_aggregate_id: Option<String>,
        /// Number of ScoredItems that survived ranking + made it into the
        /// final recall output. Surfaces a "this recall was meaningful"
        /// signal — large = many results, zero = nothing recalled.
        surfaced_item_count: u32,
        /// Non-secret source labels that contributed surfaced recall items.
        match_sources: Vec<String>,
        /// Non-secret ranking stages/reasons applied during recall.
        ranking_reasons: Vec<String>,
        /// Structured degraded-mode reason labels, if recall degraded.
        degraded_reasons: Vec<String>,
        /// Non-secret embedding provider/model/dimension identity used for
        /// query vectors. Never includes vectors, query text, API keys, or URLs.
        embedding_provider_identity: Option<serde_json::Value>,
        /// Non-secret SKOS expansion cues used to widen the retrieval query.
        taxonomy_expansion: Vec<serde_json::Value>,
    },
}

impl GatewayEvent {
    /// Get the agent ID for this event (if available).
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::SurfaceCreated { .. }
            | Self::SurfaceUpdated { .. }
            | Self::SurfaceDeleted { .. }
            | Self::SurfaceValidationFailed { .. } => None,
            Self::AgentStarted { agent_id, .. } => Some(agent_id),
            Self::AgentCompleted { agent_id, .. } => Some(agent_id),
            Self::AgentStopped { agent_id, .. } => Some(agent_id),
            Self::Error { agent_id, .. } => agent_id.as_deref(),
            Self::Token { agent_id, .. } => Some(agent_id),
            Self::Thinking { agent_id, .. } => Some(agent_id),
            Self::ToolCall { agent_id, .. } => Some(agent_id),
            Self::ToolResult { agent_id, .. } => Some(agent_id),
            Self::TurnComplete { agent_id, .. } => Some(agent_id),
            Self::IterationUpdate { agent_id, .. } => Some(agent_id),
            Self::ContinuationPrompt { agent_id, .. } => Some(agent_id),
            Self::Respond { .. } => None,
            Self::DelegationStarted {
                parent_agent_id, ..
            } => Some(parent_agent_id),
            Self::DelegationCompleted {
                parent_agent_id, ..
            } => Some(parent_agent_id),
            Self::MessageAdded { .. } => None,
            Self::TokenUsage { .. } => None,
            Self::Heartbeat { .. } => None,
            Self::SessionContinuationReady { root_agent_id, .. } => Some(root_agent_id),
            Self::WardChanged { .. } => None,
            Self::PlanUpdate { .. } => None,
            Self::IterationsExtended { .. } => None,
            Self::SessionTitleChanged { .. } => None,
            Self::IntentAnalysisStarted { .. } => None,
            Self::IntentAnalysisComplete { .. } => None,
            Self::IntentAnalysisSkipped { .. } => None,
            Self::CustomizationFileChanged { .. } => None,
            Self::RecallTrace { agent_id, .. } => Some(agent_id),
        }
    }

    /// Get the session ID for this event (primary routing key).
    ///
    /// All events should have a session_id. This is the primary key used
    /// for subscription routing - clients subscribe to session_id and receive
    /// all events for that session.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::SurfaceCreated { session_id, .. }
            | Self::SurfaceUpdated { session_id, .. }
            | Self::SurfaceDeleted { session_id, .. }
            | Self::SurfaceValidationFailed { session_id, .. } => Some(session_id),
            Self::AgentStarted { session_id, .. } => Some(session_id),
            Self::AgentCompleted { session_id, .. } => Some(session_id),
            Self::AgentStopped { session_id, .. } => Some(session_id),
            Self::Error { session_id, .. } => session_id.as_deref(),
            Self::Token { session_id, .. } => Some(session_id),
            Self::Thinking { session_id, .. } => Some(session_id),
            Self::ToolCall { session_id, .. } => Some(session_id),
            Self::ToolResult { session_id, .. } => Some(session_id),
            Self::TurnComplete { session_id, .. } => Some(session_id),
            Self::IterationUpdate { session_id, .. } => Some(session_id),
            Self::ContinuationPrompt { session_id, .. } => Some(session_id),
            Self::Respond { session_id, .. } => Some(session_id),
            Self::DelegationStarted { session_id, .. } => Some(session_id),
            Self::DelegationCompleted { session_id, .. } => Some(session_id),
            Self::MessageAdded { session_id, .. } => Some(session_id),
            Self::TokenUsage { session_id, .. } => Some(session_id),
            Self::Heartbeat { session_id, .. } => Some(session_id),
            Self::SessionContinuationReady { session_id, .. } => Some(session_id),
            Self::WardChanged { session_id, .. } => Some(session_id),
            Self::PlanUpdate { session_id, .. } => Some(session_id),
            Self::IterationsExtended { session_id, .. } => Some(session_id),
            Self::SessionTitleChanged { session_id, .. } => Some(session_id),
            Self::IntentAnalysisStarted { session_id, .. } => Some(session_id),
            Self::IntentAnalysisComplete { session_id, .. } => Some(session_id),
            Self::IntentAnalysisSkipped { session_id, .. } => Some(session_id),
            Self::CustomizationFileChanged { .. } => None,
            // Global telemetry — no session routing.
            Self::RecallTrace { .. } => None,
        }
    }

    /// Get the execution ID for this event (filtering key).
    ///
    /// Used for client-side filtering to show only events from a specific
    /// execution (e.g., root-only view or subagent-specific view).
    pub fn execution_id(&self) -> Option<&str> {
        match self {
            Self::SurfaceCreated { execution_id, .. }
            | Self::SurfaceUpdated { execution_id, .. }
            | Self::SurfaceDeleted { execution_id, .. }
            | Self::SurfaceValidationFailed { execution_id, .. } => Some(execution_id),
            Self::AgentStarted { execution_id, .. } => Some(execution_id),
            Self::AgentCompleted { execution_id, .. } => Some(execution_id),
            Self::AgentStopped { execution_id, .. } => Some(execution_id),
            Self::Error { execution_id, .. } => execution_id.as_deref(),
            Self::Token { execution_id, .. } => Some(execution_id),
            Self::Thinking { execution_id, .. } => Some(execution_id),
            Self::ToolCall { execution_id, .. } => Some(execution_id),
            Self::ToolResult { execution_id, .. } => Some(execution_id),
            Self::TurnComplete { execution_id, .. } => Some(execution_id),
            Self::IterationUpdate { execution_id, .. } => Some(execution_id),
            Self::ContinuationPrompt { execution_id, .. } => Some(execution_id),
            Self::Respond { execution_id, .. } => Some(execution_id),
            Self::DelegationStarted {
                parent_execution_id,
                ..
            } => Some(parent_execution_id),
            Self::DelegationCompleted {
                parent_execution_id,
                ..
            } => Some(parent_execution_id),
            Self::MessageAdded { execution_id, .. } => Some(execution_id),
            Self::TokenUsage { execution_id, .. } => Some(execution_id),
            Self::Heartbeat { execution_id, .. } => Some(execution_id),
            Self::SessionContinuationReady {
                root_execution_id, ..
            } => Some(root_execution_id),
            Self::WardChanged { execution_id, .. } => Some(execution_id),
            Self::PlanUpdate { execution_id, .. } => Some(execution_id),
            Self::IterationsExtended { execution_id, .. } => Some(execution_id),
            Self::SessionTitleChanged { .. } => None,
            Self::IntentAnalysisStarted { execution_id, .. } => Some(execution_id),
            Self::IntentAnalysisComplete { execution_id, .. } => Some(execution_id),
            Self::IntentAnalysisSkipped { execution_id, .. } => Some(execution_id),
            Self::CustomizationFileChanged { .. } => None,
            // Global telemetry — no execution routing.
            Self::RecallTrace { .. } => None,
        }
    }

    /// Get the conversation ID for this event (legacy, for backward compatibility).
    ///
    /// @deprecated Use session_id() for routing and execution_id() for filtering.
    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::SurfaceCreated { .. }
            | Self::SurfaceUpdated { .. }
            | Self::SurfaceDeleted { .. }
            | Self::SurfaceValidationFailed { .. } => None,
            Self::AgentStarted {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::AgentCompleted {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::AgentStopped {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Error {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Token {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Thinking {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::ToolCall {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::ToolResult {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::TurnComplete {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::IterationUpdate {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::ContinuationPrompt {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Respond {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::DelegationStarted {
                parent_conversation_id,
                ..
            } => parent_conversation_id.as_deref(),
            Self::DelegationCompleted {
                parent_conversation_id,
                ..
            } => parent_conversation_id.as_deref(),
            Self::MessageAdded {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::TokenUsage {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Heartbeat {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::SessionContinuationReady { session_id, .. } => Some(session_id),
            Self::WardChanged { .. } => None,
            Self::PlanUpdate {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::IterationsExtended {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::SessionTitleChanged { .. } => None,
            Self::IntentAnalysisStarted { .. } => None,
            Self::IntentAnalysisComplete { .. } => None,
            Self::IntentAnalysisSkipped { .. } => None,
            Self::CustomizationFileChanged { .. } => None,
            Self::RecallTrace {
                conversation_id, ..
            } => conversation_id.as_deref(),
        }
    }
}
