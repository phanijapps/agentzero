//! # Gateway
//!
//! HTTP and WebSocket gateway for the AgentZero daemon.
//!
//! This crate provides the network interface for the agent runtime,
//! enabling multiple clients (Tauri, CLI, web) to interact with agents.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │              Gateway                     │
//! ├─────────────────────────────────────────┤
//! │  WebSocket :18790  │  HTTP :18791       │
//! ├─────────────────────────────────────────┤
//! │           Event Bus (broadcast)         │
//! └─────────────────────────────────────────┘
//!              │
//!              ▼
//!     ┌─────────────────┐
//!     │  Agent Runtime  │
//!     └─────────────────┘
//! ```
//!
//! ## Features
//!
//! - **WebSocket API** - Real-time streaming for agent conversations
//! - **HTTP API** - RESTful endpoints for agents, conversations, tools
//! - **Event Bus** - Broadcast events to all connected clients

pub mod bus;
pub mod config;
pub mod connectors;
pub mod cron;
pub mod error;
pub mod events;
pub mod execution;
pub mod hooks;
pub mod http;
pub mod memory_llm_factory;
#[cfg(feature = "rig-parity-capture")]
#[doc(hidden)]
pub mod parity;
pub mod server;
pub mod services;
pub mod state;
pub mod templates;
pub mod websocket;

#[cfg(test)]
pub mod test_utils;

pub use bus::{BusError, GatewayBus, HttpGatewayBus, SessionHandle, SessionRequest};
pub use config::GatewayConfig;
pub use connectors::{ConnectorConfig, ConnectorRegistry, ConnectorService, DispatchContext};
pub use cron::{CronJobConfig, CronScheduler, CronService};
pub use error::{GatewayError, Result};
pub use execution::{
    DelegationContext, DelegationRegistry, ExecutionConfig, ExecutionHandle, ExecutionRunner,
};
pub use hooks::{Attachment, Hook, HookContext, HookRegistry, HookType, ResponseFormat};
pub use server::GatewayServer;
pub use services::{AgentRegistry, AgentService, RuntimeService};
pub use state::AppState;

/// Default WebSocket port
pub const DEFAULT_WS_PORT: u16 = 18790;

/// Default HTTP port
pub const DEFAULT_HTTP_PORT: u16 = 18791;
