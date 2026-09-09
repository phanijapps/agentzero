//! # Connector Service
//!
//! CRUD operations and persistence for connectors.

use crate::config::{
    ConnectorConfig, ConnectorsStore, CreateConnectorRequest, UpdateConnectorRequest,
};
use agent_primitives::vault_paths::SharedVaultPaths;
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs;
use tracing::{debug, info};

/// Errors from connector service operations.
#[derive(Error, Debug)]
pub enum ConnectorServiceError {
    #[error("Connector not found: {0}")]
    NotFound(String),

    #[error("Connector already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid connector ID: {0}")]
    InvalidId(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result type for connector service operations.
pub type ConnectorResult<T> = Result<T, ConnectorServiceError>;

/// Service for managing connectors with persistence.
#[derive(Clone)]
pub struct ConnectorService {
    paths: SharedVaultPaths,
}

impl ConnectorService {
    /// Create a new connector service.
    pub fn new(paths: SharedVaultPaths) -> Self {
        Self { paths }
    }

    /// Get the config file path.
    fn config_path(&self) -> PathBuf {
        self.paths.connectors()
    }

    /// Load connectors from disk.
    async fn load(&self) -> ConnectorResult<ConnectorsStore> {
        if !self.config_path().exists() {
            debug!("Connectors config not found, returning empty store");
            return Ok(ConnectorsStore::default());
        }

        let content = fs::read_to_string(&self.config_path()).await?;
        let store: ConnectorsStore = serde_json::from_str(&content)?;
        debug!(
            count = store.connectors.len(),
            "Loaded connectors from disk"
        );
        Ok(store)
    }

    /// Save connectors to disk.
    async fn save(&self, store: &ConnectorsStore) -> ConnectorResult<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.config_path().parent() {
            fs::create_dir_all(parent).await?;
        }

        let content = serde_json::to_string_pretty(store)?;
        fs::write(&self.config_path(), content).await?;
        debug!(
            path = %self.config_path().display(),
            count = store.connectors.len(),
            "Saved connectors to disk"
        );
        Ok(())
    }

    /// List all connectors.
    pub async fn list(&self) -> ConnectorResult<Vec<ConnectorConfig>> {
        let store = self.load().await?;
        Ok(store.connectors)
    }

    /// Get a connector by ID.
    pub async fn get(&self, id: &str) -> ConnectorResult<ConnectorConfig> {
        let store = self.load().await?;
        store
            .connectors
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| ConnectorServiceError::NotFound(id.to_string()))
    }

    /// Create a new connector.
    pub async fn create(
        &self,
        request: CreateConnectorRequest,
    ) -> ConnectorResult<ConnectorConfig> {
        // Validate ID
        if request.id.is_empty() {
            return Err(ConnectorServiceError::InvalidId(
                "ID cannot be empty".to_string(),
            ));
        }

        if !request
            .id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ConnectorServiceError::InvalidId(format!(
                "ID '{}' contains invalid characters (only alphanumeric, -, _ allowed)",
                request.id
            )));
        }

        let mut store = self.load().await?;

        // Check for duplicates
        if store.connectors.iter().any(|c| c.id == request.id) {
            return Err(ConnectorServiceError::AlreadyExists(request.id));
        }

        let now = chrono::Utc::now();
        let connector = ConnectorConfig {
            id: request.id.clone(),
            name: request.name,
            transport: request.transport,
            metadata: request.metadata,
            enabled: request.enabled,
            outbound_enabled: request.outbound_enabled,
            inbound_enabled: request.inbound_enabled,
            created_at: Some(now),
            updated_at: Some(now),
        };

        store.connectors.push(connector.clone());
        self.save(&store).await?;

        info!(connector_id = %connector.id, "Created connector");
        Ok(connector)
    }

    /// Update an existing connector.
    pub async fn update(
        &self,
        id: &str,
        request: UpdateConnectorRequest,
    ) -> ConnectorResult<ConnectorConfig> {
        let mut store = self.load().await?;

        let connector = store
            .connectors
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| ConnectorServiceError::NotFound(id.to_string()))?;

        // Apply updates
        if let Some(name) = request.name {
            connector.name = name;
        }
        if let Some(transport) = request.transport {
            connector.transport = transport;
        }
        if let Some(metadata) = request.metadata {
            connector.metadata = metadata;
        }
        if let Some(enabled) = request.enabled {
            connector.enabled = enabled;
        }
        if let Some(outbound_enabled) = request.outbound_enabled {
            connector.outbound_enabled = outbound_enabled;
        }
        if let Some(inbound_enabled) = request.inbound_enabled {
            connector.inbound_enabled = inbound_enabled;
        }
        connector.updated_at = Some(chrono::Utc::now());

        let updated = connector.clone();
        self.save(&store).await?;

        info!(connector_id = %id, "Updated connector");
        Ok(updated)
    }

    /// Delete a connector.
    pub async fn delete(&self, id: &str) -> ConnectorResult<()> {
        let mut store = self.load().await?;

        let initial_len = store.connectors.len();
        store.connectors.retain(|c| c.id != id);

        if store.connectors.len() == initial_len {
            return Err(ConnectorServiceError::NotFound(id.to_string()));
        }

        self.save(&store).await?;
        info!(connector_id = %id, "Deleted connector");
        Ok(())
    }

    /// Test connectivity to a connector.
    pub async fn test(&self, id: &str) -> ConnectorResult<TestResult> {
        let connector = self.get(id).await?;

        match &connector.transport {
            super::config::ConnectorTransport::Http { callback_url, .. } => {
                // Try a HEAD or OPTIONS request to test connectivity
                let client = reqwest::Client::builder().build().expect("reqwest client");
                let start = std::time::Instant::now();

                match client
                    .head(callback_url)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(response) => {
                        let latency_ms = start.elapsed().as_millis() as u64;
                        Ok(TestResult {
                            success: true,
                            message: format!("Connection successful (HTTP {})", response.status()),
                            latency_ms: Some(latency_ms),
                        })
                    }
                    Err(e) => Ok(TestResult {
                        success: false,
                        message: format!("Connection failed: {}", e),
                        latency_ms: None,
                    }),
                }
            }
            super::config::ConnectorTransport::Cli { command, .. } => {
                // Check if command exists
                let which_result = if cfg!(windows) {
                    tokio::process::Command::new("where")
                        .arg(command)
                        .output()
                        .await
                } else {
                    tokio::process::Command::new("which")
                        .arg(command)
                        .output()
                        .await
                };

                match which_result {
                    Ok(output) if output.status.success() => Ok(TestResult {
                        success: true,
                        message: format!("Command '{}' found", command),
                        latency_ms: None,
                    }),
                    _ => Ok(TestResult {
                        success: false,
                        message: format!("Command '{}' not found in PATH", command),
                        latency_ms: None,
                    }),
                }
            }
            _ => Ok(TestResult {
                success: false,
                message: "Transport type not yet testable".to_string(),
                latency_ms: None,
            }),
        }
    }

    /// Enable a connector.
    pub async fn enable(&self, id: &str) -> ConnectorResult<ConnectorConfig> {
        self.update(
            id,
            UpdateConnectorRequest {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
    }

    /// Disable a connector.
    pub async fn disable(&self, id: &str) -> ConnectorResult<ConnectorConfig> {
        self.update(
            id,
            UpdateConnectorRequest {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
    }
}

/// Result of a connector connectivity test.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
}

// Default is now derived on UpdateConnectorRequest

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConnectorTransport;
    use agent_primitives::vault_paths::VaultPaths;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn test_service() -> (ConnectorService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(temp_dir.path().to_path_buf()));
        let service = ConnectorService::new(paths);
        (service, temp_dir)
    }

    #[tokio::test]
    async fn test_crud_operations() {
        let (service, _temp) = test_service().await;

        // Create
        let created = service
            .create(CreateConnectorRequest {
                id: "test-connector".to_string(),
                name: "Test Connector".to_string(),
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

        assert_eq!(created.id, "test-connector");
        assert!(created.created_at.is_some());

        // List
        let list = service.list().await.unwrap();
        assert_eq!(list.len(), 1);

        // Get
        let retrieved = service.get("test-connector").await.unwrap();
        assert_eq!(retrieved.name, "Test Connector");

        // Update
        let updated = service
            .update(
                "test-connector",
                UpdateConnectorRequest {
                    name: Some("Updated Name".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "Updated Name");

        // Delete
        service.delete("test-connector").await.unwrap();
        let list = service.list().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_duplicate_prevention() {
        let (service, _temp) = test_service().await;

        service
            .create(CreateConnectorRequest {
                id: "unique".to_string(),
                name: "First".to_string(),
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

        let result = service
            .create(CreateConnectorRequest {
                id: "unique".to_string(),
                name: "Second".to_string(),
                transport: ConnectorTransport::Http {
                    callback_url: "http://localhost:9002".to_string(),
                    method: "POST".to_string(),
                    headers: HashMap::new(),
                    timeout_ms: None,
                },
                metadata: Default::default(),
                enabled: true,
                outbound_enabled: true,
                inbound_enabled: true,
            })
            .await;

        assert!(matches!(
            result,
            Err(ConnectorServiceError::AlreadyExists(_))
        ));
    }

    #[tokio::test]
    async fn test_invalid_id() {
        let (service, _temp) = test_service().await;

        let result = service
            .create(CreateConnectorRequest {
                id: "invalid id with spaces".to_string(),
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
            .await;

        assert!(matches!(result, Err(ConnectorServiceError::InvalidId(_))));
    }
}
