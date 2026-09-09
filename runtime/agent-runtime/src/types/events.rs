// ============================================================================
// STREAM EVENT TYPES
// Events emitted during agent execution
// ============================================================================

use agent_surfaces::WorkSurface;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Events emitted during agent execution
///
/// These events are streamed to the frontend/application layer
/// to provide real-time feedback during agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    /// Metadata about the execution (agent info, model, etc.)
    #[serde(rename = "metadata")]
    Metadata {
        timestamp: u64,
        agent_id: String,
        model: String,
        provider: String,
    },

    /// A token from the LLM response
    #[serde(rename = "token")]
    Token { timestamp: u64, content: String },

    /// Reasoning/thinking content from the LLM
    #[serde(rename = "reasoning")]
    Reasoning { timestamp: u64, content: String },

    /// A tool call has started
    #[serde(rename = "tool_call_start")]
    ToolCallStart {
        timestamp: u64,
        tool_id: String,
        tool_name: String,
        args: Value,
    },

    /// A tool call has completed
    #[serde(rename = "tool_call_end")]
    ToolCallEnd {
        timestamp: u64,
        tool_id: String,
        tool_name: String,
        args: Value,
    },

    /// Result from a tool execution
    #[serde(rename = "tool_result")]
    ToolResult {
        timestamp: u64,
        tool_id: String,
        result: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_result: Option<String>,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<i64>,
    },

    /// Execution is complete
    #[serde(rename = "done")]
    Done {
        timestamp: u64,
        final_message: String,
        token_count: usize,
    },

    /// An error occurred during execution
    #[serde(rename = "error")]
    Error {
        timestamp: u64,
        error: String,
        recoverable: bool,
    },

    // ========================================================================
    // GENERATIVE UI EVENTS
    // ========================================================================
    /// Request to display content to the user
    #[serde(rename = "show_content")]
    ShowContent {
        timestamp: u64,
        content_type: String,
        title: String,
        content: String,
        metadata: Option<Value>,
        file_path: Option<String>,
        is_attachment: Option<bool>,
        base64: Option<bool>,
    },

    /// Request to input from the user
    #[serde(rename = "request_input")]
    RequestInput {
        timestamp: u64,
        form_id: String,
        form_type: String,
        title: String,
        description: Option<String>,
        schema: Value,
        submit_button: Option<String>,
    },

    /// A bounded, declarative work surface. Gateway validation determines
    /// whether it is published; normal execution never depends on it.
    #[serde(rename = "work_surface")]
    WorkSurface {
        timestamp: u64,
        surface: WorkSurface,
    },
    #[serde(rename = "work_surface_updated")]
    WorkSurfaceUpdated {
        timestamp: u64,
        surface: WorkSurface,
    },
    #[serde(rename = "work_surface_deleted")]
    WorkSurfaceDeleted { timestamp: u64, surface_id: String },

    // ========================================================================
    // ACTION EVENTS
    // ========================================================================
    /// Respond action from the respond tool.
    /// Signals that a response should be sent to the originating hook.
    #[serde(rename = "action_respond")]
    ActionRespond {
        timestamp: u64,
        message: String,
        format: String,
        conversation_id: Option<String>,
        session_id: Option<String>,
        /// Artifacts declared by the agent in its response.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifacts: Vec<agent_primitives::event::ArtifactDeclaration>,
    },

    /// Delegate action from the delegate tool.
    /// Signals that a task should be delegated to a subagent.
    #[serde(rename = "action_delegate")]
    ActionDelegate {
        timestamp: u64,
        agent_id: String,
        task: String,
        context: Option<Value>,
        wait_for_result: bool,
        max_iterations: Option<u32>,
        output_schema: Option<Value>,
        skills: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_assignment: Option<agent_primitives::event::AgentCapabilityAssignment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        planning_capability_catalog: Option<Value>,
        complexity: Option<String>,
        mode: Option<String>,
        parallel: bool,
        child_execution_id: Option<String>,
    },

    /// Plan update action from the `update_plan` tool.
    /// Signals that the agent's plan has been updated.
    #[serde(rename = "action_plan_update")]
    ActionPlanUpdate {
        timestamp: u64,
        plan: Value,
        explanation: Option<String>,
    },

    // ========================================================================
    // METRICS EVENTS
    // ========================================================================
    /// Token usage update after an LLM call.
    /// Cumulative counts of tokens consumed in the session.
    #[serde(rename = "token_update")]
    TokenUpdate {
        timestamp: u64,
        /// Cumulative input tokens (prompt tokens)
        tokens_in: u64,
        /// Cumulative output tokens (completion tokens)
        tokens_out: u64,
    },

    // ========================================================================
    // CHECKPOINT EVENTS
    // ========================================================================
    /// Execution heartbeat — emitted during silent phases (e.g., LLM reasoning)
    /// to signal the execution is still alive.
    #[serde(rename = "heartbeat")]
    Heartbeat { timestamp: u64 },

    /// Execution context state for checkpoint persistence.
    ///
    /// Emitted at the end of execution (after Done) to allow the gateway
    /// to persist the context state for session resumption. Contains skill
    /// tracking information and other tool context state.
    #[serde(rename = "context_state")]
    ContextState {
        /// Timestamp when state was captured
        timestamp: u64,
        /// Serialized tool context state (skill graph, loaded skills, etc.)
        state: Value,
    },

    // ========================================================================
    // WARD EVENTS
    // ========================================================================
    /// Agent switched to a different ward (project directory).
    #[serde(rename = "ward_changed")]
    WardChanged {
        timestamp: u64,
        /// The ward the agent switched to
        ward_id: String,
    },

    /// Executor auto-extended iterations because the agent is making progress.
    #[serde(rename = "iterations_extended")]
    IterationsExtended {
        timestamp: u64,
        /// Total iterations used so far
        iterations_used: u32,
        /// Additional iterations granted
        iterations_added: u32,
        /// Human-readable reason for extension
        reason: String,
    },

    // ========================================================================
    // SESSION EVENTS
    // ========================================================================
    /// Session title changed by runtime title derivation or legacy log replay.
    #[serde(rename = "session_title_changed")]
    SessionTitleChanged {
        timestamp: u64,
        /// The new session title
        title: String,
    },
}

impl StreamEvent {
    /// Get the timestamp for this event
    #[must_use]
    pub const fn timestamp(&self) -> u64 {
        match self {
            Self::Metadata { timestamp, .. }
            | Self::Token { timestamp, .. }
            | Self::Reasoning { timestamp, .. }
            | Self::ToolCallStart { timestamp, .. }
            | Self::ToolCallEnd { timestamp, .. }
            | Self::ToolResult { timestamp, .. }
            | Self::Done { timestamp, .. }
            | Self::Error { timestamp, .. }
            | Self::ShowContent { timestamp, .. }
            | Self::RequestInput { timestamp, .. }
            | Self::WorkSurface { timestamp, .. }
            | Self::WorkSurfaceUpdated { timestamp, .. }
            | Self::WorkSurfaceDeleted { timestamp, .. }
            | Self::ActionRespond { timestamp, .. }
            | Self::ActionDelegate { timestamp, .. }
            | Self::ActionPlanUpdate { timestamp, .. }
            | Self::TokenUpdate { timestamp, .. }
            | Self::Heartbeat { timestamp, .. }
            | Self::ContextState { timestamp, .. }
            | Self::WardChanged { timestamp, .. }
            | Self::IterationsExtended { timestamp, .. }
            | Self::SessionTitleChanged { timestamp, .. } => *timestamp,
        }
    }

    /// Check if this event is a terminal event (execution complete)
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

/// Helper function to get current timestamp in milliseconds
#[must_use]
pub fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_event_serialization() {
        let event = StreamEvent::Token {
            timestamp: 12345,
            content: "Hello".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"token\""));
    }

    #[test]
    fn test_terminal_event_detection() {
        assert!(StreamEvent::Done {
            timestamp: 0,
            final_message: String::new(),
            token_count: 0
        }
        .is_terminal());

        assert!(StreamEvent::Error {
            timestamp: 0,
            error: String::new(),
            recoverable: false
        }
        .is_terminal());

        assert!(!StreamEvent::Token {
            timestamp: 0,
            content: String::new()
        }
        .is_terminal());
    }

    #[test]
    fn timestamp_returns_for_every_variant() {
        let cases: Vec<StreamEvent> = vec![
            StreamEvent::Metadata {
                timestamp: 1,
                agent_id: "a".into(),
                model: "m".into(),
                provider: "p".into(),
            },
            StreamEvent::Token {
                timestamp: 2,
                content: "t".into(),
            },
            StreamEvent::Reasoning {
                timestamp: 3,
                content: "r".into(),
            },
            StreamEvent::ToolCallStart {
                timestamp: 4,
                tool_id: "id".into(),
                tool_name: "n".into(),
                args: Value::Null,
            },
            StreamEvent::ToolCallEnd {
                timestamp: 5,
                tool_id: "id".into(),
                tool_name: "n".into(),
                args: Value::Null,
            },
            StreamEvent::ToolResult {
                timestamp: 6,
                tool_id: "id".into(),
                result: "r".into(),
                context_result: Some("context r".into()),
                error: None,
                duration_ms: Some(10),
            },
            StreamEvent::Done {
                timestamp: 7,
                final_message: "f".into(),
                token_count: 0,
            },
            StreamEvent::Error {
                timestamp: 8,
                error: "e".into(),
                recoverable: true,
            },
            StreamEvent::ShowContent {
                timestamp: 9,
                content_type: "ct".into(),
                title: "t".into(),
                content: "c".into(),
                metadata: None,
                file_path: None,
                is_attachment: None,
                base64: None,
            },
            StreamEvent::RequestInput {
                timestamp: 10,
                form_id: "f".into(),
                form_type: "t".into(),
                title: "t".into(),
                description: None,
                schema: Value::Null,
                submit_button: None,
            },
            StreamEvent::ActionRespond {
                timestamp: 11,
                message: "m".into(),
                format: "text".into(),
                conversation_id: None,
                session_id: None,
                artifacts: vec![],
            },
            StreamEvent::ActionDelegate {
                timestamp: 12,
                agent_id: "a".into(),
                task: "t".into(),
                context: None,
                wait_for_result: false,
                max_iterations: None,
                output_schema: None,
                skills: vec![],
                capability_assignment: None,
                planning_capability_catalog: None,
                complexity: None,
                mode: None,
                parallel: false,
                child_execution_id: None,
            },
            StreamEvent::ActionPlanUpdate {
                timestamp: 13,
                plan: Value::Null,
                explanation: None,
            },
            StreamEvent::TokenUpdate {
                timestamp: 14,
                tokens_in: 0,
                tokens_out: 0,
            },
            StreamEvent::Heartbeat { timestamp: 15 },
            StreamEvent::ContextState {
                timestamp: 16,
                state: Value::Null,
            },
            StreamEvent::WardChanged {
                timestamp: 17,
                ward_id: "w".into(),
            },
            StreamEvent::IterationsExtended {
                timestamp: 18,
                iterations_used: 1,
                iterations_added: 1,
                reason: "r".into(),
            },
            StreamEvent::SessionTitleChanged {
                timestamp: 19,
                title: "t".into(),
            },
        ];
        for (expected, ev) in cases.iter().enumerate() {
            // expected starts at 0 but we used 1.. — adjust
            assert_eq!(ev.timestamp(), (expected + 1) as u64);
            assert_eq!(
                ev.is_terminal(),
                matches!(ev, StreamEvent::Done { .. } | StreamEvent::Error { .. })
            );
        }
    }

    #[test]
    fn current_timestamp_is_nonzero() {
        let t = current_timestamp();
        assert!(t > 0);
    }
}
