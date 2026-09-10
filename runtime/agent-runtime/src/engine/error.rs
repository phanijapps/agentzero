//! Errors crossing the gateway-facing execution boundary.

/// Executor errors
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// Maximum iterations reached with no progress detected.
    #[error("Maximum iterations reached")]
    MaxIterationsReached,

    /// Cooperative stop — caller signaled the executor to stop via the
    /// optional `stop_flag` parameter on `execute_stream`. Distinct from
    /// `LlmError` so callers can short-circuit cleanup paths instead of
    /// treating it as a real failure.
    #[error("Execution stopped by caller")]
    Stopped,

    /// Maximum iterations reached but agent needs user intervention.
    #[error("Max iterations reached after {iterations_used} iterations: {reason}")]
    MaxIterationsNeedsIntervention {
        /// Total iterations consumed
        iterations_used: u32,
        /// Diagnosis of why the agent stopped
        reason: String,
    },

    /// LLM API error.
    #[error("LLM error: {0}")]
    LlmError(String),

    /// Tool execution error.
    #[error("Tool error: {0}")]
    ToolError(String),

    /// MCP server error.
    #[error("MCP error: {0}")]
    McpError(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Middleware pipeline error.
    #[error("Middleware error: {0}")]
    MiddlewareError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    // ------------- ExecutorError display -------------
    #[test]
    fn executor_error_messages() {
        assert_eq!(
            format!("{}", ExecutorError::MaxIterationsReached),
            "Maximum iterations reached"
        );
        assert_eq!(
            format!("{}", ExecutorError::Stopped),
            "Execution stopped by caller"
        );
        let with_intervention = ExecutorError::MaxIterationsNeedsIntervention {
            iterations_used: 5,
            reason: "stuck".to_string(),
        };
        let s = format!("{with_intervention}");
        assert!(s.contains("5"));
        assert!(s.contains("stuck"));
        assert!(format!("{}", ExecutorError::LlmError("e".into())).contains("LLM"));
        assert!(format!("{}", ExecutorError::ToolError("e".into())).contains("Tool"));
        assert!(format!("{}", ExecutorError::McpError("e".into())).contains("MCP"));
        assert!(format!("{}", ExecutorError::ConfigError("e".into())).contains("Configuration"));
        assert!(format!("{}", ExecutorError::MiddlewareError("e".into())).contains("Middleware"));
    }
}
