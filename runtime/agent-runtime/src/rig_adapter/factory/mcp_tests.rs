use super::*;
use crate::{
    engine::{AgentEngine, ExecutorConfig},
    llm::{ChatMessage, ChatResponse, LlmClient, LlmConfig, LlmError, StreamCallback, StreamChunk},
    mcp::{McpClient, McpError, McpManager, McpServerConfig, McpTool},
    middleware::MiddlewarePipeline,
    rig_adapter::RigModelConfig,
    tools::ToolRegistry,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

struct ScriptedProvider {
    tool: String,
    calls: Mutex<Vec<(Vec<ChatMessage>, Option<Value>)>>,
}
#[async_trait::async_trait]
impl LlmClient for ScriptedProvider {
    fn model(&self) -> &str {
        "fixture"
    }
    fn provider(&self) -> &str {
        "fixture"
    }
    async fn chat(&self, _: Vec<ChatMessage>, _: Option<Value>) -> Result<ChatResponse, LlmError> {
        unreachable!()
    }
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let mut calls = self.calls.lock().unwrap();
        let first = calls.is_empty();
        calls.push((messages, tools));
        if !first {
            callback(StreamChunk::Token("done".into()));
        }
        Ok(ChatResponse {
            content: if first { String::new() } else { "done".into() },
            tool_calls: first.then(|| {
                vec![crate::types::ToolCall::new(
                    "call-1".into(),
                    self.tool.clone(),
                    json!({"value":"native-visible"}),
                )]
            }),
            reasoning: None,
            usage: None,
        })
    }
}
fn prepare(
    manager: Arc<McpManager>,
    tool: &str,
) -> (PreparedExecution, Arc<ScriptedProvider>, RigAgentConfig) {
    let provider = Arc::new(ScriptedProvider {
        tool: tool.into(),
        calls: Mutex::new(Vec::new()),
    });
    let mut config = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    config.mcps = vec!["authorized.server".into()];
    let prepared = PreparedExecution::new(
        config,
        provider.clone(),
        Arc::new(ToolRegistry::new()),
        manager,
        Arc::new(MiddlewarePipeline::new()),
    );
    let rig = RigAgentConfig::new(
        "actor",
        "Actor",
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
    (prepared, provider, rig)
}
struct FakeClient {
    calls: Arc<AtomicUsize>,
    discovery: Arc<AtomicUsize>,
    fail: bool,
    raw_names: Vec<String>,
}
#[async_trait::async_trait]
impl McpClient for FakeClient {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn call_tool(&self, name: &str, _: Value) -> Result<Value, McpError> {
        assert_eq!(name, "raw.echo");
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"content":[{"type":"text","text":"fake-visible"}]}))
    }
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        self.discovery.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(McpError::ProtocolError("fixture failure".into()));
        }
        Ok(self
            .raw_names
            .iter()
            .map(|name| McpTool {
                name: name.clone(),
                description: "fixture".into(),
                parameters: Some(json!({"value":{"type":"string"}})),
            })
            .collect())
    }
}
async fn insert(
    manager: &McpManager,
    id: &str,
    fail: bool,
    names: &[&str],
) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let discovery = Arc::new(AtomicUsize::new(0));
    manager
        .insert_test_client(
            id,
            Arc::new(FakeClient {
                calls: calls.clone(),
                discovery: discovery.clone(),
                fail,
                raw_names: names.iter().map(|name| (*name).into()).collect(),
            }),
        )
        .await;
    (calls, discovery)
}
#[tokio::test]
async fn production_rig_turn_calls_namespaced_stdio_tool_with_original_identity() {
    let manager = Arc::new(McpManager::new());
    let config:McpServerConfig=serde_json::from_value(json!({"type":"stdio","id":"authorized.server","name":"fixture","description":"fixture","command":"python3","args":["-u",concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/mcp_stdio_probe.py")],"enabled":true})).unwrap();
    manager.start_server(config).await.unwrap();
    let (mut prepared, provider, rig) = prepare(manager, "authorized_server__echo");
    prepared.resolve_mcp_tools().await.unwrap();
    let engine = build_engine(prepared, rig);
    assert_eq!(engine.execute("call echo", &[]).await.unwrap(), "done");
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let schema = &calls[0].1.as_ref().unwrap()[0]["function"];
    assert_eq!(schema["name"], "authorized_server__echo");
    assert_eq!(schema["parameters"]["additionalProperties"], false);
    assert!(
        calls[1]
            .0
            .iter()
            .any(|message| message.role == "tool"
                && message.text_content().contains("native-visible"))
    );
}
#[tokio::test]
async fn mcp_registration_denies_unconfigured_hidden_disabled_and_planning_blocked_tools() {
    for mode in ["unconfigured", "hidden", "disabled", "planning"] {
        let manager = Arc::new(McpManager::new());
        let (calls, _) = insert(&manager, "authorized.server", false, &["raw.echo"]).await;
        let (other_calls, other_discovery) =
            insert(&manager, "unconfigured", false, &["raw.echo"]).await;
        let selected = if mode == "unconfigured" {
            "unconfigured__raw_echo"
        } else {
            "authorized_server__raw_echo"
        };
        let (mut prepared, provider, rig) = prepare(manager, selected);
        if mode == "hidden" {
            prepared.config.model_hidden_tools.insert(selected.into());
        }
        if mode == "disabled" {
            prepared.config.tools_enabled = false;
        }
        if mode == "planning" {
            prepared.config.initial_state.insert(
                agent_tools::guards::PLANNING_GATE_STATE.into(),
                serde_json::to_value(agent_tools::guards::PlanningGate::awaiting_ward(
                    "plan first",
                ))
                .unwrap(),
            );
        }
        prepared.resolve_mcp_tools().await.unwrap();
        let engine = build_engine(prepared, rig);
        let _ = engine.execute("try tool", &[]).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0, "mode {mode}");
        assert_eq!(other_calls.load(Ordering::SeqCst), 0);
        assert_eq!(other_discovery.load(Ordering::SeqCst), 0);
        let requests = provider.calls.lock().unwrap();
        assert!(!requests[0]
            .1
            .as_ref()
            .unwrap_or(&Value::Null)
            .to_string()
            .contains("unconfigured"));
        if mode == "planning" {
            assert!(requests.iter().any(|(messages, _)| messages
                .iter()
                .any(|message| message.role == "tool"
                    && message.text_content().contains("planner-agent"))));
        }
    }
}
#[tokio::test]
async fn configured_binding_dispatches_raw_name_and_discovery_failure_is_removed_once() {
    let notices = Arc::new(AtomicUsize::new(0));
    let counter = notices.clone();
    let manager = Arc::new(
        McpManager::new().with_startup_failure_observer(Arc::new(move |id| {
            assert_eq!(id, "failed");
            counter.fetch_add(1, Ordering::SeqCst);
        })),
    );
    let (calls, _) = insert(&manager, "authorized.server", false, &["raw.echo"]).await;
    let (_, discovery) = insert(&manager, "failed", true, &[]).await;
    let (mut prepared, _, rig) = prepare(manager.clone(), "authorized_server__raw_echo");
    prepared.config.mcps.push("failed".into());
    prepared.resolve_mcp_tools().await.unwrap();
    prepared.resolve_mcp_tools().await.unwrap();
    assert!(manager.get_client("failed").await.is_none());
    assert_eq!(notices.load(Ordering::SeqCst), 1);
    assert_eq!(discovery.load(Ordering::SeqCst), 1);
    build_engine(prepared, rig)
        .execute("call tool", &[])
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn ambiguous_normalized_names_fail_closed() {
    let manager = Arc::new(McpManager::new());
    let (calls, _) = insert(
        &manager,
        "authorized.server",
        false,
        &["raw.echo", "raw_echo"],
    )
    .await;
    let (mut prepared, _, _) = prepare(manager, "authorized_server__raw_echo");
    assert!(prepared.resolve_mcp_tools().await.is_err());
    assert!(prepared.model_visible_tools().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn prepared_mcp_binding_does_not_follow_manager_replacement() {
    let manager = Arc::new(McpManager::new());
    let (original_calls, _) = insert(&manager, "authorized.server", false, &["raw.echo"]).await;
    let (mut prepared, _, rig) = prepare(manager.clone(), "authorized_server__raw_echo");
    prepared.resolve_mcp_tools().await.unwrap();
    let (replacement_calls, _) = insert(&manager, "authorized.server", false, &["raw.echo"]).await;

    build_engine(prepared, rig)
        .execute("call the prepared tool", &[])
        .await
        .unwrap();
    assert_eq!(original_calls.load(Ordering::SeqCst), 1);
    assert_eq!(replacement_calls.load(Ordering::SeqCst), 0);
}
