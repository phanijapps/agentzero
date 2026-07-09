//! # Execution Events
//!
//! Event conversion and emission helpers for agent execution.

use agent_runtime::StreamEvent;
use gateway_events::GatewayEvent;

/// Convert an agent runtime stream event to a gateway event.
///
/// This function maps the internal executor events to gateway-level events
/// that can be broadcast to connected clients.
///
/// Returns `None` for internal events that should not be broadcast to the UI
/// (e.g., `ContextState` for checkpoint persistence).
pub fn convert_stream_event(
    event: StreamEvent,
    agent_id: &str,
    conversation_id: &str,
    session_id: &str,
    execution_id: &str,
) -> Option<GatewayEvent> {
    match event {
        StreamEvent::Metadata { .. } => Some(GatewayEvent::AgentStarted {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::Token { content, .. } => Some(GatewayEvent::Token {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            delta: content,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::Reasoning { content, .. } => Some(GatewayEvent::Thinking {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            content,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::ToolCallStart {
            tool_id, tool_name, args, ..
        } => Some(GatewayEvent::ToolCall {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            tool_id,
            tool_name,
            args,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::ToolResult {
            tool_id, result, error, ..
        } => Some(GatewayEvent::ToolResult {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            tool_id,
            result,
            error,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::Done { final_message, .. } => Some(GatewayEvent::TurnComplete {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            message: final_message,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::Error { error, .. } => Some(GatewayEvent::Error {
            agent_id: Some(agent_id.to_string()),
            session_id: Some(session_id.to_string()),
            execution_id: Some(execution_id.to_string()),
            message: error,
            conversation_id: Some(conversation_id.to_string()),
        }),
        // Action events from tools
        StreamEvent::ActionRespond {
            message,
            session_id: respond_session_id,
            ..  // artifacts handled by stream.rs before conversion
        } => Some(GatewayEvent::Respond {
            session_id: respond_session_id.unwrap_or_else(|| session_id.to_string()),
            execution_id: execution_id.to_string(),
            message,
            conversation_id: Some(conversation_id.to_string()),
        }),
        // ActionDelegate is handled by the runner/delegation system directly,
        // which emits DelegationStarted with proper IDs. Don't emit here to avoid
        // duplicate events or confusing the UI.
        StreamEvent::ActionDelegate { .. } => None,
        // Heartbeat signals execution is alive during silent phases (e.g., LLM reasoning).
        StreamEvent::Heartbeat { .. } => Some(GatewayEvent::Heartbeat {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            conversation_id: Some(conversation_id.to_string()),
        }),
        // ContextState is internal state for checkpoint persistence - don't emit to UI.
        StreamEvent::ContextState { .. } => None,
        // Ward changed - agent switched to a different project directory.
        StreamEvent::WardChanged { ward_id, .. } => Some(GatewayEvent::WardChanged {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            ward_id,
        }),
        // IterationsExtended — auto-extension event from executor
        StreamEvent::IterationsExtended {
            iterations_used, iterations_added, reason, ..
        } => Some(GatewayEvent::IterationsExtended {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            iterations_used,
            iterations_added,
            reason,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::ActionPlanUpdate { plan, explanation, .. } => Some(GatewayEvent::PlanUpdate {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            plan,
            explanation,
            conversation_id: Some(conversation_id.to_string()),
        }),
        StreamEvent::SessionTitleChanged { title, .. } => Some(GatewayEvent::SessionTitleChanged {
            session_id: session_id.to_string(),
            title,
        }),
        // No gateway equivalents — intentionally not broadcast:
        //   ToolCallEnd: pair with ToolResult, redundant downstream.
        //   ShowContent/RequestInput: generative-UI events handled elsewhere.
        //   TokenUpdate: consumed by the batch writer, not the UI stream.
        // Listed explicitly so adding a new StreamEvent variant is a compile
        // error here instead of a silent `None`.
        StreamEvent::ToolCallEnd { .. }
        | StreamEvent::ShowContent { .. }
        | StreamEvent::RequestInput { .. }
        | StreamEvent::TokenUpdate { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(event: StreamEvent) -> Option<GatewayEvent> {
        convert_stream_event(event, "agent-1", "conv-1", "session-1", "exec-1")
    }

    // --------------------------------------------------------------------
    // Variants that MAP to a GatewayEvent
    // --------------------------------------------------------------------

    #[test]
    fn metadata_maps_to_agent_started() {
        let out = convert(StreamEvent::Metadata {
            timestamp: 0,
            agent_id: "ignored-runtime-agent".into(),
            model: "gpt-4".into(),
            provider: "openai".into(),
        });
        let Some(GatewayEvent::AgentStarted {
            agent_id,
            session_id,
            execution_id,
            conversation_id,
        }) = out
        else {
            panic!("expected AgentStarted, got {out:?}");
        };
        // Uses the outer agent_id argument, not the one inside the event.
        assert_eq!(agent_id, "agent-1");
        assert_eq!(session_id, "session-1");
        assert_eq!(execution_id, "exec-1");
        assert_eq!(conversation_id.as_deref(), Some("conv-1"));
    }

    #[test]
    fn token_maps_to_token_delta() {
        let out = convert(StreamEvent::Token {
            timestamp: 0,
            content: "Hello".into(),
        });
        let Some(GatewayEvent::Token {
            agent_id,
            delta,
            conversation_id,
            ..
        }) = out
        else {
            panic!("expected Token, got {out:?}");
        };
        assert_eq!(agent_id, "agent-1");
        assert_eq!(delta, "Hello");
        assert_eq!(conversation_id.as_deref(), Some("conv-1"));
    }

    #[test]
    fn reasoning_maps_to_thinking() {
        let out = convert(StreamEvent::Reasoning {
            timestamp: 0,
            content: "let me think".into(),
        });
        let Some(GatewayEvent::Thinking { content, .. }) = out else {
            panic!("expected Thinking, got {out:?}");
        };
        assert_eq!(content, "let me think");
    }

    #[test]
    fn tool_call_start_maps_to_tool_call() {
        let out = convert(StreamEvent::ToolCallStart {
            timestamp: 0,
            tool_id: "t1".into(),
            tool_name: "search".into(),
            args: serde_json::json!({"q": "rust"}),
        });
        let Some(GatewayEvent::ToolCall {
            tool_id,
            tool_name,
            args,
            ..
        }) = out
        else {
            panic!("expected ToolCall, got {out:?}");
        };
        assert_eq!(tool_id, "t1");
        assert_eq!(tool_name, "search");
        assert_eq!(args, serde_json::json!({"q": "rust"}));
    }

    #[test]
    fn tool_result_maps_to_tool_result() {
        let out = convert(StreamEvent::ToolResult {
            timestamp: 0,
            tool_id: "t1".into(),
            result: "42".into(),
            context_result: None,
            error: Some("soft fail".into()),
            duration_ms: Some(12),
        });
        let Some(GatewayEvent::ToolResult {
            tool_id,
            result,
            error,
            ..
        }) = out
        else {
            panic!("expected ToolResult, got {out:?}");
        };
        assert_eq!(tool_id, "t1");
        assert_eq!(result, "42");
        assert_eq!(error.as_deref(), Some("soft fail"));
    }

    #[test]
    fn done_maps_to_turn_complete() {
        let out = convert(StreamEvent::Done {
            timestamp: 0,
            final_message: "done!".into(),
            token_count: 12,
        });
        let Some(GatewayEvent::TurnComplete { message, .. }) = out else {
            panic!("expected TurnComplete, got {out:?}");
        };
        assert_eq!(message, "done!");
    }

    #[test]
    fn error_maps_to_error_with_optional_ids() {
        let out = convert(StreamEvent::Error {
            timestamp: 0,
            error: "boom".into(),
            recoverable: false,
        });
        let Some(GatewayEvent::Error {
            agent_id,
            session_id,
            execution_id,
            message,
            ..
        }) = out
        else {
            panic!("expected Error, got {out:?}");
        };
        assert_eq!(agent_id.as_deref(), Some("agent-1"));
        assert_eq!(session_id.as_deref(), Some("session-1"));
        assert_eq!(execution_id.as_deref(), Some("exec-1"));
        assert_eq!(message, "boom");
    }

    #[test]
    fn action_respond_keeps_outer_session_when_inner_is_none() {
        let out = convert(StreamEvent::ActionRespond {
            timestamp: 0,
            message: "hi".into(),
            format: "markdown".into(),
            conversation_id: None,
            session_id: None,
            artifacts: vec![],
        });
        let Some(GatewayEvent::Respond {
            session_id,
            message,
            ..
        }) = out
        else {
            panic!("expected Respond, got {out:?}");
        };
        assert_eq!(session_id, "session-1");
        assert_eq!(message, "hi");
    }

    #[test]
    fn action_respond_inner_session_id_overrides_outer() {
        let out = convert(StreamEvent::ActionRespond {
            timestamp: 0,
            message: "hi".into(),
            format: "markdown".into(),
            conversation_id: None,
            session_id: Some("inner-session".into()),
            artifacts: vec![],
        });
        let Some(GatewayEvent::Respond { session_id, .. }) = out else {
            panic!("expected Respond, got {out:?}");
        };
        assert_eq!(session_id, "inner-session");
    }

    #[test]
    fn ward_changed_maps_through() {
        let out = convert(StreamEvent::WardChanged {
            timestamp: 0,
            ward_id: "maritime".into(),
        });
        let Some(GatewayEvent::WardChanged { ward_id, .. }) = out else {
            panic!("expected WardChanged, got {out:?}");
        };
        assert_eq!(ward_id, "maritime");
    }

    #[test]
    fn iterations_extended_maps_through() {
        let out = convert(StreamEvent::IterationsExtended {
            timestamp: 0,
            iterations_used: 5,
            iterations_added: 3,
            reason: "progress".into(),
        });
        let Some(GatewayEvent::IterationsExtended {
            iterations_used,
            iterations_added,
            reason,
            ..
        }) = out
        else {
            panic!("expected IterationsExtended, got {out:?}");
        };
        assert_eq!(iterations_used, 5);
        assert_eq!(iterations_added, 3);
        assert_eq!(reason, "progress");
    }

    #[test]
    fn action_plan_update_maps_to_plan_update() {
        let out = convert(StreamEvent::ActionPlanUpdate {
            timestamp: 0,
            plan: serde_json::json!([{"step": "one"}]),
            explanation: Some("why".into()),
        });
        let Some(GatewayEvent::PlanUpdate {
            plan, explanation, ..
        }) = out
        else {
            panic!("expected PlanUpdate, got {out:?}");
        };
        assert_eq!(plan, serde_json::json!([{"step": "one"}]));
        assert_eq!(explanation.as_deref(), Some("why"));
    }

    #[test]
    fn session_title_changed_maps_through() {
        let out = convert(StreamEvent::SessionTitleChanged {
            timestamp: 0,
            title: "My Session".into(),
        });
        let Some(GatewayEvent::SessionTitleChanged { title, .. }) = out else {
            panic!("expected SessionTitleChanged, got {out:?}");
        };
        assert_eq!(title, "My Session");
    }

    #[test]
    fn heartbeat_maps_through() {
        let out = convert(StreamEvent::Heartbeat { timestamp: 1 });
        let Some(GatewayEvent::Heartbeat { session_id, .. }) = out else {
            panic!("expected Heartbeat, got {out:?}");
        };
        assert_eq!(session_id, "session-1");
    }

    // --------------------------------------------------------------------
    // Variants that are INTENTIONALLY dropped (map to None)
    // --------------------------------------------------------------------

    #[test]
    fn context_state_is_dropped() {
        assert!(convert(StreamEvent::ContextState {
            timestamp: 0,
            state: serde_json::json!({}),
        })
        .is_none());
    }

    #[test]
    fn action_delegate_is_dropped() {
        assert!(convert(StreamEvent::ActionDelegate {
            timestamp: 0,
            agent_id: "c".into(),
            task: "t".into(),
            context: None,
            wait_for_result: false,
            max_iterations: None,
            output_schema: None,
            skills: vec![],
            complexity: None,
            mode: None,
            parallel: false,
            child_execution_id: None,
        })
        .is_none());
    }

    #[test]
    fn tool_call_end_is_dropped() {
        assert!(convert(StreamEvent::ToolCallEnd {
            timestamp: 0,
            tool_id: "t1".into(),
            tool_name: "search".into(),
            args: serde_json::json!(null),
        })
        .is_none());
    }

    #[test]
    fn show_content_is_dropped() {
        assert!(convert(StreamEvent::ShowContent {
            timestamp: 0,
            content_type: "text".into(),
            title: "x".into(),
            content: "y".into(),
            metadata: None,
            file_path: None,
            is_attachment: None,
            base64: None,
        })
        .is_none());
    }

    #[test]
    fn request_input_is_dropped() {
        assert!(convert(StreamEvent::RequestInput {
            timestamp: 0,
            form_id: "f1".into(),
            form_type: "schema".into(),
            title: "t".into(),
            description: None,
            schema: serde_json::json!({}),
            submit_button: None,
        })
        .is_none());
    }

    #[test]
    fn token_update_is_dropped() {
        assert!(convert(StreamEvent::TokenUpdate {
            timestamp: 0,
            tokens_in: 10,
            tokens_out: 20,
        })
        .is_none());
    }
}
