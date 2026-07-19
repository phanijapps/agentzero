// ============================================================================
// STDIO MCP CLIENT
// ============================================================================

//! # STDIO MCP Client
//!
//! Stdio transport implementation for MCP clients (subprocess communication).

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

use super::client::McpClient;
use super::error::McpError;
use super::tool::McpTool;

/// Wall-clock budget for an entire stdio MCP exchange (spawn + init +
/// request + response). A wedged Node MCP server used to park a tokio
/// worker forever; this caps it. 60 s is generous enough for slow tool
/// implementations while bounding the worst case. The spawned child
/// uses `kill_on_drop` so timing out actually frees the process —
/// otherwise we'd leak zombies.
const STDIO_MCP_TIMEOUT: Duration = Duration::from_secs(60);

/// Stdio-based MCP client (subprocess communication)
pub(super) struct StdioMcpClient {
    #[allow(dead_code)] // Reserved for future connection tracking
    id: String,
    name: String,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
}

impl StdioMcpClient {
    pub(super) fn new(
        id: String,
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<Self, McpError> {
        tracing::debug!(mcp_id = %id, "Created STDIO MCP client");
        Ok(Self {
            id,
            name,
            command,
            args,
            env,
        })
    }

    /// Spawn the MCP server process and execute a tool call
    async fn spawn_and_call(&self, tool_name: &str, arguments: &Value) -> Result<Value, McpError> {
        tracing::debug!(mcp_id = %self.id, "Starting STDIO MCP request");

        // Build the command - on Windows, use cmd.exe to properly resolve PATH
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd.exe");
            // Build the full command string with proper escaping
            let full_cmd = if self.args.is_empty() {
                self.command.clone()
            } else {
                format!("{} {}", self.command, self.args.join(" "))
            };
            c.args(["/c", &full_cmd]);
            c.kill_on_drop(true);
            c
        };

        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new(&self.command);
            c.args(&self.args);
            c.kill_on_drop(true);
            c
        };

        // Set environment variables if provided
        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        // Create JSON-RPC requests for initialization and tool call
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "agent-runtime",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });

        let initialized_notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        let tool_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });

        tracing::debug!(mcp_id = %self.id, "Sending STDIO MCP tool request");

        // Spawn the process and communicate via stdin/stdout
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|_| spawn_mcp_error())?;

        // Write all requests to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;

            // Send initialize request
            let init_str = format!("{init_request}\n");
            stdin.write_all(init_str.as_bytes()).await.map_err(|_| {
                McpError::ProtocolError("Failed to write MCP initialization".to_string())
            })?;
            stdin.flush().await.map_err(|_| {
                McpError::ProtocolError("Failed to initialize MCP request".to_string())
            })?;

            // Send initialized notification
            let notif_str = format!("{initialized_notification}\n");
            stdin.write_all(notif_str.as_bytes()).await.map_err(|_| {
                McpError::ProtocolError("Failed to notify MCP initialization".to_string())
            })?;
            stdin.flush().await.map_err(|_| {
                McpError::ProtocolError("Failed to initialize MCP request".to_string())
            })?;

            // Send tool call request
            let tool_str = format!("{tool_request}\n");
            stdin.write_all(tool_str.as_bytes()).await.map_err(|_| {
                McpError::ProtocolError("Failed to send MCP tool request".to_string())
            })?;
            stdin.flush().await.map_err(|_| {
                McpError::ProtocolError("Failed to send MCP tool request".to_string())
            })?;
        }

        // Read response from stdout
        let output = child.wait_with_output().await.map_err(|_| {
            McpError::ProtocolError("Failed to read MCP process response".to_string())
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        tracing::debug!(
            mcp_id = %self.id,
            success = output.status.success(),
            "STDIO MCP process completed"
        );

        if !output.status.success() {
            return Err(McpError::ProtocolError("MCP process failed".to_string()));
        }

        // Parse JSON responses - we need to find the tool call response (id: 2)
        let mut tool_result = None;

        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(response) = serde_json::from_str::<Value>(line) {
                // Look for tool call response (id: 2)
                if response.get("id").and_then(serde_json::Value::as_i64) == Some(2) {
                    // Check for JSON-RPC error first
                    if response.get("error").is_some() {
                        return Err(McpError::ProtocolError(
                            "MCP returned a protocol error".to_string(),
                        ));
                    }

                    tool_result = response
                        .get("result")
                        .or_else(|| response.get("content"))
                        .cloned();
                }
            }
        }

        tool_result
            .ok_or_else(|| McpError::ProtocolError("No tool result in MCP response".to_string()))
    }

    /// List tools by spawning the process and calling tools/list
    async fn spawn_and_list(&self) -> Result<Vec<McpTool>, McpError> {
        tracing::debug!(mcp_id = %self.id, "Starting STDIO MCP tool discovery");

        // Build the command - on Windows, use cmd.exe to properly resolve PATH
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd.exe");
            // Build the full command string with proper escaping
            let full_cmd = if self.args.is_empty() {
                self.command.clone()
            } else {
                format!("{} {}", self.command, self.args.join(" "))
            };
            c.args(["/c", &full_cmd]);
            c.kill_on_drop(true);
            c
        };

        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new(&self.command);
            c.args(&self.args);
            c.kill_on_drop(true);
            c
        };

        // Set environment variables if provided
        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        // Create JSON-RPC requests for initialization and tools/list
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "agent-runtime",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });

        let initialized_notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        let tools_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });

        // Spawn the process
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|_| spawn_mcp_error())?;

        // Write all requests to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;

            // Send initialize request
            let init_str = format!("{init_request}\n");
            stdin.write_all(init_str.as_bytes()).await.map_err(|_| {
                McpError::ProtocolError("Failed to write MCP initialization".to_string())
            })?;
            stdin.flush().await.map_err(|_| {
                McpError::ProtocolError("Failed to initialize MCP request".to_string())
            })?;

            // Send initialized notification
            let notif_str = format!("{initialized_notification}\n");
            stdin.write_all(notif_str.as_bytes()).await.map_err(|_| {
                McpError::ProtocolError("Failed to notify MCP initialization".to_string())
            })?;
            stdin.flush().await.map_err(|_| {
                McpError::ProtocolError("Failed to initialize MCP request".to_string())
            })?;

            // Send tools/list request
            let tools_str = format!("{tools_request}\n");
            stdin.write_all(tools_str.as_bytes()).await.map_err(|_| {
                McpError::ProtocolError("Failed to send MCP discovery request".to_string())
            })?;
            stdin.flush().await.map_err(|_| {
                McpError::ProtocolError("Failed to send MCP discovery request".to_string())
            })?;
        }

        // Read response
        let output = child.wait_with_output().await.map_err(|_| {
            McpError::ProtocolError("Failed to read MCP process response".to_string())
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        tracing::debug!(
            mcp_id = %self.id,
            success = output.status.success(),
            "STDIO MCP discovery completed"
        );

        if !output.status.success() {
            return Err(McpError::ProtocolError("MCP process failed".to_string()));
        }

        // Parse JSON responses - we need to find the tools/list response
        let mut tools_array = None;

        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(response) = serde_json::from_str::<Value>(line) {
                // Skip initialize response (id: 1)
                if response.get("id").and_then(serde_json::Value::as_i64) == Some(1) {
                    continue;
                }

                // Look for tools/list response (id: 2)
                if response.get("id").and_then(serde_json::Value::as_i64) == Some(2) {
                    // Check for JSON-RPC error first
                    if response.get("error").is_some() {
                        return Err(McpError::ProtocolError(
                            "MCP returned a protocol error".to_string(),
                        ));
                    }

                    if let Some(tools) = response
                        .get("result")
                        .and_then(|v| v.get("tools"))
                        .and_then(|v| v.as_array())
                    {
                        tools_array = Some(tools.clone());
                    } else {
                        return Err(McpError::ProtocolError(
                            "No tools array in MCP response".to_string(),
                        ));
                    }
                }
            }
        }

        let tools_array = tools_array
            .ok_or_else(|| McpError::ProtocolError("No tools/list response found".to_string()))?;

        let mut tools = Vec::new();
        for tool in tools_array {
            let name = tool
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let description = tool
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let parameters = tool.get("inputSchema").cloned();

            tools.push(McpTool {
                name,
                description,
                parameters,
            });
        }

        Ok(tools)
    }
}

fn spawn_mcp_error() -> McpError {
    McpError::ConnectionFailed("Failed to spawn MCP process on daemon host".to_string())
}

#[async_trait]
impl McpClient for StdioMcpClient {
    fn name(&self) -> &str {
        &self.name
    }

    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, McpError> {
        match tokio::time::timeout(
            STDIO_MCP_TIMEOUT,
            self.spawn_and_call(tool_name, &arguments),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => Err(McpError::ProtocolError(format!(
                "STDIO MCP call_tool timed out after {}s ({})",
                STDIO_MCP_TIMEOUT.as_secs(),
                self.id,
            ))),
        }
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        match tokio::time::timeout(STDIO_MCP_TIMEOUT, self.spawn_and_list()).await {
            Ok(res) => res,
            Err(_) => Err(McpError::ProtocolError(format!(
                "STDIO MCP list_tools timed out after {}s ({})",
                STDIO_MCP_TIMEOUT.as_secs(),
                self.id,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_error_is_redacted_to_a_safe_host_diagnostic() {
        let message = spawn_mcp_error().to_string();

        assert!(message.contains("daemon host"));
        assert!(!message.contains("missing"));
    }
}
