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
        tracing::debug!(
            "Creating STDIO MCP client: {} with command: {}",
            name,
            command
        );
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
        tracing::debug!(
            "STDIO MCP spawning: {} with args: {:?}",
            self.command,
            self.args
        );

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

        tracing::debug!(
            "STDIO MCP sending tool call: {} with args: {}",
            tool_name,
            arguments
        );

        // Spawn the process and communicate via stdin/stdout
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| spawn_mcp_error(&self.command, e))?;

        // Write all requests to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;

            // Send initialize request
            let init_str = format!("{init_request}\n");
            stdin.write_all(init_str.as_bytes()).await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to write init to stdin: {e}"))
            })?;
            stdin
                .flush()
                .await
                .map_err(|e| McpError::ProtocolError(format!("Failed to flush init: {e}")))?;

            // Send initialized notification
            let notif_str = format!("{initialized_notification}\n");
            stdin.write_all(notif_str.as_bytes()).await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to write notification to stdin: {e}"))
            })?;
            stdin.flush().await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to flush notification: {e}"))
            })?;

            // Send tool call request
            let tool_str = format!("{tool_request}\n");
            stdin.write_all(tool_str.as_bytes()).await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to write tool request to stdin: {e}"))
            })?;
            stdin.flush().await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to flush tool request: {e}"))
            })?;
        }

        // Read response from stdout
        let output = child.wait_with_output().await.map_err(|e| {
            McpError::ProtocolError(format!("Failed to read from MCP process: {e}"))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        tracing::debug!("STDIO MCP process exited with: {:?}", output.status);
        tracing::debug!("STDIO MCP stdout: {}", stdout);
        if !stderr.is_empty() {
            tracing::debug!("STDIO MCP stderr: {}", stderr);
        }

        if !output.status.success() {
            return Err(McpError::ProtocolError(format!(
                "MCP process failed: {stderr}"
            )));
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
                    if let Some(error) = response.get("error") {
                        return Err(McpError::ProtocolError(format!("MCP error: {error}")));
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
        tracing::debug!("STDIO MCP listing tools for: {}", self.name);

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
            .map_err(|e| spawn_mcp_error(&self.command, e))?;

        // Write all requests to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;

            // Send initialize request
            let init_str = format!("{init_request}\n");
            stdin.write_all(init_str.as_bytes()).await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to write init to stdin: {e}"))
            })?;
            stdin
                .flush()
                .await
                .map_err(|e| McpError::ProtocolError(format!("Failed to flush init: {e}")))?;

            // Send initialized notification
            let notif_str = format!("{initialized_notification}\n");
            stdin.write_all(notif_str.as_bytes()).await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to write notification to stdin: {e}"))
            })?;
            stdin.flush().await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to flush notification: {e}"))
            })?;

            // Send tools/list request
            let tools_str = format!("{tools_request}\n");
            stdin.write_all(tools_str.as_bytes()).await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to write tools request to stdin: {e}"))
            })?;
            stdin.flush().await.map_err(|e| {
                McpError::ProtocolError(format!("Failed to flush tools request: {e}"))
            })?;
        }

        // Read response
        let output = child.wait_with_output().await.map_err(|e| {
            McpError::ProtocolError(format!("Failed to read from MCP process: {e}"))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        tracing::debug!("STDIO MCP process exited with: {:?}", output.status);
        tracing::debug!("STDIO MCP stdout: {}", stdout);
        if !stderr.is_empty() {
            tracing::debug!("STDIO MCP stderr: {}", stderr);
        }

        if !output.status.success() {
            return Err(McpError::ProtocolError(format!(
                "MCP process failed: {stderr}"
            )));
        }

        // Parse JSON responses - we need to find the tools/list response
        let mut tools_array = None;

        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(response) = serde_json::from_str::<Value>(line) {
                tracing::debug!("STDIO MCP parsed response line: {}", response);

                // Skip initialize response (id: 1)
                if response.get("id").and_then(serde_json::Value::as_i64) == Some(1) {
                    continue;
                }

                // Look for tools/list response (id: 2)
                if response.get("id").and_then(serde_json::Value::as_i64) == Some(2) {
                    // Check for JSON-RPC error first
                    if let Some(error) = response.get("error") {
                        return Err(McpError::ProtocolError(format!("MCP error: {error}")));
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

fn spawn_mcp_error(command: &str, error: std::io::Error) -> McpError {
    McpError::ConnectionFailed(format!(
        "Failed to spawn MCP process '{command}'. Stdio MCP commands run on the daemon host and must be installed in the daemon PATH: {error}"
    ))
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
                self.name,
            ))),
        }
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        match tokio::time::timeout(STDIO_MCP_TIMEOUT, self.spawn_and_list()).await {
            Ok(res) => res,
            Err(_) => Err(McpError::ProtocolError(format!(
                "STDIO MCP list_tools timed out after {}s ({})",
                STDIO_MCP_TIMEOUT.as_secs(),
                self.name,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_error_mentions_daemon_host_and_command() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let message = spawn_mcp_error("npx", error).to_string();

        assert!(message.contains("npx"));
        assert!(message.contains("daemon host"));
        assert!(message.contains("daemon PATH"));
    }
}
