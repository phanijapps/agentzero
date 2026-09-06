// ============================================================================
// LOGGING MODULE
// Structured logging configuration
// ============================================================================

//! # Logging Module
//!
//! Structured logging utilities for the agent runtime framework.
//!
//! ## Features
//!
//! - Configurable log levels (Error, Warn, Info, Debug, Trace)
//! - Optional file and line number information
//! - Environment variable support via `RUST_LOG`
//! - Convenient macros for logging with context
//!
//! ## Usage
//!
//! See the README.md file for detailed usage examples.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Keep third-party MCP and Rig payloads out of application diagnostics.
/// Apply as a separate filter so an environment directive cannot override it.
pub fn safe_runtime_diagnostics(metadata: &tracing::Metadata<'_>) -> bool {
    // rmcp traces raw protocol events; sse-stream even includes malformed
    // lines in warnings. Rig records raw prompts, tool arguments and results
    // before host hooks run. AgentZero emits bounded diagnostics and typed
    // execution events at its own boundaries instead of these raw payloads.
    let root = metadata.target().split("::").next().unwrap_or_default();
    !matches!(root, "rmcp" | "sse_stream")
        && !crate::rig_adapter::PAYLOAD_DIAGNOSTIC_TARGETS.contains(&root)
}

/// Log level for the application
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogLevel {
    /// Error level
    Error,
    /// Warning level
    Warn,
    /// Info level
    #[default]
    Info,
    /// Debug level
    Debug,
    /// Trace level
    Trace,
}

impl LogLevel {
    /// Convert to tracing Level
    #[must_use]
    pub const fn as_tracing(&self) -> tracing::Level {
        match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        }
    }

    /// Parse from string
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }
}

/// Initialize logging for the agent runtime
///
/// # Arguments
/// * `level` - The minimum log level to display
/// * `with_file` - Whether to include file/line info (default: false for performance)
pub fn init_logging(level: LogLevel, with_file: bool) {
    let env_filter = EnvFilter::builder()
        .with_default_directive(level.as_tracing().into())
        .from_env_lossy();

    let registry = tracing_subscriber::registry().with(env_filter).with(
        tracing_subscriber::filter::filter_fn(safe_runtime_diagnostics),
    );

    if with_file {
        registry
            .with(
                fmt::layer()
                    .with_file(true)
                    .with_line_number(true)
                    .with_target(true),
            )
            .init();
    } else {
        registry
            .with(
                fmt::layer()
                    .with_file(false)
                    .with_line_number(false)
                    .with_target(false),
            )
            .init();
    }
}

/// Initialize logging from environment variable
///
/// Reads the `RUST_LOG` environment variable to set the log level.
/// If not set, defaults to `INFO`.
pub fn init_logging_from_env(with_file: bool) {
    let env_filter = EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env_lossy();

    let registry = tracing_subscriber::registry().with(env_filter).with(
        tracing_subscriber::filter::filter_fn(safe_runtime_diagnostics),
    );

    if with_file {
        registry
            .with(
                fmt::layer()
                    .with_file(true)
                    .with_line_number(true)
                    .with_target(true),
            )
            .init();
    } else {
        registry
            .with(
                fmt::layer()
                    .with_file(false)
                    .with_line_number(false)
                    .with_target(false),
            )
            .init();
    }
}

/// Macro for logging with appropriate context
///
/// These macros are convenience wrappers around tracing macros
/// that automatically include relevant context.
#[macro_export]
macro_rules! agent_info {
    ($($arg:tt)*) => {
        tracing::info!($($arg)*);
    };
}

#[macro_export]
macro_rules! agent_warn {
    ($($arg:tt)*) => {
        tracing::warn!($($arg)*);
    };
}

#[macro_export]
macro_rules! agent_error {
    ($($arg:tt)*) => {
        tracing::error!($($arg)*);
    };
}

#[macro_export]
macro_rules! agent_debug {
    ($($arg:tt)*) => {
        tracing::debug!($($arg)*);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("Debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("invalid"), None);
    }
}
