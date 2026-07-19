//! OAuth support for protected remote MCP servers.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::mcp::{McpOAuthPendingRecord, McpOAuthStatus, McpOAuthTokenRecord, McpService};

const OAUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const PENDING_TTL_SECS: i64 = 10 * 60;

/// Response returned when an OAuth flow is ready for the browser.
#[derive(Debug, Clone, Serialize)]
pub struct McpOAuthStartResponse {
    #[serde(rename = "authUrl")]
    pub auth_url: String,
    pub state: String,
}

/// OAuth service for MCP auth lifecycle.
pub struct McpOAuthService {
    mcp_service: Arc<McpService>,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct ProtectedResourceMetadata {
    authorization_servers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DynamicClientRegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl McpOAuthService {
    #[must_use]
    pub fn new(mcp_service: Arc<McpService>) -> Self {
        Self {
            mcp_service,
            client: reqwest::Client::builder()
                .timeout(OAUTH_HTTP_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client"),
        }
    }

    #[must_use]
    pub fn status(&self, mcp_id: &str) -> McpOAuthStatus {
        self.mcp_service.oauth_status(mcp_id)
    }

    pub fn disconnect(&self, mcp_id: &str) -> Result<(), String> {
        self.mcp_service.disconnect_oauth(mcp_id)
    }

    pub async fn begin_authorization(
        &self,
        mcp_id: &str,
        redirect_uri: &str,
    ) -> Result<McpOAuthStartResponse, String> {
        validate_redirect_uri(redirect_uri)?;
        let config = self.mcp_service.get(mcp_id)?;
        let auth = config
            .auth()
            .ok_or_else(|| "MCP server is not configured for OAuth".to_string())?;
        let resource = mcp_resource_url(&config)?;
        validate_oauth_url(&resource)?;

        let protected_metadata = self.discover_protected_resource(&resource).await?;
        let auth_server = protected_metadata
            .authorization_servers
            .first()
            .ok_or_else(|| {
                "OAuth protected resource did not advertise authorization_servers".to_string()
            })?;
        let auth_metadata = self.discover_authorization_server(auth_server).await?;
        validate_oauth_url(
            &Url::parse(&auth_metadata.authorization_endpoint)
                .map_err(|e| format!("Invalid authorization endpoint URL: {e}"))?,
        )?;
        validate_oauth_url(
            &Url::parse(&auth_metadata.token_endpoint)
                .map_err(|e| format!("Invalid token endpoint URL: {e}"))?,
        )?;
        if let Some(registration_endpoint) = &auth_metadata.registration_endpoint {
            validate_oauth_url(
                &Url::parse(registration_endpoint)
                    .map_err(|e| format!("Invalid registration endpoint URL: {e}"))?,
            )?;
        }

        let (client_id, client_secret) = match &auth.client_id {
            Some(client_id) => (client_id.clone(), None),
            None => self
                .register_dynamic_client(
                    auth_metadata
                        .registration_endpoint
                        .as_deref()
                        .ok_or_else(|| {
                            "OAuth server does not support dynamic client registration; z-Bot cannot complete zero-config OAuth for this MCP server".to_string()
                        })?,
                    redirect_uri,
                )
                .await?,
        };

        let code_verifier = generate_code_verifier();
        let code_challenge = pkce_challenge(&code_verifier);
        let state = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        self.mcp_service.save_oauth_pending(
            &state,
            McpOAuthPendingRecord {
                mcp_id: mcp_id.to_string(),
                code_verifier,
                redirect_uri: redirect_uri.to_string(),
                resource: resource.to_string(),
                expires_at_unix: now + PENDING_TTL_SECS,
                client_id: Some(client_id.clone()),
                client_secret,
                token_endpoint: auth_metadata.token_endpoint.clone(),
            },
        )?;

        let mut auth_url = Url::parse(&auth_metadata.authorization_endpoint)
            .map_err(|e| format!("Invalid authorization endpoint URL: {e}"))?;
        {
            let mut pairs = auth_url.query_pairs_mut();
            pairs.append_pair("response_type", "code");
            pairs.append_pair("client_id", &client_id);
            pairs.append_pair("redirect_uri", redirect_uri);
            pairs.append_pair("code_challenge", &code_challenge);
            pairs.append_pair("code_challenge_method", "S256");
            pairs.append_pair("state", &state);
            pairs.append_pair("resource", resource.as_str());
            let scopes = requested_scopes(&auth.scopes, &auth_metadata.scopes_supported);
            if !scopes.is_empty() {
                pairs.append_pair("scope", &scopes.join(" "));
            }
        }

        Ok(McpOAuthStartResponse {
            auth_url: auth_url.to_string(),
            state,
        })
    }

    pub async fn complete_callback(&self, state: &str, code: &str) -> Result<String, String> {
        let pending = self
            .mcp_service
            .remove_oauth_pending(state)?
            .ok_or_else(|| "OAuth callback state is unknown or already used".to_string())?;
        if pending.expires_at_unix <= chrono::Utc::now().timestamp() {
            return Err("OAuth callback state expired".to_string());
        }
        let current_config = self.mcp_service.get(&pending.mcp_id)?;
        if !current_config.is_oauth() {
            return Err("OAuth callback MCP server is no longer configured for OAuth".to_string());
        }
        let current_resource = mcp_resource_url(&current_config)?;
        if current_resource.as_str() != pending.resource {
            return Err("OAuth callback state no longer matches current MCP resource".to_string());
        }
        let client_id = pending
            .client_id
            .as_deref()
            .ok_or_else(|| "OAuth callback state is missing client_id".to_string())?;

        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", pending.redirect_uri.clone()),
            ("client_id", client_id.to_string()),
            ("code_verifier", pending.code_verifier.clone()),
            ("resource", pending.resource.clone()),
        ];
        if let Some(secret) = &pending.client_secret {
            form.push(("client_secret", secret.clone()));
        }

        let token_endpoint = Url::parse(&pending.token_endpoint)
            .map_err(|e| format!("Invalid token endpoint URL in pending state: {e}"))?;
        validate_oauth_url(&token_endpoint)?;
        let token_response = self
            .client
            .post(token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("OAuth token exchange failed: {e}"))?;
        let token_status = token_response.status();
        if !token_status.is_success() {
            let error_body = token_response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable response body>".to_string());
            return Err(format!(
                "OAuth token exchange failed with status {}: {}",
                token_status,
                sanitize_oauth_error_body(&error_body)
            ));
        }
        let token: TokenResponse = token_response
            .json()
            .await
            .map_err(|e| format!("Failed to parse OAuth token response: {e}"))?;
        let expires_at_unix = token
            .expires_in
            .map(|expires_in| chrono::Utc::now().timestamp() + expires_in);

        self.mcp_service.save_oauth_token(
            &pending.mcp_id,
            McpOAuthTokenRecord {
                access_token: token.access_token,
                refresh_token: token.refresh_token,
                expires_at_unix,
                client_id: pending.client_id,
                client_secret: pending.client_secret,
                resource: pending.resource,
                token_endpoint: pending.token_endpoint,
            },
        )?;

        Ok(pending.mcp_id)
    }

    async fn discover_protected_resource(
        &self,
        resource: &Url,
    ) -> Result<ProtectedResourceMetadata, String> {
        if let Some(metadata_url) = self.probe_resource_metadata_header(resource).await? {
            return self.fetch_protected_resource_metadata(&metadata_url).await;
        }

        for metadata_url in protected_resource_well_known_urls(resource)? {
            if let Ok(metadata) = self.fetch_protected_resource_metadata(&metadata_url).await {
                return Ok(metadata);
            }
        }
        Err("OAuth protected resource metadata discovery failed".to_string())
    }

    async fn probe_resource_metadata_header(&self, resource: &Url) -> Result<Option<Url>, String> {
        let response = self
            .client
            .post(resource.clone())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": "oauth-discovery",
                "method": "tools/list",
                "params": null
            }))
            .send()
            .await
            .map_err(|e| format!("OAuth resource probe failed: {e}"))?;
        reject_redirect_response(response.status(), "OAuth resource probe")?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(None);
        }
        let Some(header) = response.headers().get(reqwest::header::WWW_AUTHENTICATE) else {
            return Ok(None);
        };
        let header = header
            .to_str()
            .map_err(|e| format!("Invalid WWW-Authenticate header: {e}"))?;
        parse_resource_metadata_header(header)
    }

    async fn fetch_protected_resource_metadata(
        &self,
        url: &Url,
    ) -> Result<ProtectedResourceMetadata, String> {
        validate_oauth_url(url)?;
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| format!("Failed to fetch protected resource metadata: {e}"))?;
        reject_redirect_response(response.status(), "Protected resource metadata")?;
        response
            .error_for_status()
            .map_err(|e| format!("Protected resource metadata error: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse protected resource metadata: {e}"))
    }

    async fn discover_authorization_server(
        &self,
        auth_server: &str,
    ) -> Result<AuthorizationServerMetadata, String> {
        let auth_server = Url::parse(auth_server)
            .map_err(|e| format!("Invalid authorization server URL: {e}"))?;
        validate_oauth_url(&auth_server)?;
        for metadata_url in authorization_server_metadata_urls(&auth_server)? {
            if let Ok(metadata) = self
                .fetch_authorization_server_metadata(&metadata_url)
                .await
            {
                return Ok(metadata);
            }
        }
        Err("OAuth authorization server metadata discovery failed".to_string())
    }

    async fn fetch_authorization_server_metadata(
        &self,
        url: &Url,
    ) -> Result<AuthorizationServerMetadata, String> {
        validate_oauth_url(url)?;
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| format!("Failed to fetch authorization server metadata: {e}"))?;
        reject_redirect_response(response.status(), "Authorization server metadata")?;
        response
            .error_for_status()
            .map_err(|e| format!("Authorization server metadata error: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse authorization server metadata: {e}"))
    }

    async fn register_dynamic_client(
        &self,
        registration_endpoint: &str,
        redirect_uri: &str,
    ) -> Result<(String, Option<String>), String> {
        let url = Url::parse(registration_endpoint)
            .map_err(|e| format!("Invalid registration endpoint URL: {e}"))?;
        validate_oauth_url(&url)?;
        let response = self
            .client
            .post(url)
            .json(&json!({
                "client_name": "z-Bot",
                "redirect_uris": [redirect_uri],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none"
            }))
            .send()
            .await
            .map_err(|e| format!("Dynamic client registration failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Dynamic client registration failed with status {}",
                response.status()
            ));
        }
        let registered: DynamicClientRegistrationResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse dynamic client registration response: {e}"))?;
        Ok((registered.client_id, registered.client_secret))
    }
}

fn mcp_resource_url(config: &agent_runtime::McpServerConfig) -> Result<Url, String> {
    let url = match config {
        agent_runtime::McpServerConfig::Http { url, .. }
        | agent_runtime::McpServerConfig::Sse { url, .. }
        | agent_runtime::McpServerConfig::StreamableHttp { url, .. } => url,
        agent_runtime::McpServerConfig::Stdio { .. } => {
            return Err("stdio MCP servers do not support OAuth".to_string())
        }
    };
    Url::parse(url).map_err(|e| format!("Invalid MCP resource URL: {e}"))
}

fn generate_code_verifier() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().to_string().replace('-', ""),
        uuid::Uuid::new_v4().to_string().replace('-', "")
    )
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn requested_scopes(configured: &[String], supported: &[String]) -> Vec<String> {
    if !configured.is_empty() {
        return configured.to_vec();
    }
    match supported {
        [only] => vec![only.clone()],
        _ => Vec::new(),
    }
}

fn sanitize_oauth_error_body(body: &str) -> String {
    let mut value = body.trim().chars().take(512).collect::<String>();
    for key in ["access_token", "refresh_token", "client_secret", "code"] {
        value = redact_json_like_secret(&value, key);
    }
    if value.is_empty() {
        "<empty response body>".to_string()
    } else {
        value
    }
}

fn redact_json_like_secret(input: &str, key: &str) -> String {
    let quoted = format!("\"{key}\"");
    let Some(start) = input.find(&quoted) else {
        return input.to_string();
    };
    let Some(colon_offset) = input[start + quoted.len()..].find(':') else {
        return input.to_string();
    };
    let value_start = start + quoted.len() + colon_offset + 1;
    let rest = &input[value_start..];
    let Some(first_quote_offset) = rest.find('"') else {
        return input.to_string();
    };
    let secret_start = value_start + first_quote_offset + 1;
    let Some(secret_end_offset) = input[secret_start..].find('"') else {
        return input.to_string();
    };
    let secret_end = secret_start + secret_end_offset;
    format!(
        "{}<redacted>{}",
        &input[..secret_start],
        &input[secret_end..]
    )
}

fn validate_oauth_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" && !is_localhost_url(url) {
        return Err(format!(
            "OAuth URL must use HTTPS unless it targets localhost: {}",
            url
        ));
    }
    if is_localhost_url(url) {
        return Ok(());
    }
    if let Some(host) = url.host_str() {
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_local_ip(ip) {
                return Err(format!(
                    "OAuth URL cannot target private or link-local addresses: {}",
                    url
                ));
            }
        }
    }
    Ok(())
}

fn is_localhost_url(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    )
}

fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn validate_redirect_uri(redirect_uri: &str) -> Result<(), String> {
    let url = Url::parse(redirect_uri).map_err(|e| format!("Invalid OAuth redirect URI: {e}"))?;
    if (url.scheme() == "http" || url.scheme() == "https") && is_localhost_url(&url) {
        return Ok(());
    }
    Err("OAuth redirect URI must target localhost".to_string())
}

fn reject_redirect_response(status: reqwest::StatusCode, context: &str) -> Result<(), String> {
    if status.is_redirection() {
        return Err(format!(
            "{context} returned redirect status {}; OAuth discovery does not follow redirects",
            status
        ));
    }
    Ok(())
}

fn parse_resource_metadata_header(header: &str) -> Result<Option<Url>, String> {
    let Some(start) = header.find("resource_metadata=") else {
        return Ok(None);
    };
    let value = &header[start + "resource_metadata=".len()..];
    let value = value
        .trim_start_matches('"')
        .split(['"', ',', ' '])
        .next()
        .unwrap_or("")
        .trim();
    if value.is_empty() {
        return Ok(None);
    }
    let url = Url::parse(value).map_err(|e| format!("Invalid resource_metadata URL: {e}"))?;
    validate_oauth_url(&url)?;
    Ok(Some(url))
}

fn protected_resource_well_known_urls(resource: &Url) -> Result<Vec<Url>, String> {
    let origin = resource
        .origin()
        .ascii_serialization()
        .trim_end_matches('/')
        .to_string();
    let path = resource.path().trim_start_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(
            Url::parse(&format!(
                "{origin}/.well-known/oauth-protected-resource/{path}"
            ))
            .map_err(|e| format!("Invalid protected resource metadata URL: {e}"))?,
        );
    }
    urls.push(
        Url::parse(&format!("{origin}/.well-known/oauth-protected-resource"))
            .map_err(|e| format!("Invalid protected resource metadata URL: {e}"))?,
    );
    for url in &urls {
        validate_oauth_url(url)?;
    }
    Ok(urls)
}

fn authorization_server_metadata_urls(auth_server: &Url) -> Result<Vec<Url>, String> {
    let origin = auth_server
        .origin()
        .ascii_serialization()
        .trim_end_matches('/')
        .to_string();
    let path = auth_server.path().trim_start_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(
            Url::parse(&format!(
                "{origin}/.well-known/oauth-authorization-server/{path}"
            ))
            .map_err(|e| format!("Invalid authorization server metadata URL: {e}"))?,
        );
        urls.push(
            Url::parse(&format!("{origin}/.well-known/openid-configuration/{path}"))
                .map_err(|e| format!("Invalid OIDC metadata URL: {e}"))?,
        );
    }
    urls.push(
        Url::parse(&format!("{origin}/.well-known/oauth-authorization-server"))
            .map_err(|e| format!("Invalid authorization server metadata URL: {e}"))?,
    );
    urls.push(
        Url::parse(&format!("{origin}/.well-known/openid-configuration"))
            .map_err(|e| format!("Invalid OIDC metadata URL: {e}"))?,
    );
    for url in &urls {
        validate_oauth_url(url)?;
    }
    Ok(urls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_oauth_pkce_challenge_is_url_safe_sha256() {
        let challenge = pkce_challenge("test-verifier");
        assert!(!challenge.contains('='));
        assert!(challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn mcp_oauth_uses_configured_scopes_first() {
        let scopes = requested_scopes(&["configured".to_string()], &["internal".to_string()]);

        assert_eq!(scopes, vec!["configured"]);
    }

    #[test]
    fn mcp_oauth_defaults_to_single_advertised_scope() {
        let scopes = requested_scopes(&[], &["internal".to_string()]);

        assert_eq!(scopes, vec!["internal"]);
    }

    #[test]
    fn mcp_oauth_does_not_default_to_multiple_advertised_scopes() {
        let scopes = requested_scopes(&[], &["read".to_string(), "trade".to_string()]);

        assert!(scopes.is_empty());
    }

    #[test]
    fn mcp_oauth_sanitizes_token_exchange_error_body() {
        let sanitized = sanitize_oauth_error_body(
            r#"{"error":"invalid_grant","code":"abc","access_token":"secret","client_secret":"secret"}"#,
        );

        assert!(sanitized.contains("invalid_grant"));
        assert!(!sanitized.contains("abc"));
        assert!(!sanitized.contains(r#":"secret""#));
        assert!(sanitized.contains("<redacted>"));
    }

    #[test]
    fn mcp_oauth_rejects_non_https_non_localhost_urls() {
        let url = Url::parse("http://example.com/oauth").unwrap();
        assert!(validate_oauth_url(&url).is_err());

        let localhost = Url::parse("http://localhost:9999/oauth").unwrap();
        assert!(validate_oauth_url(&localhost).is_ok());

        let private_https = Url::parse("https://192.168.1.5/oauth").unwrap();
        assert!(validate_oauth_url(&private_https).is_err());
    }

    #[test]
    fn mcp_oauth_rejects_non_local_redirect_uris() {
        assert!(validate_redirect_uri("http://localhost:18791/api/mcps/oauth/callback").is_ok());
        assert!(validate_redirect_uri("https://127.0.0.1/callback").is_ok());
        assert!(validate_redirect_uri("https://example.com/callback").is_err());
        assert!(validate_redirect_uri("http://192.168.1.5/callback").is_err());
    }

    #[test]
    fn mcp_oauth_parses_resource_metadata_header() {
        let url = parse_resource_metadata_header(
            r#"Bearer resource_metadata="https://example.com/.well-known/oauth-protected-resource""#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn mcp_oauth_builds_protected_resource_well_known_urls() {
        let resource = Url::parse("https://agent.robinhood.com/mcp/trading").unwrap();
        let urls = protected_resource_well_known_urls(&resource).unwrap();
        assert_eq!(
            urls[0].as_str(),
            "https://agent.robinhood.com/.well-known/oauth-protected-resource/mcp/trading"
        );
        assert_eq!(
            urls[1].as_str(),
            "https://agent.robinhood.com/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn mcp_oauth_builds_path_preserving_authorization_server_metadata_urls() {
        let auth_server = Url::parse("https://auth.example.com/tenant/a").unwrap();
        let urls = authorization_server_metadata_urls(&auth_server).unwrap();
        assert_eq!(
            urls[0].as_str(),
            "https://auth.example.com/.well-known/oauth-authorization-server/tenant/a"
        );
        assert_eq!(
            urls[1].as_str(),
            "https://auth.example.com/.well-known/openid-configuration/tenant/a"
        );
        assert_eq!(
            urls[2].as_str(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
    }
}
