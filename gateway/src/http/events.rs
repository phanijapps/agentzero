//! # Events HTTP Endpoint
//!
//! Server-Sent Events (SSE) endpoint for streaming events to adapters.
//!
//! External adapters (WhatsApp, Telegram, etc) can subscribe to this endpoint
//! to receive events and route responses back to their respective platforms.
//!
//! ## Endpoint
//!
//! `GET /api/events/{conversation_id}`
//!
//! ## Event Types
//!
//! - `respond` - Response from the respond tool
//! - `agent_completed` - Agent finished execution
//! - `token` - Streaming token
//! - `tool_call` - Tool invocation
//! - `tool_result` - Tool result

use crate::events::GatewayEvent;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    response::sse::{Event, Sse},
};
use futures::stream::{self, Stream};
use std::convert::Infallible;
use std::time::Duration;

/// Stream events for a specific conversation.
///
/// GET /api/events/{conversation_id}
///
/// Returns a Server-Sent Events stream with events for the specified conversation.
/// Adapters can use this to receive agent responses and route them to external platforms.
pub async fn event_stream(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let event_bus = state.event_bus().clone();
    let receiver = event_bus.subscribe_all();
    let target_conversation = conversation_id.clone();

    let stream = stream::unfold(
        (receiver, target_conversation),
        |(mut rx, target_conv): (tokio::sync::broadcast::Receiver<GatewayEvent>, String)| async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // Filter events for this conversation
                        if let Some(conv_id) = event.conversation_id() {
                            if conv_id == target_conv {
                                // Convert to SSE event
                                if let Some(sse_event) = gateway_event_to_sse(&event) {
                                    return Some((Ok(sse_event), (rx, target_conv)));
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Skip lagged events
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return None;
                    }
                }
            }
        },
    );

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

/// Stream all events (for debugging/monitoring).
///
/// GET /api/events
///
/// Returns all events from the event bus. Use with caution in production.
pub async fn all_events_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let event_bus = state.event_bus().clone();
    let receiver = event_bus.subscribe_all();

    let stream = stream::unfold(
        receiver,
        |mut rx: tokio::sync::broadcast::Receiver<GatewayEvent>| async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Some(sse_event) = gateway_event_to_sse(&event) {
                            return Some((Ok(sse_event), rx));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return None;
                    }
                }
            }
        },
    );

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

/// Convert a GatewayEvent to an SSE Event.
fn gateway_event_to_sse(event: &GatewayEvent) -> Option<Event> {
    let (event_type, data) = match event {
        GatewayEvent::Respond {
            session_id,
            execution_id,
            message,
            conversation_id,
        } => (
            "respond",
            serde_json::json!({
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "message": message
            }),
        ),

        GatewayEvent::AgentCompleted {
            agent_id,
            session_id,
            execution_id,
            result,
            conversation_id,
        } => (
            "agent_completed",
            serde_json::json!({
                "agent_id": agent_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "result": result
            }),
        ),

        GatewayEvent::Token {
            agent_id,
            session_id,
            execution_id,
            delta,
            conversation_id,
        } => (
            "token",
            serde_json::json!({
                "agent_id": agent_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "delta": delta
            }),
        ),

        GatewayEvent::ToolCall {
            agent_id,
            session_id,
            execution_id,
            tool_id,
            tool_name,
            args,
            conversation_id,
        } => (
            "tool_call",
            serde_json::json!({
                "agent_id": agent_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "tool_id": tool_id,
                "tool_name": tool_name,
                "args": args
            }),
        ),

        GatewayEvent::ToolResult {
            agent_id,
            session_id,
            execution_id,
            tool_id,
            result,
            error,
            conversation_id,
        } => (
            "tool_result",
            serde_json::json!({
                "agent_id": agent_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "tool_id": tool_id,
                "result": result,
                "error": error
            }),
        ),

        GatewayEvent::TurnComplete {
            agent_id,
            session_id,
            execution_id,
            message,
            conversation_id,
        } => (
            "turn_complete",
            serde_json::json!({
                "agent_id": agent_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "message": message
            }),
        ),

        GatewayEvent::DelegationStarted {
            session_id,
            parent_execution_id,
            child_execution_id,
            parent_agent_id,
            child_agent_id,
            task,
            parent_conversation_id,
            child_conversation_id,
        } => (
            "delegation_started",
            serde_json::json!({
                "session_id": session_id,
                "parent_execution_id": parent_execution_id,
                "child_execution_id": child_execution_id,
                "parent_agent_id": parent_agent_id,
                "child_agent_id": child_agent_id,
                "parent_conversation_id": parent_conversation_id,
                "child_conversation_id": child_conversation_id,
                "task": task
            }),
        ),

        GatewayEvent::DelegationCompleted {
            session_id,
            parent_execution_id,
            child_execution_id,
            parent_agent_id,
            child_agent_id,
            result,
            parent_conversation_id,
            child_conversation_id,
        } => (
            "delegation_completed",
            serde_json::json!({
                "session_id": session_id,
                "parent_execution_id": parent_execution_id,
                "child_execution_id": child_execution_id,
                "parent_agent_id": parent_agent_id,
                "child_agent_id": child_agent_id,
                "parent_conversation_id": parent_conversation_id,
                "child_conversation_id": child_conversation_id,
                "result": result
            }),
        ),

        GatewayEvent::Error {
            agent_id,
            session_id,
            execution_id,
            message,
            conversation_id,
        } => (
            "error",
            serde_json::json!({
                "agent_id": agent_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "conversation_id": conversation_id,
                "message": message
            }),
        ),

        // Skip other events that aren't relevant for adapters
        _ => return None,
    };

    Event::default().event(event_type).json_data(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_event_to_sse() {
        let event = GatewayEvent::Respond {
            session_id: "session-456".to_string(),
            execution_id: "exec-789".to_string(),
            message: "Hello!".to_string(),
            conversation_id: Some("conv-123".to_string()),
        };

        let sse_event = gateway_event_to_sse(&event);
        assert!(sse_event.is_some());
    }

    #[test]
    fn test_agent_completed_to_sse() {
        let event = GatewayEvent::AgentCompleted {
            agent_id: "agent-1".to_string(),
            session_id: "session-456".to_string(),
            execution_id: "exec-789".to_string(),
            result: Some("Done!".to_string()),
            conversation_id: Some("conv-123".to_string()),
        };

        let sse_event = gateway_event_to_sse(&event);
        assert!(sse_event.is_some());
    }
}
