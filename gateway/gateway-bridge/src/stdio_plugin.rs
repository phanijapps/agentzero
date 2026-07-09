//! # STDIO Plugin
//!
//! Process management for STDIO-based plugins.
//!
//! Handles spawning Node.js plugin processes, npm dependency installation,
//! and message framing (newline-delimited JSON).

use crate::outbox::OutboxRepository;
use crate::pending_requests::PendingRequests;
use crate::plugin_config::{PluginConfig, PluginError, PluginState, PluginUserConfig};
use crate::protocol::{BridgeServerMessage, WorkerCapability, WorkerMessage, WorkerResource};
use crate::push;
use crate::registry::BridgeRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

/// Config file name (must match plugin_service.rs).
const CONFIG_FILE_NAME: &str = ".config.json";

/// Timeout for npm install in seconds.
const NPM_INSTALL_TIMEOUT_SECS: u64 = 120;

/// Timeout for hello handshake in seconds.
const HELLO_TIMEOUT_SECS: u64 = 10;

/// Heartbeat interval in seconds.
const HEARTBEAT_SECONDS: u64 = 30;

/// A STDIO-based plugin that communicates via newline-delimited JSON.
pub struct StdioPlugin {
    /// Plugin configuration.
    config: PluginConfig,
    /// Plugin directory path.
    plugin_dir: PathBuf,
    /// Current state.
    state: PluginState,
    /// Last error message (if failed).
    last_error: Option<String>,
    /// Child process handle.
    child: Option<Child>,
    /// Stdin for sending messages to plugin.
    stdin: Option<ChildStdin>,
    /// Channel to receive messages from the plugin task.
    msg_rx: Option<mpsc::Receiver<WorkerMessage>>,
    /// Task handle for the stdout reader.
    reader_task: Option<tokio::task::JoinHandle<()>>,
    /// Bridge registry for registration.
    registry: Arc<BridgeRegistry>,
    /// Bridge outbox for message delivery.
    outbox: Arc<OutboxRepository>,
    /// Pending requests for correlation.
    pending: Arc<PendingRequests>,
    /// Channel for sending server messages to the plugin.
    server_tx: Option<mpsc::UnboundedSender<BridgeServerMessage>>,
    /// Heartbeat task handle.
    heartbeat_task: Option<tokio::task::JoinHandle<()>>,
    /// Gateway bus for triggering agent sessions.
    bus: Option<Arc<dyn gateway_bus::GatewayBus>>,
}

impl StdioPlugin {
    /// Create a new STDIO plugin instance.
    pub fn new(
        config: PluginConfig,
        plugin_dir: PathBuf,
        registry: Arc<BridgeRegistry>,
        outbox: Arc<OutboxRepository>,
        bus: Option<Arc<dyn gateway_bus::GatewayBus>>,
    ) -> Self {
        Self {
            config,
            plugin_dir,
            state: PluginState::Discovered,
            last_error: None,
            child: None,
            stdin: None,
            msg_rx: None,
            reader_task: None,
            registry,
            outbox,
            pending: Arc::new(PendingRequests::new()),
            server_tx: None,
            heartbeat_task: None,
            bus,
        }
    }

    /// Get the plugin ID.
    pub fn id(&self) -> &str {
        &self.config.id
    }

    /// Get the plugin configuration.
    pub fn config(&self) -> &PluginConfig {
        &self.config
    }

    /// Get the current plugin state.
    pub fn state(&self) -> PluginState {
        self.state
    }

    /// Get the last error message.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Check if node_modules exists in the plugin directory.
    fn node_modules_exist(&self) -> bool {
        self.plugin_dir.join("node_modules").exists()
    }

    /// Load user configuration from .config.json in the plugin directory.
    fn load_user_config(&self) -> PluginUserConfig {
        let config_path = self.plugin_dir.join(CONFIG_FILE_NAME);

        match PluginUserConfig::load(&config_path) {
            Ok(config) => {
                tracing::debug!(
                    plugin_id = %self.config.id,
                    path = %config_path.display(),
                    "Loaded plugin user config"
                );
                config
            }
            Err(e) => {
                tracing::debug!(
                    plugin_id = %self.config.id,
                    "No user config found, using defaults: {}", e
                );
                PluginUserConfig::default()
            }
        }
    }

    /// Ensure npm dependencies are installed.
    pub async fn ensure_dependencies(&mut self) -> Result<(), PluginError> {
        if self.node_modules_exist() {
            tracing::debug!(
                plugin_id = %self.config.id,
                "node_modules already exists, skipping npm install"
            );
            return Ok(());
        }

        tracing::info!(
            plugin_id = %self.config.id,
            "Installing dependencies for plugin"
        );

        self.state = PluginState::Installing;

        // Run npm install --production
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(NPM_INSTALL_TIMEOUT_SECS),
            async {
                #[cfg(windows)]
                let output = Command::new("cmd.exe")
                    .args(["/c", "npm install --production"])
                    .current_dir(&self.plugin_dir)
                    .output()
                    .await?;

                #[cfg(not(windows))]
                let output = Command::new("npm")
                    .args(["install", "--production"])
                    .current_dir(&self.plugin_dir)
                    .output()
                    .await?;

                Ok::<_, std::io::Error>(output)
            },
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    tracing::info!(
                        plugin_id = %self.config.id,
                        "npm install completed successfully"
                    );
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let error = format!("npm install failed: {}", stderr);
                    tracing::error!(
                        plugin_id = %self.config.id,
                        "npm install failed: {}", stderr
                    );
                    self.state = PluginState::Failed;
                    self.last_error = Some(error.clone());
                    Err(PluginError::NpmInstallFailed(error))
                }
            }
            Ok(Err(e)) => {
                let error = format!("Failed to run npm install: {}", e);
                tracing::error!(
                    plugin_id = %self.config.id,
                    "Failed to run npm install: {}", e
                );
                self.state = PluginState::Failed;
                self.last_error = Some(error.clone());
                Err(PluginError::NpmInstallFailed(error))
            }
            Err(_) => {
                let error = format!("npm install timed out after {}s", NPM_INSTALL_TIMEOUT_SECS);
                tracing::error!(
                    plugin_id = %self.config.id,
                    "npm install timed out"
                );
                self.state = PluginState::Failed;
                self.last_error = Some(error.clone());
                Err(PluginError::NpmInstallFailed(error))
            }
        }
    }

    /// Start the plugin process and run the message loop.
    pub async fn start(&mut self) -> Result<(), PluginError> {
        if !self.config.enabled {
            self.state = PluginState::Disabled;
            return Err(PluginError::Disabled(self.config.id.clone()));
        }

        if self.state == PluginState::Running {
            return Err(PluginError::AlreadyRunning(self.config.id.clone()));
        }

        // Ensure dependencies are installed
        self.ensure_dependencies().await?;

        tracing::info!(
            plugin_id = %self.config.id,
            entry = %self.config.entry,
            "Starting plugin process"
        );

        self.state = PluginState::Starting;

        // Build the command to run node with the entry script
        let entry_path = self.plugin_dir.join(&self.config.entry);

        #[cfg(windows)]
        let mut cmd = Command::new("cmd.exe");
        #[cfg(windows)]
        {
            let entry_str = entry_path.to_string_lossy();
            cmd.args(["/c", "node", &entry_str]);
        }

        #[cfg(not(windows))]
        let mut cmd = Command::new("node");
        #[cfg(not(windows))]
        {
            cmd.arg(&entry_path);
        }

        // Set working directory
        cmd.current_dir(&self.plugin_dir);

        // Load user config and merge secrets with env vars
        let user_config = self.load_user_config();
        let resolved_env = user_config.resolve_env_with_secrets(&self.config.env);
        for (key, value) in resolved_env {
            cmd.env(key, value);
        }

        // Set up stdio
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Kill the process when the handle is dropped
        cmd.kill_on_drop(true);

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| {
            let error = format!("Failed to spawn plugin process: {}", e);
            self.state = PluginState::Failed;
            self.last_error = Some(error.clone());
            PluginError::SpawnFailed(error)
        })?;

        // Take stdin and stdout
        let stdin = child.stdin.take().ok_or_else(|| {
            PluginError::SpawnFailed("Failed to get stdin from plugin process".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            PluginError::SpawnFailed("Failed to get stdout from plugin process".to_string())
        })?;

        // Create channel for messages from the plugin
        let (msg_tx, msg_rx) = mpsc::channel::<WorkerMessage>(64);

        // Spawn a task to read stdout and parse JSON messages
        let plugin_id = self.config.id.clone();
        let reader_task = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                match serde_json::from_str::<WorkerMessage>(&line) {
                    Ok(msg) => {
                        tracing::trace!(
                            plugin_id = %plugin_id,
                            "Received message from plugin: {:?}",
                            msg
                        );
                        if msg_tx.send(msg).await.is_err() {
                            tracing::debug!(
                                plugin_id = %plugin_id,
                                "Plugin message channel closed"
                            );
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            plugin_id = %plugin_id,
                            line = %line,
                            "Failed to parse plugin message: {}",
                            e
                        );
                    }
                }
            }

            tracing::debug!(plugin_id = %plugin_id, "Plugin stdout reader ended");
        });

        // Store handles
        self.stdin = Some(stdin);
        self.msg_rx = Some(msg_rx);
        self.reader_task = Some(reader_task);
        self.child = Some(child);

        // Wait for Hello handshake
        let (capabilities, resources) = self.wait_for_hello().await?;

        // After successful handshake, run the main loop
        self.run_loop(capabilities, resources).await
    }

    /// Wait for the plugin to send a Hello message.
    async fn wait_for_hello(
        &mut self,
    ) -> Result<(Vec<WorkerCapability>, Vec<WorkerResource>), PluginError> {
        let msg_rx = self.msg_rx.as_mut().ok_or_else(|| {
            PluginError::CommunicationError("No message channel available".to_string())
        })?;

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(HELLO_TIMEOUT_SECS), async {
                while let Some(msg) = msg_rx.recv().await {
                    if let WorkerMessage::Hello {
                        adapter_id,
                        capabilities,
                        resources,
                        resume: _,
                    } = msg
                    {
                        // Verify adapter_id matches
                        if adapter_id != self.config.id {
                            return Err(PluginError::HandshakeFailed(format!(
                                "Plugin sent wrong adapter_id: expected {}, got {}",
                                self.config.id, adapter_id
                            )));
                        }
                        return Ok((capabilities, resources));
                    }
                    // Ignore other messages during handshake
                }
                Err(PluginError::HandshakeFailed(
                    "Connection closed before Hello".to_string(),
                ))
            })
            .await;

        match result {
            Ok(Ok((capabilities, resources))) => {
                tracing::info!(
                    plugin_id = %self.config.id,
                    capabilities = capabilities.len(),
                    resources = resources.len(),
                    "Plugin sent Hello"
                );
                Ok((capabilities, resources))
            }
            Ok(Err(e)) => {
                self.state = PluginState::Failed;
                self.last_error = Some(e.to_string());
                Err(e)
            }
            Err(_) => {
                let error = format!("Hello handshake timed out after {}s", HELLO_TIMEOUT_SECS);
                self.state = PluginState::Failed;
                self.last_error = Some(error.clone());
                Err(PluginError::HandshakeFailed(error))
            }
        }
    }

    /// Run the main message loop after Hello handshake.
    async fn run_loop(
        &mut self,
        capabilities: Vec<WorkerCapability>,
        resources: Vec<WorkerResource>,
    ) -> Result<(), PluginError> {
        let adapter_id = self.config.id.clone();

        // Step 1: Create channel for sending messages to this plugin
        let (server_tx, mut server_rx) = mpsc::unbounded_channel::<BridgeServerMessage>();
        self.server_tx = Some(server_tx.clone());

        // Step 2: Register with BridgeRegistry
        if let Err(e) = self
            .registry
            .register(
                adapter_id.clone(),
                capabilities,
                resources,
                server_tx,
                self.pending.clone(),
            )
            .await
        {
            let error = format!("Registration failed: {}", e);
            tracing::error!(plugin_id = %adapter_id, "{}", error);
            self.state = PluginState::Failed;
            self.last_error = Some(error.clone());
            return Err(PluginError::HandshakeFailed(error));
        }

        tracing::info!(
            plugin_id = %adapter_id,
            "Plugin registered with bridge registry"
        );

        // Step 3: Send HelloAck
        self.send_hello_ack().await?;

        // Step 4: Replay pending outbox items
        let replay_items = self.outbox.get_unacked(&adapter_id).unwrap_or_default();
        if !replay_items.is_empty() {
            tracing::info!(
                plugin_id = %adapter_id,
                count = replay_items.len(),
                "Replaying pending outbox items"
            );
            for item in &replay_items {
                push::push_single_item(&self.outbox, &self.registry, &adapter_id, item).await;
            }
        }

        // Step 5: Spawn heartbeat task
        let heartbeat_adapter_id = adapter_id.clone();
        let heartbeat_registry = self.registry.clone();
        let heartbeat_handle = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_SECONDS));
            loop {
                interval.tick().await;
                if heartbeat_registry
                    .send(&heartbeat_adapter_id, BridgeServerMessage::Ping)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        self.heartbeat_task = Some(heartbeat_handle);

        // Step 6: Set state to running
        self.state = PluginState::Running;
        tracing::info!(plugin_id = %adapter_id, "Plugin started successfully");

        // Step 7: Message loop
        // Take ownership of msg_rx for the loop
        let mut msg_rx = self.msg_rx.take().ok_or_else(|| {
            PluginError::CommunicationError("No message channel available".to_string())
        })?;

        // Take ownership of stdin for sending
        let mut stdin = self
            .stdin
            .take()
            .ok_or_else(|| PluginError::CommunicationError("No stdin available".to_string()))?;

        // Clone things we need in the loop
        let outbox = self.outbox.clone();
        let pending = self.pending.clone();
        let bus = self.bus.clone();

        loop {
            tokio::select! {
                // Messages from the plugin
                plugin_msg = msg_rx.recv() => {
                    match plugin_msg {
                        Some(msg) => {
                            Self::handle_plugin_message_static(
                                &adapter_id,
                                msg,
                                &outbox,
                                &pending,
                                bus.as_deref(),
                            ).await;
                        }
                        None => {
                            tracing::info!(plugin_id = %adapter_id, "Plugin disconnected");
                            break;
                        }
                    }
                }

                // Messages to send to the plugin (from registry.send())
                server_msg = server_rx.recv() => {
                    match server_msg {
                        Some(msg) => {
                            let json = match serde_json::to_string(&msg) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::warn!("Failed to serialize server message: {}", e);
                                    continue;
                                }
                            };
                            if stdin.write_all(format!("{}\n", json).as_bytes()).await.is_err()
                                || stdin.flush().await.is_err()
                            {
                                tracing::warn!(plugin_id = %adapter_id, "Failed to send message to plugin");
                                break;
                            }
                        }
                        None => {
                            // Channel closed
                            break;
                        }
                    }
                }
            }
        }

        // Step 8: Cleanup
        self.cleanup(&adapter_id).await;

        // Put stdin back for cleanup
        self.stdin = Some(stdin);
        self.msg_rx = Some(msg_rx);

        Ok(())
    }

    /// Handle a message from the plugin (static version to avoid borrow issues).
    async fn handle_plugin_message_static(
        adapter_id: &str,
        msg: WorkerMessage,
        outbox: &OutboxRepository,
        pending: &PendingRequests,
        bus: Option<&dyn gateway_bus::GatewayBus>,
    ) {
        match msg {
            WorkerMessage::Pong => {
                tracing::trace!(plugin_id = %adapter_id, "Received pong");
            }

            WorkerMessage::Ack { outbox_id } => {
                tracing::debug!(plugin_id = %adapter_id, outbox_id = %outbox_id, "ACK received");
                if let Err(e) = outbox.mark_sent(&outbox_id) {
                    tracing::warn!("Failed to mark outbox sent: {}", e);
                }
            }

            WorkerMessage::Fail {
                outbox_id,
                error,
                retry_after_seconds,
            } => {
                tracing::warn!(
                    plugin_id = %adapter_id,
                    outbox_id = %outbox_id,
                    error = %error,
                    "FAIL received from plugin"
                );
                let retry_after = retry_after_seconds
                    .map(|s| chrono::Utc::now() + chrono::Duration::seconds(s as i64));
                if let Err(e) = outbox.mark_failed(&outbox_id, &error, retry_after) {
                    tracing::warn!("Failed to mark outbox failed: {}", e);
                }
            }

            WorkerMessage::ResourceResponse { request_id, data } => {
                tracing::debug!(plugin_id = %adapter_id, request_id = %request_id, "ResourceResponse received");
                if !pending.resolve(&request_id, data) {
                    tracing::warn!("No pending request for: {}", request_id);
                }
            }

            WorkerMessage::CapabilityResponse { request_id, result } => {
                tracing::debug!(plugin_id = %adapter_id, request_id = %request_id, "CapabilityResponse received");
                if !pending.resolve(&request_id, result) {
                    tracing::warn!("No pending request for: {}", request_id);
                }
            }

            WorkerMessage::Inbound {
                text,
                thread_id,
                sender,
                agent_id,
                metadata,
            } => {
                tracing::info!(plugin_id = %adapter_id, "Inbound message from plugin");

                if let Some(bus) = bus {
                    let mut request = gateway_bus::SessionRequest::new(
                        agent_id.unwrap_or_else(|| "root".to_string()),
                        text,
                    )
                    .with_respond_to(vec![adapter_id.to_string()])
                    .with_connector_id(adapter_id.to_string());

                    // Set source to "connector"
                    request.source = serde_json::from_str("\"connector\"").unwrap_or_default();

                    if let Some(tid) = thread_id {
                        request = request.with_thread_id(tid);
                    }

                    // Note: sender is stored in metadata, no with_sender method
                    if let Some(meta) = metadata {
                        request.metadata = Some(meta);
                    }

                    // Add sender to metadata if present
                    if let Some(s) = sender {
                        request.metadata = Some({
                            let mut m = request.metadata.unwrap_or_else(|| serde_json::json!({}));
                            if let Some(obj) = m.as_object_mut() {
                                obj.insert("sender".to_string(), serde_json::json!(s));
                            }
                            m
                        });
                    }

                    if let Err(e) = bus.submit(request).await {
                        tracing::error!(plugin_id = %adapter_id, "Failed to submit inbound message: {}", e);
                    }
                } else {
                    tracing::warn!(plugin_id = %adapter_id, "No bus available, cannot process inbound message");
                }
            }

            WorkerMessage::Hello { .. } => {
                tracing::warn!(plugin_id = %adapter_id, "Received unexpected Hello after registration");
            }
        }
    }

    /// Cleanup resources on disconnect.
    async fn cleanup(&mut self, adapter_id: &str) {
        // Cancel heartbeat task
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }

        // Unregister from bridge registry
        self.registry.unregister(adapter_id).await;

        // Cancel pending requests
        self.pending.cancel_all();

        // Reset inflight items back to pending for retry on reconnect
        if let Err(e) = self.outbox.reset_inflight(adapter_id) {
            tracing::warn!(plugin_id = %adapter_id, "Failed to reset inflight: {}", e);
        }

        // Update state
        self.state = PluginState::Stopped;

        tracing::info!(plugin_id = %adapter_id, "Plugin session ended");
    }

    /// Send HelloAck to the plugin.
    async fn send_hello_ack(&mut self) -> Result<(), PluginError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| PluginError::CommunicationError("No stdin available".to_string()))?;

        let ack = BridgeServerMessage::HelloAck {
            server_time: chrono::Utc::now().to_rfc3339(),
            heartbeat_seconds: HEARTBEAT_SECONDS,
        };

        let json = serde_json::to_string(&ack).map_err(|e| {
            PluginError::CommunicationError(format!("Failed to serialize HelloAck: {}", e))
        })?;

        stdin
            .write_all(format!("{}\n", json).as_bytes())
            .await
            .map_err(|e| {
                PluginError::CommunicationError(format!("Failed to send HelloAck: {}", e))
            })?;

        stdin.flush().await.map_err(|e| {
            PluginError::CommunicationError(format!("Failed to flush HelloAck: {}", e))
        })?;

        tracing::debug!(plugin_id = %self.config.id, "Sent HelloAck to plugin");
        Ok(())
    }

    /// Stop the plugin process.
    pub async fn stop(&mut self) -> Result<(), PluginError> {
        if self.state != PluginState::Running {
            return Ok(());
        }

        tracing::info!(plugin_id = %self.config.id, "Stopping plugin");

        // Cancel heartbeat task
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }

        // Cancel reader task
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }

        // Close stdin
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.shutdown().await;
        }

        // Kill the process
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }

        // Unregister from bridge registry
        self.registry.unregister(&self.config.id).await;

        // Cancel pending requests
        self.pending.cancel_all();

        // Close channels
        self.msg_rx = None;
        self.server_tx = None;

        self.state = PluginState::Stopped;

        tracing::info!(plugin_id = %self.config.id, "Plugin stopped");
        Ok(())
    }

    /// Send a message to the plugin.
    pub async fn send(&mut self, msg: BridgeServerMessage) -> Result<(), PluginError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| PluginError::CommunicationError("Plugin not running".to_string()))?;

        let json = serde_json::to_string(&msg).map_err(|e| {
            PluginError::CommunicationError(format!("Failed to serialize message: {}", e))
        })?;

        stdin
            .write_all(format!("{}\n", json).as_bytes())
            .await
            .map_err(|e| {
                PluginError::CommunicationError(format!("Failed to send message: {}", e))
            })?;

        stdin.flush().await.map_err(|e| {
            PluginError::CommunicationError(format!("Failed to flush message: {}", e))
        })?;

        Ok(())
    }
}

impl Drop for StdioPlugin {
    fn drop(&mut self) {
        // Clean up tasks
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_modules_exist() {
        let dir = tempfile::tempdir().unwrap();
        let config = PluginConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            ..Default::default()
        };

        let registry = Arc::new(BridgeRegistry::new());
        let outbox = Arc::new(OutboxRepository::new(Arc::new(
            zbot_stores_sqlite::DatabaseManager::new(Arc::new(gateway_services::VaultPaths::new(
                dir.path().to_path_buf(),
            )))
            .unwrap(),
        )));

        let plugin = StdioPlugin::new(config, dir.path().to_path_buf(), registry, outbox, None);

        assert!(!plugin.node_modules_exist());

        // Create node_modules
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        assert!(plugin.node_modules_exist());
    }
}
