//! # Continuation Module
//!
//! Handles automatic spawning of continuation turns when delegations complete.
//!
//! When all delegated subagents complete and the session has requested continuation,
//! this module spawns a new execution turn for the root agent to process the results.

use execution_state::{AgentExecution, StateService};
use gateway_events::{EventBus, GatewayEvent};
use zbot_stores_sqlite::DatabaseManager;

/// Spawn a continuation turn for a session.
///
/// This creates a new root execution to continue processing after delegations complete.
/// The continuation execution allows the root agent to synthesize and respond based on
/// the results from all completed subagents.
///
/// # Arguments
///
/// * `state_service` - State service for managing session/execution state
/// * `event_bus` - Event bus for emitting continuation events
/// * `session_id` - The session ID to continue
/// * `root_agent_id` - The root agent ID
/// * `root_execution_id` - The previous root execution ID (for reference)
pub async fn spawn_continuation_turn(
    state_service: &StateService<DatabaseManager>,
    event_bus: &EventBus,
    session_id: &str,
    root_agent_id: &str,
    root_execution_id: &str,
) -> Result<String, String> {
    // Clear continuation flag first to prevent double-spawn
    state_service.clear_continuation(session_id)?;

    // Create a new root execution for the continuation turn
    let continuation_exec = AgentExecution::new_root(session_id, root_agent_id);
    let continuation_id = continuation_exec.id.clone();

    state_service.create_execution(&continuation_exec)?;

    tracing::info!(
        session_id = %session_id,
        continuation_id = %continuation_id,
        previous_root = %root_execution_id,
        agent_id = %root_agent_id,
        "Created continuation execution"
    );

    // Emit event to notify that continuation is ready to be invoked
    // The runner or HTTP handler will pick this up and invoke the agent
    event_bus
        .publish(GatewayEvent::SessionContinuationReady {
            session_id: session_id.to_string(),
            root_agent_id: root_agent_id.to_string(),
            root_execution_id: continuation_id.clone(),
        })
        .await;

    Ok(continuation_id)
}

/// Check if a session needs continuation and spawn if necessary.
///
/// This is a convenience function that checks the session state and spawns
/// a continuation turn if needed.
pub async fn check_and_spawn_continuation(
    state_service: &StateService<DatabaseManager>,
    event_bus: &EventBus,
    session_id: &str,
) -> Result<Option<String>, String> {
    // Get the session
    let session = state_service
        .get_session(session_id)?
        .ok_or_else(|| format!("Session not found: {}", session_id))?;

    // Check if continuation is needed
    if !session.needs_continuation() {
        return Ok(None);
    }

    // Get the root execution
    let root_exec = state_service
        .get_root_execution(session_id)?
        .ok_or_else(|| format!("Root execution not found for session: {}", session_id))?;

    // Spawn continuation
    let continuation_id = spawn_continuation_turn(
        state_service,
        event_bus,
        session_id,
        &root_exec.agent_id,
        &root_exec.id,
    )
    .await?;

    Ok(Some(continuation_id))
}

#[cfg(test)]
mod tests {
    // Tests would require mocking StateService and EventBus
    // which is complex for unit tests. Integration tests are more appropriate.
}
