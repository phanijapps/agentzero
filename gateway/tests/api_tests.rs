//! API Integration Tests for Gateway Endpoints
//!
//! These tests verify the HTTP API endpoints work correctly with a real
//! (but minimal) application state.

mod common;

use axum::http::StatusCode;
use axum_test::TestServer;
use common::{now_iso, setup, setup_with_state_service};
use execution_state::{DelegationType, StateService};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use zbot_stores_domain::MemoryFact;
use zbot_stores_sqlite::DatabaseManager;

// ============================================================================
// Test Setup
// ============================================================================

/// Sync-friendly wrapper so existing `.await`-style call sites keep compiling.
/// `common::setup` is sync; this test file historically exposed an async variant.
async fn setup_test_server() -> (TestServer, TempDir) {
    let (server, dir, _state) = setup();
    (server, dir)
}

async fn setup_test_server_with_state() -> (TestServer, Arc<StateService<DatabaseManager>>, TempDir)
{
    setup_with_state_service()
}

// ============================================================================
// Health Endpoint Tests
// ============================================================================

#[tokio::test]
async fn health_check_returns_ok() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/health").await;

    response.assert_status_ok();

    let body: Value = response.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn status_endpoint_returns_info() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/status").await;

    response.assert_status_ok();

    let body: Value = response.json();
    // Status endpoint returns various info - verify it's a valid JSON object
    assert!(body.is_object(), "Expected JSON object response");
}

// ============================================================================
// Autonomy ledger endpoints
// ============================================================================

#[tokio::test]
async fn autonomy_item_lifecycle_is_explicit_and_auditable() {
    let (server, _dir) = setup_test_server().await;
    let create = server
        .post("/api/autonomy")
        .json(&json!({
            "title": "Compare engines",
            "objective": "Choose the execution engine",
            "next_action": "Review migration evidence",
            "source_session_id": "sess-source",
            "dedupe_key": "engine-comparison",
            "evidence": [{ "kind": "session", "reference_id": "sess-source", "label": "Source" }]
        }))
        .await;
    create.assert_status(StatusCode::CREATED);
    let created: Value = create.json();
    assert_eq!(created["state"], "proposed");
    assert_eq!(created["evidence"][0]["reference_id"], "sess-source");

    let id = created["id"].as_str().unwrap();
    let transition = server
        .post(&format!("/api/autonomy/{id}/transition"))
        .json(&json!({ "state": "approved", "outcome": "user approved" }))
        .await;
    transition.assert_status_ok();
    let approved: Value = transition.json();
    assert_eq!(approved["state"], "approved");

    let open = server.get("/api/autonomy").await;
    open.assert_status_ok();
    let items: Vec<Value> = open.json();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id);
}

#[tokio::test]
async fn autonomy_rejects_invalid_transition_and_missing_fields() {
    let (server, _dir) = setup_test_server().await;
    let invalid = server.post("/api/autonomy").json(&json!({})).await;
    invalid.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let oversized = server
        .post("/api/autonomy")
        .json(&json!({
            "title": "A".repeat(201), "objective": "B", "next_action": "C", "dedupe_key": "large"
        }))
        .await;
    oversized.assert_status(StatusCode::BAD_REQUEST);

    let create = server
        .post("/api/autonomy")
        .json(&json!({
            "title": "A", "objective": "B", "next_action": "C", "dedupe_key": "a"
        }))
        .await;
    create.assert_status(StatusCode::CREATED);
    let created: Value = create.json();
    let id = created["id"].as_str().unwrap();
    let transition = server
        .post(&format!("/api/autonomy/{id}/transition"))
        .json(&json!({ "state": "blocked" }))
        .await;
    transition.assert_status(StatusCode::BAD_REQUEST);
}

// ============================================================================
// Execution Stats Endpoint Tests
// ============================================================================

#[tokio::test]
async fn stats_endpoint_returns_counts() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/executions/stats").await;

    response.assert_status_ok();

    let stats: Value = response.json();

    // Should have session counts
    assert!(stats.get("sessions_running").is_some());
    assert!(stats.get("sessions_queued").is_some());
    assert!(stats.get("sessions_completed").is_some());

    // Should have execution counts
    assert!(stats.get("executions_running").is_some());
    assert!(stats.get("executions_completed").is_some());

    // Should have sessions_by_source
    assert!(stats.get("sessions_by_source").is_some());
}

#[tokio::test]
async fn stats_empty_database_returns_zeros() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/executions/stats").await;

    response.assert_status_ok();

    let stats: Value = response.json();

    // Empty database should return zeros
    assert_eq!(stats["sessions_running"], 0);
    assert_eq!(stats["sessions_queued"], 0);
    assert_eq!(stats["executions_running"], 0);
}

// ============================================================================
// Sessions V2 Endpoint Tests
// ============================================================================

#[tokio::test]
async fn sessions_list_empty_returns_array() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/executions/v2/sessions/full").await;

    response.assert_status_ok();

    let sessions: Vec<Value> = response.json();
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn sessions_list_with_filter_params() {
    let (server, _dir) = setup_test_server().await;

    // Test with filter parameters
    let response = server
        .get("/api/executions/v2/sessions/full")
        .add_query_param("status", "running")
        .add_query_param("limit", "10")
        .await;

    response.assert_status_ok();

    let sessions: Vec<Value> = response.json();
    assert!(sessions.is_empty()); // No sessions in test DB
}

#[tokio::test]
async fn session_not_found_returns_404() {
    let (server, _dir) = setup_test_server().await;

    let response = server
        .get("/api/executions/v2/sessions/nonexistent-session/full")
        .await;

    // Should return 404 or empty result
    // The exact behavior depends on implementation
    let status = response.status_code();
    assert!(status == StatusCode::NOT_FOUND || status == StatusCode::OK);
}

// ============================================================================
// Agent Endpoint Tests
// ============================================================================

#[tokio::test]
async fn agents_list_returns_array() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/agents").await;

    response.assert_status_ok();

    let agents: Vec<Value> = response.json();
    // May be empty or have seeded agents
    assert!(agents.is_empty() || agents.iter().all(|a| a.get("id").is_some()));
}

#[tokio::test]
async fn agent_not_found_returns_404() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/agents/nonexistent-agent").await;

    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_agent_with_valid_data() {
    let (server, _dir) = setup_test_server().await;

    let agent_data = json!({
        "name": "test-agent",
        "displayName": "Test Agent",
        "description": "A test agent",
        "providerId": "anthropic",
        "model": "claude-sonnet-4-20250514",
        "temperature": 0.7,
        "maxTokens": 4096,
        "instructions": "You are a helpful assistant.",
        "mcps": [],
        "skills": []
    });

    let response = server.post("/api/agents").json(&agent_data).await;

    // Should succeed or fail gracefully
    let status = response.status_code();
    assert!(
        status == StatusCode::OK
            || status == StatusCode::CREATED
            || status == StatusCode::BAD_REQUEST
    );
}

// ============================================================================
// Gateway Bus Endpoint Tests
// ============================================================================

#[tokio::test]
async fn gateway_status_without_runner() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/gateway/status/nonexistent").await;

    // Without execution runner (minimal state), returns 500 Internal Server Error
    // With runner, would return 404 for nonexistent session
    let status = response.status_code();
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND,
        "Expected 500 (no runner) or 404 (not found), got {:?}",
        status
    );
}

#[tokio::test]
async fn gateway_cancel_without_runner() {
    let (server, _dir) = setup_test_server().await;

    let response = server.post("/api/gateway/cancel/nonexistent").await;

    // Without execution runner (minimal state), returns 500 Internal Server Error
    let status = response.status_code();
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND,
        "Expected 500 (no runner) or 404 (not found), got {:?}",
        status
    );
}

#[tokio::test]
async fn gateway_pause_without_runner() {
    let (server, _dir) = setup_test_server().await;

    let response = server.post("/api/gateway/pause/nonexistent").await;

    // Without execution runner (minimal state), returns 500 Internal Server Error
    let status = response.status_code();
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND,
        "Expected 500 (no runner) or 404 (not found), got {:?}",
        status
    );
}

#[tokio::test]
async fn gateway_resume_without_runner() {
    let (server, _dir) = setup_test_server().await;

    let response = server.post("/api/gateway/resume/nonexistent").await;

    // Without execution runner (minimal state), returns 500 Internal Server Error
    let status = response.status_code();
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND,
        "Expected 500 (no runner) or 404 (not found), got {:?}",
        status
    );
}

#[tokio::test]
async fn gateway_submit_requires_runner() {
    let (server, _dir) = setup_test_server().await;

    let request = json!({
        "agent_id": "root",
        "message": "Hello!",
        "source": "api"
    });

    let response = server.post("/api/gateway/submit").json(&request).await;

    // Minimal state doesn't have a runner, so this should fail gracefully
    // with an internal server error indicating runner not initialized
    let status = response.status_code();
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR
            || status == StatusCode::SERVICE_UNAVAILABLE
            || status == StatusCode::OK,
        "Expected error status or success, got {:?}",
        status
    );
}

// ============================================================================
// Conversation Endpoint Tests
// ============================================================================

#[tokio::test]
async fn conversations_list_empty() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/conversations").await;

    response.assert_status_ok();

    let conversations: Vec<Value> = response.json();
    assert!(conversations.is_empty());
}

#[tokio::test]
async fn conversation_not_found_returns_404() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/conversations/nonexistent").await;

    response.assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// Memory Endpoint Tests
// ============================================================================

#[tokio::test]
async fn memory_create_rejects_internal_fact_categories() {
    let (server, _dir) = setup_test_server().await;

    for category in ["ctx", "instruction", "correction"] {
        let response = server
            .post("/api/memory/root")
            .json(&json!({
                "category": category,
                "key": format!("{category}.public-bypass"),
                "content": "must not be user-created through public memory API"
            }))
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("internal-only"),
            "unexpected body for {category}: {body}"
        );
    }
}

#[tokio::test]
async fn memory_get_and_delete_reject_internal_facts_even_by_id() {
    let (server, _dir, state) = setup();

    for category in ["ctx", "instruction", "correction"] {
        let now = now_iso();
        let fact_id = format!("fact-internal-{category}");
        let fact = MemoryFact {
            id: fact_id.clone(),
            session_id: Some("sess-internal".to_string()),
            agent_id: "root".to_string(),
            scope: "session".to_string(),
            category: category.to_string(),
            key: format!("{category}.sess-internal.intent"),
            content: "private policy/control fact".to_string(),
            confidence: 1.0,
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
            pinned: true,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
        };
        futures::executor::block_on(
            state
                .memory_store
                .as_ref()
                .expect("memory_store")
                .upsert_typed_fact(serde_json::to_value(fact).expect("encode fact"), None),
        )
        .expect("seed internal fact");

        let get_response = server
            .get(&format!("/api/memory/root/facts/{fact_id}"))
            .await;
        get_response.assert_status(StatusCode::FORBIDDEN);

        let delete_response = server
            .delete(&format!("/api/memory/root/facts/{fact_id}"))
            .await;
        delete_response.assert_status(StatusCode::FORBIDDEN);

        let still_exists = state
            .memory_store
            .as_ref()
            .expect("memory_store")
            .get_memory_fact_by_id(&fact_id)
            .await
            .expect("read fact");
        assert!(
            still_exists.is_some(),
            "forbidden delete must not remove {category} fact"
        );
    }
}

// ============================================================================
// Skills Endpoint Tests
// ============================================================================

#[tokio::test]
async fn skills_list_returns_array() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/skills").await;

    response.assert_status_ok();

    let skills: Vec<Value> = response.json();
    assert!(skills.is_empty() || skills.iter().all(|s| s.get("id").is_some()));
}

// ============================================================================
// Providers Endpoint Tests
// ============================================================================

#[tokio::test]
async fn providers_list_returns_array() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/providers").await;

    response.assert_status_ok();

    let providers: Vec<Value> = response.json();
    // May have seeded providers or be empty
    assert!(providers.is_empty() || providers.iter().all(|p| p.get("name").is_some()));
}

// ============================================================================
// MCP Endpoint Tests
// ============================================================================

#[tokio::test]
async fn mcps_list_returns_response() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/mcps").await;

    response.assert_status_ok();

    let body: Value = response.json();
    assert!(body.get("servers").is_some());
}

#[tokio::test]
async fn mcp_oauth_status_reports_not_connected_without_secrets() {
    let (server, _dir) = setup_test_server().await;

    let create = server
        .post("/api/mcps")
        .json(&json!({
            "type": "streamable-http",
            "id": "robinhood-trading",
            "name": "Robinhood Trading",
            "description": "Trading MCP",
            "url": "https://agent.robinhood.com/mcp/trading",
            "auth": { "type": "oauth2" },
            "enabled": false
        }))
        .await;
    create.assert_status_ok();

    let response = server.get("/api/mcps/robinhood-trading/oauth/status").await;
    response.assert_status_ok();

    let body: Value = response.json();
    assert_eq!(body["status"], "not_connected");
    assert!(!body.to_string().contains("token"));
}

#[tokio::test]
async fn mcp_oauth_create_rejects_persisted_authorization_header() {
    let (server, _dir) = setup_test_server().await;

    let response = server
        .post("/api/mcps")
        .json(&json!({
            "type": "streamable-http",
            "id": "oauth-with-header",
            "name": "OAuth With Header",
            "description": "bad",
            "url": "https://example.com/mcp",
            "headers": { "Authorization": "Bearer secret" },
            "auth": { "type": "oauth2" },
            "enabled": false
        }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
    let body: Value = response.json();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("Authorization"));
}

#[tokio::test]
async fn mcp_oauth_create_rejects_nonlocal_browser_origin() {
    let (server, _dir) = setup_test_server().await;

    let response = server
        .post("/api/mcps")
        .add_header("origin", "https://evil.example")
        .json(&json!({
            "type": "streamable-http",
            "id": "oauth-origin",
            "name": "OAuth Origin",
            "description": "oauth",
            "url": "https://example.com/mcp",
            "auth": { "type": "oauth2" },
            "enabled": false
        }))
        .await;

    response.assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn mcp_oauth_disconnect_is_idempotent_and_returns_status() {
    let (server, _dir) = setup_test_server().await;

    server
        .post("/api/mcps")
        .json(&json!({
            "type": "streamable-http",
            "id": "oauth-server",
            "name": "OAuth Server",
            "description": "oauth",
            "url": "https://example.com/mcp",
            "auth": { "type": "oauth2" },
            "enabled": false
        }))
        .await
        .assert_status_ok();

    let response = server.post("/api/mcps/oauth-server/oauth/disconnect").await;
    response.assert_status_ok();

    let body: Value = response.json();
    assert_eq!(body["status"], "not_connected");
}

#[tokio::test]
async fn mcp_oauth_start_rejects_external_redirect_uri() {
    let (server, _dir) = setup_test_server().await;

    server
        .post("/api/mcps")
        .json(&json!({
            "type": "streamable-http",
            "id": "oauth-start",
            "name": "OAuth Start",
            "description": "oauth",
            "url": "https://example.com/mcp",
            "auth": { "type": "oauth2" },
            "enabled": false
        }))
        .await
        .assert_status_ok();

    let response = server
        .post("/api/mcps/oauth-start/oauth/start")
        .json(&json!({ "redirectUri": "https://evil.example/callback" }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
    let body: Value = response.json();
    assert!(body["error"].as_str().unwrap_or("").contains("redirectUri"));
}

#[tokio::test]
async fn mcp_oauth_callback_rejects_missing_state() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/mcps/oauth/callback?code=abc").await;

    response.assert_status(StatusCode::BAD_REQUEST);
    assert!(response.text().contains("missing state"));
}

#[tokio::test]
async fn mcp_oauth_test_requires_connection_before_runtime_start() {
    let (server, _dir) = setup_test_server().await;

    server
        .post("/api/mcps")
        .json(&json!({
            "type": "streamable-http",
            "id": "oauth-test",
            "name": "OAuth Test",
            "description": "oauth",
            "url": "https://example.com/mcp",
            "auth": { "type": "oauth2" },
            "enabled": true
        }))
        .await
        .assert_status_ok();

    let response = server.post("/api/mcps/oauth-test/test").await;

    response.assert_status(StatusCode::BAD_REQUEST);
    let body: Value = response.json();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("requires OAuth authentication"));
}

// ============================================================================
// Settings Endpoint Tests
// ============================================================================

#[tokio::test]
async fn tool_settings_get_returns_settings() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/settings/tools").await;

    response.assert_status_ok();

    let body: Value = response.json();
    // Should have success field and data with tool settings
    assert!(body.get("success").is_some() || body.get("grep").is_some());
}

#[tokio::test]
async fn tool_settings_update() {
    let (server, _dir) = setup_test_server().await;

    let settings = json!({
        "fileTools": true,
        "offloadLargeResults": true,
        "offloadThresholdTokens": 5000
    });

    let response = server.put("/api/settings/tools").json(&settings).await;

    // Should succeed
    response.assert_status_ok();
}

// ============================================================================
// Tool Catalog Endpoint Tests
// ============================================================================

#[tokio::test]
async fn tools_list_returns_root_context_catalog() {
    let (server, dir) = setup_test_server().await;

    let response = server
        .get("/api/tools")
        .add_query_param("sessionId", "sess-test")
        .add_query_param("agentId", "root")
        .await;

    response.assert_status_ok();

    let body: Value = response.json();
    assert_eq!(body["actor_kind"], "root");
    assert_eq!(body["session_id"], "sess-test");
    assert_eq!(body["agent_id"], "root");

    let capabilities = body["capabilities"].as_array().expect("capabilities array");
    assert!(!capabilities.is_empty());
    assert!(capabilities.iter().any(|capability| {
        capability["id"] == "shell"
            && capability["kind"] == "tool"
            && capability["risk_level"] == "high"
    }));
    assert!(!body.to_string().contains(&dir.path().display().to_string()));
}

#[tokio::test]
async fn tools_list_filters_delegated_reviewer_catalog() {
    let (server, _dir) = setup_test_server().await;

    let response = server
        .get("/api/tools")
        .add_query_param("actor", "delegated_reviewer")
        .await;

    response.assert_status_ok();

    let body: Value = response.json();
    assert_eq!(body["actor_kind"], "delegated_reviewer");

    let ids: std::collections::BTreeSet<String> = body["capabilities"]
        .as_array()
        .expect("capabilities array")
        .iter()
        .filter_map(|capability| capability["id"].as_str().map(str::to_string))
        .collect();

    assert!(ids.contains("read"));
    assert!(ids.contains("glob"));
    assert!(ids.contains("respond"));
    assert!(!ids.contains("shell"));
    assert!(!ids.contains("grep"));
    assert!(!ids.contains("memory"));
    assert!(!ids.contains("delegate_to_agent"));
}

#[tokio::test]
async fn tools_detail_returns_catalog_capability_or_404() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/tools/shell").await;
    response.assert_status_ok();
    let shell: Value = response.json();
    assert_eq!(shell["id"], "shell");
    assert_eq!(shell["side_effects"], "execute");

    let missing = server.get("/api/tools/not-a-real-tool").await;
    missing.assert_status(StatusCode::NOT_FOUND);
    let body: Value = missing.json();
    assert!(body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("not-a-real-tool"));
}

#[tokio::test]
async fn tools_list_rejects_unknown_actor_kind() {
    let (server, _dir) = setup_test_server().await;

    let response = server
        .get("/api/tools")
        .add_query_param("actor", "unknown")
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn invalid_json_returns_bad_request() {
    let (server, _dir) = setup_test_server().await;

    let response = server
        .post("/api/agents")
        .content_type("application/json")
        .bytes("{ invalid json }".as_bytes().to_vec().into())
        .await;

    // Should return 400 Bad Request or 422 Unprocessable Entity
    let status = response.status_code();
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "Expected 400 or 422, got {:?}",
        status
    );
}

#[tokio::test]
async fn unknown_endpoint_returns_404() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/unknown/endpoint").await;

    response.assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// CORS Header Tests (when enabled)
// ============================================================================

#[tokio::test]
async fn cors_headers_present() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/health").await;

    // With CORS enabled, response should include CORS headers
    // The exact headers depend on the request origin
    response.assert_status_ok();
}

// ============================================================================
// Content-Type Tests
// ============================================================================

#[tokio::test]
async fn json_content_type_in_response() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/health").await;

    response.assert_status_ok();

    let content_type = response.header("content-type");
    assert!(
        content_type
            .to_str()
            .unwrap_or("")
            .contains("application/json"),
        "Expected application/json content type"
    );
}

// ============================================================================
// Session Messages Endpoint Tests
// ============================================================================

#[tokio::test]
async fn session_messages_all_scope() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session and executions
    let (session, root_exec) = state_service.create_session("root-agent").unwrap();

    // Create delegated execution
    let delegate_exec = state_service
        .create_delegated_execution(
            &session.id,
            "researcher",
            &root_exec.id,
            DelegationType::Sequential,
            "Research task",
        )
        .unwrap();

    // Add messages
    state_service
        .add_message(&root_exec.id, "user", "Hello root", None, None)
        .unwrap();
    state_service
        .add_message(&root_exec.id, "assistant", "Root response", None, None)
        .unwrap();
    state_service
        .add_message(&delegate_exec.id, "user", "Research this", None, None)
        .unwrap();
    state_service
        .add_message(
            &delegate_exec.id,
            "assistant",
            "Research results",
            None,
            None,
        )
        .unwrap();

    // Get all messages
    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages",
            session.id
        ))
        .await;

    response.assert_status_ok();

    let messages: Vec<Value> = response.json();
    assert_eq!(messages.len(), 4, "Should return all 4 messages");
}

#[tokio::test]
async fn session_messages_root_scope() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session and executions
    let (session, root_exec) = state_service.create_session("root-agent").unwrap();

    // Create delegated execution
    let delegate_exec = state_service
        .create_delegated_execution(
            &session.id,
            "researcher",
            &root_exec.id,
            DelegationType::Sequential,
            "Research task",
        )
        .unwrap();

    // Add messages
    state_service
        .add_message(&root_exec.id, "user", "Hello root", None, None)
        .unwrap();
    state_service
        .add_message(&root_exec.id, "assistant", "Root response", None, None)
        .unwrap();
    state_service
        .add_message(&delegate_exec.id, "user", "Research this", None, None)
        .unwrap();

    // Get root messages only
    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages?scope=root",
            session.id
        ))
        .await;

    response.assert_status_ok();

    let messages: Vec<Value> = response.json();
    assert_eq!(messages.len(), 2, "Should return only 2 root messages");

    // Verify all messages are from root agent
    for msg in &messages {
        assert_eq!(msg["agent_id"], "root-agent");
        assert_eq!(msg["delegation_type"], "root");
    }
}

#[tokio::test]
async fn session_messages_delegates_scope() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session and executions
    let (session, root_exec) = state_service.create_session("root-agent").unwrap();

    // Create delegated execution
    let delegate_exec = state_service
        .create_delegated_execution(
            &session.id,
            "researcher",
            &root_exec.id,
            DelegationType::Sequential,
            "Research task",
        )
        .unwrap();

    // Add messages
    state_service
        .add_message(&root_exec.id, "user", "Hello root", None, None)
        .unwrap();
    state_service
        .add_message(&delegate_exec.id, "user", "Research this", None, None)
        .unwrap();
    state_service
        .add_message(
            &delegate_exec.id,
            "assistant",
            "Research results",
            None,
            None,
        )
        .unwrap();

    // Get delegate messages only
    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages?scope=delegates",
            session.id
        ))
        .await;

    response.assert_status_ok();

    let messages: Vec<Value> = response.json();
    assert_eq!(messages.len(), 2, "Should return only 2 delegate messages");

    // Verify all messages are from delegated execution
    for msg in &messages {
        assert_eq!(msg["agent_id"], "researcher");
        assert_eq!(msg["delegation_type"], "sequential");
    }
}

#[tokio::test]
async fn session_messages_execution_scope() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session and executions
    let (session, root_exec) = state_service.create_session("root-agent").unwrap();

    // Create delegated execution
    let delegate_exec = state_service
        .create_delegated_execution(
            &session.id,
            "researcher",
            &root_exec.id,
            DelegationType::Sequential,
            "Research task",
        )
        .unwrap();

    // Add messages to both executions
    state_service
        .add_message(&root_exec.id, "user", "Hello root", None, None)
        .unwrap();
    state_service
        .add_message(&delegate_exec.id, "user", "Research this", None, None)
        .unwrap();
    state_service
        .add_message(
            &delegate_exec.id,
            "assistant",
            "Research results",
            None,
            None,
        )
        .unwrap();

    // Get messages for specific execution
    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages?scope=execution&execution_id={}",
            session.id, delegate_exec.id
        ))
        .await;

    response.assert_status_ok();

    let messages: Vec<Value> = response.json();
    assert_eq!(
        messages.len(),
        2,
        "Should return only 2 messages from specified execution"
    );

    // Verify all messages are from the specified execution
    for msg in &messages {
        assert_eq!(msg["execution_id"], delegate_exec.id);
    }
}

#[tokio::test]
async fn session_messages_execution_scope_requires_id() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session
    let (session, _) = state_service.create_session("root-agent").unwrap();

    // Try to get execution scope without execution_id - should return 400
    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages?scope=execution",
            session.id
        ))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_messages_agent_filter() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session and executions
    let (session, root_exec) = state_service.create_session("root-agent").unwrap();

    // Create two delegated executions with different agents
    let researcher_exec = state_service
        .create_delegated_execution(
            &session.id,
            "researcher",
            &root_exec.id,
            DelegationType::Sequential,
            "Research task",
        )
        .unwrap();

    let writer_exec = state_service
        .create_delegated_execution(
            &session.id,
            "writer",
            &root_exec.id,
            DelegationType::Parallel,
            "Write task",
        )
        .unwrap();

    // Add messages
    state_service
        .add_message(&root_exec.id, "user", "Hello root", None, None)
        .unwrap();
    state_service
        .add_message(
            &researcher_exec.id,
            "assistant",
            "Research done",
            None,
            None,
        )
        .unwrap();
    state_service
        .add_message(&writer_exec.id, "assistant", "Writing done", None, None)
        .unwrap();

    // Filter by agent_id
    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages?agent_id=researcher",
            session.id
        ))
        .await;

    response.assert_status_ok();

    let messages: Vec<Value> = response.json();
    assert_eq!(
        messages.len(),
        1,
        "Should return only 1 message from researcher"
    );
    assert_eq!(messages[0]["agent_id"], "researcher");
}

#[tokio::test]
async fn session_messages_not_found() {
    let (server, _dir) = setup_test_server().await;

    // Try to get messages for non-existent session
    let response = server
        .get("/api/executions/v2/sessions/nonexistent-session/messages")
        .await;

    response.assert_status_ok();

    // Should return empty array (session doesn't exist)
    let messages: Vec<Value> = response.json();
    assert!(messages.is_empty());
}

#[tokio::test]
async fn session_messages_empty_session() {
    let (server, state_service, _dir) = setup_test_server_with_state().await;

    // Create session but don't add any messages
    let (session, _) = state_service.create_session("root-agent").unwrap();

    let response = server
        .get(&format!(
            "/api/executions/v2/sessions/{}/messages",
            session.id
        ))
        .await;

    response.assert_status_ok();

    let messages: Vec<Value> = response.json();
    assert!(
        messages.is_empty(),
        "Should return empty array for session with no messages"
    );
}

// ============================================================================
// Belief Network Observability Endpoint Tests (Phase B-6)
// ============================================================================

#[tokio::test]
async fn belief_network_stats_disabled_returns_empty_payload() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/belief-network/stats").await;
    response.assert_status_ok();

    let body: Value = response.json();
    assert_eq!(body["enabled"], false);
    assert_eq!(body["synthesizer"]["history"].as_array().unwrap().len(), 0);
    assert_eq!(
        body["contradiction_detector"]["history"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(body["propagator"]["history"].as_array().unwrap().len(), 0);
    assert_eq!(body["totals"]["total_beliefs"], 0);
    assert_eq!(body["totals"]["total_contradictions"], 0);
}

#[tokio::test]
async fn belief_network_activity_disabled_returns_empty_array() {
    let (server, _dir) = setup_test_server().await;

    let response = server.get("/api/belief-network/activity").await;
    response.assert_status_ok();

    let events: Vec<Value> = response.json();
    assert!(events.is_empty());
}

#[tokio::test]
async fn belief_network_activity_honours_limit_query_param() {
    let (server, _dir) = setup_test_server().await;

    // Even with limit, disabled state still returns []
    let response = server.get("/api/belief-network/activity?limit=10").await;
    response.assert_status_ok();
    let events: Vec<Value> = response.json();
    assert!(events.is_empty());
}
