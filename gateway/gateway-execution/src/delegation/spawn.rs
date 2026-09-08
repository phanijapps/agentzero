//! # Delegation Spawning
//!
//! Handles spawning of delegated subagents.

use super::callback::{handle_delegation_failure, handle_delegation_success};
use super::context::{infer_delegation_mode, DelegationContext, DelegationMode, DelegationRequest};
use super::registry::DelegationRegistry;
use crate::errors::ExecutionError;
use agent_runtime::{BoxedAgentEngine, ContextActorKind, ToolResultContextConfig};
use api_logs::{ExecutionLog, LogCategory, LogLevel, LogService};
use execution_state::{SessionWardClaim, StateService};
use gateway_events::{EventBus, GatewayEvent};
use gateway_services::{AgentService, SharedVaultPaths, SkillService};
#[cfg(test)]
use gateway_services::{McpService, ProviderService};
#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Component, Path};
use std::sync::Arc;
#[cfg(test)]
use tokio::sync::RwLock;
use tokio::sync::{mpsc, OwnedSemaphorePermit};
use zbot_runtime_sqlite::DatabaseManager;

use crate::agent_pool::{AgentResultBus, AgentWaitError};

use agent_runtime::ChatMessage;

use crate::handle::ExecutionHandle;
use crate::invoke::{
    broadcast_event, build_execution_engine, collect_agents_summary, collect_skills_summary,
    detect_subagent_role, mcp_startup_failure_observer, process_stream_event,
    spawn_batch_writer_with_traces, subagent_rules, AgentLoader, ExecutorBuilder,
    ResponseAccumulator, RuntimeActorKind, StreamContext,
};
use crate::lifecycle::{
    complete_execution, crash_execution, emit_delegation_completed, emit_delegation_started,
    start_execution, CompleteExecution, CrashExecution, DelegationCompletedEvent,
};

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
    ctx: &crate::runner::exec_ctx::ExecCtx,
    request: &DelegationRequest,
    delegation_permit: Option<OwnedSemaphorePermit>,
) -> Result<String, ExecutionError> {
    let event_bus = ctx.event_bus.clone();
    let agent_service = ctx.agent_service.clone();
    let provider_service = ctx.provider_service.clone();
    let mcp_service = ctx.mcp_service.clone();
    let skill_service = ctx.skill_service.clone();
    let paths = ctx.paths.clone();
    let messages = ctx.messages.clone();
    let session_meta = ctx.session_meta.clone();
    let checkpoints = ctx.checkpoints.clone();
    let handles = ctx.control.handles.clone();
    let delegation_registry = ctx.control.delegation_registry.clone();
    let delegation_tx = ctx.delegation_tx.clone();
    let log_service = ctx.log_service.clone();
    let state_service = ctx.state_service.clone();
    let memory_store = ctx.memory_store.clone();
    let distiller = ctx.distiller.clone();
    let memory_recall = ctx.memory_recall.clone();
    let peer_messages = ctx.peer_messages.clone();
    let a2a_delegation = ctx.a2a_delegation.clone();
    let rate_limiters = ctx.rate_limiters.clone();
    let integrations_snapshot = ctx.integrations.snapshot();
    let kg_store = integrations_snapshot.kg_store;
    let ingestion_adapter = integrations_snapshot.ingestion_adapter;
    #[allow(unused_variables)]
    let goal_adapter = integrations_snapshot.goal_adapter;
    let steering_registry = ctx.steering_registry.clone();
    let agent_result_bus = ctx.agent_result_bus.clone();

    // Generate the child conversation identity before validation so even an
    // assignment rejected prior to child-session creation can use the common
    // failure callback and delegation-completion path.
    let child_conversation_id = format!(
        "{}-sub-{}",
        request.session_id,
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0")
    );

    // Reject a mismatched or nonexistent dynamic target before creating the
    // child session or invoking AgentLoader. This prevents the loader's
    // compatibility auto-create path from turning an untrusted model target
    // into a capability-bearing specialist.
    let has_dynamic_assignment = request.capability_assignment.is_some();
    let ward_target_is_valid = !request.child_agent_id.starts_with("ward:")
        || dynamic_target_exists(&agent_service, paths.vault_dir(), &request.child_agent_id).await;
    if !ward_target_is_valid
        || !validate_dynamic_assignment_target(
            &agent_service,
            paths.vault_dir(),
            &request.child_agent_id,
            request.capability_assignment.as_ref(),
        )
        .await
    {
        log_capability_resolution(
            &log_service,
            CapabilityResolutionLog {
                execution_id: &request.child_execution_id,
                session_id: &request.session_id,
                agent_id: &request.child_agent_id,
                origin: "rejected_target",
                requested_skills: &[],
                requested_mcps: &[],
                effective_skills: &[],
                effective_mcps: &[],
                unresolved_skill_count: 1,
                unresolved_count: 1,
                rejection_codes: &[],
            },
        );
        let error = "Dynamic capability assignment target rejected";
        handle_early_spawn_failure(EarlySpawnFailure {
            request,
            child_conversation_id: &child_conversation_id,
            child_session_id: None,
            error,
            messages: messages.as_ref(),
            state_service: &state_service,
            log_service: &log_service,
            event_bus: &event_bus,
            delegation_registry: &delegation_registry,
            agent_result_bus: &agent_result_bus,
        })
        .await;
        return Err(ExecutionError::from(error.to_string()));
    }

    // A ward-agent target is an authoritative execution workspace. Persist it
    // on the parent before the child starts, then copy the effective workspace
    // to the isolated child session. Artifact declarations read their own
    // session row, so executor-only context is insufficient here.
    let (parent_ward_id, claimed_ward) = bind_parent_ward_for_delegation(&state_service, request)?;
    if let Some(ward_id) = claimed_ward {
        event_bus
            .publish(GatewayEvent::WardChanged {
                session_id: request.session_id.clone(),
                execution_id: request.parent_execution_id.clone(),
                ward_id,
            })
            .await;
    }
    let session_ward_id = effective_ward_id(&request.child_agent_id, parent_ward_id);

    // Create a child session for subagent isolation.
    let mut child_session =
        execution_state::Session::new_child(&request.child_agent_id, &request.session_id);
    child_session.ward_id = session_ward_id.clone();
    let child_session_id = child_session.id.clone();

    if let Err(e) = state_service.create_session_from(&child_session) {
        tracing::warn!("Failed to create child session: {}", e);
    }

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

    // Dynamic assignments are valid only for an existing configured agent or
    // an already-created ward. This check occurs before `AgentLoader` can
    // auto-create a specialist, so an arbitrary delegate target can never
    // obtain an MCP merely by naming one in a tool call.
    let dynamic_assignment = request.capability_assignment.as_ref();

    // Load agent and provider using AgentLoader
    let agent_loader = AgentLoader::new(&agent_service, &provider_service, paths.clone());
    let (mut agent, provider) = match agent_loader
        .load_or_create_specialist(&request.child_agent_id)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            handle_early_spawn_failure(EarlySpawnFailure {
                request,
                child_conversation_id: &child_conversation_id,
                child_session_id: Some(&child_session_id),
                error: &e.to_string(),
                messages: messages.as_ref(),
                state_service: &state_service,
                log_service: &log_service,
                event_bus: &event_bus,
                delegation_registry: &delegation_registry,
                agent_result_bus: &agent_result_bus,
            })
            .await;
            return Err(e);
        }
    };

    // The dedicated planner is descriptive only. It can discover available
    // capabilities through the host catalog but must never start an MCP while
    // deciding the graph.
    if request.child_agent_id == "planner-agent" {
        agent.mcps.clear();
    }

    // Detect actor kind before prompt rules. Warm ward agents keep full-tool
    // actor policy even when the task contains review-like language.
    let actor_kind = actor_kind_for_delegation(&request.child_agent_id, &request.task);
    let role = match actor_kind {
        RuntimeActorKind::DelegatedReviewer => crate::invoke::SubagentRole::Reviewer,
        RuntimeActorKind::Root
        | RuntimeActorKind::DelegatedExecutor
        | RuntimeActorKind::WardAgent
        | RuntimeActorKind::RemotePeer => crate::invoke::SubagentRole::Executor,
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

    // Explicit dynamic skills are recommendations to the existing lazy
    // `load_skill` workflow. Validate the planner/root choice against the
    // current service catalog before it reaches model instructions; omitted
    // legacy assignments retain their prior hint unchanged.
    let dynamic_skill_resolution = match dynamic_assignment {
        Some(assignment) => resolve_dynamic_skills(&skill_service, &assignment.skills).await,
        None => None,
    };
    let recommended_skills: &[String] = dynamic_skill_resolution.as_ref().map_or_else(
        || {
            if has_dynamic_assignment {
                &[] as &[String]
            } else {
                request.skills.as_slice()
            }
        },
        |resolution| resolution.effective.as_slice(),
    );
    if !recommended_skills.is_empty() {
        let skill_names = recommended_skills.join(", ");
        agent.instructions.push_str(&format!(
            "\nRecommended skills: {}. Use load_skill to load any you need.\n",
            skill_names
        ));
    }

    // An explicit assignment overrides static `agent.mcps`, including an
    // empty array. Resolution accepts only canonical enabled/runtime-ready
    // server IDs and records aggregate reason codes without raw request data.
    if request.child_agent_id == "planner-agent" {
        // The planner may discover and record capabilities, but it is never
        // an execution target. A model-supplied mapping cannot remount MCPs
        // after the static planner list was cleared above.
        agent.mcps.clear();
        if let Some(assignment) = dynamic_assignment {
            log_capability_resolution(
                &log_service,
                CapabilityResolutionLog {
                    execution_id: &execution_id,
                    session_id: &session_id,
                    agent_id: &request.child_agent_id,
                    origin: "planner_runtime_prohibited",
                    requested_skills: dynamic_skill_resolution
                        .as_ref()
                        .map_or(&[], |resolution| resolution.effective.as_slice()),
                    requested_mcps: &[],
                    effective_skills: dynamic_skill_resolution
                        .as_ref()
                        .map_or(&[], |resolution| resolution.effective.as_slice()),
                    effective_mcps: &[],
                    unresolved_skill_count: dynamic_skill_resolution
                        .as_ref()
                        .map_or(assignment.skills.len(), |resolution| {
                            resolution.unresolved_count
                        }),
                    unresolved_count: assignment.mcps.len(),
                    rejection_codes: &["planner_runtime_prohibited"],
                },
            );
        }
    } else if let Some(assignment) = dynamic_assignment {
        let known_mcp_ids = catalog_mcp_ids(request.planning_capability_catalog.as_ref());
        let known_mcp_id_set = known_mcp_ids.iter().collect::<HashSet<_>>();
        let known_requested_mcps = assignment
            .mcps
            .iter()
            .filter(|id| known_mcp_id_set.contains(id))
            .cloned()
            .collect::<Vec<_>>();
        match mcp_service.resolve_dynamic_runtime_ids_with_catalog(&assignment.mcps, &known_mcp_ids)
        {
            Ok(resolution) => {
                agent.mcps = resolution.effective_ids.clone();
                let rejection_codes = resolution
                    .rejections
                    .iter()
                    .map(|reason| reason.as_str())
                    .collect::<Vec<_>>();
                log_capability_resolution(
                    &log_service,
                    CapabilityResolutionLog {
                        execution_id: &execution_id,
                        session_id: &session_id,
                        agent_id: &request.child_agent_id,
                        origin: capability_assignment_origin(delegation_mode),
                        requested_skills: dynamic_skill_resolution
                            .as_ref()
                            .map_or(&[], |resolution| resolution.effective.as_slice()),
                        requested_mcps: &resolution.canonical_requested_ids,
                        effective_skills: dynamic_skill_resolution
                            .as_ref()
                            .map_or(&[], |resolution| resolution.effective.as_slice()),
                        effective_mcps: &resolution.effective_ids,
                        unresolved_skill_count: dynamic_skill_resolution
                            .as_ref()
                            .map_or(assignment.skills.len(), |resolution| {
                                resolution.unresolved_count
                            }),
                        unresolved_count: resolution.rejections.len(),
                        rejection_codes: &rejection_codes,
                    },
                );
            }
            Err(_) => {
                // Fail closed: a catalog/configuration read failure leaves no
                // dynamic MCPs mounted and does not fall back to static ones.
                agent.mcps.clear();
                log_capability_resolution(
                    &log_service,
                    CapabilityResolutionLog {
                        execution_id: &execution_id,
                        session_id: &session_id,
                        agent_id: &request.child_agent_id,
                        origin: "dynamic_resolution_unavailable",
                        requested_skills: dynamic_skill_resolution
                            .as_ref()
                            .map_or(&[], |resolution| resolution.effective.as_slice()),
                        requested_mcps: &known_requested_mcps,
                        effective_skills: dynamic_skill_resolution
                            .as_ref()
                            .map_or(&[], |resolution| resolution.effective.as_slice()),
                        effective_mcps: &[],
                        unresolved_skill_count: dynamic_skill_resolution
                            .as_ref()
                            .map_or(assignment.skills.len(), |resolution| {
                                resolution.unresolved_count
                            }),
                        unresolved_count: assignment.mcps.len(),
                        rejection_codes: &[],
                    },
                );
            }
        }
    } else if !has_dynamic_assignment {
        log_capability_resolution(
            &log_service,
            CapabilityResolutionLog {
                execution_id: &execution_id,
                session_id: &session_id,
                agent_id: &request.child_agent_id,
                origin: "legacy_fallback",
                requested_skills: &[],
                requested_mcps: &[],
                effective_skills: &[],
                effective_mcps: &agent.mcps,
                unresolved_skill_count: 0,
                unresolved_count: 0,
                rejection_codes: &[],
            },
        );
    } else {
        // A present assignment that fails target validation is still dynamic.
        // Do not silently revive static MCPs or legacy skill hints.
        agent.mcps.clear();
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
    let available_agents = collect_agents_summary(&agent_service, &paths).await;
    let available_skills = collect_skills_summary(&skill_service).await;

    // Get tool settings
    let settings_service = gateway_services::SettingsService::new(paths.clone());
    let tool_settings = settings_service.get_tool_settings().unwrap_or_default();
    let tool_result_context =
        crate::runner::prompt_safe_tool_result_config(&tool_settings, paths.vault_dir());

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
        // ward-slim P4: the planner's ward surface is lifecycle + lint
        // (validate_template_context grants it read-only post-write lint);
        // every other delegated actor is lifecycle-only.
        .with_ward_audience(ward_audience_for_child(&request.child_agent_id))
        .with_initial_state(
            "execution_id",
            serde_json::Value::String(execution_id.clone()),
        )
        .with_initial_state("app:delegation_mode", delegation_mode.as_state_value())
        .with_mcp_startup_failure_observer(mcp_startup_failure_observer(
            log_service.clone(),
            execution_id.clone(),
            session_id.clone(),
            request.child_agent_id.clone(),
        ));

    if should_register_planner_catalog(&request.child_agent_id, delegation_mode) {
        if let Some(catalog) = request.planning_capability_catalog.as_ref() {
            builder = builder.with_initial_state(
                agent_runtime::tools::PLANNER_CAPABILITY_CATALOG_STATE,
                catalog.clone(),
            );
        }
    }

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
    let goal_adapter_for_recall = goal_adapter.clone();
    if let Some(a) = goal_adapter {
        builder = builder.with_goal_adapter(a);
    }
    if let Some(recall) = memory_recall.clone() {
        builder = builder.with_memory_recall(recall);
    }
    if let Some(peer_messages) = peer_messages {
        builder = builder.with_peer_messages(peer_messages);
    }
    if let Some(service) = a2a_delegation {
        builder = builder.with_a2a_delegation(service);
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
            handle_early_spawn_failure(EarlySpawnFailure {
                request,
                child_conversation_id: &child_conversation_id,
                child_session_id: Some(&child_session_id),
                error: &e.to_string(),
                messages: messages.as_ref(),
                state_service: &state_service,
                log_service: &log_service,
                event_bus: &event_bus,
                delegation_registry: &delegation_registry,
                agent_result_bus: &agent_result_bus,
            })
            .await;
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
        let authorization = crate::invoke::unified_recall_adapter::recall_authorization_context(
            recall,
            request.child_agent_id.clone(),
            "delegated_executor",
            &request.session_id,
            ward_id,
        );
        if let Some(authorization) = authorization {
            match crate::invoke::unified_recall_adapter::automatic_unified_recall(
                recall.clone(),
                goal_adapter_for_recall,
                authorization,
                &request.task,
                10,
            )
            .await
            {
                Ok(response) if !response.results.is_empty() => {
                    let formatted = crate::recall::format_unified_recall_response_with_options(
                        &response,
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
                            count = response.count,
                            "Primed subagent with unified recalled context"
                        );
                        vec![ChatMessage::system(formatted)]
                    }
                }
                Ok(_) => Vec::new(),
                Err(e) => {
                    tracing::warn!(
                        agent = %request.child_agent_id,
                        reason = ?e.code,
                        "Delegation recall failed, proceeding without priming"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
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

    // Spawn the execution task. Engine construction is unconditional and its
    // config is always resolved today, but if the choke ever fails the child
    // must go through the same early-failure cleanup as the builder above —
    // never leak a RUNNING child row, a registered handle, or a hung parent.
    let executor = match build_execution_engine(executor) {
        Ok(engine) => engine,
        Err(error) => {
            tracing::error!(
                child = %request.child_agent_id,
                %error,
                "Delegated engine construction failed"
            );
            handle_early_spawn_failure(EarlySpawnFailure {
                request,
                child_conversation_id: &child_conversation_id,
                child_session_id: Some(&child_session_id),
                error: &error.to_string(),
                messages: messages.as_ref(),
                state_service: &state_service,
                log_service: &log_service,
                event_bus: &event_bus,
                delegation_registry: &delegation_registry,
                agent_result_bus: &agent_result_bus,
            })
            .await;
            return Err(error);
        }
    };
    spawn_execution_task(SpawnContext {
        executor,
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

const MAX_DYNAMIC_SKILLS: usize = 25;

struct DynamicSkillResolution {
    effective: Vec<String>,
    unresolved_count: usize,
}

/// Resolve planner/root-provided skill recommendations against the live skill
/// service. The child sees only canonical, deduplicated names and never raw
/// rejected model values.
async fn resolve_dynamic_skills(
    skill_service: &SkillService,
    requested: &[String],
) -> Option<DynamicSkillResolution> {
    let available = skill_service.list().await.ok()?;
    let known = available
        .into_iter()
        .map(|skill| skill.name)
        .collect::<HashSet<_>>();
    let mut effective = Vec::new();
    let mut seen = HashSet::new();
    let mut unresolved_count = requested.len().saturating_sub(MAX_DYNAMIC_SKILLS);

    for skill in requested.iter().take(MAX_DYNAMIC_SKILLS) {
        if known.contains(skill) && seen.insert(skill.clone()) {
            effective.push(skill.clone());
        } else if !known.contains(skill) {
            unresolved_count += 1;
        }
    }

    Some(DynamicSkillResolution {
        effective,
        unresolved_count,
    })
}

async fn dynamic_target_exists(
    agent_service: &AgentService,
    vault_dir: &Path,
    agent_id: &str,
) -> bool {
    if let Some(ward_id) = agent_id.strip_prefix("ward:") {
        if ward_id.is_empty() || ward_id.contains(['/', '\\']) {
            return false;
        }
        let mut components = Path::new(ward_id).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return false;
        }
        return std::fs::symlink_metadata(vault_dir.join("wards").join(ward_id))
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    }

    agent_service.get(agent_id).await.is_ok()
}

async fn validate_dynamic_assignment_target(
    agent_service: &AgentService,
    vault_dir: &Path,
    child_agent_id: &str,
    assignment: Option<&agent_primitives::event::AgentCapabilityAssignment>,
) -> bool {
    match assignment {
        None => true,
        Some(assignment) => {
            assignment.agent_id == child_agent_id
                && dynamic_target_exists(agent_service, vault_dir, child_agent_id).await
        }
    }
}

/// A step-executor delegation is the root's execution of a planner-written
/// step briefing. Keep the provenance host-derived from the validated posture
/// rather than a model-supplied metadata field.
fn capability_assignment_origin(mode: DelegationMode) -> &'static str {
    if mode == DelegationMode::StepExecutor {
        "planner"
    } else {
        "dynamic"
    }
}

/// Ward-tool action audience for a delegated child (ward-slim P4):
/// the planner keeps read-only post-write `lint`; every other delegated
/// actor is lifecycle-only — matching what `validate_template_context`
/// permits.
fn ward_audience_for_child(child_agent_id: &str) -> agent_tools::WardAudience {
    if child_agent_id == "planner-agent" {
        agent_tools::WardAudience::Planner
    } else {
        agent_tools::WardAudience::Subagent
    }
}

fn should_register_planner_catalog(child_agent_id: &str, mode: DelegationMode) -> bool {
    child_agent_id == "planner-agent"
        || (child_agent_id.starts_with("ward:") && mode == DelegationMode::WardBackedBuild)
}

fn catalog_mcp_ids(catalog: Option<&serde_json::Value>) -> Vec<String> {
    catalog
        .and_then(|catalog| catalog.get("mcps"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// Persist only canonical accepted IDs and aggregate closed rejection codes.
/// Raw assignments, config errors, URLs, and secret-bearing MCP config are
/// deliberately excluded from this execution-log record.
struct CapabilityResolutionLog<'a> {
    execution_id: &'a str,
    session_id: &'a str,
    agent_id: &'a str,
    origin: &'a str,
    requested_skills: &'a [String],
    requested_mcps: &'a [String],
    effective_skills: &'a [String],
    effective_mcps: &'a [String],
    unresolved_skill_count: usize,
    unresolved_count: usize,
    rejection_codes: &'a [&'a str],
}

fn log_capability_resolution(
    log_service: &LogService<DatabaseManager>,
    resolution: CapabilityResolutionLog<'_>,
) {
    let metadata = capability_resolution_metadata(&resolution);
    let entry = ExecutionLog::new(
        resolution.execution_id,
        resolution.session_id,
        resolution.agent_id,
        LogLevel::Info,
        LogCategory::Intent,
        "Resolved execution capabilities",
    )
    .with_metadata(metadata);
    if log_service.log(entry).is_err() {
        tracing::debug!(
            agent_id = resolution.agent_id,
            "Failed to persist capability resolution audit event"
        );
    }
}

fn capability_resolution_metadata(resolution: &CapabilityResolutionLog<'_>) -> serde_json::Value {
    serde_json::json!({
        "origin": resolution.origin,
        "requested_skills": resolution.requested_skills,
        "requested_mcps": resolution.requested_mcps,
        "effective_skills": resolution.effective_skills,
        "effective_mcps": resolution.effective_mcps,
        "unresolved_skill_count": resolution.unresolved_skill_count,
        "unresolved_count": resolution.unresolved_count,
        "rejection_codes": resolution.rejection_codes,
    })
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
    distiller: Option<Arc<distillation::SessionDistiller>>,
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
    let parent_conversation_id = request.parent_conversation_id.clone();
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
        let mut child_engine_state: Option<serde_json::Value> = None;
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
                // Private checkpoint snapshot for the child's turn boundary.
                agent_runtime::StreamEvent::ContextState { state, .. } => {
                    child_engine_state = Some(state.clone());
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

        // Confirm queued rows before the checkpoint lists them as represented
        // outputs; also keeps the final child row visible before the result
        // bus can race the periodic flush.
        batch_writer.flush().await;
        let child_represented_ids = batch_writer.written_message_ids();

        // Turn-boundary checkpoint — the child's context state plus its
        // private snapshot and recovery cursor beside it.
        crate::runner::recovery::write_turn_checkpoint(crate::runner::recovery::TurnCheckpoint {
            checkpoints: &checkpoints,
            state_service: &state_service,
            execution_id: &execution_id,
            session_id: &child_session_id,
            llm_turn: handle.current_iteration(),
            response: &accumulated_response,
            engine_state: child_engine_state.as_ref(),
            input_cursor: 0, // child input is in-memory, not durable rows
            represented_output_ids: &child_represented_ids,
        });

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
                    parent_agent_id: &parent_agent,
                    parent_execution_id: &parent_execution_id,
                    parent_conversation_id: &parent_conversation_id,
                    error: &crash_report,
                    allow_parent_continuation: true,
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

/// Inputs for an early spawn failure. The child executor may not exist yet,
/// but the synchronous delegation handler already incremented the parent's
/// pending count and exposed the child execution id.
struct EarlySpawnFailure<'a> {
    request: &'a DelegationRequest,
    child_conversation_id: &'a str,
    child_session_id: Option<&'a str>,
    error: &'a str,
    messages: &'a dyn zbot_conversation::MessageStore,
    state_service: &'a StateService<DatabaseManager>,
    log_service: &'a LogService<DatabaseManager>,
    event_bus: &'a EventBus,
    delegation_registry: &'a DelegationRegistry,
    agent_result_bus: &'a AgentResultBus,
}

/// Route pre-executor failures through the same observable completion path as
/// failures after execution starts. This keeps parent callbacks, `wait_agent`,
/// pending-delegation bookkeeping, and continuation wakeups consistent.
async fn handle_early_spawn_failure(ctx: EarlySpawnFailure<'_>) {
    // Finish child-session state before decrementing the parent pending count;
    // the latter can publish `SessionContinuationReady`, whose consumer must
    // never observe a failed child still marked running.
    if let Some(child_session_id) = ctx.child_session_id {
        if let Err(error) = ctx.state_service.crash_session(child_session_id) {
            tracing::warn!(
                child_session_id = %child_session_id,
                error = %error,
                "Failed to mark child session crashed after spawn failure"
            );
        }
    }

    ctx.agent_result_bus.reject(
        &ctx.request.child_execution_id,
        AgentWaitError::Crashed {
            error: ctx.error.to_string(),
        },
    );
    handle_execution_failure(HandleExecutionFailure {
        messages: ctx.messages,
        state_service: ctx.state_service,
        log_service: ctx.log_service,
        event_bus: ctx.event_bus,
        delegation_registry: ctx.delegation_registry,
        execution_id: &ctx.request.child_execution_id,
        session_id: &ctx.request.session_id,
        agent_id: &ctx.request.child_agent_id,
        conv_id: ctx.child_conversation_id,
        parent_agent_id: &ctx.request.parent_agent_id,
        parent_execution_id: &ctx.request.parent_execution_id,
        parent_conversation_id: &ctx.request.parent_conversation_id,
        error: ctx.error,
        // Ward entry consumes the invocation-local planning gate before the
        // child executor is built. If planner construction fails, waking a
        // continuation would build a fresh ungated root executor. Record the
        // failure and finish bookkeeping, but require a later user turn to
        // retry intent/ward selection under a new gate.
        allow_parent_continuation: ctx.request.child_agent_id != "planner-agent",
    })
    .await;
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
    parent_agent_id: &'a str,
    parent_execution_id: &'a str,
    parent_conversation_id: &'a str,
    error: &'a str,
    allow_parent_continuation: bool,
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
        parent_agent_id,
        parent_execution_id,
        parent_conversation_id,
        error,
        allow_parent_continuation,
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
    let delegation_completion = state_service.complete_delegation(session_id);
    if !allow_parent_continuation {
        if let Err(error) = state_service.clear_continuation(session_id) {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to clear continuation after planner startup failure"
            );
        }
        crash_execution(CrashExecution {
            state_service,
            log_service,
            event_bus,
            execution_id: parent_execution_id,
            session_id,
            agent_id: parent_agent_id,
            conversation_id: parent_conversation_id,
            error: "planner_startup_failed",
            crash_session: true,
        })
        .await;
        tracing::warn!(
            session_id = %session_id,
            agent_id = %agent_id,
            "Planner startup failed; parent continuation suppressed to keep planning gate closed"
        );
    } else {
        match delegation_completion {
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
                        root_execution_id = %root_exec.id,
                        "All delegations complete (including failed), continuation ready"
                    );
                }
            }
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to complete delegation tracking: {}", e),
        }
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

/// Bind a ward-agent delegation to the parent session exactly once. The
/// returned parent ward remains authoritative when an already-bound session
/// dispatches a different ward agent; that child still receives its explicit
/// target through [`effective_ward_id`].
fn bind_parent_ward_for_delegation(
    state_service: &StateService<DatabaseManager>,
    request: &DelegationRequest,
) -> Result<(Option<String>, Option<String>), String> {
    let Some(ward_id) = request.child_agent_id.strip_prefix("ward:") else {
        return Ok((
            state_service
                .get_session(&request.session_id)?
                .and_then(|session| session.ward_id),
            None,
        ));
    };

    match state_service.claim_session_ward_if_unset(&request.session_id, ward_id)? {
        SessionWardClaim::Claimed(ward_id) => Ok((Some(ward_id.clone()), Some(ward_id))),
        SessionWardClaim::Existing(ward_id) => Ok((Some(ward_id), None)),
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
        RuntimeActorKind::RemotePeer => ContextActorKind::RemotePeer,
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

    /// Build a minimal ExecCtx from the individual services tests construct.
    #[allow(clippy::too_many_arguments)]
    fn test_exec_ctx(
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
        delegation_tx: tokio::sync::mpsc::UnboundedSender<DelegationRequest>,
        log_service: Arc<LogService<DatabaseManager>>,
        state_service: Arc<StateService<DatabaseManager>>,
        memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
        distiller: Option<Arc<distillation::SessionDistiller>>,
        memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
        peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
        a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
        rate_limiters: Arc<
            std::sync::RwLock<
                std::collections::HashMap<String, Arc<agent_runtime::ProviderRateLimiter>>,
            >,
        >,
        _kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
        _ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
        _goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
        steering_registry: Arc<agent_runtime::SteeringRegistry>,
        agent_result_bus: Arc<AgentResultBus>,
    ) -> Arc<crate::runner::exec_ctx::ExecCtx> {
        Arc::new(crate::runner::exec_ctx::ExecCtx {
            event_bus,
            agent_service,
            provider_service,
            mcp_service,
            skill_service,
            paths,
            log_service,
            state_service: state_service.clone(),
            messages,
            session_meta,
            checkpoints,
            control: crate::runner::session_control::SessionControl {
                handles,
                delegation_registry,
                state_service,
            },
            delegation_tx,
            delegation_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            connector_registry: None,
            bridge_registry: None,
            bridge_outbox: None,
            memory_store,
            embedding_client: None,
            distiller,
            handoff_writer: None,
            memory_recall,
            peer_messages,
            a2a_delegation,
            procedure_store: None,
            ward_usage: Arc::new(gateway_services::WardUsage::new(std::path::PathBuf::from(
                "/tmp/test-wards",
            ))),
            model_registry: Arc::new(arc_swap::ArcSwapOption::from(None)),
            rate_limiters,
            integrations: crate::runner::integrations::SharedIntegrations::default(),
            steering_registry,
            agent_result_bus,
            ward_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
        })
    }

    struct AppendObserverMessageStore {
        inner: Arc<dyn zbot_conversation::MessageStore>,
        on_append: Arc<dyn Fn(&zbot_conversation::Message) + Send + Sync>,
    }

    impl zbot_conversation::MessageStore for AppendObserverMessageStore {
        fn append(&self, message: &zbot_conversation::Message) -> anyhow::Result<()> {
            (self.on_append)(message);
            self.inner.append(message)
        }

        fn replay(
            &self,
            session_id: &str,
            after_seq: Option<i64>,
            limit: usize,
        ) -> anyhow::Result<Vec<zbot_conversation::Message>> {
            self.inner.replay(session_id, after_seq, limit)
        }

        fn tool_sequence_for_session(&self, session_id: &str) -> anyhow::Result<Vec<String>> {
            self.inner.tool_sequence_for_session(session_id)
        }
    }

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
    fn ward_delegation_claims_parent_and_propagates_its_workspace_to_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault dirs");
        let state = StateService::new(Arc::new(DatabaseManager::new(paths).expect("state db")));
        let (parent, execution) = state.create_session("root").expect("parent session");
        let request = DelegationRequest {
            parent_agent_id: "root".to_string(),
            session_id: parent.id.clone(),
            parent_execution_id: execution.id,
            parent_conversation_id: parent.id.clone(),
            child_agent_id: "ward:software-delivery-optimization".to_string(),
            child_execution_id: "exec-ward-child".to_string(),
            task: "work in the ward".to_string(),
            mode: None,
            context: None,
            max_iterations: None,
            output_schema: None,
            skills: Vec::new(),
            capability_assignment: None,
            planning_capability_catalog: None,
            complexity: None,
            parallel: false,
        };

        let (parent_ward, claimed) =
            bind_parent_ward_for_delegation(&state, &request).expect("bind ward");
        let child_ward = effective_ward_id(&request.child_agent_id, parent_ward);

        assert_eq!(claimed.as_deref(), Some("software-delivery-optimization"));
        assert_eq!(
            child_ward.as_deref(),
            Some("software-delivery-optimization")
        );
        assert_eq!(
            state
                .get_session(&parent.id)
                .expect("read parent")
                .and_then(|session| session.ward_id)
                .as_deref(),
            Some("software-delivery-optimization")
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

    #[tokio::test]
    async fn dynamic_skill_resolution_accepts_only_live_unique_skill_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_dir = dir.path().join("research");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: research\ndescription: Find evidence\n---\n",
        )
        .expect("skill file");
        let service = SkillService::with_roots(vec![dir.path().to_path_buf()]);

        let resolution = resolve_dynamic_skills(
            &service,
            &[
                "research".to_string(),
                "missing".to_string(),
                "research".to_string(),
            ],
        )
        .await
        .expect("skill catalog reads");
        assert_eq!(resolution.effective, vec!["research"]);
        assert_eq!(resolution.unresolved_count, 1);
    }

    #[test]
    fn planned_step_assignments_are_audited_as_planner_origin() {
        assert_eq!(
            capability_assignment_origin(DelegationMode::StepExecutor),
            "planner"
        );
        assert_eq!(
            capability_assignment_origin(DelegationMode::WardBackedBuild),
            "dynamic"
        );
    }

    /// ward-slim P4: the spawned planner keeps read-only lint; every other
    /// delegated child is lifecycle-only. Pins the spawn wiring — deleting
    /// the audience line at the builder fails this test.
    #[test]
    fn ward_audience_for_delegated_children() {
        assert_eq!(
            ward_audience_for_child("planner-agent"),
            agent_tools::WardAudience::Planner
        );
        for other in ["builder-agent", "ward:financial-analysis", "web-researcher"] {
            assert_eq!(
                ward_audience_for_child(other),
                agent_tools::WardAudience::Subagent,
                "{other} must be lifecycle-only"
            );
        }
    }

    #[test]
    fn planner_catalog_registration_requires_a_planning_transition() {
        assert!(should_register_planner_catalog(
            "planner-agent",
            DelegationMode::WardBackedBuild
        ));
        assert!(should_register_planner_catalog(
            "ward:graphics",
            DelegationMode::WardBackedBuild
        ));
        assert!(!should_register_planner_catalog(
            "ward:graphics",
            DelegationMode::StepExecutor
        ));
        assert!(!should_register_planner_catalog(
            "builder-agent",
            DelegationMode::StepExecutor
        ));
    }

    #[tokio::test]
    async fn invalid_dynamic_target_is_rejected_without_creating_a_specialist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        std::fs::create_dir_all(&agents_dir).expect("agents dir");
        let service = AgentService::new(agents_dir.clone());
        let assignment = agent_primitives::event::AgentCapabilityAssignment {
            agent_id: "ghost-agent".to_string(),
            skills: vec![],
            mcps: vec!["blender-mcp".to_string()],
        };

        let accepted = validate_dynamic_assignment_target(
            &service,
            dir.path(),
            "different-agent",
            Some(&assignment),
        )
        .await;

        assert!(!accepted);
        assert!(!agents_dir.join("ghost-agent").exists());
        assert!(!agents_dir.join("different-agent").exists());
    }

    #[tokio::test]
    async fn spawn_failures_complete_parent_and_child_lifecycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault dirs");
        let db = Arc::new(DatabaseManager::new(paths.clone()).expect("state db"));
        let state_service = Arc::new(StateService::new(db.clone()));
        let log_service = Arc::new(LogService::new(db));
        let (session, root_execution) = state_service.create_session("root").expect("root session");
        let child_execution_id = "exec-invalid-dynamic-target";
        state_service
            .create_delegated_execution_with_id(
                child_execution_id,
                &session.id,
                "different-agent",
                &root_execution.id,
                execution_state::DelegationType::Sequential,
                "build a scene",
            )
            .expect("pre-created child execution");
        state_service
            .register_delegation(&session.id)
            .expect("pending delegation");
        state_service
            .request_continuation(&session.id)
            .expect("continuation request");

        let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db())
            .expect("conversation pool");
        let messages: Arc<dyn zbot_conversation::MessageStore> =
            Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
        let session_meta: Arc<dyn zbot_conversation::SessionMetaStore> =
            Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool.clone()));
        let checkpoints: Arc<dyn zbot_conversation::CheckpointStore> =
            Arc::new(zbot_conversation::SqliteCheckpointStore::new(pool));
        let (delegation_tx, _delegation_rx) = mpsc::unbounded_channel();
        let assignment = agent_primitives::event::AgentCapabilityAssignment {
            agent_id: "ghost-agent".to_string(),
            skills: vec![],
            mcps: vec!["blender-mcp".to_string()],
        };
        let request = DelegationRequest {
            parent_agent_id: "root".to_string(),
            session_id: session.id.clone(),
            parent_execution_id: root_execution.id.clone(),
            parent_conversation_id: "root-conversation".to_string(),
            child_agent_id: "different-agent".to_string(),
            child_execution_id: child_execution_id.to_string(),
            task: "build a scene".to_string(),
            mode: Some(DelegationMode::StepExecutor),
            context: None,
            max_iterations: None,
            output_schema: None,
            skills: vec![],
            capability_assignment: Some(assignment),
            planning_capability_catalog: Some(serde_json::json!({
                "skills": [],
                "mcps": [{"id": "blender-mcp"}]
            })),
            complexity: None,
            parallel: false,
        };

        let event_bus = Arc::new(EventBus::new());
        let agent_service = Arc::new(AgentService::new(paths.agents_dir()));
        let provider_service = Arc::new(ProviderService::new(paths.clone()));
        let mcp_service = Arc::new(McpService::new(paths.clone()));
        let skill_service = Arc::new(SkillService::new(paths.skills_dir()));
        let handles = Arc::new(RwLock::new(HashMap::new()));
        let delegation_registry = Arc::new(DelegationRegistry::new());
        let rate_limiters = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let steering_registry = Arc::new(agent_runtime::SteeringRegistry::new());
        let agent_result_bus = Arc::new(AgentResultBus::new());

        let result_ctx = test_exec_ctx(
            event_bus.clone(),
            agent_service.clone(),
            provider_service.clone(),
            mcp_service.clone(),
            skill_service.clone(),
            paths.clone(),
            messages.clone(),
            session_meta.clone(),
            checkpoints.clone(),
            handles.clone(),
            delegation_registry.clone(),
            delegation_tx.clone(),
            log_service.clone(),
            state_service.clone(),
            None,
            None,
            None,
            None,
            None,
            rate_limiters.clone(),
            None,
            None,
            None,
            steering_registry.clone(),
            agent_result_bus.clone(),
        );
        let result = spawn_delegated_agent(&result_ctx, &request, None).await;

        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Dynamic capability assignment target rejected"));
        let session_after = state_service
            .get_session(&session.id)
            .expect("read parent session")
            .expect("parent session remains");
        assert_eq!(session_after.pending_delegations, 0);
        assert!(session_after.continuation_needed);
        let child_after = state_service
            .get_execution(child_execution_id)
            .expect("read child execution")
            .expect("child execution remains auditable");
        assert_eq!(
            child_after.status,
            execution_state::ExecutionStatus::Crashed
        );
        let callback = messages
            .replay(&session.id, None, 10)
            .expect("parent callback replay");
        assert_eq!(callback.len(), 1);
        assert!(callback[0]
            .content
            .contains("Dynamic capability assignment target rejected"));
        assert!(!paths.agents_dir().join("ghost-agent").exists());
        assert!(!paths.agents_dir().join("different-agent").exists());

        // A target can pass assignment validation and still fail later while
        // loading its provider. That post-session failure must crash the child
        // session before the parent continuation is released.
        let broken_agent_dir = paths.agents_dir().join("broken-agent");
        std::fs::create_dir_all(&broken_agent_dir).expect("broken agent dir");
        std::fs::write(
            broken_agent_dir.join("config.yaml"),
            "name: broken-agent\n\
             displayName: Broken Agent\n\
             description: Missing provider fixture\n\
             providerId: missing-provider\n\
             model: missing-model\n\
             temperature: 0.1\n\
             maxTokens: 8192\n\
             thinkingEnabled: false\n\
             voiceRecordingEnabled: false\n\
             skills: []\n\
             mcps: []\n",
        )
        .expect("broken agent config");
        std::fs::write(broken_agent_dir.join("AGENTS.md"), "test fixture\n")
            .expect("broken agent instructions");
        let broken_execution_id = "exec-broken-agent-loader";
        state_service
            .create_delegated_execution_with_id(
                broken_execution_id,
                &session.id,
                "broken-agent",
                &root_execution.id,
                execution_state::DelegationType::Sequential,
                "load missing provider",
            )
            .expect("pre-created loader-failure execution");
        state_service
            .register_delegation(&session.id)
            .expect("second pending delegation");
        let broken_request = DelegationRequest {
            child_agent_id: "broken-agent".to_string(),
            child_execution_id: broken_execution_id.to_string(),
            task: "load missing provider".to_string(),
            capability_assignment: Some(agent_primitives::event::AgentCapabilityAssignment {
                agent_id: "broken-agent".to_string(),
                skills: vec![],
                mcps: vec![],
            }),
            planning_capability_catalog: Some(serde_json::json!({
                "skills": [],
                "mcps": []
            })),
            ..request.clone()
        };

        let callback_observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_observed_for_append = callback_observed.clone();
        let state_for_append = state_service.clone();
        let ordering_messages: Arc<dyn zbot_conversation::MessageStore> =
            Arc::new(AppendObserverMessageStore {
                inner: messages.clone(),
                on_append: Arc::new(move |message| {
                    assert_eq!(message.role, "system");
                    let broken_execution = state_for_append
                        .get_execution(broken_execution_id)
                        .expect("read broken execution at parent callback")
                        .expect("broken execution exists at parent callback");
                    let child_session_id = broken_execution
                        .child_session_id
                        .expect("loader failure created a child session");
                    let child_session = state_for_append
                        .get_session(&child_session_id)
                        .expect("read failed child at parent callback")
                        .expect("failed child exists at parent callback");
                    assert_eq!(
                        child_session.status,
                        execution_state::SessionStatus::Crashed,
                        "parent callback exposed a failed child before its session crashed"
                    );
                    callback_observed_for_append.store(true, std::sync::atomic::Ordering::SeqCst);
                }),
            });

        let error_ctx = test_exec_ctx(
            event_bus.clone(),
            agent_service,
            provider_service,
            mcp_service,
            skill_service,
            paths.clone(),
            ordering_messages,
            session_meta,
            checkpoints,
            handles,
            delegation_registry.clone(),
            delegation_tx,
            log_service.clone(),
            state_service.clone(),
            None,
            None,
            None,
            None,
            None,
            rate_limiters,
            None,
            None,
            None,
            steering_registry.clone(),
            agent_result_bus.clone(),
        );
        let error = spawn_delegated_agent(&error_ctx, &broken_request, None)
            .await
            .expect_err("missing provider must fail spawn");
        assert!(
            error
                .to_string()
                .to_string()
                .to_ascii_lowercase()
                .contains("provider"),
            "unexpected loader failure: {error}"
        );
        assert!(
            callback_observed.load(std::sync::atomic::Ordering::SeqCst),
            "loader failure must synchronously persist a parent callback"
        );

        let parent_after_loader_failure = state_service
            .get_session(&session.id)
            .expect("read parent after loader failure")
            .expect("parent remains after loader failure");
        assert_eq!(parent_after_loader_failure.pending_delegations, 0);
        let broken_execution = state_service
            .get_execution(broken_execution_id)
            .expect("read broken execution")
            .expect("broken execution remains auditable");
        assert_eq!(
            broken_execution.status,
            execution_state::ExecutionStatus::Crashed
        );
        let child_session_id = broken_execution
            .child_session_id
            .expect("loader failure created a child session");
        let child_session = state_service
            .get_session(&child_session_id)
            .expect("read failed child session")
            .expect("failed child session remains auditable");
        assert_eq!(
            child_session.status,
            execution_state::SessionStatus::Crashed
        );
        let callbacks = messages
            .replay(&session.id, None, 10)
            .expect("both parent callbacks replay");
        assert_eq!(callbacks.len(), 2);

        // Reproduce the narrow race after Ward entry has transitioned the
        // planning gate but before the planner executor finishes building.
        // A missing/corrupt template at this point must report the failed
        // child without waking an ungated root continuation.
        let planner_execution_id = "exec-planner-template-race";
        state_service
            .create_delegated_execution_with_id(
                planner_execution_id,
                &session.id,
                "planner-agent",
                &root_execution.id,
                execution_state::DelegationType::Sequential,
                "persist the ward plan",
            )
            .expect("pre-created planner execution");
        state_service
            .register_delegation(&session.id)
            .expect("planner pending delegation");
        let planner_request = DelegationRequest {
            child_agent_id: "planner-agent".to_string(),
            child_execution_id: planner_execution_id.to_string(),
            task: "persist the ward plan".to_string(),
            capability_assignment: None,
            ..request
        };
        let mut planner_lifecycle_events = event_bus.subscribe_all();

        handle_early_spawn_failure(EarlySpawnFailure {
            request: &planner_request,
            child_conversation_id: "planner-template-race-conversation",
            child_session_id: None,
            error: "planner_template_unavailable",
            messages: messages.as_ref(),
            state_service: &state_service,
            log_service: &log_service,
            event_bus: &event_bus,
            delegation_registry: &delegation_registry,
            agent_result_bus: &agent_result_bus,
        })
        .await;

        let parent_after_planner_failure = state_service
            .get_session(&session.id)
            .expect("read parent after planner failure")
            .expect("parent remains after planner failure");
        assert_eq!(parent_after_planner_failure.pending_delegations, 0);
        assert_eq!(
            parent_after_planner_failure.status,
            execution_state::SessionStatus::Crashed,
            "planner startup fail-close must terminate the paused root session"
        );
        assert!(
            !parent_after_planner_failure.continuation_needed,
            "failed planner construction must not release an ungated continuation"
        );
        let root_after_planner_failure = state_service
            .get_execution(&root_execution.id)
            .expect("read root after planner failure")
            .expect("root remains auditable after planner failure");
        assert_eq!(
            root_after_planner_failure.status,
            execution_state::ExecutionStatus::Crashed,
            "suppressed continuation must not strand a running root execution"
        );
        let callbacks = messages
            .replay(&session.id, None, 10)
            .expect("planner failure callback replay");
        assert_eq!(callbacks.len(), 3);
        assert!(callbacks[2]
            .content
            .contains("planner_template_unavailable"));
        let mut saw_bounded_root_error = false;
        while let Ok(event) = planner_lifecycle_events.try_recv() {
            if matches!(
                event,
                GatewayEvent::Error {
                    ref execution_id,
                    ref message,
                    ..
                } if execution_id.as_deref() == Some(root_execution.id.as_str())
                    && message == "planner_startup_failed"
            ) {
                saw_bounded_root_error = true;
            }
        }
        assert!(
            saw_bounded_root_error,
            "planner startup fail-close must publish the bounded root lifecycle error"
        );
    }

    #[test]
    fn capability_log_metadata_separates_requested_from_effective_ids() {
        let requested_skills = vec!["modeling".to_string()];
        let requested_mcps = vec!["blender-mcp".to_string()];
        let effective_mcps = Vec::new();
        let metadata = capability_resolution_metadata(&CapabilityResolutionLog {
            execution_id: "exec-1",
            session_id: "session-1",
            agent_id: "builder-agent",
            origin: "planner",
            requested_skills: &requested_skills,
            requested_mcps: &requested_mcps,
            effective_skills: &requested_skills,
            effective_mcps: &effective_mcps,
            unresolved_skill_count: 0,
            unresolved_count: 1,
            rejection_codes: &["deleted"],
        });

        assert_eq!(
            metadata["requested_mcps"],
            serde_json::json!(["blender-mcp"])
        );
        assert_eq!(metadata["effective_mcps"], serde_json::json!([]));
        assert_eq!(metadata["rejection_codes"], serde_json::json!(["deleted"]));
    }

    #[test]
    fn persisted_capability_log_uses_canonical_ids_and_closed_rejection_codes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(dir.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault dirs");
        let mcp_service = McpService::new(paths.clone());
        mcp_service
            .save(&[
                agent_runtime::McpServerConfig::Stdio {
                    id: Some("ready".to_string()),
                    name: "Ready".to_string(),
                    description: "ready server".to_string(),
                    command: "sensitive-ready-command".to_string(),
                    args: vec![],
                    env: None,
                    enabled: true,
                    validated: None,
                },
                agent_runtime::McpServerConfig::Stdio {
                    id: Some("disabled".to_string()),
                    name: "Disabled".to_string(),
                    description: "disabled server".to_string(),
                    command: "sensitive-disabled-command".to_string(),
                    args: vec![],
                    env: None,
                    enabled: false,
                    validated: None,
                },
                agent_runtime::McpServerConfig::StreamableHttp {
                    id: Some("oauth".to_string()),
                    name: "OAuth".to_string(),
                    description: "oauth server".to_string(),
                    url: "https://secret.internal.example/mcp".to_string(),
                    headers: None,
                    auth: Some(agent_runtime::McpAuthConfig {
                        auth_type: agent_runtime::McpAuthType::OAuth2,
                        client_id: None,
                        scopes: vec![],
                    }),
                    enabled: true,
                    validated: None,
                },
            ])
            .expect("save MCP catalog");
        let resolution = mcp_service
            .resolve_dynamic_runtime_ids_with_catalog(
                &[
                    "ready".to_string(),
                    "disabled".to_string(),
                    "oauth".to_string(),
                    "deleted".to_string(),
                    "raw-secret-unknown".to_string(),
                ],
                &[
                    "ready".to_string(),
                    "disabled".to_string(),
                    "oauth".to_string(),
                    "deleted".to_string(),
                ],
            )
            .expect("resolve dynamic MCPs");
        let rejection_codes = resolution
            .rejections
            .iter()
            .map(|reason| reason.as_str())
            .collect::<Vec<_>>();

        let db = Arc::new(DatabaseManager::new(paths).expect("logs db"));
        let log_service = LogService::new(db);
        log_capability_resolution(
            &log_service,
            CapabilityResolutionLog {
                execution_id: "exec-capability-audit",
                session_id: "session-capability-audit",
                agent_id: "builder-agent",
                origin: "planner",
                requested_skills: &[],
                requested_mcps: &resolution.canonical_requested_ids,
                effective_skills: &[],
                effective_mcps: &resolution.effective_ids,
                unresolved_skill_count: 0,
                unresolved_count: resolution.rejections.len(),
                rejection_codes: &rejection_codes,
            },
        );

        let detail = log_service
            .get_session_detail("exec-capability-audit")
            .expect("query capability audit")
            .expect("capability audit exists");
        assert_eq!(detail.logs.len(), 1);
        let metadata = detail.logs[0]
            .metadata
            .as_ref()
            .expect("capability metadata");
        assert_eq!(
            metadata["requested_mcps"],
            serde_json::json!(["ready", "disabled", "oauth", "deleted"])
        );
        assert_eq!(metadata["effective_mcps"], serde_json::json!(["ready"]));
        assert_eq!(
            metadata["rejection_codes"],
            serde_json::json!(["disabled", "oauth_unavailable", "deleted", "unknown_id"])
        );
        let persisted = serde_json::to_string(metadata).expect("metadata JSON");
        assert!(!persisted.contains("raw-secret-unknown"));
        assert!(!persisted.contains("secret.internal.example"));
        assert!(!persisted.contains("sensitive-disabled-command"));
    }
}
