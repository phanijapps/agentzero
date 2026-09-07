use super::super::continuation_execution::invoke_continuation;
use super::test_support::*;
use super::*;
use agent_runtime::SteerResult;
use execution_state::{DelegationType, Session};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn captured_invokers_observe_stores_installed_after_construction() {
    use zbot_engram_adapter::{AdapterConfig, EngramKnowledgeGraphStore};
    use zbot_stores_sqlite::{
        GatewayGoalStore, GatewayKgEpisodeStore, GoalRepository, KgEpisodeRepository,
        KnowledgeDatabase,
    };
    let mut harness = build_harness("http://unused".into()).await;
    let continuation = harness.runner.ctx.clone();
    let delegation = harness.runner.ctx.clone();
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
        harness.runner.ctx.integrations.snapshot(),
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
        &continuation.control.handles,
        &harness.runner.ctx.control.handles
    ));
    assert!(Arc::ptr_eq(
        &delegation.control.delegation_registry,
        &harness.runner.ctx.control.delegation_registry
    ));
    assert!(Arc::ptr_eq(
        &delegation.rate_limiters,
        &harness.runner.ctx.rate_limiters
    ));
    assert!(Arc::ptr_eq(
        &continuation.steering_registry,
        &harness.runner.ctx.steering_registry
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
    let mut events = harness.runner.ctx.event_bus.subscribe_all();
    let session_id;
    if continuation {
        let (session, root) = harness.state.create_session("root").unwrap();
        harness.state.start_execution(&root.id).unwrap();
        harness.state.complete_execution(&root.id).unwrap();
        harness.state.complete_session(&session.id).unwrap();
        session_id = session.id;
        invoke_continuation(&harness.runner.ctx, &session_id, "root")
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
        .ctx
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
    let mut events = harness.runner.ctx.event_bus.subscribe_all();
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
        .ctx
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
        .ctx
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
    let messages = harness.runner.ctx.messages.clone();
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
        .ctx
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
    let mut events = harness.runner.ctx.event_bus.subscribe_all();
    let mut completed_count = 0_usize;
    invoke_continuation(&harness.runner.ctx, &session_id, "root")
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
        .ctx
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
        .ctx
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
        .ctx
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
    let error = invoke_continuation(&harness.runner.ctx, &session_id, "root")
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
        .ctx
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
    let mut events = harness.runner.ctx.event_bus.subscribe_all();
    let invoker = harness.runner.ctx.clone();
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
    let mut ctx = (*harness.runner.ctx).clone();
    ctx.checkpoints = Arc::new(FailingCheckpoints);
    let error = invoke_continuation(&ctx, &session_id, "root")
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

    invoke_continuation(&harness.runner.ctx, &session.id, "root")
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
    let peer_messages = harness.runner.ctx.peer_messages.as_ref().unwrap();
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
