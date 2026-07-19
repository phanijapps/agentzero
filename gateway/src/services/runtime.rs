//! # Runtime Service
//!
//! Service for managing agent execution runtime.
//!
//! This service coordinates agent execution through the ExecutionRunner
//! and provides a high-level API for invoking agents.

use crate::connectors::ConnectorRegistry;
use crate::events::{EventBus, GatewayEvent};
use crate::execution::{
    ExecutionConfig, ExecutionHandle, ExecutionRunner, MemoryRecall, SessionDistiller,
};
use crate::hooks::HookContext;
use crate::services::{AgentService, McpService, ProviderService, SharedVaultPaths, SkillService};
use api_logs::LogService;
use execution_state::StateService;
use std::sync::Arc;
use zbot_runtime_sqlite::DatabaseManager;

/// Execution state for a conversation.
#[derive(Debug, Clone)]
pub struct ExecutionState {
    pub agent_id: String,
    pub conversation_id: String,
    pub is_running: bool,
    pub iteration: u32,
    pub max_iterations: u32,
    pub stop_requested: bool,
}

/// Runtime service for managing agent execution.
pub struct RuntimeService {
    /// Event bus for broadcasting events.
    event_bus: Arc<EventBus>,

    /// Execution runner (optional - set when paths is known)
    runner: Option<Arc<ExecutionRunner>>,

    /// Vault paths
    paths: Option<SharedVaultPaths>,
}

impl RuntimeService {
    /// Create a new runtime service.
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            event_bus,
            runner: None,
            paths: None,
        }
    }

    /// Create a runtime service with an execution runner.
    #[allow(clippy::too_many_arguments)]
    pub fn with_runner(
        event_bus: Arc<EventBus>,
        agent_service: Arc<AgentService>,
        provider_service: Arc<ProviderService>,
        paths: SharedVaultPaths,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
        checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
        mcp_service: Arc<McpService>,
        skill_service: Arc<SkillService>,
        log_service: Arc<LogService<DatabaseManager>>,
        state_service: Arc<StateService<DatabaseManager>>,
    ) -> Self {
        let memory_llm_factory: Arc<dyn gateway_memory::MemoryLlmFactory> = Arc::new(
            crate::memory_llm_factory::ProviderServiceLlmFactory::new(provider_service.clone()),
        );
        Self::with_runner_and_connectors(
            event_bus,
            agent_service,
            provider_service,
            paths,
            messages,
            session_meta,
            checkpoints,
            mcp_service,
            skill_service,
            log_service,
            state_service,
            None,
            None, // memory_store
            None, // distiller
            None, // memory_recall
            None, // bridge_registry
            None, // bridge_outbox
            None, // embedding_client
            2,    // default max_parallel_agents
            None, // kg_store
            None, // kg_episode_store
            None, // ingestion_adapter
            None, // goal_adapter
            None, // procedure_store
            gateway_memory::ProcedureRecommendationConfig::default(),
            memory_llm_factory,
        )
    }

    /// Create a runtime service with execution runner and connector registry.
    #[allow(clippy::too_many_arguments)]
    pub fn with_runner_and_connectors(
        event_bus: Arc<EventBus>,
        agent_service: Arc<AgentService>,
        provider_service: Arc<ProviderService>,
        paths: SharedVaultPaths,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
        checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
        mcp_service: Arc<McpService>,
        skill_service: Arc<SkillService>,
        log_service: Arc<LogService<DatabaseManager>>,
        state_service: Arc<StateService<DatabaseManager>>,
        connector_registry: Option<Arc<ConnectorRegistry>>,
        memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
        distiller: Option<Arc<SessionDistiller>>,
        memory_recall: Option<Arc<MemoryRecall>>,
        bridge_registry: Option<Arc<gateway_bridge::BridgeRegistry>>,
        bridge_outbox: Option<Arc<gateway_bridge::OutboxRepository>>,
        embedding_client: Option<Arc<dyn agent_runtime::llm::embedding::EmbeddingClient>>,
        max_parallel_agents: u32,
        kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
        kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>>,
        ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
        goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
        procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
        procedure_recommendation_cfg: gateway_memory::ProcedureRecommendationConfig,
        memory_llm_factory: Arc<dyn gateway_memory::MemoryLlmFactory>,
    ) -> Self {
        let handoff_writer = memory_store.as_ref().map(|fs| {
            let llm = Arc::new(gateway_execution::sleep::LlmHandoffWriter::new(
                memory_llm_factory.clone(),
            ));
            Arc::new(gateway_execution::sleep::HandoffWriter::new(
                llm,
                fs.clone(),
                messages.clone(),
            ))
        });

        let mut runner = ExecutionRunner::with_config(gateway_execution::ExecutionRunnerConfig {
            event_bus: event_bus.clone(),
            agent_service,
            provider_service,
            paths: paths.clone(),
            messages,
            session_meta,
            checkpoints,
            mcp_service,
            skill_service,
            log_service,
            state_service,
            connector_registry,
            memory_store,
            distiller,
            handoff_writer,
            memory_recall,
            bridge_registry,
            bridge_outbox,
            embedding_client,
            procedure_store,
            procedure_recommendation_cfg,
            max_parallel_agents,
            ward_usage: Arc::new(gateway_services::WardUsage::new(paths.wards_dir())),
        });

        // Initialize fallback-only model metadata registry.
        runner.set_model_registry(Arc::new(gateway_services::models::ModelRegistry::load()));

        if let Some(ks) = kg_store {
            runner.set_kg_store(ks);
        }

        if let Some(repo) = kg_episode_store {
            runner.set_kg_episode_store(repo);
        }

        if let Some(a) = ingestion_adapter {
            runner.set_ingestion_adapter(a);
        }

        if let Some(a) = goal_adapter {
            runner.set_goal_adapter(a);
        }

        Self {
            event_bus,
            runner: Some(Arc::new(runner)),
            paths: Some(paths),
        }
    }

    /// Get the event bus.
    pub fn event_bus(&self) -> Arc<EventBus> {
        self.event_bus.clone()
    }

    /// Get the execution runner.
    pub fn runner(&self) -> Option<&Arc<ExecutionRunner>> {
        self.runner.as_ref()
    }

    /// Invoke an agent with a message.
    ///
    /// Returns (ExecutionHandle, session_id).
    /// - If session_id is provided, continues that session
    /// - If session_id is None, creates a new session
    pub async fn invoke(
        &self,
        agent_id: &str,
        conversation_id: &str,
        message: &str,
    ) -> Result<(ExecutionHandle, String), String> {
        self.invoke_with_session(agent_id, conversation_id, message, None)
            .await
    }

    /// Invoke an agent with a message and explicit session ID.
    ///
    /// Returns (ExecutionHandle, session_id).
    pub async fn invoke_with_session(
        &self,
        agent_id: &str,
        conversation_id: &str,
        message: &str,
        session_id: Option<String>,
    ) -> Result<(ExecutionHandle, String), String> {
        let runner = self.runner.as_ref().ok_or_else(|| {
            "Runtime not initialized with executor. Call with_runner() first.".to_string()
        })?;

        let paths = self
            .paths
            .clone()
            .ok_or_else(|| "Vault paths not set".to_string())?;

        let mut config = ExecutionConfig::new(
            agent_id.to_string(),
            conversation_id.to_string(),
            paths.vault_dir().clone(),
        );

        if let Some(sid) = session_id {
            config = config.with_session_id(sid);
        }

        runner.invoke(config, message.to_string()).await
    }

    /// Start a fresh execution for a server-validated decision-thread packet.
    /// This deliberately does not accept a session id, caller metadata, or a
    /// caller-controlled message, so it cannot reuse generic session resume.
    pub async fn invoke_ledger_resume(
        &self,
        agent_id: &str,
        conversation_id: &str,
        packet: zbot_conversation::LedgerResumePacket,
    ) -> Result<(ExecutionHandle, String), String> {
        let runner = self.runner.as_ref().ok_or_else(|| {
            "Runtime not initialized with executor. Call with_runner() first.".to_string()
        })?;
        let paths = self
            .paths
            .clone()
            .ok_or_else(|| "Vault paths not set".to_string())?;
        let config = ExecutionConfig::new(
            agent_id.to_string(),
            conversation_id.to_string(),
            paths.vault_dir().clone(),
        )
        .with_ledger_resume_packet(packet);
        runner
            .invoke(
                config,
                "Continue the explicitly selected approved decision thread using the saved next action."
                    .to_string(),
            )
            .await
    }

    /// Invoke an agent with a message and hook context.
    ///
    /// The hook context is passed to tools so they can route responses
    /// back to the originating channel (WebSocket, webhook, etc).
    pub async fn invoke_with_hook(
        &self,
        agent_id: &str,
        conversation_id: &str,
        message: &str,
        hook_context: HookContext,
        session_id: Option<String>,
    ) -> Result<(ExecutionHandle, String), String> {
        let runner = self.runner.as_ref().ok_or_else(|| {
            "Runtime not initialized with executor. Call with_runner() first.".to_string()
        })?;

        let paths = self
            .paths
            .clone()
            .ok_or_else(|| "Vault paths not set".to_string())?;

        let mut config = ExecutionConfig::new(
            agent_id.to_string(),
            conversation_id.to_string(),
            paths.vault_dir().clone(),
        )
        .with_hook_context(hook_context);

        if let Some(sid) = session_id {
            config = config.with_session_id(sid);
        }

        runner.invoke(config, message.to_string()).await
    }

    /// Invoke an agent with hook context and a session-ready callback.
    ///
    /// The callback fires after session creation but before any events are
    /// emitted, allowing the caller to subscribe before intent analysis fires.
    #[allow(clippy::too_many_arguments)]
    pub async fn invoke_with_hook_and_callback(
        &self,
        agent_id: &str,
        conversation_id: &str,
        message: &str,
        hook_context: HookContext,
        session_id: Option<String>,
        on_session_ready: Option<gateway_execution::OnSessionReady>,
        mode: Option<String>,
        client_message_id: Option<String>,
    ) -> Result<(ExecutionHandle, String), String> {
        let runner = self.runner.as_ref().ok_or_else(|| {
            "Runtime not initialized with executor. Call with_runner() first.".to_string()
        })?;

        let paths = self
            .paths
            .clone()
            .ok_or_else(|| "Vault paths not set".to_string())?;

        let mut config = ExecutionConfig::new(
            agent_id.to_string(),
            conversation_id.to_string(),
            paths.vault_dir().clone(),
        )
        .with_hook_context(hook_context);

        if let Some(sid) = session_id {
            config = config.with_session_id(sid);
        }

        if let Some(m) = mode {
            config = config.with_mode(m);
        }

        if let Some(client_message_id) = client_message_id {
            config = config.with_client_message_id(client_message_id);
        }

        runner
            .invoke_with_callback(config, message.to_string(), on_session_ready)
            .await
    }

    /// Invoke with a placeholder response (for testing without LLM).
    pub async fn invoke_placeholder(
        &self,
        agent_id: &str,
        conversation_id: &str,
        message: &str,
    ) -> Result<(), String> {
        // Emit start event
        let placeholder_session_id = format!("placeholder-{}", uuid::Uuid::new_v4());
        let placeholder_execution_id = format!("exec-placeholder-{}", uuid::Uuid::new_v4());
        self.event_bus
            .publish(GatewayEvent::AgentStarted {
                agent_id: agent_id.to_string(),
                session_id: placeholder_session_id.clone(),
                execution_id: placeholder_execution_id.clone(),
                conversation_id: Some(conversation_id.to_string()),
            })
            .await;

        // Emit a placeholder completion event after a short delay
        let event_bus = self.event_bus.clone();
        let agent_id = agent_id.to_string();
        let conversation_id = conversation_id.to_string();
        let message = message.to_string();

        tokio::spawn(async move {
            // Simulate processing
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Emit completion
            event_bus
                .publish(GatewayEvent::AgentCompleted {
                    agent_id: agent_id.clone(),
                    session_id: placeholder_session_id.clone(),
                    execution_id: placeholder_execution_id.clone(),
                    result: Some(format!(
                        "Gateway placeholder response. Set OPENAI_API_KEY for real execution. Message: {}",
                        message.chars().take(50).collect::<String>()
                    )),
                    conversation_id: Some(conversation_id.clone()),
                })
                .await;
        });

        Ok(())
    }

    /// Stop an agent execution.
    pub async fn stop(&self, conversation_id: &str) -> Result<(), String> {
        if let Some(runner) = &self.runner {
            runner.stop(conversation_id).await
        } else {
            Err("Runtime not initialized with executor".to_string())
        }
    }

    /// Continue an agent execution after max iterations.
    pub async fn continue_execution(
        &self,
        conversation_id: &str,
        additional_iterations: u32,
    ) -> Result<(), String> {
        if let Some(runner) = &self.runner {
            runner
                .continue_execution(conversation_id, additional_iterations)
                .await
        } else {
            Err("Runtime not initialized with executor".to_string())
        }
    }

    /// Pause an agent execution.
    pub async fn pause(&self, session_id: &str) -> Result<(), String> {
        if let Some(runner) = &self.runner {
            runner.pause(session_id).await
        } else {
            Err("Runtime not initialized with executor".to_string())
        }
    }

    /// Resume a paused agent execution.
    pub async fn resume(&self, session_id: &str) -> Result<(), String> {
        if let Some(runner) = &self.runner {
            runner.resume(session_id).await
        } else {
            Err("Runtime not initialized with executor".to_string())
        }
    }

    /// Cancel an agent execution.
    pub async fn cancel(&self, session_id: &str) -> Result<(), String> {
        if let Some(runner) = &self.runner {
            runner.cancel(session_id).await
        } else {
            Err("Runtime not initialized with executor".to_string())
        }
    }

    /// End a session (mark as completed).
    ///
    /// Called when user explicitly ends a session via /end, /new, or +new button.
    pub async fn end_session(&self, session_id: &str) -> Result<(), String> {
        if let Some(runner) = &self.runner {
            runner.end_session(session_id).await
        } else {
            Err("Runtime not initialized with executor".to_string())
        }
    }

    /// Get execution handle for a conversation.
    pub async fn get_handle(&self, conversation_id: &str) -> Option<ExecutionHandle> {
        if let Some(runner) = &self.runner {
            runner.get_handle(conversation_id).await
        } else {
            None
        }
    }

    /// Check if an agent is currently executing.
    pub async fn is_running(&self, conversation_id: &str) -> bool {
        if let Some(handle) = self.get_handle(conversation_id).await {
            !handle.is_stop_requested()
        } else {
            false
        }
    }
}

/// Create a shared runtime service.
pub fn shared_runtime_service(event_bus: Arc<EventBus>) -> Arc<RuntimeService> {
    Arc::new(RuntimeService::new(event_bus))
}

/// Create a shared runtime service with execution runner.
#[allow(clippy::too_many_arguments)]
pub fn shared_runtime_service_with_runner(
    event_bus: Arc<EventBus>,
    agent_service: Arc<AgentService>,
    provider_service: Arc<ProviderService>,
    paths: SharedVaultPaths,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    mcp_service: Arc<McpService>,
    skill_service: Arc<SkillService>,
    log_service: Arc<LogService<DatabaseManager>>,
    state_service: Arc<StateService<DatabaseManager>>,
) -> Arc<RuntimeService> {
    Arc::new(RuntimeService::with_runner(
        event_bus,
        agent_service,
        provider_service,
        paths,
        messages,
        session_meta,
        checkpoints,
        mcp_service,
        skill_service,
        log_service,
        state_service,
    ))
}
