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
use super::native::NativeMcpClient;
use super::tool::McpTool;

/// Host-owned observer for a safe, canonical MCP startup/discovery failure.
/// The runtime never passes error text across this boundary.
pub type McpStartupFailureObserver = Arc<dyn Fn(&str) + Send + Sync>;

/// Manager for MCP server connections
pub struct McpManager {
    servers: RwLock<HashMap<String, Arc<dyn McpClient>>>,
    startup_failure_observer: Option<McpStartupFailureObserver>,
    closed: tokio::sync::watch::Sender<bool>,
}

impl McpManager {
    /// Create a new MCP manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            startup_failure_observer: None,
            closed: tokio::sync::watch::channel(false).0,
        }
    }

    /// Register a host-side audit observer for nonfatal startup failures.
    #[must_use]
    pub fn with_startup_failure_observer(mut self, observer: McpStartupFailureObserver) -> Self {
        self.startup_failure_observer = Some(observer);
        self
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
        let mut closed = self.closed.subscribe();
        tokio::select! {
        biased;
        _ = closed.wait_for(|value| *value) => Err(McpError::ProtocolError("MCP session closed".into())),
        result = self.start_server_inner(config) => result }
    }

    async fn install(&self, id: String, client: Arc<dyn McpClient>) -> Result<(), McpError> {
        let previous = {
            let mut servers = self.servers.write().await;
            if *self.closed.borrow() {
                drop(servers);
                client.close().await;
                return Err(McpError::ProtocolError("MCP session closed".into()));
            }
            servers.insert(id, client)
        };
        if let Some(previous) = previous {
            previous.close().await;
        }
        Ok(())
    }

    async fn start_server_inner(&self, config: McpServerConfig) -> Result<(), McpError> {
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
                let client = Arc::new(
                    NativeMcpClient::stdio(name, command, args, env.unwrap_or_default()).await?,
                );
                self.install(id, client).await
            }
            McpServerConfig::Http {
                id,
                name,
                url,
                headers,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                let client = Arc::new(HttpMcpClient::new(
                    id.clone(),
                    name,
                    url,
                    headers.unwrap_or_default(),
                ));
                self.install(id, client).await
            }
            McpServerConfig::Sse {
                id,
                name,
                url,
                headers,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                // Both configured POST transports accept JSON and SSE bodies.
                // Share decoding, timeouts and credential redaction.
                let client = Arc::new(HttpMcpClient::new(
                    id.clone(),
                    name,
                    url,
                    headers.unwrap_or_default(),
                ));
                self.install(id, client).await
            }
            McpServerConfig::StreamableHttp {
                id,
                name,
                url,
                headers,
                ..
            } => {
                let id = id.unwrap_or_else(|| name.clone());
                let client = Arc::new(
                    NativeMcpClient::streamable(name, url, headers.unwrap_or_default()).await?,
                );
                self.install(id, client).await
            }
        }
    }

    /// Get an MCP client by ID
    pub async fn get_client(&self, id: &str) -> Option<Arc<dyn McpClient>> {
        self.servers.read().await.get(id).cloned()
    }

    /// Remove a server after a failed discovery handshake and notify the host
    /// using only its canonical configured ID. Removing it prevents retries on
    /// later turns in the same executor.
    pub async fn mark_startup_failed(&self, id: &str) {
        let removed = self.servers.write().await.remove(id);
        if let Some(client) = removed {
            client.close().await;
        }
        self.notify_startup_failure(id);
    }

    /// Report a startup failure that occurred before a client was inserted.
    pub fn notify_startup_failure(&self, id: &str) {
        if let Some(observer) = &self.startup_failure_observer {
            observer(id);
        }
    }

    #[cfg(test)]
    pub(crate) async fn insert_test_client(&self, id: &str, client: Arc<dyn McpClient>) {
        self.servers.write().await.insert(id.to_string(), client);
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
        let servers: Vec<_> = self.servers.read().await.values().cloned().collect();

        for client in servers {
            let tools = client.list_tools().await?;
            all_tools.extend(tools);
        }

        Ok(all_tools)
    }

    /// Close this session permanently, interrupting requests before waiting for
    /// each transport's bounded cleanup. Never hold the registry lock over I/O.
    pub async fn close(&self) {
        self.closed.send_replace(true);
        let clients: Vec<_> = self
            .servers
            .write()
            .await
            .drain()
            .map(|(_, client)| client)
            .collect();
        for client in &clients {
            client.cancel();
        }
        // Each task owns its client until cleanup completes. Canceling the
        // caller drops join handles, not cleanup futures or session owners.
        let cleanup: Vec<_> = clients
            .into_iter()
            .map(|client| {
                tokio::spawn(async move {
                    if tokio::time::timeout(std::time::Duration::from_secs(5), client.close())
                        .await
                        .is_err()
                    {
                        tracing::warn!("MCP client cleanup exceeded its budget");
                    }
                })
            })
            .collect();
        futures::future::join_all(cleanup).await;
    }
}

impl Drop for McpManager {
    fn drop(&mut self) {
        self.closed.send_replace(true);
        for client in self.servers.get_mut().values() {
            client.cancel();
        }
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
