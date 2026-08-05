//! # Axum WebSocket handler
//!
//! Upgrade handler that runs the same session / subscription protocol as
//! [`super::handler::WebSocketHandler`] but lives on the HTTP router at
//! the `/ws` path. This unifies the gateway behind a single port so
//! firewalled mobile clients and simple reverse-proxy setups don't need
//! a second open port for WebSocket traffic.
//!
//! Clients connect to `ws://<host>:<http_port>/ws`.

use super::handler::{handle_client_message, WebSocketHandler};
use super::session::WsSession;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use gateway_ws_protocol::{ClientMessage, ServerMessage};

/// Axum route handler for `GET /ws`. Upgrades the HTTP connection to a
/// WebSocket, then runs the existing session protocol against it.
///
/// Plumbed via an `Extension` layer because the handler isn't part of
/// `AppState` (it's the owner of subscriptions/sessions and a peer of,
/// not a dependency of, the HTTP layer).
pub async fn axum_ws_upgrade_handler(
    ws: WebSocketUpgrade,
    Extension(handler): Extension<Arc<WebSocketHandler>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_axum_connection(socket, handler).await {
            warn!("Axum WebSocket connection error: {}", e);
        }
    })
}

/// Per-connection driver for Axum's `WebSocket` type. Session registration,
/// subscription routing, and client-message dispatch use the shared gateway
/// handler state.
async fn handle_axum_connection(
    socket: WebSocket,
    handler: Arc<WebSocketHandler>,
) -> Result<(), String> {
    let sessions = handler.sessions();
    let subscriptions = handler.subscriptions();

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();

    // Register session + subscribe to subscription manager.
    let session = WsSession::new(tx.clone());
    let session_id = sessions.register(session).await;
    subscriptions.connect(session_id.clone(), tx.clone()).await;

    // Initial Connected frame so the client knows which session it got.
    let connected_msg = ServerMessage::Connected {
        session_id: session_id.clone(),
    };
    let connected_text =
        serde_json::to_string(&connected_msg).map_err(|e| format!("serialize connected: {e}"))?;
    ws_tx
        .send(Message::Text(connected_text))
        .await
        .map_err(|e| format!("send connected: {e}"))?;

    // Outbound forwarder: channel → WebSocket.
    let forward_session_id = session_id.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(text) => {
                    if ws_tx.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to serialize ServerMessage: {}", e);
                }
            }
        }
        debug!("Axum message forwarder for {} stopped", forward_session_id);
    });

    // Inbound loop: WebSocket → client message handler. Axum wraps
    // incoming frames as `Ok(Message)` / `Err(axum::Error)`.
    while let Some(result) = ws_rx.next().await {
        match result {
            Ok(Message::Text(text)) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(client_msg) => {
                    if let Err(e) = handle_client_message(
                        &session_id,
                        client_msg,
                        &sessions,
                        &handler.runtime(),
                        subscriptions.clone(),
                        handler.agent_tasks(),
                    )
                    .await
                    {
                        warn!("Error handling message: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Invalid message format: {}", e);
                }
            },
            Ok(Message::Close(_)) => break,
            Ok(_) => { /* ping/pong/binary — axum handles keepalive itself */ }
            Err(e) => {
                error!("Axum WebSocket error: {}", e);
                break;
            }
        }
    }

    subscriptions.disconnect(&session_id).await;
    sessions.unregister(&session_id).await;
    info!("Axum session {} disconnected", session_id);

    Ok(())
}
