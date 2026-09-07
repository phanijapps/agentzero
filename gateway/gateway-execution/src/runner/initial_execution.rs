//! Initial root invocation: setup completion and stream dispatch.
//!
//! The entry-point counterpart of `continuation_execution.rs` — the runner
//! methods that finish two-phase bootstrap and spawn the shared
//! `ExecutionStream` for a fresh root execution.

use super::core::ExecutionRunner;
use super::OnSessionReady;
use crate::config::ExecutionConfig;
use crate::handle::ExecutionHandle;
use crate::lifecycle::{crash_execution, CrashExecution};

impl ExecutionRunner {
    /// Invoke an agent with a message.
    ///
    /// Returns an execution handle for controlling the execution and the session ID.
    ///
    /// # Session Behavior
    ///
    /// - If `config.session_id` is Some: continues that session with a new execution
    /// - If `config.session_id` is None: creates a new session
    ///
    /// # Errors
    ///
    /// Returns an error if the agent or provider cannot be loaded.
    pub async fn invoke(
        &self,
        config: ExecutionConfig,
        message: String,
    ) -> Result<(ExecutionHandle, String), String> {
        self.invoke_with_callback(config, message, None).await
    }

    /// Invoke an agent with an optional session-ready callback.
    ///
    /// The callback fires after the submitted root message is durable but
    /// BEFORE any agent or intent events are emitted, so the caller's
    /// subscriber sees every event from `AgentStarted` onward.
    ///
    /// # Event ordering
    ///
    /// ```text
    /// begin_setup  [get_or_create_session, persist_routing,
    ///               persist_root_message, start_execution, store_handle,
    ///               on_session_ready CALLBACK]
    /// → finish_setup [emit_agent_started, load_agent, run_intent_analysis,
    ///                 inject_placeholder, build executor]
    /// → tokio::spawn
    /// ```
    pub async fn invoke_with_callback(
        &self,
        mut config: ExecutionConfig,
        message: String,
        on_session_ready: Option<OnSessionReady>,
    ) -> Result<(ExecutionHandle, String), String> {
        // Phase 1: create session + handle, BEFORE any events fire.
        let partial = self
            .bootstrap
            .begin_setup(&mut config, &message, on_session_ready)
            .await?;

        self.finish_initial_invoke(config, message, partial, true)
            .await
    }

    /// Invoke through the ordinary append/bootstrap path while keeping setup
    /// failures normalized for a durable-work boundary.
    pub async fn invoke_redacted_with_callback(
        &self,
        mut config: ExecutionConfig,
        message: String,
        on_session_ready: Option<OnSessionReady>,
    ) -> Result<(ExecutionHandle, String), String> {
        config = config.with_redacted_diagnostics();
        let partial = self
            .bootstrap
            .begin_setup(&mut config, &message, on_session_ready)
            .await?;
        self.finish_initial_invoke(config, message, partial, false)
            .await
    }

    /// Resume the ordinary initial invocation from an exact durable root
    /// message. Unlike delegation continuation, this runs the same phase-two
    /// bootstrap and execution stream as [`Self::invoke_with_callback`].
    pub async fn invoke_persisted_with_callback(
        &self,
        mut config: ExecutionConfig,
        message: String,
        execution_id: String,
        message_id: String,
        on_session_ready: Option<OnSessionReady>,
    ) -> Result<(ExecutionHandle, String), String> {
        config = config.with_redacted_diagnostics();
        let partial = self
            .bootstrap
            .begin_setup_from_persisted(
                &mut config,
                &message,
                &execution_id,
                &message_id,
                on_session_ready,
            )
            .await?;
        self.finish_initial_invoke(config, message, partial, false)
            .await
    }

    async fn finish_initial_invoke(
        &self,
        config: ExecutionConfig,
        message: String,
        partial: super::invoke_bootstrap::PartialSetup,
        log_internal_error: bool,
    ) -> Result<(ExecutionHandle, String), String> {
        let partial_execution_id = partial.execution_id.clone();
        let partial_session_id = partial.session_id.clone();
        let partial_handle = partial.handle.clone();
        let setup = match self
            .bootstrap
            .finish_setup(&config, &message, partial)
            .await
        {
            Ok(setup) => setup,
            Err(error) => {
                if log_internal_error {
                    tracing::error!(
                        session_id = %partial_session_id,
                        execution_id = %partial_execution_id,
                        error = %error,
                        "Invocation setup failed after execution start"
                    );
                }
                {
                    let mut handles = self.ctx.control.handles.write().await;
                    if handles
                        .get(&config.conversation_id)
                        .is_some_and(|handle| handle.is_same_execution(&partial_handle))
                    {
                        handles.remove(&config.conversation_id);
                    }
                }
                const SAFE_SETUP_ERROR: &str = "Unable to start this request";
                crash_execution(CrashExecution {
                    state_service: &self.ctx.control.state_service,
                    log_service: &self.ctx.log_service,
                    event_bus: &self.ctx.event_bus,
                    execution_id: &partial_execution_id,
                    session_id: &partial_session_id,
                    agent_id: &config.agent_id,
                    conversation_id: &config.conversation_id,
                    error: SAFE_SETUP_ERROR,
                    crash_session: true,
                })
                .await;
                return Err(SAFE_SETUP_ERROR.to_owned());
            }
        };

        let integrations = self.ctx.integrations.snapshot();
        let stream = super::execution_stream::ExecutionStream {
            event_bus: self.ctx.event_bus.clone(),
            state_service: self.ctx.control.state_service.clone(),
            log_service: self.ctx.log_service.clone(),
            messages: self.ctx.messages.clone(),
            checkpoints: self.ctx.checkpoints.clone(),
            delegation_tx: self.ctx.delegation_tx.clone(),
            delegation_registry: self.ctx.control.delegation_registry.clone(),
            handles: self.ctx.control.handles.clone(),
            distiller: self.ctx.distiller.clone(),
            handoff_writer: self.ctx.handoff_writer.clone(),
            kg_episode_store: integrations.kg_episode_store,
            paths: self.ctx.paths.clone(),
            kg_store: integrations.kg_store,
            ingestion_adapter: integrations.ingestion_adapter,
            memory_store: self.ctx.memory_store.clone(),
            connector_registry: self.ctx.connector_registry.clone(),
            bridge_registry: self.ctx.bridge_registry.clone(),
            bridge_outbox: self.ctx.bridge_outbox.clone(),
        };
        let ctx = super::execution_stream::ExecutionContext {
            mode: super::execution_stream::ExecutionMode::Root,
            execution_id: setup.execution_id,
            session_id: setup.session_id.clone(),
            agent_id: config.agent_id.clone(),
            conversation_id: config.conversation_id.clone(),
            handle: setup.handle.clone(),
            respond_to: config.respond_to.clone(),
            thread_id: config.thread_id.clone(),
            message,
            scanned_input_cursor: setup.scanned_input_cursor,
            authored_prompt_id: Some(setup.root_message_id),
            history: setup.history,
            recommended_skills: setup.recommended_skills,
        };
        let peer_registry = self.ctx.steering_registry.clone();
        let peer_execution_id = ctx.execution_id.clone();
        tokio::spawn(async move {
            let _ = stream.run(ctx, setup.executor).await;
            peer_registry.remove(&peer_execution_id);
        });
        Ok((setup.handle, setup.session_id))
    }
}
