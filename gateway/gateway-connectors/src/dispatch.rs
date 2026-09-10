//! # Connector Dispatch
//!
//! Transport-specific dispatch logic for sending payloads to connectors.

use crate::config::{ConnectorConfig, ConnectorPayload, ConnectorTransport, DispatchContext};
use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Errors that can occur during connector dispatch.
#[derive(Error, Debug)]
pub enum DispatchError {
    #[error("Connector not found: {0}")]
    NotFound(String),

    #[error("Connector disabled: {0}")]
    Disabled(String),

    #[error("Outbound disabled for connector: {0}")]
    OutboundDisabled(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("CLI execution error: {0}")]
    Cli(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Transport not supported: {0}")]
    UnsupportedTransport(String),

    #[error("Timeout")]
    Timeout,

    #[error("Connection error: {0}")]
    Connection(String),
}

/// Result type for dispatch operations.
pub type DispatchResult<T> = Result<T, DispatchError>;

/// Response from a connector dispatch.
#[derive(Debug, Clone)]
pub struct DispatchResponse {
    /// HTTP status code (for HTTP transport).
    pub status: Option<u16>,
    /// Response body.
    pub body: Option<String>,
    /// Whether the dispatch was successful.
    pub success: bool,
}

/// Dispatch a payload to a connector.
pub async fn dispatch(
    connector: &ConnectorConfig,
    capability: &str,
    payload: serde_json::Value,
    context: &DispatchContext,
) -> DispatchResult<DispatchResponse> {
    if !connector.enabled {
        return Err(DispatchError::Disabled(connector.id.clone()));
    }

    if !connector.outbound_enabled {
        return Err(DispatchError::OutboundDisabled(connector.id.clone()));
    }

    let connector_payload = ConnectorPayload {
        context: context.clone(),
        capability: capability.to_string(),
        payload,
    };

    match &connector.transport {
        ConnectorTransport::Http {
            callback_url,
            method,
            headers,
            timeout_ms,
        } => {
            dispatch_http(
                &connector.id,
                callback_url,
                method,
                headers,
                timeout_ms.unwrap_or(30000),
                &connector_payload,
            )
            .await
        }
        ConnectorTransport::Cli { command, args, env } => {
            dispatch_cli(&connector.id, command, args, env, &connector_payload).await
        }
        ConnectorTransport::Grpc { .. } => {
            warn!(
                connector_id = %connector.id,
                "gRPC transport not yet implemented"
            );
            Err(DispatchError::UnsupportedTransport("grpc".to_string()))
        }
        ConnectorTransport::WebSocket { .. } => {
            warn!(
                connector_id = %connector.id,
                "WebSocket transport not yet implemented"
            );
            Err(DispatchError::UnsupportedTransport("websocket".to_string()))
        }
        ConnectorTransport::Ipc { .. } => {
            warn!(
                connector_id = %connector.id,
                "IPC transport not yet implemented"
            );
            Err(DispatchError::UnsupportedTransport("ipc".to_string()))
        }
    }
}

/// Dispatch via HTTP transport.
async fn dispatch_http(
    connector_id: &str,
    callback_url: &str,
    method: &str,
    headers: &std::collections::HashMap<String, String>,
    timeout_ms: u64,
    payload: &ConnectorPayload,
) -> DispatchResult<DispatchResponse> {
    debug!(
        connector_id = %connector_id,
        url = %callback_url,
        method = %method,
        capability = %payload.capability,
        "Dispatching HTTP request to connector"
    );

    let client = reqwest::Client::builder().build().expect("reqwest client");

    let mut request = match method.to_uppercase().as_str() {
        "POST" => client.post(callback_url),
        "PUT" => client.put(callback_url),
        _ => {
            return Err(DispatchError::Http(format!(
                "Unsupported HTTP method: {}",
                method
            )))
        }
    };

    // Add custom headers
    for (key, value) in headers {
        request = request.header(key, value);
    }

    // Set timeout and content type
    request = request
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .header("Content-Type", "application/json")
        .json(payload);

    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let success = response.status().is_success();
            let body = if success {
                response.text().await.ok()
            } else {
                Some(crate::http_failure_summary(status))
            };

            if success {
                info!(
                    connector_id = %connector_id,
                    status = %status,
                    "Connector dispatch successful"
                );
            } else {
                warn!(
                    connector_id = %connector_id,
                    status = %status,
                    "Connector dispatch received non-success status"
                );
            }

            Ok(DispatchResponse {
                status: Some(status),
                body,
                success,
            })
        }
        Err(e) => {
            let e = e.without_url();
            error!(
                connector_id = %connector_id,
                error = %e,
                "Connector HTTP dispatch failed"
            );

            if e.is_timeout() {
                Err(DispatchError::Timeout)
            } else if e.is_connect() {
                Err(DispatchError::Connection(e.to_string()))
            } else {
                Err(DispatchError::Http(e.to_string()))
            }
        }
    }
}

/// Dispatch via CLI transport (execute command with payload as stdin).
async fn dispatch_cli(
    connector_id: &str,
    command: &str,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
    payload: &ConnectorPayload,
) -> DispatchResult<DispatchResponse> {
    debug!(
        connector_id = %connector_id,
        command = %command,
        args = ?args,
        capability = %payload.capability,
        "Dispatching CLI command to connector"
    );

    let payload_json = serde_json::to_string(payload)?;

    let mut cmd = Command::new(command);
    cmd.args(args)
        .envs(env.iter())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        error!(
            connector_id = %connector_id,
            command = %command,
            error = %e,
            "Failed to spawn CLI command"
        );
        DispatchError::Cli(format!("Failed to spawn command: {}", e))
    })?;

    // Write payload to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(payload_json.as_bytes()).await {
            warn!(
                connector_id = %connector_id,
                error = %e,
                "Failed to write to CLI stdin"
            );
        }
    }

    // Wait for command to complete
    let output = child.wait_with_output().await.map_err(|e| {
        error!(
            connector_id = %connector_id,
            error = %e,
            "Failed to wait for CLI command"
        );
        DispatchError::Cli(format!("Command execution failed: {}", e))
    })?;

    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if success {
        info!(
            connector_id = %connector_id,
            exit_code = ?output.status.code(),
            "CLI connector dispatch successful"
        );
    } else {
        warn!(
            connector_id = %connector_id,
            exit_code = ?output.status.code(),
            stderr = %stderr,
            "CLI connector dispatch failed"
        );
    }

    Ok(DispatchResponse {
        status: output.status.code().map(|c| c as u16),
        body: if stdout.is_empty() {
            if stderr.is_empty() {
                None
            } else {
                Some(stderr)
            }
        } else {
            Some(stdout)
        },
        success,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_context() -> DispatchContext {
        DispatchContext {
            session_id: "test-session".to_string(),
            thread_id: Some("test-thread".to_string()),
            agent_id: "root".to_string(),
            timestamp: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_dispatch_disabled_connector() {
        let connector = ConnectorConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            transport: ConnectorTransport::Http {
                callback_url: "http://localhost:9999".to_string(),
                method: "POST".to_string(),
                headers: HashMap::new(),
                timeout_ms: None,
            },
            metadata: Default::default(),
            enabled: false,
            outbound_enabled: true,
            inbound_enabled: true,
            created_at: None,
            updated_at: None,
        };

        let result = dispatch(&connector, "test", serde_json::json!({}), &test_context()).await;

        assert!(matches!(result, Err(DispatchError::Disabled(_))));
    }

    #[tokio::test]
    async fn test_dispatch_outbound_disabled() {
        let connector = ConnectorConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            transport: ConnectorTransport::Http {
                callback_url: "http://localhost:9999".to_string(),
                method: "POST".to_string(),
                headers: HashMap::new(),
                timeout_ms: None,
            },
            metadata: Default::default(),
            enabled: true,
            outbound_enabled: false,
            inbound_enabled: true,
            created_at: None,
            updated_at: None,
        };

        let result = dispatch(&connector, "test", serde_json::json!({}), &test_context()).await;

        assert!(matches!(result, Err(DispatchError::OutboundDisabled(_))));
    }
}
