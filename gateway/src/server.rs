//! # Gateway Server
//!
//! Main server lifecycle management.

use crate::bus::HttpGatewayBus;
use crate::config::GatewayConfig;
use crate::cron::{CronScheduler, CronService};
use crate::error::{GatewayError, Result};
use crate::events::EventBus;
use crate::http::create_http_router;
use crate::services::{AgentService, RuntimeService};
use crate::state::AppState;
use crate::websocket::WebSocketHandler;
use discovery::InterfaceEnumerator;
use gateway_services::{FileWatcher, WatchConfig};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Gateway server managing HTTP and WebSocket endpoints.
pub struct GatewayServer {
    config: GatewayConfig,
    state: AppState,
    ws_handler: Arc<WebSocketHandler>,
    shutdown_tx: Option<broadcast::Sender<()>>,
    bridge_retry_handle: Option<tokio::task::JoinHandle<()>>,
    durable_work_worker: Option<gateway_bus::WorkWorkerHandle>,
    file_watcher: Option<FileWatcher>,
}

impl GatewayServer {
    /// Create a new gateway server with the given configuration.
    pub fn new(config: GatewayConfig, vault_dir: PathBuf) -> Self {
        let state = AppState::new(vault_dir);
        let ws_handler = Arc::new(WebSocketHandler::new_with_surfaces(
            state.event_bus.clone(),
            state.runtime.clone(),
            config.agent_surfaces_enabled,
        ));

        Self {
            config,
            state,
            ws_handler,
            shutdown_tx: None,
            bridge_retry_handle: None,
            durable_work_worker: None,
            file_watcher: None,
        }
    }

    /// Create with default configuration and default data directory.
    pub fn with_defaults() -> Self {
        // Use ~/Documents/zbot as the default data directory
        let data_dir = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("zbot");
        Self::new(GatewayConfig::default(), data_dir)
    }

    /// Get the application state.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Get the event bus for broadcasting events.
    pub fn event_bus(&self) -> Arc<EventBus> {
        self.state.event_bus.clone()
    }

    /// Get the WebSocket handler.
    pub fn ws_handler(&self) -> Arc<WebSocketHandler> {
        self.ws_handler.clone()
    }

    /// Get the agent service.
    pub fn agents(&self) -> Arc<AgentService> {
        self.state.agents.clone()
    }

    /// Get the runtime service.
    pub fn runtime(&self) -> Arc<RuntimeService> {
        self.state.runtime.clone()
    }

    /// Start the gateway server.
    ///
    /// This spawns the HTTP server, including its `/ws` WebSocket route, and
    /// returns immediately.
    /// Use `shutdown()` to stop the servers.
    pub async fn start(&mut self) -> Result<()> {
        // Mark any RUNNING sessions as CRASHED (daemon was interrupted)
        self.recover_crashed_sessions();

        // Reset any bridge outbox items left inflight from crash
        if let Err(e) = self.state.bridge_outbox.reset_all_inflight() {
            warn!("Failed to reset bridge inflight items: {}", e);
        }

        // Create gateway bus for bridge inbound messages (MUST be before seed_defaults
        // which starts plugins, so plugins have the bus available for inbound messages)
        if let Some(runner) = self.state.runtime.runner() {
            let bus: Arc<dyn gateway_bus::GatewayBus> = Arc::new(HttpGatewayBus::new(
                runner.clone(),
                self.state.state_service.clone(),
                self.state.paths.vault_dir().clone(),
            ));
            // Set bus on plugin manager so plugins can trigger agent sessions
            self.state.plugin_manager.set_bus(bus.clone()).await;
            self.state.bridge_bus = Some(bus);
        }

        // Reconcile embedding backend: preflight Ollama, reindex on
        // dim/model mismatch, spawn periodic health loop.
        self.state.reconcile_embeddings_at_boot().await;

        // Seed default agents and other initial data (starts plugins)
        self.state.seed_defaults().await;

        // Initialize connector registry
        if let Err(e) = self.state.connector_registry.init().await {
            warn!("Failed to initialize connector registry: {}", e);
        }

        // Start file watchers for hot-reload
        self.start_file_watchers();

        // Initialize cron scheduler
        self.init_cron_scheduler().await;

        // Spawn bridge outbox retry loop
        self.bridge_retry_handle = Some(gateway_bridge::spawn_retry_loop(
            self.state.bridge_registry.clone(),
            self.state.bridge_outbox.clone(),
        ));

        let (shutdown_tx, _) = broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx.clone());
        self.start_durable_work_worker()?;

        // Spawn the WS background tasks (subscription cleanup + event
        // router) BEFORE any WS transport starts accepting connections.
        // These used to live inside `WebSocketHandler::run` — moving them
        // out fixes a regression where the unified-port mode
        // (`run` disabled by default) left the event router unspawned,
        // so invokes ran server-side but no tokens reached the UI.
        self.ws_handler.spawn_background_tasks(&shutdown_tx);

        // Read network settings from AppSettings (cached in SettingsService).
        // If exposeToLan changed since startup, the user must restart — this
        // read happens at boot so a stale toggle from a prior run is fine.
        let network_cfg = match self.state.settings.load() {
            Ok(s) => s.network,
            Err(e) => {
                warn!(
                    "Failed to load settings.json for network config: {}; defaulting to LAN exposure ON",
                    e
                );
                discovery::DiscoveryConfig::default()
            }
        };
        let resolved_host = crate::config::resolve_bind_host(&network_cfg);
        if resolved_host != self.config.host {
            info!(
                "Bind host resolved from network settings: {} (was {})",
                resolved_host, self.config.host
            );
            self.config.host = resolved_host;
        }

        // Start HTTP server. The WebSocket upgrade route (`/ws`) shares
        // this listener — mobile clients and single-port deployments
        // don't need a second firewall hole.
        let http_addr = self.config.http_addr();
        let http_router = create_http_router(
            self.config.clone(),
            self.state.clone(),
            self.ws_handler.clone(),
        );
        let http_shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            info!("Starting HTTP server on {}", http_addr);
            let listener = match tokio::net::TcpListener::bind(&http_addr).await {
                Ok(l) => l,
                Err(e) => {
                    warn!("Failed to bind HTTP server: {}", e);
                    return;
                }
            };

            let server = axum::serve(
                listener,
                http_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let mut rx = http_shutdown_rx;
                let _ = rx.recv().await;
                info!("HTTP server shutting down");
            });

            if let Err(e) = server.await {
                warn!("HTTP server error: {}", e);
            }
        });

        // Start mDNS advertisement when LAN exposure is enabled.
        if network_cfg.expose_to_lan {
            let actual_port = self.config.http_port;

            let instance_name = network_cfg
                .discovery
                .instance_name
                .clone()
                .unwrap_or_else(default_instance_name);

            let instance_id = match network_cfg.discovery.instance_id.clone() {
                Some(id) => id,
                None => {
                    let new_id = uuid::Uuid::new_v4().to_string();
                    if let Err(e) = persist_instance_id(&self.state.settings, &new_id) {
                        warn!("failed to persist generated instance_id: {}", e);
                    }
                    new_id
                }
            };

            // Same const lives in `http::health` — mDNS TXT and the
            // `/api/health` body should agree on what version is
            // running. See that module's `VERSION` for the
            // `option_env!("BUILD_VERSION")` fallback shape.
            const VERSION: &str = match option_env!("BUILD_VERSION") {
                Some(v) => v,
                None => env!("CARGO_PKG_VERSION"),
            };
            let mut txt = std::collections::BTreeMap::new();
            txt.insert("version".into(), VERSION.to_string());
            txt.insert("instance".into(), instance_id.clone());
            txt.insert("name".into(), instance_name.clone());
            txt.insert("path".into(), "/".into());
            txt.insert("ws".into(), "1".into());
            for (k, v) in &network_cfg.discovery.txt_records {
                txt.entry(k.clone()).or_insert_with(|| v.clone());
            }

            let enumerator = discovery::RealEnumerator;
            let interfaces = discovery::filter_interfaces(
                enumerator.enumerate(),
                &network_cfg.discovery.exclude_interfaces,
            );
            let addrs = discovery::ipv4_only(&interfaces);

            let info = discovery::ServiceInfo {
                instance_name: instance_name.clone(),
                service_type: network_cfg.discovery.service_type.clone(),
                hostname_alias: network_cfg.discovery.hostname_alias.clone(),
                port: actual_port,
                txt,
                addrs,
            };

            let advertiser: Arc<dyn discovery::Advertiser> = match discovery::MdnsAdvertiser::new()
            {
                Ok(a) => Arc::new(a),
                Err(e) => {
                    warn!(
                        "mDNS responder failed to start: {}; daemon reachable via IP only",
                        e
                    );
                    discovery::noop()
                }
            };

            match advertiser.advertise(info) {
                Ok(handle) => {
                    if let Ok(mut guard) = self.state.advertise_handle.lock() {
                        *guard = Some(handle);
                    }
                    info!("mDNS advertising started for {}", instance_name);
                }
                Err(e) => warn!("mDNS advertise failed: {}; daemon reachable via IP only", e),
            }

            // Decision: AppState.advertiser stays as noop. The live mDNS daemon's
            // lifetime is rooted in the AdvertiseHandle's boxed AdvertiseInner
            // (see discovery::advertiser), not in this Arc. /api/network/info
            // treats advertise_handle.is_some() as the source of truth for "mDNS
            // is live", not AppState.advertiser's concrete type.
            drop(advertiser);
        }

        info!(
            "Gateway started - HTTP+WS (unified): {}",
            self.config.http_addr()
        );

        Ok(())
    }

    /// Shutdown the gateway server gracefully.
    ///
    /// This pauses all running sessions so they can be resumed on restart,
    /// disconnects bridge workers, and sends the shutdown signal.
    pub async fn shutdown(&mut self) {
        // Withdraw mDNS advertisement before tearing down the listener.
        if let Ok(mut guard) = self.state.advertise_handle.lock() {
            if let Some(handle) = guard.take() {
                drop(handle); // Drop impl sends goodbye, blocks ~100ms
                info!("mDNS advertisement withdrawn");
            }
        }

        // Pause all running sessions before shutting down
        self.pause_running_sessions();

        // Stop file watcher
        if let Some(ref mut w) = self.file_watcher {
            w.stop();
        }

        // Disconnect all bridge workers and abort retry loop
        self.state.bridge_registry.disconnect_all().await;
        if let Some(handle) = self.bridge_retry_handle.take() {
            handle.abort();
        }

        if let Some(worker) = self.durable_work_worker.take() {
            if let Err(error) = worker.shutdown().await {
                warn!(
                    reason_code = error.reason_code(),
                    "Durable work worker shutdown did not settle its active claim"
                );
            }
        }

        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(());
            info!("Gateway shutdown signal sent");
        }
    }

    fn start_durable_work_worker(&mut self) -> Result<()> {
        if self.durable_work_worker.is_some() {
            return Err(GatewayError::Internal(
                "durable work worker already started".to_owned(),
            ));
        }
        let config = gateway_bus::WorkWorkerConfig::new(
            "zbot.local",
            format!("gateway-{}", uuid::Uuid::new_v4()),
            gateway_bus::WorkWorkerLimits::default(),
        )
        .map_err(|_| {
            GatewayError::Internal("durable work worker configuration invalid".to_owned())
        })?;
        let worker = gateway_bus::DurableWorkWorker::new(
            self.state.durable_work_store.clone(),
            self.state.durable_work_transport.clone(),
            gateway_bus::WorkHandlerRegistry::empty(),
            config,
        );
        self.durable_work_worker = Some(worker.start());
        Ok(())
    }

    /// Pause all running sessions during graceful shutdown.
    ///
    /// This marks running sessions as paused so they can be resumed when the server restarts.
    fn pause_running_sessions(&self) {
        match self.state.state_service.mark_running_as_paused() {
            Ok(count) if count > 0 => {
                info!("Paused {} running session(s) for graceful shutdown", count);
            }
            Ok(_) => {
                info!("No running sessions to pause");
            }
            Err(e) => {
                warn!("Failed to pause running sessions: {}", e);
            }
        }
    }

    /// Recover sessions that were interrupted by unexpected crash.
    ///
    /// Sessions in RUNNING state at startup indicate an unexpected crash
    /// (graceful shutdown would have paused them). Mark them as CRASHED.
    fn recover_crashed_sessions(&self) {
        match self.state.state_service.mark_running_as_crashed() {
            Ok(count) if count > 0 => {
                warn!(
                    "Found {} session(s) still in RUNNING state - marked as CRASHED (unexpected shutdown)",
                    count
                );
            }
            Ok(_) => {
                // No running sessions means either clean start or graceful previous shutdown
            }
            Err(e) => {
                warn!("Failed to recover crashed sessions: {}", e);
            }
        }
    }

    /// Start a single consolidated file watcher for skills + agents.
    ///
    /// One inotify instance shared across both directories; falls back to
    /// polling if inotify is unavailable (e.g. kernel `fs.inotify.max_user_instances`
    /// cap hit). Either way the daemon starts successfully and hot-reload works.
    fn start_file_watchers(&mut self) {
        let mut watcher = FileWatcher::new(WatchConfig::default());

        let skills = self.state.skills.clone();
        watcher.add_watch(self.state.paths.skills_dir(), "skills", move |path| {
            tracing::info!("Skills changed: {:?}, invalidating cache", path);
            let skills = skills.clone();
            tokio::spawn(async move {
                skills.invalidate_cache().await;
            });
        });

        let agents = self.state.agents.clone();
        watcher.add_watch(self.state.paths.agents_dir(), "agents", move |path| {
            tracing::info!("Agents changed: {:?}, invalidating cache", path);
            let agents = agents.clone();
            tokio::spawn(async move {
                agents.invalidate_cache().await;
            });
        });

        let event_bus = self.state.event_bus.clone();
        let config_dir = self.state.paths.config_dir();
        let config_dir_for_filter = config_dir.clone();
        watcher.add_watch(config_dir.clone(), "customization", move |path| {
            // Compute relative path; reject anything outside the customization allow-list.
            let rel = match path.strip_prefix(&config_dir_for_filter) {
                Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                Err(_) => return,
            };
            if !rel.ends_with(".md") {
                return;
            }
            let parts: Vec<&str> = rel.split('/').collect();
            let valid = matches!(parts.as_slice(), ["agent", _] | ["agent-prompts", _]);
            if !valid {
                return;
            }
            let modified_at = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                })
                .unwrap_or_default();
            let event = gateway_events::GatewayEvent::CustomizationFileChanged {
                path: rel,
                modified_at,
            };
            event_bus.publish_sync(event);
        });

        watcher.start();
        self.file_watcher = Some(watcher);
    }

    /// Initialize the cron scheduler.
    async fn init_cron_scheduler(&mut self) {
        // Get the execution runner from the runtime service
        let runner = match self.state.runtime.runner() {
            Some(r) => r.clone(),
            None => {
                warn!("Cannot initialize cron scheduler: execution runner not available");
                return;
            }
        };

        // Create the gateway bus for cron to submit sessions
        let gateway_bus = Arc::new(HttpGatewayBus::new(
            runner,
            self.state.state_service.clone(),
            self.state.paths.vault_dir().clone(),
        ));

        // Create cron service and scheduler
        let cron_service = CronService::new(self.state.paths.clone());

        match CronScheduler::new(cron_service, gateway_bus).await {
            Ok(scheduler) => {
                // Start the scheduler
                if let Err(e) = scheduler.start().await {
                    warn!("Failed to start cron scheduler: {}", e);
                    return;
                }

                // Store in state
                self.state.cron_scheduler = Some(Arc::new(scheduler));

                info!("Cron scheduler initialized and started");
            }
            Err(e) => {
                warn!("Failed to create cron scheduler: {}", e);
            }
        }
    }
}

pub(crate) fn default_instance_name() -> String {
    let raw = gethostname::gethostname().to_string_lossy().into_owned();
    let trimmed = raw.trim_end_matches(".local").to_string();
    if trimmed.is_empty() {
        "agentzero".to_string()
    } else {
        trimmed
    }
}

pub(crate) fn persist_instance_id(
    settings: &gateway_services::SettingsService,
    new_id: &str,
) -> std::result::Result<(), String> {
    let mut current = settings.load()?;
    current.network.discovery.instance_id = Some(new_id.to_string());
    settings.save(&current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use execution_state::{
        WorkAuthorization, WorkDraft, WorkEnvelope, WorkFailureCode, WorkPolicy, WorkPolicyError,
        WorkStatus,
    };
    use gateway_bus::WorkTransport;
    use tempfile::TempDir;

    struct GatewayWorkPolicy;

    impl WorkPolicy for GatewayWorkPolicy {
        fn authorize(
            &self,
            _draft: &WorkDraft,
        ) -> std::result::Result<WorkAuthorization, WorkPolicyError> {
            Ok(WorkAuthorization::new(
                "gateway-test",
                "node-local",
                "root",
                "session-test",
                "execution-test",
            ))
        }
    }

    #[tokio::test]
    async fn test_server_creation() {
        let temp_dir = TempDir::new().unwrap();
        let vault_dir = temp_dir.path().to_path_buf();
        let server = GatewayServer::new(GatewayConfig::default(), vault_dir);
        assert!(server.shutdown_tx.is_none());
    }

    #[tokio::test]
    async fn test_custom_config() {
        let temp_dir = TempDir::new().unwrap();
        let vault_dir = temp_dir.path().to_path_buf();
        let config = GatewayConfig {
            http_port: 19001,
            ..GatewayConfig::default()
        };
        let server = GatewayServer::new(config, vault_dir);
        assert_eq!(server.config.http_port, 19001);
    }

    #[tokio::test]
    async fn gateway_start_and_shutdown_own_one_durable_worker() {
        let temp_dir = TempDir::new().unwrap();
        let vault_dir = temp_dir.path().to_path_buf();
        let config = GatewayConfig {
            host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            http_port: 0,
            ..GatewayConfig::default()
        };
        let mut server = GatewayServer::new(config, vault_dir);

        server.start().await.unwrap();
        assert!(server
            .durable_work_worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished()));

        let envelope = WorkEnvelope::authorize(
            WorkDraft::new(
                "test.same-store",
                "zbot.local",
                serde_json::json!({"value": "same-store"}),
            ),
            &GatewayWorkPolicy,
            chrono::Utc::now(),
        )
        .unwrap();
        server.state.durable_work_store.enqueue(&envelope).unwrap();
        server
            .state
            .durable_work_transport
            .publish(&envelope)
            .await
            .unwrap();
        let stored = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(item) = server.state.durable_work_store.get(envelope.id()).unwrap() {
                    if item.status() == WorkStatus::DeadLetter {
                        return item;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            stored.last_failure_code(),
            Some(WorkFailureCode::HandlerUnavailable)
        );
        assert!(server.start_durable_work_worker().is_err());

        server.shutdown().await;
        assert!(server.durable_work_worker.is_none());
    }
}
