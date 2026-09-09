//! Shared integration-test fixtures.
//!
//! Rust's integration-test convention: files under `tests/common/` are NOT
//! compiled as standalone test binaries. Sibling test files import them
//! with `mod common;` — this collapses the setup boilerplate that
//! `api_tests.rs`, `memory_search_handler.rs`, `memory_unified_search.rs`,
//! and `ward_content_endpoint.rs` otherwise duplicate verbatim.
//!
//! Scope: keep this small. Anything truly test-specific (seed payloads,
//! per-file constants) stays in the test file.

// Integration-test helpers end up with a long unused tail for any given test
// file — Rust warns on each. Silencing module-wide keeps each call site clean.
#![allow(dead_code)]

use std::sync::Arc;

use axum_test::TestServer;
use execution_state::StateService;
use gateway::{http::create_http_router, websocket::WebSocketHandler, AppState, GatewayConfig};
use tempfile::TempDir;
use zbot_runtime_sqlite::DatabaseManager;
use zbot_stores_domain::{Procedure, SessionEpisode, WikiArticle};

/// Current time as RFC3339 string — the form persisted alongside seeded rows.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Minimal `AppState` rooted at a fresh temp dir, with `agents/` and `skills/`
/// subdirs pre-created (the gateway router touches both on startup).
pub fn make_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("agents")).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    let state = AppState::minimal(dir.path().to_path_buf());
    (dir, state)
}

/// Spin up the full HTTP router against minimal state. Most handler tests
/// want this triplet so they can seed via `state` and call via `server`.
pub fn setup() -> (TestServer, TempDir, AppState) {
    let (dir, state) = make_state();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let router = create_http_router(GatewayConfig::default(), state.clone(), ws_handler);
    let server = TestServer::new(router).expect("test server");
    (server, dir, state)
}

/// Variant for tests that need the `StateService` handle directly (e.g., to
/// insert execution rows before calling the API).
pub fn setup_with_state_service() -> (TestServer, Arc<StateService<DatabaseManager>>, TempDir) {
    let (dir, state) = make_state();
    let state_service = state.state_service.clone();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let router = create_http_router(GatewayConfig::default(), state, ws_handler);
    let server = TestServer::new(router).expect("test server");
    (server, state_service, dir)
}

// ---------------------------------------------------------------------------
// Semantic-store seed helpers.
// ---------------------------------------------------------------------------

pub fn upsert_wiki_article(state: &AppState, article: &WikiArticle) {
    futures::executor::block_on(
        state
            .wiki_store
            .as_ref()
            .expect("wiki_store")
            .upsert_article(article.clone(), None),
    )
    .expect("upsert wiki");
}

pub fn upsert_procedure(state: &AppState, procedure: &Procedure) {
    futures::executor::block_on(
        state
            .procedure_store
            .as_ref()
            .expect("procedure_store")
            .upsert_procedure(procedure.clone(), None),
    )
    .expect("upsert procedure");
}

pub fn insert_episode(state: &AppState, episode: &SessionEpisode) {
    futures::executor::block_on(
        state
            .episode_store
            .as_ref()
            .expect("episode_store")
            .insert_episode(episode.clone(), None),
    )
    .expect("insert episode");
}
