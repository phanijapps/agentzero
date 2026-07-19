//! # HTTP Module
//!
//! RESTful HTTP API for the gateway.

mod agents;
mod artifacts;
mod autonomy;
mod belief_network;
mod beliefs;
mod bridge;
mod chat;
mod cleanup;
pub(crate) mod commissioning;
mod connectors;
mod conversations;
mod cron;
mod customization;
mod embeddings;
mod events;
mod gateway_bus;
mod graph;
mod health;
mod hierarchy;
mod ingest;
mod mcps;
mod memory;
mod memory_search;
mod models;
mod network;
mod openapi;
mod paths;
mod plugins;
mod providers;
mod sessions;
mod settings;
mod skills;
mod surfaces;
mod tools;
mod traces;
mod upload;
mod vault;
mod ward_actions;
mod ward_content;
mod ward_curator;
mod ward_usage;
mod webhooks;

use crate::config::GatewayConfig;
use crate::state::AppState;
use crate::websocket::{axum_ws_upgrade_handler, WebSocketHandler};
use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
    Extension, Router,
};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::info;

/// Create the HTTP router with all endpoints.
///
/// `ws_handler` is threaded in via an Axum [`Extension`] so the `/ws`
/// WebSocket-upgrade route can reach the same session/subscription
/// state the legacy 18790 listener uses. Running both on one port lets
/// firewalled mobile clients and simple reverse proxies work without
/// an extra hole for the WS protocol.
pub fn create_http_router(
    config: GatewayConfig,
    state: AppState,
    ws_handler: Arc<WebSocketHandler>,
) -> Router {
    let cors = if config.cors_enabled {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::new()
    };

    let mut router = Router::new()
        // OpenAPI documentation
        .route("/api/openapi.yaml", get(openapi::openapi_yaml))
        .route("/api/openapi.json", get(openapi::openapi_json))
        .route("/api/docs", get(openapi::swagger_ui))
        // Health endpoints
        .route("/api/health", get(health::health_check))
        .route("/api/status", get(health::status))
        // Vault path discovery (tells UI where the daemon's data lives)
        .route("/api/paths", get(paths::get_paths))
        // Cleanup operations (bounded to vault-owned directories)
        .route("/api/cleanup/vault-temp", post(cleanup::cleanup_vault_temp))
        // Network info (LAN discoverability snapshot for Settings UI)
        .route("/api/network/info", get(network::get_network_info))
        // Agent endpoints
        .route("/api/agents", get(agents::list_agents))
        .route("/api/agents", post(agents::create_agent))
        .route("/api/agents/:id", get(agents::get_agent))
        .route("/api/agents/:id", put(agents::update_agent))
        .route("/api/agents/:id", delete(agents::delete_agent))
        // Conversation endpoints
        .route("/api/conversations", get(conversations::list_conversations))
        .route(
            "/api/conversations",
            post(conversations::create_conversation),
        )
        .route(
            "/api/conversations/:id",
            get(conversations::get_conversation),
        )
        .route(
            "/api/conversations/:id",
            delete(conversations::delete_conversation),
        )
        .route(
            "/api/conversations/:id/messages",
            get(conversations::list_messages),
        )
        .route(
            "/api/autonomy",
            get(autonomy::list_items).post(autonomy::create_item),
        )
        .route("/api/autonomy/:id", get(autonomy::get_item))
        .route("/api/surfaces/actions", post(surfaces::invoke_action))
        .route(
            "/api/autonomy/:id/transition",
            post(autonomy::transition_item),
        )
        .route("/api/autonomy/:id/resume", post(autonomy::resume_item))
        .route("/api/autonomy/:id/eligibility", get(autonomy::eligibility))
        // Tool endpoints
        .route("/api/tools", get(tools::list_tools))
        .route("/api/tools/:name", get(tools::get_tool))
        // Skill endpoints
        .route("/api/skills", get(skills::list_skills))
        .route("/api/skills", post(skills::create_skill))
        .route("/api/skills/:id", get(skills::get_skill))
        .route("/api/skills/:id", put(skills::update_skill))
        .route("/api/skills/:id", delete(skills::delete_skill))
        // Provider endpoints
        .nest("/api/providers", providers::routes())
        // Durable first-run commissioning
        .route(
            "/api/commissioning/status",
            get(commissioning::get_commissioning_status),
        )
        .route(
            "/api/commissioning/local/diagnose",
            post(commissioning::diagnose_local_runtime),
        )
        .route(
            "/api/commissioning/complete",
            post(commissioning::complete_commissioning),
        )
        // Model registry endpoints
        .route("/api/models", get(models::list_models))
        .route("/api/models/:id", get(models::get_model))
        // MCP endpoints
        .route("/api/mcps", get(mcps::list_mcps))
        .route("/api/mcps", post(mcps::create_mcp))
        .route("/api/mcps/:id", get(mcps::get_mcp))
        .route("/api/mcps/:id", put(mcps::update_mcp))
        .route("/api/mcps/:id", delete(mcps::delete_mcp))
        .route("/api/mcps/:id/test", post(mcps::test_mcp))
        .route("/api/mcps/:id/oauth/status", get(mcps::mcp_oauth_status))
        .route("/api/mcps/:id/oauth/start", post(mcps::start_mcp_oauth))
        .route(
            "/api/mcps/:id/oauth/disconnect",
            post(mcps::disconnect_mcp_oauth),
        )
        .route("/api/mcps/oauth/callback", get(mcps::mcp_oauth_callback))
        // Connector endpoints
        .route("/api/connectors", get(connectors::list_connectors))
        .route("/api/connectors", post(connectors::create_connector))
        .route("/api/connectors/:id", get(connectors::get_connector))
        .route("/api/connectors/:id", put(connectors::update_connector))
        .route("/api/connectors/:id", delete(connectors::delete_connector))
        .route(
            "/api/connectors/:id/metadata",
            get(connectors::get_connector_metadata),
        )
        .route("/api/connectors/:id/test", post(connectors::test_connector))
        .route(
            "/api/connectors/:id/enable",
            post(connectors::enable_connector),
        )
        .route(
            "/api/connectors/:id/disable",
            post(connectors::disable_connector),
        )
        .route("/api/connectors/:id/inbound", post(connectors::inbound))
        .route(
            "/api/connectors/:id/inbound-log",
            get(connectors::get_inbound_log),
        )
        // Cron job endpoints
        .route("/api/cron", get(cron::list_cron_jobs))
        .route("/api/cron", post(cron::create_cron_job))
        .route("/api/cron/:id", get(cron::get_cron_job))
        .route("/api/cron/:id", put(cron::update_cron_job))
        .route("/api/cron/:id", delete(cron::delete_cron_job))
        .route("/api/cron/:id/trigger", post(cron::trigger_cron_job))
        .route("/api/cron/:id/enable", post(cron::enable_cron_job))
        .route("/api/cron/:id/disable", post(cron::disable_cron_job))
        // Webhook endpoints
        .route(
            "/api/webhooks/:hook_type/:hook_id",
            post(webhooks::handle_webhook),
        )
        .route(
            "/api/webhooks/:hook_type/:hook_id/verify",
            get(webhooks::verify_webhook),
        )
        .route(
            "/api/webhooks/whatsapp/:phone_number_id/messages",
            post(webhooks::handle_whatsapp_webhook),
        )
        .route(
            "/api/webhooks/telegram/:bot_id",
            post(webhooks::handle_telegram_webhook),
        )
        // SSE Events endpoints
        .route("/api/events", get(events::all_events_stream))
        .route("/api/events/:conversation_id", get(events::event_stream))
        // Settings endpoints
        .route("/api/settings/tools", get(settings::get_tool_settings))
        .route("/api/settings/tools", put(settings::update_tool_settings))
        .route("/api/settings/logs", get(settings::get_log_settings))
        .route("/api/settings/logs", put(settings::update_log_settings))
        .route(
            "/api/settings/execution",
            get(settings::get_execution_settings),
        )
        .route(
            "/api/settings/execution",
            put(settings::update_execution_settings),
        )
        .route("/api/settings/network", get(settings::get_network_settings))
        .route(
            "/api/settings/network",
            put(settings::update_network_settings),
        )
        // Customization endpoints
        .route("/api/customization/files", get(customization::list_files))
        .route("/api/customization/file", get(customization::get_file))
        .route("/api/customization/file", put(customization::put_file))
        // Embedding backend selection (Phase 1)
        .route("/api/embeddings/health", get(embeddings::get_health))
        .route("/api/embeddings/models", get(embeddings::list_models))
        .route(
            "/api/embeddings/ollama-models",
            get(embeddings::list_ollama_models),
        )
        .route("/api/embeddings/configure", post(embeddings::configure))
        .route("/api/embeddings/reindex", post(embeddings::reindex))
        // Memory endpoints
        .route("/api/memory", get(memory::list_all_memory_facts))
        .route(
            "/api/memory/search",
            get(memory::search_all_memory_facts).post(memory_search::memory_search),
        )
        .route("/api/memory/consolidate", post(memory::consolidate))
        .route("/api/procedures/dedupe", post(memory::dedupe_procedures))
        .route("/api/memory/stats", get(memory::stats))
        .route("/api/memory/health", get(memory::health))
        .route(
            "/api/memory/:agent_id",
            get(memory::list_memory_facts).post(memory::create_memory_fact),
        )
        .route(
            "/api/memory/:agent_id/search",
            get(memory::search_memory_facts),
        )
        .route(
            "/api/memory/:agent_id/facts/:fact_id",
            get(memory::get_memory_fact),
        )
        .route(
            "/api/memory/:agent_id/facts/:fact_id",
            delete(memory::delete_memory_fact),
        )
        // Belief Network endpoints (Phase B-5 — UI surface).
        // Gated by `execution.memory.beliefNetwork.enabled`; handlers
        // return 503 when the feature is off.
        .route("/api/beliefs/:agent_id", get(beliefs::list_beliefs))
        .route(
            "/api/beliefs/:agent_id/:belief_id",
            get(beliefs::get_belief_detail),
        )
        .route(
            "/api/beliefs/:agent_id/:belief_id/contradictions",
            get(beliefs::list_belief_contradictions),
        )
        .route(
            "/api/contradictions/:agent_id",
            get(beliefs::list_recent_contradictions),
        )
        .route(
            "/api/contradictions/:contradiction_id/resolve",
            post(beliefs::resolve_contradiction),
        )
        // Ward curator — per-ward usage telemetry (Phase A.3) +
        // heuristic cleanup endpoint (Phase B). Spec:
        // 2026-05-23-ward-curator-spec.md. Under /api/curator/ rather
        // than /api/wards/curator to avoid colliding with /api/wards/:ward_id.
        .route("/api/curator/usage", get(ward_usage::list_usage))
        .route("/api/curator/usage/:ward", get(ward_usage::get_usage))
        .route("/api/curator/usage/:ward/pin", post(ward_usage::set_pinned))
        .route("/api/curator/cleanup", post(ward_curator::cleanup))
        .route("/api/curator/restore", post(ward_curator::restore))
        .route("/api/curator/consolidate", post(ward_curator::consolidate))
        // Ward listing (Memory Tab Command Deck — Task 9)
        .route("/api/wards", get(ward_content::list_wards))
        // Vault filesystem browser — local-only, read-only ward tree + preview.
        .route("/api/vault/wards", get(vault::list_vault_wards))
        .route("/api/vault/wards/:ward_id/tree", get(vault::get_vault_tree))
        .route(
            "/api/vault/wards/:ward_id/search",
            get(vault::search_vault_files),
        )
        .route("/api/vault/wards/:ward_id/file", get(vault::get_vault_file))
        // Ward content aggregator (Memory Tab Command Deck — Task 5)
        .route(
            "/api/wards/:ward_id/content",
            get(ward_content::get_ward_content),
        )
        // Ward actions — opens folder in native OS file browser (R14c)
        .route(
            "/api/wards/:ward_id/open",
            post(ward_actions::open_ward_folder),
        )
        // Upload endpoint
        .route(
            "/api/upload",
            post(upload::upload_file).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        // Chat session endpoints
        .route("/api/chat/init", post(chat::init_chat_session))
        .route("/api/chat/session", delete(chat::clear_chat_session))
        .route(
            "/api/sessions/:session_id/messages",
            get(chat::get_session_messages),
        )
        // Session archive endpoints
        .route("/api/sessions/archive", post(sessions::archive_sessions))
        .route("/api/sessions/restore/:id", post(sessions::restore_session))
        .route("/api/sessions/:id/state", get(sessions::get_session_state))
        .route("/api/traces/query", post(traces::query_traces))
        // Hard-delete a session with memory-preserving cascade (R18)
        .route("/api/sessions/:id", delete(sessions::delete_session))
        // Artifact endpoints
        .route(
            "/api/sessions/:session_id/artifacts",
            get(artifacts::list_session_artifacts),
        )
        .route(
            "/api/artifacts/:artifact_id/content",
            get(artifacts::serve_artifact_content),
        )
        // Knowledge Graph endpoints (cross-agent observatory routes first)
        .route("/api/graph/stats", get(graph::graph_stats))
        .route("/api/graph/all/entities", get(graph::all_entities))
        .route(
            "/api/graph/all/relationships",
            get(graph::all_relationships),
        )
        .route("/api/graph/:agent_id/stats", get(graph::get_graph_stats))
        .route("/api/graph/:agent_id/entities", get(graph::list_entities))
        .route(
            "/api/graph/:agent_id/relationships",
            get(graph::list_relationships),
        )
        .route("/api/graph/:agent_id/search", get(graph::search_entities))
        .route(
            "/api/graph/:agent_id/entities/:entity_id/neighbors",
            get(graph::get_entity_neighbors),
        )
        .route(
            "/api/graph/:agent_id/entities/:entity_id/subgraph",
            get(graph::get_entity_subgraph),
        )
        .route("/api/graph/reindex", post(graph::reindex_all_wards))
        // Streaming ingestion endpoints
        .route("/api/graph/ingest", post(ingest::ingest))
        .route(
            "/api/graph/ingest/:source_id/progress",
            get(ingest::progress),
        )
        // Belief Network observability (Phase B-6)
        .route("/api/belief-network/stats", get(belief_network::get_stats))
        .route(
            "/api/belief-network/activity",
            get(belief_network::get_activity),
        )
        // Hierarchical-memory observability (Phase H-3/H-4 follow-up)
        .route("/api/hierarchy/stats", get(hierarchy::get_stats))
        // Distillation endpoints
        .route("/api/distillation/status", get(graph::distillation_status))
        .route(
            "/api/distillation/undistilled",
            get(graph::undistilled_sessions),
        )
        .route(
            "/api/distillation/trigger/:session_id",
            post(graph::trigger_distillation),
        )
        // Logs endpoints (from api-logs crate)
        .nest_service("/api/logs", api_logs::routes(state.log_service.clone()))
        // Execution state endpoints (from execution-state crate)
        .nest_service(
            "/api/executions",
            execution_state::routes(state.state_service.clone()),
        )
        // Gateway Bus endpoints (for external connectors and API integrations)
        .nest("/api/gateway", gateway_bus::routes())
        // Bridge endpoints
        .route("/api/bridge/workers", get(bridge::list_workers))
        .route("/bridge/ws", get(bridge::ws_upgrade))
        // Plugin endpoints
        .route("/api/plugins", get(plugins::list_plugins))
        .route("/api/plugins/:id", get(plugins::get_plugin))
        .route("/api/plugins/:id/start", post(plugins::start_plugin))
        .route("/api/plugins/:id/stop", post(plugins::stop_plugin))
        .route("/api/plugins/:id/restart", post(plugins::restart_plugin))
        .route("/api/plugins/:id/config", get(plugins::get_plugin_config))
        .route(
            "/api/plugins/:id/config",
            put(plugins::update_plugin_config),
        )
        .route(
            "/api/plugins/:id/secrets",
            get(plugins::list_plugin_secrets),
        )
        .route(
            "/api/plugins/:id/secrets/:key",
            put(plugins::set_plugin_secret),
        )
        .route(
            "/api/plugins/:id/secrets/:key",
            delete(plugins::delete_plugin_secret),
        )
        .route("/api/plugins/discover", post(plugins::discover_plugins))
        // State
        .with_state(state);

    // Add static file serving for web dashboard
    if config.serve_dashboard {
        if let Some(static_dir) = &config.static_dir {
            let path = PathBuf::from(static_dir);
            if path.exists() {
                info!("Serving dashboard from: {}", static_dir);
                let index_file = path.join("index.html");
                let serve_dir = ServeDir::new(&path).not_found_service(ServeFile::new(&index_file));
                router = router.fallback_service(serve_dir);
            } else {
                tracing::warn!("Static directory not found: {}", static_dir);
            }
        }
    }

    // Unified WebSocket upgrade on the same port as HTTP. Clients connect
    // to `ws://host:<http_port>/ws`. The Extension layer makes the shared
    // session registry / subscription manager / runtime available to the
    // upgrade handler — same state the legacy listener uses.
    router = router.route("/ws", get(axum_ws_upgrade_handler));

    router
        .layer(Extension(config.clone()))
        .layer(Extension(ws_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
