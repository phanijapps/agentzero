//! # Conversation Endpoints
//!
//! CRUD operations for conversations.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Conversation response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationResponse {
    pub id: String,
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub title: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(rename = "messageCount")]
    pub message_count: u32,
}

/// Message response.
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub metadata: Option<Value>,
}

/// Create conversation request.
#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub title: Option<String>,
}

/// GET /api/conversations - List all conversations.
///
/// Note: This will be connected to the session database in Phase 3b.
pub async fn list_conversations(State(_state): State<AppState>) -> Json<Vec<ConversationResponse>> {
    // TODO: Connect to daily_sessions in Phase 3b
    Json(vec![])
}

/// POST /api/conversations - Create a new conversation.
pub async fn create_conversation(
    State(_state): State<AppState>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<ConversationResponse>, StatusCode> {
    // TODO: Connect to daily_sessions in Phase 3b
    let _ = (&request.agent_id, request.title.as_deref());
    Err(StatusCode::NOT_IMPLEMENTED)
}

/// GET /api/conversations/:id - Get a conversation by ID.
pub async fn get_conversation(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> Result<Json<ConversationResponse>, StatusCode> {
    // TODO: Connect to daily_sessions in Phase 3b
    Err(StatusCode::NOT_FOUND)
}

/// DELETE /api/conversations/:id - Delete a conversation.
pub async fn delete_conversation(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> StatusCode {
    // TODO: Connect to daily_sessions in Phase 3b
    StatusCode::NOT_IMPLEMENTED
}

/// GET /api/conversations/:id/messages - List messages in a conversation.
pub async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<MessageResponse>>, StatusCode> {
    match state.messages.replay(&id, None, 500) {
        Ok(messages) => {
            let responses: Vec<MessageResponse> = messages
                .into_iter()
                .map(|m| MessageResponse {
                    id: m.id,
                    role: m.role,
                    content: m.content,
                    timestamp: m.created_at,
                    metadata: None,
                })
                .collect();
            Ok(Json(responses))
        }
        Err(e) => {
            tracing::warn!("Failed to get messages for conversation {}: {}", id, e);
            // Return empty array if conversation doesn't exist yet
            Ok(Json(vec![]))
        }
    }
}
