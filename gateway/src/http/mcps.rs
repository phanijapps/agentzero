//! # MCP Endpoints
//!
//! CRUD operations for MCP server configurations.

use crate::services::{mcp::McpServerSummary, McpOAuthService, McpOAuthStartResponse};
use crate::state::AppState;
use agent_runtime::{McpAuthConfig, McpServerConfig};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    Json,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DEFAULT_OAUTH_REDIRECT_URI: &str = "http://localhost:18791/api/mcps/oauth/callback";

/// MCP server list response.
#[derive(Debug, Serialize)]
pub struct McpListResponse {
    pub servers: Vec<McpServerSummary>,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// OAuth status response.
#[derive(Debug, Serialize)]
pub struct McpOAuthStatusResponse {
    pub status: String,
}

/// OAuth start request.
#[derive(Debug, Deserialize)]
pub struct McpOAuthStartRequest {
    #[serde(default, rename = "redirectUri")]
    pub redirect_uri: Option<String>,
}

/// OAuth callback query.
#[derive(Debug, Deserialize)]
pub struct McpOAuthCallbackQuery {
    pub state: Option<String>,
    pub code: Option<String>,
    pub error: Option<String>,
}

/// Create MCP server request.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CreateMcpRequest {
    /// Stdio-based MCP server
    #[serde(rename = "stdio")]
    Stdio {
        id: Option<String>,
        name: String,
        description: String,
        command: String,
        args: Vec<String>,
        env: Option<HashMap<String, String>>,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    /// HTTP-based MCP server
    #[serde(rename = "http")]
    Http {
        id: Option<String>,
        name: String,
        description: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        auth: Option<McpAuthConfig>,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    /// SSE-based MCP server
    #[serde(rename = "sse")]
    Sse {
        id: Option<String>,
        name: String,
        description: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        auth: Option<McpAuthConfig>,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    /// Streamable HTTP MCP server
    #[serde(rename = "streamable-http")]
    StreamableHttp {
        id: Option<String>,
        name: String,
        description: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        auth: Option<McpAuthConfig>,
        #[serde(default = "default_true")]
        enabled: bool,
    },
}

fn default_true() -> bool {
    true
}

impl From<CreateMcpRequest> for McpServerConfig {
    fn from(req: CreateMcpRequest) -> Self {
        match req {
            CreateMcpRequest::Stdio {
                id,
                name,
                description,
                command,
                args,
                env,
                enabled,
            } => McpServerConfig::Stdio {
                id,
                name,
                description,
                command,
                args,
                env,
                enabled,
                validated: None,
            },
            CreateMcpRequest::Http {
                id,
                name,
                description,
                url,
                headers,
                auth,
                enabled,
            } => McpServerConfig::Http {
                id,
                name,
                description,
                url,
                headers,
                auth,
                enabled,
                validated: None,
            },
            CreateMcpRequest::Sse {
                id,
                name,
                description,
                url,
                headers,
                auth,
                enabled,
            } => McpServerConfig::Sse {
                id,
                name,
                description,
                url,
                headers,
                auth,
                enabled,
                validated: None,
            },
            CreateMcpRequest::StreamableHttp {
                id,
                name,
                description,
                url,
                headers,
                auth,
                enabled,
            } => McpServerConfig::StreamableHttp {
                id,
                name,
                description,
                url,
                headers,
                auth,
                enabled,
                validated: None,
            },
        }
    }
}

fn validate_mcp_config(config: &McpServerConfig) -> Result<(), String> {
    if config.is_oauth() && config.has_authorization_header() {
        return Err(
            "OAuth MCP servers cannot persist Authorization headers; connect with OAuth instead"
                .to_string(),
        );
    }
    Ok(())
}

/// GET /api/mcps - List all MCP servers.
pub async fn list_mcps(State(state): State<AppState>) -> Json<McpListResponse> {
    match state.mcp_service.list_summaries() {
        Ok(servers) => Json(McpListResponse { servers }),
        Err(e) => {
            tracing::error!("Failed to list MCP servers: {}", e);
            Json(McpListResponse { servers: vec![] })
        }
    }
}

/// GET /api/mcps/:id - Get an MCP server by ID.
pub async fn get_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpServerConfig>, StatusCode> {
    match state.mcp_service.get(&id) {
        Ok(config) => Ok(Json(config)),
        Err(e) => {
            tracing::warn!("MCP server not found: {} - {}", id, e);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

/// POST /api/mcps - Create a new MCP server.
pub async fn create_mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateMcpRequest>,
) -> Result<Json<McpServerConfig>, (StatusCode, Json<ErrorResponse>)> {
    let config: McpServerConfig = request.into();

    if requires_local_origin_guard(&config) {
        require_local_origin(&headers)?;
    }

    if let Err(e) = validate_mcp_config(&config) {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })));
    }

    match state.mcp_service.add(config.clone()) {
        Ok(()) => Ok(Json(config)),
        Err(e) => {
            tracing::error!("Failed to create MCP server: {}", e);
            Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))
        }
    }
}

/// PUT /api/mcps/:id - Update an MCP server.
pub async fn update_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateMcpRequest>,
) -> Result<Json<McpServerConfig>, (StatusCode, Json<ErrorResponse>)> {
    // Verify the server exists
    if state.mcp_service.get(&id).is_err() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("MCP server not found: {}", id),
            }),
        ));
    }

    let config: McpServerConfig = request.into();

    if requires_local_origin_guard(&config) {
        require_local_origin(&headers)?;
    }

    if let Err(e) = validate_mcp_config(&config) {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })));
    }

    match state.mcp_service.update(&id, config.clone()) {
        Ok(()) => Ok(Json(config)),
        Err(e) => {
            tracing::error!("Failed to update MCP server: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e }),
            ))
        }
    }
}

/// DELETE /api/mcps/:id - Delete an MCP server.
pub async fn delete_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    match state.mcp_service.delete(&id) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            tracing::warn!("Failed to delete MCP server: {} - {}", id, e);
            Err((StatusCode::NOT_FOUND, Json(ErrorResponse { error: e })))
        }
    }
}

/// GET /api/mcps/:id/oauth/status - Get non-secret OAuth status.
pub async fn mcp_oauth_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpOAuthStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    if state.mcp_service.get(&id).is_err() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("MCP server not found: {}", id),
            }),
        ));
    }
    let oauth = McpOAuthService::new(state.mcp_service.clone());
    Ok(Json(McpOAuthStatusResponse {
        status: oauth.status(&id).as_str().to_string(),
    }))
}

/// POST /api/mcps/:id/oauth/start - Start OAuth authorization.
pub async fn start_mcp_oauth(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<McpOAuthStartRequest>,
) -> Result<Json<McpOAuthStartResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_local_origin(&headers)?;
    let redirect_uri = oauth_redirect_uri(&headers, request.redirect_uri.as_deref())?;
    let oauth = McpOAuthService::new(state.mcp_service.clone());
    oauth
        .begin_authorization(&id, &redirect_uri)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))
}

/// POST /api/mcps/:id/oauth/disconnect - Remove OAuth tokens/state.
pub async fn disconnect_mcp_oauth(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<McpOAuthStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_local_origin(&headers)?;
    let oauth = McpOAuthService::new(state.mcp_service.clone());
    oauth
        .disconnect(&id)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))?;
    Ok(Json(McpOAuthStatusResponse {
        status: oauth.status(&id).as_str().to_string(),
    }))
}

/// GET /api/mcps/oauth/callback - OAuth redirect target.
pub async fn mcp_oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<McpOAuthCallbackQuery>,
) -> (StatusCode, Html<String>) {
    if let Some(error) = query.error {
        return (
            StatusCode::BAD_REQUEST,
            Html(format!(
                "OAuth authorization failed: {}",
                html_escape(&error)
            )),
        );
    }
    let Some(state_value) = query.state else {
        return (
            StatusCode::BAD_REQUEST,
            Html("OAuth authorization failed: missing state".to_string()),
        );
    };
    let Some(code) = query.code else {
        return (
            StatusCode::BAD_REQUEST,
            Html("OAuth authorization failed: missing code".to_string()),
        );
    };

    let oauth = McpOAuthService::new(state.mcp_service.clone());
    match oauth.complete_callback(&state_value, &code).await {
        Ok(mcp_id) => (StatusCode::OK, Html(oauth_success_html(&mcp_id))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Html(format!("OAuth authorization failed: {}", html_escape(&e))),
        ),
    }
}

fn oauth_success_html(mcp_id: &str) -> String {
    let escaped_mcp_id = html_escape(mcp_id);
    let js_mcp_id = serde_json::to_string(mcp_id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OAuth Complete</title>
  <style>
    body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #111827; }}
    code {{ background: #f3f4f6; padding: 0.125rem 0.25rem; border-radius: 0.25rem; }}
  </style>
</head>
<body>
  <h1>OAuth authorization complete</h1>
  <p>MCP server <code>{escaped_mcp_id}</code> is connected. Returning to Integrations...</p>
  <script>
    (function () {{
      var mcpId = {js_mcp_id};
      var payload = {{ mcpId: mcpId, status: "connected", at: Date.now() }};
      try {{ localStorage.setItem("zbot:mcpOAuthComplete", JSON.stringify(payload)); }} catch (_) {{}}
      try {{ new BroadcastChannel("zbot:mcp-oauth").postMessage(payload); }} catch (_) {{}}
      setTimeout(function () {{
        window.location.replace("/integrations?tab=tools");
      }}, 800);
    }})();
  </script>
</body>
</html>"#
    )
}

fn oauth_redirect_uri(
    headers: &HeaderMap,
    requested: Option<&str>,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    if let Some(uri) = requested {
        validate_mcp_oauth_redirect_uri(uri)?;
        return Ok(uri.to_string());
    }

    if let Some(origin) = local_origin_from_headers(headers) {
        return Ok(format!(
            "{}/api/mcps/oauth/callback",
            origin.trim_end_matches('/')
        ));
    }

    Ok(DEFAULT_OAUTH_REDIRECT_URI.to_string())
}

fn validate_mcp_oauth_redirect_uri(uri: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let valid = Url::parse(uri).is_ok_and(|url| {
        is_local_origin(uri)
            && url.path() == "/api/mcps/oauth/callback"
            && url.query().is_none()
            && url.fragment().is_none()
    });
    if valid {
        return Ok(());
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "OAuth redirectUri must be a localhost /api/mcps/oauth/callback URL".to_string(),
        }),
    ))
}

fn local_origin_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("origin")
        .or_else(|| headers.get("referer"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| is_local_origin(url.as_str()))
        .map(|url| url.origin().ascii_serialization())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn requires_local_origin_guard(config: &McpServerConfig) -> bool {
    config.is_oauth() || config.has_authorization_header()
}

fn require_local_origin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let origin = headers
        .get("origin")
        .or_else(|| headers.get("referer"))
        .and_then(|value| value.to_str().ok());

    let Some(origin) = origin else {
        return Ok(());
    };

    if is_local_origin(origin) {
        return Ok(());
    }

    Err((
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "OAuth MCP mutations require a localhost Origin or Referer".to_string(),
        }),
    ))
}

fn is_local_origin(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    matches!(
        url.host_str(),
        Some(host)
            if host.eq_ignore_ascii_case("localhost")
                || host == "127.0.0.1"
                || host == "::1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn oauth_redirect_uri_accepts_local_dev_callback() {
        let uri = "http://localhost:3000/api/mcps/oauth/callback";

        assert!(validate_mcp_oauth_redirect_uri(uri).is_ok());
    }

    #[test]
    fn oauth_redirect_uri_rejects_external_callback() {
        let uri = "https://evil.example/api/mcps/oauth/callback";

        assert!(validate_mcp_oauth_redirect_uri(uri).is_err());
    }

    #[test]
    fn oauth_redirect_uri_rejects_wrong_local_path() {
        let uri = "http://localhost:3000/oauth/callback";

        assert!(validate_mcp_oauth_redirect_uri(uri).is_err());
    }

    #[test]
    fn oauth_redirect_uri_uses_request_origin_when_no_explicit_uri() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("http://localhost:3000"));

        let redirect_uri = oauth_redirect_uri(&headers, None).unwrap();

        assert_eq!(
            redirect_uri,
            "http://localhost:3000/api/mcps/oauth/callback"
        );
    }
}

/// Test result response.
#[derive(Debug, Serialize)]
pub struct McpTestResult {
    pub success: bool,
    pub message: String,
    pub tools: Option<Vec<String>>,
}

/// POST /api/mcps/:id/test - Test an MCP server connection.
pub async fn test_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpTestResult>, (StatusCode, Json<ErrorResponse>)> {
    // Get the MCP config
    let config = match state.mcp_service.get_for_runtime(&id) {
        Ok(c) => c,
        Err(e) => {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            return Err((status, Json(ErrorResponse { error: e })));
        }
    };

    // Try to start the server and list tools
    use agent_runtime::McpManager;

    let manager = McpManager::new();

    match manager.start_server(config).await {
        Ok(()) => {
            // Try to list tools
            match manager.list_all_tools().await {
                Ok(tools) => {
                    let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
                    let count = tool_names.len();
                    Ok(Json(McpTestResult {
                        success: true,
                        message: format!("Connected successfully. Found {} tools.", count),
                        tools: Some(tool_names),
                    }))
                }
                Err(e) => Ok(Json(McpTestResult {
                    success: false,
                    message: format!("Connected but failed to list tools: {}", e),
                    tools: None,
                })),
            }
        }
        Err(e) => Ok(Json(McpTestResult {
            success: false,
            message: format!("Failed to connect: {}", e),
            tools: None,
        })),
    }
}
