// ============================================================================
// MCP CLIENT TRAIT
// ============================================================================

//! # MCP Client Trait
//!
//! Common trait for all MCP client implementations.

use async_trait::async_trait;
use serde_json::Value;

use super::error::McpError;
use super::tool::McpTool;

/// Trait for MCP client implementations
#[async_trait]
pub trait McpClient: Send + Sync {
    /// Stop accepting work and interrupt pending requests synchronously.
    fn cancel(&self) {}

    /// Release the owning session and wait for bounded transport cleanup.
    async fn close(&self) {
        self.cancel();
    }

    /// Get the client name
    fn name(&self) -> &str;

    /// Call a tool on this MCP server
    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, McpError>;

    /// List available tools from this MCP server
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError>;
}
