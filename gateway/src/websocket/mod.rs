//! # WebSocket Module
//!
//! WebSocket server for real-time agent communication.
//! Message types are provided by the `gateway-ws-protocol` crate.

mod axum_handler;
pub(crate) mod handler;
mod session;
mod subscriptions;

pub use axum_handler::axum_ws_upgrade_handler;

// Re-export message types from gateway-ws-protocol crate
pub use gateway_ws_protocol::{
    ClientMessage, ServerMessage, SubscriptionErrorCode, SubscriptionScope,
};

pub use handler::WebSocketHandler;
pub use session::{SessionRegistry, WsSession};
pub use subscriptions::{RoutingResult, SubscribeError, SubscribeResult, SubscriptionManager};
