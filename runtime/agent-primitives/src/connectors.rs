//! # Connector Resource Provider
//!
//! Trait for querying resources from external connectors.
//!
//! This allows agents to discover and query data exposed by connector
//! resource URIs (contacts, aliases, messages, etc.).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Summary of a connector visible to agent tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorInfo {
    /// Connector ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Available resources (read-only, GET).
    pub resources: Vec<ResourceInfo>,
    /// Available capabilities (actions, POST).
    pub capabilities: Vec<CapabilityInfo>,
}

/// Summary of a queryable resource on a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceInfo {
    /// Resource name (e.g., "contacts", "aliases").
    pub name: String,
    /// URI or URI template.
    pub uri: String,
    /// HTTP method.
    pub method: String,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Summary of an invocable capability on a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityInfo {
    /// Capability name (e.g., "send_message", "create_ticket").
    pub name: String,
    /// JSON schema for the capability payload.
    #[serde(default)]
    pub schema: serde_json::Value,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Trait for querying resources from connectors.
///
/// Implemented at the gateway layer where the connector registry is available.
/// Injected into agent tools via the executor builder.
#[async_trait]
pub trait ConnectorResourceProvider: Send + Sync {
    /// List all connectors with their available resources.
    async fn list_connectors(&self) -> Result<Vec<ConnectorInfo>, String>;

    /// Query a resource from a specific connector.
    ///
    /// Fetches data from the connector's resource URI, expanding any
    /// `{param}` template variables with the provided params map.
    async fn query_resource(
        &self,
        connector_id: &str,
        resource_name: &str,
        params: Option<HashMap<String, String>>,
    ) -> Result<serde_json::Value, String>;

    /// Invoke a capability on a connector (e.g., send_message).
    ///
    /// Dispatches the payload to the connector's transport endpoint
    /// with the specified capability name.
    async fn invoke_capability(
        &self,
        connector_id: &str,
        capability: &str,
        payload: serde_json::Value,
        session_id: &str,
        agent_id: &str,
    ) -> Result<serde_json::Value, String>;
}
