//! # Composite Resource Provider
//!
//! Merges HTTP connectors and bridge workers into a single
//! `ConnectorResourceProvider` implementation.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use agent_primitives::connectors::{ConnectorInfo, ConnectorResourceProvider};

/// Composite provider that routes queries to either HTTP connectors or bridge workers.
///
/// On each request, the provider checks if the target connector_id is a bridge
/// worker (by listing connected bridge workers). If so, routes to the bridge
/// provider. Otherwise, falls through to the HTTP provider.
pub struct CompositeResourceProvider {
    http_provider: Option<Arc<dyn ConnectorResourceProvider>>,
    bridge_provider: Option<Arc<dyn ConnectorResourceProvider>>,
}

impl CompositeResourceProvider {
    /// Create a new composite provider.
    pub fn new(
        http_provider: Option<Arc<dyn ConnectorResourceProvider>>,
        bridge_provider: Option<Arc<dyn ConnectorResourceProvider>>,
    ) -> Self {
        Self {
            http_provider,
            bridge_provider,
        }
    }
}

#[async_trait]
impl ConnectorResourceProvider for CompositeResourceProvider {
    async fn list_connectors(&self) -> Result<Vec<ConnectorInfo>, String> {
        let mut all = Vec::new();
        if let Some(http) = &self.http_provider {
            all.extend(http.list_connectors().await.unwrap_or_default());
        }
        if let Some(bridge) = &self.bridge_provider {
            all.extend(bridge.list_connectors().await.unwrap_or_default());
        }
        Ok(all)
    }

    async fn query_resource(
        &self,
        connector_id: &str,
        resource_name: &str,
        params: Option<HashMap<String, String>>,
    ) -> Result<serde_json::Value, String> {
        // Check bridge workers first (only lists connected ones)
        if let Some(bridge) = &self.bridge_provider {
            let bridge_connectors = bridge.list_connectors().await.unwrap_or_default();
            if bridge_connectors.iter().any(|c| c.id == connector_id) {
                return bridge
                    .query_resource(connector_id, resource_name, params)
                    .await;
            }
        }

        // Fall through to HTTP connectors
        if let Some(http) = &self.http_provider {
            return http
                .query_resource(connector_id, resource_name, params)
                .await;
        }

        Err(format!("Connector '{}' not found", connector_id))
    }

    async fn invoke_capability(
        &self,
        connector_id: &str,
        capability: &str,
        payload: serde_json::Value,
        session_id: &str,
        agent_id: &str,
    ) -> Result<serde_json::Value, String> {
        // Check bridge workers first
        if let Some(bridge) = &self.bridge_provider {
            let bridge_connectors = bridge.list_connectors().await.unwrap_or_default();
            if bridge_connectors.iter().any(|c| c.id == connector_id) {
                return bridge
                    .invoke_capability(connector_id, capability, payload, session_id, agent_id)
                    .await;
            }
        }

        // Fall through to HTTP connectors
        if let Some(http) = &self.http_provider {
            return http
                .invoke_capability(connector_id, capability, payload, session_id, agent_id)
                .await;
        }

        Err(format!("Connector '{}' not found", connector_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        connectors: Vec<ConnectorInfo>,
    }

    #[async_trait]
    impl ConnectorResourceProvider for MockProvider {
        async fn list_connectors(&self) -> Result<Vec<ConnectorInfo>, String> {
            Ok(self.connectors.clone())
        }

        async fn query_resource(
            &self,
            connector_id: &str,
            _resource_name: &str,
            _params: Option<HashMap<String, String>>,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"source": "mock", "connector": connector_id}))
        }

        async fn invoke_capability(
            &self,
            connector_id: &str,
            capability: &str,
            _payload: serde_json::Value,
            _session_id: &str,
            _agent_id: &str,
        ) -> Result<serde_json::Value, String> {
            Ok(
                serde_json::json!({"source": "mock", "connector": connector_id, "capability": capability}),
            )
        }
    }

    #[tokio::test]
    async fn test_list_merges_both_sources() {
        let http = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "signal".to_string(),
                name: "Signal".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let bridge = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "crm".to_string(),
                name: "CRM Worker".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let composite =
            CompositeResourceProvider::new(Some(Arc::new(http)), Some(Arc::new(bridge)));

        let connectors = composite.list_connectors().await.unwrap();
        assert_eq!(connectors.len(), 2);
        assert!(connectors.iter().any(|c| c.id == "signal"));
        assert!(connectors.iter().any(|c| c.id == "crm"));
    }

    #[tokio::test]
    async fn test_query_routes_to_bridge_when_connected() {
        let http = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "signal".to_string(),
                name: "Signal".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let bridge = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "crm".to_string(),
                name: "CRM".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let composite =
            CompositeResourceProvider::new(Some(Arc::new(http)), Some(Arc::new(bridge)));

        // Query a bridge connector
        let result = composite
            .query_resource("crm", "contacts", None)
            .await
            .unwrap();
        assert_eq!(result["source"], "mock");
        assert_eq!(result["connector"], "crm");
    }

    #[tokio::test]
    async fn test_query_falls_through_to_http() {
        let http = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "signal".to_string(),
                name: "Signal".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let bridge = MockProvider { connectors: vec![] };
        let composite =
            CompositeResourceProvider::new(Some(Arc::new(http)), Some(Arc::new(bridge)));

        // Query an HTTP connector
        let result = composite
            .query_resource("signal", "aliases", None)
            .await
            .unwrap();
        assert_eq!(result["connector"], "signal");
    }

    #[tokio::test]
    async fn test_invoke_routes_correctly() {
        let http = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "signal".to_string(),
                name: "Signal".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let bridge = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "crm".to_string(),
                name: "CRM".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let composite =
            CompositeResourceProvider::new(Some(Arc::new(http)), Some(Arc::new(bridge)));

        let result = composite
            .invoke_capability("crm", "send_email", serde_json::json!({}), "sess-1", "root")
            .await
            .unwrap();
        assert_eq!(result["connector"], "crm");
        assert_eq!(result["capability"], "send_email");
    }

    #[tokio::test]
    async fn test_not_found_when_no_providers() {
        let composite = CompositeResourceProvider::new(None, None);
        let result = composite.query_resource("unknown", "data", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_only() {
        let http = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "signal".to_string(),
                name: "Signal".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let composite = CompositeResourceProvider::new(Some(Arc::new(http)), None);

        let connectors = composite.list_connectors().await.unwrap();
        assert_eq!(connectors.len(), 1);

        let result = composite
            .query_resource("signal", "aliases", None)
            .await
            .unwrap();
        assert_eq!(result["connector"], "signal");
    }

    #[tokio::test]
    async fn test_bridge_only() {
        let bridge = MockProvider {
            connectors: vec![ConnectorInfo {
                id: "crm".to_string(),
                name: "CRM".to_string(),
                resources: vec![],
                capabilities: vec![],
            }],
        };
        let composite = CompositeResourceProvider::new(None, Some(Arc::new(bridge)));

        let connectors = composite.list_connectors().await.unwrap();
        assert_eq!(connectors.len(), 1);
    }
}
