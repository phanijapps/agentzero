//! # ExecutionStream
//!
//! Per-execution event loop. Consumes an AgentZero engine stream,
//! accumulates tool calls, drives lifecycle transitions, and fires
//! post-execution background tasks (distillation, ward indexing).
//!
//! Field list = dependency contract.

use std::collections::HashMap;
use std::sync::Arc;

use agent_runtime::{BoxedAgentEngine, ChatMessage, ToolResultContextConfig};
use api_logs::LogService;
use execution_state::StateService;
use gateway_events::EventBus;
use gateway_services::SharedVaultPaths;
use tokio::sync::{mpsc, RwLock};
use zbot_stores_sqlite::{ConversationRepository, DatabaseManager};

use crate::delegation::extract_structured_result;
use crate::delegation::{DelegationRegistry, DelegationRequest};
use crate::handle::ExecutionHandle;
use crate::invoke::micro_recall::MicroRecallContext;
use crate::invoke::working_memory_middleware;
use crate::invoke::{
    broadcast_event, process_stream_event, spawn_batch_writer_with_traces, BatchWriterHandle,
    ResponseAccumulator, StreamContext, ToolCallAccumulator, WorkingMemory,
};
use crate::lifecycle::{
    complete_execution, crash_execution, stop_execution, CompleteExecution, CrashExecution,
    StopExecution,
};

// ============================================================================
// STRUCT
// ============================================================================

/// Per-execution event loop handler.
///
/// Constructed by the caller in `invoke_with_callback`, wrapped in a
/// `tokio::spawn`, and consumed by a single call to [`ExecutionStream::run`].
/// Not long-lived — each invocation creates a fresh instance.
pub struct ExecutionStream {
    pub event_bus: Arc<EventBus>,
    pub state_service: Arc<StateService<DatabaseManager>>,
    pub log_service: Arc<LogService<DatabaseManager>>,
    pub conversation_repo: Arc<ConversationRepository>,
    pub messages: Arc<dyn zbot_conversation::MessageStore>,
    pub checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    pub delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    pub delegation_registry: Arc<DelegationRegistry>,
    pub handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    pub distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    pub kg_episode_repo: Option<Arc<zbot_stores_sqlite::KgEpisodeRepository>>,
    pub paths: SharedVaultPaths,
    pub kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    pub memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub connector_registry: Option<Arc<gateway_connectors::ConnectorRegistry>>,
    pub bridge_registry: Option<Arc<gateway_bridge::BridgeRegistry>>,
    pub bridge_outbox: Option<Arc<gateway_bridge::OutboxRepository>>,
    pub handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
}

/// Per-execution identifiers, handle, and message payload.
/// Constructed by callers as part of session setup and passed verbatim to
/// [`ExecutionStream::run`].
pub struct ExecutionContext {
    pub execution_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub handle: ExecutionHandle,
    pub respond_to: Option<Vec<String>>,
    pub thread_id: Option<String>,
    pub message: String,
    pub history: Vec<ChatMessage>,
    pub recommended_skills: Vec<String>,
}

// ============================================================================
// EVENT ACCUMULATOR (stream-local mutable state)
// ============================================================================

/// Per-turn mutable state for the stream event loop.
/// Kept in one struct so event handlers take `&mut EventAccumulator`
/// instead of 10 parameters.
struct EventAccumulator {
    tool_acc: ToolCallAccumulator,
    turn_tool_calls: Vec<serde_json::Value>,
    turn_text: String,
    working_memory: WorkingMemory,
    pending_recall_triggers: Vec<(crate::invoke::micro_recall::MicroRecallTrigger, u32)>,
    current_tool_name: String,
}

/// Borrowed dependencies the stream-event handlers need to observe but not
/// mutate. Constructed once per spawn, passed by reference into each handler.
struct EventHandlerDeps<'a> {
    batch_writer: &'a BatchWriterHandle,
    session_id: &'a str,
    execution_id: &'a str,
    agent_id: &'a str,
    handle: &'a ExecutionHandle,
    tool_result_context: &'a ToolResultContextConfig,
    kg_episode_repo: Option<&'a Arc<zbot_stores_sqlite::KgEpisodeRepository>>,
    kg_store: Option<&'a Arc<dyn zbot_stores::KnowledgeGraphStore>>,
}

/// Pull the `message` (or `text`) arg from a `respond` tool call in
/// the current turn's tool-call list, when one is present.
///
/// The `respond` tool transports the agent's final answer in its
/// args rather than via streamed assistant tokens. Without this
/// helper, the assistant row persisted at tool-result time would be
/// the `"[tool calls]"` placeholder and the answer would only live
/// in the WS event stream — invisible after a page reload.
///
/// Accepts either `args.text` or `args.message` (both shapes appear
/// across older / newer respond-tool callers; see
/// `gateway/gateway-execution/src/session_state.rs` for the matching
/// lookup that runs against execution_log metadata).
fn extract_respond_message(tool_calls: &[serde_json::Value]) -> Option<String> {
    for tc in tool_calls {
        if tc.get("tool_name").and_then(|v| v.as_str()) != Some("respond") {
            continue;
        }
        let args = tc.get("args")?;
        let msg = args
            .get("text")
            .or_else(|| args.get("message"))
            .and_then(|v| v.as_str())?;
        if msg.is_empty() {
            continue;
        }
        return Some(msg.to_string());
    }
    None
}

/// Handle a `StreamEvent::ToolCallStart` — record the call, update the
/// current tool name, and append to the per-turn tool-call list.
fn handle_tool_call_start(
    acc: &mut EventAccumulator,
    tool_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
) {
    acc.tool_acc
        .start_call(tool_id.to_string(), tool_name.to_string(), args.clone());
    acc.current_tool_name = tool_name.to_string();
    acc.turn_tool_calls.push(serde_json::json!({
        "tool_id": tool_id,
        "tool_name": tool_name,
        "args": args,
    }));
}

/// Handle a `StreamEvent::ToolResult` — flush the pending assistant turn,
/// emit the tool message, update working memory, fire-and-forget graph
/// extraction, and collect micro-recall triggers for post-stream execution.
fn handle_tool_result(
    acc: &mut EventAccumulator,
    deps: &EventHandlerDeps<'_>,
    tool_id: &str,
    result: &str,
    context_result: Option<&str>,
    error: Option<&str>,
) {
    acc.tool_acc
        .complete_call(tool_id, result.to_string(), error.map(String::from));

    // Emit the assistant message for this turn (with accumulated tool_calls).
    //
    // Content priority:
    //   1. Streamed assistant text in `turn_text` (the agent emitted text
    //      before calling the tool — e.g. "Let me check the graph…")
    //   2. The `respond` tool's `message`/`text` arg when one is present
    //      in this turn's tool-call list. The agent encoded the final
    //      answer as a tool-arg instead of streaming it; persisting the
    //      arg here is what makes the answer survive a reload.
    //   3. The `"[tool calls]"` placeholder for non-respond tools whose
    //      turn had no streamed text (graph_query, memory, etc.).
    if !acc.turn_tool_calls.is_empty() {
        let tc_json = serde_json::to_string(&acc.turn_tool_calls).unwrap_or_default();
        let content = if !acc.turn_text.is_empty() {
            std::mem::take(&mut acc.turn_text)
        } else if let Some(msg) = extract_respond_message(&acc.turn_tool_calls) {
            msg
        } else {
            "[tool calls]".to_string()
        };
        deps.batch_writer.session_message(
            deps.session_id,
            deps.execution_id,
            "assistant",
            &content,
            Some(&tc_json),
            None,
        );
        acc.turn_tool_calls.clear();
    }

    // Emit tool result message
    let tool_content = super::prompt_safe_tool_content(
        &acc.current_tool_name,
        result,
        context_result,
        error,
        deps.tool_result_context,
    );
    deps.batch_writer.session_message(
        deps.session_id,
        deps.execution_id,
        "tool",
        &tool_content,
        None,
        Some(tool_id),
    );

    // Update working memory from tool result
    working_memory_middleware::process_tool_result(
        &mut acc.working_memory,
        &acc.current_tool_name,
        result,
        error,
        deps.handle.current_iteration(),
    );

    // Phase 6d: real-time graph extraction from tool output.
    // Non-blocking — fires in a background task so the execution
    // loop never waits.
    if let (Some(ep_repo), Some(kg)) = (deps.kg_episode_repo, deps.kg_store) {
        let tool_name_cl = acc.current_tool_name.clone();
        let tool_id_cl = tool_id.to_string();
        let result_cl = result.to_string();
        let session_id_cl = deps.session_id.to_string();
        let agent_id_cl = deps.agent_id.to_string();
        let ep_store: Arc<dyn zbot_stores_traits::KgEpisodeStore> = Arc::new(
            zbot_stores_sqlite::GatewayKgEpisodeStore::new(ep_repo.clone()),
        );
        let kg_cl = kg.clone();
        tokio::spawn(async move {
            crate::tool_result_extractor::extract_and_persist(
                &tool_name_cl,
                &tool_id_cl,
                &result_cl,
                &session_id_cl,
                &agent_id_cl,
                ep_store.as_ref(),
                kg_cl.as_ref(),
            )
            .await;
        });
    }

    // Detect micro-recall triggers (sync) — executed after stream completes
    let triggers = working_memory_middleware::detect_recall_triggers(
        &acc.working_memory,
        &acc.current_tool_name,
        result,
        error,
    );

    let iter = deps.handle.current_iteration();
    for trigger in triggers {
        acc.pending_recall_triggers.push((trigger, iter));
    }
}

// ============================================================================
// IMPL
// ============================================================================

impl ExecutionStream {
    /// Per-execution entry point. Body is the verbatim contents of the old
    /// `ExecutionRunner::spawn_execution_task` (the inside of the
    /// `tokio::spawn(async move { … })` block), with `self.<field>`
    /// replacing every captured-runner-field access and `ctx.<field>`
    /// replacing every `args.<field>` access.
    pub async fn run(
        &self,
        ctx: ExecutionContext,
        executor: BoxedAgentEngine,
    ) -> Result<(), String> {
        let ExecutionContext {
            execution_id,
            session_id,
            agent_id,
            conversation_id,
            handle,
            respond_to,
            thread_id,
            message,
            mut history,
            recommended_skills,
        } = ctx;

        // Create batch writer for non-blocking DB writes (message writes route
        // through MessageStore; conversation_repo stays as fallback).
        let batch_writer = spawn_batch_writer_with_traces(
            self.state_service.clone(),
            self.log_service.clone(),
            Some(self.conversation_repo.clone()),
            self.paths.traces_dir(),
            Some(self.messages.clone()),
        );

        // Create stream context for event processing
        let stream_ctx = StreamContext::new(
            agent_id.clone(),
            conversation_id.clone(),
            session_id.clone(),
            execution_id.clone(),
            self.event_bus.clone(),
            self.log_service.clone(),
            self.state_service.clone(),
            self.delegation_tx.clone(),
            self.paths.vault_dir().clone(),
        )
        .with_batch_writer(batch_writer.clone())
        .with_recommended_skills(recommended_skills.clone());

        let mut response_acc = ResponseAccumulator::new();
        let settings_service = gateway_services::SettingsService::new(self.paths.clone());
        let tool_settings = settings_service.get_tool_settings().unwrap_or_default();
        let tool_result_context =
            super::prompt_safe_tool_result_config(&tool_settings, self.paths.vault_dir());

        // Append user message to session stream BEFORE execution
        batch_writer.session_message(&session_id, &execution_id, "user", &message, None, None);

        // Per-turn mutable state — kept in one struct so the event
        // handlers take `&mut EventAccumulator` instead of 10 parameters.
        let mut acc = EventAccumulator {
            tool_acc: ToolCallAccumulator::new(),
            turn_tool_calls: Vec::new(),
            turn_text: String::new(),
            working_memory: WorkingMemory::new(1500),
            pending_recall_triggers: Vec::new(),
            current_tool_name: String::new(),
        };

        // Reconstruct delegation state from history so working memory accurately
        // reflects which agents have been delegated to and which have completed.
        //
        // Pass 1: scan tool messages for delegate_to_agent results → mark as "running"
        // Pass 2: scan system messages for callbacks → mark as "completed" + decrement pending
        // Also seed corrections from recalled system messages.
        for msg in &history {
            match msg.role.as_str() {
                "tool" => {
                    let content = msg.text_content();
                    // Detect delegate_to_agent returns — sentinel is "status":"delegated"
                    if content.contains("\"status\":\"delegated\"")
                        || content.contains("\"status\": \"delegated\"")
                    {
                        working_memory_middleware::process_tool_result(
                            &mut acc.working_memory,
                            "delegate_to_agent",
                            &content,
                            None,
                            0,
                        );
                    }
                }
                "system" => {
                    let content = msg.text_content();
                    // Seed corrections from recall messages
                    if content.contains("Recalled") || content.contains("correction") {
                        for line in content.lines() {
                            let trimmed = line.trim().trim_start_matches("- ");
                            if trimmed.starts_with("[correction]")
                                || trimmed.starts_with("[pattern]")
                            {
                                acc.working_memory.add_correction(trimmed);
                            }
                        }
                    }
                    // Detect delegation callbacks and mark agents as completed.
                    // Structured path: <!-- structured-result {"agent":"...", ...} -->
                    if let Some(envelope) = extract_structured_result(&content) {
                        if let Some(agent_id) = envelope
                            .get("agent")
                            .and_then(|v: &serde_json::Value| v.as_str())
                        {
                            let result_str = envelope
                                .get("data")
                                .map(|d: &serde_json::Value| d.to_string())
                                .unwrap_or_default();
                            working_memory_middleware::process_callback_message(
                                &mut acc.working_memory,
                                agent_id,
                                &result_str,
                            );
                        }
                    } else if content.contains("## From ")
                        || content.starts_with("## Delegation Failed")
                    {
                        // Plain-text callback: "## From Research Agent\n..."
                        // Error callback: "## Delegation Failed\n**Agent:** Research Agent\n..."
                        let agent_id = if let Some(line) =
                            content.lines().find(|l| l.starts_with("## From "))
                        {
                            let display = line.trim_start_matches("## From ").trim();
                            // Reverse format_agent_display_name: "Research Agent" → "research-agent"
                            display.to_lowercase().replace(' ', "-")
                        } else if let Some(line) =
                            content.lines().find(|l| l.starts_with("**Agent:**"))
                        {
                            let display = line.trim_start_matches("**Agent:**").trim();
                            display.to_lowercase().replace(' ', "-")
                        } else {
                            String::new()
                        };
                        if !agent_id.is_empty() {
                            working_memory_middleware::process_callback_message(
                                &mut acc.working_memory,
                                &agent_id,
                                &content,
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        // Inject working memory into history if it has content
        if !acc.working_memory.is_empty() {
            history.push(ChatMessage::system(acc.working_memory.format_for_prompt()));
        }

        // Immutable handler deps — constructed once, borrowed into each
        // event handler call.
        let session_id_inner = session_id.clone();
        let execution_id_inner = execution_id.clone();
        let agent_id_inner = agent_id.clone();
        let batch_writer_inner = batch_writer.clone();
        let kg_episode_repo_inner = self.kg_episode_repo.clone();
        let kg_store_inner = self.kg_store.clone();

        // Execute with streaming — closure dispatches into free-fn
        // handlers defined at module scope (handle_tool_call_start,
        // handle_tool_result). Keeps the spawn body flat.
        //
        // Pass the handle's stop signal so the executor's mid-stream
        // recv loop can abort the in-flight LLM task when the user
        // clicks Stop. On observation it returns ExecutorError::Stopped
        // which we handle as a graceful exit below (stop_execution,
        // not crash_execution).
        let stop_sig = Some(handle.stop_signal());
        let mut on_event = |event| {
            if handle.is_stop_requested() {
                return;
            }

            handle.increment();

            let deps = EventHandlerDeps {
                batch_writer: &batch_writer_inner,
                session_id: &session_id_inner,
                execution_id: &execution_id_inner,
                agent_id: &agent_id_inner,
                handle: &handle,
                tool_result_context: &tool_result_context,
                kg_episode_repo: kg_episode_repo_inner.as_ref(),
                kg_store: kg_store_inner.as_ref(),
            };

            // Stream messages to session as they happen
            match &event {
                agent_runtime::StreamEvent::ToolCallStart {
                    tool_id,
                    tool_name,
                    args,
                    ..
                } => handle_tool_call_start(&mut acc, tool_id, tool_name, args),
                agent_runtime::StreamEvent::ToolResult {
                    tool_id,
                    result,
                    context_result,
                    error,
                    ..
                } => handle_tool_result(
                    &mut acc,
                    &deps,
                    tool_id,
                    result,
                    context_result.as_deref(),
                    error.as_deref(),
                ),
                agent_runtime::StreamEvent::Token { content, .. } => {
                    acc.turn_text.push_str(content);
                }
                _ => {}
            }

            // Process the event (logging, delegation, token tracking)
            let (gateway_event, response_delta) = process_stream_event(&stream_ctx, &event);

            // Accumulate response content
            if let Some(delta) = response_delta {
                response_acc.append(&delta);
            }

            // Broadcast the gateway event (if not an internal-only event)
            if let Some(event) = gateway_event {
                broadcast_event(stream_ctx.event_bus.clone(), event);
            }
        };
        let result = executor
            .execute_stream_with_stop_flag(&message, &history, stop_sig, &mut on_event)
            .await;

        // Execute micro-recall triggers collected during the stream
        if !acc.pending_recall_triggers.is_empty() {
            let recall_ctx = MicroRecallContext {
                memory_store: self.memory_store.clone(),
                kg_store: self.kg_store.clone(),
                agent_id: agent_id.clone(),
            };
            for (trigger, iter) in &acc.pending_recall_triggers {
                working_memory_middleware::execute_micro_recall_triggers(
                    &mut acc.working_memory,
                    std::slice::from_ref(trigger),
                    &recall_ctx,
                    *iter,
                )
                .await;
            }
        }

        let accumulated_response = response_acc.into_response();

        tracing::info!(
            execution_id = %execution_id,
            response_len = accumulated_response.len(),
            tool_calls_count = acc.tool_acc.len(),
            "Execution stream completed"
        );

        // Emit any remaining text that wasn't flushed as part of a tool-call turn.
        // If turn_text is empty, the response was already written when the last
        // ToolResult (e.g., from the respond tool) flushed it. Don't write again.
        if !acc.turn_text.is_empty() {
            batch_writer.session_message(
                &session_id,
                &execution_id,
                "assistant",
                &acc.turn_text,
                None,
                None,
            );

            // Log the response for session replay
            let response_log = api_logs::ExecutionLog::new(
                &execution_id,
                &session_id,
                &agent_id,
                api_logs::LogLevel::Info,
                api_logs::LogCategory::Response,
                &accumulated_response,
            );
            batch_writer.log(response_log);
        }

        // Turn-boundary checkpoint — write a versioned snapshot of the
        // agent's context state so session_state can read it in O(1)
        // (T12) instead of replaying execution_logs.
        super::core::write_turn_checkpoint(
            &self.checkpoints,
            &self.state_service,
            &execution_id,
            &session_id,
            handle.current_iteration(),
            &accumulated_response,
        );

        // Handle completion
        match result {
            Ok(()) => {
                // Check if this execution spawned delegations that are still active.
                // Use session.pending_delegations (set synchronously in handle_delegation)
                // rather than delegation_registry (populated asynchronously by spawn).
                let has_active_delegations = self
                    .state_service
                    .get_session(&session_id)
                    .ok()
                    .flatten()
                    .map(|s| s.has_pending_delegations())
                    .unwrap_or(false);

                if has_active_delegations {
                    // Root paused for delegation — do NOT complete execution.
                    // The continuation callback will handle completion.
                    tracing::info!(
                        session_id = %session_id,
                        "Root paused for delegation — skipping execution completion"
                    );

                    // Request continuation so the session resumes when delegations complete
                    if let Err(e) = self.state_service.request_continuation(&session_id) {
                        tracing::warn!("Failed to request continuation: {}", e);
                    }

                    // Aggregate tokens so UI shows progress
                    if let Err(e) = self.state_service.aggregate_session_tokens(&session_id) {
                        tracing::warn!("Failed to aggregate session tokens: {}", e);
                    }
                } else {
                    // Normal completion — no active delegations
                    complete_execution(CompleteExecution {
                        state_service: &self.state_service,
                        log_service: &self.log_service,
                        event_bus: &self.event_bus,
                        execution_id: &execution_id,
                        session_id: &session_id,
                        agent_id: &agent_id,
                        conversation_id: &conversation_id,
                        response: Some(accumulated_response),
                        connector_registry: self.connector_registry.as_ref(),
                        respond_to: respond_to.as_ref(),
                        thread_id: thread_id.as_deref(),
                        bridge_registry: self.bridge_registry.as_ref(),
                        bridge_outbox: self.bridge_outbox.as_ref(),
                    })
                    .await;
                }

                // Ward AGENTS.md and memory-bank/ are curated manually by agents;
                // the runtime no longer rewrites them post-execution.
                let session_ward = self
                    .state_service
                    .get_session(&session_id)
                    .ok()
                    .flatten()
                    .and_then(|s| s.ward_id);

                // Fire-and-forget session distillation, followed by ward artifact indexing.
                if let Some(distiller) = self.distiller.as_ref() {
                    let distiller = distiller.clone();
                    let sid = session_id.clone();
                    let aid = agent_id.clone();
                    let ward_id_for_indexer = session_ward.clone();
                    // Phase C: trait-routed indexer. Wrap the SQLite
                    // kg_episode_repo as a KgEpisodeStore for the test
                    // path; production already has the trait wired via
                    // AppState but ExecutionStream's struct still holds
                    // the concrete repo for backward compat. Same shape
                    // for kg_store: the SqliteKgStore wrap of graph_storage.
                    let kg_episode_store_for_indexer: Option<
                        Arc<dyn zbot_stores_traits::KgEpisodeStore>,
                    > = self.kg_episode_repo.as_ref().map(|r| {
                        Arc::new(zbot_stores_sqlite::GatewayKgEpisodeStore::new(r.clone()))
                            as Arc<dyn zbot_stores_traits::KgEpisodeStore>
                    });
                    let kg_store_for_indexer: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>> =
                        self.kg_store.clone();
                    let paths_for_indexer = self.paths.clone();
                    tokio::spawn(async move {
                        if let Err(e) = distiller.distill(&sid, &aid).await {
                            tracing::warn!("Session distillation failed: {}", e);
                        }
                        super::core::run_ward_artifact_indexer(
                            &ward_id_for_indexer,
                            &sid,
                            &aid,
                            kg_episode_store_for_indexer.as_ref(),
                            kg_store_for_indexer.as_ref(),
                            &paths_for_indexer,
                        )
                        .await;
                    });
                }

                // Session handoff — fire-and-forget, silent on failure.
                if let Some(writer) = self.handoff_writer.as_ref() {
                    let writer = writer.clone();
                    let sid = session_id.clone();
                    let aid = agent_id.clone();
                    let wid = session_ward.clone().unwrap_or_default();
                    tokio::spawn(async move {
                        writer.write(&sid, &aid, &wid).await;
                    });
                }
            }
            Err(agent_runtime::ExecutorError::Stopped) => {
                // Cooperative stop — the executor's mid-stream poll
                // observed handle.stop() and aborted. We don't call
                // stop_execution here; the trailing
                // `if handle.is_stop_requested()` block below is the
                // canonical path. Calling it twice would warn
                // "Failed to cancel session: Cannot cancel session in
                // CANCELLED state" on the second attempt because the
                // session is already terminal.
                tracing::info!(
                    session_id = %session_id,
                    "Cooperative stop observed; trailing check will finalize"
                );

                // Cancel any orphaned delegations spawned before the stop.
                cancel_session_delegations(
                    &session_id,
                    &self.delegation_registry,
                    &self.handles,
                    &self.state_service,
                )
                .await;
            }
            Err(e) => {
                // Crash execution and emit events
                crash_execution(CrashExecution {
                    state_service: &self.state_service,
                    log_service: &self.log_service,
                    event_bus: &self.event_bus,
                    execution_id: &execution_id,
                    session_id: &session_id,
                    agent_id: &agent_id,
                    conversation_id: &conversation_id,
                    error: &e.to_string(),
                    crash_session: true, // crash session for root execution
                })
                .await;

                // Cancel any orphaned delegations for this session
                cancel_session_delegations(
                    &session_id,
                    &self.delegation_registry,
                    &self.handles,
                    &self.state_service,
                )
                .await;
            }
        }

        // Check if stopped
        if handle.is_stop_requested() {
            stop_execution(StopExecution {
                state_service: &self.state_service,
                log_service: &self.log_service,
                event_bus: &self.event_bus,
                execution_id: &execution_id,
                session_id: &session_id,
                agent_id: &agent_id,
                conversation_id: &conversation_id,
                iteration: handle.current_iteration(),
            })
            .await;
        }

        Ok(())
    }
}

// ============================================================================
// ORPHAN DELEGATION CLEANUP
// ============================================================================

/// Cancel all in-flight delegations for a session.
/// Called when root execution crashes to prevent orphaned subagents.
async fn cancel_session_delegations(
    session_id: &str,
    delegation_registry: &DelegationRegistry,
    handles: &RwLock<HashMap<String, crate::handle::ExecutionHandle>>,
    state_service: &execution_state::StateService<DatabaseManager>,
) {
    let active = delegation_registry.get_by_session_id(session_id);

    if active.is_empty() {
        return;
    }

    tracing::info!(
        session_id = %session_id,
        count = active.len(),
        "Cancelling orphaned delegations"
    );

    for (child_conv_id, _ctx) in &active {
        // Stop the execution handle
        {
            let handles_guard = handles.read().await;
            if let Some(handle) = handles_guard.get(child_conv_id) {
                handle.stop();
            }
        }

        // Remove from registry
        delegation_registry.remove(child_conv_id);

        // Decrement pending_delegations so session can complete
        if let Err(e) = state_service.complete_delegation(session_id) {
            tracing::debug!("Failed to decrement pending_delegations: {}", e);
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use api_logs::LogService;
    use execution_state::StateService;
    use gateway_events::EventBus;
    use gateway_services::VaultPaths;
    use tokio::sync::{mpsc, RwLock};
    use zbot_stores_sqlite::{ConversationRepository, DatabaseManager};

    #[test]
    fn execution_stream_constructs_with_minimum_required_deps() {
        // Compile-as-assertion: locks in the field list as the dependency
        // contract. End-to-end coverage lives in the e2e suite (Tasks 7+8).
        #[allow(deprecated)]
        let dir = tempfile::tempdir().unwrap();
        #[allow(deprecated)]
        let path = dir.into_path();
        let paths = Arc::new(VaultPaths::new(path));
        let db = Arc::new(DatabaseManager::new(paths.clone()).unwrap());
        let state = Arc::new(StateService::new(db.clone()));
        let logs = Arc::new(LogService::new(db.clone()));
        let convo = Arc::new(ConversationRepository::new(db));
        let bus = Arc::new(EventBus::new());
        let (tx, _rx) = mpsc::unbounded_channel();
        let registry = Arc::new(crate::delegation::DelegationRegistry::new());
        let handles = Arc::new(RwLock::new(HashMap::new()));

        let _ = ExecutionStream {
            event_bus: bus,
            state_service: state,
            log_service: logs,
            conversation_repo: convo.clone(),
            messages: Arc::new(zbot_conversation::SqliteMessageStore::new(
                zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap(),
            )),
            checkpoints: Arc::new(zbot_conversation::SqliteCheckpointStore::new(
                zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap(),
            )),
            delegation_tx: tx,
            delegation_registry: registry,
            handles,
            distiller: None,
            kg_episode_repo: None,
            paths,
            kg_store: None,
            memory_store: None,
            connector_registry: None,
            bridge_registry: None,
            bridge_outbox: None,
            handoff_writer: None,
        };
    }

    // ----- extract_respond_message: persistence of the agent's final answer
    //       when emitted via the `respond` tool. Regression coverage for the
    //       Quick-Chat-reloads-blank bug.

    #[test]
    fn extract_respond_message_picks_message_arg() {
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "message": "Here is your answer." }
        })];
        assert_eq!(
            extract_respond_message(&calls),
            Some("Here is your answer.".to_string())
        );
    }

    #[test]
    fn extract_respond_message_picks_text_arg_as_fallback() {
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "text": "Older-style payload." }
        })];
        assert_eq!(
            extract_respond_message(&calls),
            Some("Older-style payload.".to_string())
        );
    }

    #[test]
    fn extract_respond_message_prefers_text_over_message_when_both_present() {
        // The lookup in session_state.rs's `find_respond` checks `text`
        // first, then `message`. Mirror that ordering here so both
        // call sites behave consistently.
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "text": "from-text", "message": "from-message" }
        })];
        assert_eq!(
            extract_respond_message(&calls),
            Some("from-text".to_string())
        );
    }

    #[test]
    fn extract_respond_message_skips_non_respond_tools() {
        let calls = vec![
            serde_json::json!({
                "tool_name": "graph_query",
                "args": { "action": "search", "query": "user" }
            }),
            serde_json::json!({
                "tool_name": "memory",
                "args": { "action": "recall" }
            }),
        ];
        assert_eq!(extract_respond_message(&calls), None);
    }

    #[test]
    fn extract_respond_message_returns_first_respond_when_multiple() {
        // Unusual but possible — the model could emit two respond calls
        // in one turn. The first is what flushes the assistant row.
        let calls = vec![
            serde_json::json!({
                "tool_name": "respond",
                "args": { "message": "first" }
            }),
            serde_json::json!({
                "tool_name": "respond",
                "args": { "message": "second" }
            }),
        ];
        assert_eq!(extract_respond_message(&calls), Some("first".to_string()));
    }

    #[test]
    fn extract_respond_message_returns_none_when_message_empty() {
        // Empty string is treated as "no answer" so we don't persist a
        // blank assistant row that hides the placeholder fallback.
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "message": "" }
        })];
        assert_eq!(extract_respond_message(&calls), None);
    }

    #[test]
    fn extract_respond_message_returns_none_when_no_message_arg() {
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "format": "json" }
        })];
        assert_eq!(extract_respond_message(&calls), None);
    }

    #[test]
    fn extract_respond_message_handles_mixed_tool_calls() {
        // Realistic scenario: graph_query, then respond. The respond
        // payload is what we want as the persisted assistant content.
        let calls = vec![
            serde_json::json!({
                "tool_name": "graph_query",
                "args": { "action": "search", "query": "user" }
            }),
            serde_json::json!({
                "tool_name": "respond",
                "args": { "message": "Here is what I found." }
            }),
        ];
        assert_eq!(
            extract_respond_message(&calls),
            Some("Here is what I found.".to_string())
        );
    }
}
