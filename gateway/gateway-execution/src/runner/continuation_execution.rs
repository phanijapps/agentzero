//! Continuation preparation and execution; stream semantics remain unchanged.
use super::core::{
    attach_mid_session_recall_hook, run_ward_artifact_indexer, write_turn_checkpoint,
};
use crate::delegation::{DelegationRegistry, DelegationRequest};
use crate::handle::ExecutionHandle;
use crate::invoke::{
    assistant_turn_content, broadcast_event, collect_agents_summary, collect_skills_summary,
    process_stream_event, select_engine, spawn_batch_writer_with_traces, AgentLoader,
    ExecutorBuilder, ResponseAccumulator, StreamContext, ToolCallAccumulator,
};
use crate::lifecycle::{
    complete_execution, crash_execution, emit_agent_started, stop_execution, CompleteExecution,
    CrashExecution, StopExecution,
};
use agent_runtime::{BoxedAgentEngine, ChatMessage, ContextActorKind};
use api_logs::LogService;
use execution_state::{SessionPlanSnapshot, StateService};
use gateway_events::EventBus;
use gateway_services::{AgentService, McpService, ProviderService, SharedVaultPaths};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use zbot_runtime_sqlite::DatabaseManager;

/// Explicit inputs for a single continuation invocation.
pub(super) struct ContinuationArgs<'a> {
    pub(super) session_id: &'a str,
    pub(super) root_agent_id: &'a str,
    pub(super) event_bus: Arc<EventBus>,
    pub(super) agent_service: Arc<AgentService>,
    pub(super) provider_service: Arc<ProviderService>,
    pub(super) mcp_service: Arc<McpService>,
    pub(super) skill_service: Arc<gateway_services::SkillService>,
    pub(super) paths: SharedVaultPaths,
    pub(super) messages: Arc<dyn zbot_conversation::MessageStore>,
    pub(super) checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    pub(super) handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    pub(super) delegation_registry: Arc<DelegationRegistry>,
    pub(super) delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    pub(super) log_service: Arc<LogService<DatabaseManager>>,
    pub(super) state_service: Arc<StateService<DatabaseManager>>,
    pub(super) memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub(super) embedding_client: Option<Arc<dyn agent_runtime::llm::embedding::EmbeddingClient>>,
    pub(super) distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    pub(super) handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
    pub(super) memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
    pub(super) peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
    pub(super) a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
    pub(super) steering_registry: Arc<agent_runtime::SteeringRegistry>,
    pub(super) model_registry: Option<Arc<gateway_services::models::ModelRegistry>>,
    pub(super) kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    pub(super) kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>>,
    pub(super) ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
    pub(super) goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
    pub(super) procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    pub(super) ward_usage: Arc<gateway_services::WardUsage>,
}

/// Prepend scoped, sanitized unified recall to `history` as a system message
/// at position 0.
///
/// Uses the most recent user message in `history` as the recall query so the
/// recalled facts are relevant to the task at hand (vs. a hardcoded placeholder).
/// No-op when `memory_recall` is `None`, the recall call errors, or it returns
/// no items.
async fn prepend_continuation_recall(
    history: &mut Vec<ChatMessage>,
    memory_recall: Option<&Arc<crate::recall::MemoryRecall>>,
    goals: Option<&Arc<dyn agent_tools::GoalAccess>>,
    agent_id: &str,
    session_id: &str,
    ward_id: Option<&str>,
) -> std::collections::HashSet<String> {
    let mut initial_recall_keys = std::collections::HashSet::new();
    let Some(recall) = memory_recall else {
        return initial_recall_keys;
    };

    // Use the last user message as the recall query.
    let query = history
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.text_content())
        .unwrap_or_else(|| "continuation recall".to_string());

    let Some(authorization) = crate::invoke::unified_recall_adapter::recall_authorization_context(
        recall, agent_id, "root", session_id, ward_id,
    ) else {
        tracing::debug!(
            agent_id,
            "Continuation recall unavailable without provider scope"
        );
        return initial_recall_keys;
    };

    match crate::invoke::unified_recall_adapter::automatic_unified_recall(
        Arc::clone(recall),
        goals.cloned(),
        authorization,
        query,
        10,
    )
    .await
    {
        Ok(response) if !response.results.is_empty() => {
            let formatted = crate::recall::format_unified_recall_response_with_options(
                &response,
                crate::recall::ContextPacketBuildOptions::new(
                    format!("{agent_id}:continuation-recall"),
                    agent_id.to_string(),
                    ContextActorKind::Root,
                    1_200,
                )
                .with_ward_id(ward_id.map(str::to_string)),
            );
            if !formatted.is_empty() {
                initial_recall_keys.extend(
                    response
                        .results
                        .iter()
                        .map(crate::recall::unified_item_dedup_key),
                );
                history.insert(0, ChatMessage::system(formatted));
            }
            tracing::info!(
                item_count = response.count,
                "Recalled unified context for continuation"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(reason = ?e.code, "Continuation recall failed"),
    }
    initial_recall_keys
}

/// Build the system-message prompt that seeds a continuation turn.
///
/// Prefer the persisted session plan, which is the plan the current execution
/// actually owns. A ward `specs/**/plan.md` is only a legacy fallback because
/// it can belong to an unrelated earlier task in the same ward.
///
/// Side effect: when a plan is found and a fact store is available, the plan
/// text is written to `ctx.<session_id>.plan` so subagents can fetch it via
/// `memory(get_fact, …)` without re-reading the file.
async fn build_continuation_message(
    paths: &SharedVaultPaths,
    session_id: &str,
    ward_id: Option<&str>,
    session_plan: Option<&SessionPlanSnapshot>,
    fact_store: Option<&Arc<dyn zbot_stores::MemoryFactStore>>,
) -> String {
    let plan_hint = session_plan
        .map(render_session_plan_for_continuation)
        .or_else(|| {
            ward_id.and_then(|wid| {
                let specs_dir = paths.vault_dir().join("wards").join(wid).join("specs");
                find_latest_plan(&specs_dir)
            })
        });

    let Some(plan) = plan_hint else {
        return "[Delegation completed. Review the delegate result already in context. \
                 If the user's goal is satisfied, respond with the final answer. \
                 If work remains, delegate the next concrete step or continue directly.]"
            .to_string();
    };

    // Populate session ctx with the plan so subagents can fetch it via
    // memory(get_fact, key="ctx.<sid>.plan") instead of re-reading the specs
    // file each turn.
    if let (Some(fs), Some(ward)) = (fact_store, ward_id) {
        crate::session_ctx::writer::plan_snapshot(fs, session_id, ward, &plan).await;
    }

    format!(
        "[DELEGATION COMPLETED. YOUR PLAN IS BELOW.\n\
         Review the delegate result already in context against this plan.\n\
         If the user's goal is satisfied, respond with the final answer.\n\
         If work remains, delegate the next concrete step or continue directly.\n\
         Avoid re-reading files unless the delegate result is insufficient.]\n\n{}",
        plan
    )
}

fn render_session_plan_for_continuation(snapshot: &SessionPlanSnapshot) -> String {
    let explanation = snapshot
        .explanation
        .as_deref()
        .map(|text| format!("\n\n{text}"))
        .unwrap_or_default();
    let steps = snapshot
        .plan
        .iter()
        .map(|step| format!("- [{:?}] {}", step.status, step.step))
        .collect::<Vec<_>>()
        .join("\n");
    format!("## Current session plan\n\n{steps}{explanation}")
}

// ============================================================================
// CONTINUATION HANDLER
// ============================================================================

/// Invoke the root agent to continue after all delegations have completed.
///
/// This is called when all subagents have finished and the root agent needs
/// to process their results and decide what to do next:
/// - Respond to the user with synthesized results
/// - Delegate to more subagents if needed
/// - Continue its orchestration loop
///
/// The agent sees the full session context including:
/// - Original user message
/// - Previous assistant responses
/// - Callback messages from completed subagents (as system messages)
pub(super) async fn invoke_continuation(args: ContinuationArgs<'_>) -> Result<(), String> {
    let ContinuationArgs {
        session_id,
        root_agent_id,
        event_bus,
        agent_service,
        provider_service,
        mcp_service,
        skill_service,
        paths,
        messages,
        checkpoints,
        handles,
        delegation_registry: _delegation_registry,
        delegation_tx,
        log_service,
        state_service,
        memory_store,
        embedding_client: _embedding_client,
        distiller,
        handoff_writer,
        memory_recall,
        peer_messages,
        a2a_delegation,
        steering_registry,
        model_registry,
        kg_store,
        kg_episode_store,
        ingestion_adapter,
        goal_adapter,
        procedure_store,
        ward_usage,
    } = args;
    // Generate a new conversation ID for this continuation turn
    let conversation_id = format!(
        "{}-cont-{}",
        session_id,
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0")
    );

    let execution_id = match state_service.get_root_execution(session_id)? {
        Some(root_exec) => root_exec.id,
        None => {
            let execution = execution_state::AgentExecution::new_root(session_id, root_agent_id);
            state_service.create_execution(&execution)?;
            execution.id
        }
    };

    state_service.reactivate_session(session_id)?;
    state_service.reactivate_execution(&execution_id)?;
    let _ = log_service.log_session_start(&execution_id, &conversation_id, root_agent_id, None);

    let handle = ExecutionHandle::new(50);
    {
        let mut handles_guard = handles.write().await;
        handles_guard.insert(conversation_id.clone(), handle.clone());
    }
    emit_agent_started(
        &event_bus,
        root_agent_id,
        &conversation_id,
        session_id,
        &execution_id,
    )
    .await;

    // Load agent and provider (with orchestrator config from settings)
    let settings_for_loader = gateway_services::SettingsService::new(paths.clone());
    let agent_loader = AgentLoader::new(&agent_service, &provider_service, paths.clone())
        .with_settings(&settings_for_loader);
    let (agent, provider) = agent_loader.load_or_create_root(root_agent_id).await?;

    // Load full session conversation (includes tool calls, results, and callbacks).
    let mut history: Vec<ChatMessage> = messages
        .replay(session_id, None, 200)
        .map_err(|_| "continuation_history_read_failed".to_string())
        .map(|rows| crate::conversation_history::messages_to_chat_format(&rows))?;

    // Look up active ward from session (needed for recall ward affinity)
    let session = state_service
        .get_session(session_id)
        .map_err(|_| "continuation_session_read_failed".to_string())?
        .ok_or_else(|| "continuation_session_missing".to_string())?;
    if session.root_agent_id != root_agent_id {
        return Err("continuation_identity_mismatch".to_string());
    }
    let session_ward_id = session.ward_id;
    let session_plan = state_service
        .get_mission_control_session_tokens(session_id)
        .map_err(|_| "continuation_plan_read_failed".to_string())?
        .and_then(|tokens| tokens.current_plan);

    // Prepend scoped unified recall (if any) to history as a bounded system
    // message at position 0. No-op when recall is unavailable, fails closed,
    // or returns no renderable context.
    let initial_recall_keys = prepend_continuation_recall(
        &mut history,
        memory_recall.as_ref(),
        goal_adapter.as_ref(),
        root_agent_id,
        session_id,
        session_ward_id.as_deref(),
    )
    .await;

    tracing::info!(
        session_id = %session_id,
        execution_id = %execution_id,
        history_count = %history.len(),
        "Loading session history for continuation"
    );

    // Get tool settings
    let settings_service = gateway_services::SettingsService::new(paths.clone());
    let tool_settings = settings_service.get_tool_settings().unwrap_or_default();
    let tool_result_context =
        super::prompt_safe_tool_result_config(&tool_settings, paths.vault_dir());

    // Collect available agents and skills
    let available_agents = collect_agents_summary(&agent_service, &paths).await;
    let available_skills = collect_skills_summary(&skill_service).await;

    // Ward AGENTS.md and memory-bank/ are curated manually by agents;
    // the runtime no longer rewrites them before continuation.

    // Build executor
    let mut builder = ExecutorBuilder::new(paths.vault_dir().clone(), tool_settings);
    if let Some(registry) = model_registry {
        builder = builder.with_model_registry(registry);
    }

    // Trait-routed fact store used for save_fact and ctx writes during
    // continuation. Wired via AppState.
    let fact_store: Option<Arc<dyn zbot_stores::MemoryFactStore>> = memory_store.clone();
    // Clone for session-ctx plan_snapshot below — the builder moves the
    // primary Arc, so we keep a separate handle to write plan text to
    // ctx.<sid>.plan on continuations that load a plan.md.
    let fact_store_for_ctx = fact_store.clone();
    if let Some(fs) = fact_store {
        builder = builder.with_fact_store(fs);
    }
    if let Some(ks) = kg_store.clone() {
        builder = builder.with_kg_store(ks);
    }
    if let Some(a) = ingestion_adapter.clone() {
        builder = builder.with_ingestion_adapter(a);
    }
    let goal_adapter_for_mid_session_recall = goal_adapter.clone();
    if let Some(a) = goal_adapter {
        builder = builder.with_goal_adapter(a);
    }
    {
        // Ward-curator observer (matches the bootstrap path's wiring).
        let observer = std::sync::Arc::new(
            crate::invoke::ward_usage_adapter::WardUsageAdapter::new(ward_usage.clone()),
        );
        builder = builder
            .with_ward_usage(observer)
            .with_ward_usage_service(ward_usage.clone());
    }
    if let Some(ps) = procedure_store.clone() {
        builder = builder.with_procedure_store(ps);
    }
    if let Some(recall) = memory_recall.clone() {
        builder = builder.with_memory_recall(recall);
    }
    let peer_messaging_enabled = peer_messages.is_some();
    if let Some(peer_messages) = peer_messages {
        builder = builder.with_peer_messages(peer_messages);
    }
    if let Some(service) = a2a_delegation {
        builder = builder.with_a2a_delegation(service);
    }
    builder = builder.with_initial_state(
        "execution_id",
        serde_json::Value::String(execution_id.clone()),
    );

    let mut executor = builder
        .build(
            &agent,
            &provider,
            &conversation_id,
            session_id,
            &available_agents,
            &available_skills,
            None, // No hook context for continuation
            &mcp_service,
            session_ward_id.as_deref(),
        )
        .await?;

    attach_mid_session_recall_hook(
        &mut executor,
        memory_recall.as_ref(),
        goal_adapter_for_mid_session_recall.as_ref(),
        root_agent_id,
        session_id,
        session_ward_id.as_deref(),
        initial_recall_keys,
    );
    if peer_messaging_enabled {
        let steering_handle = executor.enable_steering();
        steering_registry.register_peer_only(&execution_id, steering_handle);
    }
    let executor: BoxedAgentEngine = select_engine(executor);

    // Build a focused continuation message with the plan injected if one exists.
    let continuation_message = build_continuation_message(
        &paths,
        session_id,
        session_ward_id.as_deref(),
        session_plan.as_ref(),
        fact_store_for_ctx.as_ref(),
    )
    .await;

    // Spawn execution task
    let session_id_clone = session_id.to_string();
    let agent_id_clone = root_agent_id.to_string();

    tokio::spawn(async move {
        // Create batch writer for non-blocking DB writes.
        let batch_writer = spawn_batch_writer_with_traces(
            state_service.clone(),
            log_service.clone(),
            paths.traces_dir(),
            messages.clone(),
        );

        let stream_ctx = StreamContext::new(
            agent_id_clone.clone(),
            conversation_id.clone(),
            session_id_clone.clone(),
            execution_id.clone(),
            event_bus.clone(),
            log_service.clone(),
            state_service.clone(),
            delegation_tx,
            paths.vault_dir().clone(),
        )
        .with_batch_writer(batch_writer.clone());

        let mut response_acc = ResponseAccumulator::new();
        let mut tool_acc = ToolCallAccumulator::new();

        // Append continuation system message to session stream
        batch_writer.session_message(
            &session_id_clone,
            &execution_id,
            "system",
            &continuation_message,
            None,
            None,
        );

        let session_id_inner = session_id_clone.clone();
        let execution_id_inner = execution_id.clone();
        let batch_writer_inner = batch_writer.clone();
        let mut turn_tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut turn_text = String::new();

        // Phase 6d: clones for real-time tool-result extraction (fire-and-forget).
        let kg_episode_store_inner = kg_episode_store.clone();
        let kg_store_inner = kg_store.clone();
        let agent_id_inner = agent_id_clone.clone();
        // Track current tool name so the extractor can dispatch by name.
        let mut current_tool_name = String::new();

        let stop_sig = Some(handle.stop_signal());
        let mut on_event = |event| {
            if handle.is_stop_requested() {
                return;
            }

            handle.increment();

            // Stream messages to session as they happen
            match &event {
                agent_runtime::StreamEvent::ToolCallStart {
                    tool_id,
                    tool_name,
                    args,
                    ..
                } => {
                    tool_acc.start_call(tool_id.clone(), tool_name.clone(), args.clone());
                    current_tool_name = tool_name.clone();
                    turn_tool_calls.push(serde_json::json!({
                        "tool_id": tool_id,
                        "tool_name": tool_name,
                        "args": args,
                    }));
                }
                agent_runtime::StreamEvent::ToolResult {
                    tool_id,
                    result,
                    context_result,
                    error,
                    ..
                } => {
                    tool_acc.complete_call(tool_id, result.clone(), error.clone());

                    // Emit assistant message for this turn
                    if !turn_tool_calls.is_empty() {
                        let tc_json = serde_json::to_string(&turn_tool_calls).unwrap_or_default();
                        let content = assistant_turn_content(&mut turn_text, &turn_tool_calls);
                        batch_writer_inner.session_message(
                            &session_id_inner,
                            &execution_id_inner,
                            "assistant",
                            &content,
                            Some(&tc_json),
                            None,
                        );
                        turn_tool_calls.clear();
                    }

                    // Emit tool result message
                    let tool_content = super::prompt_safe_tool_content(
                        &current_tool_name,
                        result,
                        context_result.as_deref(),
                        error.as_deref(),
                        &tool_result_context,
                    );
                    batch_writer_inner.session_message(
                        &session_id_inner,
                        &execution_id_inner,
                        "tool",
                        &tool_content,
                        None,
                        Some(tool_id),
                    );

                    // Phase 6d: real-time graph extraction from tool output.
                    // Non-blocking — fires in a background task so the
                    // execution loop never waits.
                    if let (Some(ref ep_store), Some(ref kg)) =
                        (&kg_episode_store_inner, &kg_store_inner)
                    {
                        let tool_name_cl = current_tool_name.clone();
                        let tool_id_cl = tool_id.clone();
                        let result_cl = result.clone();
                        let session_id_cl = session_id_inner.clone();
                        let agent_id_cl = agent_id_inner.clone();
                        let ep_store = ep_store.clone();
                        let kg_cl = kg.clone();
                        let intake_cl = ingestion_adapter.clone();
                        tokio::spawn(async move {
                            crate::tool_result_extractor::extract_and_persist(
                                crate::tool_result_extractor::ExtractAndPersistRequest {
                                    tool_name: &tool_name_cl,
                                    tool_call_id: &tool_id_cl,
                                    result_text: &result_cl,
                                    session_id: &session_id_cl,
                                    agent_id: &agent_id_cl,
                                    evidence_intake: intake_cl.as_deref(),
                                    episode_store: ep_store.as_ref(),
                                    kg: kg_cl.as_ref(),
                                },
                            )
                            .await;
                        });
                    }
                }
                agent_runtime::StreamEvent::Token { content, .. } => {
                    turn_text.push_str(content);
                }
                _ => {}
            }

            let (gateway_event, response_delta) = process_stream_event(&stream_ctx, &event);

            if let Some(delta) = response_delta {
                response_acc.append(&delta);
            }

            // Broadcast the gateway event (if not an internal-only event)
            if let Some(event) = gateway_event {
                broadcast_event(stream_ctx.event_bus.clone(), event);
            }
        };
        let result = executor
            .execute_stream_with_stop_flag(&continuation_message, &history, stop_sig, &mut on_event)
            .await;

        let accumulated_response = response_acc.into_response();

        // Emit any remaining text that wasn't flushed as part of a tool-call turn.
        if !turn_text.is_empty() {
            batch_writer.session_message(
                &session_id_clone,
                &execution_id,
                "assistant",
                &turn_text,
                None,
                None,
            );
        }

        // Turn-boundary checkpoint — write a versioned snapshot of the
        // agent's context state so session_state can read it in O(1)
        // (T12) instead of replaying execution_logs.
        write_turn_checkpoint(
            &checkpoints,
            &state_service,
            &execution_id,
            &session_id_clone,
            handle.current_iteration(),
            &accumulated_response,
        );

        // A terminal completion event is also a client snapshot boundary.
        // Flush the queued final assistant message before publishing it.
        if result.is_ok() {
            batch_writer.flush().await;
        }

        match result {
            Ok(()) => {
                // Check if this continuation spawned new delegations
                let has_active_delegations = state_service
                    .get_session(&session_id_clone)
                    .ok()
                    .flatten()
                    .map(|s| s.has_pending_delegations())
                    .unwrap_or(false);

                if has_active_delegations {
                    // Root delegated again — wait for subagent, don't complete
                    tracing::info!(
                        session_id = %session_id_clone,
                        "Continuation paused for delegation — skipping execution completion"
                    );
                    if let Err(e) = state_service.request_continuation(&session_id_clone) {
                        tracing::warn!("Failed to request continuation: {}", e);
                    }
                    if let Err(e) = state_service.aggregate_session_tokens(&session_id_clone) {
                        tracing::warn!("Failed to aggregate session tokens: {}", e);
                    }
                } else {
                    // No more delegations — complete normally
                    complete_execution(CompleteExecution {
                        state_service: &state_service,
                        log_service: &log_service,
                        event_bus: &event_bus,
                        execution_id: &execution_id,
                        session_id: &session_id_clone,
                        agent_id: &agent_id_clone,
                        conversation_id: &conversation_id,
                        response: Some(accumulated_response),
                        connector_registry: None,
                        respond_to: None,
                        thread_id: None,
                        bridge_registry: None,
                        bridge_outbox: None,
                    })
                    .await;
                }

                // Fire-and-forget session distillation, followed by ward artifact indexing.
                if let Some(distiller) = distiller {
                    let sid = session_id_clone.clone();
                    let aid = agent_id_clone.clone();
                    let ward_id_for_indexer = state_service
                        .get_session(&sid)
                        .ok()
                        .flatten()
                        .and_then(|s| s.ward_id);
                    // Both indexer dependencies are backend-neutral stores.
                    let kg_episode_store_for_indexer = kg_episode_store.clone();
                    let kg_store_for_indexer = kg_store.clone();
                    let paths_for_indexer = paths.clone();
                    tokio::spawn(async move {
                        if let Err(e) = distiller.distill(&sid, &aid).await {
                            tracing::warn!("Continuation distillation failed: {}", e);
                        }
                        run_ward_artifact_indexer(
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
                if let Some(writer) = handoff_writer {
                    let sid = session_id_clone.clone();
                    let aid = agent_id_clone.clone();
                    let wid = state_service
                        .get_session(&sid)
                        .ok()
                        .flatten()
                        .and_then(|s| s.ward_id)
                        .unwrap_or_default();
                    tokio::spawn(async move {
                        writer.write(&sid, &aid, &wid).await;
                    });
                }
            }
            Err(agent_runtime::ExecutorError::Stopped) => {
                // Cooperative stop — the trailing
                // `if handle.is_stop_requested()` block below calls
                // stop_execution. No crash report; no double call (which
                // would warn "Cannot cancel session in CANCELLED state"
                // because cancel_session is non-idempotent).
                tracing::info!(
                    session_id = %session_id_clone,
                    "Continuation stopped cooperatively"
                );
            }
            Err(e) => {
                crash_execution(CrashExecution {
                    state_service: &state_service,
                    log_service: &log_service,
                    event_bus: &event_bus,
                    execution_id: &execution_id,
                    session_id: &session_id_clone,
                    agent_id: &agent_id_clone,
                    conversation_id: &conversation_id,
                    error: &e.to_string(),
                    crash_session: true,
                })
                .await;
            }
        }

        if handle.is_stop_requested() {
            stop_execution(StopExecution {
                state_service: &state_service,
                log_service: &log_service,
                event_bus: &event_bus,
                execution_id: &execution_id,
                session_id: &session_id_clone,
                agent_id: &agent_id_clone,
                conversation_id: &conversation_id,
                iteration: handle.current_iteration(),
            })
            .await;
        }
        steering_registry.remove(&execution_id);
    });

    Ok(())
}

/// Find the most recent plan.md under a specs/ directory.
/// Planner saves to specs/{domain_task}/plan.md — we glob for it.
fn find_latest_plan(specs_dir: &std::path::Path) -> Option<String> {
    if !specs_dir.exists() {
        return None;
    }

    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;

    // Search specs/*/plan.md and specs/plan.md
    if let Ok(entries) = std::fs::read_dir(specs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Direct specs/plan.md
            if path.is_file() && path.file_name().map(|f| f == "plan.md").unwrap_or(false) {
                if let Ok(meta) = path.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                            newest = Some((modified, path));
                        }
                    }
                }
            } else if path.is_dir() {
                // specs/{subdir}/plan.md
                let plan_path = path.join("plan.md");
                if plan_path.exists() {
                    if let Ok(meta) = plan_path.metadata() {
                        if let Ok(modified) = meta.modified() {
                            if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                                newest = Some((modified, plan_path));
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some((_, path)) = newest {
        let content = std::fs::read_to_string(&path).ok()?;
        if content.trim().is_empty() {
            return None;
        }
        tracing::info!(path = %path.display(), "Injecting plan into continuation message");
        Some(content)
    } else {
        None
    }
}

#[cfg(test)]
mod continuation_message_tests {
    use super::*;
    use execution_state::{SessionPlanStep, SessionPlanStepStatus};
    use gateway_services::VaultPaths;
    use std::sync::Arc;

    #[tokio::test]
    async fn continuation_without_plan_allows_final_response() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths: SharedVaultPaths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));

        let message = build_continuation_message(&paths, "session-1", None, None, None).await;

        assert!(message.contains("If the user's goal is satisfied"));
        assert!(message.contains("respond with the final answer"));
        assert!(
            !message.contains("delegate the next step in your plan immediately"),
            "continuation must not force another delegation after every child result"
        );
    }

    #[tokio::test]
    async fn continuation_with_plan_allows_final_response() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths: SharedVaultPaths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let plan_dir = tmp.path().join("wards/political-analysis/specs/hormuz");
        std::fs::create_dir_all(&plan_dir).expect("plan dir");
        std::fs::write(plan_dir.join("plan.md"), "- Write report\n").expect("plan");

        let message =
            build_continuation_message(&paths, "session-1", Some("political-analysis"), None, None)
                .await;

        assert!(message.contains("- Write report"));
        assert!(message.contains("respond with the final answer"));
        assert!(
            !message.contains("One action only: delegate_to_agent"),
            "plan continuations must be able to finish instead of re-delegating"
        );
    }

    #[tokio::test]
    async fn continuation_uses_the_current_session_plan_not_an_unrelated_ward_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths: SharedVaultPaths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let plan_dir = tmp.path().join("wards/financial-analysis/specs/old-task");
        std::fs::create_dir_all(&plan_dir).expect("plan dir");
        std::fs::write(plan_dir.join("plan.md"), "- Unrelated ward plan\n").expect("plan");
        let session_plan = SessionPlanSnapshot {
            execution_id: "exec-current".to_owned(),
            explanation: Some("Finish the active research".to_owned()),
            plan: vec![SessionPlanStep {
                step: "Synthesize the Uber and Lyft research".to_owned(),
                status: SessionPlanStepStatus::InProgress,
            }],
            updated_at: "2026-07-17T00:00:00Z".to_owned(),
            source_event_timestamp: 1,
            source_event_sequence: 1,
        };

        let message = build_continuation_message(
            &paths,
            "session-1",
            Some("financial-analysis"),
            Some(&session_plan),
            None,
        )
        .await;

        assert!(message.contains("Synthesize the Uber and Lyft research"));
        assert!(!message.contains("Unrelated ward plan"));
    }
}
