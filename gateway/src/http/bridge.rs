//! # Bridge WebSocket Route
//!
//! Axum WebSocket upgrade handler for bridge worker connections.

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use gateway_bridge::WorkerSummary;

use crate::state::AppState;

/// WebSocket upgrade handler for bridge workers.
///
/// Workers connect to `GET /bridge/ws` and are upgraded to a WebSocket
/// connection. The handler delegates to `gateway_bridge::handle_worker_connection`
/// for the full lifecycle (Hello handshake, message loop, cleanup).
pub async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    // Reads through the transport group: registry + outbox + bus.
    let transport = state.transport();
    let registry = transport.bridge_registry.clone();
    let outbox = transport.bridge_outbox.clone();
    let bus = transport.bridge_bus.clone();

    ws.on_upgrade(move |socket| {
        gateway_bridge::handle_worker_connection(socket, registry, outbox, bus)
    })
}

/// List all connected bridge workers.
pub async fn list_workers(State(state): State<AppState>) -> Json<Vec<WorkerSummary>> {
    let workers = state.bridge_registry().list_entries().await;
    Json(workers)
}
