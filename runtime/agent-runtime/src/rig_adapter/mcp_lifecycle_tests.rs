//! The production factory owns MCP cleanup across every execution lifetime.

use super::{factory::build_engine, model::LlmCompletionModel, RigAgentConfig, RigModelConfig};
use crate::{
    engine::{AgentEngine, ExecutorConfig, PreparedExecution},
    llm::{ChatMessage, ChatResponse, LlmClient, LlmConfig, LlmError, StreamCallback},
    mcp::{McpManager, McpServerConfig},
    middleware::MiddlewarePipeline,
    tools::ToolRegistry,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

struct Provider {
    mode: &'static str,
    entered: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl LlmClient for Provider {
    fn model(&self) -> &str {
        "fixture"
    }
    fn provider(&self) -> &str {
        "fixture"
    }
    async fn chat(&self, _: Vec<ChatMessage>, _: Option<Value>) -> Result<ChatResponse, LlmError> {
        unreachable!("Rig streams")
    }
    async fn chat_stream(
        &self,
        _: Vec<ChatMessage>,
        _: Option<Value>,
        _: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        self.entered.notify_one();
        match self.mode {
            "pending" => futures::future::pending().await,
            "error" => Err(LlmError::ApiError("fixture failure".into())),
            _ => Ok(ChatResponse {
                content: "done".into(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            }),
        }
    }
}

async fn fixture(
    mode: &'static str,
) -> (
    super::engine::RigAgentEngine<LlmCompletionModel>,
    Arc<McpManager>,
    tempfile::TempDir,
    Arc<Provider>,
) {
    let directory = tempfile::tempdir().unwrap();
    let manager = Arc::new(McpManager::new());
    let config: McpServerConfig = serde_json::from_value(json!({
        "type":"stdio", "id":"fixture", "name":"fixture", "description":"isolated execution fixture",
        "command":"python3", "args":["-u", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mcp_stdio_probe.py")],
        "env":{"PROBE_PID_FILE":directory.path().join("pid")}, "enabled":true,
    })).unwrap();
    manager.start_server(config).await.unwrap();
    let provider = Arc::new(Provider {
        mode,
        entered: tokio::sync::Notify::new(),
    });
    let prepared = PreparedExecution::new(
        ExecutorConfig::new("agent".into(), "fixture".into(), "fixture".into()),
        provider.clone(),
        Arc::new(ToolRegistry::new()),
        manager.clone(),
        Arc::new(MiddlewarePipeline::new()),
    );
    let config = RigAgentConfig::new(
        "agent",
        "Agent",
        "fixture",
        "",
        RigModelConfig::from_llm_config(
            &LlmConfig::new(
                "http://unused".into(),
                String::new(),
                "fixture".into(),
                "fixture".into(),
            ),
            8192,
        ),
    );
    (build_engine(prepared, config), manager, directory, provider)
}

async fn assert_closed(manager: &McpManager, directory: &tempfile::TempDir) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let removed = manager.get_client("fixture").await.is_none();
            #[cfg(target_os = "linux")]
            let reaped = {
                let pid = std::fs::read_to_string(directory.path().join("pid")).unwrap();
                !std::path::Path::new(&format!("/proc/{}", pid.trim())).exists()
            };
            #[cfg(not(target_os = "linux"))]
            let reaped = {
                let _ = directory;
                true
            };
            if removed && reaped {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("execution releases its session and child within five seconds");
}

#[tokio::test]
async fn factory_closes_mcp_after_success_and_provider_failure() {
    for mode in ["success", "error"] {
        let (engine, manager, directory, _) = fixture(mode).await;
        let retained = manager.get_client("fixture").await.unwrap();
        let result = engine.execute_stream("hello", &[], &mut |_| {}).await;
        assert_eq!(result.is_err(), mode == "error");
        assert_closed(&manager, &directory).await;
        assert!(retained.list_tools().await.is_err());
    }
}

#[tokio::test]
async fn dropped_rig_future_closes_mcp_while_engine_and_peer_are_retained() {
    let (engine, manager, directory, provider) = fixture("pending").await;
    let retained = manager.get_client("fixture").await.unwrap();
    let mut sink = |_| {};
    let mut execution = Box::pin(engine.execute_stream("hello", &[], &mut sink));
    tokio::select! {
        _ = &mut execution => panic!("provider remains pending"),
        _ = provider.entered.notified() => {},
        _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("provider did not start"),
    }
    drop(execution);
    assert_closed(&manager, &directory).await;
    assert!(retained.list_tools().await.is_err());
}

#[tokio::test]
async fn dropping_never_run_engine_closes_its_mcp_session_only() {
    let (first, manager, directory, _) = fixture("success").await;
    let (second, other, other_directory, _) = fixture("success").await;
    drop(first);
    assert_closed(&manager, &directory).await;
    assert_eq!(other.list_all_tools().await.unwrap().len(), 1);
    drop(second);
    assert_closed(&other, &other_directory).await;
}

#[test]
fn engine_cleanup_keeps_its_runtime_when_dropped_outside_async_context() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (engine, manager, directory, _) = runtime.block_on(fixture("success"));
    drop(engine);
    runtime.block_on(assert_closed(&manager, &directory));
}
