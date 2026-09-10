//! Persistent MCP sessions owned by AgentZero, using the pinned MCP SDK.

use super::{
    http::{auth_redaction_values, redact_json},
    native_http::{SessionHttpClient, StartupGuard},
    McpClient, McpError, McpTool,
};
use async_trait::async_trait;
use rmcp::{
    model::CallToolRequestParams,
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
        TokioChildProcess,
    },
    RoleClient, ServiceExt,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Mutex, time::Duration};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct NativeMcpClient {
    name: String,
    peer: rmcp::Peer<RoleClient>,
    session: Mutex<Option<RunningService<RoleClient, ()>>>,
    canceled: tokio::sync::watch::Sender<bool>,
    timeout: Duration,
    secrets: Vec<String>,
}

impl NativeMcpClient {
    pub(super) async fn stdio(
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<Self, McpError> {
        #[cfg(not(windows))]
        let mut cmd = {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            cmd
        };
        #[cfg(windows)]
        let mut cmd = {
            // Preserve configured .cmd/npm launchers on Windows.
            let mut cmd = tokio::process::Command::new("cmd.exe");
            cmd.arg("/c").arg(command).args(args);
            cmd
        };
        cmd.envs(&env).kill_on_drop(true);
        let (transport, _) = TokioChildProcess::builder(cmd)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| {
                McpError::ConnectionFailed("Failed to spawn MCP process on daemon host".into())
            })?;
        let session = tokio::time::timeout(STARTUP_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| failure("MCP initialization timed out"))?
            .map_err(|_| failure("MCP initialization failed"))?;
        // Environment values may contain credentials; never echo them back into
        // model inventory/results even when a server returns them verbatim.
        Ok(Self::new(
            name,
            session,
            Duration::from_secs(60),
            env.into_iter()
                .filter(|(key, value)| {
                    let key = key.to_ascii_uppercase();
                    !value.is_empty()
                        && [
                            "SECRET",
                            "TOKEN",
                            "PASSWORD",
                            "API_KEY",
                            "CREDENTIAL",
                            "AUTH",
                        ]
                        .iter()
                        .any(|marker| key.contains(marker))
                })
                .map(|(_, value)| value)
                .collect(),
        ))
    }

    pub(super) async fn streamable(
        name: String,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<Self, McpError> {
        let secrets = auth_redaction_values(&headers);
        // reqwest 0.13's no-provider mode requires explicit installation. Keep
        // the workspace's existing ring backend, respecting host installation.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let custom_headers =
            headers
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.parse()
                            .map_err(|_| failure("Invalid MCP header name"))?,
                        value
                            .parse()
                            .map_err(|_| failure("Invalid MCP header value"))?,
                    ))
                })
                .collect::<Result<
                    HashMap<reqwest::header::HeaderName, reqwest::header::HeaderValue>,
                    McpError,
                >>()?;
        let config = StreamableHttpClientTransportConfig::with_uri(url)
            .custom_headers(custom_headers)
            .reinit_on_expired_session(false);
        let canceled = tokio::sync::watch::channel(false).0;
        let mut startup_guard = StartupGuard(Some(canceled.clone()));
        let client = reqwest_mcp::Client::builder()
            .pool_max_idle_per_host(0)
            .connect_timeout(STARTUP_TIMEOUT)
            .build()
            .map_err(|_| failure("MCP HTTP client initialization failed"))?;
        let transport = StreamableHttpClientTransport::with_client(
            SessionHttpClient {
                client,
                canceled: canceled.clone(),
            },
            config,
        );
        let session = tokio::time::timeout(STARTUP_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| failure("MCP initialization timed out"))?
            .map_err(|_| failure("MCP initialization failed"))?;
        let mut owner = Self::new(name, session, Duration::from_secs(30), secrets);
        owner.canceled = canceled;
        startup_guard.0.take();
        Ok(owner)
    }

    fn new(
        name: String,
        session: RunningService<RoleClient, ()>,
        timeout: Duration,
        secrets: Vec<String>,
    ) -> Self {
        let peer = session.peer().clone();
        Self {
            name,
            peer,
            session: Mutex::new(Some(session)),
            canceled: tokio::sync::watch::channel(false).0,
            timeout,
            secrets,
        }
    }

    async fn request<T>(
        &self,
        request: impl std::future::Future<Output = Result<T, rmcp::ServiceError>>,
    ) -> Result<T, McpError> {
        let mut canceled = self.canceled.subscribe();
        tokio::select! {
        biased;
        _ = canceled.wait_for(|value| *value) => Err(failure("MCP session closed")),
        result = tokio::time::timeout(self.timeout, request) => result
            .map_err(|_| failure("MCP request timed out"))?
            .map_err(|_| failure("MCP request failed")) }
    }
}

fn failure(message: &str) -> McpError {
    McpError::ProtocolError(message.into())
}

#[async_trait]
impl McpClient for NativeMcpClient {
    fn name(&self) -> &str {
        &self.name
    }
    fn cancel(&self) {
        self.canceled.send_replace(true);
        if let Some(session) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            session.cancellation_token().cancel();
        }
    }
    async fn close(&self) {
        self.cancel();
        let session = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(mut session) = session {
            if !matches!(session.close_with_timeout(CLOSE_TIMEOUT).await, Ok(Some(_))) {
                tracing::warn!("MCP session cleanup did not complete within its budget");
            }
        }
    }
    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, McpError> {
        let params = CallToolRequestParams::new(tool_name.to_owned()).with_arguments(
            arguments
                .as_object()
                .cloned()
                .ok_or_else(|| failure("MCP arguments must be an object"))?,
        );
        let response = self.request(self.peer.call_tool(params)).await?;
        let mut value =
            serde_json::to_value(response).map_err(|_| failure("Invalid MCP tool response"))?;
        redact_json(&mut value, &self.secrets);
        Ok(value)
    }
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let tools = self.request(self.peer.list_all_tools()).await?;
        tools
            .into_iter()
            .map(|tool| {
                let mut value = serde_json::to_value(tool)
                    .map_err(|_| failure("Invalid MCP tool definition"))?;
                redact_json(&mut value, &self.secrets);
                Ok(McpTool {
                    name: value["name"].as_str().unwrap_or_default().into(),
                    description: value["description"].as_str().unwrap_or_default().into(),
                    parameters: value.get("inputSchema").cloned(),
                })
            })
            .collect()
    }
}

impl Drop for NativeMcpClient {
    fn drop(&mut self) {
        self.cancel();
    }
}
