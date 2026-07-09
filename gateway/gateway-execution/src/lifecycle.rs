//! # Lifecycle Module
//!
//! Manages session and execution lifecycle transitions.
//!
//! This module centralizes all state transitions for sessions and executions,
//! including creation, completion, error handling, and cancellation.

use api_logs::{LogService, SessionStatus};
use execution_state::{AgentExecution, Session, StateService, TriggerSource};
use gateway_bridge::{BridgeRegistry, OutboxRepository as BridgeOutbox};
use gateway_connectors::{ConnectorRegistry, DispatchContext};
use gateway_events::{EventBus, GatewayEvent};
use std::sync::Arc;
use zbot_stores_sqlite::DatabaseManager;

// ============================================================================
// SESSION CREATION
// ============================================================================

/// Result of session/execution setup.
pub struct SessionSetup {
    /// Session ID
    pub session_id: String,
    /// Execution ID
    pub execution_id: String,
    /// Active ward from existing session (None for new sessions)
    pub ward_id: Option<String>,
}

/// Get or create a session and execution for an agent invocation.
///
/// If `existing_session_id` is provided and the session exists, creates a new
/// execution within that session. Otherwise, creates a new session and execution.
///
/// If the existing session was in a terminal state (completed/crashed), it will
/// be reactivated to running status.
///
/// The `source` parameter determines the trigger source for new sessions.
pub fn get_or_create_session(
    state_service: &StateService<DatabaseManager>,
    agent_id: &str,
    existing_session_id: Option<&str>,
    source: TriggerSource,
) -> SessionSetup {
    if let Some(session_id) = existing_session_id {
        // Try to continue existing session
        let session = match state_service.get_session(session_id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!("Session {} not found, creating new session", session_id);
                return create_new_session(state_service, agent_id, source);
            }
            Err(e) => {
                tracing::warn!("Failed to get session: {}", e);
                return create_new_session(state_service, agent_id, source);
            }
        };

        // Reuse the existing root execution (one continuous conversation)
        let execution_id = match state_service.get_root_execution(session_id) {
            Ok(Some(root_exec)) => root_exec.id,
            _ => {
                // Fallback: create new root execution if none found
                let execution = AgentExecution::new_root(session_id, agent_id);
                if let Err(e) = state_service.create_execution(&execution) {
                    tracing::warn!("Failed to create execution in existing session: {}", e);
                }
                execution.id
            }
        };

        // Reactivate session if it was in a terminal state (completed/crashed)
        // This handles the case where user sends a new message to a completed session
        if let Err(e) = state_service.reactivate_session(session_id) {
            tracing::warn!("Failed to reactivate session: {}", e);
        }

        // Also reactivate the execution if it was completed
        if let Err(e) = state_service.reactivate_execution(&execution_id) {
            tracing::warn!("Failed to reactivate execution: {}", e);
        }

        return SessionSetup {
            session_id: session_id.to_string(),
            execution_id,
            ward_id: session.ward_id,
        };
    }

    create_new_session(state_service, agent_id, source)
}

fn create_new_session(
    state_service: &StateService<DatabaseManager>,
    agent_id: &str,
    source: TriggerSource,
) -> SessionSetup {
    let (session, execution) = state_service
        .create_session_with_source(agent_id, source)
        .unwrap_or_else(|e| {
            tracing::warn!("Failed to create session: {}", e);
            let s = Session::new_with_source(agent_id, source);
            let e = AgentExecution::new_root(&s.id, agent_id);
            (s, e)
        });

    SessionSetup {
        session_id: session.id,
        execution_id: execution.id,
        ward_id: None,
    }
}

/// Start an execution and log the start event.
pub fn start_execution(
    state_service: &StateService<DatabaseManager>,
    log_service: &LogService<DatabaseManager>,
    execution_id: &str,
    session_id: &str,
    agent_id: &str,
    parent_execution_id: Option<&str>,
) {
    // Transition execution state: QUEUED -> RUNNING
    if let Err(e) = state_service.start_execution(execution_id) {
        tracing::warn!("Failed to start execution: {}", e);
    }

    // Log the execution start
    if let Err(e) =
        log_service.log_session_start(execution_id, session_id, agent_id, parent_execution_id)
    {
        tracing::warn!("Failed to log execution start: {}", e);
    }

    tracing::info!(
        session_id = %session_id,
        execution_id = %execution_id,
        agent_id = %agent_id,
        "Execution started"
    );
}

// ============================================================================
// COMPLETION HANDLING
// ============================================================================

/// Inputs for [`complete_execution`]. Groups the 13 historical positional
/// parameters — four same-type `&str` ids that silently misorder, plus five
/// Option-typed "dispatch policy" slots — into named fields.
pub struct CompleteExecution<'a> {
    pub state_service: &'a StateService<DatabaseManager>,
    pub log_service: &'a LogService<DatabaseManager>,
    pub event_bus: &'a EventBus,
    pub execution_id: &'a str,
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub conversation_id: &'a str,
    pub response: Option<String>,
    pub connector_registry: Option<&'a Arc<ConnectorRegistry>>,
    pub respond_to: Option<&'a Vec<String>>,
    pub thread_id: Option<&'a str>,
    pub bridge_registry: Option<&'a Arc<BridgeRegistry>>,
    pub bridge_outbox: Option<&'a Arc<BridgeOutbox>>,
}

/// Handle successful execution completion.
///
/// Updates state, logs the completion, emits events, and dispatches to connectors.
///
/// For root executions with pending delegations, this will request a continuation
/// turn to be spawned after all delegations complete.
///
/// If `respond_to` contains connector IDs and `connector_registry` is provided,
/// the response will be dispatched to those connectors.
pub async fn complete_execution(ctx: CompleteExecution<'_>) {
    let CompleteExecution {
        state_service,
        log_service,
        event_bus,
        execution_id,
        session_id,
        agent_id,
        conversation_id,
        response,
        connector_registry,
        respond_to,
        thread_id,
        bridge_registry,
        bridge_outbox,
    } = ctx;
    // Update execution status to COMPLETED
    if let Err(e) = state_service.complete_execution(execution_id) {
        tracing::warn!("Failed to complete execution: {}", e);
    }

    // Eagerly aggregate session tokens so UI shows real values
    // (web sessions never auto-complete, so without this they'd stay at 0)
    if let Err(e) = state_service.aggregate_session_tokens(session_id) {
        tracing::warn!("Failed to aggregate session tokens: {}", e);
    }

    // Check if this is a root execution and has pending delegations
    // If so, request continuation after delegations complete
    if let Ok(Some(execution)) = state_service.get_execution(execution_id) {
        if execution.is_root() {
            if let Ok(Some(session)) = state_service.get_session(session_id) {
                if session.has_pending_delegations() {
                    // Root execution completed while delegations are pending
                    // Request continuation to process results when they arrive
                    if let Err(e) = state_service.request_continuation(session_id) {
                        tracing::warn!("Failed to request continuation: {}", e);
                    } else {
                        tracing::info!(
                            session_id = %session_id,
                            pending = session.pending_delegations,
                            "Root execution complete, continuation requested for pending delegations"
                        );
                    }
                }
            }
        }
    }

    // Try to complete session if no other executions are running
    match state_service.try_complete_session(session_id) {
        Ok(true) => tracing::debug!("Session completed"),
        Ok(false) => tracing::debug!("Session still has running executions"),
        Err(e) => tracing::warn!("Failed to check/complete session: {}", e),
    }

    // Log session end
    let _ = log_service.log_session_end(
        execution_id,
        session_id,
        agent_id,
        SessionStatus::Completed,
        Some("Execution completed successfully"),
    );

    // Emit completion event
    event_bus
        .publish(GatewayEvent::AgentCompleted {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            result: response.clone(),
            conversation_id: Some(conversation_id.to_string()),
        })
        .await;

    // Dispatch response to connectors if respond_to is specified
    // Only dispatch to ConnectorRegistry for connectors NOT in BridgeRegistry
    if let (Some(registry), Some(connector_ids), Some(bridge)) =
        (connector_registry, respond_to, bridge_registry)
    {
        if !connector_ids.is_empty() {
            if let Some(response_text) = &response {
                // Filter out bridge workers (plugins) - they'll be handled below
                let mut connector_only_ids: Vec<String> = Vec::new();
                for id in connector_ids {
                    if !bridge.is_connected(id).await {
                        connector_only_ids.push(id.clone());
                    }
                }

                if !connector_only_ids.is_empty() {
                    let context = DispatchContext {
                        session_id: session_id.to_string(),
                        thread_id: thread_id.map(|t| t.to_string()),
                        agent_id: agent_id.to_string(),
                        timestamp: chrono::Utc::now(),
                    };

                    let payload = serde_json::json!({
                        "message": response_text,
                        "execution_id": execution_id,
                        "conversation_id": conversation_id,
                    });

                    tracing::info!(
                        session_id = %session_id,
                        connectors = ?connector_only_ids,
                        "Dispatching response to connectors"
                    );

                    let results = registry
                        .dispatch_to_many(&connector_only_ids, "respond", payload, &context)
                        .await;

                    for (connector_id, result) in results {
                        match result {
                            Ok(resp) => {
                                tracing::info!(
                                    connector_id = %connector_id,
                                    success = resp.success,
                                    "Connector dispatch completed"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    connector_id = %connector_id,
                                    error = %e,
                                    "Connector dispatch failed"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Bridge workers: push response via outbox (reliable delivery)
    if let (Some(bridge), Some(outbox), Some(connector_ids)) =
        (bridge_registry, bridge_outbox, respond_to)
    {
        if let Some(response_text) = &response {
            let payload = serde_json::json!({
                "message": response_text,
                "execution_id": execution_id,
                "session_id": session_id,
                "thread_id": thread_id,
            });

            for id in connector_ids {
                if bridge.is_connected(id).await {
                    if let Err(e) = gateway_bridge::enqueue_and_push(
                        id,
                        "respond",
                        &payload,
                        Some(session_id),
                        thread_id,
                        Some(agent_id),
                        outbox,
                        bridge,
                    )
                    .await
                    {
                        tracing::warn!(
                            connector_id = %id,
                            "Bridge dispatch failed: {}",
                            e
                        );
                    }
                }
            }
        }
    }
}

/// Inputs for [`crash_execution`]. Four same-type `&str` ids + a bool
/// (`crash_session`) that a positional call can silently flip — named
/// fields make both failure modes compile errors instead.
pub struct CrashExecution<'a> {
    pub state_service: &'a StateService<DatabaseManager>,
    pub log_service: &'a LogService<DatabaseManager>,
    pub event_bus: &'a EventBus,
    pub execution_id: &'a str,
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub conversation_id: &'a str,
    pub error: &'a str,
    pub crash_session: bool,
}

/// Handle execution error/crash.
///
/// Updates state, logs the error, and emits events.
pub async fn crash_execution(ctx: CrashExecution<'_>) {
    let CrashExecution {
        state_service,
        log_service,
        event_bus,
        execution_id,
        session_id,
        agent_id,
        conversation_id,
        error,
        crash_session,
    } = ctx;
    // Update execution status to CRASHED
    if let Err(e) = state_service.crash_execution(execution_id, error) {
        tracing::warn!("Failed to crash execution: {}", e);
    }

    // Optionally crash the session too (for root executions)
    if crash_session {
        // crash_session() already aggregates tokens
        if let Err(e) = state_service.crash_session(session_id) {
            tracing::warn!("Failed to crash session: {}", e);
        }
    } else {
        // Subagent crash: still aggregate tokens for UI visibility
        if let Err(e) = state_service.aggregate_session_tokens(session_id) {
            tracing::warn!("Failed to aggregate session tokens: {}", e);
        }
    }

    // Log session error
    let _ = log_service.log_session_end(
        execution_id,
        session_id,
        agent_id,
        SessionStatus::Error,
        Some(error),
    );

    // Emit error event
    event_bus
        .publish(GatewayEvent::Error {
            agent_id: Some(agent_id.to_string()),
            session_id: Some(session_id.to_string()),
            execution_id: Some(execution_id.to_string()),
            message: error.to_string(),
            conversation_id: Some(conversation_id.to_string()),
        })
        .await;
}

/// Inputs for [`stop_execution`].
pub struct StopExecution<'a> {
    pub state_service: &'a StateService<DatabaseManager>,
    pub log_service: &'a LogService<DatabaseManager>,
    pub event_bus: &'a EventBus,
    pub execution_id: &'a str,
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub conversation_id: &'a str,
    pub iteration: u32,
}

/// Handle user-initiated stop.
///
/// Updates state, logs the stop, and emits events.
pub async fn stop_execution(ctx: StopExecution<'_>) {
    let StopExecution {
        state_service,
        log_service,
        event_bus,
        execution_id,
        session_id,
        agent_id,
        conversation_id,
        iteration,
    } = ctx;
    // Update session status to CANCELLED
    if let Err(e) = state_service.cancel_session(session_id) {
        tracing::warn!("Failed to cancel session: {}", e);
    }

    // Log session stopped
    let _ = log_service.log_session_end(
        execution_id,
        session_id,
        agent_id,
        SessionStatus::Stopped,
        Some("Stopped by user"),
    );

    // Emit stopped event
    event_bus
        .publish(GatewayEvent::AgentStopped {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            iteration,
            conversation_id: Some(conversation_id.to_string()),
        })
        .await;
}

// ============================================================================
// EVENT EMISSION HELPERS
// ============================================================================

/// Emit agent started event.
pub async fn emit_agent_started(
    event_bus: &EventBus,
    agent_id: &str,
    conversation_id: &str,
    session_id: &str,
    execution_id: &str,
) {
    event_bus
        .publish(GatewayEvent::AgentStarted {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            conversation_id: Some(conversation_id.to_string()),
        })
        .await;
}

/// Emit delegation started event.
///
/// Includes both `parent_conversation_id` (so the parent's WebSocket subscription
/// receives this event in real time) and `child_conversation_id` (used as the
/// frontend key for subagent activities).
#[allow(clippy::too_many_arguments)]
pub async fn emit_delegation_started(
    event_bus: &EventBus,
    parent_agent_id: &str,
    session_id: &str,
    parent_execution_id: &str,
    parent_conversation_id: &str,
    child_agent_id: &str,
    child_execution_id: &str,
    child_conversation_id: &str,
    task: &str,
) {
    event_bus
        .publish(GatewayEvent::DelegationStarted {
            session_id: session_id.to_string(),
            parent_execution_id: parent_execution_id.to_string(),
            child_execution_id: child_execution_id.to_string(),
            parent_agent_id: parent_agent_id.to_string(),
            child_agent_id: child_agent_id.to_string(),
            task: task.to_string(),
            parent_conversation_id: Some(parent_conversation_id.to_string()),
            child_conversation_id: Some(child_conversation_id.to_string()),
        })
        .await;
}

/// Inputs for [`emit_delegation_completed`].
pub struct DelegationCompletedEvent<'a> {
    pub event_bus: &'a EventBus,
    pub parent_agent_id: &'a str,
    pub session_id: &'a str,
    pub child_agent_id: &'a str,
    pub child_execution_id: &'a str,
    pub parent_conversation_id: Option<&'a str>,
    pub child_conversation_id: Option<&'a str>,
    pub result: Option<String>,
}

/// Emit delegation completed event.
pub async fn emit_delegation_completed(ctx: DelegationCompletedEvent<'_>) {
    let DelegationCompletedEvent {
        event_bus,
        parent_agent_id,
        session_id,
        child_agent_id,
        child_execution_id,
        parent_conversation_id,
        child_conversation_id,
        result,
    } = ctx;
    event_bus
        .publish(GatewayEvent::DelegationCompleted {
            session_id: session_id.to_string(),
            parent_execution_id: session_id.to_string(), // Will be updated by caller
            child_execution_id: child_execution_id.to_string(),
            parent_agent_id: parent_agent_id.to_string(),
            child_agent_id: child_agent_id.to_string(),
            result,
            parent_conversation_id: parent_conversation_id.map(|s| s.to_string()),
            child_conversation_id: child_conversation_id.map(|s| s.to_string()),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_setup_struct() {
        let setup = SessionSetup {
            session_id: "session-123".to_string(),
            execution_id: "exec-456".to_string(),
            ward_id: Some("my-project".to_string()),
        };
        assert_eq!(setup.session_id, "session-123");
        assert_eq!(setup.execution_id, "exec-456");
        assert_eq!(setup.ward_id, Some("my-project".to_string()));
    }

    /// Regression: `emit_delegation_started` must populate `parent_conversation_id`
    /// on the published event so the parent's WebSocket subscription (scoped by
    /// conversation_id) receives the event in real time.
    ///
    /// See `defect_research_realtime_events_race.md` — when this was hardcoded
    /// to `None`, /research pages missed delegation events until manual refresh.
    #[tokio::test]
    async fn emit_delegation_started_populates_parent_conversation_id() {
        let event_bus = EventBus::new();
        let mut rx = event_bus.subscribe_all();

        emit_delegation_started(
            &event_bus,
            "root",
            "session-abc",
            "exec-parent-7",
            "conv-parent-xyz",
            "child-agent",
            "exec-child-1",
            "conv-child-1",
            "do work",
        )
        .await;

        let event = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("event must be received before timeout")
            .expect("event must not be lagged/closed");

        match event {
            GatewayEvent::DelegationStarted {
                parent_execution_id,
                parent_conversation_id,
                child_conversation_id,
                ..
            } => {
                assert_eq!(
                    parent_execution_id, "exec-parent-7",
                    "parent_execution_id must be the parent agent's exec id, not the session id \
                     (UI childrenOf() filters by parent_execution_id and drops subagents otherwise)"
                );
                assert_eq!(
                    parent_conversation_id,
                    Some("conv-parent-xyz".to_string()),
                    "parent_conversation_id must be threaded through (was hardcoded to None)"
                );
                assert_eq!(
                    child_conversation_id,
                    Some("conv-child-1".to_string()),
                    "child_conversation_id should still be set"
                );
            }
            other => panic!("expected DelegationStarted, got: {:?}", other),
        }
    }
}
