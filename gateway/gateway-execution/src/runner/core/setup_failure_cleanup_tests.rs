
use super::*;
use execution_state::{
    DelegationType, ExecutionStatus, Session, SessionStatus, SqliteWorkStore, WorkStore,
};
use gateway_bus::LocalWorkTransport;
use gateway_services::{agents::Agent, providers::Provider, VaultPaths};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn post_start_setup_failure_crashes_the_session_and_removes_its_handle() {
    let temp = tempfile::tempdir().unwrap();
    let paths: SharedVaultPaths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    let db = Arc::new(DatabaseManager::new(paths.clone()).unwrap());
    let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap();
    let messages: Arc<dyn zbot_conversation::MessageStore> =
        Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
    let session_meta: Arc<dyn zbot_conversation::SessionMetaStore> =
        Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool.clone()));
    let checkpoints: Arc<dyn zbot_conversation::CheckpointStore> =
        Arc::new(zbot_conversation::SqliteCheckpointStore::new(pool));
    let state_service = Arc::new(StateService::new(db.clone()));
    let event_bus = Arc::new(EventBus::new());
    let runner = ExecutionRunner::with_config(ExecutionRunnerConfig {
        event_bus: event_bus.clone(),
        agent_service: Arc::new(AgentService::new(paths.agents_dir())),
        provider_service: Arc::new(ProviderService::new(paths.clone())),
        paths: paths.clone(),
        mcp_service: Arc::new(McpService::new(paths.clone())),
        skill_service: Arc::new(gateway_services::SkillService::new(paths.skills_dir())),
        log_service: Arc::new(LogService::new(db)),
        state_service: state_service.clone(),
        ward_usage: Arc::new(gateway_services::WardUsage::new(paths.wards_dir())),
        messages,
        session_meta,
        checkpoints,
        connector_registry: None,
        memory_store: None,
        distiller: None,
        handoff_writer: None,
        memory_recall: None,
        peer_messages: None,
        a2a_delegation: None,
        bridge_registry: None,
        bridge_outbox: None,
        embedding_client: None,
        procedure_store: None,
        procedure_recommendation_cfg: gateway_memory::ProcedureRecommendationConfig::default(),
        max_parallel_agents: 1,
    });
    let session_id = Arc::new(Mutex::new(None));
    let callback_session_id = session_id.clone();
    let on_session_ready: OnSessionReady = Box::new(move |id| {
        Box::pin(async move {
            *callback_session_id.lock().unwrap() = Some(id);
        })
    });
    let mut events = event_bus.subscribe_all();
    let conversation_id = "setup-failure-conversation";
    let result = runner
        .invoke_with_callback(
            ExecutionConfig::new(
                "root".to_string(),
                conversation_id.to_string(),
                paths.vault_dir().clone(),
            ),
            "trigger a setup failure without a configured provider".to_string(),
            Some(on_session_ready),
        )
        .await;

    assert!(matches!(
        result,
        Err(ref message) if message == "Unable to start this request"
    ));
    let session_id = session_id
        .lock()
        .unwrap()
        .clone()
        .expect("phase one must expose the session before setup fails");
    assert!(runner.get_handle(conversation_id).await.is_none());
    let session = state_service
        .get_session_with_executions(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(session.session.status, SessionStatus::Crashed);
    assert_eq!(session.executions[0].status, ExecutionStatus::Crashed);

    let mut emitted_safe_error = false;
    while let Ok(event) = events.try_recv() {
        if let GatewayEvent::Error { message, .. } = event {
            emitted_safe_error |= message == "Unable to start this request";
        }
    }
    assert!(
        emitted_safe_error,
        "setup cleanup must publish a safe error"
    );
}

#[tokio::test]
async fn smart_resume_preserves_execution_id_addressed_by_durable_peer_work() {
    let temp = tempfile::tempdir().unwrap();
    let paths: SharedVaultPaths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (_connection, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let db = Arc::new(DatabaseManager::new(paths.clone()).unwrap());
    let state_service = Arc::new(StateService::new(db.clone()));
    let provider_service = Arc::new(ProviderService::new(paths.clone()));
    provider_service
        .create(Provider {
            id: Some("provider-resume-test".to_owned()),
            name: "Resume Test".to_owned(),
            description: "local test provider".to_owned(),
            api_key: "test-key".to_owned(),
            base_url: provider_url,
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
    let agent_service = Arc::new(AgentService::new(paths.agents_dir()));
    agent_service
        .create(Agent {
            id: "resume-test-agent".to_owned(),
            name: "resume-test-agent".to_owned(),
            display_name: "Resume Test Agent".to_owned(),
            description: "test resumed delegation".to_owned(),
            agent_type: Some("specialist".to_owned()),
            provider_id: "provider-resume-test".to_owned(),
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
    let store: Arc<dyn WorkStore> = Arc::new(SqliteWorkStore::new(db.clone()));
    let transport = Arc::new(LocalWorkTransport::new());
    let peer_messages = Arc::new(crate::peer_messaging::DurablePeerMessageService::new(
        store.clone(),
        transport,
        state_service.clone(),
        crate::peer_messaging::PEER_MESSAGE_TARGET,
    ));

    let (session, root) = state_service.create_session("root").unwrap();
    state_service.start_execution(&root.id).unwrap();
    let child = state_service
        .create_delegated_execution(
            &session.id,
            "resume-test-agent",
            &root.id,
            DelegationType::Sequential,
            "continue durable work",
        )
        .unwrap();
    state_service.start_execution(&child.id).unwrap();
    let child_session = Session::new_child(&child.agent_id, &session.id);
    state_service.create_session_from(&child_session).unwrap();
    state_service
        .set_child_session_id(&child.id, &child_session.id)
        .unwrap();

    let receipt = peer_messages
        .enqueue_message(
            crate::peer_messaging::PeerMessageContext {
                node_id: crate::peer_messaging::PEER_MESSAGE_TARGET.to_owned(),
                agent_id: root.agent_id.clone(),
                session_id: session.id.clone(),
                execution_id: root.id.clone(),
            },
            &child.id,
            "survive smart resume",
        )
        .await
        .unwrap();
    state_service.crash_session(&session.id).unwrap();
    state_service.crash_session(&child_session.id).unwrap();

    let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap();
    let messages: Arc<dyn zbot_conversation::MessageStore> =
        Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
    let session_meta: Arc<dyn zbot_conversation::SessionMetaStore> =
        Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool.clone()));
    let checkpoints: Arc<dyn zbot_conversation::CheckpointStore> =
        Arc::new(zbot_conversation::SqliteCheckpointStore::new(pool));
    let runner = ExecutionRunner::with_config(ExecutionRunnerConfig {
        event_bus: Arc::new(EventBus::new()),
        agent_service,
        provider_service,
        paths: paths.clone(),
        mcp_service: Arc::new(McpService::new(paths.clone())),
        skill_service: Arc::new(gateway_services::SkillService::new(paths.skills_dir())),
        log_service: Arc::new(LogService::new(db)),
        state_service: state_service.clone(),
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

    runner.resume(&session.id).await.unwrap();

    let executions = state_service
        .get_session_with_executions(&session.id)
        .unwrap()
        .unwrap();
    let delegated: Vec<_> = executions
        .executions
        .iter()
        .filter(|execution| execution.parent_execution_id.is_some())
        .collect();
    assert_eq!(
        delegated.len(),
        1,
        "resume must not mint a replacement target"
    );
    assert_eq!(delegated[0].id, child.id);
    assert_eq!(delegated[0].status, ExecutionStatus::Running);
    let pending = store.get(&receipt.message_id).unwrap().unwrap();
    assert_eq!(
        pending.envelope().payload()["target_execution_id"],
        child.id
    );
}
