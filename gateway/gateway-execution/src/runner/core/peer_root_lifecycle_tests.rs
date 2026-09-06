
use super::super::continuation_execution::{invoke_continuation, ContinuationArgs};
use super::*;
use agent_runtime::SteerResult;
use execution_state::{DelegationType, Session, SqliteWorkStore, WorkStore};
use gateway_bus::LocalWorkTransport;
use gateway_services::{agents::Agent, providers::Provider, VaultPaths};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

struct Harness {
    _temp: tempfile::TempDir,
    runner: ExecutionRunner,
    state: Arc<StateService<DatabaseManager>>,
    steering: Arc<agent_runtime::SteeringRegistry>,
    paths: SharedVaultPaths,
}

async fn read_request(stream: &mut TcpStream) {
    read_request_body(stream).await;
}

/// Read one HTTP request and return its body (for history assertions).
async fn read_request_body(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected = None;
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0, "client closed before sending the request");
        bytes.extend_from_slice(&buffer[..read]);
        if expected.is_none() {
            if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or_default();
                expected = Some(header_end + 4 + content_length);
            }
        }
        if let Some(length) = expected {
            if bytes.len() >= length {
                return String::from_utf8_lossy(&bytes[expected_header_end(&bytes)..]).to_string();
            }
        }
    }
}

fn expected_header_end(bytes: &[u8]) -> usize {
    bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .map(|end| end + 4)
        .unwrap_or(0)
}

async fn write_sse(stream: &mut TcpStream, body: &str) {
    let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

async fn spawn_two_turn_llm() -> (String, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (seen_tx, seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        read_request(&mut first).await;
        let _ = seen_tx.send(());
        let _ = release_rx.await;
        let tool = concat!(
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-test\",\"function\":{\"name\":\"missing_test_tool\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: [DONE]\n\n"
            );
        write_sse(&mut first, tool).await;

        let (mut second, _) = listener.accept().await.unwrap();
        read_request(&mut second).await;
        let done = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            );
        write_sse(&mut second, done).await;
    });
    (format!("http://{address}/v1"), seen_rx, release_tx)
}

async fn build_harness(base_url: String) -> Harness {
    let temp = tempfile::tempdir().unwrap();
    let paths: SharedVaultPaths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    let db = Arc::new(DatabaseManager::new(paths.clone()).unwrap());
    let state = Arc::new(StateService::new(db.clone()));
    let provider_service = Arc::new(ProviderService::new(paths.clone()));
    provider_service
        .create(Provider {
            id: Some("provider-peer-test".to_owned()),
            name: "Peer Test".to_owned(),
            description: "local test provider".to_owned(),
            api_key: "test-key".to_owned(),
            base_url,
            models: vec!["test-model".to_owned()],
            embedding_models: None,
            embedding_dimensions: None,
            verified: Some(true),
            is_default: true,
            created_at: None,
            max_concurrent_requests: None,
            context_window: Some(8_192),
            default_model: Some("test-model".to_owned()),
            rate_limits: None,
            model_configs: None,
        })
        .unwrap();
    let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap();
    let messages: Arc<dyn zbot_conversation::MessageStore> =
        Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
    let session_meta: Arc<dyn zbot_conversation::SessionMetaStore> =
        Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool.clone()));
    let checkpoints: Arc<dyn zbot_conversation::CheckpointStore> =
        Arc::new(zbot_conversation::SqliteCheckpointStore::new(pool));
    let work_store: Arc<dyn WorkStore> = Arc::new(SqliteWorkStore::new(db.clone()));
    let peer_messages = Arc::new(crate::peer_messaging::DurablePeerMessageService::new(
        work_store,
        Arc::new(LocalWorkTransport::new()),
        state.clone(),
        crate::peer_messaging::PEER_MESSAGE_TARGET,
    ));
    let agent_service = Arc::new(AgentService::new(paths.agents_dir()));
    agent_service
        .create(Agent {
            id: "resume-test-agent".to_owned(),
            name: "resume-test-agent".to_owned(),
            display_name: "Resume Test Agent".to_owned(),
            description: "test resumed delegation".to_owned(),
            agent_type: Some("specialist".to_owned()),
            provider_id: "provider-peer-test".to_owned(),
            model: "test-model".to_owned(),
            temperature: 0.0,
            max_input_tokens: 8_192,
            max_input_tokens_explicit: true,
            max_tokens: 256,
            thinking_enabled: false,
            voice_recording_enabled: false,
            system_instruction: None,
            instructions: "wait for work".to_owned(),
            mcps: vec![],
            skills: vec![],
            middleware: None,
            created_at: None,
        })
        .await
        .unwrap();
    let runner = ExecutionRunner::with_config(ExecutionRunnerConfig {
        event_bus: Arc::new(EventBus::new()),
        agent_service,
        provider_service,
        paths: paths.clone(),
        mcp_service: Arc::new(McpService::new(paths.clone())),
        skill_service: Arc::new(gateway_services::SkillService::new(paths.skills_dir())),
        log_service: Arc::new(LogService::new(db)),
        state_service: state.clone(),
        ward_usage: Arc::new(gateway_services::WardUsage::new(paths.wards_dir())),
        messages,
        session_meta,
        checkpoints,
        connector_registry: None,
        memory_store: None,
        distiller: None,
        handoff_writer: None,
        memory_recall: None,
        peer_messages: Some(peer_messages),
        a2a_delegation: None,
        bridge_registry: None,
        bridge_outbox: None,
        embedding_client: None,
        procedure_store: None,
        procedure_recommendation_cfg: gateway_memory::ProcedureRecommendationConfig::default(),
        max_parallel_agents: 1,
    });
    Harness {
        steering: runner.steering_registry.clone(),
        _temp: temp,
        runner,
        state,
        paths,
    }
}

#[tokio::test]
async fn captured_invokers_observe_stores_installed_after_construction() {
    use zbot_engram_adapter::{AdapterConfig, EngramKnowledgeGraphStore};
    use zbot_stores_sqlite::{
        GatewayGoalStore, GatewayKgEpisodeStore, GoalRepository, KgEpisodeRepository,
        KnowledgeDatabase,
    };
    let mut harness = build_harness("http://unused".into()).await;
    let continuation = harness.runner.make_continuation_invoker();
    let delegation = harness.runner.make_delegation_invoker();
    let graph: Arc<dyn zbot_stores::KnowledgeGraphStore> = Arc::new(
        EngramKnowledgeGraphStore::open(AdapterConfig::engram_for_data_root(
            harness._temp.path(),
            "engram-late-binding-test.db",
        ))
        .unwrap(),
    );
    let db = Arc::new(KnowledgeDatabase::new(harness.paths.clone()).unwrap());
    let episodes: Arc<dyn zbot_stores_traits::KgEpisodeStore> = Arc::new(
        GatewayKgEpisodeStore::new(Arc::new(KgEpisodeRepository::new(db.clone()))),
    );
    let ingestion: Arc<dyn agent_tools::IngestionAccess> =
        Arc::new(crate::invoke::ingest_adapter::IngestionAdapter::new(
            Arc::new(crate::ingest::IngestionQueue::start(
                0,
                episodes.clone(),
                graph.clone(),
                Arc::new(crate::ingest::extractor::NoopExtractor::new()),
            )),
            episodes.clone(),
            graph.clone(),
        ));
    let goals: Arc<dyn agent_tools::GoalAccess> =
        Arc::new(crate::invoke::goal_adapter::GoalAdapter::new(Arc::new(
            GatewayGoalStore::new(Arc::new(GoalRepository::new(db))),
        )));
    harness.runner.set_kg_store(graph.clone());
    harness.runner.set_kg_episode_store(episodes.clone());
    harness.runner.set_ingestion_adapter(ingestion.clone());
    harness.runner.set_goal_adapter(goals.clone());
    for snapshot in [
        continuation.integrations.snapshot(),
        delegation.integrations.snapshot(),
        harness.runner.bootstrap.integrations.snapshot(),
    ] {
        assert!(snapshot
            .kg_store
            .as_ref()
            .is_some_and(|store| Arc::ptr_eq(store, &graph)));
        assert!(snapshot
            .kg_episode_store
            .as_ref()
            .is_some_and(|store| Arc::ptr_eq(store, &episodes)));
        assert!(snapshot
            .ingestion_adapter
            .as_ref()
            .is_some_and(|adapter| Arc::ptr_eq(adapter, &ingestion)));
        assert!(snapshot
            .goal_adapter
            .as_ref()
            .is_some_and(|adapter| Arc::ptr_eq(adapter, &goals)));
    }
    assert!(Arc::ptr_eq(
        &continuation.handles,
        &harness.runner.control.handles
    ));
    assert!(Arc::ptr_eq(
        &delegation.delegation_registry,
        &harness.runner.control.delegation_registry
    ));
    assert!(Arc::ptr_eq(
        &delegation.rate_limiters,
        &harness.runner.rate_limiters
    ));
    assert!(Arc::ptr_eq(
        &continuation.steering_registry,
        &harness.runner.steering_registry
    ));
}

async fn assert_respond_persisted_before_completion(continuation: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let delta = serde_json::json!({"choices":[{"delta":{"tool_calls":[{
                "index":0,"id":"respond-no-token","function":{
                    "name":"respond","arguments":r#"{"message":"durable answer without tokens"}"#
                }
            }]},"finish_reason":null}]});
        let body = format!("data: {delta}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n");
        write_sse(&mut socket, &body).await;
    });
    let harness = build_harness(base_url).await;
    let mut events = harness.runner.event_bus.subscribe_all();
    let session_id;
    if continuation {
        let (session, root) = harness.state.create_session("root").unwrap();
        harness.state.start_execution(&root.id).unwrap();
        harness.state.complete_execution(&root.id).unwrap();
        harness.state.complete_session(&session.id).unwrap();
        session_id = session.id;
        invoke_continuation(ContinuationArgs {
            session_id: &session_id,
            root_agent_id: "root",
            event_bus: harness.runner.event_bus.clone(),
            agent_service: harness.runner.agent_service.clone(),
            provider_service: harness.runner.provider_service.clone(),
            mcp_service: harness.runner.mcp_service.clone(),
            skill_service: harness.runner.skill_service.clone(),
            paths: harness.runner.paths.clone(),
            messages: harness.runner.messages.clone(),
            checkpoints: harness.runner.checkpoints.clone(),
            handles: harness.runner.control.handles.clone(),
            delegation_registry: harness.runner.control.delegation_registry.clone(),
            delegation_tx: harness.runner.delegation_tx.clone(),
            log_service: harness.runner.log_service.clone(),
            state_service: harness.runner.control.state_service.clone(),
            memory_store: None,
            embedding_client: None,
            distiller: None,
            handoff_writer: None,
            memory_recall: None,
            peer_messages: harness.runner.peer_messages.clone(),
            a2a_delegation: harness.runner.a2a_delegation.clone(),
            steering_registry: harness.steering.clone(),
            model_registry: None,
            kg_store: None,
            kg_episode_store: None,
            ingestion_adapter: None,
            goal_adapter: None,
            procedure_store: None,
            ward_usage: harness.runner.ward_usage.clone(),
        })
        .await
        .unwrap();
    } else {
        (_, session_id) = harness
            .runner
            .invoke_with_callback(
                ExecutionConfig::new(
                    "root".to_owned(),
                    "respond-persistence".to_owned(),
                    harness.paths.vault_dir().clone(),
                )
                .with_mode("chat".to_owned()),
                "answer now".to_owned(),
                None,
            )
            .await
            .unwrap();
    }
    timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await.unwrap() {
                gateway_events::GatewayEvent::AgentCompleted {
                    session_id: completed,
                    ..
                } if completed == session_id => break,
                _ => {}
            }
        }
    })
    .await
    .expect("completion event");
    // Read immediately at the public completion boundary, without polling the store.
    let rows = harness
        .runner
        .messages
        .replay(&session_id, None, 100)
        .unwrap();
    let answers: Vec<_> = rows
        .iter()
        .filter(|row| row.role == "assistant" && row.content == "durable answer without tokens")
        .collect();
    assert_eq!(
        answers.len(),
        1,
        "answer must already be durable exactly once"
    );
    let result = rows
        .iter()
        .find(|row| row.tool_call_id.as_deref() == Some("respond-no-token"))
        .unwrap();
    assert!(
        answers[0].seq < result.seq,
        "assistant arguments precede the tool result"
    );
    assert!(answers[0].tool_calls.as_ref().unwrap().contains("respond"));
    provider.await.unwrap();
}

#[tokio::test]
async fn root_no_token_respond_is_durable_before_completion() {
    assert_respond_persisted_before_completion(false).await;
}

#[tokio::test]
async fn continuation_no_token_respond_is_durable_before_completion() {
    assert_respond_persisted_before_completion(true).await;
}

#[tokio::test]
async fn turn_checkpoint_records_cursor_and_represented_outputs() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        write_sse(&mut socket, body).await;
    });
    let harness = build_harness(base_url).await;
    let (_, session_id) = harness
        .runner
        .invoke_with_callback(
            ExecutionConfig::new(
                "root".to_owned(),
                "checkpoint-cursor".to_owned(),
                harness.paths.vault_dir().clone(),
            )
            .with_mode("chat".to_owned()),
            "checkpoint me".to_owned(),
            None,
        )
        .await
        .unwrap();
    let mut events = harness.runner.event_bus.subscribe_all();
    timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await.unwrap() {
                GatewayEvent::AgentCompleted {
                    session_id: done, ..
                } if done == session_id => break,
                _ => {}
            }
        }
    })
    .await
    .expect("completion event");
    let execution_id = harness
        .state
        .get_root_execution(&session_id)
        .unwrap()
        .expect("root execution")
        .id;
    let checkpoint = harness
        .runner
        .checkpoints
        .latest(&execution_id)
        .unwrap()
        .expect("turn checkpoint");
    let context: serde_json::Value =
        serde_json::from_str(checkpoint.context_state.as_deref().unwrap()).unwrap();
    let cursor: super::super::recovery::RecoveryCursor = serde_json::from_value(
        context
            .get(super::super::recovery::GATEWAY_RECOVERY_KEY)
            .cloned()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        cursor.input_cursor, 0,
        "fresh session scanned no prior rows"
    );
    let rows = harness
        .runner
        .messages
        .replay(&session_id, None, 100)
        .unwrap();
    assert!(
        cursor
            .represented_output_ids
            .iter()
            .all(|id| rows.iter().any(|row| &row.id == id)),
        "represented ids must reference durable rows"
    );
    assert!(
        rows.iter().any(
            |row| row.execution_id.as_deref() == Some(execution_id.as_str())
                && cursor.represented_output_ids.contains(&row.id)
        ),
        "this execution's durable rows are represented outputs"
    );
    provider.await.unwrap();
}

/// Seed a session with prior rows, a represented prompt row and a racing
/// callback, then prove the continuation composes tape + callback without
/// duplicating the prompt or prior turns.
#[tokio::test]
async fn continuation_restores_tape_with_racing_callback_and_advances_cursor() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (body_tx, body_rx) = tokio::sync::oneshot::channel::<String>();
    let provider = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let body = read_request_body(&mut socket).await;
        let _ = body_tx.send(body);
        let done = "data: {\"choices\":[{\"delta\":{\"content\":\"resumed\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        write_sse(&mut socket, done).await;
    });
    let harness = build_harness(base_url).await;
    let (session, root_exec) = harness.state.create_session("root").unwrap();
    let session_id = session.id.clone();
    let execution_id = root_exec.id.clone();
    let messages = harness.runner.messages.clone();
    let prior_user = zbot_conversation::Message {
        id: "msg-prior-user".to_owned(),
        execution_id: None,
        session_id: session_id.clone(),
        role: "user".to_owned(),
        content: "prior question from human".to_owned(),
        created_at: chrono::Utc::now().to_rfc3339(),
        token_count: 4,
        tool_calls: None,
        tool_call_id: None,
        seq: 0,
    };
    let prior_assistant = zbot_conversation::Message {
        id: "msg-prior-assistant".to_owned(),
        role: "assistant".to_owned(),
        content: "prior answer from agent".to_owned(),
        ..prior_user.clone()
    };
    let prompt_row = zbot_conversation::Message {
        id: "msg-prompt-restore".to_owned(),
        role: "user".to_owned(),
        content: "do the thing".to_owned(),
        execution_id: Some(execution_id.clone()),
        ..prior_user.clone()
    };
    messages.append(&prior_user).unwrap();
    messages.append(&prior_assistant).unwrap();
    messages.append(&prompt_row).unwrap();
    let callback = zbot_conversation::Message {
        id: "msg-callback".to_owned(),
        role: "system".to_owned(),
        content: "## From Research Agent\ndurable result".to_owned(),
        ..prior_user.clone()
    };
    messages.append(&callback).unwrap();

    // Private tape: prior turns + the prompt. Cursor covers the two prior
    // rows; the prompt row is a represented output of this execution.
    let tape = serde_json::json!({
        "version": 1,
        "owned_preamble": null,
        "messages": [
            {"role":"user","content":[{"type":"text","text":"prior question from human"}],"tool_calls":null,"tool_call_id":null,"is_summary":false},
            {"role":"assistant","content":[{"type":"text","text":"prior answer from agent"}],"tool_calls":null,"tool_call_id":null,"is_summary":false},
            {"role":"user","content":[{"type":"text","text":"do the thing"}],"tool_calls":null,"tool_call_id":null,"is_summary":false}
        ],
        "mutable_state": {}
    });
    let mut engine_state = serde_json::Map::new();
    engine_state.insert(
        agent_runtime::engine::snapshot::CHECKPOINT_KEY.to_owned(),
        tape,
    );
    let context_state = super::super::recovery::checkpoint_context_state(
        serde_json::json!({"intent": null, "ward": null}),
        Some(&serde_json::Value::Object(engine_state)),
        &super::super::recovery::RecoveryCursor {
            input_cursor: 2,
            represented_output_ids: vec!["msg-prompt-restore".to_owned()],
        },
    );
    harness
        .runner
        .checkpoints
        .write(&zbot_conversation::Checkpoint {
            id: "cp-restore".to_owned(),
            execution_id: execution_id.clone(),
            session_id: session_id.clone(),
            llm_turn: 1,
            last_message_id: String::new(),
            pending_tool_calls: None,
            context_state: Some(context_state),
            child_executions: None,
            schema_version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();

    harness.state.start_execution(&execution_id).unwrap();
    harness.state.complete_execution(&execution_id).unwrap();
    harness.state.complete_session(&session_id).unwrap();
    let mut events = harness.runner.event_bus.subscribe_all();
    let mut completed_count = 0_usize;
    invoke_continuation(ContinuationArgs {
        session_id: &session_id,
        root_agent_id: "root",
        event_bus: harness.runner.event_bus.clone(),
        agent_service: harness.runner.agent_service.clone(),
        provider_service: harness.runner.provider_service.clone(),
        mcp_service: harness.runner.mcp_service.clone(),
        skill_service: harness.runner.skill_service.clone(),
        paths: harness.runner.paths.clone(),
        messages: harness.runner.messages.clone(),
        checkpoints: harness.runner.checkpoints.clone(),
        handles: harness.runner.control.handles.clone(),
        delegation_registry: harness.runner.control.delegation_registry.clone(),
        delegation_tx: harness.runner.delegation_tx.clone(),
        log_service: harness.runner.log_service.clone(),
        state_service: harness.runner.control.state_service.clone(),
        memory_store: None,
        embedding_client: None,
        distiller: None,
        handoff_writer: None,
        memory_recall: None,
        peer_messages: harness.runner.peer_messages.clone(),
        a2a_delegation: None,
        steering_registry: harness.steering.clone(),
        model_registry: None,
        kg_store: None,
        kg_episode_store: None,
        ingestion_adapter: None,
        goal_adapter: None,
        procedure_store: None,
        ward_usage: harness.runner.ward_usage.clone(),
    })
    .await
    .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await.unwrap() {
                GatewayEvent::AgentCompleted {
                    session_id: done, ..
                } if done == session_id => {
                    completed_count += 1;
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("completion event");
    // Exactly one terminal outcome: drain stragglers and re-count.
    tokio::time::sleep(Duration::from_millis(200)).await;
    while let Ok(event) = events.try_recv() {
        if matches!(
            &event,
            GatewayEvent::AgentCompleted { session_id: done, .. } if *done == session_id
        ) {
            completed_count += 1;
        }
    }
    assert_eq!(completed_count, 1, "no duplicate terminal event");
    let body = body_rx.await.unwrap();
    let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
    let request_text = payload["messages"].to_string();
    for (needle, count) in [
        ("prior question from human", 1),
        ("prior answer from agent", 1),
        ("do the thing", 1),
        ("## From Research Agent", 1),
    ] {
        assert_eq!(
            request_text.matches(needle).count(),
            count,
            "{needle} must appear exactly {count} time(s) in the model request"
        );
    }
    // The next checkpoint advanced past the callback and the new own rows
    // are represented outputs.
    let next = harness
        .runner
        .checkpoints
        .latest(&execution_id)
        .unwrap()
        .expect("advanced checkpoint");
    let context: serde_json::Value =
        serde_json::from_str(next.context_state.as_deref().unwrap()).unwrap();
    let cursor: super::super::recovery::RecoveryCursor = serde_json::from_value(
        context
            .get(super::super::recovery::GATEWAY_RECOVERY_KEY)
            .cloned()
            .unwrap(),
    )
    .unwrap();
    assert!(
        cursor.input_cursor >= 3,
        "cursor advanced across scanned rows"
    );
    // Old represented rows sit at/below the advanced cursor, so they need
    // no repeat entry; the new set covers this invocation's own rows.
    assert!(
        !cursor.represented_output_ids.is_empty(),
        "this invocation's durable outputs are represented"
    );
    let rows = harness
        .runner
        .messages
        .replay(&session_id, None, 100)
        .unwrap();
    assert!(
        cursor
            .represented_output_ids
            .iter()
            .all(|id| rows.iter().any(|row| &row.id == id)),
        "represented ids reference durable rows"
    );
    provider.await.unwrap();
}

#[tokio::test]
async fn malformed_private_snapshot_fails_continuation_explicitly() {
    let harness = build_harness("http://127.0.0.1:1/v1".to_owned()).await;
    let (session, root_exec) = harness.state.create_session("root").unwrap();
    let session_id = session.id.clone();
    let execution_id = root_exec.id.clone();
    let mut engine_state = serde_json::Map::new();
    engine_state.insert(
        agent_runtime::engine::snapshot::CHECKPOINT_KEY.to_owned(),
        serde_json::json!({"version": 99, "messages": [], "mutable_state": {}}),
    );
    let context_state = super::super::recovery::checkpoint_context_state(
        serde_json::json!({"intent": null}),
        Some(&serde_json::Value::Object(engine_state)),
        &super::super::recovery::RecoveryCursor::default(),
    );
    harness
        .runner
        .checkpoints
        .write(&zbot_conversation::Checkpoint {
            id: "cp-bad".to_owned(),
            execution_id: execution_id.clone(),
            session_id: session_id.clone(),
            llm_turn: 1,
            last_message_id: String::new(),
            pending_tool_calls: None,
            context_state: Some(context_state),
            child_executions: None,
            schema_version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    let error = invoke_continuation(ContinuationArgs {
        session_id: &session_id,
        root_agent_id: "root",
        event_bus: harness.runner.event_bus.clone(),
        agent_service: harness.runner.agent_service.clone(),
        provider_service: harness.runner.provider_service.clone(),
        mcp_service: harness.runner.mcp_service.clone(),
        skill_service: harness.runner.skill_service.clone(),
        paths: harness.runner.paths.clone(),
        messages: harness.runner.messages.clone(),
        checkpoints: harness.runner.checkpoints.clone(),
        handles: harness.runner.control.handles.clone(),
        delegation_registry: harness.runner.control.delegation_registry.clone(),
        delegation_tx: harness.runner.delegation_tx.clone(),
        log_service: harness.runner.log_service.clone(),
        state_service: harness.runner.control.state_service.clone(),
        memory_store: None,
        embedding_client: None,
        distiller: None,
        handoff_writer: None,
        memory_recall: None,
        peer_messages: harness.runner.peer_messages.clone(),
        a2a_delegation: None,
        steering_registry: harness.steering.clone(),
        model_registry: None,
        kg_store: None,
        kg_episode_store: None,
        ingestion_adapter: None,
        goal_adapter: None,
        procedure_store: None,
        ward_usage: harness.runner.ward_usage.clone(),
    })
    .await
    .unwrap_err();
    assert!(
        error.contains("Unsupported execution checkpoint version"),
        "malformed snapshot must fail explicitly: {error}"
    );
}

/// The same failure driven through the watcher's invoker must crash the
/// session and publish a terminal error — not leave it hanging with
/// completed delegations and no outcome.
#[tokio::test]
async fn continuation_spawn_failure_crashes_session_with_terminal_event() {
    let harness = build_harness("http://127.0.0.1:1/v1".to_owned()).await;
    let (session, root_exec) = harness.state.create_session("root").unwrap();
    let session_id = session.id.clone();
    let execution_id = root_exec.id.clone();
    let mut engine_state = serde_json::Map::new();
    engine_state.insert(
        agent_runtime::engine::snapshot::CHECKPOINT_KEY.to_owned(),
        serde_json::json!({"version": 1, "messages": [], "mutable_state": {}}),
    );
    // Snapshot without cursor metadata → explicit cursor failure.
    let context_state = serde_json::Value::Object(engine_state).to_string();
    harness
        .runner
        .checkpoints
        .write(&zbot_conversation::Checkpoint {
            id: "cp-no-cursor".to_owned(),
            execution_id: execution_id.clone(),
            session_id: session_id.clone(),
            llm_turn: 1,
            last_message_id: String::new(),
            pending_tool_calls: None,
            context_state: Some(context_state),
            child_executions: None,
            schema_version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    let mut events = harness.runner.event_bus.subscribe_all();
    let invoker = harness.runner.make_continuation_invoker();
    use crate::runner::ContinuationSpawner as _;
    invoker
        .spawn_continuation(session_id.clone(), "root".to_owned())
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await.unwrap() {
                GatewayEvent::Error {
                    session_id: done, ..
                } if done == Some(session_id.clone()) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("terminal error event");
    let crashed = harness.state.get_session(&session_id).unwrap().unwrap();
    assert_eq!(
        crashed.status,
        execution_state::SessionStatus::Crashed,
        "session must reach a terminal crashed state, not hang"
    );
}

#[tokio::test]
async fn checkpoint_read_error_fails_continuation_explicitly() {
    struct FailingCheckpoints;
    impl zbot_conversation::CheckpointStore for FailingCheckpoints {
        fn write(&self, _cp: &zbot_conversation::Checkpoint) -> anyhow::Result<()> {
            Ok(())
        }
        fn latest(
            &self,
            _execution_id: &str,
        ) -> anyhow::Result<Option<zbot_conversation::Checkpoint>> {
            Err(anyhow::anyhow!("store offline"))
        }
    }
    let harness = build_harness("http://127.0.0.1:1/v1".to_owned()).await;
    let (session, _root_exec) = harness.state.create_session("root").unwrap();
    let session_id = session.id.clone();
    let error = invoke_continuation(ContinuationArgs {
        session_id: &session_id,
        root_agent_id: "root",
        event_bus: harness.runner.event_bus.clone(),
        agent_service: harness.runner.agent_service.clone(),
        provider_service: harness.runner.provider_service.clone(),
        mcp_service: harness.runner.mcp_service.clone(),
        skill_service: harness.runner.skill_service.clone(),
        paths: harness.runner.paths.clone(),
        messages: harness.runner.messages.clone(),
        checkpoints: std::sync::Arc::new(FailingCheckpoints),
        handles: harness.runner.control.handles.clone(),
        delegation_registry: harness.runner.control.delegation_registry.clone(),
        delegation_tx: harness.runner.delegation_tx.clone(),
        log_service: harness.runner.log_service.clone(),
        state_service: harness.runner.control.state_service.clone(),
        memory_store: None,
        embedding_client: None,
        distiller: None,
        handoff_writer: None,
        memory_recall: None,
        peer_messages: harness.runner.peer_messages.clone(),
        a2a_delegation: None,
        steering_registry: harness.steering.clone(),
        model_registry: None,
        kg_store: None,
        kg_episode_store: None,
        ingestion_adapter: None,
        goal_adapter: None,
        procedure_store: None,
        ward_usage: harness.runner.ward_usage.clone(),
    })
    .await
    .unwrap_err();
    assert!(
        error.contains("continuation_checkpoint_read_failed"),
        "checkpoint query errors are not an absent checkpoint: {error}"
    );
}

async fn deliver_during_first_turn(
    steering: Arc<agent_runtime::SteeringRegistry>,
    execution_id: String,
    first_seen: oneshot::Receiver<()>,
    release_first: oneshot::Sender<()>,
) {
    timeout(Duration::from_secs(5), first_seen)
        .await
        .expect("first LLM request")
        .unwrap();
    assert!(steering.has_peer_handle(&execution_id));
    assert_eq!(
        steering.steer(&execution_id, "parent path must not target root"),
        SteerResult::AgentNotRunning
    );
    let peer = tokio::spawn({
        let steering = steering.clone();
        let execution_id = execution_id.clone();
        async move { steering.steer_peer(&execution_id, "peer reply").await }
    });
    tokio::task::yield_now().await;
    release_first.send(()).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(5), peer)
            .await
            .expect("peer delivery")
            .unwrap(),
        SteerResult::Delivered
    );
    timeout(Duration::from_secs(5), async {
        while steering.has_peer_handle(&execution_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("peer handle cleanup");
}

#[tokio::test]
async fn initial_invoke_registers_root_for_peer_only_delivery_and_cleans_up() {
    let (base_url, first_seen, release_first) = spawn_two_turn_llm().await;
    let harness = build_harness(base_url).await;
    let (_, session_id) = harness
        .runner
        .invoke_with_callback(
            ExecutionConfig::new(
                "root".to_owned(),
                "peer-root-initial".to_owned(),
                harness.paths.vault_dir().clone(),
            )
            .with_mode("chat".to_owned()),
            "hi".to_owned(),
            None,
        )
        .await
        .unwrap();
    let execution_id = harness
        .state
        .get_root_execution(&session_id)
        .unwrap()
        .unwrap()
        .id;
    deliver_during_first_turn(harness.steering, execution_id, first_seen, release_first).await;
}

#[tokio::test]
async fn continuation_registers_root_for_peer_only_delivery_and_cleans_up() {
    let (base_url, first_seen, release_first) = spawn_two_turn_llm().await;
    let harness = build_harness(base_url).await;
    let (session, root) = harness.state.create_session("root").unwrap();
    harness.state.start_execution(&root.id).unwrap();
    harness.state.complete_execution(&root.id).unwrap();
    harness.state.complete_session(&session.id).unwrap();

    invoke_continuation(ContinuationArgs {
        session_id: &session.id,
        root_agent_id: "root",
        event_bus: harness.runner.event_bus.clone(),
        agent_service: harness.runner.agent_service.clone(),
        provider_service: harness.runner.provider_service.clone(),
        mcp_service: harness.runner.mcp_service.clone(),
        skill_service: harness.runner.skill_service.clone(),
        paths: harness.runner.paths.clone(),
        messages: harness.runner.messages.clone(),
        checkpoints: harness.runner.checkpoints.clone(),
        handles: harness.runner.control.handles.clone(),
        delegation_registry: harness.runner.control.delegation_registry.clone(),
        delegation_tx: harness.runner.delegation_tx.clone(),
        log_service: harness.runner.log_service.clone(),
        state_service: harness.runner.control.state_service.clone(),
        memory_store: None,
        embedding_client: None,
        distiller: None,
        handoff_writer: None,
        memory_recall: None,
        peer_messages: harness.runner.peer_messages.clone(),
        a2a_delegation: harness.runner.a2a_delegation.clone(),
        steering_registry: harness.steering.clone(),
        model_registry: None,
        kg_store: None,
        kg_episode_store: None,
        ingestion_adapter: None,
        goal_adapter: None,
        procedure_store: None,
        ward_usage: harness.runner.ward_usage.clone(),
    })
    .await
    .unwrap();

    deliver_during_first_turn(harness.steering, root.id, first_seen, release_first).await;
}

#[tokio::test]
async fn graceful_restart_resume_rebuilds_paused_peer_target_with_same_id() {
    let (base_url, _first_seen, _release_first) = spawn_two_turn_llm().await;
    let harness = build_harness(base_url).await;
    let (session, root) = harness.state.create_session("root").unwrap();
    harness.state.start_execution(&root.id).unwrap();
    let child = harness
        .state
        .create_delegated_execution(
            &session.id,
            "resume-test-agent",
            &root.id,
            DelegationType::Sequential,
            "resume after graceful restart",
        )
        .unwrap();
    harness.state.start_execution(&child.id).unwrap();
    let child_session = Session::new_child(&child.agent_id, &session.id);
    harness.state.create_session_from(&child_session).unwrap();
    harness
        .state
        .set_child_session_id(&child.id, &child_session.id)
        .unwrap();
    let peer_messages = harness.runner.peer_messages.as_ref().unwrap();
    let receipt = peer_messages
        .enqueue_message(
            crate::peer_messaging::PeerMessageContext {
                node_id: crate::peer_messaging::PEER_MESSAGE_TARGET.to_owned(),
                agent_id: root.agent_id.clone(),
                session_id: session.id.clone(),
                execution_id: root.id.clone(),
            },
            &child.id,
            "survive graceful restart",
        )
        .await
        .unwrap();
    harness.state.register_delegation(&session.id).unwrap();
    harness.state.request_continuation(&session.id).unwrap();

    harness.state.mark_running_as_paused().unwrap();
    assert_eq!(
        harness
            .state
            .get_execution(&child.id)
            .unwrap()
            .unwrap()
            .status,
        execution_state::ExecutionStatus::Paused
    );
    harness.runner.resume(&session.id).await.unwrap();

    let resumed = harness
        .state
        .get_session_with_executions(&session.id)
        .unwrap()
        .unwrap();
    let delegated: Vec<_> = resumed
        .executions
        .iter()
        .filter(|execution| execution.parent_execution_id.is_some())
        .collect();
    assert_eq!(delegated.len(), 1);
    assert_eq!(delegated[0].id, child.id);
    assert_eq!(
        delegated[0].status,
        execution_state::ExecutionStatus::Running
    );
    let resumed_session = harness.state.get_session(&session.id).unwrap().unwrap();
    assert_eq!(resumed_session.pending_delegations, 1);
    assert!(resumed_session.continuation_needed);
    let pending = peer_messages
        .store()
        .get(&receipt.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        pending.envelope().payload()["target_execution_id"],
        child.id
    );
}
