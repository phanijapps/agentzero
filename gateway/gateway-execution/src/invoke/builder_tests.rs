use super::*;
use super::{
    build_context_capability_catalog, build_runtime_middleware_pipeline, collect_agents_summary,
    resolve_effective_max_input, ExecutorBuilder,
};
use crate::agent_pool::AgentResultBus;
use crate::config::GatewayFileSystem;
use crate::invoke::executor::build_execution_engine;
use crate::invoke::policy::RuntimeActorKind;
use crate::invoke::tool_catalog::{default_visible, split_target, visibility_policy};
use agent_primitives::connectors::{CapabilityInfo, ConnectorInfo, ResourceInfo};
use agent_primitives::vault_paths::SharedVaultPaths;
use agent_primitives::FileSystemContext;
use agent_primitives::{Tool, ToolContext as ToolContextTrait};
use agent_runtime::llm::{ChatResponse, LlmError, StreamCallback};
use agent_runtime::tools::ToolContext as RuntimeToolContext;
use agent_runtime::LlmClient;
use agent_runtime::{
    ContextActorKind, ContextCapabilityCatalog, ContextCapabilityHealth, ContextCapabilityKind,
    ContextLatencyHint, ContextRiskLevel, ContextSideEffects,
};
use agent_tools::{RecallVisibilityScope, WriteFileTool};
use agent_tools::{ToolSettings, WardTool};
use async_trait::async_trait;
use execution_state::StateService;
use gateway_services::models::DEFAULT_MAX_INPUT_TOKENS;
use gateway_services::McpService;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use zbot_runtime_sqlite::DatabaseManager;

struct StubSummaryClient;

struct SurfaceJourneyLlm {
    calls: Arc<AtomicUsize>,
    arguments: Value,
}

#[async_trait]
impl LlmClient for SurfaceJourneyLlm {
    fn model(&self) -> &str {
        "surface-journey"
    }

    fn provider(&self) -> &str {
        "test"
    }

    async fn chat(
        &self,
        _messages: Vec<agent_runtime::ChatMessage>,
        _tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        unreachable!()
    }

    async fn chat_stream(
        &self,
        _messages: Vec<agent_runtime::ChatMessage>,
        _tools: Option<Value>,
        _callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: Some(vec![agent_runtime::ToolCall::new(
                    "surface-call".to_string(),
                    "present_surface".to_string(),
                    self.arguments.clone(),
                )]),
                reasoning: None,
                usage: None,
            })
        } else {
            Ok(ChatResponse {
                content: "canonical answer".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
    }
}

struct MockConnectorProvider;

#[tokio::test]
async fn available_agents_include_existing_wards_as_virtual_agents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths: SharedVaultPaths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    std::fs::create_dir_all(paths.wards_dir().join("financial-analysis")).unwrap();
    std::fs::create_dir_all(paths.wards_dir().join(".hidden")).unwrap();
    let service = gateway_services::AgentService::new(paths.agents_dir());

    let summaries = collect_agents_summary(&service, &paths).await;

    assert!(summaries.iter().any(|agent| {
        agent["id"] == "ward:financial-analysis"
            && agent["name"] == "Ward Agent: financial-analysis"
    }));
    assert!(!summaries.iter().any(|agent| agent["id"] == "ward:.hidden"));
}

#[async_trait]
impl ConnectorResourceProvider for MockConnectorProvider {
    async fn list_connectors(&self) -> std::result::Result<Vec<ConnectorInfo>, String> {
        Ok(vec![ConnectorInfo {
            id: "signal".to_string(),
            name: "Signal Bridge".to_string(),
            resources: vec![ResourceInfo {
                name: "aliases".to_string(),
                uri: "http://localhost/aliases".to_string(),
                method: "GET".to_string(),
                description: Some("List aliases".to_string()),
            }],
            capabilities: vec![CapabilityInfo {
                name: "send_message".to_string(),
                schema: serde_json::json!({"type": "object"}),
                description: Some("Send message".to_string()),
            }],
        }])
    }

    async fn query_resource(
        &self,
        _connector_id: &str,
        _resource_name: &str,
        _params: Option<HashMap<String, String>>,
    ) -> std::result::Result<Value, String> {
        Ok(serde_json::json!([]))
    }

    async fn invoke_capability(
        &self,
        _connector_id: &str,
        _capability: &str,
        _payload: Value,
        _session_id: &str,
        _agent_id: &str,
    ) -> std::result::Result<Value, String> {
        Ok(serde_json::json!({"ok": true}))
    }
}

#[async_trait]
impl LlmClient for StubSummaryClient {
    fn model(&self) -> &str {
        "stub"
    }

    fn provider(&self) -> &str {
        "stub"
    }

    async fn chat(
        &self,
        _messages: Vec<agent_runtime::ChatMessage>,
        _tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        Ok(ChatResponse {
            content: "summary".to_string(),
            tool_calls: None,
            reasoning: None,
            usage: None,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<agent_runtime::ChatMessage>,
        tools: Option<Value>,
        _callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        self.chat(messages, tools).await
    }
}

#[test]
fn runtime_middleware_order_keeps_context_editing_before_plan_block() {
    let pipeline = build_runtime_middleware_pipeline(100_000, true, None);
    assert_eq!(
        pipeline.pre_processor_names(),
        vec!["context_editing", "plan_block"]
    );
}

#[test]
fn runtime_middleware_order_puts_enabled_summarization_after_plan_block() {
    let summary_client = Arc::new(StubSummaryClient);
    let pipeline = build_runtime_middleware_pipeline(100_000, true, Some(summary_client));
    assert_eq!(
        pipeline.pre_processor_names(),
        vec!["context_editing", "plan_block", "summarization"]
    );
}

#[test]
fn runtime_middleware_order_keeps_plan_block_when_context_window_unknown() {
    let pipeline = build_runtime_middleware_pipeline(0, false, None);
    assert_eq!(pipeline.pre_processor_names(), vec!["plan_block"]);
}

fn sample_agent() -> Agent {
    Agent {
        id: "agent-1".to_string(),
        name: "code-agent".to_string(),
        display_name: "Code Agent".to_string(),
        description: "Writes code".to_string(),
        agent_type: Some("specialist".to_string()),
        provider_id: "provider-1".to_string(),
        model: "gpt-test".to_string(),
        temperature: 0.25,
        max_input_tokens: 64_000,
        max_input_tokens_explicit: true,
        max_tokens: 4_096,
        thinking_enabled: true,
        voice_recording_enabled: false,
        system_instruction: None,
        instructions: "Follow the project rules.".to_string(),
        mcps: vec!["filesystem".to_string()],
        skills: vec!["rust".to_string()],
        middleware: None,
        created_at: None,
    }
}

fn sample_provider() -> Provider {
    Provider {
        id: Some("provider-1".to_string()),
        name: "Provider One".to_string(),
        description: "OpenAI-compatible test provider".to_string(),
        api_key: "sk-test".to_string(),
        base_url: "http://localhost:9999/v1".to_string(),
        models: vec!["gpt-test".to_string()],
        embedding_models: None,
        embedding_dimensions: None,
        verified: None,
        is_default: true,
        created_at: None,
        max_concurrent_requests: None,
        context_window: Some(32_000),
        default_model: Some("gpt-test".to_string()),
        rate_limits: None,
        model_configs: Some(HashMap::from([(
            "gpt-test".to_string(),
            gateway_services::providers::ModelConfig {
                capabilities: gateway_services::models::ModelCapabilities::default(),
                max_input: Some(24_576),
                max_output: Some(2_048),
                source: "user".to_string(),
            },
        )])),
    }
}

#[tokio::test]
async fn root_executor_uses_the_effective_active_ward_as_tool_context() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "financial-analysis")
        .expect("template ward");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build(
            &agent,
            &provider,
            "conversation-1",
            "session-1",
            &[],
            &[],
            None,
            &mcp_service,
            Some("financial-analysis"),
        )
        .await
        .expect("executor build");

    assert_eq!(
        executor.config().initial_state.get("ward_id"),
        Some(&serde_json::Value::String("financial-analysis".to_string()))
    );
    let packet = executor
        .config()
        .initial_state
        .get("ward_template")
        .expect("root template packet");
    assert_eq!(packet["status"], "available");
    assert_eq!(packet["session_id"], "session-1");
    assert_eq!(packet["ward_id"], "financial-analysis");
    let instruction = executor.config().system_instruction.as_deref().unwrap();
    assert!(instruction.starts_with(&agent.instructions));
    assert!(instruction.contains("# Active Ward Template"));
    assert!(instruction.contains(packet["digest"].as_str().unwrap()));
    assert!(!executor
        .config()
        .initial_state
        .contains_key("ward_lint_report"));
}

#[tokio::test]
async fn invalid_template_is_non_terminal_and_not_injected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "broken").expect("ward");
    std::fs::write(paths.ward_layout_snapshot("broken"), "not: [valid").expect("corrupt snapshot");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build(
            &agent,
            &sample_provider(),
            "conversation-broken",
            "session-broken",
            &[],
            &[],
            None,
            &mcp_service,
            Some("broken"),
        )
        .await
        .expect("template failure must not stop root orchestration");

    assert_eq!(
        executor.config().initial_state["ward_template"]["status"],
        "unavailable"
    );
    assert_eq!(
        executor.config().system_instruction.as_deref(),
        Some(agent.instructions.as_str())
    );
}

#[tokio::test]
async fn delegated_executor_receives_no_template_packet_or_prompt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "existing").expect("ward");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-child",
            "session-child",
            &[],
            &[],
            None,
            &mcp_service,
            Some("existing"),
        )
        .await
        .expect("delegated executor");

    assert!(!executor
        .config()
        .initial_state
        .contains_key("ward_template"));
    assert_eq!(
        executor.config().system_instruction.as_deref(),
        Some(agent.instructions.as_str())
    );
}

#[tokio::test]
async fn planner_executor_receives_selected_template_packet_and_prompt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "history-library").expect("ward");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.id = "planner-agent".to_string();
    agent.mcps.clear();
    agent.skills.clear();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-planner",
            "session-planner",
            &[],
            &[],
            None,
            &mcp_service,
            Some("history-library"),
        )
        .await
        .expect("planner executor");

    let packet = executor
        .config()
        .initial_state
        .get("ward_template")
        .expect("planner template packet");
    assert_eq!(packet["status"], "available");
    assert_eq!(packet["session_id"], "session-planner");
    assert_eq!(packet["ward_id"], "history-library");
    let instruction = executor.config().system_instruction.as_deref().unwrap();
    let expected =
        crate::invoke::ward_layout_adapter::GatewayWardLayoutAccess::new(dir.path().to_path_buf())
            .state(
                "history-library",
                "session-planner",
                packet["root_context_id"].as_str().unwrap(),
            )
            .context
            .expect("expected adapter context");
    assert_eq!(
        instruction.strip_prefix(&format!("{}\n\n", agent.instructions)),
        Some(expected.as_str())
    );
    assert!(instruction.contains(packet["digest"].as_str().unwrap()));
    assert!(instruction.contains("spec.md"));
    assert!(instruction.contains("plan.md"));
    assert!(!instruction.contains("apiVersion:"));
    assert!(!instruction.contains(dir.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn planner_refinement_tool_replay_persists_required_roles_only_in_selected_ward() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    for ward in ["history-library", "sibling-library"] {
        gateway_services::create_ward_from_template(&paths, ward).expect("ward");
    }
    let mcp_service = McpService::new(paths.clone());
    let mut agent = sample_agent();
    agent.id = "planner-agent".to_string();
    agent.mcps.clear();
    agent.skills.clear();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-planner",
            "session-planner",
            &[],
            &[],
            None,
            &mcp_service,
            Some("history-library"),
        )
        .await
        .expect("planner executor");
    let ctx: Arc<dyn ToolContextTrait> = Arc::new(RuntimeToolContext::full_with_state(
        "planner-agent".to_string(),
        Some("conversation-planner".to_string()),
        Vec::new(),
        executor.config().initial_state.clone(),
    ));
    let fs: Arc<dyn FileSystemContext> = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let writer = WriteFileTool::new(fs.clone());
    for (path, content) in [
        (
            ".zbot/specs/discovery-of-india/spec.md",
            "# Specification\n",
        ),
        (".zbot/specs/discovery-of-india/plan.md", "# Plan\n"),
    ] {
        writer
            .execute(
                ctx.clone(),
                serde_json::json!({"path":path,"content":content}),
            )
            .await
            .expect("planner write");
    }

    let ward_tool = WardTool::new(
        fs,
        None,
        None,
        Arc::new(
            crate::invoke::ward_layout_adapter::GatewayWardLayoutAccess::new(
                dir.path().to_path_buf(),
            ),
        ),
    );
    let lint = ward_tool
        .execute(
            ctx,
            serde_json::json!({"action":"lint","name":"history-library"}),
        )
        .await
        .expect("planner lint");
    assert_eq!(lint["ok"], true);
    assert_eq!(lint["data"]["valid"], true);

    let selected = paths
        .wards_dir()
        .join("history-library/.zbot/specs/discovery-of-india");
    assert!(selected.join("spec.md").is_file());
    assert!(selected.join("plan.md").is_file());
    assert!(!selected.join("tasks").exists());
    assert!(!paths
        .wards_dir()
        .join("sibling-library/.zbot/specs/discovery-of-india")
        .exists());
}

#[tokio::test]
async fn planner_no_persistent_role_replay_remains_ephemeral() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "ephemeral-only").expect("ward");
    std::fs::write(
        paths.ward_layout_snapshot("ephemeral-only"),
        r#"apiVersion: zbot.dev/v1alpha1
kind: WardLayout
root:
  kind: directory
  children:
    - { id: canonical, match: '{ward}.md', kind: file, format: markdown }
    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }
    - { id: log, match: log.md, kind: file, format: markdown }
extensions: {}
"#,
    )
    .expect("role-free template");
    let loaded = gateway_services::load_ward_layout(&paths.ward_layout_snapshot("ephemeral-only"))
        .expect("load role-free template");
    gateway_services::CompiledWardLayout::compile(&loaded.document)
        .expect("compile role-free template");
    let mcp_service = McpService::new(paths.clone());
    let mut agent = sample_agent();
    agent.id = "planner-agent".to_string();
    agent.mcps.clear();
    agent.skills.clear();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-ephemeral",
            "session-ephemeral",
            &[],
            &[],
            None,
            &mcp_service,
            Some("ephemeral-only"),
        )
        .await
        .expect("planner executor");
    let instruction = executor.config().system_instruction.as_deref().unwrap();
    assert!(!instruction.contains("spec.md"));
    assert!(!instruction.contains("plan.md"));

    let ctx: Arc<dyn ToolContextTrait> = Arc::new(RuntimeToolContext::full_with_state(
        "planner-agent".to_string(),
        Some("conversation-ephemeral".to_string()),
        Vec::new(),
        executor.config().initial_state.clone(),
    ));
    let fs: Arc<dyn FileSystemContext> = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let lint = WardTool::new(
        fs,
        None,
        None,
        Arc::new(
            crate::invoke::ward_layout_adapter::GatewayWardLayoutAccess::new(
                dir.path().to_path_buf(),
            ),
        ),
    )
    .execute(
        ctx,
        serde_json::json!({"action":"lint","name":"ephemeral-only"}),
    )
    .await
    .expect("planner lint");
    assert_eq!(lint["data"]["valid"], true);
    assert!(
        !paths.wards_dir().join("ephemeral-only/.zbot").exists(),
        "a role-free template must not acquire fallback planning artifacts"
    );
}

#[tokio::test]
async fn planner_missing_plan_role_replay_does_not_invent_a_fallback_plan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "spec-only").expect("ward");
    std::fs::write(
        paths.ward_layout_snapshot("spec-only"),
        r#"apiVersion: zbot.dev/v1alpha1
kind: WardLayout
definitions:
  specification:
    kind: directory
    children:
      - { id: specification, match: spec.md, kind: file, format: markdown }
root:
  kind: directory
  children:
    - { id: canonical, match: '{ward}.md', kind: file, format: markdown }
    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }
    - { id: log, match: log.md, kind: file, format: markdown }
    - id: zbot
      match: .zbot
      kind: directory
      required: false
      children:
        - id: specifications
          match: specs
          kind: directory
          required: false
          children:
            - { id: specification-node, match: '*', required: false, repeat: true, $ref: specification }
extensions: {}
"#,
    )
    .expect("spec-only template");
    let loaded = gateway_services::load_ward_layout(&paths.ward_layout_snapshot("spec-only"))
        .expect("load spec-only template");
    gateway_services::CompiledWardLayout::compile(&loaded.document)
        .expect("compile spec-only template");
    let mcp_service = McpService::new(paths.clone());
    let mut agent = sample_agent();
    agent.id = "planner-agent".to_string();
    agent.mcps.clear();
    agent.skills.clear();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-spec-only",
            "session-spec-only",
            &[],
            &[],
            None,
            &mcp_service,
            Some("spec-only"),
        )
        .await
        .expect("planner executor");
    let instruction = executor.config().system_instruction.as_deref().unwrap();
    assert!(instruction.contains("spec.md"));
    assert!(!instruction.contains("plan.md"));

    let ctx: Arc<dyn ToolContextTrait> = Arc::new(RuntimeToolContext::full_with_state(
        "planner-agent".to_string(),
        Some("conversation-spec-only".to_string()),
        Vec::new(),
        executor.config().initial_state.clone(),
    ));
    let fs: Arc<dyn FileSystemContext> = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    WriteFileTool::new(fs.clone())
        .execute(
            ctx.clone(),
            serde_json::json!({
                "path":".zbot/specs/discovery-of-india/spec.md",
                "content":"# Specification\n"
            }),
        )
        .await
        .expect("declared spec write");
    let lint = WardTool::new(
        fs,
        None,
        None,
        Arc::new(
            crate::invoke::ward_layout_adapter::GatewayWardLayoutAccess::new(
                dir.path().to_path_buf(),
            ),
        ),
    )
    .execute(ctx, serde_json::json!({"action":"lint","name":"spec-only"}))
    .await
    .expect("planner lint");
    assert_eq!(lint["data"]["valid"], true);
    let refinement = paths
        .wards_dir()
        .join("spec-only/.zbot/specs/discovery-of-india");
    assert!(refinement.join("spec.md").is_file());
    assert!(
        !refinement.join("plan.md").exists(),
        "the planner must not invent a fallback path for an absent plan role"
    );
}

#[tokio::test]
async fn planner_executor_fails_closed_when_selected_ward_is_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.id = "planner-agent".to_string();
    agent.mcps.clear();
    agent.skills.clear();

    let result = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-planner",
            "session-planner",
            &[],
            &[],
            None,
            &mcp_service,
            Some("missing-ward"),
        )
        .await;
    let error = match result {
        Ok(_) => panic!("planner with a missing selected ward must fail closed"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("planner_template_unavailable"));
}

#[tokio::test]
async fn planner_executor_fails_closed_when_selected_template_is_invalid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
    gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
    gateway_services::create_ward_from_template(&paths, "broken").expect("ward");
    std::fs::write(paths.ward_layout_snapshot("broken"), "not: [valid").expect("corrupt snapshot");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.id = "planner-agent".to_string();
    agent.mcps.clear();
    agent.skills.clear();

    let result = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build(
            &agent,
            &sample_provider(),
            "conversation-planner",
            "session-planner",
            &[],
            &[],
            None,
            &mcp_service,
            Some("broken"),
        )
        .await;
    let error = match result {
        Ok(_) => panic!("invalid planner template must fail closed"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("planner_template_unavailable"));
}

#[test]
fn effective_max_input_uses_provider_limit_for_legacy_default() {
    let mut agent = sample_agent();
    agent.max_input_tokens = DEFAULT_MAX_INPUT_TOKENS;
    agent.max_input_tokens_explicit = false;
    let provider = sample_provider();

    assert_eq!(resolve_effective_max_input(&agent, &provider), 24_576);
}

#[test]
fn effective_max_input_keeps_explicit_agent_limit() {
    let mut agent = sample_agent();
    agent.max_input_tokens = 64_000;
    agent.max_input_tokens_explicit = true;
    let provider = sample_provider();

    assert_eq!(resolve_effective_max_input(&agent, &provider), 64_000);
}

#[test]
fn effective_max_input_keeps_explicit_default_value() {
    let mut agent = sample_agent();
    agent.max_input_tokens = DEFAULT_MAX_INPUT_TOKENS;
    agent.max_input_tokens_explicit = true;
    let provider = sample_provider();

    assert_eq!(
        resolve_effective_max_input(&agent, &provider),
        DEFAULT_MAX_INPUT_TOKENS
    );
}

#[test]
fn rig_agent_config_preserves_gateway_agent_and_model_settings() {
    let agent = sample_agent();
    let llm = LlmConfig::new(
        "https://llm.local/v1".to_string(),
        "sk-test".to_string(),
        agent.model.clone(),
        agent.provider_id.clone(),
    )
    .with_temperature(agent.temperature)
    .with_max_tokens(agent.max_tokens)
    .with_thinking(agent.thinking_enabled)
    .with_provider_params(serde_json::json!({"parallel_tool_calls": false}));

    let mapped = build_rig_agent_config(&agent, &llm, agent.max_input_tokens);

    assert_eq!(mapped.agent_id, "agent-1");
    assert_eq!(mapped.name, "Code Agent");
    assert_eq!(mapped.description, "Writes code");
    assert_eq!(mapped.instructions, "Follow the project rules.");
    assert_eq!(mapped.model.provider_id, "provider-1");
    assert_eq!(mapped.model.base_url, "https://llm.local/v1");
    assert_eq!(mapped.model.api_key, "sk-test");
    assert_eq!(mapped.model.model, "gpt-test");
    assert_eq!(mapped.model.temperature, 0.25);
    assert_eq!(mapped.model.max_tokens, 4_096);
    assert_eq!(mapped.model.context_window_tokens, 64_000);
    assert_eq!(
        mapped.model.completion_additional_params(),
        Ok(Some(serde_json::json!({
            "parallel_tool_calls": false,
            "thinking": {"type": "enabled"}
        })))
    );
}

#[tokio::test]
async fn execution_engine_is_rig_regardless_of_environment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.max_input_tokens = DEFAULT_MAX_INPUT_TOKENS;
    agent.max_input_tokens_explicit = false;
    agent.max_tokens = 4_096;
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();

    // No engine-selection flag exists: whatever the environment holds,
    // construction returns the Rig engine. (Env mutation is safe here —
    // this is the only test touching ZBOT_ENGINE in the process.)
    for value in [None, Some("rig"), Some("legacy"), Some("garbage")] {
        match value {
            Some(v) => std::env::set_var("ZBOT_ENGINE", v),
            None => std::env::remove_var("ZBOT_ENGINE"),
        }
        // Rebuild per iteration: PreparedExecution is not Clone.
        let prepared = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
            .build(
                &agent,
                &provider,
                "c",
                "s",
                &[],
                &[],
                None,
                &mcp_service,
                None,
            )
            .await
            .expect("executor build");
        let engine =
            build_execution_engine(prepared).expect("engine construction is unconditional");
        assert_eq!(
            engine.engine_name(),
            "rig",
            "ZBOT_ENGINE={value:?} must not change engine selection"
        );
    }
    std::env::remove_var("ZBOT_ENGINE");
}

#[tokio::test]
async fn missing_rig_config_fails_explicitly_instead_of_falling_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();
    let mut prepared = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build(
            &agent,
            &provider,
            "c",
            "s",
            &[],
            &[],
            None,
            &mcp_service,
            None,
        )
        .await
        .expect("executor build");
    // Strip the resolved config: construction must fail closed, never
    // silently fall back to an alternative loop.
    prepared.rig_config = None;
    let error = match build_execution_engine(prepared) {
        Err(error) => error,
        Ok(_) => panic!("missing rig config must fail explicitly"),
    };
    assert!(
        error
            .to_string()
            .contains("rig_execution_config_unresolved"),
        "explicit failure, got: {error}"
    );
}

#[tokio::test]
async fn builder_resolves_mcp_display_name_for_real_rig_dispatch() {
    use agent_runtime::AgentEngine;

    struct AliasLlm;
    #[async_trait]
    impl LlmClient for AliasLlm {
        fn model(&self) -> &str {
            "fixture"
        }
        fn provider(&self) -> &str {
            "fixture"
        }
        async fn chat(
            &self,
            _messages: Vec<agent_runtime::types::ChatMessage>,
            _tools: Option<serde_json::Value>,
        ) -> Result<agent_runtime::llm::ChatResponse, agent_runtime::llm::LlmError> {
            Ok(agent_runtime::llm::ChatResponse {
                content: String::new(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        async fn chat_stream(
            &self,
            _messages: Vec<agent_runtime::types::ChatMessage>,
            _tools: Option<serde_json::Value>,
            _callback: agent_runtime::llm::StreamCallback,
        ) -> Result<agent_runtime::llm::ChatResponse, agent_runtime::llm::LlmError> {
            Ok(agent_runtime::llm::ChatResponse {
                content: String::new(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().unwrap();
    let service = McpService::new(paths);
    service.add(serde_json::from_value(serde_json::json!({
        "type":"stdio", "id":"canonical-probe", "name":"Friendly Probe",
        "description":"fixture", "command":"python3", "enabled":true,
        "args":["-u", std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/agent-runtime/tests/fixtures/mcp_stdio_probe.py")]
    })).unwrap()).unwrap();
    let mut agent = sample_agent();
    agent.mcps = vec!["Friendly Probe".into()];
    agent.skills.clear();
    let mut prepared = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build(
            &agent,
            &sample_provider(),
            "conversation",
            "session",
            &[],
            &[],
            None,
            &service,
            None,
        )
        .await
        .unwrap();
    assert_eq!(prepared.config().mcps, vec!["canonical-probe"]);
    prepared.llm_client = Arc::new(AliasLlm);
    let rig = prepared.rig_config.clone().unwrap();
    let engine = agent_runtime::rig_adapter::factory::build_engine(prepared, rig);
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        engine.execute("call the fixture", &[]),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn builder_attaches_rig_agent_config_from_production_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.max_input_tokens = DEFAULT_MAX_INPUT_TOKENS;
    agent.max_input_tokens_explicit = false;
    agent.max_tokens = 4_096;
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build(
            &agent,
            &provider,
            "conversation-1",
            "session-1",
            &[],
            &[],
            None,
            &mcp_service,
            None,
        )
        .await
        .expect("executor build");

    let rig = executor
        .rig_config
        .as_ref()
        .expect("rig config should be attached");
    assert_eq!(rig.agent_id, "agent-1");
    assert_eq!(rig.name, "Code Agent");
    assert_eq!(rig.instructions, "Follow the project rules.");
    assert_eq!(rig.model.provider_id, "provider-1");
    assert_eq!(rig.model.base_url, "http://localhost:9999/v1");
    assert_eq!(rig.model.api_key, "sk-test");
    assert_eq!(rig.model.model, "gpt-test");
    assert_eq!(rig.model.temperature, 0.25);
    assert_eq!(rig.model.max_tokens, 2_048);
    assert_eq!(rig.model.context_window_tokens, 24_576);
    assert!(rig.model.thinking_enabled);
}

#[tokio::test]
async fn builder_hides_broad_context_pull_tools_from_model_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build(
            &agent,
            &provider,
            "conversation-1",
            "session-1",
            &[],
            &[],
            None,
            &mcp_service,
            None,
        )
        .await
        .expect("executor build");

    assert!(executor.tool_registry().contains("memory"));
    assert!(executor.tool_registry().contains("memory_write"));
    assert!(executor.config().model_hidden_tools.contains("memory"));
    assert!(executor.config().model_hidden_tools.contains("graph_query"));
    assert!(executor
        .config()
        .model_hidden_tools
        .contains("query_resource"));
    let visible_names = executor
        .model_visible_tools()
        .into_iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();
    assert!(!visible_names.contains("memory"));
    assert!(visible_names.contains("memory_write"));
    assert!(visible_names.contains("shell"));
    assert!(visible_names.contains("ward"));
}

#[tokio::test]
async fn builder_exposes_narrow_recall_when_memory_recall_is_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();
    let mut recall =
        gateway_memory::MemoryRecall::new(None, Arc::new(gateway_memory::RecallConfig::default()));
    recall.set_provider_scope(gateway_memory::RecallProviderScope::new(
        "tenant-a".to_string(),
        true,
        false,
    ));
    let recall = Arc::new(recall);

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_memory_recall(recall)
        .build(
            &agent,
            &provider,
            "conversation-1",
            "session-1",
            &[],
            &[],
            None,
            &mcp_service,
            None,
        )
        .await
        .expect("executor build");

    let visible_names = executor
        .model_visible_tools()
        .into_iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();
    assert!(visible_names.contains("recall"));
    assert!(!visible_names.contains("memory"));
    assert!(!visible_names.contains("graph_query"));
    assert!(!visible_names.contains("query_resource"));
}

#[tokio::test]
async fn builder_exposes_connector_split_and_hides_query_resource_from_model_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("vault dirs");
    let mcp_service = McpService::new(paths);
    let mut agent = sample_agent();
    agent.mcps.clear();
    agent.skills.clear();
    let provider = sample_provider();

    let executor = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_connector_provider(Arc::new(MockConnectorProvider))
        .build(
            &agent,
            &provider,
            "conversation-1",
            "session-1",
            &[],
            &[],
            None,
            &mcp_service,
            None,
        )
        .await
        .expect("executor build");

    assert!(executor.tool_registry().contains("query_resource"));
    assert!(executor.tool_registry().contains("connector_resource"));
    assert!(executor.tool_registry().contains("connector_invoke"));

    let visible_names = executor
        .model_visible_tools()
        .into_iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();
    assert!(!visible_names.contains("query_resource"));
    assert!(visible_names.contains("connector_resource"));
    assert!(visible_names.contains("connector_invoke"));
}

fn registry_names(actor_kind: RuntimeActorKind) -> BTreeSet<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(actor_kind)
        .build_tool_registry(fs_context)
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect()
}

struct FakeA2aDelegation;

#[async_trait]
impl crate::a2a::A2aDelegationService for FakeA2aDelegation {
    async fn list_peers(
        &self,
        _context: &crate::a2a::A2aDelegationContext,
    ) -> Result<Vec<crate::a2a::A2aPeerSummary>, crate::a2a::A2aDelegationError> {
        Ok(Vec::new())
    }

    async fn delegate(
        &self,
        _context: crate::a2a::A2aDelegationContext,
        _peer_id: &str,
        _content: &str,
    ) -> Result<crate::a2a::A2aDelegationReceipt, crate::a2a::A2aDelegationError> {
        Ok(crate::a2a::A2aDelegationReceipt {
            task_id: "work-test".to_owned(),
        })
    }
}

fn registry_names_with_a2a(actor_kind: RuntimeActorKind) -> BTreeSet<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(actor_kind)
        .with_a2a_delegation(Arc::new(FakeA2aDelegation))
        .build_tool_registry(fs_context)
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect()
}

#[test]
fn a2a_tools_are_root_and_ward_only() {
    for actor in [RuntimeActorKind::Root, RuntimeActorKind::WardAgent] {
        assert_has(
            &registry_names_with_a2a(actor),
            &["list_zbots", "delegate_to_zbot"],
        );
    }
    for actor in [
        RuntimeActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer,
        RuntimeActorKind::RemotePeer,
    ] {
        assert_missing(
            &registry_names_with_a2a(actor),
            &["list_zbots", "delegate_to_zbot"],
        );
    }
}

// A2A AC8/AC15 — remote peers receive no local side-effect surface.
#[test]
fn remote_peer_inventory_is_exactly_respond() {
    assert_eq!(
        registry_names(RuntimeActorKind::RemotePeer),
        BTreeSet::from(["respond".to_string()])
    );
    let catalog = catalog_for_actor(RuntimeActorKind::RemotePeer);
    assert_eq!(catalog.actor_kind, ContextActorKind::RemotePeer);
    assert_eq!(
        catalog
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>(),
        vec!["respond"]
    );
}

#[test]
fn built_in_registry_raw_name_frequencies_match_characterized_actor_inventories() {
    for actor_kind in [
        RuntimeActorKind::Root,
        RuntimeActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer,
        RuntimeActorKind::WardAgent,
        RuntimeActorKind::RemotePeer,
    ] {
        for file_tools in [false, true] {
            let dir = tempfile::tempdir().expect("tempdir");
            let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
            let tool_settings = ToolSettings {
                file_tools,
                ..ToolSettings::default()
            };
            let frequencies = ExecutorBuilder::new(dir.path().to_path_buf(), tool_settings)
                .with_actor_kind(actor_kind)
                .build_tool_registry(fs_context)
                .get_all()
                .iter()
                .fold(
                    std::collections::BTreeMap::<String, usize>::new(),
                    |mut frequencies, tool| {
                        *frequencies.entry(tool.name().to_string()).or_default() += 1;
                        frequencies
                    },
                );

            let expected_names: &[&str] = match (actor_kind, file_tools) {
                (RuntimeActorKind::Root, false) => &[
                    "delegate_to_agent",
                    "memory",
                    "memory_write",
                    "multimodal_analyze",
                    "present_surface",
                    "read",
                    "respond",
                    "shell",
                    "update_plan",
                    "ward",
                ],
                (RuntimeActorKind::Root, true) => &[
                    "delegate_to_agent",
                    "memory",
                    "memory_write",
                    "multimodal_analyze",
                    "present_surface",
                    "read",
                    "respond",
                    "shell",
                    "update_plan",
                    "ward",
                ],
                (RuntimeActorKind::DelegatedExecutor, false) => &[
                    "edit_file",
                    "load_skill",
                    "memory",
                    "memory_write",
                    "multimodal_analyze",
                    "read",
                    "respond",
                    "shell",
                    "ward",
                    "write_file",
                ],
                (RuntimeActorKind::DelegatedExecutor, true) => &[
                    "edit_file",
                    "load_skill",
                    "memory",
                    "memory_write",
                    "multimodal_analyze",
                    "read",
                    "respond",
                    "shell",
                    "ward",
                    "write_file",
                ],
                (RuntimeActorKind::DelegatedReviewer, _) => {
                    &["load_skill", "multimodal_analyze", "read", "respond"]
                }
                (RuntimeActorKind::WardAgent, _) => &[
                    "delegate_to_agent",
                    "edit_file",
                    "load_skill",
                    "memory",
                    "memory_write",
                    "multimodal_analyze",
                    "present_surface",
                    "read",
                    "respond",
                    "shell",
                    "update_plan",
                    "ward",
                    "write_file",
                ],
                (RuntimeActorKind::RemotePeer, _) => &["respond"],
            };
            let expected = expected_names
                .iter()
                .map(|name| ((*name).to_string(), 1_usize))
                .collect::<std::collections::BTreeMap<_, _>>();

            assert_eq!(
                frequencies, expected,
                "{actor_kind:?} with file_tools={file_tools} must preserve its characterized tool inventory with one registration per name"
            );
        }
    }
}

#[tokio::test]
async fn present_surface_emits_bounded_create_and_update_markers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::Root)
        .build_tool_registry(fs_context);
    let tool = registry
        .find("present_surface")
        .expect("root registry must expose present_surface");
    let ctx: Arc<dyn agent_primitives::ToolContext> = Arc::new(agent_runtime::ToolContext::new());
    let descriptor = serde_json::json!({
        "surface_id": "automatic-summary",
        "components": [{
            "id": "metric",
            "type": "MetricCard",
            "props": {"title": "Total", "value_path": "/value"}
        }],
        "data": {"value": 42}
    });

    let created = tool
        .execute(ctx.clone(), descriptor.clone())
        .await
        .expect("valid descriptor");
    assert_eq!(created["__work_surface"], true);
    assert_eq!(created["surface"]["catalog_id"], "zbot/work-surface/v1");
    assert_eq!(created["surface"]["surface_id"], "automatic-summary");

    let mut update = descriptor;
    update["update"] = serde_json::json!(true);
    update["data"]["value"] = serde_json::json!(84);
    let updated = tool
        .execute(ctx, update)
        .await
        .expect("valid update descriptor");
    assert_eq!(updated["__work_surface_updated"], true);
    assert_eq!(updated["surface"]["data"]["value"], 84);
    assert!(updated.get("__work_surface").is_none());
}

#[tokio::test]
async fn real_present_surface_tool_reaches_validated_gateway_events() {
    for (update, valid) in [(false, true), (true, true), (false, false)] {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
            dir.path().to_path_buf(),
        ));
        paths.ensure_dirs_exist().expect("vault dirs");
        let service = McpService::new(paths.clone());
        let arguments = if valid {
            serde_json::json!({
                "surface_id": "integrated-surface",
                "components": [{
                    "id": "metric",
                    "type": "MetricCard",
                    "props": {"value_path": "/value"}
                }],
                "data": {"value": 42},
                "update": update
            })
        } else {
            serde_json::json!({
                "surface_id": "integrated-surface",
                "components": [{
                    "id": "approval",
                    "type": "ApprovalGate",
                    "props": {"action_id": "inspect", "target": "private"}
                }],
                "data": {}
            })
        };
        // T10: the sole engine constructs through the unconditional
        // factory; the surface journey runs on Rig like every other turn.
        let mut agent = sample_agent();
        agent.mcps.clear();
        agent.skills.clear();
        let mut prepared = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
            .with_actor_kind(RuntimeActorKind::Root)
            .build(
                &agent,
                &sample_provider(),
                "conversation",
                "session",
                &[],
                &[],
                None,
                &service,
                None,
            )
            .await
            .expect("prepared execution");
        prepared.llm_client = Arc::new(SurfaceJourneyLlm {
            calls: Arc::new(AtomicUsize::new(0)),
            arguments,
        });
        let executor = build_execution_engine(prepared).expect("rig engine");
        let mut runtime_events = Vec::new();
        let mut sink = |event: agent_runtime::StreamEvent| runtime_events.push(event);
        executor
            .execute_stream("show the metrics", &[], &mut sink)
            .await
            .expect("execution");
        let gateway_events = runtime_events
            .into_iter()
            .filter_map(|event| {
                crate::events::convert_stream_event(
                    event,
                    "root",
                    "conversation",
                    "session",
                    "execution",
                )
            })
            .collect::<Vec<_>>();

        let has_created = gateway_events.iter().any(|event| {
            matches!(
                event,
                gateway_events::GatewayEvent::SurfaceCreated { surface, .. }
                    if surface.surface_id == "integrated-surface"
            )
        });
        let has_updated = gateway_events.iter().any(|event| {
            matches!(
                event,
                gateway_events::GatewayEvent::SurfaceUpdated { surface, .. }
                    if surface.surface_id == "integrated-surface"
            )
        });
        assert_eq!(has_created, valid && !update);
        assert_eq!(has_updated, valid && update);
    }
}

#[tokio::test]
async fn present_surface_rejects_actionable_or_executable_descriptors_without_echo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::Root)
        .build_tool_registry(fs_context);
    let tool = registry
        .find("present_surface")
        .expect("root registry must expose present_surface");
    let ctx: Arc<dyn agent_primitives::ToolContext> = Arc::new(agent_runtime::ToolContext::new());
    let rejected = [
        serde_json::json!({
            "surface_id": "actionable",
            "components": [{
                "id": "approval",
                "type": "ApprovalGate",
                "props": {"action_id": "inspect", "target": "secret-target"}
            }],
            "data": {}
        }),
        serde_json::json!({
            "surface_id": "executable",
            "components": [{
                "id": "metric",
                "type": "MetricCard",
                "props": {
                    "value_path": "/value",
                    "url": "https://secret.invalid",
                    "html": "<script>secret-payload</script>",
                    "code": "secret-code",
                    "action": "secret-action"
                }
            }],
            "data": {"value": "secret-value"}
        }),
        serde_json::json!({
            "surface_id": "malformed-pointer",
            "components": [{
                "id": "metric",
                "type": "MetricCard",
                "props": {"value_path": "not-a-json-pointer"}
            }],
            "data": {"value": "secret-value"}
        }),
        serde_json::json!({
            "surface_id": "oversized",
            "components": [{
                "id": "metric",
                "type": "MetricCard",
                "props": {"value_path": "/value"}
            }],
            "data": {"value": "secret-value".repeat(10_000)}
        }),
        serde_json::json!({
            "surface_id": "unknown-component",
            "components": [{
                "id": "unknown",
                "type": "RemoteIframe",
                "props": {}
            }],
            "data": {}
        }),
    ];

    for descriptor in rejected {
        let error = tool
            .execute(ctx.clone(), descriptor)
            .await
            .expect_err("actionable/executable descriptor must fail")
            .to_string();
        assert!(error.len() <= 256, "tool errors must remain bounded");
        for secret in [
            "secret-target",
            "secret.invalid",
            "secret-payload",
            "secret-code",
            "secret-action",
            "secret-value",
        ] {
            assert!(!error.contains(secret), "error echoed rejected payload");
        }
    }
}

#[tokio::test]
async fn rejected_present_surface_does_not_block_canonical_response() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::Root)
        .build_tool_registry(fs_context);
    let present = registry
        .find("present_surface")
        .expect("root registry must expose present_surface");
    let respond = registry
        .find("respond")
        .expect("root registry must retain respond");
    let ctx = Arc::new(agent_runtime::ToolContext::new());

    present
        .execute(
            ctx.clone(),
            serde_json::json!({
                "surface_id": "invalid",
                "components": [{
                    "id": "approval",
                    "type": "ApprovalGate",
                    "props": {"action_id": "inspect", "target": "private"}
                }],
                "data": {}
            }),
        )
        .await
        .expect_err("invalid surface must fail closed");

    respond
        .execute(
            ctx.clone(),
            serde_json::json!({"message": "The canonical answer remains available."}),
        )
        .await
        .expect("respond must remain available");
    let actions = agent_primitives::ToolContext::actions(ctx.as_ref());
    let response = actions.respond.expect("respond action");
    assert_eq!(response.message, "The canonical answer remains available.");
}

#[test]
fn present_surface_is_limited_to_user_facing_actors() {
    assert_has(
        &registry_names(RuntimeActorKind::Root),
        &["present_surface"],
    );
    assert_has(
        &registry_names(RuntimeActorKind::WardAgent),
        &["present_surface"],
    );
    assert_missing(
        &registry_names(RuntimeActorKind::DelegatedExecutor),
        &["present_surface"],
    );
    assert_missing(
        &registry_names(RuntimeActorKind::DelegatedReviewer),
        &["present_surface"],
    );

    let catalog = catalog_for_actor(RuntimeActorKind::Root);
    let capability = catalog_capability(&catalog, "present_surface");
    assert_eq!(capability.side_effects, ContextSideEffects::WriteLocal);
    assert_eq!(capability.risk_level, ContextRiskLevel::Low);
    assert!(capability.default_visible);
    let description = capability.description.to_ascii_lowercase();
    for required in [
        "use when",
        "do not use",
        "canonical response",
        "secrets",
        "system prompts",
        "developer instructions",
        "hidden reasoning",
        "unrelated connector",
        "unrelated tool data",
        "contextual title",
        "title_path",
        "never rely on generic component type labels",
    ] {
        assert!(
            description.contains(required),
            "present_surface guidance must mention {required}"
        );
    }
}

#[test]
fn session_scoped_registry_exposes_recall_but_keeps_memory_hidden() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let mut recall =
        gateway_memory::MemoryRecall::new(None, Arc::new(gateway_memory::RecallConfig::default()));
    recall.set_provider_scope(gateway_memory::RecallProviderScope::new(
        "tenant-a".to_string(),
        true,
        false,
    ));
    let recall = Arc::new(recall);
    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_memory_recall(recall)
        .build_tool_registry_with_recall(
            fs_context,
            Some(RecallAuthorizationContext {
                user_id: "default".to_string(),
                agent_id: "root".to_string(),
                actor_kind: "root".to_string(),
                session_id: Some("sess-a".to_string()),
                ward_id: Some("ward-a".to_string()),
                visibility: RecallVisibilityScope {
                    tenant_id: None,
                    workspace_id: None,
                    allowed_ward_ids: vec!["ward-a".to_string()],
                    allowed_session_ids: vec!["sess-a".to_string()],
                    allowed_global_sources: Vec::new(),
                },
            }),
        );
    assert!(registry.contains("recall"));
    assert!(registry.contains("memory"));
}

#[test]
fn recall_catalog_is_available_only_when_the_adapter_is_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let unavailable = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .build_context_capability_catalog(Some("session-1".to_string()), Some("root".to_string()));
    let unavailable_recall = catalog_capability(&unavailable, "recall");
    assert_eq!(
        unavailable_recall.health,
        ContextCapabilityHealth::Unavailable
    );
    assert!(!unavailable_recall.default_visible);
    assert_eq!(
        unavailable_recall.visibility_policy,
        "catalog_only_unavailable"
    );
    assert!(unavailable_recall.input_schema.is_some());

    let mut recall =
        gateway_memory::MemoryRecall::new(None, Arc::new(gateway_memory::RecallConfig::default()));
    recall.set_provider_scope(gateway_memory::RecallProviderScope::new(
        "tenant-a".to_string(),
        true,
        false,
    ));
    let recall = Arc::new(recall);
    let available = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_memory_recall(recall)
        .build_context_capability_catalog(Some("session-1".to_string()), Some("root".to_string()));
    let available_recall = catalog_capability(&available, "recall");
    assert_eq!(available_recall.health, ContextCapabilityHealth::Available);
    assert!(available_recall.default_visible);
    assert_eq!(
        available_recall.visibility_policy,
        "default_visible_unified_recall_exception"
    );
}

fn registry_names_with_agent_control_deps(actor_kind: RuntimeActorKind) -> BTreeSet<String> {
    registry_names_with_agent_control_deps_and_mode(actor_kind, None)
}

fn registry_names_with_agent_control_deps_and_mode(
    actor_kind: RuntimeActorKind,
    delegation_mode: Option<&str>,
) -> BTreeSet<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("ensure vault dirs");
    let db = Arc::new(DatabaseManager::new(paths.clone()).expect("db init"));
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(
        zbot_conversation::open_conversation_pool(&paths.conversations_db())
            .expect("conversation pool"),
    ));

    let state = Arc::new(StateService::new(db.clone()));
    let peer_messages = Arc::new(crate::peer_messaging::DurablePeerMessageService::new(
        Arc::new(execution_state::SqliteWorkStore::new(db)),
        Arc::new(gateway_bus::LocalWorkTransport::new()),
        state.clone(),
        crate::peer_messaging::PEER_MESSAGE_TARGET,
    ));
    let mut builder = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(actor_kind)
        .with_state_service(state)
        .with_steering_registry(Arc::new(agent_runtime::SteeringRegistry::new()))
        .with_agent_result_bus(Arc::new(AgentResultBus::new()))
        .with_message_store(messages)
        .with_peer_messages(peer_messages);
    if let Some(mode) = delegation_mode {
        builder = builder.with_initial_state(
            "app:delegation_mode",
            serde_json::Value::String(mode.to_string()),
        );
    }

    builder
        .build_tool_registry(fs_context)
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect()
}

fn catalog_for_actor(actor_kind: RuntimeActorKind) -> ContextCapabilityCatalog {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(actor_kind)
        .build_tool_registry(fs_context);

    build_context_capability_catalog(
        actor_kind,
        registry.as_ref(),
        Some("session-1".to_string()),
        Some("agent-1".to_string()),
    )
}

fn catalog_for_actor_with_connector(actor_kind: RuntimeActorKind) -> ContextCapabilityCatalog {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(actor_kind)
        .with_connector_provider(Arc::new(MockConnectorProvider))
        .build_tool_registry(fs_context);

    build_context_capability_catalog(
        actor_kind,
        registry.as_ref(),
        Some("session-1".to_string()),
        Some("agent-1".to_string()),
    )
}

fn catalog_for_actor_with_join_deps(actor_kind: RuntimeActorKind) -> ContextCapabilityCatalog {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
        dir.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().expect("ensure vault dirs");
    let db = Arc::new(DatabaseManager::new(paths.clone()).expect("db init"));
    let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(
        zbot_conversation::open_conversation_pool(&paths.conversations_db())
            .expect("conversation pool"),
    ));
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));

    let registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(actor_kind)
        .with_agent_result_bus(Arc::new(AgentResultBus::new()))
        .with_state_service(Arc::new(StateService::new(db.clone())))
        .with_message_store(messages)
        .build_tool_registry(fs_context);

    build_context_capability_catalog(
        actor_kind,
        registry.as_ref(),
        Some("session-1".to_string()),
        Some("agent-1".to_string()),
    )
}

fn catalog_ids(catalog: &ContextCapabilityCatalog) -> BTreeSet<String> {
    catalog
        .capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect()
}

fn catalog_capability<'a>(
    catalog: &'a ContextCapabilityCatalog,
    id: &str,
) -> &'a ContextCapability {
    catalog
        .capabilities
        .iter()
        .find(|capability| capability.id == id)
        .unwrap_or_else(|| panic!("expected catalog capability {id}"))
}

fn assert_has(names: &BTreeSet<String>, expected: &[&str]) {
    for name in expected {
        assert!(names.contains(*name), "expected tool {name}");
    }
}

fn assert_missing(names: &BTreeSet<String>, denied: &[&str]) {
    for name in denied {
        assert!(!names.contains(*name), "unexpected tool {name}");
    }
}

#[test]
fn root_context_catalog_reflects_current_actor_policy() {
    let catalog = catalog_for_actor(RuntimeActorKind::Root);
    let ids = catalog_ids(&catalog);

    assert_eq!(catalog.actor_kind, ContextActorKind::Root);
    assert_has(
        &ids,
        &[
            "shell",
            "memory",
            "memory_write",
            "ward",
            "respond",
            "delegate_to_agent",
        ],
    );
    assert_missing(
        &ids,
        &[
            "write_file",
            "edit_file",
            "load_skill",
            "list_mcps",
            "set_session_title",
        ],
    );

    let shell = catalog_capability(&catalog, "shell");
    assert_eq!(shell.kind, ContextCapabilityKind::Tool);
    assert_eq!(shell.side_effects, ContextSideEffects::Execute);
    assert_eq!(shell.risk_level, ContextRiskLevel::High);
    assert_eq!(shell.owner_crate.as_deref(), Some("agent-tools"));
    assert!(shell.input_schema.is_some());
    assert!(shell.actor_policy.contains(&ContextActorKind::Root));
    assert!(shell
        .actor_policy
        .contains(&ContextActorKind::DelegatedExecutor));
    assert!(shell.actor_policy.contains(&ContextActorKind::WardAgent));
    assert!(!shell
        .actor_policy
        .contains(&ContextActorKind::DelegatedReviewer));
    assert!(shell.default_visible);
    assert_eq!(
        shell.split_target.as_deref(),
        Some("actions:shell_execute; resources:command_result_handles")
    );
}

#[test]
fn delegated_reviewer_catalog_is_read_only_and_review_safe() {
    let catalog = catalog_for_actor(RuntimeActorKind::DelegatedReviewer);
    let ids = catalog_ids(&catalog);

    assert_eq!(catalog.actor_kind, ContextActorKind::DelegatedReviewer);
    assert_has(&ids, &["read", "respond", "load_skill"]);
    assert_missing(
        &ids,
        &[
            "grep",
            "shell",
            "write_file",
            "edit_file",
            "ward",
            "memory",
            "memory_write",
            "delegate_to_agent",
            "wait_agent",
            "set_session_title",
            "list_skills",
            "list_mcps",
        ],
    );
    assert_eq!(
        catalog.capabilities.len(),
        ids.len(),
        "catalog de-duplicates duplicate registry entries"
    );

    let read = catalog_capability(&catalog, "read");
    assert_eq!(read.side_effects, ContextSideEffects::ReadExternal);
    assert!(read
        .actor_policy
        .contains(&ContextActorKind::DelegatedReviewer));
}

#[test]
fn wait_agent_catalog_metadata_marks_parallel_join_action() {
    let root_catalog = catalog_for_actor_with_join_deps(RuntimeActorKind::Root);
    let wait_agent = catalog_capability(&root_catalog, "wait_agent");

    assert_eq!(wait_agent.kind, ContextCapabilityKind::Tool);
    assert_eq!(wait_agent.side_effects, ContextSideEffects::ReadExternal);
    assert_eq!(wait_agent.risk_level, ContextRiskLevel::Low);
    assert_eq!(
        wait_agent.latency_hint,
        Some(ContextLatencyHint::Background)
    );
    assert_eq!(wait_agent.owner_crate.as_deref(), Some("gateway-execution"));
    assert_eq!(wait_agent.audit_policy.as_deref(), Some("join_audit"));
    assert!(!wait_agent.default_visible);
    assert_eq!(
        wait_agent.visibility_policy,
        "visible_when_parallel_children_active"
    );
    assert_eq!(
        wait_agent.split_target.as_deref(),
        Some("action:parallel_join")
    );
    assert!(wait_agent.actor_policy.contains(&ContextActorKind::Root));
    assert!(wait_agent
        .actor_policy
        .contains(&ContextActorKind::WardAgent));
    assert!(!wait_agent
        .actor_policy
        .contains(&ContextActorKind::DelegatedExecutor));
    assert!(!wait_agent
        .actor_policy
        .contains(&ContextActorKind::DelegatedReviewer));

    let executor_catalog = catalog_for_actor_with_join_deps(RuntimeActorKind::DelegatedExecutor);
    assert_missing(&catalog_ids(&executor_catalog), &["wait_agent"]);
}

#[test]
fn delegated_executor_keeps_implementation_tools_without_orchestration() {
    let names = registry_names(RuntimeActorKind::DelegatedExecutor);

    assert_has(
        &names,
        &[
            "shell",
            "write_file",
            "edit_file",
            "read",
            "ward",
            "memory",
            "memory_write",
            "respond",
            "load_skill",
        ],
    );
    assert_missing(
        &names,
        &[
            "delegate_to_agent",
            "grep",
            "wait_agent",
            "kill_agent",
            "steer_agent",
            "update_plan",
            "set_session_title",
            "list_skills",
            "list_mcps",
        ],
    );
}

#[test]
fn delegated_reviewer_is_read_only_and_non_orchestrating() {
    let names = registry_names(RuntimeActorKind::DelegatedReviewer);

    assert_has(&names, &["read", "respond", "load_skill"]);
    assert_missing(
        &names,
        &[
            "grep",
            "shell",
            "write_file",
            "edit_file",
            "ward",
            "memory",
            "memory_write",
            "delegate_to_agent",
            "wait_agent",
            "kill_agent",
            "steer_agent",
            "update_plan",
            "set_session_title",
            "list_skills",
            "list_mcps",
        ],
    );
}

#[test]
fn root_keeps_orchestration_without_implementation_file_writes() {
    let names = registry_names(RuntimeActorKind::Root);

    assert_has(
        &names,
        &[
            "shell",
            "memory",
            "memory_write",
            "ward",
            "update_plan",
            "read",
            "respond",
            "delegate_to_agent",
        ],
    );
    assert_missing(
        &names,
        &[
            "write_file",
            "edit_file",
            "grep",
            "load_skill",
            "list_skills",
            "list_mcps",
            "set_session_title",
        ],
    );
}

#[test]
fn ward_agent_gets_root_and_executor_first_party_tools() {
    let names = registry_names(RuntimeActorKind::WardAgent);

    assert_has(
        &names,
        &[
            "shell",
            "write_file",
            "edit_file",
            "read",
            "ward",
            "memory",
            "memory_write",
            "update_plan",
            "respond",
            "delegate_to_agent",
            "load_skill",
        ],
    );
    assert_missing(
        &names,
        &["grep", "set_session_title", "list_skills", "list_mcps"],
    );
}

#[test]
fn broad_tools_expose_split_target_metadata() {
    let catalog = catalog_for_actor(RuntimeActorKind::Root);
    let name = "memory";
    let capability = catalog_capability(&catalog, name);
    assert!(
        !capability.default_visible,
        "{name} should move behind resource/context packet lanes"
    );
    assert!(
        capability.split_target.is_some(),
        "{name} must name its split target"
    );
    assert_eq!(
        capability.visibility_policy,
        "hidden_from_model_use_context_resources"
    );

    let memory_write = catalog_capability(&catalog, "memory_write");
    assert!(memory_write.default_visible);
    assert_eq!(
        memory_write.visibility_policy,
        "default_visible_memory_write_action"
    );
    assert_eq!(
        memory_write.split_target.as_deref(),
        Some("action:memory_write")
    );

    let name = "graph_query";
    assert!(
        !default_visible(name),
        "{name} should move behind resource/context packet lanes"
    );
    assert_eq!(
        visibility_policy(name),
        "hidden_from_model_use_context_resources"
    );
    assert!(split_target(name).is_some());

    assert!(!default_visible("query_resource"));
    assert_eq!(
        visibility_policy("query_resource"),
        "hidden_from_model_use_connector_split"
    );
    assert!(split_target("query_resource").is_some());

    let connector_catalog = catalog_for_actor_with_connector(RuntimeActorKind::Root);
    let query_resource = catalog_capability(&connector_catalog, "query_resource");
    assert!(!query_resource.default_visible);
    assert_eq!(
        query_resource.visibility_policy,
        "hidden_from_model_use_connector_split"
    );
    assert_eq!(
        query_resource.split_target.as_deref(),
        Some("action:connector_invoke; resources:connector_resource")
    );
    let connector_resource = catalog_capability(&connector_catalog, "connector_resource");
    assert!(connector_resource.default_visible);
    assert_eq!(
        connector_resource.visibility_policy,
        "default_visible_connector_resource_read"
    );
    assert_eq!(
        connector_resource.side_effects,
        ContextSideEffects::ReadExternal
    );
    let connector_invoke = catalog_capability(&connector_catalog, "connector_invoke");
    assert!(connector_invoke.default_visible);
    assert_eq!(
        connector_invoke.visibility_policy,
        "default_visible_connector_invoke_action"
    );
    assert_eq!(
        connector_invoke.side_effects,
        ContextSideEffects::WriteExternal
    );

    for name in ["shell", "ward"] {
        let capability = catalog_capability(&catalog, name);
        assert!(
            capability.default_visible,
            "{name} remains a default-visible action tool"
        );
        assert!(
            capability.split_target.is_some(),
            "{name} must name its split target"
        );
        assert_eq!(capability.visibility_policy, "default_visible_action_tool");
    }

    let ward_catalog = catalog_for_actor(RuntimeActorKind::WardAgent);
    let load_skill = catalog_capability(&ward_catalog, "load_skill");
    assert!(load_skill.default_visible);
    assert_eq!(
        load_skill.split_target.as_deref(),
        Some("resources:skill_packet/skill_section_handles")
    );
    assert_eq!(
        load_skill.visibility_policy,
        "default_visible_bounded_packet"
    );
}

#[test]
fn ward_agent_is_not_marked_as_ordinary_subagent() {
    assert!(RuntimeActorKind::DelegatedExecutor.is_ordinary_subagent());
    assert!(RuntimeActorKind::DelegatedReviewer.is_ordinary_subagent());
    assert!(!RuntimeActorKind::WardAgent.is_ordinary_subagent());
    assert!(!RuntimeActorKind::Root.is_ordinary_subagent());
}

#[test]
fn root_and_ward_get_handoff_tools_when_agent_control_deps_are_wired() {
    let root_names = registry_names_with_agent_control_deps(RuntimeActorKind::Root);
    assert_has(
        &root_names,
        &[
            "list_session_agents",
            "handoff_to_agent",
            "steer_agent",
            "wait_agent",
            "kill_agent",
            "message_agent",
            "reply_to_agent",
        ],
    );

    let ward_names = registry_names_with_agent_control_deps(RuntimeActorKind::WardAgent);
    assert_has(
        &ward_names,
        &[
            "list_session_agents",
            "handoff_to_agent",
            "steer_agent",
            "wait_agent",
            "kill_agent",
            "message_agent",
            "reply_to_agent",
        ],
    );
}

#[test]
fn ordinary_subagents_get_scoped_reply_but_not_initiation_or_control_tools() {
    let executor_names =
        registry_names_with_agent_control_deps(RuntimeActorKind::DelegatedExecutor);
    assert_missing(
        &executor_names,
        &[
            "list_session_agents",
            "handoff_to_agent",
            "message_agent",
            "steer_agent",
        ],
    );
    assert_has(&executor_names, &["reply_to_agent"]);

    let reviewer_names =
        registry_names_with_agent_control_deps(RuntimeActorKind::DelegatedReviewer);
    assert_missing(
        &reviewer_names,
        &[
            "list_session_agents",
            "handoff_to_agent",
            "message_agent",
            "steer_agent",
        ],
    );
    assert_has(&reviewer_names, &["reply_to_agent"]);
}

#[test]
fn builder_extra_initial_state_carries_delegation_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let builder = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_initial_state(
            "app:delegation_mode",
            serde_json::Value::String("direct_artifact".to_string()),
        );

    let entries = builder.extra_initial_state.expect("extra state");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "app:delegation_mode");
    assert_eq!(entries[0].1, "direct_artifact");
}

#[test]
fn planner_capability_lookup_requires_host_catalog_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let ordinary = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .build_tool_registry(fs_context.clone())
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();
    assert!(
        !ordinary.contains("lookup_capabilities"),
        "ordinary workers must not receive planner lookup"
    );

    let root_handoff = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::Root)
        .with_initial_state(
            agent_runtime::tools::PLANNING_CAPABILITY_CATALOG_STATE,
            serde_json::json!({"skills": [], "mcps": []}),
        )
        .build_tool_registry(fs_context.clone())
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();
    assert!(
        !root_handoff.contains("lookup_capabilities"),
        "root may carry a host handoff but must not receive planner lookup"
    );

    let planner = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .with_initial_state(
            agent_runtime::tools::PLANNER_CAPABILITY_CATALOG_STATE,
            serde_json::json!({"skills": [], "mcps": []}),
        )
        .build_tool_registry(fs_context)
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();
    assert!(
        planner.contains("lookup_capabilities"),
        "a host-attached planner catalog enables lookup"
    );
}

/// ward-slim P4: the ward surface mirrors what the guard permits per
/// actor — root keeps its concept actions and drops lint; the planner
/// override keeps lint and drops concept actions; other actors are
/// lifecycle-only.
#[test]
fn ward_tool_audience_splits_by_actor_and_override() {
    let dir = tempfile::tempdir().expect("tempdir");

    let root_fs = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let root_registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::Root)
        .build_tool_registry(root_fs);
    let root_ward = root_registry.find("ward").expect("ward registered");
    let root_schema = root_ward.parameters_schema().unwrap().to_string();
    assert!(
        root_schema.contains("create_concept"),
        "root keeps concept actions"
    );
    assert!(!root_schema.contains("lint"), "root drops lint");

    let planner_fs = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let planner_registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::DelegatedExecutor)
        .with_ward_audience(agent_tools::WardAudience::Planner)
        .build_tool_registry(planner_fs);
    let planner_ward = planner_registry.find("ward").expect("ward registered");
    let planner_schema = planner_ward.parameters_schema().unwrap().to_string();
    assert!(planner_schema.contains("lint"), "planner keeps lint");
    assert!(
        !planner_schema.contains("create_concept"),
        "planner drops concept actions (guard rejects them)"
    );

    let sub_fs = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let sub_registry = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::WardAgent)
        .build_tool_registry(sub_fs);
    let sub_ward = sub_registry.find("ward").expect("ward registered");
    let sub_schema = sub_ward.parameters_schema().unwrap().to_string();
    for template in ["lint", "dry_run", "create_concept"] {
        assert!(
            !sub_schema.contains(template),
            "ward-agent must not see {template}"
        );
    }
}

#[test]
fn step_executor_cannot_lookup_capabilities_or_delegate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let names = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::WardAgent)
        .with_initial_state(
            "app:delegation_mode",
            serde_json::Value::String("step_executor".to_string()),
        )
        .with_initial_state(
            agent_runtime::tools::PLANNER_CAPABILITY_CATALOG_STATE,
            serde_json::json!({"skills": [], "mcps": []}),
        )
        .build_tool_registry(fs_context)
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();

    assert!(!names.contains("lookup_capabilities"));
    assert!(!names.contains("delegate_to_agent"));

    let names = registry_names_with_agent_control_deps_and_mode(
        RuntimeActorKind::WardAgent,
        Some("step_executor"),
    );
    assert_missing(
        &names,
        &[
            "delegate_to_agent",
            "lookup_capabilities",
            "list_session_agents",
            "handoff_to_agent",
            "steer_agent",
            "wait_agent",
            "kill_agent",
        ],
    );
    assert_has(&names, &["reply_to_agent"]);
}

#[test]
fn ward_backed_planner_retains_lookup_and_delegation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs_context = Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
    let names = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
        .with_actor_kind(RuntimeActorKind::WardAgent)
        .with_initial_state(
            "app:delegation_mode",
            serde_json::Value::String("ward_backed_build".to_string()),
        )
        .with_initial_state(
            agent_runtime::tools::PLANNER_CAPABILITY_CATALOG_STATE,
            serde_json::json!({"skills": [], "mcps": []}),
        )
        .build_tool_registry(fs_context)
        .get_all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();

    assert!(names.contains("lookup_capabilities"));
    assert!(names.contains("delegate_to_agent"));
}
