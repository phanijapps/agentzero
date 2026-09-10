mod common;

use axum::http::StatusCode;
use axum_test::{TestServer, TestServerConfig, Transport};
use common::make_state;
use gateway::{
    http::create_http_router, tasks::a2a::A2aInboundPayload, websocket::WebSocketHandler, AppState,
    GatewayConfig,
};
use gateway_a2a::client::{A2aTransport, HttpA2aTransport};
use gateway_a2a::peers::{AddPeer, IssueCredential, PeerStore};
use gateway_a2a::{outbound_send_message_request, APPLICATION_A2A_JSON};
use serde_json::{json, Value};
use std::sync::Arc;

fn send_body(message_id: &str, text: &str) -> Value {
    json!({
        "message": {
            "messageId": message_id,
            "role": "ROLE_USER",
            "parts": [{ "text": text, "mediaType": "text/plain" }]
        },
        "configuration": {
            "acceptedOutputModes": ["text/plain"],
            "returnImmediately": true
        }
    })
}

fn setup_enabled() -> (TestServer, tempfile::TempDir, String, String, AppState) {
    let (dir, state) = make_state();
    let peers = PeerStore::new(dir.path());
    let peer_a = peers
        .issue_credential(IssueCredential {
            peer_id: "peer-a".into(),
            display_name: Some("Peer A".into()),
            target_agent_id: "assistant".into(),
            lifetime_days: None,
        })
        .unwrap()
        .token
        .exposed()
        .to_string();
    let peer_b = peers
        .issue_credential(IssueCredential {
            peer_id: "peer-b".into(),
            display_name: Some("Peer B".into()),
            target_agent_id: "assistant".into(),
            lifetime_days: None,
        })
        .unwrap()
        .token
        .exposed()
        .to_string();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let router = create_http_router(
        GatewayConfig {
            a2a_enabled: true,
            a2a_public_base_url: Some("https://zbot-a.example.test".into()),
            a2a_allowed_origins: vec!["https://trusted-client.example".into()],
            ..GatewayConfig::default()
        },
        state.clone(),
        ws_handler,
    );
    (TestServer::new(router).unwrap(), dir, peer_a, peer_b, state)
}

#[tokio::test]
async fn a2a_is_default_off_and_agent_card_is_public_when_enabled() {
    let (dir, state) = make_state();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let disabled = TestServer::new(create_http_router(
        GatewayConfig::default(),
        state,
        ws_handler,
    ))
    .unwrap();
    disabled
        .get("/.well-known/agent-card.json")
        .await
        .assert_status_not_found();
    drop(dir);

    let (enabled, _dir, _peer_a, _peer_b, _state) = setup_enabled();
    let card = enabled.get("/.well-known/agent-card.json").await;
    card.assert_status_ok();
    let value: Value = card.json();
    assert_eq!(value["supportedInterfaces"][0]["protocolVersion"], "1.0");
    assert_eq!(value["capabilities"]["streaming"], false);
    assert_eq!(value["securityRequirements"], json!([{ "peerBearer": [] }]));
}

#[tokio::test]
async fn authenticated_send_is_durable_idempotent_scoped_and_cancelable() {
    let (server, _dir, peer_a, peer_b, _state) = setup_enabled();

    let unauthenticated = server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .json(&send_body("msg-no-auth", "must not persist"))
        .content_type(APPLICATION_A2A_JSON)
        .await;
    unauthenticated.assert_status_unauthorized();

    let accepted = server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-1", "summarize this"))
        .content_type(APPLICATION_A2A_JSON)
        .await;
    accepted.assert_status_ok();
    assert_eq!(
        accepted.header("content-type").to_str().unwrap(),
        APPLICATION_A2A_JSON
    );
    let accepted_value: Value = accepted.json();
    let task_id = accepted_value["task"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        accepted_value["task"]["status"]["state"],
        "TASK_STATE_SUBMITTED"
    );

    let duplicate = server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-1", "summarize this"))
        .content_type(APPLICATION_A2A_JSON)
        .await;
    duplicate.assert_status_ok();
    let duplicate_value: Value = duplicate.json();
    assert_eq!(duplicate_value["task"]["id"], task_id);

    server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-1", "different payload"))
        .content_type(APPLICATION_A2A_JSON)
        .await
        .assert_status_bad_request();

    server
        .get(&format!("/a2a/tasks/{task_id}"))
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_b}"))
        .await
        .assert_status_not_found();

    let listed = server
        .get("/a2a/tasks")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    listed.assert_status_ok();
    let listed_value: Value = listed.json();
    assert_eq!(listed_value["tasks"].as_array().unwrap().len(), 1);

    let canceled = server
        .post(&format!("/a2a/tasks/{task_id}:cancel"))
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    canceled.assert_status_ok();
    assert_eq!(
        canceled.json::<Value>()["status"]["state"],
        "TASK_STATE_CANCELED"
    );
}

#[tokio::test]
async fn hardened_http_client_interoperates_with_the_real_a2a_router() {
    let (dir_b, state_b) = make_state();
    let inbound_token = PeerStore::new(dir_b.path())
        .issue_credential(IssueCredential {
            peer_id: "peer-a".into(),
            display_name: Some("Peer A".into()),
            target_agent_id: "assistant".into(),
            lifetime_days: None,
        })
        .unwrap()
        .token
        .exposed()
        .to_owned();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state_b.event_bus.clone(),
        state_b.runtime.clone(),
    ));
    let router = create_http_router(
        GatewayConfig {
            a2a_enabled: true,
            ..GatewayConfig::default()
        },
        state_b,
        ws_handler,
    );
    let server = TestServer::new_with_config(
        router,
        TestServerConfig {
            transport: Some(Transport::HttpRandomPort),
            ..TestServerConfig::default()
        },
    )
    .unwrap();

    let dir_a = tempfile::tempdir().unwrap();
    let peers_a = PeerStore::new(dir_a.path());
    peers_a
        .add_peer(AddPeer {
            peer_id: "peer-b".into(),
            display_name: "Peer B".into(),
            origin: server.server_address().unwrap().to_string(),
            target_agent_id: "assistant".into(),
            outbound_token: Some(inbound_token),
            allow_private_http: false,
        })
        .unwrap();
    let snapshot = peers_a.load_snapshot().unwrap();
    let peer_b = snapshot.get("peer-b").unwrap();
    let client = HttpA2aTransport::default();
    let task = client
        .send_message(
            peer_b,
            &outbound_send_message_request("msg-real-http", "real transport request").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(task.status.state, a2a::TaskState::Submitted);

    let projected = client.get_task(peer_b, &task.id).await.unwrap();
    assert_eq!(projected.id, task.id);
    assert_eq!(projected.status.state, a2a::TaskState::Submitted);
}

#[tokio::test]
async fn browser_origin_is_rejected_before_enqueue() {
    let (server, _dir, peer_a, _peer_b, _state) = setup_enabled();
    server
        .post("/a2a/message:send")
        .add_header("origin", "https://attacker.example")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-origin", "must not persist"))
        .content_type(APPLICATION_A2A_JSON)
        .await
        .assert_status_unauthorized();

    let listed = server
        .get("/a2a/tasks")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    listed.assert_status(StatusCode::OK);
    assert!(listed.json::<Value>()["tasks"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn repeated_authentication_failures_exhaust_the_origin_budget() {
    let (server, _dir, peer_a, _peer_b, _state) = setup_enabled();
    for attempt in 0..10 {
        server
            .post("/a2a/message:send")
            .add_header("A2A-Version", "1.0")
            .add_header("authorization", format!("Bearer invalid-{attempt}"))
            .json(&send_body(
                &format!("msg-invalid-{attempt}"),
                "must not persist",
            ))
            .content_type(APPLICATION_A2A_JSON)
            .await
            .assert_status_unauthorized();
    }

    server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-rate-limited", "must not persist"))
        .content_type(APPLICATION_A2A_JSON)
        .await
        .assert_status_unauthorized();
}

#[tokio::test]
async fn per_peer_outstanding_task_limit_preserves_idempotent_replay() {
    let (server, _dir, peer_a, _peer_b, _state) = setup_enabled();
    let mut first_task_id = None;
    for index in 0..100 {
        let accepted = server
            .post("/a2a/message:send")
            .add_header("A2A-Version", "1.0")
            .add_header("authorization", format!("Bearer {peer_a}"))
            .json(&send_body(&format!("msg-capacity-{index}"), "bounded work"))
            .content_type(APPLICATION_A2A_JSON)
            .await;
        accepted.assert_status_ok();
        if index == 0 {
            first_task_id = accepted.json::<Value>()["task"]["id"]
                .as_str()
                .map(str::to_owned);
        }
    }

    let replay = server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-capacity-0", "bounded work"))
        .content_type(APPLICATION_A2A_JSON)
        .await;
    replay.assert_status_ok();
    assert_eq!(
        replay.json::<Value>()["task"]["id"].as_str(),
        first_task_id.as_deref()
    );

    server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-over-capacity", "must not persist"))
        .content_type(APPLICATION_A2A_JSON)
        .await
        .assert_status_bad_request();
}

#[tokio::test]
async fn malformed_queries_and_optional_operations_use_a2a_errors() {
    let (server, _dir, peer_a, _peer_b, _state) = setup_enabled();

    let malformed = server
        .get("/a2a/tasks?pageSize=not-a-number")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    malformed.assert_status_bad_request();
    assert_eq!(
        malformed.header("content-type").to_str().unwrap(),
        APPLICATION_A2A_JSON
    );
    assert_eq!(
        malformed.json::<Value>()["error"]["status"],
        "INVALID_PARAMS"
    );

    let unknown_query = server
        .get("/a2a/tasks?unexpected=true")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    unknown_query.assert_status_bad_request();

    let unsupported = server
        .post("/a2a/message:stream")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    unsupported.assert_status_bad_request();
    assert_eq!(
        unsupported.json::<Value>()["error"]["details"][0]["reason"],
        "UNSUPPORTED_OPERATION"
    );
}

#[tokio::test]
async fn completed_task_projects_the_canonical_assistant_artifact() {
    let (server, _dir, peer_a, _peer_b, state) = setup_enabled();
    let accepted = server
        .post("/a2a/message:send")
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .json(&send_body("msg-artifact", "produce an answer"))
        .content_type(APPLICATION_A2A_JSON)
        .await;
    accepted.assert_status_ok();
    let accepted_value: Value = accepted.json();
    let task_id = accepted_value["task"]["id"].as_str().unwrap();

    let leased = state
        .durable_work_store
        .claim_next(
            gateway::tasks::durable_agent::AGENT_TASK_TARGET,
            "test-worker",
            chrono::Utc::now(),
            std::time::Duration::from_secs(30),
        )
        .unwrap()
        .unwrap();
    let payload: A2aInboundPayload =
        serde_json::from_value(leased.envelope().payload().clone()).unwrap();
    let session = execution_state::Session::new_with_id(
        &payload.session_id,
        &payload.target_agent_id,
        execution_state::TriggerSource::Web,
    )
    .unwrap();
    state.state_service.create_session_from(&session).unwrap();
    let execution = execution_state::AgentExecution::new_root_with_id(
        &payload.execution_id,
        &payload.session_id,
        &payload.target_agent_id,
    )
    .unwrap();
    state.state_service.create_execution(&execution).unwrap();
    state
        .messages
        .append(&zbot_conversation::Message {
            id: format!("msg-assistant-{}", uuid::Uuid::new_v4()),
            execution_id: Some(payload.execution_id.clone()),
            session_id: payload.session_id.clone(),
            role: "assistant".into(),
            content: "canonical remote answer".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            token_count: 6,
            tool_calls: None,
            tool_call_id: None,
            seq: 0,
        })
        .unwrap();
    state
        .durable_work_store
        .complete(
            leased.envelope().id(),
            "test-worker",
            leased.lease_token().unwrap(),
            chrono::Utc::now(),
        )
        .unwrap();

    let completed = server
        .get(&format!("/a2a/tasks/{task_id}?includeArtifacts=true"))
        .add_header("A2A-Version", "1.0")
        .add_header("authorization", format!("Bearer {peer_a}"))
        .await;
    completed.assert_status_ok();
    let value: Value = completed.json();
    assert_eq!(value["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(
        value["artifacts"][0]["parts"][0]["text"],
        "canonical remote answer"
    );
}
