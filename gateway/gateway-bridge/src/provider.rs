//! # Bridge Resource Provider
//!
//! Implementation of `ConnectorResourceProvider` for bridge workers.
//!
//! Routes resource queries and capability invocations over the WebSocket
//! connection to the worker, using request/response correlation.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::outbox::OutboxRepository;
use crate::protocol::BridgeServerMessage;
use crate::registry::BridgeRegistry;
use agent_primitives::connectors::{
    CapabilityInfo, ConnectorInfo, ConnectorResourceProvider, ResourceInfo,
};

/// Timeout for worker resource/capability requests.
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// Bridge implementation of `ConnectorResourceProvider`.
///
/// Routes queries to connected bridge workers via WebSocket.
pub struct BridgeResourceProvider {
    registry: Arc<BridgeRegistry>,
    outbox_repo: Arc<OutboxRepository>,
}

impl BridgeResourceProvider {
    /// Create a new bridge resource provider.
    pub fn new(registry: Arc<BridgeRegistry>, outbox_repo: Arc<OutboxRepository>) -> Self {
        Self {
            registry,
            outbox_repo,
        }
    }
}

#[async_trait]
impl ConnectorResourceProvider for BridgeResourceProvider {
    async fn list_connectors(&self) -> Result<Vec<ConnectorInfo>, String> {
        let entries = self.registry.list_entries().await;
        Ok(entries
            .into_iter()
            .map(|e| {
                let name = e.adapter_id.clone();
                ConnectorInfo {
                    id: e.adapter_id,
                    name,
                    resources: e
                        .resources
                        .into_iter()
                        .map(|r| ResourceInfo {
                            name: r.name,
                            uri: String::new(),
                            method: "BRIDGE".to_string(),
                            description: r.description,
                        })
                        .collect(),
                    capabilities: e
                        .capabilities
                        .into_iter()
                        .map(|c| CapabilityInfo {
                            name: c.name,
                            schema: c.schema.unwrap_or(serde_json::Value::Null),
                            description: c.description,
                        })
                        .collect(),
                }
            })
            .collect())
    }

    async fn query_resource(
        &self,
        connector_id: &str,
        resource_name: &str,
        params: Option<HashMap<String, String>>,
    ) -> Result<serde_json::Value, String> {
        // Get the pending requests handle
        let pending = self
            .registry
            .pending_requests(connector_id)
            .await
            .map_err(|e| format!("Worker '{}' not connected: {}", connector_id, e))?;

        // Generate request ID and register
        let request_id = format!("req-{}", uuid::Uuid::new_v4());
        let rx = pending.register(request_id.clone());

        // Send ResourceQuery to worker
        let params_value = params.map(|p| serde_json::to_value(p).unwrap_or_default());
        let msg = BridgeServerMessage::ResourceQuery {
            request_id: request_id.clone(),
            resource: resource_name.to_string(),
            params: params_value,
        };

        self.registry
            .send(connector_id, msg)
            .await
            .map_err(|e| format!("Failed to send query: {}", e))?;

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS), rx)
            .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(format!(
                "Request {} cancelled (worker disconnected)",
                request_id
            )),
            Err(_) => {
                // Clean up the pending request
                pending.reject(&request_id, "Timeout".to_string());
                Err(format!(
                    "Resource query timed out after {}s",
                    REQUEST_TIMEOUT_SECONDS
                ))
            }
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
        // Check if worker is connected for synchronous invocation
        if self.registry.is_connected(connector_id).await {
            // Synchronous: send via WS and wait for response
            let pending = self
                .registry
                .pending_requests(connector_id)
                .await
                .map_err(|e| format!("Worker '{}' not connected: {}", connector_id, e))?;

            let request_id = format!("req-{}", uuid::Uuid::new_v4());
            let rx = pending.register(request_id.clone());

            let msg = BridgeServerMessage::CapabilityInvoke {
                request_id: request_id.clone(),
                capability: capability.to_string(),
                payload: payload.clone(),
            };

            self.registry
                .send(connector_id, msg)
                .await
                .map_err(|e| format!("Failed to send capability invoke: {}", e))?;

            match tokio::time::timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS), rx)
                .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(format!(
                    "Request {} cancelled (worker disconnected)",
                    request_id
                )),
                Err(_) => {
                    pending.reject(&request_id, "Timeout".to_string());
                    Err(format!(
                        "Capability invoke timed out after {}s",
                        REQUEST_TIMEOUT_SECONDS
                    ))
                }
            }
        } else {
            // Asynchronous: write to outbox for delivery when worker reconnects
            let id = self
                .outbox_repo
                .insert(
                    connector_id,
                    capability,
                    &payload,
                    Some(session_id),
                    None,
                    Some(agent_id),
                )
                .map_err(|e| format!("Failed to enqueue capability: {}", e))?;

            Ok(serde_json::json!({
                "queued": true,
                "outbox_id": id,
                "message": "Worker offline, queued for delivery on reconnect"
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending_requests::PendingRequests;
    use tokio::sync::mpsc;

    fn setup() -> (Arc<BridgeRegistry>, Arc<OutboxRepository>) {
        use gateway_services::VaultPaths;

        let dir = tempfile::TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(dir.path().to_path_buf()));
        let db = Arc::new(zbot_stores_sqlite::DatabaseManager::new(paths).unwrap());
        let outbox = Arc::new(OutboxRepository::new(db));
        let registry = Arc::new(BridgeRegistry::new());
        (registry, outbox)
    }

    #[tokio::test]
    async fn test_list_connectors_empty() {
        let (registry, outbox) = setup();
        let provider = BridgeResourceProvider::new(registry, outbox);
        let connectors = provider.list_connectors().await.unwrap();
        assert!(connectors.is_empty());
    }

    #[tokio::test]
    async fn test_list_connectors_with_worker() {
        let (registry, outbox) = setup();
        let (tx, _rx) = mpsc::unbounded_channel();
        let pending = Arc::new(PendingRequests::new());

        registry
            .register(
                "crm".to_string(),
                vec![crate::protocol::WorkerCapability {
                    name: "send_email".to_string(),
                    description: Some("Send email".to_string()),
                    schema: None,
                }],
                vec![crate::protocol::WorkerResource {
                    name: "contacts".to_string(),
                    description: Some("CRM contacts".to_string()),
                }],
                tx,
                pending,
            )
            .await
            .unwrap();

        let provider = BridgeResourceProvider::new(registry, outbox);
        let connectors = provider.list_connectors().await.unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].id, "crm");
        assert_eq!(connectors[0].resources.len(), 1);
        assert_eq!(connectors[0].capabilities.len(), 1);
        assert_eq!(connectors[0].resources[0].method, "BRIDGE");
    }

    #[tokio::test]
    async fn test_query_resource_not_connected() {
        let (registry, outbox) = setup();
        let provider = BridgeResourceProvider::new(registry, outbox);
        let result = provider.query_resource("unknown", "contacts", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_query_resource_with_response() {
        let (registry, outbox) = setup();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pending = Arc::new(PendingRequests::new());
        let pending_clone = pending.clone();

        registry
            .register("crm".to_string(), vec![], vec![], tx, pending)
            .await
            .unwrap();

        let provider = BridgeResourceProvider::new(registry, outbox);

        // Spawn worker simulator that responds to queries
        tokio::spawn(async move {
            if let Some(BridgeServerMessage::ResourceQuery { request_id, .. }) = rx.recv().await {
                pending_clone.resolve(&request_id, serde_json::json!([{"name": "Alice"}]));
            }
        });

        let result = provider.query_resource("crm", "contacts", None).await;
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.is_array());
    }

    #[tokio::test]
    async fn test_invoke_capability_queued_when_offline() {
        let (registry, outbox) = setup();
        let provider = BridgeResourceProvider::new(registry, outbox);

        let result = provider
            .invoke_capability(
                "slack",
                "send_message",
                serde_json::json!({"text": "hi"}),
                "sess-1",
                "root",
            )
            .await;

        // Should succeed but return queued response
        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(data["queued"], true);
        assert!(data["outbox_id"].as_str().unwrap().starts_with("obx-"));
    }

    #[tokio::test]
    async fn test_invoke_capability_synchronous() {
        let (registry, outbox) = setup();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pending = Arc::new(PendingRequests::new());
        let pending_clone = pending.clone();

        registry
            .register("slack".to_string(), vec![], vec![], tx, pending)
            .await
            .unwrap();

        let provider = BridgeResourceProvider::new(registry, outbox);

        // Spawn worker simulator
        tokio::spawn(async move {
            if let Some(BridgeServerMessage::CapabilityInvoke { request_id, .. }) = rx.recv().await
            {
                pending_clone.resolve(
                    &request_id,
                    serde_json::json!({"success": true, "message_id": "msg-123"}),
                );
            }
        });

        let result = provider
            .invoke_capability(
                "slack",
                "send_message",
                serde_json::json!({"text": "hi"}),
                "sess-1",
                "root",
            )
            .await;

        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(data["success"], true);
    }
}
