//! # Delegation Spawning
//!
//! Handles spawning of delegated subagents.

use super::callback::{handle_delegation_failure, handle_delegation_success};
use super::context::{infer_delegation_mode, DelegationContext, DelegationRequest};
use super::registry::DelegationRegistry;
use agent_runtime::{BoxedAgentEngine, ContextActorKind, ToolResultContextConfig};
use api_logs::LogService;
use execution_state::StateService;
use gateway_events::{EventBus, GatewayEvent};
use gateway_services::{AgentService, McpService, ProviderService, SharedVaultPaths, SkillService};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, OwnedSemaphorePermit, RwLock};
use zbot_stores_sqlite::DatabaseManager;

use crate::agent_pool::{AgentResultBus, AgentWaitError};

use agent_runtime::ChatMessage;

use crate::handle::ExecutionHandle;
use crate::invoke::{
    broadcast_event, collect_agents_summary, collect_skills_summary, detect_subagent_role,
    process_stream_event, select_engine, spawn_batch_writer_with_traces, subagent_rules,
    AgentLoader, ExecutorBuilder, ResponseAccumulator, RuntimeActorKind, StreamContext,
};
use crate::lifecycle::{
    complete_execution, crash_execution, emit_delegation_completed, emit_delegation_started,
    start_execution, CompleteExecution, CrashExecution, DelegationCompletedEvent,
};
use crate::recall::MemoryRecall;

/// Spawn a delegated agent.
///
/// This is a standalone function that runs a delegated agent using a pre-created
/// execution record. The execution record is created synchronously by
/// `handle_delegation()` in `stream.rs` to prevent a race condition.
///
/// This function handles:
/// - Starting the pre-created execution (QUEUED → RUNNING)
/// - Loading the child agent configuration
/// - Building and running the executor
/// - Sending callbacks to the parent on completion
/// - Marking execution as CRASHED if spawn fails
#[allow(clippy::too_many_arguments)]
pub async fn spawn_delegated_agent(
    request: &DelegationRequest,
    event_bus: Arc<EventBus>,
    agent_service: Arc<AgentService>,
    provider_service: Arc<ProviderService>,
    mcp_service: Arc<McpService>,
    skill_service: Arc<SkillService>,
    paths: SharedVaultPaths,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    delegation_registry: Arc<DelegationRegistry>,
    delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    log_service: Arc<LogService<DatabaseManager>>,
    state_service: Arc<StateService<DatabaseManager>>,
    delegation_permit: Option<OwnedSemaphorePermit>,
    memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    memory_recall: Option<Arc<MemoryRecall>>,
    rate_limiters: Arc<
        std::sync::RwLock<
            std::collections::HashMap<String, Arc<agent_runtime::ProviderRateLimiter>>,
        >,
    >,
    kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
    goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
    steering_registry: Arc<agent_runtime::SteeringRegistry>,
    agent_result_bus: Arc<AgentResultBus>,
) -> Result<String, String> {
    // Create a child session for subagent isolation
    let child_session =
        execution_state::Session::new_child(&request.child_agent_id, &request.session_id);
    let child_session_id = child_session.id.clone();

    if let Err(e) = state_service.create_session_from(&child_session) {
        tracing::warn!("Failed to create child session: {}", e);
    }

    // Generate child conversation ID (legacy, for event routing)
    let child_conversation_id = format!(
        "{}-sub-{}",
        request.session_id,
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0")
    );

    // Use the pre-created execution_id from the request
    // The execution was created synchronously by handle_delegation() to prevent
    // a race condition where try_complete_session() could mark the session
    // COMPLETED before the subagent execution exists.
    let execution_id = request.child_execution_id.clone();
    let session_id = request.session_id.clone();
    let delegation_mode =
        infer_delegation_mode(&request.child_agent_id, &request.task, request.mode);

    // Link the pre-created execution to its child session (for smart resume)
    if let Err(e) = state_service.set_child_session_id(&execution_id, &child_session_id) {
        tracing::warn!("Failed to set child_session_id on execution: {}", e);
    }

    // Start execution (QUEUED → RUNNING) and log
    start_execution(
        &state_service,
        &log_service,
        &execution_id,
        &session_id,
        &request.child_agent_id,
        Some(&request.parent_execution_id),
    );

    // Register the delegation
    let delegation_context = DelegationContext::new(
        &session_id,
        &request.parent_execution_id,
        &request.parent_agent_id,
        &child_conversation_id, // legacy conversation_id
    );
    let delegation_context =
        delegation_context.with_child_conversation_id(child_conversation_id.clone());
    let delegation_context = if let Some(ctx) = request.context.clone() {
        delegation_context.with_context(ctx)
    } else {
        delegation_context
    };
    let delegation_context = if let Some(schema) = request.output_schema.clone() {
        delegation_context.with_output_schema(schema)
    } else {
        delegation_context
    };
    let delegation_context = delegation_context.with_mode(delegation_mode);
    delegation_registry.register(&execution_id, delegation_context);

    // Note: pending_delegations is incremented synchronously in handle_delegation (stream.rs).
    // Do NOT increment again here — would double-count and break continuation.

    // Emit delegation started event
    emit_delegation_started(
        &event_bus,
        &request.parent_agent_id,
        &session_id,
        &request.parent_execution_id,
        &request.parent_conversation_id,
        &request.child_agent_id,
        &execution_id,
        &child_conversation_id,
        &request.task,
    )
    .await;

    // Load agent and provider using AgentLoader
    let agent_loader = AgentLoader::new(&agent_service, &provider_service, paths.clone());
    let (mut agent, provider) = match agent_loader
        .load_or_create_specialist(&request.child_agent_id)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            // Mark the pre-created execution as crashed so session can complete
            crash_spawn_failure(&state_service, &execution_id, &e);
            delegation_registry.remove(&execution_id);
            return Err(e);
        }
    };

    // Detect actor kind before prompt rules. Warm ward agents keep full-tool
    // actor policy even when the task contains review-like language.
    let actor_kind = actor_kind_for_delegation(&request.child_agent_id, &request.task);
    let role = match actor_kind {
        RuntimeActorKind::DelegatedReviewer => crate::invoke::SubagentRole::Reviewer,
        RuntimeActorKind::Root
        | RuntimeActorKind::DelegatedExecutor
        | RuntimeActorKind::WardAgent => crate::invoke::SubagentRole::Executor,
    };
    tracing::info!(
        child_agent = %request.child_agent_id,
        actor_kind = ?actor_kind,
        role = ?role,
        delegation_mode = %delegation_mode.as_str(),
        "Delegated actor kind detected"
    );

    // PREPEND rules as the FIRST thing in instructions.
    // Rules must come before agent AGENTS.md, ward context, specs — everything.
    // The agent reads rules first, then context. Rules frame all decisions.
    let rules = subagent_rules(role, delegation_mode);
    let original_instructions = std::mem::take(&mut agent.instructions);
    agent.instructions = format!("{}\n\n{}", rules, original_instructions);

    // Skill hints (one line)
    if !request.skills.is_empty() {
        let skill_names = request.skills.join(", ");
        agent.instructions.push_str(&format!(
            "\nRecommended skills: {}. Use load_skill to load any you need.\n",
            skill_names
        ));
    }

    // Inject output contract into child agent instructions when schema is provided
    if let Some(ref schema) = request.output_schema {
        let schema_str = serde_json::to_string_pretty(schema).unwrap_or_default();
        agent.instructions.push_str(&format!(
            "\n\n## Output Contract\nYour response MUST be a JSON object matching this schema:\n```json\n{}\n```\nRespond with ONLY the JSON object. No explanation before or after the JSON.",
            schema_str
        ));
    }

    // Collect available agents and skills for executor state
    let available_agents = collect_agents_summary(&agent_service).await;
    let available_skills = collect_skills_summary(&skill_service).await;

    // Get tool settings
    let settings_service = gateway_services::SettingsService::new(paths.clone());
    let tool_settings = settings_service.get_tool_settings().unwrap_or_default();
    let tool_result_context =
        crate::runner::prompt_safe_tool_result_config(&tool_settings, paths.vault_dir());

    // Look up the parent session's ward, then resolve the ward this
    // delegation actually runs in: a `ward:<name>` target runs in its
    // own ward; everything else inherits the parent's.
    let parent_ward_id = state_service
        .get_session(&request.session_id)
        .ok()
        .flatten()
        .and_then(|s| s.ward_id);
    let session_ward_id = effective_ward_id(&request.child_agent_id, parent_ward_id);

    // Inject ward context so subagent starts with complete knowledge
    if let Some(ref ward_id) = session_ward_id {
        let ward_dir = paths.vault_dir().join("wards").join(ward_id);
        let agents_md_path = ward_dir.join("AGENTS.md");

        if let Ok(agents_md) = std::fs::read_to_string(&agents_md_path) {
            agent
                .instructions
                .push_str(&format!("\n# Ward Context ({})\n{}\n", ward_id, agents_md));
        }

        // Inject core module docs so subagent knows available functions
        let core_docs_path = ward_dir.join("memory-bank").join("core_docs.md");
        if let Ok(core_docs) = std::fs::read_to_string(&core_docs_path) {
            // Only inject if reasonably sized (< 4KB to avoid context bloat)
            if core_docs.len() < 4096 {
                agent
                    .instructions
                    .push_str(&format!("\n# Available Core Modules\n{}\n", core_docs));
            } else {
                agent.instructions.push_str(
                    "\n# Core Modules\nSee memory-bank/core_docs.md for available functions. Read it before writing new code.\n"
                );
            }
        }

        // List active spec PATHS (not content) — agent can cat if needed.
        // Content injection was 8-12KB per delegation — too much context bloat.
        let specs_dir = ward_dir.join("specs");
        if specs_dir.exists() {
            let mut spec_files = Vec::new();
            collect_spec_files(&specs_dir, &specs_dir, &mut spec_files);
            if !spec_files.is_empty() {
                agent.instructions.push_str("\n# Specs\n");
                for rel_path in &spec_files {
                    agent.instructions.push_str(&format!("- {}\n", rel_path));
                }
            }
        }

        tracing::info!(
            child_agent = %request.child_agent_id,
            ward_id = %ward_id,
            "Injected ward context for subagent"
        );
    }

    // Build fallback-only model metadata registry.
    let model_registry = Arc::new(gateway_services::models::ModelRegistry::load());

    // Get shared rate limiter for the child's provider
    let provider_id = provider.id.clone().unwrap_or_else(|| provider.name.clone());
    let rate_limiter = {
        let guard = rate_limiters.read().unwrap_or_else(|e| e.into_inner());
        guard.get(&provider_id).cloned()
    };

    // Build executor using ExecutorBuilder
    let mut builder = ExecutorBuilder::new(paths.vault_dir().clone(), tool_settings)
        .with_model_registry(model_registry)
        .with_actor_kind(actor_kind)
        .with_initial_state("app:delegation_mode", delegation_mode.as_state_value());

    if let Some(limiter) = rate_limiter {
        builder = builder.with_rate_limiter(limiter);
    }

    // Build fact store for subagent (so save_fact uses DB, not file
    // fallback). Trait-routed memory_store is wired in both SQLite and
    // SurrealDB modes; this is what makes `memory.get_fact` /
    // `memory.save_fact` work for subagents through the configured backend.
    if let Some(fs) = memory_store.clone() {
        builder = builder.with_fact_store(fs);
    }
    if let Some(ks) = kg_store.clone() {
        builder = builder.with_kg_store(ks);
    }
    if let Some(a) = ingestion_adapter {
        builder = builder.with_ingestion_adapter(a);
    }
    if let Some(a) = goal_adapter {
        builder = builder.with_goal_adapter(a);
    }
    builder = builder
        .with_state_service(state_service.clone())
        .with_steering_registry(steering_registry.clone())
        .with_agent_result_bus(agent_result_bus.clone())
        .with_message_store(messages.clone());

    let mut executor = match builder
        .build(
            &agent,
            &provider,
            &child_conversation_id,
            &request.session_id,
            &available_agents,
            &available_skills,
            None,
            &mcp_service,
            session_ward_id.as_deref(),
        )
        .await
    {
        Ok(e) => e,
        Err(e) => {
            // Mark the pre-created execution as crashed so session can complete
            crash_spawn_failure(&state_service, &execution_id, &e);
            delegation_registry.remove(&execution_id);
            return Err(e);
        }
    };

    // Register steering handle so parent agents can steer this subagent via steer_agent tool
    let steering_handle = executor.enable_steering();
    steering_registry.register(&execution_id, steering_handle);

    // Delegation recall: prime the subagent with unified scored recall over
    // facts, wiki, procedures, graph nodes, episodes, and goals.
    let initial_history = if let Some(recall) = &memory_recall {
        let ward_id = session_ward_id.as_deref();
        match recall
            .recall_unified(&request.child_agent_id, &request.task, ward_id, &[], 10)
            .await
        {
            Ok(items) if !items.is_empty() => {
                let formatted = crate::recall::format_scored_items_with_options(
                    &items,
                    crate::recall::ContextPacketBuildOptions::new(
                        format!("{execution_id}:delegation-recall"),
                        request.child_agent_id.clone(),
                        context_actor_kind(actor_kind),
                        1_200,
                    )
                    .with_conversation_id(Some(child_conversation_id.clone()))
                    .with_ward_id(session_ward_id.clone()),
                );
                if formatted.is_empty() {
                    Vec::new()
                } else {
                    tracing::info!(
                        agent = %request.child_agent_id,
                        count = items.len(),
                        "Primed subagent with unified recalled context"
                    );
                    vec![ChatMessage::system(formatted)]
                }
            }
            Ok(_) => Vec::new(),
            Err(e) => {
                tracing::warn!(
                    agent = %request.child_agent_id,
                    error = %e,
                    "Delegation recall failed, proceeding without priming"
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Create execution handle
    // Complexity-based iteration budget (overrides default if complexity is set)
    let max_iter = match request.complexity.as_deref() {
        Some("S") => request.max_iterations.unwrap_or(15),
        Some("M") => request.max_iterations.unwrap_or(30),
        Some("L") => request.max_iterations.unwrap_or(50),
        Some("XL") => request.max_iterations.unwrap_or(100),
        _ => request.max_iterations.unwrap_or(1000),
    };
    let handle = ExecutionHandle::new(max_iter);
    let handle_clone = handle.clone();

    // Store handle (by conversation_id for general lookups, by execution_id for kill_agent)
    {
        let mut handles_guard = handles.write().await;
        handles_guard.insert(child_conversation_id.clone(), handle.clone());
    }
    agent_result_bus.register_handle(&execution_id, handle.clone());

    // The post-execution state_handoff hook reuses the same trait store
    // the executor was wired with. Cloning is cheap (Arc) and lets the
    // handoff fire after the executor has consumed its own copy.
    let fact_store_for_ctx: Option<Arc<dyn zbot_stores::MemoryFactStore>> = memory_store.clone();

    // Phase 7: pass the memory_store handle through so spawn_execution_task
    // can query ctx.state.* rows when building the ward_snapshot preamble.
    let memory_store_for_snapshot = memory_store.clone();

    // Spawn the execution task
    spawn_execution_task(SpawnContext {
        executor: select_engine(executor),
        handle: handle_clone,
        request: request.clone(),
        execution_id: execution_id.clone(),
        session_id,
        child_session_id,
        conv_id: child_conversation_id.clone(),
        event_bus,
        messages,
        session_meta,
        checkpoints,
        delegation_registry,
        delegation_tx,
        log_service,
        state_service,
        paths,
        delegation_permit,
        initial_history,
        tool_result_context,
        fact_store_for_ctx,
        memory_store_for_snapshot,
        distiller,
        steering_registry,
        agent_result_bus,
    });

    tracing::info!(
        parent_agent = %request.parent_agent_id,
        child_agent = %request.child_agent_id,
        child_conversation = %child_conversation_id,
        "Spawned delegated subagent"
    );

    Ok(child_conversation_id)
}

/// Return the `<reuse_check>` imperative for coding-capable agents.
///
/// Injected at the top of the task prompt (above the ward_snapshot)
/// for agents that write or modify code. The block uses concrete ✓/✗
/// examples because anchored patterns steer LLMs more reliably than
/// abstract policy text. Sonnet 4.6 complies with this style at >95%
/// in our evals; weaker models need the validation loop too.
fn reuse_check_block(agent_id: &str) -> Option<&'static str> {
    let writes_code = matches!(agent_id, "code-agent" | "data-analyst");
    if !writes_code {
        return None;
    }
    Some(
        "<reuse_check>\n\
         Before writing ANY code:\n\
         1. Read the <ward_snapshot> below. If it contains a Conventions block, follow it exactly — it declares this ward's language, module_root (where reusable code lives), import_syntax, and signature_registry. Do not improvise a different layout.\n\
         2. Inspect the Primitives section. Every listed symbol is importable from the ward's module_root. Plan your imports against them.\n\
         3. Emit a reuse_audit block in your first action or respond() message, in this exact shape:\n\
         \n\
           reuse_audit:\n\
             looking_for: <symbols you need>\n\
             found:       <subset already listed in Primitives — will import via import_syntax>\n\
             missing:     <subset not listed — will add to module_root/ and register in signature_registry>\n\
             plan:        <one-sentence import+implement sequence>\n\
         \n\
         4. THEN write code. Imports use the Conventions' import_syntax. New primitives go under module_root/ (at the WARD ROOT, never inside the task directory) and get appended to signature_registry.\n\
         \n\
         ✓ CORRECT: Import a listed primitive via the ward's import_syntax; call it with new args.\n\
         ✓ CORRECT: Extend a primitive to accept a new argument (parameterize in place, don't fork a near-copy under a new name).\n\
         ✓ CORRECT: Add a genuinely new primitive to module_root/, register it, then import from the task script.\n\
         ✗ WRONG: Writing a parallel copy of a listed primitive under a different name (e.g. `goog-dcf-model.py` when `core/valuation.py::dcf_valuation(...)` is listed).\n\
         ✗ WRONG: Putting reusable code inside the task directory (`<task>/core/`, `<task>/lib/`). Reusable code is ward-level per Conventions.\n\
         ✗ WRONG: Writing code without emitting the reuse_audit block first — that's guessing, not reusing.\n\
         </reuse_check>",
    )
}

/// All inputs required to spawn the async delegated-agent execution task.
///
/// Replaces an 18-positional-argument function signature. Using a struct
/// literal at the call site means:
///
/// - Adding a 19th field is a single line in both this struct and the
///   caller — no positional reshuffling.
/// - Swapping two like-typed fields (e.g. `session_id` vs `child_session_id`,
///   both `String`) is caught by name at construction, not silently at runtime.
/// - Fields can be reordered freely without breaking call sites.
struct SpawnContext {
    // --- Execution identity ---
    executor: BoxedAgentEngine,
    handle: ExecutionHandle,
    request: DelegationRequest,
    execution_id: String,
    session_id: String,
    child_session_id: String,
    /// Child conversation id (what the downstream stream/log services key on).
    conv_id: String,
    initial_history: Vec<ChatMessage>,
    tool_result_context: ToolResultContextConfig,

    // --- Resource control ---
    delegation_permit: Option<OwnedSemaphorePermit>,

    // --- Shared services ---
    event_bus: Arc<EventBus>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    delegation_registry: Arc<DelegationRegistry>,
    delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    log_service: Arc<LogService<DatabaseManager>>,
    state_service: Arc<StateService<DatabaseManager>>,
    paths: SharedVaultPaths,

    // --- Optional memory wiring (Phase 4b + 7 ward_snapshot preamble) ---
    fact_store_for_ctx: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    memory_store_for_snapshot: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    /// Distiller for the subagent's child session — fired after
    /// `complete_session(child_session_id)`.
    distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    /// Steering registry — used to remove the handle when the subagent finishes.
    steering_registry: Arc<agent_runtime::SteeringRegistry>,
    /// Result bus — resolves wait_agent and kill_agent primitives.
    agent_result_bus: Arc<AgentResultBus>,
}

/// Spawn the async execution task for the delegated agent.
fn spawn_execution_task(ctx: SpawnContext) {
    let SpawnContext {
        executor,
        handle,
        request,
        execution_id,
        session_id,
        child_session_id,
        conv_id,
        initial_history,
        tool_result_context,
        delegation_permit,
        event_bus,
        messages,
        session_meta,
        checkpoints,
        delegation_registry,
        delegation_tx,
        log_service,
        state_service,
        paths,
        fact_store_for_ctx,
        memory_store_for_snapshot,
        distiller,
        steering_registry,
        agent_result_bus,
    } = ctx;

    let agent_id = request.child_agent_id.clone();

    // Phase 4b + 7: build the task prefix layers.
    //
    // Outer layer (Phase 7): <ward_snapshot> block with AGENTS.md,
    // memory-bank/ward.md, memory-bank/core_docs.md, memory-bank/structure.md
    // read fresh from disk, plus the recent state.<exec_id> handoffs from
    // this session's ctx. Push model — the subagent sees what exists in
    // the ward without having to query.
    //
    // Inner layer (Phase 4b): <session_ctx ... /> tag with sid + tool
    // hint so the subagent can fetch more ctx fields on demand.
    //
    // Ward lookup is cheap and falls back to "__global__" if it can't be resolved.
    let ward_for_preamble = session_meta.session_ward_id(&session_id).ok().flatten();

    // Step 1 of 2: session_ctx tag (always emitted, tiny)
    let with_ctx_tag = crate::session_ctx::preamble::prepend_to_task(
        &session_id,
        ward_for_preamble.as_deref(),
        None, // step position — plumbed in a follow-up
        None,
        &[], // prior_states — plumbed in a follow-up
        &request.task,
    );

    let parent_agent = request.parent_agent_id.clone();
    let parent_execution_id = request.parent_execution_id.clone();
    let paths_for_snapshot = paths.clone();
    let session_id_for_snapshot = session_id.clone();
    let child_agent_id_for_block = request.child_agent_id.clone();

    tokio::spawn(async move {
        // Hold the delegation permit for the duration of the task.
        // When this task completes (or is dropped), the permit is released,
        // allowing another queued delegation to proceed.
        let _delegation_permit = delegation_permit;

        // Step 2: ward_snapshot block (prepended if a real ward is active).
        // Reads ward_snapshot via `MemoryFactStore`, so awaited inside the
        // spawned task rather than in the sync caller.
        let with_snapshot = if let Some(ref ward) = ward_for_preamble {
            crate::session_ctx::snapshot::prepend_to_task(
                ward,
                &session_id_for_snapshot,
                &paths_for_snapshot.wards_dir(),
                memory_store_for_snapshot.as_ref(),
                &with_ctx_tag,
            )
            .await
        } else {
            with_ctx_tag
        };

        // Step 3: reuse_check imperative for coding-capable agents.
        // Placed at the very top of the prompt so the LLM reads it before
        // any other context. Concrete ✓/✗ examples anchor the behavior —
        // stronger than free-text policy recall.
        let task_msg = if let Some(block) = reuse_check_block(&child_agent_id_for_block) {
            format!("{}\n\n{}", block, with_snapshot)
        } else {
            with_snapshot
        };

        // Create batch writer for session message and trace streaming.
        let batch_writer = spawn_batch_writer_with_traces(
            state_service.clone(),
            log_service.clone(),
            paths.traces_dir(),
            messages.clone(),
        );

        // Create stream context for event processing
        let stream_ctx = StreamContext::new(
            agent_id.clone(),
            conv_id.clone(),
            session_id.clone(),
            execution_id.clone(),
            event_bus.clone(),
            log_service.clone(),
            state_service.clone(),
            delegation_tx,
            paths.vault_dir().clone(),
        )
        .with_batch_writer(batch_writer.clone());

        let mut response_acc = ResponseAccumulator::new();

        // Append task message to child session stream
        batch_writer.session_message(
            &child_session_id,
            &execution_id,
            "user",
            &task_msg,
            None,
            None,
        );

        let child_session_id_inner = child_session_id.clone();
        let execution_id_inner = execution_id.clone();
        let batch_writer_inner = batch_writer.clone();
        let mut turn_tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut turn_text = String::new();
        let mut current_tool_name = String::new();

        let stop_sig = Some(handle.stop_signal());
        let mut on_event = |event| {
            if handle.is_stop_requested() {
                return;
            }

            handle.increment();

            // Stream messages to child session
            match &event {
                agent_runtime::StreamEvent::ToolCallStart {
                    tool_id,
                    tool_name,
                    args,
                    ..
                } => {
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
                    if !turn_tool_calls.is_empty() {
                        let tc_json = serde_json::to_string(&turn_tool_calls).unwrap_or_default();
                        let content = if turn_text.is_empty() {
                            "[tool calls]".to_string()
                        } else {
                            std::mem::take(&mut turn_text)
                        };
                        batch_writer_inner.session_message(
                            &child_session_id_inner,
                            &execution_id_inner,
                            "assistant",
                            &content,
                            Some(&tc_json),
                            None,
                        );
                        turn_tool_calls.clear();
                    }

                    let tool_content = crate::runner::prompt_safe_tool_content(
                        &current_tool_name,
                        result,
                        context_result.as_deref(),
                        error.as_deref(),
                        &tool_result_context,
                    );
                    batch_writer_inner.session_message(
                        &child_session_id_inner,
                        &execution_id_inner,
                        "tool",
                        &tool_content,
                        None,
                        Some(tool_id),
                    );
                }
                agent_runtime::StreamEvent::Token { content, .. } => {
                    turn_text.push_str(content);
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
            .execute_stream_with_stop_flag(&task_msg, &initial_history, stop_sig, &mut on_event)
            .await;

        let accumulated_response = response_acc.into_response();

        // Emit final assistant response to child session stream
        if !accumulated_response.is_empty() {
            batch_writer.session_message(
                &child_session_id,
                &execution_id,
                "assistant",
                &accumulated_response,
                None,
                None,
            );
        }

        // Turn-boundary checkpoint — write a versioned snapshot of the
        // subagent's context state so session_state can read it in O(1).
        crate::runner::core::write_turn_checkpoint(
            &checkpoints,
            &state_service,
            &execution_id,
            &child_session_id,
            handle.current_iteration(),
            &accumulated_response,
        );

        match result {
            Ok(()) => {
                // Unblock any wait_agent before firing callbacks.
                agent_result_bus.resolve(&execution_id, &agent_id, &accumulated_response);

                handle_execution_success(HandleExecutionSuccess {
                    messages: messages.as_ref(),
                    session_meta: session_meta.as_ref(),
                    state_service: &state_service,
                    log_service: &log_service,
                    event_bus: &event_bus,
                    delegation_registry: &delegation_registry,
                    execution_id: &execution_id,
                    session_id: &session_id,
                    agent_id: &agent_id,
                    conv_id: &conv_id,
                    response: &accumulated_response,
                    parent_agent: &parent_agent,
                    parent_execution_id: &parent_execution_id,
                    fact_store_for_ctx: fact_store_for_ctx.as_ref(),
                })
                .await;
            }
            Err(agent_runtime::ExecutorError::Stopped) => {
                // Cooperative stop cascaded from parent — reject any waiter then exit cleanly.
                agent_result_bus.reject(
                    &execution_id,
                    AgentWaitError::Crashed {
                        error: "agent stopped cooperatively".to_string(),
                    },
                );
                tracing::info!(
                    session_id = %session_id,
                    "Delegated agent stopped cooperatively"
                );
            }
            Err(e) => {
                // Build structured crash report with plan status and ward files
                let crash_report = build_crash_report(
                    &agent_id,
                    &e.to_string(),
                    messages.as_ref(),
                    &child_session_id,
                    &state_service,
                    &session_id,
                    &paths,
                );

                // Unblock any wait_agent with the crash error before DB writes.
                agent_result_bus.reject(
                    &execution_id,
                    AgentWaitError::Crashed {
                        error: crash_report.clone(),
                    },
                );

                handle_execution_failure(HandleExecutionFailure {
                    messages: messages.as_ref(),
                    state_service: &state_service,
                    log_service: &log_service,
                    event_bus: &event_bus,
                    delegation_registry: &delegation_registry,
                    execution_id: &execution_id,
                    session_id: &session_id,
                    agent_id: &agent_id,
                    conv_id: &conv_id,
                    parent_execution_id: &parent_execution_id,
                    error: &crash_report,
                })
                .await;
            }
        }

        // Deregister steering handle — subagent is no longer steerable
        steering_registry.remove(&execution_id);

        // Mark child session as completed (prevents orphaned "running" sessions)
        if let Err(e) = state_service.complete_session(&child_session_id) {
            tracing::warn!(child_session_id = %child_session_id, "Failed to complete child session: {}", e);
        }

        // Fire-and-forget subagent distillation. The subagent's transcript
        // is the only place where the substantive tool results (web_fetch
        // article content, shell stdout, etc.) live — root distillation
        // sees only the admin chatter (delegate_to_agent + callbacks), so
        // without this the KG misses everything subagents discover.
        if let Some(distiller) = distiller.as_ref() {
            let distiller = distiller.clone();
            let sid = child_session_id.clone();
            let aid = agent_id.clone();
            tokio::spawn(async move {
                if let Err(e) = distiller.distill(&sid, &aid).await {
                    tracing::warn!(
                        child_session_id = %sid,
                        agent_id = %aid,
                        error = %e,
                        "Subagent distillation failed"
                    );
                }
            });
        }
    });
}

/// Inputs for `handle_execution_success` — same pattern as `SpawnContext`
/// but borrowed (these are called from inside the spawn-owned async closure).
struct HandleExecutionSuccess<'a> {
    messages: &'a dyn zbot_conversation::MessageStore,
    session_meta: &'a dyn zbot_conversation::SessionMetaStore,
    state_service: &'a StateService<DatabaseManager>,
    log_service: &'a LogService<DatabaseManager>,
    event_bus: &'a EventBus,
    delegation_registry: &'a DelegationRegistry,
    execution_id: &'a str,
    session_id: &'a str,
    agent_id: &'a str,
    conv_id: &'a str,
    response: &'a str,
    parent_agent: &'a str,
    parent_execution_id: &'a str,
    fact_store_for_ctx: Option<&'a Arc<dyn zbot_stores::MemoryFactStore>>,
}

/// Handle successful execution completion.
async fn handle_execution_success(ctx: HandleExecutionSuccess<'_>) {
    let HandleExecutionSuccess {
        messages,
        session_meta,
        state_service,
        log_service,
        event_bus,
        delegation_registry,
        execution_id,
        session_id,
        agent_id,
        conv_id,
        response,
        parent_agent,
        parent_execution_id,
        fact_store_for_ctx,
    } = ctx;
    // Messages already streamed to child session during execution

    // Complete execution and emit events
    // Delegations don't dispatch to connectors (they're internal subagent calls)
    complete_execution(CompleteExecution {
        state_service,
        log_service,
        event_bus,
        execution_id,
        session_id,
        agent_id,
        conversation_id: conv_id,
        response: Some(response.to_string()),
        connector_registry: None,
        respond_to: None,
        thread_id: None,
        bridge_registry: None,
        bridge_outbox: None,
    })
    .await;

    // Get delegation context before removing (for callback check)
    let delegation_ctx = delegation_registry.get(execution_id);

    // Emit delegation completed with proper conversation IDs for routing
    let parent_conv_id = delegation_ctx
        .as_ref()
        .map(|ctx| ctx.parent_conversation_id.as_str());
    emit_delegation_completed(DelegationCompletedEvent {
        event_bus,
        parent_agent_id: parent_agent,
        session_id,
        child_agent_id: agent_id,
        child_execution_id: execution_id,
        parent_conversation_id: parent_conv_id,
        child_conversation_id: Some(conv_id),
        result: Some(response.to_string()),
    })
    .await;

    // Persist the parent callback before marking delegation completion. The
    // completion event wakes the continuation watcher; waking it first creates
    // a race where the root can resume without the child result in context.
    handle_delegation_success(
        delegation_ctx.as_ref(),
        messages,
        event_bus,
        session_id,
        parent_execution_id,
        agent_id,
        conv_id,
        response,
    )
    .await;

    // Check if this was the last delegation and continuation is needed
    match state_service.complete_delegation(session_id) {
        Ok(true) => {
            // Get root execution for continuation
            if let Ok(Some(root_exec)) = state_service.get_root_execution(session_id) {
                event_bus
                    .publish(GatewayEvent::SessionContinuationReady {
                        session_id: session_id.to_string(),
                        root_agent_id: root_exec.agent_id.clone(),
                        root_execution_id: root_exec.id.clone(),
                    })
                    .await;
                tracing::info!(
                    session_id = %session_id,
                    root_execution_id = %root_exec.id,
                    "All delegations complete, continuation ready"
                );
            }
        }
        Ok(false) => {} // More delegations pending
        Err(e) => tracing::warn!("Failed to complete delegation tracking: {}", e),
    }

    // Phase 2b: write a state_handoff fact so the next subagent in this
    // session can fetch this one's summary by exact key. We look up the
    // session's ward so the ctx row is stored per-ward (matches plan +
    // intent snapshots). Fire-and-forget — a failed write logs a warning
    // but never disrupts delegation completion.
    if let Some(fs) = fact_store_for_ctx {
        let ward_id = session_meta
            .session_ward_id(session_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| "__global__".to_string());
        crate::session_ctx::writer::state_handoff(
            fs,
            session_id,
            &ward_id,
            execution_id,
            agent_id,
            None, // step number not known at this call site
            chrono::Utc::now(),
            response, // the subagent's respond() payload becomes the summary
            &[],      // artifacts extraction deferred — subagent responds may
                      // include them in future revisions
        )
        .await;
    }

    // Remove from delegation registry
    delegation_registry.remove(execution_id);
}

/// Mark a pre-created execution as crashed when spawn fails.
///
/// This is called when the spawn process fails early (e.g., agent not found,
/// executor build error). The execution was created with status QUEUED in
/// `handle_delegation()`, so we need to mark it CRASHED to allow the session
/// to complete properly.
fn crash_spawn_failure(
    state_service: &StateService<DatabaseManager>,
    execution_id: &str,
    error: &str,
) {
    if let Err(e) = state_service.crash_execution(execution_id, error) {
        tracing::warn!(
            execution_id = %execution_id,
            error = %e,
            "Failed to mark spawn failure as crashed"
        );
    } else {
        tracing::info!(
            execution_id = %execution_id,
            error = %error,
            "Marked failed spawn as crashed"
        );
    }
}

/// Inputs for `handle_execution_failure`. Same-type String ids travel together;
/// named fields prevent order-swap bugs between `session_id` and
/// `parent_execution_id`.
struct HandleExecutionFailure<'a> {
    messages: &'a dyn zbot_conversation::MessageStore,
    state_service: &'a StateService<DatabaseManager>,
    log_service: &'a LogService<DatabaseManager>,
    event_bus: &'a EventBus,
    delegation_registry: &'a DelegationRegistry,
    execution_id: &'a str,
    session_id: &'a str,
    agent_id: &'a str,
    conv_id: &'a str,
    parent_execution_id: &'a str,
    error: &'a str,
}

/// Handle execution failure.
async fn handle_execution_failure(ctx: HandleExecutionFailure<'_>) {
    let HandleExecutionFailure {
        messages,
        state_service,
        log_service,
        event_bus,
        delegation_registry,
        execution_id,
        session_id,
        agent_id,
        conv_id,
        parent_execution_id,
        error,
    } = ctx;
    // Messages already streamed to child session during execution

    // Crash execution and emit events (don't crash session for subagent)
    crash_execution(CrashExecution {
        state_service,
        log_service,
        event_bus,
        execution_id,
        session_id,
        agent_id,
        conversation_id: conv_id,
        error,
        crash_session: false, // don't crash session for subagent
    })
    .await;

    // Send error callback to parent
    handle_delegation_failure(
        messages,
        event_bus,
        session_id,
        parent_execution_id,
        agent_id,
        conv_id,
        error,
    )
    .await;

    // Check if this was the last delegation and continuation is needed
    // (even failures count as completed delegations)
    match state_service.complete_delegation(session_id) {
        Ok(true) => {
            if let Ok(Some(root_exec)) = state_service.get_root_execution(session_id) {
                event_bus
                    .publish(GatewayEvent::SessionContinuationReady {
                        session_id: session_id.to_string(),
                        root_agent_id: root_exec.agent_id.clone(),
                        root_execution_id: root_exec.id.clone(),
                    })
                    .await;
                tracing::info!(
                    session_id = %session_id,
                    "All delegations complete (including failed), continuation ready"
                );
            }
        }
        Ok(false) => {}
        Err(e) => tracing::warn!("Failed to complete delegation tracking: {}", e),
    }

    delegation_registry.remove(execution_id);
}

/// Build a structured crash report with plan status and ward file listing.
///
/// When a subagent fails, this provides the parent agent with actionable
/// intelligence about what was accomplished before the crash, enabling
/// better retry strategies.
fn build_crash_report(
    agent_id: &str,
    error: &str,
    messages: &dyn zbot_conversation::MessageStore,
    child_session_id: &str,
    state_service: &StateService<DatabaseManager>,
    parent_session_id: &str,
    paths: &SharedVaultPaths,
) -> String {
    let mut report = format!("DELEGATION FAILED: {}\n\nERROR: {}\n", agent_id, error);

    // Try to extract plan status from child session messages.
    // Plan updates appear as tool results containing JSON with `__plan_update: true`.
    let mut found_plan = false;
    if let Ok(messages) = messages.replay(child_session_id, None, 200) {
        // Scan tool-result messages for plan updates (last one is most recent)
        let plan_messages: Vec<_> = messages
            .iter()
            .filter(|m| m.content.contains("__plan_update"))
            .collect();

        if let Some(last_plan_msg) = plan_messages.last() {
            if let Ok(plan_data) = serde_json::from_str::<serde_json::Value>(&last_plan_msg.content)
            {
                if let Some(steps) = plan_data.get("plan").and_then(|p| p.as_array()) {
                    let completed: Vec<_> = steps
                        .iter()
                        .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("completed"))
                        .filter_map(|s| s.get("step").and_then(|v| v.as_str()))
                        .collect();
                    let pending: Vec<_> = steps
                        .iter()
                        .filter(|s| s.get("status").and_then(|v| v.as_str()) != Some("completed"))
                        .filter_map(|s| s.get("step").and_then(|v| v.as_str()))
                        .collect();

                    found_plan = true;
                    if !completed.is_empty() {
                        report.push_str("\nCOMPLETED STEPS:\n");
                        for s in &completed {
                            report.push_str(&format!("  [done] {}\n", s));
                        }
                    }
                    if !pending.is_empty() {
                        report.push_str("\nREMAINING STEPS:\n");
                        for s in &pending {
                            report.push_str(&format!("  [todo] {}\n", s));
                        }
                    }
                }
            }
        }
    }

    if !found_plan {
        report.push_str("\nPARTIAL WORK COMPLETED:\nNo plan was created\n");
    }

    // List files in the ward (if one is active for this session)
    if let Ok(Some(session)) = state_service.get_session(parent_session_id) {
        if let Some(ward_id) = &session.ward_id {
            let ward_dir = paths.ward_dir(ward_id);
            if ward_dir.exists() {
                if let Ok(entries) = walkdir_simple(&ward_dir) {
                    if !entries.is_empty() {
                        report.push_str("\nFILES IN WARD:\n");
                        for entry in entries.iter().take(20) {
                            report.push_str(&format!("  {}\n", entry));
                        }
                        if entries.len() > 20 {
                            report.push_str(&format!(
                                "  ... and {} more files\n",
                                entries.len() - 20
                            ));
                        }
                    }
                }
            }
        }
    }

    report.push_str(
        "\nSUGGESTION: Break remaining work into smaller, focused tasks. \
         Existing files can be reused.\n",
    );
    report
}

/// Recursively collect .md spec file paths relative to specs_root.
fn collect_spec_files(dir: &std::path::Path, specs_root: &std::path::Path, out: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip archive directory
                if path.file_name().map(|n| n == "archive").unwrap_or(false) {
                    continue;
                }
                collect_spec_files(&path, specs_root, out);
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(rel) = path.strip_prefix(specs_root) {
                    out.push(format!("specs/{}", rel.display()));
                }
            }
        }
    }
}

/// The ward directory a delegated agent should operate in. A `ward:<name>`
/// delegation always runs in its own ward, regardless of the parent's
/// active ward; any other agent inherits the parent session's ward.
fn effective_ward_id(child_agent_id: &str, parent_ward_id: Option<String>) -> Option<String> {
    match child_agent_id.strip_prefix("ward:") {
        Some(name) => Some(name.to_string()),
        None => parent_ward_id,
    }
}

fn actor_kind_for_delegation(child_agent_id: &str, task: &str) -> RuntimeActorKind {
    if child_agent_id.strip_prefix("ward:").is_some() {
        RuntimeActorKind::WardAgent
    } else {
        RuntimeActorKind::from(detect_subagent_role(child_agent_id, task))
    }
}

fn context_actor_kind(actor_kind: RuntimeActorKind) -> ContextActorKind {
    match actor_kind {
        RuntimeActorKind::Root => ContextActorKind::Root,
        RuntimeActorKind::DelegatedExecutor => ContextActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer => ContextActorKind::DelegatedReviewer,
        RuntimeActorKind::WardAgent => ContextActorKind::WardAgent,
    }
}

/// Simple recursive directory listing that skips hidden files and common noise.
fn walkdir_simple(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if name.starts_with('.') || name.contains("__pycache__") {
            continue;
        }
        if path.is_file() {
            files.push(name);
        } else if path.is_dir() {
            if let Ok(sub_files) = walkdir_simple(&path) {
                for sf in sub_files {
                    files.push(format!("{}/{}", name, sf));
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_ward_id_uses_ward_prefix_over_parent() {
        assert_eq!(
            effective_ward_id("ward:maritime", Some("finance".to_string())),
            Some("maritime".to_string())
        );
    }

    #[test]
    fn effective_ward_id_falls_back_to_parent_for_normal_agents() {
        assert_eq!(
            effective_ward_id("planner", Some("finance".to_string())),
            Some("finance".to_string())
        );
        assert_eq!(effective_ward_id("planner", None), None);
    }

    #[test]
    fn effective_ward_id_uses_ward_prefix_when_parent_is_none() {
        // The primary case: the root delegates to a ward without itself
        // being in one — the ward delegation still lands in its own ward.
        assert_eq!(
            effective_ward_id("ward:maritime", None),
            Some("maritime".to_string())
        );
    }

    #[test]
    fn actor_kind_for_delegation_detects_reviewers() {
        assert_eq!(
            actor_kind_for_delegation("reviewer-agent", "Review the implementation"),
            RuntimeActorKind::DelegatedReviewer
        );
        assert_eq!(
            actor_kind_for_delegation("builder-agent", "Build the implementation"),
            RuntimeActorKind::DelegatedExecutor
        );
    }

    #[test]
    fn actor_kind_for_delegation_keeps_ward_agents_full_tool() {
        assert_eq!(
            actor_kind_for_delegation("ward:maritime", "Review the implementation"),
            RuntimeActorKind::WardAgent
        );
    }
}
