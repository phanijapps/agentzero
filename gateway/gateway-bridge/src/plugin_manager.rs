//! # Plugin Manager
//!
//! Lifecycle management for STDIO plugins.
//!
//! Handles discovery, start/stop/restart, and wires plugins to BridgeRegistry.

use crate::outbox::OutboxRepository;
use crate::plugin_config::{PluginConfig, PluginError, PluginState, PluginSummary};
use crate::registry::BridgeRegistry;
use crate::stdio_plugin::StdioPlugin;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Manages the lifecycle of STDIO plugins.
pub struct PluginManager {
    /// Map of plugin ID to plugin instance.
    plugins: Arc<RwLock<HashMap<String, PluginEntry>>>,
    /// Bridge registry for worker registration.
    registry: Arc<BridgeRegistry>,
    /// Bridge outbox for message delivery.
    outbox: Arc<OutboxRepository>,
    /// Directory containing plugins.
    plugins_dir: PathBuf,
    /// Gateway bus for triggering agent sessions (set later).
    bus: Arc<Mutex<Option<Arc<dyn gateway_bus::GatewayBus>>>>,
}

/// Config file name (must match plugin_service.rs).
const CONFIG_FILE_NAME: &str = ".config.json";

/// Entry for a managed plugin.
struct PluginEntry {
    /// Plugin configuration.
    config: PluginConfig,
    /// Plugin directory path.
    plugin_dir: PathBuf,
    /// Current state.
    state: PluginState,
    /// Last error message.
    last_error: Option<String>,
    /// Running plugin task handle (if running).
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl PluginManager {
    /// Create a new plugin manager.
    pub fn new(
        plugins_dir: PathBuf,
        registry: Arc<BridgeRegistry>,
        outbox: Arc<OutboxRepository>,
        bus: Option<Arc<dyn gateway_bus::GatewayBus>>,
    ) -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            registry,
            outbox,
            plugins_dir,
            bus: Arc::new(Mutex::new(bus)),
        }
    }

    /// Discover all plugins in the plugins directory.
    ///
    /// Scans the plugins directory for subdirectories containing plugin.json.
    pub async fn discover(&self) -> Result<Vec<String>, PluginError> {
        // Ensure plugins directory exists
        if !self.plugins_dir.exists() {
            tracing::info!(
                plugins_dir = %self.plugins_dir.display(),
                "Plugins directory does not exist, creating"
            );
            std::fs::create_dir_all(&self.plugins_dir)?;
        }

        let mut discovered = Vec::new();
        let mut plugins = self.plugins.write().await;

        // Read directory entries
        let entries = match std::fs::read_dir(&self.plugins_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read plugins directory: {}", e);
                return Ok(discovered);
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Check for plugin.json
            let manifest_path = path.join("plugin.json");
            if !manifest_path.exists() {
                tracing::debug!(
                    dir = %path.display(),
                    "Skipping directory without plugin.json"
                );
                continue;
            }

            // Load plugin config
            match PluginConfig::from_dir(&path) {
                Ok(config) => {
                    let plugin_id = config.id.clone();

                    // Skip if already registered
                    if plugins.contains_key(&plugin_id) {
                        tracing::debug!(
                            plugin_id = %plugin_id,
                            "Plugin already discovered"
                        );
                        continue;
                    }

                    tracing::info!(
                        plugin_id = %plugin_id,
                        name = %config.name,
                        "Discovered plugin"
                    );

                    // Initialize config file if it doesn't exist
                    let config_path = path.join(CONFIG_FILE_NAME);
                    if !config_path.exists() {
                        if let Err(e) = initialize_plugin_config(&config_path) {
                            tracing::warn!(
                                plugin_id = %plugin_id,
                                "Failed to initialize config file: {}", e
                            );
                        }
                    }

                    let initial_state = if config.enabled {
                        PluginState::Discovered
                    } else {
                        PluginState::Disabled
                    };

                    plugins.insert(
                        plugin_id.clone(),
                        PluginEntry {
                            config,
                            plugin_dir: path,
                            state: initial_state,
                            last_error: None,
                            task_handle: None,
                        },
                    );

                    discovered.push(plugin_id);
                }
                Err(e) => {
                    tracing::warn!(
                        dir = %path.display(),
                        "Failed to load plugin: {}", e
                    );
                }
            }
        }

        Ok(discovered)
    }

    /// Start all enabled plugins.
    pub async fn start_all(&self) {
        let plugins = self.plugins.read().await;
        let plugin_ids: Vec<String> = plugins
            .iter()
            .filter(|(_, entry)| entry.config.enabled && entry.state != PluginState::Running)
            .map(|(id, _)| id.clone())
            .collect();
        drop(plugins);

        for plugin_id in plugin_ids {
            if let Err(e) = self.start(&plugin_id).await {
                tracing::error!(plugin_id = %plugin_id, "Failed to start plugin: {}", e);
            }
        }
    }

    /// Stop all running plugins.
    pub async fn stop_all(&self) {
        let plugins = self.plugins.read().await;
        let plugin_ids: Vec<String> = plugins
            .iter()
            .filter(|(_, entry)| entry.state == PluginState::Running)
            .map(|(id, _)| id.clone())
            .collect();
        drop(plugins);

        for plugin_id in plugin_ids {
            if let Err(e) = self.stop(&plugin_id).await {
                tracing::error!(plugin_id = %plugin_id, "Failed to stop plugin: {}", e);
            }
        }
    }

    /// Start a specific plugin.
    pub async fn start(&self, plugin_id: &str) -> Result<(), PluginError> {
        let (config, plugin_dir) = {
            let mut plugins = self.plugins.write().await;
            let entry = plugins
                .get_mut(plugin_id)
                .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;

            if entry.state == PluginState::Running {
                return Err(PluginError::AlreadyRunning(plugin_id.to_string()));
            }

            if !entry.config.enabled {
                entry.state = PluginState::Disabled;
                return Err(PluginError::Disabled(plugin_id.to_string()));
            }

            entry.state = PluginState::Starting;
            entry.last_error = None;

            (entry.config.clone(), entry.plugin_dir.clone())
        };

        // Create a new plugin instance and spawn it
        let registry = self.registry.clone();
        let outbox = self.outbox.clone();
        let bus = self.bus.lock().await.clone();
        let plugin_id_owned = plugin_id.to_string();
        let plugins = self.plugins.clone();

        let handle = tokio::spawn(async move {
            let mut plugin = StdioPlugin::new(config, plugin_dir, registry, outbox, bus);

            if let Err(e) = plugin.start().await {
                tracing::error!(plugin_id = %plugin_id_owned, "Plugin failed to start: {}", e);

                // Update state to failed
                let mut plugins = plugins.write().await;
                if let Some(entry) = plugins.get_mut(&plugin_id_owned) {
                    entry.state = PluginState::Failed;
                    entry.last_error = Some(e.to_string());
                }
            } else {
                // Update state to running
                let mut plugins = plugins.write().await;
                if let Some(entry) = plugins.get_mut(&plugin_id_owned) {
                    entry.state = PluginState::Running;
                    entry.task_handle = None; // Task completed successfully
                }
            }
        });

        // Store the task handle
        let mut plugins = self.plugins.write().await;
        if let Some(entry) = plugins.get_mut(plugin_id) {
            entry.task_handle = Some(handle);
        }

        Ok(())
    }

    /// Stop a specific plugin.
    pub async fn stop(&self, plugin_id: &str) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().await;
        let entry = plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;

        if entry.state != PluginState::Running {
            return Err(PluginError::NotRunning(plugin_id.to_string()));
        }

        // Abort the task if it's still running
        if let Some(handle) = entry.task_handle.take() {
            handle.abort();
        }

        entry.state = PluginState::Stopped;
        tracing::info!(plugin_id = %plugin_id, "Plugin stopped");

        Ok(())
    }

    /// Restart a specific plugin.
    pub async fn restart(&self, plugin_id: &str) -> Result<(), PluginError> {
        // Stop if running
        let _ = self.stop(plugin_id).await;

        // Small delay to allow cleanup
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Start again
        self.start(plugin_id).await
    }

    /// Get the state of a specific plugin.
    pub async fn get_state(&self, plugin_id: &str) -> Option<PluginState> {
        let plugins = self.plugins.read().await;
        plugins.get(plugin_id).map(|e| e.state)
    }

    /// List all plugins with their summaries.
    pub async fn list(&self) -> Vec<PluginSummary> {
        let plugins = self.plugins.read().await;
        plugins
            .values()
            .map(|entry| PluginSummary {
                id: entry.config.id.clone(),
                name: entry.config.name.clone(),
                version: entry.config.version.clone(),
                description: entry.config.description.clone(),
                state: entry.state,
                auto_restart: entry.config.auto_restart,
                enabled: entry.config.enabled,
                error: entry.last_error.clone(),
            })
            .collect()
    }

    /// Get a plugin summary by ID.
    pub async fn get(&self, plugin_id: &str) -> Option<PluginSummary> {
        let plugins = self.plugins.read().await;
        plugins.get(plugin_id).map(|entry| PluginSummary {
            id: entry.config.id.clone(),
            name: entry.config.name.clone(),
            version: entry.config.version.clone(),
            description: entry.config.description.clone(),
            state: entry.state,
            auto_restart: entry.config.auto_restart,
            enabled: entry.config.enabled,
            error: entry.last_error.clone(),
        })
    }

    /// Check if a plugin exists.
    pub async fn exists(&self, plugin_id: &str) -> bool {
        let plugins = self.plugins.read().await;
        plugins.contains_key(plugin_id)
    }

    /// Get the number of managed plugins.
    pub async fn len(&self) -> usize {
        let plugins = self.plugins.read().await;
        plugins.len()
    }

    /// Check if there are no plugins.
    pub async fn is_empty(&self) -> bool {
        let plugins = self.plugins.read().await;
        plugins.is_empty()
    }

    /// Set the gateway bus for triggering agent sessions from inbound messages.
    ///
    /// This should be called after the bus is created (typically in server.start())
    /// to enable plugins to trigger agent sessions.
    pub async fn set_bus(&self, bus: Arc<dyn gateway_bus::GatewayBus>) {
        let mut bus_guard = self.bus.lock().await;
        *bus_guard = Some(bus);
        tracing::info!("PluginManager bus configured");
    }
}

/// Initialize an empty config file for a plugin.
fn initialize_plugin_config(path: &std::path::Path) -> Result<(), std::io::Error> {
    let empty_config = "{}\n";
    std::fs::write(path, empty_config)?;

    // Set file permissions to owner-only for security (on Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    tracing::info!(
        path = %path.display(),
        "Initialized plugin config file"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_plugin_manager_creation() {
        let dir = tempdir().unwrap();
        let registry = Arc::new(BridgeRegistry::new());

        // Create a mock database manager
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        let db = Arc::new(zbot_stores_sqlite::DatabaseManager::new(paths).unwrap());
        let outbox = Arc::new(OutboxRepository::new(db));

        let manager = PluginManager::new(dir.path().join("plugins"), registry, outbox, None);

        assert!(manager.is_empty().await);
    }

    #[tokio::test]
    async fn test_discover_empty_dir() {
        let dir = tempdir().unwrap();
        let registry = Arc::new(BridgeRegistry::new());

        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        let db = Arc::new(zbot_stores_sqlite::DatabaseManager::new(paths).unwrap());
        let outbox = Arc::new(OutboxRepository::new(db));

        let manager = PluginManager::new(dir.path().join("plugins"), registry, outbox, None);

        let discovered = manager.discover().await.unwrap();
        assert!(discovered.is_empty());
    }

    #[tokio::test]
    async fn test_discover_plugin() {
        let dir = tempdir().unwrap();
        let plugins_dir = dir.path().join("plugins");
        let plugin_dir = plugins_dir.join("test-plugin");

        // Create plugin directory with manifest
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let manifest = r#"{"id": "test-plugin", "name": "Test Plugin"}"#;
        std::fs::write(plugin_dir.join("plugin.json"), manifest).unwrap();

        let registry = Arc::new(BridgeRegistry::new());
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        let db = Arc::new(zbot_stores_sqlite::DatabaseManager::new(paths).unwrap());
        let outbox = Arc::new(OutboxRepository::new(db));

        let manager = PluginManager::new(plugins_dir, registry, outbox, None);

        let discovered = manager.discover().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0], "test-plugin");

        let list = manager.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "test-plugin");
        assert_eq!(list[0].name, "Test Plugin");
        assert_eq!(list[0].state, PluginState::Discovered);
    }
}
