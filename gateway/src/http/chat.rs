//! # Chat Session Endpoints
//!
//! HTTP API for persistent chat session initialization and message history.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// REQUEST / RESPONSE TYPES
// ============================================================================

/// Response for POST /api/chat/init.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatInitResponse {
    pub session_id: String,
    pub conversation_id: String,
    pub created: bool,
}

/// Query parameters for GET /api/sessions/:id/messages.
#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub limit: Option<u32>,
}

/// A single message in the session history response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageResponse {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<String>,
    pub timestamp: String,
}

// ============================================================================
// ENDPOINTS
// ============================================================================

/// POST /api/chat/init
///
/// Returns (or creates) the persistent chat session. Idempotent and
/// self-healing: if the cached session id in settings points at a row
/// that no longer exists in the database (orphaned slot), we rebuild
/// both the session row and the cached ids. Fresh installs create a new
/// session on first call.
pub async fn init_chat_session(
    State(state): State<AppState>,
) -> Result<Json<ChatInitResponse>, (StatusCode, String)> {
    let settings = state
        .settings
        .get_execution_settings()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let runner = state.runtime.runner().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Runtime not available".to_string(),
        )
    })?;
    let state_service = runner.state_service();

    // Reuse the cached session only when its DB row actually exists.
    if let (Some(session_id), Some(conv_id)) =
        (&settings.chat.session_id, &settings.chat.conversation_id)
    {
        let row_present = matches!(state_service.get_session(session_id), Ok(Some(_)));
        if row_present {
            return Ok(Json(ChatInitResponse {
                session_id: session_id.clone(),
                conversation_id: conv_id.clone(),
                created: false,
            }));
        }
        tracing::warn!(
            "chat session {} cached in settings but missing in DB — rebuilding",
            session_id
        );
    }

    let session_id = format!("sess-chat-{}", uuid::Uuid::new_v4());
    let conversation_id = format!("chat-{}", uuid::Uuid::new_v4());

    let mut session =
        execution_state::Session::new_with_source("root", execution_state::TriggerSource::Web);
    session.id = session_id.clone();
    state_service.create_session_from(&session).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create chat session: {}", e),
        )
    })?;
    state_service
        .set_session_mode(&session_id, "fast")
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to set chat mode: {}", e),
            )
        })?;

    let mut updated_settings = settings.clone();
    updated_settings.chat = gateway_services::ChatConfig {
        session_id: Some(session_id.clone()),
        conversation_id: Some(conversation_id.clone()),
    };
    state
        .settings
        .update_execution_settings(updated_settings)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(ChatInitResponse {
        session_id,
        conversation_id,
        created: true,
    }))
}

/// DELETE /api/chat/session
///
/// Hard-deletes the current reserved chat session, every descendant
/// subagent session it spawned, and all per-session data for that
/// subtree, then clears the `settings.chat` slot. Memory facts and
/// the knowledge graph are preserved — only the conversation rows
/// disappear. The next call to `POST /api/chat/init` self-heals into
/// a fresh session.
pub async fn clear_chat_session(
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, String)> {
    let settings = state
        .settings
        .get_execution_settings()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // No-op if the slot is already empty — still return 204 for idempotency.
    if settings.chat.session_id.is_none() && settings.chat.conversation_id.is_none() {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Cascade-delete the subtree before clearing the slot. If the cascade
    // fails, leave the slot intact so the next clear can retry — partial
    // success (slot cleared but rows still in DB) would be worse.
    if let Some(session_id) = settings.chat.session_id.clone() {
        let runner = state.runtime.runner().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Runtime not available".to_string(),
            )
        })?;
        let state_service = runner.state_service();
        let rows = state_service
            .delete_session_recursive_cascade(&session_id)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to delete chat session subtree: {}", e),
                )
            })?;
        tracing::info!(session_id = %session_id, rows, "cleared chat session subtree");
    }

    let mut updated = settings.clone();
    updated.chat = gateway_services::ChatConfig::default();
    state
        .settings
        .update_execution_settings(updated)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/sessions/:id/messages?limit=100
///
/// Returns messages for a session, ordered by timestamp (oldest first).
/// Used by the chat UI to load history on mount.
pub async fn get_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> Result<Json<Vec<SessionMessageResponse>>, (StatusCode, String)> {
    let limit = query.limit.unwrap_or(100);

    let messages = state
        .messages
        .replay(&session_id, None, limit as usize)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load messages: {}", e),
            )
        })?;

    let response: Vec<SessionMessageResponse> = messages
        .into_iter()
        .map(|m| SessionMessageResponse {
            id: m.id,
            role: m.role,
            content: m.content,
            tool_calls: m.tool_calls,
            tool_results: None,
            timestamp: m.created_at,
        })
        .collect();

    Ok(Json(response))
}
