//! Executor builder — assembles PreparedExecution for every actor path.
//!
//! Tool capability gating lives in [`super::policy`]; per-tool metadata is
//! data in [`super::tool_catalog`].

use crate::errors::ExecutionError;
use agent_primitives::vault_paths::SharedVaultPaths;
use agent_primitives::vault_paths::VaultPaths;
use agent_primitives::{ConnectorResourceProvider, FileSystemContext};
use agent_runtime::{
    ContextCapability, ContextCapabilityCatalog, ContextCapabilityHealth, ContextCapabilityKind,
    ContextEditingConfig, ContextEditingMiddleware, DelegateTool, ExecutorConfig, KeepPolicy,
    LlmClient, LlmConfig, McpManager, MiddlewarePipeline, OpenAiClient, PlanBlockMiddleware,
    PreparedExecution, RespondTool, RetryPolicy, RetryingLlmClient, RigAgentConfig, RigModelConfig,
    SummarizationConfig, SummarizationMiddleware, ToolRegistry, TriggerCondition,
};
use agent_tools::{
    ConnectorInvokeTool, ConnectorResourceTool, EditFileTool, GlobTool, GraphQueryTool,
    LoadSkillTool, MemoryTool, MultimodalAnalyzeTool, QueryResourceTool, ReadTool,
    RecallAuthorizationContext, ShellTool, ToolSettings, UpdatePlanTool, WardTool, WriteFileTool,
};
use api_logs::{ExecutionLog, LogCategory, LogLevel, LogService};
use execution_state::StateService;
use gateway_services::agents::Agent;
use gateway_services::models::{ModelRegistry, DEFAULT_MAX_INPUT_TOKENS};
use gateway_services::providers::Provider;
use gateway_services::{McpService, SettingsService, SkillService};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use zbot_conversation::MessageStore;
use zbot_runtime_sqlite::DatabaseManager;
use zbot_stores_traits::MemoryFactStore;

use super::executor::resolve_thinking_flag;
use super::policy::{
    actor_allows, actor_allows_all, actor_capabilities, actor_policy_for_capabilities,
    context_actor_kind, RuntimeActorKind, SubagentGuardHook, ToolCapability,
};
use super::setup::SubagentRole;
use super::tool_catalog::{
    audit_policy, capabilities, cost_hint, default_visible, display_name, latency_hint,
    owner_crate, risk_level, side_effects, split_target, token_hint, visibility_policy,
    MODEL_HIDDEN_TOOLS,
};

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

pub(crate) fn resolve_effective_max_input(agent: &Agent, provider: &Provider) -> u64 {
    let provider_max_input = provider
        .effective_max_input(&agent.model)
        .or(provider.context_window);
    if agent.max_input_tokens_explicit && agent.max_input_tokens > 0 {
        agent.max_input_tokens
    } else {
        provider_max_input.unwrap_or(DEFAULT_MAX_INPUT_TOKENS)
    }
}
use crate::agent_pool::AgentResultBus;
use crate::config::GatewayFileSystem;

pub fn build_context_capability_catalog(
    actor: RuntimeActorKind,
    registry: &ToolRegistry,
    session_id: Option<String>,
    agent_id: Option<String>,
) -> ContextCapabilityCatalog {
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for tool in registry.get_all() {
        if !seen.insert(tool.name().to_string()) {
            continue;
        }
        let tool_caps = capabilities(tool.name());
        if !tool_caps.is_empty() && !actor_allows_all(actor, &tool_caps) {
            continue;
        }
        entries.push(ContextCapability {
            id: tool.name().to_string(),
            kind: ContextCapabilityKind::Tool,
            display_name: display_name(tool.name()),
            description: tool.description().to_string(),
            actor_policy: actor_policy_for_capabilities(actor, &tool_caps),
            risk_level: risk_level(tool.name()),
            side_effects: side_effects(tool.name()),
            input_schema: tool.parameters_schema(),
            output_schema: None,
            resource_uri_template: None,
            cost_hint: Some(cost_hint(tool.name())),
            latency_hint: Some(latency_hint(tool.name())),
            token_hint: token_hint(tool.name()),
            health: ContextCapabilityHealth::Available,
            owner_crate: Some(owner_crate(tool.name()).to_string()),
            audit_policy: Some(audit_policy(tool.name()).to_string()),
            default_visible: default_visible(tool.name()),
            visibility_policy: visibility_policy(tool.name()).to_string(),
            split_target: split_target(tool.name()).map(str::to_string),
        });
    }

    ContextCapabilityCatalog {
        version: "2026-07-07".to_string(),
        actor_kind: context_actor_kind(actor),
        session_id,
        agent_id,
        capabilities: entries,
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
        risk_level: risk_level("recall"),
        side_effects: side_effects("recall"),
        input_schema: Some(agent_tools::recall_parameters_schema()),
        output_schema: None,
        resource_uri_template: None,
        cost_hint: Some(cost_hint("recall")),
        latency_hint: Some(latency_hint("recall")),
        token_hint: token_hint("recall"),
        health,
        owner_crate: Some(owner_crate("recall").to_string()),
        audit_policy: Some(audit_policy("recall").to_string()),
        default_visible: available,
        visibility_policy: if available {
            "default_visible_unified_recall_exception".to_string()
        } else {
            "catalog_only_unavailable".to_string()
        },
        split_target: split_target("recall").map(str::to_string),
    }
}

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
    kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>>,
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
    /// Belief Network stores for the `belief` tool (read-only).
    belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,
    belief_contradiction_store: Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>>,
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
            belief_store: None,
            belief_contradiction_store: None,
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

    /// Wire the Belief Network stores for the `belief` tool.
    pub fn with_belief_stores(
        mut self,
        belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,
        belief_contradiction_store: Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>>,
    ) -> Self {
        self.belief_store = belief_store;
        self.belief_contradiction_store = belief_contradiction_store;
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
    pub fn with_kg_store(
        mut self,
        store: Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>,
    ) -> Self {
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
    ) -> Result<PreparedExecution, ExecutionError> {
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
        .with_model_hidden_tools(MODEL_HIDDEN_TOOLS.to_vec());

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
                return Err(ExecutionError::from(
                    "planner_template_unavailable".to_string(),
                ));
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
                        return Err(ExecutionError::from(
                            "planner_template_unavailable".to_string(),
                        ));
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
                                "apiKey": api_key }),
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
            executor_config.hooks.add(Arc::new(SubagentGuardHook));
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
            Arc::new(
                MemoryTool::new(self.fact_store.clone())
                    .with_optional_evidence_intake(self.ingestion_adapter.clone()),
            ),
        );
        if let Some((recall, authorization)) = unified_recall_binding {
            let mut tool = crate::invoke::unified_recall_adapter::unified_recall_tool_with_goals(
                recall,
                self.goal_adapter.clone(),
                authorization,
            );
            if let Some(fact_store) = &self.fact_store {
                tool = tool.with_fact_store(fact_store.clone());
            }
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::MemoryRead],
                Arc::new(tool),
            );
        }
        if self.belief_store.is_some() || self.belief_contradiction_store.is_some() {
            register_if_allowed(
                &mut tool_registry,
                actor,
                &[ToolCapability::MemoryRead],
                Arc::new(agent_tools::BeliefTool::new(
                    self.belief_store.clone(),
                    self.belief_contradiction_store.clone(),
                )),
            );
        }
        register_if_allowed(
            &mut tool_registry,
            actor,
            &[ToolCapability::MemoryWrite],
            Arc::new(
                agent_tools::MemoryWriteTool::new(self.fact_store.clone())
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
            "rejection_codes": ["startup_failed"] }));
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
                        "description": format!("Delegatable agent for the existing {name} ward") }));
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
                    "description": s.description })
            })
            .collect(),
        Err(_) => vec![],
    }
}

pub(crate) fn build_runtime_middleware_pipeline(
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

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
