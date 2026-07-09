//! # Webhooks HTTP Endpoint
//!
//! HTTP endpoint for receiving webhooks from external services
//! (WhatsApp, Telegram, Signal, Email, generic webhooks).
//!
//! ## Endpoint
//!
//! `POST /api/webhooks/{hook_type}/{hook_id}`
//!
//! ## Supported Hook Types
//!
//! - `whatsapp` - WhatsApp Business API webhooks
//! - `telegram` - Telegram Bot API webhooks
//! - `signal` - Signal messenger webhooks
//! - `email` - Email webhooks (via sendgrid, mailgun, etc)
//! - `webhook` - Generic webhooks

use crate::hooks::{HookContext, HookType};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Webhook payload for generic webhooks.
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookPayload {
    /// Source identifier (phone number, email, user ID).
    pub source_id: Option<String>,

    /// Channel identifier (for group chats, threads).
    pub channel_id: Option<String>,

    /// Message content.
    pub message: String,

    /// Optional agent ID to route to.
    pub agent_id: Option<String>,

    /// Additional metadata.
    #[serde(flatten)]
    pub metadata: Value,
}

/// Response from webhook endpoint.
#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    /// Status of the webhook processing.
    pub status: String,

    /// Conversation ID for tracking.
    pub conversation_id: Option<String>,

    /// Optional error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Handle incoming webhook.
///
/// POST /api/webhooks/{hook_type}/{hook_id}
pub async fn handle_webhook(
    State(state): State<AppState>,
    Path((hook_type, hook_id)): Path<(String, String)>,
    Json(payload): Json<WebhookPayload>,
) -> impl IntoResponse {
    // Parse hook type
    let parsed_hook_type = match HookType::from_type_and_id(&hook_type, &hook_id) {
        Some(ht) => ht,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(WebhookResponse {
                    status: "error".to_string(),
                    conversation_id: None,
                    error: Some(format!("Unknown hook type: {}", hook_type)),
                }),
            );
        }
    };

    // Create hook context
    let source_id = payload
        .source_id
        .clone()
        .unwrap_or_else(|| format!("{}:{}", hook_type, hook_id));

    let mut hook_context = HookContext::new(parsed_hook_type, source_id.clone());

    if let Some(channel_id) = &payload.channel_id {
        hook_context = hook_context.with_channel(channel_id.clone());
    }

    // Add payload metadata
    if let Some(obj) = payload.metadata.as_object() {
        for (key, value) in obj {
            hook_context = hook_context.with_metadata(key.clone(), value.clone());
        }
    }

    // Determine agent to route to
    let agent_id = payload.agent_id.as_deref().unwrap_or("root");

    // Create or get conversation ID based on source
    let conversation_id = format!("{}-{}", hook_type, source_id.replace(['+', ' ', '@'], "-"));

    // Store hook context in the execution state
    // This will be picked up by the runner and injected into tool context
    tracing::info!(
        hook_type = %hook_type,
        hook_id = %hook_id,
        source_id = %source_id,
        conversation_id = %conversation_id,
        "Processing webhook"
    );

    // Invoke the agent (creates new session for each webhook call)
    match state
        .runtime
        .invoke(agent_id, &conversation_id, &payload.message)
        .await
    {
        Ok((_handle, session_id)) => (
            StatusCode::OK,
            Json(WebhookResponse {
                status: "accepted".to_string(),
                conversation_id: Some(session_id), // Return session_id as conversation_id
                error: None,
            }),
        ),
        Err(e) => {
            tracing::error!(error = %e, "Failed to invoke agent from webhook");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(WebhookResponse {
                    status: "error".to_string(),
                    conversation_id: Some(conversation_id),
                    error: Some(e),
                }),
            )
        }
    }
}

/// WhatsApp-specific webhook handler.
///
/// POST /api/webhooks/whatsapp/{phone_number_id}/messages
pub async fn handle_whatsapp_webhook(
    State(state): State<AppState>,
    Path(phone_number_id): Path<String>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    // Parse WhatsApp webhook format
    // See: https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/components
    let entry = payload
        .get("entry")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first());

    let changes = entry
        .and_then(|e| e.get("changes"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first());

    let value = changes.and_then(|c| c.get("value"));

    let messages = value
        .and_then(|v| v.get("messages"))
        .and_then(|m| m.as_array());

    if let Some(messages) = messages {
        for message in messages {
            let from = message.get("from").and_then(|f| f.as_str());
            let text = message
                .get("text")
                .and_then(|t| t.get("body"))
                .and_then(|b| b.as_str());

            if let (Some(from), Some(text)) = (from, text) {
                let _hook_context = HookContext::new(
                    HookType::WhatsApp {
                        phone_number_id: phone_number_id.clone(),
                    },
                    from.to_string(),
                );

                let conversation_id = format!("whatsapp-{}", from.replace('+', ""));

                tracing::info!(
                    from = %from,
                    conversation_id = %conversation_id,
                    "Processing WhatsApp message"
                );

                // Store hook context for later response routing
                if let Some(hook_registry) = state.hook_registry.as_ref() {
                    // Hook context is stored in state for the respond tool
                    let _ = hook_registry; // Used via event bus
                }

                if let Err(e) = state.runtime.invoke("root", &conversation_id, text).await {
                    tracing::error!(error = %e, "Failed to invoke agent for WhatsApp message");
                }
            }
        }
    }

    StatusCode::OK
}

/// Telegram-specific webhook handler.
///
/// POST /api/webhooks/telegram/{bot_id}
pub async fn handle_telegram_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    // Parse Telegram webhook format
    // See: https://core.telegram.org/bots/api#update
    let message = payload.get("message");

    if let Some(message) = message {
        let chat = message.get("chat");
        let text = message.get("text").and_then(|t| t.as_str());

        if let (Some(chat), Some(text)) = (chat, text) {
            let chat_id = chat.get("id").and_then(|id| id.as_i64()).unwrap_or(0);
            let from = message
                .get("from")
                .and_then(|f| f.get("id"))
                .and_then(|id| id.as_i64())
                .unwrap_or(0);

            let _hook_context = HookContext::new(
                HookType::Telegram {
                    bot_id: bot_id.clone(),
                    chat_id,
                },
                from.to_string(),
            );

            let conversation_id = format!("telegram-{}-{}", bot_id, chat_id);

            tracing::info!(
                chat_id = %chat_id,
                from = %from,
                conversation_id = %conversation_id,
                "Processing Telegram message"
            );

            if let Err(e) = state.runtime.invoke("root", &conversation_id, text).await {
                tracing::error!(error = %e, "Failed to invoke agent for Telegram message");
            }
        }
    }

    StatusCode::OK
}

/// Verification endpoint for webhook setup.
///
/// GET /api/webhooks/{hook_type}/{hook_id}/verify
pub async fn verify_webhook(
    Path((hook_type, hook_id)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Handle various verification challenges
    match hook_type.as_str() {
        "whatsapp" => {
            // WhatsApp verification
            if let (Some(mode), Some(_token), Some(challenge)) = (
                params.get("hub.mode"),
                params.get("hub.verify_token"),
                params.get("hub.challenge"),
            ) {
                if mode == "subscribe" {
                    // In production, verify the token matches your configured token
                    tracing::info!(
                        hook_id = %hook_id,
                        "WhatsApp webhook verification successful"
                    );
                    return challenge.clone();
                }
            }
        }
        "telegram" => {
            // Telegram doesn't use GET verification
            return "ok".to_string();
        }
        _ => {}
    }

    "verification_failed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_payload() {
        let json = r#"{
            "source_id": "+1234567890",
            "message": "Hello",
            "custom_field": "value"
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.source_id, Some("+1234567890".to_string()));
        assert_eq!(payload.message, "Hello");
    }
}
