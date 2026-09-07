//! # Executor Builder
//!
//! Builds agent executors with all required components.

use agent_primitives::{ConnectorResourceProvider, FileSystemContext};
use agent_runtime::{
    BoxedAgentEngine, ContextActorKind, ContextCapability, ContextCapabilityCatalog,
    ContextCapabilityHealth, ContextCapabilityKind, ContextCostHint, ContextEditingConfig,
    ContextEditingMiddleware, ContextLatencyHint, ContextRiskLevel, ContextSideEffects,
    DelegateTool, ExecutorConfig, KeepPolicy, LlmClient, LlmConfig, McpManager, MiddlewarePipeline,
    OpenAiClient, PlanBlockMiddleware, PreparedExecution, RespondTool, RetryPolicy,
    RetryingLlmClient, RigAgentConfig, RigModelConfig, SummarizationConfig,
    SummarizationMiddleware, ToolCallDecision, ToolRegistry, TriggerCondition,
};
use agent_tools::{
    ConnectorInvokeTool,
    ConnectorResourceTool,
    EditFileTool,
    GlobTool,
    // Knowledge graph query tool
    GraphQueryTool,
    LoadSkillTool,
    // Root orchestrator tools
    MemoryTool,
    // Multimodal vision fallback
    MultimodalAnalyzeTool,
    QueryResourceTool,
    // Optional file reading tools
    ReadTool,
    RecallAuthorizationContext,
    // Subagent tools
    ShellTool,
    ToolSettings,
    UpdatePlanTool,
    WardTool,
    WriteFileTool,
};
use api_logs::{ExecutionLog, LogCategory, LogLevel, LogService};
use execution_state::StateService;
use gateway_services::agents::Agent;
use gateway_services::models::{ModelRegistry, DEFAULT_MAX_INPUT_TOKENS};
use gateway_services::providers::Provider;
use gateway_services::{McpService, SettingsService, SharedVaultPaths, SkillService, VaultPaths};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use zbot_conversation::MessageStore;
use zbot_runtime_sqlite::DatabaseManager;
use zbot_stores::MemoryFactStore;

use super::setup::SubagentRole;
use crate::agent_pool::AgentResultBus;
use crate::config::GatewayFileSystem;

/// Resolve the effective thinking flag for an agent execution.
///
/// Previously this consulted `ModelRegistry.has_capability(model, Thinking)`
/// and silently disabled thinking on models the registry didn't know about.
/// That blocked users from using `thinkingEnabled=true` against any model
/// they typed into Settings > Advanced that wasn't in the curated registry
/// — effectively gating a user-visible setting on a local allowlist.
///
/// Current behaviour: trust the user-declared flag verbatim. If the
/// provider rejects the reasoning payload, the LLM client returns an
/// error that bubbles to the UI through the normal tool_error path.
/// The `_model` parameter is kept for future telemetry / logging without
/// changing the public signature.
pub fn resolve_thinking_flag(user_flag: bool, _model: &str) -> bool {
    user_flag
}

/// Build the Rig-facing agent config from already-resolved gateway settings.
pub fn build_rig_agent_config(
    agent: &Agent,
    llm_config: &LlmConfig,
    context_window_tokens: u64,
) -> RigAgentConfig {
    RigAgentConfig::new(
        agent.id.clone(),
        agent.display_name.clone(),
        agent.description.clone(),
        agent.instructions.clone(),
        RigModelConfig::from_llm_config(llm_config, context_window_tokens),
    )
}

/// Build the sole execution engine from prepared session inputs.
///
/// Every local entry point — root invoke, continuation, delegated children,
/// durable Research and A2A ingress — constructs here. The Rig-backed
/// engine drives unconditionally: same `LlmClient`, same actor-filtered tool
/// inventory (built-ins, MCP and skills), same shared context, and the same
/// before/after-tool hooks. There is no engine-selection environment flag
/// and no fallback; a session whose required Rig configuration cannot be
/// resolved fails explicitly.
pub fn build_execution_engine(executor: PreparedExecution) -> Result<BoxedAgentEngine, String> {
    let agent_id = executor.config().agent_id.clone();
    let Some(rig_config) = executor.rig_config.clone() else {
        return Err(format!(
            "rig_execution_config_unresolved: agent {agent_id} resolved no engine configuration"
        ));
    };
    tracing::info!(
        target: "rig_cutover",
        agent = %agent_id,
        "constructing RigAgentEngine"
    );
    Ok(Box::new(agent_runtime::rig_adapter::factory::build_engine(
        executor, rig_config,
    )))
}

fn resolve_effective_max_input(agent: &Agent, provider: &Provider) -> u64 {
    let provider_max_input = provider
        .effective_max_input(&agent.model)
        .or(provider.context_window);
    if agent.max_input_tokens_explicit && agent.max_input_tokens > 0 {
        agent.max_input_tokens
    } else {
        provider_max_input.unwrap_or(DEFAULT_MAX_INPUT_TOKENS)
    }
}

/// Runtime actor profile used to derive first-party tool capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeActorKind {
    Root,
    DelegatedExecutor,
    DelegatedReviewer,
    WardAgent,
    RemotePeer,
}

impl RuntimeActorKind {
    fn as_state_value(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::DelegatedExecutor => "delegated_executor",
            Self::DelegatedReviewer => "delegated_reviewer",
            Self::WardAgent => "ward_agent",
            Self::RemotePeer => "remote_peer",
        }
    }

    fn is_delegated_execution(self) -> bool {
        !matches!(self, Self::Root)
    }

    fn is_ordinary_subagent(self) -> bool {
        matches!(self, Self::DelegatedExecutor | Self::DelegatedReviewer)
    }

    fn subagent_role(self) -> Option<SubagentRole> {
        match self {
            Self::DelegatedExecutor => Some(SubagentRole::Executor),
            Self::DelegatedReviewer => Some(SubagentRole::Reviewer),
            Self::Root | Self::WardAgent | Self::RemotePeer => None,
        }
    }
}

impl From<SubagentRole> for RuntimeActorKind {
    fn from(role: SubagentRole) -> Self {
        match role {
            SubagentRole::Executor => Self::DelegatedExecutor,
            SubagentRole::Reviewer => Self::DelegatedReviewer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolCapability {
    AgentControl,
    AgentDelegate,
    AgentReply,
    ConnectorInvoke,
    ConnectorQuery,
    ConnectorResourceRead,
    FileRead,
    FileWrite,
    GoalWrite,
    GraphRead,
    IngestWrite,
    MemoryRead,
    MemoryWrite,
    MultimodalAnalyze,
    PeerDelegate,
    PlanWrite,
    ProcedureRun,
    Respond,
    Shell,
    SkillLoad,
    SurfacePresent,
    WardRead,
    WardWrite,
}

impl ToolCapability {
    fn as_state_value(self) -> &'static str {
        match self {
            Self::AgentControl => "agent.control",
            Self::AgentDelegate => "agent.delegate",
            Self::AgentReply => "agent.reply",
            Self::ConnectorInvoke => "connector.invoke",
            Self::ConnectorQuery => "connector.query",
            Self::ConnectorResourceRead => "connector.resource.read",
            Self::FileRead => "fs.read",
            Self::FileWrite => "fs.write",
            Self::GoalWrite => "goal.write",
            Self::GraphRead => "graph.read",
            Self::IngestWrite => "ingest.write",
            Self::MemoryRead => "memory.read",
            Self::MemoryWrite => "memory.write",
            Self::MultimodalAnalyze => "multimodal.analyze",
            Self::PeerDelegate => "peer.delegate",
            Self::PlanWrite => "plan.write",
            Self::ProcedureRun => "procedure.run",
            Self::Respond => "respond",
            Self::Shell => "process.shell",
            Self::SkillLoad => "skill.load",
            Self::SurfacePresent => "surface.present",
            Self::WardRead => "ward.read",
            Self::WardWrite => "ward.write",
        }
    }
}

fn actor_allows(actor: RuntimeActorKind, capability: ToolCapability) -> bool {
    match actor {
        RuntimeActorKind::Root => matches!(
            capability,
            ToolCapability::AgentControl
                | ToolCapability::AgentDelegate
                | ToolCapability::AgentReply
                | ToolCapability::ConnectorInvoke
                | ToolCapability::ConnectorQuery
                | ToolCapability::ConnectorResourceRead
                | ToolCapability::FileRead
                | ToolCapability::GoalWrite
                | ToolCapability::GraphRead
                | ToolCapability::IngestWrite
                | ToolCapability::MemoryRead
                | ToolCapability::MemoryWrite
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::PeerDelegate
                | ToolCapability::PlanWrite
                | ToolCapability::ProcedureRun
                | ToolCapability::Respond
                | ToolCapability::Shell
                | ToolCapability::SurfacePresent
                | ToolCapability::WardRead
                | ToolCapability::WardWrite
        ),
        RuntimeActorKind::DelegatedExecutor => matches!(
            capability,
            ToolCapability::AgentReply
                | ToolCapability::FileRead
                | ToolCapability::FileWrite
                | ToolCapability::GoalWrite
                | ToolCapability::GraphRead
                | ToolCapability::IngestWrite
                | ToolCapability::MemoryRead
                | ToolCapability::MemoryWrite
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::Respond
                | ToolCapability::Shell
                | ToolCapability::SkillLoad
                | ToolCapability::WardRead
                | ToolCapability::WardWrite
        ),
        RuntimeActorKind::DelegatedReviewer => matches!(
            capability,
            ToolCapability::AgentReply
                | ToolCapability::FileRead
                | ToolCapability::GraphRead
                | ToolCapability::MemoryRead
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::Respond
                | ToolCapability::SkillLoad
                | ToolCapability::WardRead
        ),
        RuntimeActorKind::WardAgent => true,
        RuntimeActorKind::RemotePeer => matches!(capability, ToolCapability::Respond),
    }
}

fn actor_allows_all(actor: RuntimeActorKind, capabilities: &[ToolCapability]) -> bool {
    capabilities
        .iter()
        .copied()
        .all(|capability| actor_allows(actor, capability))
}

fn actor_capabilities(actor: RuntimeActorKind) -> Vec<&'static str> {
    const ALL: &[ToolCapability] = &[
        ToolCapability::AgentControl,
        ToolCapability::AgentDelegate,
        ToolCapability::AgentReply,
        ToolCapability::ConnectorInvoke,
        ToolCapability::ConnectorQuery,
        ToolCapability::ConnectorResourceRead,
        ToolCapability::FileRead,
        ToolCapability::FileWrite,
        ToolCapability::GoalWrite,
        ToolCapability::GraphRead,
        ToolCapability::IngestWrite,
        ToolCapability::MemoryRead,
        ToolCapability::MemoryWrite,
        ToolCapability::MultimodalAnalyze,
        ToolCapability::PeerDelegate,
        ToolCapability::PlanWrite,
        ToolCapability::ProcedureRun,
        ToolCapability::Respond,
        ToolCapability::Shell,
        ToolCapability::SkillLoad,
        ToolCapability::SurfacePresent,
        ToolCapability::WardRead,
        ToolCapability::WardWrite,
    ];

    ALL.iter()
        .copied()
        .filter(|capability| actor_allows(actor, *capability))
        .map(ToolCapability::as_state_value)
        .collect()
}

/// Build an actor-filtered context capability catalog from the live tool
/// registry. This is descriptive metadata only; `build_tool_registry` and
/// `actor_allows` remain the enforcement path.
pub fn build_context_capability_catalog(
    actor: RuntimeActorKind,
    registry: &ToolRegistry,
    session_id: Option<String>,
    agent_id: Option<String>,
) -> ContextCapabilityCatalog {
    let mut seen = BTreeSet::new();
    let mut capabilities = Vec::new();
    for tool in registry.get_all() {
        if !seen.insert(tool.name().to_string()) {
            continue;
        }
        let tool_caps = tool_capabilities(tool.name());
        if !tool_caps.is_empty() && !actor_allows_all(actor, &tool_caps) {
            continue;
        }
        capabilities.push(ContextCapability {
            id: tool.name().to_string(),
            kind: ContextCapabilityKind::Tool,
            display_name: display_name(tool.name()),
            description: tool.description().to_string(),
            actor_policy: actor_policy_for_capabilities(actor, &tool_caps),
            risk_level: risk_level_for_tool(tool.name(), &tool_caps),
            side_effects: side_effects_for_tool(tool.name(), &tool_caps),
            input_schema: tool.parameters_schema(),
            output_schema: None,
            resource_uri_template: None,
            cost_hint: Some(cost_hint_for_tool(tool.name(), &tool_caps)),
            latency_hint: Some(latency_hint_for_tool(tool.name(), &tool_caps)),
            token_hint: token_hint_for_tool(tool.name()),
            health: ContextCapabilityHealth::Available,
            owner_crate: Some(owner_crate_for_tool(tool.name()).to_string()),
            audit_policy: Some(audit_policy_for_tool(tool.name(), &tool_caps).to_string()),
            default_visible: default_visible_for_tool(tool.name(), actor),
            visibility_policy: visibility_policy_for_tool(tool.name(), actor).to_string(),
            split_target: split_target_for_tool(tool.name()).map(str::to_string),
        });
    }

    ContextCapabilityCatalog {
        version: "2026-07-07".to_string(),
        actor_kind: context_actor_kind(actor),
        session_id,
        agent_id,
        capabilities,
    }
}

fn recall_catalog_capability(
    actor: RuntimeActorKind,
    health: ContextCapabilityHealth,
) -> ContextCapability {
    let capabilities = [ToolCapability::MemoryRead];
    let available = health == ContextCapabilityHealth::Available;
    ContextCapability {
        id: "recall".to_string(),
        kind: ContextCapabilityKind::Tool,
        display_name: display_name("recall"),
        description: "Retrieve bounded untrusted reference data from configured unified recall."
            .to_string(),
        actor_policy: actor_policy_for_capabilities(actor, &capabilities),
        risk_level: risk_level_for_tool("recall", &capabilities),
        side_effects: side_effects_for_tool("recall", &capabilities),
        input_schema: Some(agent_tools::recall_parameters_schema()),
        output_schema: None,
        resource_uri_template: None,
        cost_hint: Some(cost_hint_for_tool("recall", &capabilities)),
        latency_hint: Some(latency_hint_for_tool("recall", &capabilities)),
        token_hint: token_hint_for_tool("recall"),
        health,
        owner_crate: Some(owner_crate_for_tool("recall").to_string()),
        audit_policy: Some(audit_policy_for_tool("recall", &capabilities).to_string()),
        default_visible: available,
        visibility_policy: if available {
            "default_visible_unified_recall_exception".to_string()
        } else {
            "catalog_only_unavailable".to_string()
        },
        split_target: split_target_for_tool("recall").map(str::to_string),
    }
}

fn context_actor_kind(actor: RuntimeActorKind) -> ContextActorKind {
    match actor {
        RuntimeActorKind::Root => ContextActorKind::Root,
        RuntimeActorKind::DelegatedExecutor => ContextActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer => ContextActorKind::DelegatedReviewer,
        RuntimeActorKind::WardAgent => ContextActorKind::WardAgent,
        RuntimeActorKind::RemotePeer => ContextActorKind::RemotePeer,
    }
}

fn actor_policy_for_capabilities(
    fallback_actor: RuntimeActorKind,
    capabilities: &[ToolCapability],
) -> Vec<ContextActorKind> {
    if capabilities.is_empty() {
        return vec![context_actor_kind(fallback_actor)];
    }

    [
        RuntimeActorKind::Root,
        RuntimeActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer,
        RuntimeActorKind::WardAgent,
        RuntimeActorKind::RemotePeer,
    ]
    .into_iter()
    .filter(|actor| actor_allows_all(*actor, capabilities))
    .map(context_actor_kind)
    .collect()
}

fn display_name(tool_name: &str) -> String {
    tool_name
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tool_capabilities(name: &str) -> Vec<ToolCapability> {
    match name {
        "delegate_to_agent" => vec![ToolCapability::AgentDelegate],
        "delegate_to_zbot" | "list_zbots" => vec![ToolCapability::PeerDelegate],
        "connector_invoke" => vec![ToolCapability::ConnectorInvoke],
        "connector_resource" => vec![ToolCapability::ConnectorResourceRead],
        "edit" | "edit_file" | "write" | "write_file" => vec![ToolCapability::FileWrite],
        "glob" | "read" => vec![ToolCapability::FileRead],
        "goal" => vec![ToolCapability::GoalWrite],
        "graph_query" => vec![ToolCapability::GraphRead],
        "handoff_to_agent"
        | "kill_agent"
        | "list_session_agents"
        | "message_agent"
        | "steer_agent"
        | "wait_agent" => vec![ToolCapability::AgentControl],
        "reply_to_agent" => vec![ToolCapability::AgentReply],
        "ingest" => vec![ToolCapability::IngestWrite],
        "load_skill" => vec![ToolCapability::SkillLoad],
        "memory" => vec![ToolCapability::MemoryRead, ToolCapability::MemoryWrite],
        "recall" => vec![ToolCapability::MemoryRead],
        "memory_write" => vec![ToolCapability::MemoryWrite],
        "multimodal_analyze" => vec![ToolCapability::MultimodalAnalyze],
        "query_resource" => vec![ToolCapability::ConnectorQuery],
        "respond" => vec![ToolCapability::Respond],
        "run_procedure" => vec![ToolCapability::ProcedureRun],
        "shell" => vec![ToolCapability::Shell],
        "present_surface" => vec![ToolCapability::SurfacePresent],
        "update_plan" => vec![ToolCapability::PlanWrite],
        "ward" => vec![ToolCapability::WardRead, ToolCapability::WardWrite],
        _ => Vec::new(),
    }
}

fn side_effects_for_tool(name: &str, capabilities: &[ToolCapability]) -> ContextSideEffects {
    if name == "wait_agent" || name == "list_session_agents" || name == "list_zbots" {
        return ContextSideEffects::ReadExternal;
    }
    if capabilities.contains(&ToolCapability::Shell) {
        return ContextSideEffects::Execute;
    }
    if capabilities.contains(&ToolCapability::ConnectorInvoke) {
        return ContextSideEffects::WriteExternal;
    }
    if capabilities.contains(&ToolCapability::Respond) {
        return ContextSideEffects::WriteExternal;
    }
    if capabilities.iter().any(|capability| {
        matches!(
            capability,
            ToolCapability::AgentControl
                | ToolCapability::AgentDelegate
                | ToolCapability::PeerDelegate
                | ToolCapability::AgentReply
                | ToolCapability::FileWrite
                | ToolCapability::GoalWrite
                | ToolCapability::IngestWrite
                | ToolCapability::MemoryWrite
                | ToolCapability::PlanWrite
                | ToolCapability::SurfacePresent
                | ToolCapability::WardWrite
        )
    }) {
        return ContextSideEffects::WriteLocal;
    }
    if capabilities.is_empty() {
        ContextSideEffects::None
    } else {
        ContextSideEffects::ReadExternal
    }
}

fn risk_level_for_tool(name: &str, capabilities: &[ToolCapability]) -> ContextRiskLevel {
    if name == "wait_agent" || name == "list_session_agents" || name == "list_zbots" {
        return ContextRiskLevel::Low;
    }
    if capabilities.contains(&ToolCapability::Shell) {
        return ContextRiskLevel::High;
    }
    if capabilities.contains(&ToolCapability::ConnectorInvoke) {
        return ContextRiskLevel::Moderate;
    }
    if capabilities.iter().any(|capability| {
        matches!(
            capability,
            ToolCapability::AgentControl
                | ToolCapability::AgentDelegate
                | ToolCapability::PeerDelegate
                | ToolCapability::FileWrite
                | ToolCapability::IngestWrite
                | ToolCapability::WardWrite
        )
    }) {
        return ContextRiskLevel::Moderate;
    }
    ContextRiskLevel::Low
}

fn cost_hint_for_tool(_name: &str, capabilities: &[ToolCapability]) -> ContextCostHint {
    if capabilities.contains(&ToolCapability::MultimodalAnalyze)
        || capabilities.contains(&ToolCapability::ConnectorQuery)
        || capabilities.contains(&ToolCapability::ConnectorResourceRead)
        || capabilities.contains(&ToolCapability::ConnectorInvoke)
    {
        ContextCostHint::Moderate
    } else {
        ContextCostHint::Cheap
    }
}

fn latency_hint_for_tool(name: &str, capabilities: &[ToolCapability]) -> ContextLatencyHint {
    if name == "wait_agent" {
        return ContextLatencyHint::Background;
    }
    if capabilities.contains(&ToolCapability::ConnectorQuery)
        || capabilities.contains(&ToolCapability::ConnectorResourceRead)
        || capabilities.contains(&ToolCapability::ConnectorInvoke)
        || capabilities.contains(&ToolCapability::MultimodalAnalyze)
    {
        ContextLatencyHint::Slow
    } else {
        ContextLatencyHint::Local
    }
}

fn token_hint_for_tool(name: &str) -> Option<u32> {
    match name {
        "load_skill" => Some(1200),
        "memory" | "recall" | "graph_query" | "query_resource" | "connector_resource" => Some(800),
        "connector_invoke" => Some(300),
        "shell" | "read" => Some(400),
        "wait_agent" => Some(120),
        "present_surface" => Some(300),
        _ => Some(200),
    }
}

fn owner_crate_for_tool(name: &str) -> &'static str {
    match name {
        "delegate_to_agent" | "respond" | "run_procedure" => "agent-runtime",
        "delegate_to_zbot"
        | "handoff_to_agent"
        | "kill_agent"
        | "list_session_agents"
        | "message_agent"
        | "present_surface"
        | "reply_to_agent"
        | "steer_agent"
        | "wait_agent"
        | "list_zbots" => "gateway-execution",
        _ => "agent-tools",
    }
}

fn audit_policy_for_tool(name: &str, capabilities: &[ToolCapability]) -> &'static str {
    if name == "wait_agent" {
        "join_audit"
    } else if name == "present_surface" {
        "presentation_audit"
    } else if capabilities.contains(&ToolCapability::Shell) {
        "execution_audit"
    } else if matches!(
        side_effects_for_tool(name, capabilities),
        ContextSideEffects::None | ContextSideEffects::ReadExternal
    ) {
        "read_audit"
    } else {
        "mutation_audit"
    }
}

fn default_visible_for_tool(name: &str, _actor: RuntimeActorKind) -> bool {
    !matches!(
        name,
        "edit" | "write" | "wait_agent" | "memory" | "graph_query" | "query_resource"
    )
}

fn visibility_policy_for_tool(name: &str, _actor: RuntimeActorKind) -> &'static str {
    match name {
        "wait_agent" => "visible_when_parallel_children_active",
        "edit" | "write" => "legacy_alias_hidden",
        "memory" | "graph_query" => "hidden_from_model_use_context_resources",
        "recall" => "default_visible_unified_recall_exception",
        "query_resource" => "hidden_from_model_use_connector_split",
        "memory_write" => "default_visible_memory_write_action",
        "connector_resource" => "default_visible_connector_resource_read",
        "connector_invoke" => "default_visible_connector_invoke_action",
        "load_skill" => "default_visible_bounded_packet",
        "present_surface" => "default_visible_automatic_presentation",
        "shell" | "ward" => "default_visible_action_tool",
        _ => "default_visible",
    }
}

fn split_target_for_tool(name: &str) -> Option<&'static str> {
    match name {
        "memory" => Some("action:memory_write; resources:memory_recall/context_atoms"),
        "recall" => Some("resources:memory_recall/context_atoms"),
        "memory_write" => Some("action:memory_write"),
        "query_resource" => Some("action:connector_invoke; resources:connector_resource"),
        "connector_resource" => Some("resources:connector_resource"),
        "connector_invoke" => Some("action:connector_invoke"),
        "graph_query" => Some("resources:context_graph_retrieval"),
        "shell" => Some("actions:shell_execute; resources:command_result_handles"),
        "ward" => Some("actions:ward_lifecycle; resources:ward_context"),
        "load_skill" => Some("resources:skill_packet/skill_section_handles"),
        "wait_agent" => Some("action:parallel_join"),
        _ => None,
    }
}

fn model_hidden_tools_for_actor(_actor: RuntimeActorKind) -> Vec<&'static str> {
    vec!["memory", "graph_query", "query_resource"]
}

fn build_runtime_middleware_pipeline(
    context_window_tokens: u64,
    chat_mode: bool,
    summary_client: Option<Arc<dyn LlmClient>>,
) -> Arc<MiddlewarePipeline> {
    let pipeline = MiddlewarePipeline::new();
    let mut trigger_tokens = None;
    let pipeline = if context_window_tokens > 0 {
        let (trigger_pct, keep_results) = if chat_mode {
            (80, 5) // Chat: 80% trigger, keep 5 recent tool results
        } else {
            (70, 8) // Deep: 70% trigger, keep 8 recent tool results
        };
        let threshold = (context_window_tokens as usize * trigger_pct) / 100;
        trigger_tokens = Some(threshold);
        pipeline.add_pre_processor(Box::new(ContextEditingMiddleware::new(
            ContextEditingConfig {
                enabled: true,
                trigger_tokens: threshold,
                keep_tool_results: keep_results,
                min_reclaim: 500,
                clear_tool_inputs: true,
                // Loaded skills are behavioral context; keep them resident rather than
                // replacing them with reload placeholders during context editing.
                exclude_tools: vec!["load_skill".to_string()],
                cascade_unload: true,
                skill_aware_placeholders: true,
                ..Default::default()
            },
        )))
    } else {
        pipeline
    };

    // Layer 1 (pinned plan anchor) runs AFTER context editing so
    // tool-result clearing happens first on the raw tape, then
    // the fresh plan block is re-inserted at a stable slot
    // behind the system prompt. The block's `is_summary = true`
    // flag keeps it out of any future summarization pass.
    let pipeline = pipeline.add_pre_processor(Box::new(PlanBlockMiddleware::new()));

    if let (Some(client), Some(threshold)) = (summary_client, trigger_tokens) {
        let pipeline = pipeline.add_pre_processor(Box::new(SummarizationMiddleware::new(
            SummarizationConfig {
                enabled: true,
                trigger: TriggerCondition {
                    tokens: Some(threshold),
                    messages: None,
                    fraction: None,
                },
                keep: KeepPolicy {
                    messages: Some(if chat_mode { 20 } else { 30 }),
                    tokens: None,
                    fraction: None,
                },
                ..SummarizationConfig::default()
            },
            client,
        )));
        return Arc::new(pipeline);
    }

    Arc::new(pipeline)
}

// ============================================================================
// EXECUTOR BUILDER
// ============================================================================

/// Builder for creating agent executors.
///
/// Encapsulates the complex setup process for creating an executor
/// with all required components (LLM client, tools, MCP, middleware).
pub struct ExecutorBuilder {
    vault_dir: PathBuf,
    tool_settings: ToolSettings,
    fact_store: Option<Arc<dyn MemoryFactStore>>,
    connector_provider: Option<Arc<dyn ConnectorResourceProvider>>,
    rate_limiter: Option<Arc<agent_runtime::ProviderRateLimiter>>,
    model_registry: Option<Arc<ModelRegistry>>,
    actor_kind: RuntimeActorKind,
    /// Ward-tool action audience override. `None` derives from the actor:
    /// root gets lifecycle+concept actions, every other actor gets
    /// lifecycle only. The delegated planner spawn overrides to Planner
    /// (lifecycle + `lint`) — ward-slim P4 mirrors what
    /// `validate_template_context` permits per actor.
    ward_audience_override: Option<agent_tools::WardAudience>,
    subagent_non_streaming: bool,
    /// Trait-routed kg store for the `graph_query` tool.
    kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
    goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
    /// Observer for ward-tool creation events — bumps the curator sidecar's
    /// `created_by = "agent"` on every freshly-scaffolded ward.
    ward_usage: Option<Arc<dyn agent_tools::WardUsageAccess>>,
    /// Shared concrete sidecar service used by Ward layout creation so
    /// publication and durable provenance use the runner's serialization lock.
    ward_usage_service: Option<Arc<gateway_services::WardUsage>>,
    steering_registry: Option<Arc<agent_runtime::SteeringRegistry>>,
    agent_result_bus: Option<Arc<AgentResultBus>>,
    state_service: Option<Arc<StateService<DatabaseManager>>>,
    messages: Option<Arc<dyn MessageStore>>,
    /// Trait-routed procedure store for the `run_procedure` tool.
    procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    memory_recall: Option<Arc<gateway_memory::MemoryRecall>>,
    peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
    a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
    mcp_startup_failure_observer: Option<agent_runtime::mcp::McpStartupFailureObserver>,
    extra_initial_state: Option<Vec<(String, serde_json::Value)>>,
    chat_mode: bool,
    remote_peer_prompt: Option<crate::a2a::RemotePeerPrompt>,
}

impl ExecutorBuilder {
    /// Create a new executor builder.
    pub fn new(vault_dir: PathBuf, tool_settings: ToolSettings) -> Self {
        Self {
            vault_dir,
            tool_settings,
            fact_store: None,
            connector_provider: None,
            rate_limiter: None,
            model_registry: None,
            actor_kind: RuntimeActorKind::Root,
            ward_audience_override: None,
            subagent_non_streaming: true,
            kg_store: None,
            ingestion_adapter: None,
            goal_adapter: None,
            ward_usage: None,
            ward_usage_service: None,
            steering_registry: None,
            agent_result_bus: None,
            state_service: None,
            messages: None,
            procedure_store: None,
            memory_recall: None,
            peer_messages: None,
            a2a_delegation: None,
            mcp_startup_failure_observer: None,
            extra_initial_state: None,
            chat_mode: false,
            remote_peer_prompt: None,
        }
    }

    /// Set the memory fact store for DB-backed save_fact/recall.
    pub fn with_fact_store(mut self, fact_store: Arc<dyn MemoryFactStore>) -> Self {
        self.fact_store = Some(fact_store);
        self
    }

    /// Set the trait-routed procedure store for the `run_procedure` tool.
    pub fn with_procedure_store(
        mut self,
        procedure_store: Arc<dyn zbot_stores_traits::ProcedureStore>,
    ) -> Self {
        self.procedure_store = Some(procedure_store);
        self
    }

    /// Wire model-visible unified recall for this executor when memory recall
    /// is available in the gateway composition root.
    pub fn with_memory_recall(mut self, memory_recall: Arc<gateway_memory::MemoryRecall>) -> Self {
        self.memory_recall = Some(memory_recall);
        self
    }

    /// Wire durable same-session peer messaging tools for this executor.
    pub fn with_peer_messages(
        mut self,
        service: Arc<crate::peer_messaging::DurablePeerMessageService>,
    ) -> Self {
        self.peer_messages = Some(service);
        self
    }

    pub fn with_a2a_delegation(
        mut self,
        service: Arc<dyn crate::a2a::A2aDelegationService>,
    ) -> Self {
        self.a2a_delegation = Some(service);
        self
    }

    /// Persist a safe host-side event if MCP startup or discovery fails. The
    /// observer receives only a canonical configured ID, never client errors.
    pub fn with_mcp_startup_failure_observer(
        mut self,
        observer: agent_runtime::mcp::McpStartupFailureObserver,
    ) -> Self {
        self.mcp_startup_failure_observer = Some(observer);
        self
    }

    /// Set the connector resource provider for connector tools.
    pub fn with_connector_provider(mut self, provider: Arc<dyn ConnectorResourceProvider>) -> Self {
        self.connector_provider = Some(provider);
        self
    }

    /// Set the shared rate limiter for this executor's provider.
    ///
    /// The limiter is shared across all executors using the same provider,
    /// so root and subagents respect the same concurrent-request and RPM limits.
    pub fn with_rate_limiter(mut self, limiter: Arc<agent_runtime::ProviderRateLimiter>) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    /// Mark this executor as a delegated subagent (enables plan step cap).
    pub fn with_delegated(mut self, is_delegated: bool) -> Self {
        self.actor_kind = if is_delegated {
            RuntimeActorKind::DelegatedExecutor
        } else {
            RuntimeActorKind::Root
        };
        self
    }

    /// Set a specific subagent role for ordinary delegated agents.
    pub fn with_subagent_role(mut self, role: SubagentRole) -> Self {
        self.actor_kind = RuntimeActorKind::from(role);
        self
    }

    /// Override the ward tool's action audience (e.g. the delegated
    /// planner registers with `WardAudience::Planner`).
    #[must_use]
    pub fn with_ward_audience(mut self, audience: agent_tools::WardAudience) -> Self {
        self.ward_audience_override = Some(audience);
        self
    }

    /// Set the exact runtime actor kind.
    pub fn with_actor_kind(mut self, actor_kind: RuntimeActorKind) -> Self {
        self.actor_kind = actor_kind;
        self
    }

    /// Supply the isolated prompt required by [`RuntimeActorKind::RemotePeer`].
    pub fn with_remote_peer_prompt(mut self, prompt: crate::a2a::RemotePeerPrompt) -> Self {
        self.actor_kind = RuntimeActorKind::RemotePeer;
        self.remote_peer_prompt = Some(prompt);
        self
    }

    /// Set whether subagents use non-streaming requests.
    pub fn with_subagent_non_streaming(mut self, non_streaming: bool) -> Self {
        self.subagent_non_streaming = non_streaming;
        self
    }

    /// Set the fallback-only model metadata registry.
    pub fn with_model_registry(mut self, registry: Arc<ModelRegistry>) -> Self {
        self.model_registry = Some(registry);
        self
    }

    /// Set the trait-routed kg store for the `graph_query` tool.
    pub fn with_kg_store(mut self, store: Arc<dyn zbot_stores::KnowledgeGraphStore>) -> Self {
        self.kg_store = Some(store);
        self
    }

    /// Set the ingestion access adapter for the `ingest` tool.
    pub fn with_ingestion_adapter(
        mut self,
        adapter: Arc<dyn agent_tools::IngestionAccess>,
    ) -> Self {
        self.ingestion_adapter = Some(adapter);
        self
    }

    /// Set the goal access adapter for the `goal` tool.
    pub fn with_goal_adapter(mut self, adapter: Arc<dyn agent_tools::GoalAccess>) -> Self {
        self.goal_adapter = Some(adapter);
        self
    }

    /// Set the ward-usage observer for the `ward` tool's create action.
    pub fn with_ward_usage(mut self, observer: Arc<dyn agent_tools::WardUsageAccess>) -> Self {
        self.ward_usage = Some(observer);
        self
    }

    /// Set the runner-owned Ward usage service used by transactional Ward
    /// creation and provenance reads.
    pub fn with_ward_usage_service(mut self, usage: Arc<gateway_services::WardUsage>) -> Self {
        self.ward_usage_service = Some(usage);
        self
    }

    /// Set the steering registry for the `steer_agent` tool.
    pub fn with_steering_registry(
        mut self,
        registry: Arc<agent_runtime::SteeringRegistry>,
    ) -> Self {
        self.steering_registry = Some(registry);
        self
    }

    /// Set the agent result bus for `wait_agent` and `kill_agent` tools.
    pub fn with_agent_result_bus(mut self, bus: Arc<AgentResultBus>) -> Self {
        self.agent_result_bus = Some(bus);
        self
    }

    /// Set the state service used by `wait_agent` fast-path.
    pub fn with_state_service(mut self, svc: Arc<StateService<DatabaseManager>>) -> Self {
        self.state_service = Some(svc);
        self
    }

    /// Set the message store used by `wait_agent` fast-path.
    pub fn with_message_store(mut self, messages: Arc<dyn MessageStore>) -> Self {
        self.messages = Some(messages);
        self
    }

    /// Enable chat mode (disables single_action_mode for multi-tool turns, larger
    /// middleware keep window, higher compaction warn threshold).
    pub fn with_chat_mode(mut self, chat_mode: bool) -> Self {
        self.chat_mode = chat_mode;
        self
    }

    /// Add an initial state entry that will be injected into executor context.
    pub fn with_initial_state(mut self, key: &str, value: serde_json::Value) -> Self {
        self.extra_initial_state
            .get_or_insert_with(Vec::new)
            .push((key.to_string(), value));
        self
    }

    fn has_planner_capability_catalog(&self) -> bool {
        self.extra_initial_state.as_ref().is_some_and(|entries| {
            entries
                .iter()
                .any(|(key, _)| key == agent_runtime::tools::PLANNER_CAPABILITY_CATALOG_STATE)
        })
    }

    fn is_step_executor(&self) -> bool {
        self.extra_initial_state.as_ref().is_some_and(|entries| {
            entries.iter().any(|(key, value)| {
                key == "app:delegation_mode" && value.as_str() == Some("step_executor")
            })
        })
    }

    /// Build a descriptive context capability catalog from the same registry
    /// construction path used for execution.
    pub fn build_context_capability_catalog(
        &self,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) -> ContextCapabilityCatalog {
        let fs_context: Arc<dyn FileSystemContext> =
            Arc::new(GatewayFileSystem::new(self.vault_dir.clone()));
        let registry = self.build_tool_registry(fs_context);

        let mut catalog = build_context_capability_catalog(
            self.actor_kind,
            registry.as_ref(),
            session_id,
            agent_id,
        );
        if actor_allows(self.actor_kind, ToolCapability::MemoryRead)
            && !catalog
                .capabilities
                .iter()
                .any(|capability| capability.id == "recall")
        {
            let health = if self
                .memory_recall
                .as_ref()
                .is_some_and(|recall| recall.provider_scope().is_some())
            {
                ContextCapabilityHealth::Available
            } else {
                ContextCapabilityHealth::Unavailable
            };
            catalog
                .capabilities
                .push(recall_catalog_capability(self.actor_kind, health));
        }
        catalog
    }

    /// Build an executor for the given agent and provider.
    ///
    /// # Arguments
    /// * `agent` - The agent configuration
    /// * `provider` - The resolved provider
    /// * `conversation_id` - The conversation ID for this execution
    /// * `session_id` - The session ID for this execution
    /// * `available_agents` - List of available agents (for list_agents tool)
    /// * `available_skills` - List of available skills (for runtime context/catalog metadata)
    /// * `hook_context` - Optional hook context for initial state
    /// * `mcp_service` - MCP service for starting servers
    /// * `ward_id` - Optional active ward from existing session
    #[allow(clippy::too_many_arguments)]
    pub async fn build(
        &self,
        agent: &Agent,
        provider: &Provider,
        conversation_id: &str,
        session_id: &str,
        available_agents: &[serde_json::Value],
        available_skills: &[serde_json::Value],
        hook_context: Option<&serde_json::Value>,
        mcp_service: &McpService,
        ward_id: Option<&str>,
    ) -> Result<PreparedExecution, String> {
        let remote_prompt = if matches!(self.actor_kind, RuntimeActorKind::RemotePeer) {
            Some(
                self.remote_peer_prompt
                    .as_ref()
                    .ok_or_else(|| "remote peer prompt is required".to_string())?,
            )
        } else {
            None
        };
        // Build executor config
        let mut executor_config = ExecutorConfig::new(
            agent.id.clone(),
            provider.id.clone().unwrap_or_else(|| provider.name.clone()),
            agent.model.clone(),
        )
        .with_model_hidden_tools(model_hidden_tools_for_actor(self.actor_kind));

        // Add hook context to initial state if present
        if remote_prompt.is_none() {
            if let Some(hook_ctx) = hook_context {
                executor_config =
                    executor_config.with_initial_state("hook_context", hook_ctx.clone());
            }
        }

        // Cache available agents for list_agents tool
        if remote_prompt.is_none() && !available_agents.is_empty() {
            executor_config = executor_config.with_initial_state(
                "available_agents",
                serde_json::Value::Array(available_agents.to_vec()),
            );
        }

        // Cache available skills for runtime context/catalog metadata.
        if remote_prompt.is_none() && !available_skills.is_empty() {
            executor_config = executor_config.with_initial_state(
                "available_skills",
                serde_json::Value::Array(available_skills.to_vec()),
            );
        }

        // Inject session_id so tools (e.g., shell) can scope working directories
        executor_config = executor_config.with_initial_state(
            "session_id",
            serde_json::Value::String(session_id.to_string()),
        );

        let mut ward_template_prompt = None;
        if remote_prompt.is_none() {
            let is_planner = agent.id == "planner-agent";
            if is_planner && ward_id.is_none() {
                return Err("planner_template_unavailable".to_string());
            }
            if let Some(ward) = ward_id {
                executor_config = executor_config
                    .with_initial_state("ward_id", serde_json::Value::String(ward.to_string()));

                // Root and the dedicated planner independently load the selected
                // ward's canonical normalized snapshot. Ordinary delegates remain
                // isolated from template authority. Root can surface repair guidance;
                // planner fails closed because it cannot safely choose paths.
                if matches!(self.actor_kind, RuntimeActorKind::Root) || is_planner {
                    let root_context_id = uuid::Uuid::now_v7().to_string();
                    let layout = self
                        .ward_usage_service
                        .clone()
                        .map(|usage| {
                            super::ward_layout_adapter::GatewayWardLayoutAccess::with_usage(
                                self.vault_dir.clone(),
                                usage,
                            )
                        })
                        .unwrap_or_else(|| {
                            super::ward_layout_adapter::GatewayWardLayoutAccess::new(
                                self.vault_dir.clone(),
                            )
                        })
                        .state(ward, session_id, &root_context_id);
                    if is_planner && layout.context.is_none() {
                        return Err("planner_template_unavailable".to_string());
                    }
                    ward_template_prompt = layout.context;
                    executor_config = executor_config
                        .with_initial_state("ward_template", layout.packet)
                        .with_initial_state(
                            "ward_template_context_id",
                            serde_json::Value::String(root_context_id),
                        );
                }
            }
        }

        executor_config = executor_config.with_initial_state(
            "app:actor_kind",
            serde_json::Value::String(self.actor_kind.as_state_value().to_string()),
        );
        executor_config = executor_config.with_initial_state(
            "app:tool_capabilities",
            serde_json::Value::Array(
                actor_capabilities(self.actor_kind)
                    .into_iter()
                    .map(|capability| serde_json::Value::String(capability.to_string()))
                    .collect(),
            ),
        );

        if self.actor_kind.is_ordinary_subagent() {
            executor_config = executor_config
                .with_initial_state("app:is_delegated", serde_json::Value::Bool(true));
        }
        if let Some(role) = self.actor_kind.subagent_role() {
            let role = match role {
                SubagentRole::Executor => "executor",
                SubagentRole::Reviewer => "reviewer",
            };
            executor_config = executor_config
                .with_initial_state("app:subagent_role", serde_json::Value::String(role.into()));
        }

        // Inject extra initial state (e.g., ward_purpose, ward_structure from intent analysis)
        if remote_prompt.is_none() {
            if let Some(entries) = &self.extra_initial_state {
                for (key, value) in entries {
                    executor_config = executor_config.with_initial_state(key, value.clone());
                }
            }
        }

        // Inject multimodal config for the multimodal_analyze tool
        let settings_service = SettingsService::from_vault_dir(self.vault_dir.clone());
        if remote_prompt.is_none() {
            if let Ok(settings) = settings_service.load() {
                let mm = &settings.execution.multimodal;
                if let (Some(provider_id), Some(model)) = (&mm.provider_id, &mm.model) {
                    // Resolve the provider to get base_url and api_key
                    let providers_path = VaultPaths::new(self.vault_dir.clone()).providers();
                    let provider_creds = std::fs::read_to_string(&providers_path)
                        .ok()
                        .and_then(|content| {
                            serde_json::from_str::<Vec<serde_json::Value>>(&content).ok()
                        })
                        .and_then(|providers| {
                            providers
                                .into_iter()
                                .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(provider_id))
                        });

                    if let Some(prov) = provider_creds {
                        let base_url = prov.get("baseUrl").and_then(|v| v.as_str()).unwrap_or("");
                        let api_key = prov.get("apiKey").and_then(|v| v.as_str()).unwrap_or("");
                        executor_config = executor_config.with_initial_state(
                            "multimodal_config",
                            serde_json::json!({
                                "providerId": provider_id,
                                "model": model,
                                "temperature": mm.temperature,
                                "maxTokens": mm.max_tokens,
                                "baseUrl": base_url,
                                "apiKey": api_key,
                            }),
                        );
                    }
                }
            }
        }

        // User-driven: trust agent.thinking_enabled. If the provider
        // rejects the reasoning payload, the LLM client surfaces the error
        // through the normal tool_error path.
        let thinking_enabled = resolve_thinking_flag(agent.thinking_enabled, &agent.model);

        let mut effective_max_output = agent.max_tokens;
        if let Some(provider_max) = provider.effective_max_output(&agent.model) {
            if provider_max > 0 && (effective_max_output as u64) > provider_max {
                tracing::warn!(
                    agent = %agent.id,
                    model = %agent.model,
                    requested = effective_max_output,
                    clamped_to = provider_max,
                    "Clamped max_tokens to provider model config limit"
                );
                effective_max_output = provider_max as u32;
            }
        }

        let effective_max_input = resolve_effective_max_input(agent, provider);

        // Create LLM client using provider config
        let llm_config = LlmConfig::new(
            provider.base_url.clone(),
            provider.api_key.clone(),
            agent.model.clone(),
            provider.id.clone().unwrap_or_else(|| provider.name.clone()),
        )
        .with_temperature(agent.temperature)
        .with_max_tokens(effective_max_output)
        .with_thinking(thinking_enabled);

        let rig_agent_config = match remote_prompt {
            Some(prompt) => RigAgentConfig::new(
                "remote-peer",
                "Remote A2A responder",
                "Bounded public A2A skill execution",
                prompt.system_instruction(),
                RigModelConfig::from_llm_config(&llm_config, effective_max_input),
            ),
            None => build_rig_agent_config(agent, &llm_config, effective_max_input),
        };

        let raw_client: Arc<dyn agent_runtime::LlmClient> = Arc::new(
            OpenAiClient::new(llm_config)
                .map_err(|e| format!("Failed to create LLM client: {}", e))?,
        );

        // Wrap with retry logic: 3 retries, 500ms base delay, exponential backoff with jitter
        let retrying_client: Arc<dyn agent_runtime::LlmClient> =
            Arc::new(RetryingLlmClient::new(raw_client, RetryPolicy::default()));

        // Wrap with shared rate limiter if configured (limits concurrent calls and RPM per provider)
        let llm_client: Arc<dyn agent_runtime::LlmClient> =
            if let Some(ref limiter) = self.rate_limiter {
                Arc::new(agent_runtime::RateLimitedLlmClient::new(
                    retrying_client,
                    limiter.clone(),
                ))
            } else {
                retrying_client
            };
        // Stream decode errors are handled by openai.rs fallback (stream error → retry non-streaming).
        // All agents stream — no NonStreamingLlmClient wrapper needed.

        // Create file system context for tools
        let fs_context: Arc<dyn FileSystemContext> =
            Arc::new(GatewayFileSystem::new(self.vault_dir.clone()));

        // Build tool registry. The authorization context is constructed from
        // immutable executor/session inputs, not model-controlled tool state.
        let recall_authorization = self.memory_recall.as_ref().and_then(|recall| {
            crate::invoke::unified_recall_adapter::recall_authorization_context(
                recall,
                agent.id.clone(),
                self.actor_kind.as_state_value(),
                session_id,
                ward_id,
            )
        });
        let tool_registry = self.build_tool_registry_with_recall(fs_context, recall_authorization);

        // Build MCP manager
        let (mcp_manager, authorized_mcp_ids) = if remote_prompt.is_some() {
            (Arc::new(McpManager::new()), Vec::new())
        } else {
            self.build_mcp_manager(agent, mcp_service).await
        };

        // Build final executor config with system instruction
        executor_config.system_instruction = Some(match remote_prompt {
            Some(prompt) => prompt.system_instruction().to_string(),
            None => match ward_template_prompt {
                Some(template) => format!("{}\n\n{}", agent.instructions, template),
                None => agent.instructions.clone(),
            },
        });
        executor_config.conversation_id = Some(conversation_id.to_string());
        executor_config.temperature = agent.temperature;
        executor_config.max_tokens = effective_max_output;
        executor_config.context_window_tokens = effective_max_input;
        executor_config.mcps = authorized_mcp_ids;

        // Create middleware pipeline after context_window_tokens is resolved.
        let middleware_pipeline = build_runtime_middleware_pipeline(
            executor_config.context_window_tokens,
            self.chat_mode,
            Some(llm_client.clone()),
        );

        // Root is an orchestrator — enforce single action per turn (except chat mode)
        if matches!(self.actor_kind, RuntimeActorKind::Root) && !self.chat_mode {
            executor_config.single_action_mode = true;
        }

        // Chat mode: nudge at 70% so agent saves facts before 80% middleware prune
        if self.chat_mode {
            executor_config.compaction_warn_pct = 70;
        }

        // Wire execution hooks for subagents (code-agent, research-agent, etc.)
        if self.actor_kind.is_delegated_execution() {
            // beforeToolCall: block shell-as-file-writer bypass
            executor_config.before_tool_call = Some(Arc::new(|tool_name, args| {
                if tool_name == "shell" {
                    let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    // Block shell commands that create/write files — use write_file instead
                    if cmd.contains("> ")
                        || cmd.contains("cat <<")
                        || cmd.contains("heredoc")
                        || cmd.contains("echo \"") && cmd.contains("> ")
                        || cmd.contains("printf") && cmd.contains("> ")
                        || cmd.contains("tee ")
                    {
                        return ToolCallDecision::Block {
                            reason: "Use write_file to create files, not shell redirects. Shell is for running commands and reading output.".to_string()
                        };
                    }
                }
                ToolCallDecision::Allow
            }));

            // afterToolCall: inject guidance after errors to reduce fix-retry loops
            executor_config.after_tool_call = Some(Arc::new(
                |tool_name, _args, result, succeeded| {
                    if !succeeded && tool_name == "shell" {
                        Some(format!(
                            "{}\n\n[SYSTEM: Command failed. Read the error. Fix the ROOT CAUSE in your code, \
                         not the symptom. Do not retry the same command — fix the file first with edit_file.]",
                            result
                        ))
                    } else if !succeeded {
                        Some(format!(
                            "{}\n\n[SYSTEM: Tool failed. Read the error carefully before retrying.]",
                            result
                        ))
                    } else {
                        None // Pass through unchanged
                    }
                },
            ));
        }

        // Configure tool result offload settings
        executor_config.offload_large_results = self.tool_settings.offload_large_results;
        executor_config.offload_threshold_chars = self.tool_settings.offload_threshold_tokens * 4;
        executor_config.offload_dir = Some(self.vault_dir.join("temp"));

        let mut prepared = PreparedExecution::new(
            executor_config,
            llm_client,
            tool_registry,
            mcp_manager,
            middleware_pipeline,
        );
        prepared.rig_config = Some(rig_agent_config);
        prepared
            .resolve_mcp_tools()
            .await
            .map_err(|error| error.to_string())?;
        Ok(prepared)
    }

    /// Build a registry without session-scoped model recall (catalog/tests).
    fn build_tool_registry(&self, fs_context: Arc<dyn FileSystemContext>) -> Arc<ToolRegistry> {
        self.build_tool_registry_with_recall(fs_context, None)
    }

    /// Build the tool registry with core and optional tools.
    fn build_tool_registry_with_recall(
        &self,
        fs_context: Arc<dyn FileSystemContext>,
        recall_authorization: Option<RecallAuthorizationContext>,
    ) -> Arc<ToolRegistry> {
        let mut tool_registry = ToolRegistry::new();
        let actor = self.actor_kind;

        fn register_if_allowed(
            registry: &mut ToolRegistry,
            actor: RuntimeActorKind,
            capabilities: &[ToolCapability],
            tool: Arc<dyn agent_primitives::Tool>,
        ) {
            if actor_allows_all(actor, capabilities) {
                registry.register(tool);
            }
        }

        let unified_recall_binding = self
            .memory_recall
            .clone()
            .zip(recall_authorization)
            .filter(|(recall, _)| recall.provider_scope().is_some());
        let ward_layout: Arc<dyn agent_tools::WardLayoutAccess> =
            Arc::new(match self.ward_usage_service.clone() {
                Some(usage) => super::ward_layout_adapter::GatewayWardLayoutAccess::with_usage(
                    self.vault_dir.clone(),
                    usage,
                ),
                None => {
                    super::ward_layout_adapter::GatewayWardLayoutAccess::new(self.vault_dir.clone())
                }
            });
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::Shell],
            Arc::new(ShellTool::new().with_filesystem(fs_context.clone())),
        );
        {
            let mut wt = WriteFileTool::new(fs_context.clone());
            if let Some(fs) = self.fact_store.clone() {
                wt = wt.with_fact_store(fs);
            }
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::FileWrite],
                Arc::new(wt),
            );
        }
        {
            let mut et = EditFileTool::new(fs_context.clone());
            if let Some(fs) = self.fact_store.clone() {
                et = et.with_fact_store(fs);
            }
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::FileWrite],
                Arc::new(et),
            );
        }
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::SkillLoad],
            Arc::new(LoadSkillTool::new(fs_context.clone())),
        );
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::FileRead],
            Arc::new(ReadTool::new(fs_context.clone())),
        );
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::WardRead, ToolCapability::WardWrite],
            Arc::new({
                let audience = self.ward_audience_override.unwrap_or(
                    if matches!(actor, RuntimeActorKind::Root) {
                        agent_tools::WardAudience::Root
                    } else {
                        agent_tools::WardAudience::Subagent
                    },
                );
                match audience {
                    agent_tools::WardAudience::Root => WardTool::for_root(
                        fs_context.clone(),
                        self.fact_store.clone(),
                        self.ward_usage.clone(),
                        ward_layout,
                    ),
                    agent_tools::WardAudience::Planner => WardTool::for_planner(
                        fs_context.clone(),
                        self.fact_store.clone(),
                        self.ward_usage.clone(),
                        ward_layout,
                    ),
                    _ => WardTool::for_subagent(
                        fs_context.clone(),
                        self.fact_store.clone(),
                        self.ward_usage.clone(),
                        ward_layout,
                    ),
                }
            }),
        );
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::MemoryRead, ToolCapability::MemoryWrite],
            Arc::new({
                let tool = MemoryTool::new(fs_context.clone(), self.fact_store.clone())
                    .with_optional_evidence_intake(self.ingestion_adapter.clone());
                match &unified_recall_binding {
                    Some((recall, authorization)) => tool.with_unified_recall(
                        crate::invoke::unified_recall_adapter::unified_recall_binding_with_goals(
                            recall.clone(),
                            self.goal_adapter.clone(),
                            authorization.clone(),
                        ),
                    ),
                    None => tool,
                }
            }),
        );
        if let Some((recall, authorization)) = unified_recall_binding {
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::MemoryRead],
                Arc::new(
                    crate::invoke::unified_recall_adapter::unified_recall_tool_with_goals(
                        recall,
                        self.goal_adapter.clone(),
                        authorization,
                    ),
                ),
            );
        }
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::MemoryWrite],
            Arc::new(
                agent_tools::MemoryWriteTool::new(fs_context.clone(), self.fact_store.clone())
                    .with_optional_evidence_intake(self.ingestion_adapter.clone()),
            ),
        );
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::PlanWrite],
            Arc::new(UpdatePlanTool::new()),
        );
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::SurfacePresent],
            Arc::new(crate::tools::PresentSurfaceTool::new()),
        );
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::Respond],
            Arc::new(RespondTool::new()),
        );
        if !self.is_step_executor() {
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::AgentDelegate],
                Arc::new(DelegateTool::new()),
            );
        }
        if actor_allows(actor, ToolCapability::PeerDelegate) {
            if let Some(ref service) = self.a2a_delegation {
                tool_registry.register(Arc::new(crate::tools::ListZbotsTool::new(service.clone())));
                tool_registry.register(Arc::new(crate::tools::DelegateToZbotTool::new(
                    service.clone(),
                )));
            }
        }
        if self.has_planner_capability_catalog() && !self.is_step_executor() {
            tool_registry.register(Arc::new(agent_runtime::tools::CapabilityCatalogTool::new()));
        }
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::MultimodalAnalyze],
            Arc::new(MultimodalAnalyzeTool::new()),
        );

        if actor_allows(actor, ToolCapability::ProcedureRun) {
            if let Some(procedure_store) = self.procedure_store.clone() {
                let mut dispatch_registry = ToolRegistry::new();
                for t in tool_registry.get_all() {
                    dispatch_registry.register(t.clone());
                }
                let dispatch_arc = Arc::new(dispatch_registry);
                let run_procedure = agent_runtime::tools::run_procedure::RunProcedureTool::new(
                    dispatch_arc,
                    procedure_store,
                );
                tool_registry.register(Arc::new(run_procedure));
            }
        }

        if actor_allows(actor, ToolCapability::AgentControl) && !self.is_step_executor() {
            if let Some(ref svc) = self.state_service {
                tool_registry.register(Arc::new(crate::tools::ListSessionAgentsTool::new(
                    svc.clone(),
                )));
            }

            if let (Some(ref svc), Some(ref sr)) = (&self.state_service, &self.steering_registry) {
                tool_registry.register(Arc::new(crate::tools::HandoffToAgentTool::new(
                    svc.clone(),
                    sr.clone(),
                )));
            }

            if let Some(ref peer_messages) = self.peer_messages {
                tool_registry.register(Arc::new(crate::tools::MessageAgentTool::new(
                    peer_messages.clone(),
                )));
            }

            if let Some(ref sr) = self.steering_registry {
                tool_registry.register(Arc::new(crate::tools::SteerAgentTool::new(sr.clone())));
            }

            if let (Some(ref bus), Some(ref svc), Some(ref messages)) =
                (&self.agent_result_bus, &self.state_service, &self.messages)
            {
                tool_registry.register(Arc::new(crate::tools::WaitAgentTool::new(
                    bus.clone(),
                    svc.clone(),
                    messages.clone(),
                )));
                tool_registry.register(Arc::new(crate::tools::KillAgentTool::new(bus.clone())));
            }
        }

        if actor_allows(actor, ToolCapability::AgentReply) {
            if let Some(ref peer_messages) = self.peer_messages {
                tool_registry.register(Arc::new(crate::tools::ReplyToAgentTool::new(
                    peer_messages.clone(),
                )));
            }
        }

        if actor_allows(actor, ToolCapability::GraphRead) {
            if let Some(ref ks) = self.kg_store {
                let adapter = Arc::new(super::kg_store_adapter::KgStoreAdapter::new(ks.clone()));
                tool_registry.register(Arc::new(GraphQueryTool::new(adapter)));
            }
        }

        if actor_allows(actor, ToolCapability::IngestWrite) {
            if let Some(ref a) = self.ingestion_adapter {
                tool_registry.register(Arc::new(agent_tools::IngestTool::new(a.clone())));
            }
        }

        if actor_allows(actor, ToolCapability::GoalWrite) {
            if let Some(ref a) = self.goal_adapter {
                tool_registry.register(Arc::new(agent_tools::GoalTool::new(a.clone())));
            }
        }

        if self.tool_settings.file_tools
            || matches!(
                actor,
                RuntimeActorKind::DelegatedReviewer | RuntimeActorKind::WardAgent
            )
        {
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::FileRead],
                Arc::new(GlobTool),
            );
        }

        if let Some(provider) = &self.connector_provider {
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::ConnectorQuery],
                Arc::new(
                    QueryResourceTool::new(provider.clone())
                        .with_optional_evidence_intake(self.ingestion_adapter.clone()),
                ),
            );
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::ConnectorResourceRead],
                Arc::new(
                    ConnectorResourceTool::new(provider.clone())
                        .with_optional_evidence_intake(self.ingestion_adapter.clone()),
                ),
            );
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::ConnectorInvoke],
                Arc::new(ConnectorInvokeTool::new(provider.clone())),
            );
        }

        Arc::new(tool_registry)
    }

    /// Build the MCP manager and start configured servers.
    async fn build_mcp_manager(
        &self,
        agent: &Agent,
        mcp_service: &McpService,
    ) -> (Arc<McpManager>, Vec<String>) {
        let mut mcp_manager = McpManager::new();
        if let Some(observer) = self.mcp_startup_failure_observer.clone() {
            mcp_manager = mcp_manager.with_startup_failure_observer(observer);
        }
        let mcp_manager = Arc::new(mcp_manager);
        let mut authorized_ids = Vec::new();

        // Load and start MCP servers configured for this agent
        if !agent.mcps.is_empty() {
            let mcp_configs = mcp_service.get_multiple_for_runtime(&agent.mcps);
            for mcp_config in mcp_configs {
                let server_id = mcp_config.id();
                // The service accepts display-name aliases; runtime inventory and
                // dispatch must use the same canonical identity as the manager.
                authorized_ids.push(server_id.clone());
                tracing::info!("Starting MCP server: {}", server_id);
                if mcp_manager.start_server(mcp_config).await.is_err() {
                    // Fail closed for this executor: no tool registration and
                    // no retry. Keep provider/error details out of logs.
                    tracing::warn!(
                        mcp_id = %server_id,
                        rejection_code = "startup_failed",
                        "MCP server startup failed; continuing without its tools"
                    );
                    mcp_manager.notify_startup_failure(&server_id);
                }
            }
        }

        (mcp_manager, authorized_ids)
    }
}

/// Build a host-side audit observer for MCP startup/discovery failures. The
/// runtime boundary supplies a canonical ID only, so config errors, commands,
/// URLs, and server stderr cannot enter execution logs.
pub(crate) fn mcp_startup_failure_observer(
    log_service: Arc<LogService<DatabaseManager>>,
    execution_id: impl Into<String>,
    session_id: impl Into<String>,
    agent_id: impl Into<String>,
) -> agent_runtime::mcp::McpStartupFailureObserver {
    let execution_id = execution_id.into();
    let session_id = session_id.into();
    let agent_id = agent_id.into();
    Arc::new(move |mcp_id| {
        let entry = ExecutionLog::new(
            &execution_id,
            &session_id,
            &agent_id,
            LogLevel::Info,
            LogCategory::Intent,
            "MCP capability startup failed",
        )
        .with_metadata(serde_json::json!({
            "origin": "mcp_startup",
            "effective_mcps": [],
            "startup_failed_mcps": [mcp_id],
            "unresolved_count": 1,
            "rejection_codes": ["startup_failed"],
        }));
        if log_service.log(entry).is_err() {
            tracing::debug!(mcp_id, "Failed to persist MCP startup audit event");
        }
    })
}

/// Helper to collect available agents summary for executor state.
pub async fn collect_agents_summary(
    agent_service: &gateway_services::AgentService,
    paths: &SharedVaultPaths,
) -> Vec<serde_json::Value> {
    let mut summaries = match agent_service.list().await {
        Ok(all_agents) => all_agents
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": a.display_name,
                    "description": a.description
                })
            })
            .collect(),
        Err(_) => vec![],
    };

    let wards_dir = paths.wards_dir();
    let wards_root_is_real = std::fs::symlink_metadata(&wards_dir)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    if wards_root_is_real {
        if let Ok(entries) = std::fs::read_dir(wards_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let valid_name = !name.is_empty()
                    && name.len() <= 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
                let real_directory = entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink());
                if valid_name && real_directory {
                    summaries.push(serde_json::json!({
                        "id": format!("ward:{name}"),
                        "name": format!("Ward Agent: {name}"),
                        "description": format!("Delegatable agent for the existing {name} ward"),
                    }));
                }
            }
        }
    }

    summaries.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    summaries
}

/// Helper to collect available skills summary for executor state.
pub async fn collect_skills_summary(skill_service: &SkillService) -> Vec<serde_json::Value> {
    match skill_service.list().await {
        Ok(all_skills) => all_skills
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                })
            })
            .collect(),
        Err(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::connectors::{CapabilityInfo, ConnectorInfo, ResourceInfo};
    use agent_primitives::{Tool, ToolContext as ToolContextTrait};
    use agent_runtime::llm::{ChatResponse, LlmError, StreamCallback};
    use agent_runtime::tools::ToolContext as RuntimeToolContext;
    use agent_tools::{RecallVisibilityScope, WriteFileTool};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault dirs");
        gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
        gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
        gateway_services::create_ward_from_template(&paths, "broken").expect("ward");
        std::fs::write(paths.ward_layout_snapshot("broken"), "not: [valid")
            .expect("corrupt snapshot");
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let expected = crate::invoke::ward_layout_adapter::GatewayWardLayoutAccess::new(
            dir.path().to_path_buf(),
        )
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let fs: Arc<dyn FileSystemContext> =
            Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let loaded =
            gateway_services::load_ward_layout(&paths.ward_layout_snapshot("ephemeral-only"))
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
        let fs: Arc<dyn FileSystemContext> =
            Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let fs: Arc<dyn FileSystemContext> =
            Arc::new(GatewayFileSystem::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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

        assert_eq!(error, "planner_template_unavailable");
    }

    #[tokio::test]
    async fn planner_executor_fails_closed_when_selected_template_is_invalid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault dirs");
        gateway_services::seed_default_ward_layout_template(&paths).expect("layout template");
        gateway_services::seed_default_ward_agent_template(&paths).expect("agent template");
        gateway_services::create_ward_from_template(&paths, "broken").expect("ward");
        std::fs::write(paths.ward_layout_snapshot("broken"), "not: [valid")
            .expect("corrupt snapshot");
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

        assert_eq!(error, "planner_template_unavailable");
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
            error.contains("rig_execution_config_unresolved"),
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
            ) -> Result<agent_runtime::llm::ChatResponse, agent_runtime::llm::LlmError>
            {
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
            ) -> Result<agent_runtime::llm::ChatResponse, agent_runtime::llm::LlmError>
            {
                Ok(agent_runtime::llm::ChatResponse {
                    content: String::new(),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault dirs");
        let mcp_service = McpService::new(paths);
        let mut agent = sample_agent();
        agent.mcps.clear();
        agent.skills.clear();
        let provider = sample_provider();
        let mut recall = gateway_memory::MemoryRecall::new(
            None,
            Arc::new(gateway_memory::RecallConfig::default()),
        );
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
                        "glob",
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
                        "glob",
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
                    (RuntimeActorKind::DelegatedReviewer, _) => &[
                        "glob",
                        "load_skill",
                        "multimodal_analyze",
                        "read",
                        "respond",
                    ],
                    (RuntimeActorKind::WardAgent, _) => &[
                        "delegate_to_agent",
                        "edit_file",
                        "glob",
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
        let ctx: Arc<dyn agent_primitives::ToolContext> =
            Arc::new(agent_runtime::ToolContext::new());
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
            let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
            let mut prepared =
                ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
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
        let ctx: Arc<dyn agent_primitives::ToolContext> =
            Arc::new(agent_runtime::ToolContext::new());
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
        let mut recall = gateway_memory::MemoryRecall::new(
            None,
            Arc::new(gateway_memory::RecallConfig::default()),
        );
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
            .build_context_capability_catalog(
                Some("session-1".to_string()),
                Some("root".to_string()),
            );
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

        let mut recall = gateway_memory::MemoryRecall::new(
            None,
            Arc::new(gateway_memory::RecallConfig::default()),
        );
        recall.set_provider_scope(gateway_memory::RecallProviderScope::new(
            "tenant-a".to_string(),
            true,
            false,
        ));
        let recall = Arc::new(recall);
        let available = ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
            .with_memory_recall(recall)
            .build_context_capability_catalog(
                Some("session-1".to_string()),
                Some("root".to_string()),
            );
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
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
        assert_has(&ids, &["read", "glob", "respond", "load_skill"]);
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

        let executor_catalog =
            catalog_for_actor_with_join_deps(RuntimeActorKind::DelegatedExecutor);
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

        assert_has(&names, &["read", "glob", "respond", "load_skill"]);
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
                "glob",
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
            !default_visible_for_tool(name, RuntimeActorKind::Root),
            "{name} should move behind resource/context packet lanes"
        );
        assert_eq!(
            visibility_policy_for_tool(name, RuntimeActorKind::Root),
            "hidden_from_model_use_context_resources"
        );
        assert!(split_target_for_tool(name).is_some());

        assert!(!default_visible_for_tool(
            "query_resource",
            RuntimeActorKind::Root
        ));
        assert_eq!(
            visibility_policy_for_tool("query_resource", RuntimeActorKind::Root),
            "hidden_from_model_use_connector_split"
        );
        assert!(split_target_for_tool("query_resource").is_some());

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
        let planner_registry =
            ExecutorBuilder::new(dir.path().to_path_buf(), ToolSettings::default())
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
}
