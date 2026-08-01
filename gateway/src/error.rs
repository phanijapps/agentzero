//! # Gateway Errors
//!
//! Error types for gateway operations.

use thiserror::Error;

/// Gateway error type.
#[derive(Error, Debug)]
pub enum GatewayError {
    /// HTTP error.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Agent not found.
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    /// Conversation not found.
    #[error("Conversation not found: {0}")]
    ConversationNotFound(String),

    /// Invalid request.
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias for gateway operations.
pub type Result<T> = std::result::Result<T, GatewayError>;
