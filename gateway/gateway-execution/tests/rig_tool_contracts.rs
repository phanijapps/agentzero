//! Effectful tool contracts through production prepared inputs and the Rig loop.
#[cfg(unix)]
#[path = "rig_tool_contracts/context.rs"]
mod context;
use agent_primitives::{FileSystemContext, Tool};
use agent_runtime::llm::{ChatResponse, LlmError, StreamCallback, StreamChunk};
use agent_runtime::{
    AgentEngine, ChatMessage, ExecutorConfig, LlmClient, LlmConfig, McpManager, MiddlewarePipeline,
    PreparedExecution, RigAgentConfig, RigModelConfig, ToolRegistry,
};
use gateway_execution::config::GatewayFileSystem;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Script {
    steps: Vec<(&'static str, Value)>,
    requests: Mutex<Vec<Vec<ChatMessage>>>,
}
#[async_trait::async_trait]
impl LlmClient for Script {
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
        _: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let mut requests = self.requests.lock().unwrap();
        let index = requests.len();
        requests.push(messages);
        let calls = self.steps.get(index).map(|(name, args)| {
            vec![agent_runtime::types::ToolCall::new(
                format!("call-{index}"),
                (*name).into(),
                args.clone(),
            )]
        });
        if calls.is_none() {
            callback(StreamChunk::Token("done".into()));
        }
        Ok(ChatResponse {
            content: if calls.is_none() {
                "done".into()
            } else {
                String::new()
            },
            tool_calls: calls,
            reasoning: None,
            usage: None,
        })
    }
}
async fn run(
    tools: Vec<Arc<dyn Tool>>,
    steps: Vec<(&'static str, Value)>,
    state: HashMap<String, Value>,
    manager: Arc<McpManager>,
    mcps: Vec<String>,
) -> Vec<String> {
    let provider = Arc::new(Script {
        steps,
        requests: Mutex::new(Vec::new()),
    });
    let mut registry = ToolRegistry::new();
    registry.register_all(tools);
    let mut config = ExecutorConfig::new("host-agent".into(), "fixture".into(), "fixture".into());
    config.conversation_id = Some("host-session".into());
    config.initial_state = state;
    config.mcps = mcps;
    let mut prepared = PreparedExecution::new(
        config,
        provider.clone(),
        Arc::new(registry),
        manager,
        Arc::new(MiddlewarePipeline::new()),
    );
    prepared.resolve_mcp_tools().await.unwrap();
    let rig = RigAgentConfig::new(
        "host-agent",
        "Host",
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
    let engine = agent_runtime::rig_adapter::factory::build_engine(prepared, rig);
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        engine.execute("execute fixture", &[]),
    )
    .await
    .expect("bounded Rig run");
    let requests = provider.requests.lock().unwrap();
    let mut results: Vec<_> = requests
        .last()
        .unwrap()
        .iter()
        .filter(|m| m.role == "tool")
        .map(ChatMessage::text_content)
        .collect();
    if let Err(error) = outcome {
        results.push(format!("error: {error}"));
    }
    results
}
fn fs(dir: &tempfile::TempDir) -> Arc<dyn FileSystemContext> {
    Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()))
}
fn state() -> HashMap<String, Value> {
    HashMap::from([("ward_id".into(), json!("host-ward"))])
}

#[cfg(unix)]
#[tokio::test]
async fn shell_uses_configured_vault_environment_and_explicit_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        vec![Arc::new(
            agent_tools::ShellTool::new().with_filesystem(fs(&dir)),
        )],
        vec![(
            "shell",
            json!({"command":"printf '%s' \"$VIRTUAL_ENV\"","cwd":dir.path()}),
        )],
        state(),
        Arc::new(McpManager::new()),
        vec![],
    )
    .await;
    assert!(
        output
            .join("\n")
            .contains(&dir.path().join("wards/.venv").display().to_string()),
        "shell environment must use configured vault"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn shell_timeout_releases_its_launched_process() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("pending.py");
    let pidfile = dir.path().join("pid");
    std::fs::write(
        &script,
        format!(
            "import os,time\nopen({:?},'w').write(str(os.getpid()))\ntime.sleep(60)\n",
            pidfile
        ),
    )
    .unwrap();
    let output=run(vec![Arc::new(agent_tools::ShellTool::new().with_filesystem(fs(&dir)))],vec![("shell",json!({"command":format!("exec python3 {}",script.display()),"cwd":dir.path(),"timeout_seconds":1}))],state(),Arc::new(McpManager::new()),vec![]).await;
    assert!(output.join("\n").contains("timed out"));
    let pid = std::fs::read_to_string(&pidfile).unwrap();
    let procpath = format!("/proc/{}", pid.trim());
    let gone = tokio::time::timeout(Duration::from_secs(2), async {
        while std::path::Path::new(&procpath).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    if !gone {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", pid.trim()])
            .status();
    }
    assert!(gone, "timed-out shell must release its process");
}

#[tokio::test]
async fn skill_reads_reject_traversal_and_symlink_escape_through_rig() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("skills/alpha")).unwrap();
    std::fs::create_dir_all(dir.path().join("outside")).unwrap();
    std::fs::write(
        dir.path().join("skills/alpha/SKILL.md"),
        "# Alpha\nUseful instructions",
    )
    .unwrap();
    std::fs::write(dir.path().join("skills/private.txt"), "OUTSIDE-CANARY").unwrap();
    std::fs::write(dir.path().join("outside/SKILL.md"), "OUTSIDE-CANARY").unwrap();
    let mut cases = vec![
        json!({"file":"@skill:alpha/../private.txt"}),
        json!({"skill":"../outside"}),
    ];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            dir.path().join("outside/SKILL.md"),
            dir.path().join("skills/alpha/link.txt"),
        )
        .unwrap();
        cases.push(json!({"file":"@skill:alpha/link.txt"}));
    }
    for args in cases {
        let result = run(
            vec![Arc::new(agent_tools::LoadSkillTool::new(fs(&dir)))],
            vec![("load_skill", args)],
            state(),
            Arc::new(McpManager::new()),
            vec![],
        )
        .await;
        assert!(
            !result.join("\n").contains("OUTSIDE-CANARY"),
            "skill reads remain inside configured skill directory"
        );
    }
}

struct Endpoint {
    url: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Endpoint {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let (header, body) = loop {
                    let mut part = [0; 4096];
                    let n = socket.read(&mut part).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&part[..n]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
                        let len = header
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + len {
                            break (header, bytes[end + 4..end + 4 + len].to_vec());
                        }
                    }
                    assert!(bytes.len() < 65536);
                };
                let body = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                let failed = header.lines().next().unwrap().contains(" /error ");
                if failed {
                    assert!(header
                        .to_ascii_lowercase()
                        .contains("authorization: bearer connector-error-canary"));
                }
                captured
                    .lock()
                    .unwrap()
                    .push((header.lines().next().unwrap().to_string(), body));
                let response = if failed {
                    r#"{"error":"connector-error-canary"}"#
                } else {
                    r#"{"result":"connector-visible"}"#
                };
                let status = if failed {
                    "500 Internal Server Error"
                } else {
                    "200 OK"
                };
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}
#[tokio::test]
async fn connector_dispatch_uses_fixed_resources_host_identity_and_enabled_state() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::start().await;
    let registry = Arc::new(gateway_connectors::ConnectorRegistry::new(
        gateway_connectors::ConnectorService::new(Arc::new(gateway_services::VaultPaths::new(
            dir.path().to_path_buf(),
        ))),
    ));
    registry.create(serde_json::from_value(json!({"id":"configured","name":"Configured","transport":{"type":"http","callback_url":format!("{}/invoke",endpoint.url)},"metadata":{"resources":[{"name":"fixed","uri":format!("{}/fixed",endpoint.url)}],"capabilities":[{"name":"send","schema":{"type":"object"}}]}})).unwrap()).await.unwrap();
    let provider = Arc::new(gateway_execution::GatewayResourceProvider::new(
        registry.clone(),
    ));
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(agent_tools::ConnectorResourceTool::new(provider.clone())),
        Arc::new(agent_tools::ConnectorInvokeTool::new(provider)),
    ];
    let result=run(tools.clone(),vec![("connector_resource",json!({"action":"query","connector_id":"configured","resource":"fixed","url":"http://forged.invalid"})),("connector_invoke",json!({"connector_id":"configured","capability":"send","payload":{"message":"hello"},"session_id":"forged","agent_id":"forged"}))],state(),Arc::new(McpManager::new()),vec![]).await;
    assert!(result.join("\n").contains("connector-visible"));
    {
        let requests = endpoint.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].0.starts_with("GET /fixed "));
        assert!(requests[1].0.starts_with("POST /invoke "));
        assert_eq!(requests[1].1["context"]["session_id"], "host-session");
        assert_eq!(requests[1].1["context"]["agent_id"], "host-agent");
    }
    for (name, args) in [
        (
            "connector_resource",
            json!({"action":"query","connector_id":"unknown","resource":"fixed"}),
        ),
        (
            "connector_resource",
            json!({"action":"query","connector_id":"configured","resource":"unknown"}),
        ),
        (
            "connector_invoke",
            json!({"connector_id":"unknown","capability":"send","payload":{}}),
        ),
        (
            "connector_invoke",
            json!({"connector_id":"configured","capability":"unknown","payload":{}}),
        ),
    ] {
        let denied = run(
            tools.clone(),
            vec![(name, args)],
            state(),
            Arc::new(McpManager::new()),
            vec![],
        )
        .await;
        assert!(denied.join("\n").contains("not found"));
        assert_eq!(endpoint.requests.lock().unwrap().len(), 2);
    }
    registry
        .update(
            "configured",
            gateway_connectors::UpdateConnectorRequest {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for (name, args) in [
        (
            "connector_resource",
            json!({"action":"query","connector_id":"configured","resource":"fixed"}),
        ),
        (
            "connector_invoke",
            json!({"connector_id":"configured","capability":"send","payload":{}}),
        ),
    ] {
        let denied = run(
            tools.clone(),
            vec![(name, args)],
            state(),
            Arc::new(McpManager::new()),
            vec![],
        )
        .await;
        assert!(denied.join("\n").contains("disabled"));
    }
    assert_eq!(
        endpoint.requests.lock().unwrap().len(),
        2,
        "disabled and unknown dispatch must not contact downstream"
    );
    registry.create(serde_json::from_value(json!({"id":"failed","name":"Failed","transport":{"type":"http","callback_url":format!("{}/error",endpoint.url),"headers":{"Authorization":"Bearer connector-error-canary"}},"metadata":{"resources":[{"name":"fixed","uri":format!("{}/error",endpoint.url)}],"capabilities":[{"name":"send","schema":{"type":"object"}}]}})).unwrap()).await.unwrap();
    for (name, args) in [
        (
            "connector_resource",
            json!({"action":"query","connector_id":"failed","resource":"fixed"}),
        ),
        (
            "connector_invoke",
            json!({"connector_id":"failed","capability":"send","payload":{}}),
        ),
    ] {
        use tracing::instrument::WithSubscriber;
        let logs = Capture::default();
        let output = run(
            tools.clone(),
            vec![(name, args)],
            state(),
            Arc::new(McpManager::new()),
            vec![],
        )
        .with_subscriber(logs.clone())
        .await;
        assert!(output.join("\n").contains("500"));
        assert!(!output.join("\n").contains("connector-error-canary"));
        let logs = logs.0.lock().unwrap().join("\n");
        assert!(!logs.contains("connector-error-canary"));
        if name == "connector_invoke" {
            assert!(logs.contains("non-success status"));
        }
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);
impl tracing::field::Visit for Capture {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.lock().unwrap().push(format!("{field}={value:?}"));
    }
}
impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        event.record(&mut self.clone());
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

struct StateProbe;
#[async_trait::async_trait]
impl Tool for StateProbe {
    fn name(&self) -> &str {
        "inspect_fixture_state"
    }
    fn description(&self) -> &str {
        "Inspect host fixture state"
    }
    async fn execute(
        &self,
        ctx: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> agent_primitives::Result<Value> {
        Ok(
            json!({"loaded":ctx.get_state("skill:loaded_skills"),"graph":ctx.get_state("skill:graph"),"ward":ctx.get_state("ward_id")}),
        )
    }
}
#[cfg(unix)]
#[tokio::test]
async fn one_rig_run_combines_builtins_mcp_and_bounded_skill_packets_with_shared_host_state() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("skills/alpha")).unwrap();
    let body = format!(
        "# Alpha\n{}\n## Details\n{}",
        "bounded instructions ".repeat(300),
        "more details ".repeat(300)
    );
    std::fs::write(dir.path().join("skills/alpha/SKILL.md"), &body).unwrap();
    std::fs::write(
        dir.path().join("skills/alpha/reference.txt"),
        "skill-resource-visible",
    )
    .unwrap();
    let manager = Arc::new(McpManager::new());
    manager.start_server(serde_json::from_value(json!({"type":"stdio","id":"fixture","name":"Fixture","description":"fixture","command":"python3","args":["-u",std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/agent-runtime/tests/fixtures/mcp_stdio_probe.py")],"enabled":true})).unwrap()).await.unwrap();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(agent_tools::LoadSkillTool::new(fs(&dir))),
        Arc::new(agent_tools::ShellTool::new().with_filesystem(fs(&dir))),
        Arc::new(agent_tools::WriteFileTool::new(fs(&dir))),
        Arc::new(StateProbe),
    ];
    let output = run(
        tools,
        vec![
            ("load_skill", json!({"skill":"alpha"})),
            ("load_skill", json!({"file":"reference.txt"})),
            ("fixture__echo", json!({"value":"mcp-visible"})),
            ("shell", json!({"command":"pwd","ward_id":"forged-ward"})),
            (
                "write_file",
                json!({"path":"artifact.txt","content":"host-ward-output","ward_id":"forged-ward"}),
            ),
            (
                "write_file",
                json!({"path":"../escape.txt","content":"must-not-write"}),
            ),
            ("inspect_fixture_state", json!({})),
        ],
        state(),
        manager,
        vec!["fixture".into()],
    )
    .await;
    assert_eq!(output.len(), 7);
    let packet: Value = serde_json::from_str(&output[0]).unwrap();
    assert!(packet.get("instructions").is_none());
    assert!(output[0].len() < body.len());
    assert_eq!(packet["packet"]["render_policy"], "summary");
    assert!(packet["packet"]["sections"].as_array().unwrap().len() <= 6);
    assert!(output[1].contains("skill-resource-visible"));
    assert!(output[2].contains("mcp-visible"));
    assert!(output[3].contains(&dir.path().join("wards/host-ward").display().to_string()));
    assert!(!dir.path().join("wards/forged-ward").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("wards/host-ward/artifact.txt")).unwrap(),
        "host-ward-output"
    );
    assert!(!dir.path().join("wards/escape.txt").exists());
    let state: Value = serde_json::from_str(&output[6]).unwrap();
    assert_eq!(state["loaded"], json!(["alpha"]));
    assert_eq!(state["graph"]["alpha"]["tool_call_id"], "call-0");
    assert_eq!(state["ward"], "host-ward");
}
