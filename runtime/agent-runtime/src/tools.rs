// ============================================================================
// TOOLS MODULE
// Tool execution system
// ============================================================================

//! # Tools Module
//!
//! Extensible tool registry and execution framework.
//!
//! ## Submodules
//!
//! - [`registry`]: Tool registry for managing available tools
//! - [`builtin`]: Built-in tools provided by the framework
//! - [`context`]: Execution context for tool operations
//! - [`error`]: Error types for tool operations
//! - [`respond`]: Tool for sending responses to originating hooks
//! - [`delegate`]: Tool for delegating tasks to subagents

pub mod builtin;
pub mod context;
pub mod delegate;
pub mod error;
pub mod registry;
pub mod respond;
pub mod run_procedure;

pub use builtin::{FileSystemContext, NoFileSystemContext};
pub use context::ToolContext;
pub use delegate::DelegateTool;
pub use error::{ToolError as ToolExecError, ToolResult as ToolExecResult};
pub use registry::ToolRegistry;
pub use respond::RespondTool;
pub use run_procedure::RunProcedureTool;

// Re-export from types
pub use crate::types::ToolCall;

// Re-export agent_primitives::Tool as the standard tool trait
pub use agent_primitives::Tool;
