// ============================================================================
// TYPES MODULE
// Shared data structures for the agent runtime framework
// ============================================================================

//! # Types Module
//!
//! Shared data structures used throughout the agent runtime framework.
//!
//! ## Submodules
//!
//! - [`messages`]: Chat message and tool call types
//! - [`events`]: Streaming event types for execution feedback

pub mod events;
pub mod messages;

// Re-export commonly used types
pub use events::{RecoveredFailureItem, StreamEvent};
pub use messages::{ChatMessage, ToolCall};

/// Result type for tool operations
pub type ToolResult = std::result::Result<serde_json::Value, ToolError>;

/// Error type for tool operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolError {
    /// Tool execution failed
    #[error("Tool execution failed: {0}")]
    ExecutionFailed(String),

    /// Tool not found
    #[error("Tool not found: {0}")]
    NotFound(String),

    /// Invalid tool arguments
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    /// Tool timeout
    #[error("Tool timed out")]
    Timeout,
}
