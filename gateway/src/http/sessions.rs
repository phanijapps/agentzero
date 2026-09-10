//! # Session Archive Endpoints
//!
//! HTTP API for archiving and restoring session transcripts.

use super::ErrorResponse;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use gateway_execution::{SessionState, SessionStateBuilder};
use serde::{Deserialize, Serialize};

// ============================================================================
// REQUEST / RESPONSE TYPES
// ============================================================================

/// Request body for archiving old sessions.
#[derive(Debug, Deserialize)]
pub struct ArchiveRequest {
    /// Archive sessions older than this many days (default: 7)
    #[serde(default = "default_older_than_days")]
    pub older_than_days: u32,
}

fn default_older_than_days() -> u32 {
    7
}

/// Response for the archive endpoint.
#[derive(Debug, Serialize)]
pub struct ArchiveResponse {
    pub archived: usize,
    pub results: Vec<ArchiveResultEntry>,
}

/// Single session archive result.
#[derive(Debug, Serialize)]
pub struct ArchiveResultEntry {
    pub session_id: String,
    pub messages_archived: usize,
    pub logs_archived: usize,
    pub file_size: u64,
}

/// Response for the restore endpoint.
#[derive(Debug, Serialize)]
pub struct RestoreResponse {
    pub session_id: String,
    pub records_restored: usize,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// POST /api/sessions/archive
/// Archive old session transcripts to compressed JSONL files.
pub async fn archive_sessions(
    State(state): State<AppState>,
    Json(body): Json<ArchiveRequest>,
) -> Result<Json<ArchiveResponse>, (StatusCode, Json<ErrorResponse>)> {
    let archiver_slot = state.session_archiver();
    let archiver = match archiver_slot.as_deref() {
        Some(a) => a,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "Session archiver not available".to_string(),
                )),
            ));
        }
    };

    match archiver.archive_old_sessions(body.older_than_days) {
        Ok(results) => {
            let entries: Vec<ArchiveResultEntry> = results
                .iter()
                .map(|r| ArchiveResultEntry {
                    session_id: r.session_id.clone(),
                    messages_archived: r.messages_archived,
                    logs_archived: r.logs_archived,
                    file_size: r.file_size,
                })
                .collect();
            let count = entries.len();
            Ok(Json(ArchiveResponse {
                archived: count,
                results: entries,
            }))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(format!("Archive failed: {}", e))),
        )),
    }
}

/// POST /api/sessions/restore/:id
/// Restore an archived session from its compressed JSONL file.
pub async fn restore_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<RestoreResponse>, (StatusCode, Json<ErrorResponse>)> {
    let archiver_slot = state.session_archiver();
    let archiver = match archiver_slot.as_deref() {
        Some(a) => a,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "Session archiver not available".to_string(),
                )),
            ));
        }
    };

    match archiver.restore_session(&session_id) {
        Ok(records_restored) => Ok(Json(RestoreResponse {
            session_id,
            records_restored,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(format!("Restore failed: {}", e))),
        )),
    }
}

/// GET /api/sessions/:id/state — returns structured session snapshot
pub async fn get_session_state(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionState>, (StatusCode, Json<ErrorResponse>)> {
    // Reads through the services group: log + state services (messages
    // stays a single stores read).
    let services = state.services();
    let builder = SessionStateBuilder::new(
        services.log_service.clone(),
        state.messages().clone(),
        services.state_service.clone(),
    );

    match builder.build(&session_id) {
        Ok(Some(session_state)) => Ok(Json(session_state)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(format!(
                "Session not found: {}",
                session_id
            ))),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(format!(
                "Failed to build session state: {}",
                e
            ))),
        )),
    }
}

/// DELETE /api/sessions/:id — hard-delete session, every descendant
/// subagent session, and all per-session data for the entire tree.
///
/// Walks `sessions.parent_session_id` recursively, then cascades to
/// `messages`, `agent_executions`, `execution_logs`, `artifacts`
/// (DB rows only — files on disk stay), `distillation_runs`, `bridge_outbox`,
/// and `recall_log` for every session in the subtree. Semantic memory and
/// knowledge live outside the conversation DB, so cross-session memory survives
/// the cleanup.
///
/// Idempotent — also cleans orphan rows that exist only in
/// `execution_logs.conversation_id` with no matching `sessions` row, so
/// the UI's `/api/logs/sessions` view (which surfaces such orphans) can
/// always reach a "delete makes it disappear" outcome. Returns 204
/// regardless of whether a `sessions` row was present; 500 on DB error.
pub async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    match state
        .state_service()
        .delete_session_recursive_cascade(&session_id)
    {
        Ok(rows) => {
            tracing::info!(session_id = %session_id, rows, "deleted session subtree");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(format!("Delete failed: {}", e))),
        )),
    }
}
