//! # Gateway Connectors
//!
//! External connector management for bidirectional messaging.
//!
//! Connectors are external processes (any language) that register with Gateway to:
//! - **Receive** messages from agents at end of execution
//! - **Trigger** agent sessions via Gateway API

pub mod config;
pub mod dispatch;
pub mod inbound;
pub mod service;

pub use config::{
    ConnectorCapability, ConnectorConfig, ConnectorMetadata, ConnectorPayload, ConnectorResource,
    ConnectorTransport, ConnectorsStore, CreateConnectorRequest, DispatchContext, ResponseSchema,
    UpdateConnectorRequest,
};
pub use dispatch::{dispatch, DispatchError, DispatchResponse, DispatchResult};
pub use inbound::{InboundLogEntry, InboundPayload, InboundResult, InboundSender};
pub use service::{ConnectorResult, ConnectorService, ConnectorServiceError, TestResult};

/// Safe failed-HTTP diagnostic shared by dispatch and resource reads. Upstream
/// error bodies can echo host credentials and are never diagnostic context.
pub fn http_failure_summary(status: u16) -> String {
    format!("Connector returned HTTP {status}")
}

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Maximum number of inbound log entries to keep in the ring buffer.
const INBOUND_LOG_MAX: usize = 500;

/// Registry for managing and dispatching to connectors.
///
/// Provides in-memory caching of connectors with disk persistence.
#[derive(Clone)]
pub struct ConnectorRegistry {
    service: ConnectorService,
    cache: Arc<RwLock<Option<Vec<ConnectorConfig>>>>,
    inbound_log: Arc<RwLock<Vec<InboundLogEntry>>>,
}

impl ConnectorRegistry {
    /// Create a new connector registry.
    pub fn new(service: ConnectorService) -> Self {
        Self {
            service,
            cache: Arc::new(RwLock::new(None)),
            inbound_log: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Log an inbound message. Trims to INBOUND_LOG_MAX entries.
    pub async fn log_inbound(&self, entry: InboundLogEntry) {
        let mut log = self.inbound_log.write().await;
        log.push(entry);
        if log.len() > INBOUND_LOG_MAX {
            let excess = log.len() - INBOUND_LOG_MAX;
            log.drain(0..excess);
        }
    }

    /// Get inbound log entries for a connector, most recent first.
    pub async fn get_inbound_log(&self, connector_id: &str, limit: usize) -> Vec<InboundLogEntry> {
        let log = self.inbound_log.read().await;
        log.iter()
            .rev()
            .filter(|e| e.connector_id == connector_id)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Initialize the registry by loading connectors from disk.
    pub async fn init(&self) -> ConnectorResult<()> {
        let connectors = self.service.list().await?;
        let count = connectors.len();
        *self.cache.write().await = Some(connectors);
        info!(count = count, "Connector registry initialized");
        Ok(())
    }

    /// Invalidate the cache, forcing a reload on next access.
    async fn invalidate_cache(&self) {
        *self.cache.write().await = None;
    }

    /// Get connectors, loading from disk if not cached.
    async fn get_connectors(&self) -> ConnectorResult<Vec<ConnectorConfig>> {
        {
            let cache = self.cache.read().await;
            if let Some(connectors) = &*cache {
                return Ok(connectors.clone());
            }
        }

        // Load from disk and cache
        let connectors = self.service.list().await?;
        *self.cache.write().await = Some(connectors.clone());
        Ok(connectors)
    }

    /// List all connectors.
    pub async fn list(&self) -> ConnectorResult<Vec<ConnectorConfig>> {
        self.get_connectors().await
    }

    /// Get a connector by ID.
    pub async fn get(&self, id: &str) -> ConnectorResult<ConnectorConfig> {
        let connectors = self.get_connectors().await?;
        connectors
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| ConnectorServiceError::NotFound(id.to_string()))
    }

    /// Get connectors that are enabled for outbound dispatch.
    pub async fn get_enabled_outbound(&self) -> ConnectorResult<Vec<ConnectorConfig>> {
        let connectors = self.get_connectors().await?;
        Ok(connectors
            .into_iter()
            .filter(|c| c.enabled && c.outbound_enabled)
            .collect())
    }

    /// Create a new connector.
    pub async fn create(
        &self,
        request: CreateConnectorRequest,
    ) -> ConnectorResult<ConnectorConfig> {
        let connector = self.service.create(request).await?;
        self.invalidate_cache().await;
        Ok(connector)
    }

    /// Update an existing connector.
    pub async fn update(
        &self,
        id: &str,
        request: UpdateConnectorRequest,
    ) -> ConnectorResult<ConnectorConfig> {
        let connector = self.service.update(id, request).await?;
        self.invalidate_cache().await;
        Ok(connector)
    }

    /// Delete a connector.
    pub async fn delete(&self, id: &str) -> ConnectorResult<()> {
        self.service.delete(id).await?;
        self.invalidate_cache().await;
        Ok(())
    }

    /// Test connectivity to a connector.
    pub async fn test(&self, id: &str) -> ConnectorResult<TestResult> {
        self.service.test(id).await
    }

    /// Dispatch a response to multiple connectors.
    ///
    /// This is called at the end of execution to route responses to the
    /// specified connectors via their configured transport.
    pub async fn dispatch_to_many(
        &self,
        connector_ids: &[String],
        capability: &str,
        payload: serde_json::Value,
        context: &DispatchContext,
    ) -> Vec<(String, DispatchResult<DispatchResponse>)> {
        let mut results = Vec::new();

        for connector_id in connector_ids {
            let result = self
                .dispatch_to_one(connector_id, capability, payload.clone(), context)
                .await;
            results.push((connector_id.clone(), result));
        }

        results
    }

    /// Dispatch a response to a single connector.
    pub async fn dispatch_to_one(
        &self,
        connector_id: &str,
        capability: &str,
        payload: serde_json::Value,
        context: &DispatchContext,
    ) -> DispatchResult<DispatchResponse> {
        let connector = match self.get(connector_id).await {
            Ok(c) => c,
            Err(e) => {
                error!(
                    connector_id = %connector_id,
                    error = %e,
                    "Connector not found for dispatch"
                );
                return Err(DispatchError::NotFound(connector_id.to_string()));
            }
        };

        debug!(
            connector_id = %connector_id,
            capability = %capability,
            session_id = %context.session_id,
            "Dispatching to connector"
        );

        dispatch(&connector, capability, payload, context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn test_registry() -> (ConnectorRegistry, TempDir) {
        use agent_primitives::vault_paths::VaultPaths;

        let temp_dir = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(temp_dir.path().to_path_buf()));
        let service = ConnectorService::new(paths);
        let registry = ConnectorRegistry::new(service);
        registry.init().await.unwrap();
        (registry, temp_dir)
    }

    #[tokio::test]
    async fn test_registry_caching() {
        let (registry, _temp) = test_registry().await;

        // Create a connector
        registry
            .create(CreateConnectorRequest {
                id: "test".to_string(),
                name: "Test".to_string(),
                transport: ConnectorTransport::Http {
                    callback_url: "http://localhost:9001".to_string(),
                    method: "POST".to_string(),
                    headers: HashMap::new(),
                    timeout_ms: None,
                },
                metadata: Default::default(),
                enabled: true,
                outbound_enabled: true,
                inbound_enabled: true,
            })
            .await
            .unwrap();

        // Should be in cache
        let list1 = registry.list().await.unwrap();
        assert_eq!(list1.len(), 1);

        // Get same from cache
        let list2 = registry.list().await.unwrap();
        assert_eq!(list2.len(), 1);

        // Delete invalidates cache
        registry.delete("test").await.unwrap();
        let list3 = registry.list().await.unwrap();
        assert!(list3.is_empty());
    }

    #[tokio::test]
    async fn test_inbound_log_push_and_get() {
        let (registry, _temp) = test_registry().await;

        // Log some entries
        for i in 0..5 {
            registry
                .log_inbound(InboundLogEntry {
                    connector_id: "slack".to_string(),
                    message: format!("msg-{}", i),
                    sender: None,
                    thread_id: None,
                    session_id: format!("sess-{}", i),
                    received_at: chrono::Utc::now(),
                })
                .await;
        }

        // Add one for a different connector
        registry
            .log_inbound(InboundLogEntry {
                connector_id: "email".to_string(),
                message: "email-msg".to_string(),
                sender: None,
                thread_id: None,
                session_id: "sess-email".to_string(),
                received_at: chrono::Utc::now(),
            })
            .await;

        // Get all for slack (should be 5, most recent first)
        let slack_logs = registry.get_inbound_log("slack", 100).await;
        assert_eq!(slack_logs.len(), 5);
        assert_eq!(slack_logs[0].message, "msg-4"); // most recent first

        // Get with limit
        let limited = registry.get_inbound_log("slack", 2).await;
        assert_eq!(limited.len(), 2);

        // Get for email (should be 1)
        let email_logs = registry.get_inbound_log("email", 100).await;
        assert_eq!(email_logs.len(), 1);
    }

    #[tokio::test]
    async fn test_inbound_log_ring_buffer_trim() {
        let (registry, _temp) = test_registry().await;

        // Push more than INBOUND_LOG_MAX entries
        for i in 0..550 {
            registry
                .log_inbound(InboundLogEntry {
                    connector_id: "test".to_string(),
                    message: format!("msg-{}", i),
                    sender: None,
                    thread_id: None,
                    session_id: format!("sess-{}", i),
                    received_at: chrono::Utc::now(),
                })
                .await;
        }

        // Check total size is trimmed
        let log = registry.inbound_log.read().await;
        assert!(log.len() <= 500);
        // Oldest entries should be trimmed — first entry should be msg-50
        assert_eq!(log[0].message, "msg-50");
    }

    #[tokio::test]
    async fn test_get_enabled_outbound() {
        let (registry, _temp) = test_registry().await;

        // Create enabled connector
        registry
            .create(CreateConnectorRequest {
                id: "enabled".to_string(),
                name: "Enabled".to_string(),
                transport: ConnectorTransport::Http {
                    callback_url: "http://localhost:9001".to_string(),
                    method: "POST".to_string(),
                    headers: HashMap::new(),
                    timeout_ms: None,
                },
                metadata: Default::default(),
                enabled: true,
                outbound_enabled: true,
                inbound_enabled: true,
            })
            .await
            .unwrap();

        // Create disabled connector
        registry
            .create(CreateConnectorRequest {
                id: "disabled".to_string(),
                name: "Disabled".to_string(),
                transport: ConnectorTransport::Http {
                    callback_url: "http://localhost:9002".to_string(),
                    method: "POST".to_string(),
                    headers: HashMap::new(),
                    timeout_ms: None,
                },
                metadata: Default::default(),
                enabled: true,
                outbound_enabled: false, // Outbound disabled
                inbound_enabled: true,
            })
            .await
            .unwrap();

        let enabled = registry.get_enabled_outbound().await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "enabled");
    }
}
