//! # WebSocket Messages
//!
//! Message types for WebSocket communication.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

// =============================================================================
// SUBSCRIPTION SCOPE
// =============================================================================

/// Subscription scope for event filtering.
///
/// Controls which events are delivered to the subscriber:
/// - `All`: All events for the session (default, backward compatible)
/// - `Session`: Root execution events + delegation lifecycle only (clean UI view)
/// - `Execution`: All events for a specific execution (debug/detail view)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionScope {
    /// All events for the session (default, backward compatible).
    #[default]
    All,
    /// Root execution events + delegation lifecycle markers only.
    /// Hides subagent internal events (tokens, tools, thinking).
    Session,
    /// All events for a specific execution ID.
    /// Use for viewing subagent activity in detail view.
    Execution(String),
}

/// Messages from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to a conversation/session's events.
    ///
    /// The `scope` parameter controls event filtering:
    /// - `All` (default): All events for backward compatibility
    /// - `Session`: Root events + delegation lifecycle only (clean chat view)
    /// - `Execution(id)`: All events for specific execution (detail view)
    Subscribe {
        conversation_id: String,
        /// Event filtering scope (defaults to All for backward compatibility)
        #[serde(default)]
        scope: SubscriptionScope,
    },

    /// Unsubscribe from a conversation's events.
    Unsubscribe { conversation_id: String },

    /// Start or continue a conversation.
    Invoke {
        /// Agent ID to invoke
        agent_id: String,
        /// Conversation ID for tracking
        conversation_id: String,
        /// User message
        message: String,
        /// Optional session ID to continue (None = new session)
        #[serde(default)]
        session_id: Option<String>,
        /// Optional metadata
        #[serde(default)]
        metadata: Option<Value>,
        /// Execution mode: "deep" (default) or "fast" (skip intent analysis)
        #[serde(default = "default_invoke_mode")]
        mode: String,
    },

    /// Stop the current execution.
    Stop { conversation_id: String },

    /// Continue after iteration limit.
    Continue {
        conversation_id: String,
        /// Additional iterations to allow (default: 25)
        #[serde(default = "default_additional_iterations")]
        additional_iterations: u32,
    },

    /// Pause a running session.
    Pause { session_id: String },

    /// Resume a paused or crashed session.
    Resume { session_id: String },

    /// Cancel a session. The conversation id scopes the live execution handle
    /// when a session has more than one historical turn.
    Cancel {
        session_id: String,
        #[serde(default)]
        conversation_id: Option<String>,
    },

    /// End a session (mark as completed).
    /// Used when user types /end, /new, or clicks +new button.
    EndSession { session_id: String },

    /// Opt in to catalog-constrained work surfaces. Older clients never send
    /// this frame and therefore retain the existing event stream unchanged.
    PresentationCapabilities { catalogs: Vec<String> },

    /// Ping for keepalive.
    Ping,
}

fn default_additional_iterations() -> u32 {
    25
}

fn default_invoke_mode() -> String {
    "deep".to_string()
}

/// Messages from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Additive native work-surface projection for capable clients only.
    SurfaceCreated {
        session_id: String,
        execution_id: String,
        surface: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
    SurfaceUpdated {
        session_id: String,
        execution_id: String,
        surface: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
    SurfaceDeleted {
        session_id: String,
        execution_id: String,
        surface_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
    /// Bounded validation telemetry; no descriptor payload is exposed.
    SurfaceValidationFailed {
        session_id: String,
        execution_id: String,
        surface_id: String,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
    // =========================================================================
    // SUBSCRIPTION RESPONSES
    // =========================================================================
    /// Subscription confirmed.
    ///
    /// Includes `root_execution_ids` for Session scope subscriptions to enable
    /// client-side fallback filtering if needed.
    Subscribed {
        conversation_id: String,
        current_sequence: u64,
        /// Root execution IDs for this session (for Session scope).
        /// Allows client to do fallback filtering if needed.
        #[serde(skip_serializing_if = "Option::is_none")]
        root_execution_ids: Option<HashSet<String>>,
    },

    /// Unsubscription confirmed.
    Unsubscribed { conversation_id: String },

    /// Subscription error.
    SubscriptionError {
        conversation_id: String,
        code: SubscriptionErrorCode,
        message: String,
    },

    // =========================================================================
    // CONVERSATION EVENTS (with sequence numbers)
    // All events include session_id and execution_id for client-side filtering
    // =========================================================================
    /// Agent started executing.
    AgentStarted {
        agent_id: String,
        /// Session ID for subscription routing.
        session_id: String,
        /// Execution ID for filtering (root vs subagent).
        execution_id: String,
        /// Legacy conversation ID for backward compatibility.
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Agent completed execution.
    AgentCompleted {
        agent_id: String,
        session_id: String,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Agent was stopped.
    AgentStopped {
        agent_id: String,
        session_id: String,
        execution_id: String,
        iteration: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Streaming text token.
    Token {
        session_id: String,
        execution_id: String,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Thinking/reasoning content.
    Thinking {
        session_id: String,
        execution_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Tool call started.
    ToolCall {
        session_id: String,
        execution_id: String,
        tool_call_id: String,
        tool: String,
        args: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Tool call result.
    ToolResult {
        session_id: String,
        execution_id: String,
        tool_call_id: String,
        result: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Turn completed.
    TurnComplete {
        session_id: String,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        final_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Error occurred.
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        execution_id: Option<String>,
        code: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Iteration update.
    Iteration {
        session_id: String,
        execution_id: String,
        current: u32,
        max: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Continuation prompt (max iterations reached).
    ContinuationPrompt {
        session_id: String,
        execution_id: String,
        iteration: u32,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// New message added to conversation (for delegation callbacks, system messages).
    /// Frontend should refresh conversation to show the new message.
    MessageAdded {
        session_id: String,
        execution_id: String,
        role: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Execution heartbeat — execution alive, no data flowing.
    Heartbeat {
        session_id: String,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Token usage update for real-time metrics.
    TokenUsage {
        session_id: String,
        execution_id: String,
        tokens_in: u64,
        tokens_out: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Delegation started - agent delegated work to a subagent.
    DelegationStarted {
        session_id: String,
        parent_execution_id: String,
        child_execution_id: String,
        parent_agent_id: String,
        child_agent_id: String,
        task: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        child_conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Delegation completed - subagent finished and returned result.
    DelegationCompleted {
        session_id: String,
        parent_execution_id: String,
        child_execution_id: String,
        parent_agent_id: String,
        child_agent_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        child_conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    // =========================================================================
    // GLOBAL/CONNECTION MESSAGES (no sequence numbers)
    // =========================================================================
    /// Pong response.
    Pong,

    /// Connected successfully.
    Connected { session_id: String },

    /// Session paused.
    SessionPaused { session_id: String },

    /// Session resumed.
    SessionResumed { session_id: String },

    /// Session cancelled.
    SessionCancelled { session_id: String },

    /// Session ended (completed by user request).
    SessionEnded { session_id: String },

    /// Sent immediately after invoke is accepted, before execution starts.
    /// Allows client to learn session_id without waiting for AgentStarted.
    InvokeAccepted {
        session_id: String,
        conversation_id: String,
    },

    /// Agent switched to a different ward (project directory).
    WardChanged {
        session_id: String,
        execution_id: String,
        ward_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Executor auto-extended iterations because the agent is making progress.
    IterationsExtended {
        session_id: String,
        execution_id: String,
        iterations_used: u32,
        iterations_added: u32,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Agent's plan was updated via update_plan tool.
    PlanUpdate {
        session_id: String,
        execution_id: String,
        plan: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        explanation: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Session title changed by runtime title derivation or legacy log replay.
    SessionTitleChanged {
        session_id: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Intent analysis started for a root session (shows "Analyzing..." in UI).
    IntentAnalysisStarted {
        session_id: String,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Intent analysis completed for a root session.
    IntentAnalysisComplete {
        session_id: String,
        execution_id: String,
        primary_intent: String,
        hidden_intents: Vec<String>,
        recommended_skills: Vec<String>,
        recommended_agents: Vec<String>,
        ward_recommendation: Value,
        execution_strategy: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Intent analysis skipped (already analyzed in this session).
    IntentAnalysisSkipped {
        session_id: String,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },

    /// Recall pipeline executed — broadcast globally so the Observatory v2
    /// canvas can light up the hierarchy clusters the agent just consulted.
    /// See `GatewayEvent::RecallTrace` for field semantics.
    RecallTrace {
        agent_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        seed_entity_ids: Vec<String>,
        seed_aggregate_ids: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lca_aggregate_id: Option<String>,
        surfaced_item_count: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        taxonomy_expansion: Vec<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
}

/// Error codes for subscription errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionErrorCode {
    NotFound,
    LimitExceeded,
    ServerError,
}

impl ServerMessage {
    /// Create an error message.
    pub fn error(conversation_id: Option<String>, code: &str, message: &str) -> Self {
        Self::Error {
            session_id: None,
            execution_id: None,
            conversation_id,
            code: code.to_string(),
            message: message.to_string(),
            seq: None,
        }
    }

    /// Create a token message.
    pub fn token(
        session_id: &str,
        execution_id: &str,
        conversation_id: Option<String>,
        delta: &str,
    ) -> Self {
        Self::Token {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            conversation_id,
            delta: delta.to_string(),
            seq: None,
        }
    }

    /// Create a turn complete message.
    pub fn turn_complete(
        session_id: &str,
        execution_id: &str,
        conversation_id: Option<String>,
        final_message: Option<String>,
    ) -> Self {
        Self::TurnComplete {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            conversation_id,
            final_message,
            seq: None,
        }
    }

    /// Create a subscription error message.
    pub fn subscription_error(
        conversation_id: &str,
        code: SubscriptionErrorCode,
        message: &str,
    ) -> Self {
        Self::SubscriptionError {
            conversation_id: conversation_id.to_string(),
            code,
            message: message.to_string(),
        }
    }

    /// Get the conversation_id for this message, if any.
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
            Self::Error {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Iteration {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::ContinuationPrompt {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::MessageAdded {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::TokenUsage {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Heartbeat {
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
            Self::Subscribed {
                conversation_id, ..
            } => Some(conversation_id),
            Self::Unsubscribed {
                conversation_id, ..
            } => Some(conversation_id),
            Self::SubscriptionError {
                conversation_id, ..
            } => Some(conversation_id),
            Self::InvokeAccepted {
                conversation_id, ..
            } => Some(conversation_id),
            Self::WardChanged { .. } => None,
            Self::IterationsExtended {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::PlanUpdate {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::SessionTitleChanged { .. } => None,
            Self::IntentAnalysisStarted { .. } => None,
            Self::IntentAnalysisComplete { .. } => None,
            Self::IntentAnalysisSkipped { .. } => None,
            Self::RecallTrace {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::Pong
            | Self::Connected { .. }
            | Self::SessionPaused { .. }
            | Self::SessionResumed { .. }
            | Self::SessionCancelled { .. }
            | Self::SessionEnded { .. } => None,
        }
    }

    /// Return a copy of this message with the sequence number set.
    pub fn with_sequence(self, seq: u64) -> Self {
        match self {
            Self::SurfaceCreated {
                session_id,
                execution_id,
                surface,
                ..
            } => Self::SurfaceCreated {
                session_id,
                execution_id,
                surface,
                seq: Some(seq),
            },
            Self::SurfaceUpdated {
                session_id,
                execution_id,
                surface,
                ..
            } => Self::SurfaceUpdated {
                session_id,
                execution_id,
                surface,
                seq: Some(seq),
            },
            Self::SurfaceDeleted {
                session_id,
                execution_id,
                surface_id,
                ..
            } => Self::SurfaceDeleted {
                session_id,
                execution_id,
                surface_id,
                seq: Some(seq),
            },
            Self::SurfaceValidationFailed {
                session_id,
                execution_id,
                surface_id,
                reason,
                ..
            } => Self::SurfaceValidationFailed {
                session_id,
                execution_id,
                surface_id,
                reason,
                seq: Some(seq),
            },
            Self::AgentStarted {
                agent_id,
                session_id,
                execution_id,
                conversation_id,
                ..
            } => Self::AgentStarted {
                agent_id,
                session_id,
                execution_id,
                conversation_id,
                seq: Some(seq),
            },
            Self::AgentCompleted {
                agent_id,
                session_id,
                execution_id,
                conversation_id,
                result,
                ..
            } => Self::AgentCompleted {
                agent_id,
                session_id,
                execution_id,
                conversation_id,
                result,
                seq: Some(seq),
            },
            Self::AgentStopped {
                agent_id,
                session_id,
                execution_id,
                conversation_id,
                iteration,
                ..
            } => Self::AgentStopped {
                agent_id,
                session_id,
                execution_id,
                conversation_id,
                iteration,
                seq: Some(seq),
            },
            Self::Token {
                session_id,
                execution_id,
                conversation_id,
                delta,
                ..
            } => Self::Token {
                session_id,
                execution_id,
                conversation_id,
                delta,
                seq: Some(seq),
            },
            Self::Thinking {
                session_id,
                execution_id,
                conversation_id,
                content,
                ..
            } => Self::Thinking {
                session_id,
                execution_id,
                conversation_id,
                content,
                seq: Some(seq),
            },
            Self::ToolCall {
                session_id,
                execution_id,
                conversation_id,
                tool_call_id,
                tool,
                args,
                ..
            } => Self::ToolCall {
                session_id,
                execution_id,
                conversation_id,
                tool_call_id,
                tool,
                args,
                seq: Some(seq),
            },
            Self::ToolResult {
                session_id,
                execution_id,
                conversation_id,
                tool_call_id,
                result,
                error,
                ..
            } => Self::ToolResult {
                session_id,
                execution_id,
                conversation_id,
                tool_call_id,
                result,
                error,
                seq: Some(seq),
            },
            Self::TurnComplete {
                session_id,
                execution_id,
                conversation_id,
                final_message,
                ..
            } => Self::TurnComplete {
                session_id,
                execution_id,
                conversation_id,
                final_message,
                seq: Some(seq),
            },
            Self::Error {
                session_id,
                execution_id,
                conversation_id,
                code,
                message,
                ..
            } => Self::Error {
                session_id,
                execution_id,
                conversation_id,
                code,
                message,
                seq: Some(seq),
            },
            Self::Iteration {
                session_id,
                execution_id,
                conversation_id,
                current,
                max,
                ..
            } => Self::Iteration {
                session_id,
                execution_id,
                conversation_id,
                current,
                max,
                seq: Some(seq),
            },
            Self::ContinuationPrompt {
                session_id,
                execution_id,
                conversation_id,
                iteration,
                message,
                ..
            } => Self::ContinuationPrompt {
                session_id,
                execution_id,
                conversation_id,
                iteration,
                message,
                seq: Some(seq),
            },
            Self::MessageAdded {
                session_id,
                execution_id,
                conversation_id,
                role,
                content,
                ..
            } => Self::MessageAdded {
                session_id,
                execution_id,
                conversation_id,
                role,
                content,
                seq: Some(seq),
            },
            Self::TokenUsage {
                session_id,
                execution_id,
                conversation_id,
                tokens_in,
                tokens_out,
                ..
            } => Self::TokenUsage {
                session_id,
                execution_id,
                conversation_id,
                tokens_in,
                tokens_out,
                seq: Some(seq),
            },
            Self::Heartbeat {
                session_id,
                execution_id,
                conversation_id,
                ..
            } => Self::Heartbeat {
                session_id,
                execution_id,
                conversation_id,
                seq: Some(seq),
            },
            Self::DelegationStarted {
                session_id,
                parent_execution_id,
                child_execution_id,
                parent_agent_id,
                child_agent_id,
                task,
                parent_conversation_id,
                child_conversation_id,
                ..
            } => Self::DelegationStarted {
                session_id,
                parent_execution_id,
                child_execution_id,
                parent_agent_id,
                child_agent_id,
                task,
                parent_conversation_id,
                child_conversation_id,
                seq: Some(seq),
            },
            Self::DelegationCompleted {
                session_id,
                parent_execution_id,
                child_execution_id,
                parent_agent_id,
                child_agent_id,
                result,
                parent_conversation_id,
                child_conversation_id,
                ..
            } => Self::DelegationCompleted {
                session_id,
                parent_execution_id,
                child_execution_id,
                parent_agent_id,
                child_agent_id,
                result,
                parent_conversation_id,
                child_conversation_id,
                seq: Some(seq),
            },
            Self::WardChanged {
                session_id,
                execution_id,
                ward_id,
                ..
            } => Self::WardChanged {
                session_id,
                execution_id,
                ward_id,
                seq: Some(seq),
            },
            Self::IterationsExtended {
                session_id,
                execution_id,
                iterations_used,
                iterations_added,
                reason,
                conversation_id,
                ..
            } => Self::IterationsExtended {
                session_id,
                execution_id,
                iterations_used,
                iterations_added,
                reason,
                conversation_id,
                seq: Some(seq),
            },
            Self::PlanUpdate {
                session_id,
                execution_id,
                plan,
                explanation,
                conversation_id,
                ..
            } => Self::PlanUpdate {
                session_id,
                execution_id,
                plan,
                explanation,
                conversation_id,
                seq: Some(seq),
            },
            Self::SessionTitleChanged {
                session_id, title, ..
            } => Self::SessionTitleChanged {
                session_id,
                title,
                seq: Some(seq),
            },
            Self::IntentAnalysisStarted {
                session_id,
                execution_id,
                seq: _,
            } => Self::IntentAnalysisStarted {
                session_id,
                execution_id,
                seq: Some(seq),
            },
            Self::IntentAnalysisComplete {
                session_id,
                execution_id,
                primary_intent,
                hidden_intents,
                recommended_skills,
                recommended_agents,
                ward_recommendation,
                execution_strategy,
                ..
            } => Self::IntentAnalysisComplete {
                session_id,
                execution_id,
                primary_intent,
                hidden_intents,
                recommended_skills,
                recommended_agents,
                ward_recommendation,
                execution_strategy,
                seq: Some(seq),
            },
            Self::IntentAnalysisSkipped {
                session_id,
                execution_id,
                seq: _,
            } => Self::IntentAnalysisSkipped {
                session_id,
                execution_id,
                seq: Some(seq),
            },
            // Messages without sequence numbers pass through unchanged
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_message_deserialize() {
        let json = r#"{"type": "invoke", "agent_id": "root", "conversation_id": "123", "message": "Hello"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Invoke {
                agent_id,
                conversation_id,
                message,
                ..
            } => {
                assert_eq!(agent_id, "root");
                assert_eq!(conversation_id, "123");
                assert_eq!(message, "Hello");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_server_message_serialize() {
        let msg = ServerMessage::token("sess-1", "exec-1", Some("conv-1".to_string()), "Hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("token"));
        assert!(json.contains("sess-1"));
    }

    #[test]
    fn test_subscription_scope_default() {
        // Test that default scope is All (backward compatible)
        let scope: SubscriptionScope = Default::default();
        assert_eq!(scope, SubscriptionScope::All);
    }

    #[test]
    fn test_subscription_scope_deserialize_all() {
        let json = r#"{"type": "subscribe", "conversation_id": "sess-123", "scope": "all"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Subscribe {
                conversation_id,
                scope,
            } => {
                assert_eq!(conversation_id, "sess-123");
                assert_eq!(scope, SubscriptionScope::All);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_subscription_scope_deserialize_session() {
        let json = r#"{"type": "subscribe", "conversation_id": "sess-123", "scope": "session"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Subscribe {
                conversation_id,
                scope,
            } => {
                assert_eq!(conversation_id, "sess-123");
                assert_eq!(scope, SubscriptionScope::Session);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_subscription_scope_deserialize_execution() {
        let json = r#"{"type": "subscribe", "conversation_id": "sess-123", "scope": {"execution": "exec-456"}}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Subscribe {
                conversation_id,
                scope,
            } => {
                assert_eq!(conversation_id, "sess-123");
                assert_eq!(scope, SubscriptionScope::Execution("exec-456".to_string()));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_subscription_scope_default_when_missing() {
        // Test backward compatibility - scope defaults to All when not provided
        let json = r#"{"type": "subscribe", "conversation_id": "sess-123"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Subscribe {
                conversation_id,
                scope,
            } => {
                assert_eq!(conversation_id, "sess-123");
                assert_eq!(scope, SubscriptionScope::All); // Default
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn presentation_capabilities_and_surface_lifecycle_are_additive() {
        let capabilities: ClientMessage = serde_json::from_str(
            r#"{"type":"presentation_capabilities","catalogs":["zbot/work-surface/v1"]}"#,
        )
        .unwrap();
        assert!(matches!(
            capabilities,
            ClientMessage::PresentationCapabilities { .. }
        ));
        let message = ServerMessage::SurfaceDeleted {
            session_id: "s".into(),
            execution_id: "e".into(),
            surface_id: "surface".into(),
            seq: None,
        };
        let json = serde_json::to_value(message.with_sequence(3)).unwrap();
        assert_eq!(json["type"], "surface_deleted");
        assert_eq!(json["seq"], 3);
    }
}
