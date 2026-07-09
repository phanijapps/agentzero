// ============================================================================
// WEB TOOLS
// HTTP request tool with security guardrails
// ============================================================================

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use url::Url;

use agent_primitives::{AgentError, Result, Tool, ToolContext, ToolPermissions};

// ============================================================================
// SECURITY CONFIGURATION
// ============================================================================

/// Blocked hosts - internal/private IPs and metadata endpoints
const BLOCKED_HOSTS: &[&str] = &[
    // Localhost
    "localhost",
    "127.0.0.1",
    "0.0.0.0",
    "::1",
    // Private networks
    "10.",
    "172.16.",
    "172.17.",
    "172.18.",
    "172.19.",
    "172.20.",
    "172.21.",
    "172.22.",
    "172.23.",
    "172.24.",
    "172.25.",
    "172.26.",
    "172.27.",
    "172.28.",
    "172.29.",
    "172.30.",
    "172.31.",
    "192.168.",
    // Link-local
    "169.254.",
    // Cloud metadata endpoints
    "metadata.google",
    "metadata.google.internal",
    "169.254.169.254",
    "100.100.100.200", // Alibaba Cloud
    "fd00:ec2::254",   // AWS IPv6
];

/// Maximum response size (10 MB)
const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024;

/// Default request timeout (30 seconds)
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum request timeout (120 seconds)
const MAX_TIMEOUT_SECS: u64 = 120;

// ============================================================================
// WEB FETCH TOOL
// ============================================================================

/// Tool for making HTTP requests
pub struct WebFetchTool {
    client: Client,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    /// Create a new WebFetchTool with default client
    #[must_use]
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("Failed to create HTTP client");
        Self { client }
    }

    /// Check if a URL is blocked
    fn is_blocked_url(url: &Url) -> bool {
        if let Some(host) = url.host_str() {
            let host_lower = host.to_lowercase();
            for blocked in BLOCKED_HOSTS {
                if host_lower == *blocked || host_lower.starts_with(blocked) {
                    return true;
                }
            }
        }
        false
    }

    /// Validate the URL
    fn validate_url(url_str: &str) -> Result<Url> {
        let url =
            Url::parse(url_str).map_err(|e| AgentError::Tool(format!("Invalid URL: {}", e)))?;

        // Only allow http and https
        match url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(AgentError::Tool(format!(
                    "Unsupported URL scheme: {}. Only http and https are allowed.",
                    scheme
                )));
            }
        }

        // Check blocked hosts
        if Self::is_blocked_url(&url) {
            return Err(AgentError::Tool(
                "Access to internal/private networks is not allowed".to_string(),
            ));
        }

        Ok(url)
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Make HTTP requests to fetch web content. Supports GET, POST, PUT, DELETE methods. \
        Use for APIs, downloading content, or web scraping. \
        Cannot access internal/private networks (localhost, 10.x, 192.168.x, etc.)."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (http or https)"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"],
                    "default": "GET",
                    "description": "HTTP method"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Optional HTTP headers as key-value pairs"
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body for POST/PUT/PATCH"
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 120,
                    "default": 30,
                    "description": "Request timeout in seconds"
                }
            },
            "required": ["url"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::moderate(vec!["network:http".into()])
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Extract URL
        let url_str = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'url' parameter".to_string()))?;

        // Validate URL
        let url = Self::validate_url(url_str)?;

        tracing::debug!("WebFetchTool: Fetching {}", url);

        // Extract method
        let method_str = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");

        let method = match method_str.to_uppercase().as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            "PATCH" => Method::PATCH,
            "HEAD" => Method::HEAD,
            _ => {
                return Err(AgentError::Tool(format!(
                    "Unsupported method: {}",
                    method_str
                )));
            }
        };

        // Extract timeout
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        // Build request
        let mut request = self
            .client
            .request(method.clone(), url.clone())
            .timeout(Duration::from_secs(timeout_secs));

        // Add headers
        if let Some(headers) = args.get("headers").and_then(|v| v.as_object()) {
            for (key, value) in headers {
                if let Some(value_str) = value.as_str() {
                    request = request.header(key, value_str);
                }
            }
        }

        // Add body for POST/PUT/PATCH
        if matches!(method, Method::POST | Method::PUT | Method::PATCH)
            && let Some(body) = args.get("body").and_then(|v| v.as_str())
        {
            request = request.body(body.to_string());
        }

        // Send request
        let response = request
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("Request failed: {}", e)))?;

        // Get response info
        let status = response.status().as_u16();
        let status_text = response.status().canonical_reason().unwrap_or("Unknown");

        // Collect response headers
        let mut response_headers: HashMap<String, String> = HashMap::new();
        for (key, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                response_headers.insert(key.to_string(), value_str.to_string());
            }
        }

        // Check content length before downloading
        if let Some(content_length) = response.content_length()
            && content_length as usize > MAX_RESPONSE_SIZE
        {
            return Err(AgentError::Tool(format!(
                "Response too large: {} bytes (max: {} bytes)",
                content_length, MAX_RESPONSE_SIZE
            )));
        }

        // Get response body with size limit
        let bytes = response
            .bytes()
            .await
            .map_err(|e| AgentError::Tool(format!("Failed to read response: {}", e)))?;

        if bytes.len() > MAX_RESPONSE_SIZE {
            return Err(AgentError::Tool(format!(
                "Response too large: {} bytes (max: {} bytes)",
                bytes.len(),
                MAX_RESPONSE_SIZE
            )));
        }

        // Try to parse as text
        let body = String::from_utf8_lossy(&bytes).to_string();

        // Determine content type
        let content_type = response_headers
            .get("content-type")
            .cloned()
            .unwrap_or_else(|| "text/plain".to_string());

        // Try to parse JSON if content type indicates it
        let body_value: Value = if content_type.contains("application/json") {
            serde_json::from_str(&body).unwrap_or_else(|_| json!(body))
        } else {
            json!(body)
        };

        Ok(json!({
            "status": status,
            "status_text": status_text,
            "headers": response_headers,
            "body": body_value,
            "url": url.to_string(),
            "content_length": bytes.len(),
        }))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocked_urls() {
        // Blocked URLs
        assert!(WebFetchTool::is_blocked_url(
            &Url::parse("http://localhost/api").unwrap()
        ));
        assert!(WebFetchTool::is_blocked_url(
            &Url::parse("http://127.0.0.1/api").unwrap()
        ));
        assert!(WebFetchTool::is_blocked_url(
            &Url::parse("http://192.168.1.1/api").unwrap()
        ));
        assert!(WebFetchTool::is_blocked_url(
            &Url::parse("http://10.0.0.1/api").unwrap()
        ));
        assert!(WebFetchTool::is_blocked_url(
            &Url::parse("http://169.254.169.254/latest/").unwrap()
        ));
        assert!(WebFetchTool::is_blocked_url(
            &Url::parse("http://metadata.google.internal/api").unwrap()
        ));

        // Allowed URLs
        assert!(!WebFetchTool::is_blocked_url(
            &Url::parse("https://api.example.com/v1").unwrap()
        ));
        assert!(!WebFetchTool::is_blocked_url(
            &Url::parse("https://github.com/").unwrap()
        ));
    }

    #[test]
    fn test_validate_url() {
        // Valid URLs
        assert!(WebFetchTool::validate_url("https://api.example.com/v1").is_ok());
        assert!(WebFetchTool::validate_url("http://example.com/").is_ok());

        // Invalid URLs
        assert!(WebFetchTool::validate_url("not-a-url").is_err());
        assert!(WebFetchTool::validate_url("ftp://example.com/").is_err());
        assert!(WebFetchTool::validate_url("file:///etc/passwd").is_err());
        assert!(WebFetchTool::validate_url("http://localhost/").is_err());
        assert!(WebFetchTool::validate_url("http://192.168.1.1/").is_err());
    }
}
