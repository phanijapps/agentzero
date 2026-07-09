// ============================================================================
// MCP MANAGER
// ============================================================================

//! # MCP Manager
//!
//! Manager for MCP server connections and tool execution.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::client::McpClient;
use super::config::McpServerConfig;
use super::error::McpError;
use super::http::HttpMcpClient;
use super::sse::SseMcpClient;
use super::stdio::StdioMcpClient;
use super::tool::McpTool;

/// Manager for MCP server connections
pub struct McpManager {
    servers: RwLock<HashMap<String, Arc<dyn McpClient>>>,
}

impl McpManager {
    /// Create a new MCP manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
        }
    }

    /// Load MCP servers from configuration
    ///
    /// This is a placeholder - the application layer should provide
    /// server configurations through config injection.
    pub async fn load_servers(&self, _server_ids: &[String]) -> Result<(), McpError> {
        // TODO: Implement from existing code
        // The application layer should provide a way to load configs
        Ok(())
    }

    /// Start an MCP server connection
    pub async fn start_server(&self, config: McpServerConfig) -> Result<(), McpError> {
        match config {
            McpServerConfig::Stdio {
                id,
                name,
                command,
                args,
                env,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                let client = Arc::new(StdioMcpClient::new(
                    id.clone(),
                    name,
                    command,
                    args,
                    env.unwrap_or_default(),
                )?);
                self.servers.write().await.insert(id, client);
                Ok(())
            }
            McpServerConfig::Http {
                id,
                name,
                url,
                headers,
                auth: _,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                let client = Arc::new(HttpMcpClient::new(
                    id.clone(),
                    name,
                    url,
                    headers.unwrap_or_default(),
                ));
                self.servers.write().await.insert(id, client);
                Ok(())
            }
            McpServerConfig::Sse {
                id,
                name,
                url,
                headers,
                auth: _,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                let client = Arc::new(SseMcpClient::new(
                    id.clone(),
                    name,
                    url,
                    headers.unwrap_or_default(),
                ));
                self.servers.write().await.insert(id, client);
                Ok(())
            }
            McpServerConfig::StreamableHttp {
                id,
                name,
                url,
                headers,
                auth: _,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                // Streamable-http uses the same client as HTTP for now
                let client = Arc::new(HttpMcpClient::new(
                    id.clone(),
                    name,
                    url,
                    headers.unwrap_or_default(),
                ));
                self.servers.write().await.insert(id, client);
                Ok(())
            }
        }
    }

    /// Get an MCP client by ID
    pub async fn get_client(&self, id: &str) -> Option<Arc<dyn McpClient>> {
        self.servers.read().await.get(id).cloned()
    }

    /// Execute a tool on an MCP server
    pub async fn execute_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, McpError> {
        let client = self
            .get_client(server_id)
            .await
            .ok_or_else(|| McpError::ServerNotFound(server_id.to_string()))?;

        client.call_tool(tool_name, arguments).await
    }

    /// List all tools from all connected servers
    pub async fn list_all_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let mut all_tools = Vec::new();
        let servers = self.servers.read().await;

        for (_id, client) in servers.iter() {
            let tools = client.list_tools().await?;
            all_tools.extend(tools);
        }

        Ok(all_tools)
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
