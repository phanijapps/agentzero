//! Tests for `search_memory_facts` HTTP handler (Task 4 — Memory Tab Command Deck).
//!
//! Verifies hybrid/fts/semantic mode routing and `match_source` annotation
//! without requiring a real embedding backend. Relies on the default
//! `NoopEmbeddingClient` (via `AppState::minimal`) which returns
//! `EmbeddingError::ConfigError` — hybrid mode must degrade gracefully to
//! FTS-only in that case.

mod common;

use axum_test::TestServer;
use common::{make_state, now_iso};
use gateway::{http::create_http_router, websocket::WebSocketHandler, GatewayConfig};
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use zbot_stores_domain::MemoryFact;

/// Build a minimal `TestServer` + seed one memory fact whose content contains
/// the keyword `tickers`.
fn setup_with_seeded_fact(agent_id: &str) -> (TestServer, TempDir) {
    let (dir, state) = make_state();

    let now = now_iso();
    let fact = MemoryFact {
        id: "fact-test-1".to_string(),
        session_id: None,
        agent_id: agent_id.to_string(),
        scope: "agent".to_string(),
        category: "pattern".to_string(),
        key: "test.tickers".to_string(),
        content: "tickers are stock symbols".to_string(),
        confidence: 0.9,
        mention_count: 1,
        source_summary: None,
        embedding: None,
        ward_id: "__global__".to_string(),
        contradicted_by: None,
        created_at: now.clone(),
        updated_at: now,
        expires_at: None,
        valid_from: None,
        valid_until: None,
        superseded_by: None,
        pinned: false,
        epistemic_class: Some("current".to_string()),
        source_episode_id: None,
        source_ref: None,
    };
    futures::executor::block_on(
        state
            .memory_store
            .as_ref()
            .expect("memory_store")
            .upsert_typed_fact(fact.clone(), None),
    )
    .expect("upsert fact");

    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let router = create_http_router(GatewayConfig::default(), state, ws_handler);
    let server = TestServer::new(router).expect("test server");

    (server, dir)
}

#[tokio::test]
async fn hybrid_mode_returns_match_source_field() {
    let (server, _dir) = setup_with_seeded_fact("agent-1");

    let response = server
        .get("/api/memory/agent-1/search")
        .add_query_param("q", "test.tickers")
        .add_query_param("mode", "hybrid")
        .add_query_param("limit", "10")
        .await;

    response.assert_status_ok();
    let body: Value = response.json();
    let facts = body["facts"].as_array().expect("facts is array");
    assert!(!facts.is_empty(), "expected at least one fact, got empty");
    assert!(
        facts[0].get("match_source").is_some(),
        "expected match_source field on first fact, got: {}",
        facts[0]
    );
    assert_eq!(facts[0]["match_source"], "fts");
}

#[tokio::test]
async fn fts_mode_does_not_call_embedding_backend() {
    // AppState::minimal installs a NoopEmbeddingClient (default), whose embed()
    // returns ConfigError. fts mode must succeed regardless.
    let (server, _dir) = setup_with_seeded_fact("agent-2");

    let response = server
        .get("/api/memory/agent-2/search")
        .add_query_param("q", "tickers")
        .add_query_param("mode", "fts")
        .await;

    response.assert_status_ok();
    let body: Value = response.json();
    let facts = body["facts"].as_array().expect("facts is array");
    assert!(
        !facts.is_empty(),
        "fts mode should return facts without touching embedding backend; body={body}"
    );
}
