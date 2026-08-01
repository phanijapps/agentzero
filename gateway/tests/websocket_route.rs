//! Integration coverage for the client WebSocket route on the unified HTTP listener.

mod common;

use std::sync::Arc;

use axum_test::TestServer;
use gateway::{http::create_http_router, websocket::WebSocketHandler, GatewayConfig};
use gateway_ws_protocol::{ClientMessage, ServerMessage};

#[tokio::test]
async fn ws_route_upgrades_and_routes_ping() {
    let (_dir, state) = common::make_state();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let router = create_http_router(GatewayConfig::default(), state, ws_handler);
    let server = TestServer::builder()
        .http_transport()
        .build(router)
        .expect("test server");

    let mut websocket = server.get_websocket("/ws").await.into_websocket().await;

    let connected = websocket.receive_json::<ServerMessage>().await;
    let session_id = match connected {
        ServerMessage::Connected { session_id } => session_id,
        other => panic!("expected connected message, got {other:?}"),
    };
    assert!(!session_id.is_empty());

    websocket.send_json(&ClientMessage::Ping).await;
    assert!(matches!(
        websocket.receive_json::<ServerMessage>().await,
        ServerMessage::Pong
    ));

    websocket.close().await;
}
