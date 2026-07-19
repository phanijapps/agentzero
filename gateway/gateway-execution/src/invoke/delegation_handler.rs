//! # Delegation Handler
//!
//! Handle delegation events by creating child executions and dispatching requests.

use execution_state::{AgentExecution, DelegationType};

use super::event_logging::log_delegation;
use super::stream_context::StreamContext;
use crate::delegation::{DelegationMode, DelegationRequest};

/// Handle a delegation event by creating the execution synchronously and sending a request.
///
/// The execution record is created immediately with status QUEUED to prevent a race
/// condition where `try_complete_session()` could mark the session COMPLETED before
#[allow(clippy::too_many_arguments)]
/// the subagent execution exists.
pub fn handle_delegation(
    ctx: &StreamContext,
    child_agent: &str,
    task: &str,
    context: &Option<serde_json::Value>,
    max_iterations: Option<u32>,
    output_schema: &Option<serde_json::Value>,
    skills: &[String],
    capability_assignment: &Option<agent_primitives::event::AgentCapabilityAssignment>,
    planning_capability_catalog: &Option<serde_json::Value>,
    complexity: &Option<String>,
    mode: &Option<String>,
    parallel: bool,
    child_execution_id: Option<&str>,
) {
    let delegation_type = if parallel {
        DelegationType::Parallel
    } else {
        DelegationType::Sequential
    };

    // Create the delegated execution immediately (status=QUEUED)
    // This ensures try_complete_session() sees it as pending
    let child_execution_id = if let Some(id) = child_execution_id {
        // Use the pre-generated ID from DelegateTool — keeps parent's execution_id consistent
        match ctx.state_service.create_delegated_execution_with_id(
            id,
            &ctx.session_id,
            child_agent,
            &ctx.execution_id,
            delegation_type,
            task,
        ) {
            Ok(exec) => {
                tracing::debug!(
                    session_id = %ctx.session_id,
                    child_execution_id = %exec.id,
                    child_agent = %child_agent,
                    "Created delegated execution synchronously with pre-generated id"
                );
                exec.id
            }
            Err(e) => {
                tracing::error!(
                    session_id = %ctx.session_id,
                    child_agent = %child_agent,
                    error = %e,
                    "Failed to create delegated execution with provided id, using fallback"
                );
                AgentExecution::new_delegated(
                    &ctx.session_id,
                    child_agent,
                    &ctx.execution_id,
                    delegation_type,
                    task,
                )
                .id
            }
        }
    } else {
        // Legacy path (no pre-generated ID)
        match ctx.state_service.create_delegated_execution(
            &ctx.session_id,
            child_agent,
            &ctx.execution_id,
            delegation_type,
            task,
        ) {
            Ok(exec) => {
                tracing::debug!(
                    session_id = %ctx.session_id,
                    child_execution_id = %exec.id,
                    child_agent = %child_agent,
                    "Created delegated execution synchronously"
                );
                exec.id
            }
            Err(e) => {
                tracing::error!(
                    session_id = %ctx.session_id,
                    child_agent = %child_agent,
                    error = %e,
                    "Failed to create delegated execution, using fallback"
                );
                // Fallback: create in-memory execution ID (handler will create record)
                AgentExecution::new_delegated(
                    &ctx.session_id,
                    child_agent,
                    &ctx.execution_id,
                    delegation_type,
                    task,
                )
                .id
            }
        }
    };

    // Increment pending_delegations SYNCHRONOUSLY so the executor's Ok() path
    // sees it before checking has_pending_delegations(). The spawn handler will
    // NOT double-increment — register_delegation is idempotent (checks current count).
    if let Err(e) = ctx.state_service.register_delegation(&ctx.session_id) {
        tracing::warn!("Failed to register delegation synchronously: {}", e);
    }

    let parsed_mode = match mode {
        Some(value) => match value.parse::<DelegationMode>() {
            Ok(mode) => Some(mode),
            Err(e) => {
                tracing::warn!(
                    mode = %value,
                    error = %e,
                    "Ignoring invalid delegation mode from runtime action"
                );
                None
            }
        },
        None => None,
    };

    // Send request with pre-created execution_id
    let _ = ctx.delegation_tx.send(DelegationRequest {
        parent_agent_id: ctx.agent_id.clone(),
        session_id: ctx.session_id.clone(),
        parent_execution_id: ctx.execution_id.clone(),
        parent_conversation_id: ctx.conversation_id.clone(),
        child_agent_id: child_agent.to_string(),
        child_execution_id,
        task: task.to_string(),
        mode: parsed_mode,
        context: context.clone(),
        max_iterations,
        output_schema: output_schema.clone(),
        skills: skills.to_vec(),
        capability_assignment: capability_assignment.clone(),
        planning_capability_catalog: planning_capability_catalog.clone(),
        complexity: complexity.clone(),
        parallel,
    });

    log_delegation(ctx, child_agent, task);
}
