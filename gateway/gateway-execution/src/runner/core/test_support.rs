//! Shared scripting harness for runner integration tests: scripted SSE
//! providers over a real TCP listener, real stores, and the full
//! ExecutionRunner wiring.

use super::*;
use agent_primitives::vault_paths::VaultPaths;
use execution_state::{SqliteWorkStore, WorkStore};
use gateway_bus::LocalWorkTransport;
use gateway_services::{agents::Agent, providers::Provider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(super) use tokio::net::{TcpListener, TcpStream};
pub(super) use tokio::sync::oneshot;

pub(super) struct Harness {
    pub(super) _temp: tempfile::TempDir,
    pub(super) runner: ExecutionRunner,
    pub(super) state: Arc<StateService<DatabaseManager>>,
    pub(super) steering: Arc<agent_runtime::SteeringRegistry>,
    pub(super) paths: SharedVaultPaths,
}

pub(super) async fn read_request(stream: &mut TcpStream) {
    read_request_body(stream).await;
}

/// Read one HTTP request and return its body (for history assertions).
pub(super) async fn read_request_body(stream: &mut TcpStream) -> String {
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

pub(super) fn expected_header_end(bytes: &[u8]) -> usize {
    bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .map(|end| end + 4)
        .unwrap_or(0)
}

pub(super) async fn write_sse(stream: &mut TcpStream, body: &str) {
    let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

pub(super) async fn spawn_two_turn_llm() -> (String, oneshot::Receiver<()>, oneshot::Sender<()>) {
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

pub(super) async fn build_harness(base_url: String) -> Harness {
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
            context_window: Some(32_768),
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
            max_input_tokens: 32_768,
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
        steering: runner.ctx.steering_registry.clone(),
        _temp: temp,
        runner,
        state,
        paths,
    }
}
