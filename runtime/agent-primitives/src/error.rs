//! # Error Types
//!
//! Unified error handling for agent primitives.

/// Unified error type for agent primitives.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// LLM-related errors
    #[error("LLM error: {0}")]
    Llm(String),

    /// Tool execution errors
    #[error("Tool error: {0}")]
    Tool(String),

    /// MCP errors
    #[error("MCP error: {0}")]
    Mcp(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),

    /// Serialization errors
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// I/O errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic errors
    #[error("{0}")]
    Generic(String),
}

/// Result type alias for agent primitive operations.
pub type Result<T> = std::result::Result<T, AgentError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AgentError::Llm("API call failed".to_string());
        assert_eq!(err.to_string(), "LLM error: API call failed");
    }
}
