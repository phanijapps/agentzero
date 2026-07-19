//! # Agent Primitives
//!
//! Shared traits, types, and errors for zbot agent execution.
//!
//! ## Overview
//!
//! This crate provides the shared runtime primitives that sit outside Rig:
//!
//! - [`Tool`] - Tool execution interface
//! - [`ToolContext`] - Context provided to tools
//! - [`EventActions`] - tool-triggered side effects consumed by the gateway
//! - [`Part`] and [`Content`] - text and multimodal message content
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use agent_primitives::{Result, Tool, ToolContext};
//!
//! // All tools implement the Tool trait
//! ```

// ============================================================================
// HTTP USER-AGENT
// ============================================================================

/// User-Agent string attached to every outbound HTTP request made by the
/// z-bot system. Defined once here so all callers (LLM clients, MCP, gateway
/// connectors, CLI, tools) share the same value. Version tracks this crate's
/// Cargo.toml.
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; Z-bot/",
    env!("CARGO_PKG_VERSION"),
    "; +https://github.com/phanijapps/zbot)"
);

// ============================================================================
// PUBLIC API RE-EXPORTS
// ============================================================================

pub mod callbacks;
pub mod connectors;
pub mod context;
pub mod error;
pub mod event;
pub mod filesystem;
pub mod multimodal;
pub mod policy;
pub mod tool;
pub mod types;

// ============================================================================
// CONVENIENCE RE-EXPORTS
// ============================================================================

pub use callbacks::{AfterAgentCallback, BeforeAgentCallback};
pub use connectors::{CapabilityInfo, ConnectorResourceProvider};
pub use context::{CallbackContext, ReadonlyContext, ToolContext};
pub use error::{AgentError, Result};
pub use event::{DelegateAction, Event, EventActions, RespondAction};
pub use filesystem::{FileSystemContext, NoFileSystemContext};
pub use policy::{ToolPermissions, ToolRiskLevel};
pub use tool::Tool;
pub use types::{Content, Part};

// ============================================================================
// STRING UTILITIES
// ============================================================================

/// Truncate a string to at most `max_bytes` bytes at a valid UTF-8 char boundary.
/// Returns the original string if it fits, otherwise the largest valid slice.
#[inline]
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        s
    } else {
        &s[..s.floor_char_boundary(max_bytes)]
    }
}

// ============================================================================
// STATE PREFIX CONSTANTS
// ============================================================================

/// Key prefix for user preferences (persists across sessions)
pub const KEY_PREFIX_USER: &str = "user:";

/// Key prefix for application state (application-wide)
pub const KEY_PREFIX_APP: &str = "app:";

/// Key prefix for temporary data (cleared each turn)
pub const KEY_PREFIX_TEMP: &str = "temp:";
