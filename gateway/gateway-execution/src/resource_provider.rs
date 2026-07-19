//! # Gateway Resource Provider
//!
//! Implementation of `ConnectorResourceProvider` backed by the gateway's
//! `ConnectorRegistry`.

use agent_primitives::connectors::{
    CapabilityInfo, ConnectorInfo, ConnectorResourceProvider, ResourceInfo,
};
use async_trait::async_trait;
use gateway_connectors::ConnectorRegistry;
use std::collections::HashMap;
use std::sync::Arc;

/// Gateway implementation of `ConnectorResourceProvider`.
///
/// Wraps the `ConnectorRegistry` to provide resource querying capabilities
/// to agent tools.
pub struct GatewayResourceProvider {
    registry: Arc<ConnectorRegistry>,
}

impl GatewayResourceProvider {
    /// Create a new gateway resource provider.
    pub fn new(registry: Arc<ConnectorRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ConnectorResourceProvider for GatewayResourceProvider {
    async fn list_connectors(&self) -> Result<Vec<ConnectorInfo>, String> {
        let connectors = self
            .registry
            .list()
            .await
            .map_err(|e| format!("Failed to list connectors: {}", e))?;

        Ok(connectors
            .into_iter()
            .filter(|c| c.enabled)
            .map(|c| ConnectorInfo {
                id: c.id,
                name: c.name,
                resources: c
                    .metadata
                    .resources
                    .into_iter()
                    .map(|r| ResourceInfo {
                        name: r.name,
                        uri: r.uri,
                        method: r.method,
                        description: r.description,
                    })
                    .collect(),
                capabilities: c
                    .metadata
                    .capabilities
                    .into_iter()
                    .map(|cap| CapabilityInfo {
                        name: cap.name,
                        schema: cap.schema,
                        description: cap.description,
                    })
                    .collect(),
            })
            .collect())
    }

    async fn query_resource(
        &self,
        connector_id: &str,
        resource_name: &str,
        params: Option<HashMap<String, String>>,
    ) -> Result<serde_json::Value, String> {
        let connector = self
            .registry
            .get(connector_id)
            .await
            .map_err(|e| format!("Connector '{}' not found: {}", connector_id, e))?;

        // Find the resource by name
        let resource = connector
            .metadata
            .resources
            .iter()
            .find(|r| r.name == resource_name)
            .ok_or_else(|| {
                let available: Vec<&str> = connector
                    .metadata
                    .resources
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect();
                format!(
                    "Resource '{}' not found on connector '{}'. Available: {:?}",
                    resource_name, connector_id, available
                )
            })?;

        // Expand URI template with params
        let uri = expand_uri_template(&resource.uri, params.as_ref());

        // Build HTTP request with resource headers
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let mut request = match pick_http_method(&resource.method) {
            HttpMethod::Post => client.post(&uri),
            HttpMethod::Put => client.put(&uri),
            HttpMethod::Delete => client.delete(&uri),
            HttpMethod::Get => client.get(&uri),
        };

        // Apply resource-specific headers
        for (key, value) in &resource.headers {
            request = request.header(key, value);
        }

        // Also apply connector transport headers (for auth etc.)
        if let gateway_connectors::ConnectorTransport::Http { headers, .. } = &connector.transport {
            for (key, value) in headers {
                request = request.header(key, value);
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Resource request failed: {}", e))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        if !status.is_success() {
            return Err(format!(
                "Resource returned HTTP {}: {}",
                status.as_u16(),
                body.chars().take(500).collect::<String>()
            ));
        }

        // Try to parse as JSON, fall back to string value
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(json) => Ok(json),
            Err(_) => Ok(serde_json::Value::String(body)),
        }
    }

    async fn invoke_capability(
        &self,
        connector_id: &str,
        capability: &str,
        payload: serde_json::Value,
        session_id: &str,
        agent_id: &str,
    ) -> Result<serde_json::Value, String> {
        // Validate the capability exists on this connector
        let connector = self
            .registry
            .get(connector_id)
            .await
            .map_err(|e| format!("Connector '{}' not found: {}", connector_id, e))?;

        let cap_exists = connector
            .metadata
            .capabilities
            .iter()
            .any(|c| c.name == capability);

        if !cap_exists {
            let available: Vec<&str> = connector
                .metadata
                .capabilities
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            return Err(format!(
                "Capability '{}' not found on connector '{}'. Available: {:?}",
                capability, connector_id, available
            ));
        }

        let context = gateway_connectors::DispatchContext {
            session_id: session_id.to_string(),
            thread_id: None,
            agent_id: agent_id.to_string(),
            timestamp: chrono::Utc::now(),
        };

        let result = self
            .registry
            .dispatch_to_one(connector_id, capability, payload, &context)
            .await
            .map_err(|e| format!("Dispatch failed: {}", e))?;

        Ok(serde_json::json!({
            "success": result.success,
            "status": result.status,
            "body": result.body,
        }))
    }
}

/// Expand `{placeholder}` segments in a URI template against an optional
/// map of parameters. Extracted so tests can exercise the real code (not
/// a re-implementation) and so the expansion logic has a single home.
fn expand_uri_template(uri: &str, params: Option<&HashMap<String, String>>) -> String {
    let mut out = uri.to_string();
    if let Some(params) = params {
        for (key, value) in params {
            out = out.replace(&format!("{{{key}}}"), value);
        }
    }
    out
}

/// Canonical HTTP method the adapter understands. Any unknown verb
/// (including lowercased or typoed inputs) falls back to GET — same
/// behaviour as before, just spelled out in a named enum instead of a
/// `_ => GET` arm in the middle of the method.
#[derive(Debug, PartialEq, Eq)]
enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

fn pick_http_method(raw: &str) -> HttpMethod {
    match raw.to_uppercase().as_str() {
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
        _ => HttpMethod::Get,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- expand_uri_template ---

    #[test]
    fn expand_uri_template_substitutes_single_placeholder() {
        let params = HashMap::from([("id".into(), "user-123".into())]);
        let out = expand_uri_template("https://api.example.com/contacts/{id}", Some(&params));
        assert_eq!(out, "https://api.example.com/contacts/user-123");
    }

    #[test]
    fn expand_uri_template_substitutes_multiple_placeholders() {
        let params = HashMap::from([
            ("org".into(), "acme".into()),
            ("repo".into(), "project".into()),
        ]);
        let out = expand_uri_template("https://api.example.com/{org}/repos/{repo}", Some(&params));
        assert_eq!(out, "https://api.example.com/acme/repos/project");
    }

    #[test]
    fn expand_uri_template_no_params_map_returns_uri_unchanged() {
        let out = expand_uri_template("https://api.example.com/contacts", None);
        assert_eq!(out, "https://api.example.com/contacts");
    }

    #[test]
    fn expand_uri_template_leaves_unknown_placeholders_untouched() {
        // A placeholder that's not in `params` is kept as-is — the caller is
        // responsible for ensuring required params are supplied.
        let params = HashMap::from([("id".into(), "42".into())]);
        let out = expand_uri_template("/api/{id}/{missing}", Some(&params));
        assert_eq!(out, "/api/42/{missing}");
    }

    #[test]
    fn expand_uri_template_empty_params_map_returns_uri_unchanged() {
        let params = HashMap::new();
        let out = expand_uri_template("/api/{id}", Some(&params));
        assert_eq!(out, "/api/{id}");
    }

    // --- pick_http_method ---

    #[test]
    fn pick_http_method_matches_canonical_verbs() {
        assert_eq!(pick_http_method("GET"), HttpMethod::Get);
        assert_eq!(pick_http_method("POST"), HttpMethod::Post);
        assert_eq!(pick_http_method("PUT"), HttpMethod::Put);
        assert_eq!(pick_http_method("DELETE"), HttpMethod::Delete);
    }

    #[test]
    fn pick_http_method_is_case_insensitive() {
        assert_eq!(pick_http_method("post"), HttpMethod::Post);
        assert_eq!(pick_http_method("Put"), HttpMethod::Put);
        assert_eq!(pick_http_method("dElEtE"), HttpMethod::Delete);
    }

    #[test]
    fn pick_http_method_falls_back_to_get_for_unknown_verbs() {
        // Typos, unsupported verbs, and empty string all default to GET —
        // safest fallback for a user-configured connector.
        assert_eq!(pick_http_method("PATCH"), HttpMethod::Get);
        assert_eq!(pick_http_method("POSTT"), HttpMethod::Get);
        assert_eq!(pick_http_method(""), HttpMethod::Get);
    }
}
