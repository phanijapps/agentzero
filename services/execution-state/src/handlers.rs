//! # HTTP Handlers
//!
//! Axum handlers for the session state API.

use crate::repository::StateDbProvider;
use crate::service::StateService;
use crate::types::*;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use std::sync::Arc;

// ============================================================================
// SESSION HANDLERS
// ============================================================================

/// List sessions (basic).
///
/// GET /sessions?status=...&limit=...
pub async fn list_sessions<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Query(filter): Query<SessionFilter>,
) -> Result<Json<Vec<Session>>, ApiError> {
    let sessions = service.list_sessions(&filter).map_err(ApiError::Database)?;

    Ok(Json(sessions))
}

/// List sessions with executions (for dashboard).
///
/// GET /sessions/full?status=...&limit=...
pub async fn list_sessions_full<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Query(filter): Query<SessionFilter>,
) -> Result<Json<Vec<SessionWithExecutions>>, ApiError> {
    let sessions = service
        .list_sessions_with_executions(&filter)
        .map_err(ApiError::Database)?;

    Ok(Json(sessions))
}

/// List bounded Mission Control session summaries.
///
/// GET /v2/mission-control/sessions?limit=...&offset=...
pub async fn list_mission_control_sessions<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Query(filter): Query<MissionControlFilter>,
) -> Result<Json<Vec<MissionControlSessionSummary>>, ApiError> {
    let sessions = service
        .list_mission_control_summaries(&filter)
        .map_err(ApiError::Database)?;

    Ok(Json(sessions))
}

/// Get per-execution token slices for one selected Mission Control session.
///
/// GET /v2/mission-control/sessions/:id/tokens
pub async fn get_mission_control_session_tokens<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<MissionControlSessionTokens>, ApiError> {
    let tokens = service
        .get_mission_control_session_tokens(&session_id)
        .map_err(ApiError::Database)?;

    match tokens {
        Some(t) => Ok(Json(t)),
        None => Err(ApiError::NotFound(format!(
            "Session not found: {}",
            session_id
        ))),
    }
}

/// Get a single session by ID.
///
/// GET /sessions/:id
pub async fn get_session<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<Session>, ApiError> {
    let session = service
        .get_session(&session_id)
        .map_err(ApiError::Database)?;

    match session {
        Some(s) => Ok(Json(s)),
        None => Err(ApiError::NotFound(format!(
            "Session not found: {}",
            session_id
        ))),
    }
}

/// Get session with all executions.
///
/// GET /sessions/:id/full
pub async fn get_session_full<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionWithExecutions>, ApiError> {
    let session = service
        .get_session_with_executions(&session_id)
        .map_err(ApiError::Database)?;

    match session {
        Some(s) => Ok(Json(s)),
        None => Err(ApiError::NotFound(format!(
            "Session not found: {}",
            session_id
        ))),
    }
}

/// Delete a session.
///
/// DELETE /sessions/:id
pub async fn delete_session<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let deleted = service
        .delete_session(&session_id)
        .map_err(ApiError::Database)?;

    Ok(Json(DeleteResponse {
        deleted,
        id: session_id,
    }))
}

// ============================================================================
// EXECUTION HANDLERS
// ============================================================================

/// List executions with filtering.
///
/// GET /executions?session_id=...&agent_id=...
pub async fn list_executions<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Query(filter): Query<ExecutionFilter>,
) -> Result<Json<Vec<AgentExecution>>, ApiError> {
    let executions = service
        .list_executions(&filter)
        .map_err(ApiError::Database)?;

    Ok(Json(executions))
}

// ============================================================================
// EXECUTION HANDLERS
// ============================================================================

/// Get a single execution by ID.
///
/// GET /executions/:id
pub async fn get_execution<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(execution_id): Path<String>,
) -> Result<Json<AgentExecution>, ApiError> {
    let execution = service
        .get_execution(&execution_id)
        .map_err(ApiError::Database)?;

    match execution {
        Some(e) => Ok(Json(e)),
        None => Err(ApiError::NotFound(format!(
            "Execution not found: {}",
            execution_id
        ))),
    }
}

/// Get child executions for a parent.
///
/// GET /executions/:id/children
pub async fn get_child_executions<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(execution_id): Path<String>,
) -> Result<Json<Vec<AgentExecution>>, ApiError> {
    let children = service
        .get_child_executions(&execution_id)
        .map_err(ApiError::Database)?;

    Ok(Json(children))
}

// ============================================================================
// CONTROL HANDLERS
// ============================================================================

/// Pause a running session.
///
/// POST /sessions/:id/pause
pub async fn pause_session<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<StatusResponse>, ApiError> {
    service
        .pause_session(&session_id)
        .map_err(ApiError::InvalidTransition)?;

    Ok(Json(StatusResponse {
        id: session_id,
        status: "paused".to_string(),
    }))
}

/// Resume a paused session.
///
/// POST /sessions/:id/resume
pub async fn resume_session<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<StatusResponse>, ApiError> {
    service
        .resume_session(&session_id)
        .map_err(ApiError::InvalidTransition)?;

    Ok(Json(StatusResponse {
        id: session_id,
        status: "running".to_string(),
    }))
}

/// Cancel a session.
///
/// POST /sessions/:id/cancel
pub async fn cancel_session<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
) -> Result<Json<StatusResponse>, ApiError> {
    service
        .cancel_session(&session_id)
        .map_err(ApiError::InvalidTransition)?;

    Ok(Json(StatusResponse {
        id: session_id,
        status: "cancelled".to_string(),
    }))
}

// ============================================================================
// STATS HANDLERS
// ============================================================================

/// Get dashboard stats (pre-computed, ready to display).
///
/// GET /stats
pub async fn get_dashboard_stats<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
) -> Result<Json<DashboardStats>, ApiError> {
    let stats = service.get_dashboard_stats().map_err(ApiError::Database)?;

    Ok(Json(stats))
}

/// Get stats as counts map.
///
/// GET /stats/counts
pub async fn get_stats_counts<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
) -> Result<Json<std::collections::HashMap<String, u64>>, ApiError> {
    let stats = service.get_dashboard_stats().map_err(ApiError::Database)?;

    let mut counts = std::collections::HashMap::new();

    // Session counts
    counts.insert("sessions_running".to_string(), stats.sessions_running);
    counts.insert("sessions_paused".to_string(), stats.sessions_paused);
    counts.insert("sessions_completed".to_string(), stats.sessions_completed);
    counts.insert("sessions_crashed".to_string(), stats.sessions_crashed);

    // Execution counts
    counts.insert("executions_queued".to_string(), stats.executions_queued);
    counts.insert("executions_running".to_string(), stats.executions_running);
    counts.insert(
        "executions_completed".to_string(),
        stats.executions_completed,
    );
    counts.insert("executions_crashed".to_string(), stats.executions_crashed);
    counts.insert(
        "executions_cancelled".to_string(),
        stats.executions_cancelled,
    );

    // Daily stats
    counts.insert("today_sessions".to_string(), stats.today_sessions);
    counts.insert("today_tokens".to_string(), stats.today_tokens);

    Ok(Json(counts))
}

// ============================================================================
// MESSAGES HANDLERS
// ============================================================================

/// Get messages for an execution.
///
/// GET /executions/:id/messages
pub async fn get_execution_messages<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(execution_id): Path<String>,
) -> Result<Json<Vec<Message>>, ApiError> {
    let messages = service
        .get_messages(&execution_id)
        .map_err(ApiError::Database)?;

    Ok(Json(messages))
}

/// Get messages for a session with scope filtering.
///
/// GET /v2/sessions/:id/messages?scope=all|root|execution|delegates&execution_id=...&agent_id=...
///
/// Scopes:
/// - `all` (default): All messages from all executions
/// - `root`: Only messages from root executions (main chat view)
/// - `execution`: Messages from a specific execution (requires execution_id)
/// - `delegates`: Only messages from delegated executions
pub async fn get_session_messages<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionMessagesQuery>,
) -> Result<Json<Vec<SessionMessage>>, ApiError> {
    // Validate: execution scope requires execution_id
    if matches!(query.scope, MessageScope::Execution) && query.execution_id.is_none() {
        return Err(ApiError::BadRequest(
            "execution_id is required when scope=execution".to_string(),
        ));
    }

    let messages = service
        .get_session_messages(&session_id, &query)
        .map_err(ApiError::Database)?;

    Ok(Json(messages))
}

// ============================================================================
// CLEANUP HANDLERS
// ============================================================================

/// Parameters for cleanup endpoint.
#[derive(Debug, serde::Deserialize)]
pub struct CleanupParams {
    pub older_than: Option<String>,
}

/// Response for cleanup operations.
#[derive(Debug, serde::Serialize)]
pub struct CleanupResponse {
    pub deleted: u64,
}

/// Cleanup old sessions.
///
/// DELETE /cleanup?older_than=<RFC3339 timestamp>
pub async fn cleanup_old_sessions<D: StateDbProvider + 'static>(
    State(service): State<Arc<StateService<D>>>,
    Query(params): Query<CleanupParams>,
) -> Result<Json<CleanupResponse>, ApiError> {
    let older_than = params
        .older_than
        .ok_or_else(|| ApiError::BadRequest("older_than parameter required".to_string()))?;

    let deleted = service
        .delete_old_sessions(&older_than)
        .map_err(ApiError::Database)?;

    Ok(Json(CleanupResponse { deleted }))
}

// ============================================================================
// RESPONSE TYPES
// ============================================================================

/// Response for delete operations.
#[derive(Debug, serde::Serialize)]
pub struct DeleteResponse {
    pub deleted: bool,
    pub id: String,
}

/// Response for status change operations.
#[derive(Debug, serde::Serialize)]
pub struct StatusResponse {
    pub id: String,
    pub status: String,
}

/// Message type for API responses.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub id: String,
    pub execution_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<serde_json::Value>,
}

/// Extended message type for session-scoped queries.
///
/// Includes execution metadata (agent_id, delegation_type) for context.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub execution_id: String,
    pub agent_id: String,
    pub delegation_type: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<serde_json::Value>,
}

/// Scope for session messages query.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageScope {
    /// All messages from all executions in the session
    #[default]
    All,
    /// Only messages from root executions
    Root,
    /// Messages from a specific execution (requires execution_id)
    Execution,
    /// Only messages from delegated (non-root) executions
    Delegates,
}

impl MessageScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Root => "root",
            Self::Execution => "execution",
            Self::Delegates => "delegates",
        }
    }
}

/// Query parameters for session messages endpoint.
#[derive(Debug, Default, serde::Deserialize)]
pub struct SessionMessagesQuery {
    /// Scope of messages to return
    #[serde(default)]
    pub scope: MessageScope,
    /// Execution ID (required when scope=execution)
    pub execution_id: Option<String>,
    /// Filter by agent ID
    pub agent_id: Option<String>,
}
