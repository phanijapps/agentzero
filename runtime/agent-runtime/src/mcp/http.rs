// ============================================================================
// HTTP MCP CLIENT
// ============================================================================

//! # HTTP MCP Client
//!
//! HTTP transport implementation for MCP clients.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

use super::client::McpClient;
use super::error::McpError;
use super::tool::McpTool;

/// Overall request budget. Without this, a wedged upstream parks a tokio
/// worker indefinitely (the OS-level TCP timeout is ~2 min on Linux, but
/// the daemon "looks stuck" the whole time). 30 s covers any reasonable
/// MCP tool call.
const HTTP_MCP_TIMEOUT: Duration = Duration::from_secs(30);
/// TCP connect deadline — fail fast on dead hosts before issuing the call.
const HTTP_MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP-based MCP client
pub(super) struct HttpMcpClient {
    #[allow(dead_code)] // Reserved for future connection tracking
    id: String,
    name: String,
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
}

impl HttpMcpClient {
    pub(super) fn new(
        id: String,
        name: String,
        url: String,
        headers: HashMap<String, String>,
    ) -> Self {
        tracing::debug!("Creating HTTP MCP client: {} at {}", name, url);
        Self {
            id,
            name,
            url,
            headers,
            client: reqwest::Client::builder()
                .timeout(HTTP_MCP_TIMEOUT)
                .connect_timeout(HTTP_MCP_CONNECT_TIMEOUT)
                .build()
                .expect("reqwest client"),
        }
    }

    /// Send a JSON-RPC request to the HTTP MCP server
    async fn send_request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let auth_secrets = auth_redaction_values(&self.headers);
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
            "params": params
        });

        tracing::debug!("HTTP MCP request to {}: {}", self.url, request_body);

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
            .map_err(|e| McpError::ProtocolError(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let response_text = response
            .text()
            .await
            .map_err(|e| McpError::ProtocolError(format!("Failed to read response: {e}")))?;

        tracing::debug!(
            "HTTP MCP response status: {}, body: {}",
            status,
            redact_text(&response_text, &auth_secrets)
        );

        if !status.is_success() {
            if !auth_secrets.is_empty() {
                return Err(McpError::ProtocolError(format!(
                    "HTTP error {}: authenticated MCP request failed",
                    status.as_u16()
                )));
            }
            return Err(McpError::ProtocolError(format!(
                "HTTP error {}: {}",
                status.as_u16(),
                redact_text(&response_text, &auth_secrets)
            )));
        }

        let mut response_json = parse_mcp_response(&response_text, content_type.as_deref())
            .map_err(|e| {
                McpError::ProtocolError(format!(
                    "{}{}",
                    e,
                    if content_type.is_some() {
                        format!(" (content-type: {})", content_type.as_deref().unwrap())
                    } else {
                        String::new()
                    }
                ))
            })?;
        redact_json(&mut response_json, &auth_secrets);

        // Check for JSON-RPC error
        if let Some(error) = response_json.get("error") {
            return Err(McpError::ProtocolError(format!("MCP error: {error}")));
        }

        Ok(response_json)
    }
}

fn auth_redaction_values(headers: &HashMap<String, String>) -> Vec<String> {
    let mut values = Vec::new();
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("authorization") && !value.trim().is_empty() {
            values.push(value.clone());
            let mut parts = value.trim().splitn(2, char::is_whitespace);
            if parts
                .next()
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
            {
                if let Some(token) = parts
                    .next()
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                {
                    values.push(token.to_string());
                }
            }
        }
    }
    values
}

fn redact_text(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_string(), |redacted, secret| {
        if secret.is_empty() {
            redacted
        } else {
            redacted.replace(secret, "[REDACTED]")
        }
    })
}

fn redact_json(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            *text = redact_text(text, secrets);
        }
        Value::Array(values) => {
            for value in values {
                redact_json(value, secrets);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                redact_json(value, secrets);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn parse_mcp_response(response_text: &str, content_type: Option<&str>) -> Result<Value, String> {
    let is_sse = content_type
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        || response_text.trim_start().starts_with("event:")
        || response_text.trim_start().starts_with("data:");
    if is_sse {
        return parse_sse_mcp_response(response_text);
    }
    serde_json::from_str(response_text).map_err(|e| format!("Failed to parse JSON response: {e}"))
}

fn parse_sse_mcp_response(response_text: &str) -> Result<Value, String> {
    let mut event_name: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();
    let mut parse_errors = Vec::new();

    for line in response_text.lines().chain(std::iter::once("")) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if let Some(value) =
                parse_sse_event(event_name.as_deref(), &data_lines, &mut parse_errors)
            {
                return Ok(value);
            }
            event_name = None;
            data_lines.clear();
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }

    if parse_errors.is_empty() {
        Err("Failed to parse SSE response: no JSON-RPC message event found".to_string())
    } else {
        Err(format!(
            "Failed to parse SSE response: {}",
            parse_errors.join("; ")
        ))
    }
}

fn parse_sse_event(
    event_name: Option<&str>,
    data_lines: &[String],
    parse_errors: &mut Vec<String>,
) -> Option<Value> {
    if data_lines.is_empty() {
        return None;
    }
    if event_name.is_some_and(|event| event != "message") {
        return None;
    }

    let data = data_lines.join("\n");
    match serde_json::from_str::<Value>(&data) {
        Ok(value)
            if value.get("jsonrpc").is_some()
                || value.get("result").is_some()
                || value.get("error").is_some() =>
        {
            Some(value)
        }
        Ok(_) => None,
        Err(e) => {
            parse_errors.push(e.to_string());
            None
        }
    }
}

#[async_trait]
impl McpClient for HttpMcpClient {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_header_and_bearer_token_from_text() {
        let headers = HashMap::from([(
            "Authorization".to_string(),
            "Bearer access-secret".to_string(),
        )]);

        let redacted = redact_text(
            "upstream echoed Bearer access-secret and access-secret",
            &auth_redaction_values(&headers),
        );

        assert!(!redacted.contains("access-secret"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_authorization_token_from_json_strings() {
        let headers = HashMap::from([(
            "authorization".to_string(),
            "bearer access-secret".to_string(),
        )]);
        let mut value = serde_json::json!({
            "result": {
                "content": [
                    { "text": "token=access-secret" }
                ]
            }
        });

        redact_json(&mut value, &auth_redaction_values(&headers));

        assert!(!value.to_string().contains("access-secret"));
    }

    #[test]
    fn parses_plain_json_mcp_response() {
        let value = parse_mcp_response(
            r#"{"jsonrpc":"2.0","id":"1","result":{"tools":[]}}"#,
            Some("application/json"),
        )
        .unwrap();

        assert_eq!(value["result"]["tools"], serde_json::json!([]));
    }

    #[test]
    fn parses_streamable_http_sse_message_response() {
        let value = parse_mcp_response(
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"probe\",\"result\":{\"tools\":[{\"name\":\"get_accounts\"}]}}\n\n",
            Some("text/event-stream"),
        )
        .unwrap();

        assert_eq!(value["result"]["tools"][0]["name"], "get_accounts");
    }

    #[test]
    fn ignores_non_message_sse_events_until_json_rpc_message() {
        let value = parse_mcp_response(
            "event: ping\ndata: {}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"probe\",\"result\":{\"ok\":true}}\n\n",
            Some("text/event-stream; charset=utf-8"),
        )
        .unwrap();

        assert_eq!(value["result"]["ok"], true);
    }
}
