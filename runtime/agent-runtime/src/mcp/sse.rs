// ============================================================================
// SSE MCP CLIENT
// ============================================================================

//! # SSE MCP Client
//!
//! Server-Sent Events transport implementation for MCP clients.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

use super::client::McpClient;
use super::error::McpError;
use super::tool::McpTool;

/// Same budgets as the plain-HTTP MCP client — see `mcp::http`.
const SSE_MCP_TIMEOUT: Duration = Duration::from_secs(30);
const SSE_MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// SSE-based MCP client
pub(super) struct SseMcpClient {
    #[allow(dead_code)] // Reserved for future connection tracking
    id: String,
    name: String,
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
}

impl SseMcpClient {
    pub(super) fn new(
        id: String,
        name: String,
        url: String,
        headers: HashMap<String, String>,
    ) -> Self {
        tracing::debug!(mcp_id = %id, "Created SSE MCP client");
        Self {
            id,
            name,
            url,
            headers,
            client: reqwest::Client::builder()
                .timeout(SSE_MCP_TIMEOUT)
                .connect_timeout(SSE_MCP_CONNECT_TIMEOUT)
                .build()
                .expect("reqwest client"),
        }
    }

    /// Send a JSON-RPC request via POST
    async fn send_request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
            "params": params
        });

        tracing::debug!(mcp_id = %self.id, "Sending SSE MCP request");

        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        // Add custom headers (e.g., Authorization)
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }

        let response = req
            .json(&request_body)
            .send()
            .await
            .map_err(|_| McpError::ProtocolError("MCP request failed".to_string()))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|_| McpError::ProtocolError("Failed to read MCP response".to_string()))?;

        tracing::debug!(
            mcp_id = %self.id,
            status = status.as_u16(),
            "Received SSE MCP response"
        );

        if !status.is_success() {
            return Err(McpError::ProtocolError(format!(
                "MCP request failed with HTTP status {}",
                status.as_u16()
            )));
        }

        let response_json: Value = serde_json::from_str(&response_text)
            .map_err(|_| McpError::ProtocolError("Failed to parse MCP response".to_string()))?;

        // Check for JSON-RPC error
        if response_json.get("error").is_some() {
            return Err(McpError::ProtocolError(
                "MCP returned a protocol error".to_string(),
            ));
        }

        Ok(response_json)
    }
}

#[async_trait]
impl McpClient for SseMcpClient {
    fn name(&self) -> &str {
        &self.name
    }

    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, McpError> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", params).await?;

        // Extract the result from the response
        response
            .get("result")
            .or_else(|| response.get("content"))
            .cloned()
            .ok_or_else(|| McpError::ProtocolError("No result in MCP response".to_string()))
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let response = self.send_request("tools/list", Value::Null).await?;

        let tools_array = response
            .get("result")
            .and_then(|v| v.get("tools"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| McpError::ProtocolError("No tools array in MCP response".to_string()))?;

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
