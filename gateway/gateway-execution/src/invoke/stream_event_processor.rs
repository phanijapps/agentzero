//! # Stream Event Processor
//!
//! Processes individual stream events: handles side-effects, converts to gateway events,
//! and extracts response deltas for accumulation.

use agent_runtime::StreamEvent;
use agent_surfaces::{
    is_persistable_surface, ComponentType, SurfaceComponent, WorkSurface, ZBOT_WORK_SURFACE_CATALOG,
};
use execution_state::{SessionPlanInput, SessionPlanSaveOutcome, SessionPlanSnapshot};
use gateway_events::{EventBus, GatewayEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

use super::delegation_handler::handle_delegation;
use super::event_logging::{log_error, log_tool_call, log_tool_result};
use super::response_accumulator::TURN_COMPLETE_MARKER;
use super::stream_context::StreamContext;
use super::token_tracking::handle_token_update;
use super::ward_scaffolding::{collect_ward_setup_for_skill, collect_ward_setups_for_skills};

/// Process a stream event: log it, handle special cases, and return the gateway event.
///
/// Returns the gateway event (if any) and whether the response accumulator should be updated.
/// Returns `None` for the gateway event if it's an internal event that shouldn't be broadcast.
pub fn process_stream_event(
    ctx: &StreamContext,
    event: &StreamEvent,
) -> (Option<GatewayEvent>, Option<String>) {
    handle_artifact_declarations(ctx, event);
    handle_delegation_event(ctx, event);
    handle_side_effects(ctx, event);
    let plan_outcome = persist_current_plan(ctx, event);
    publish_projected_surface(ctx, event, plan_outcome.accepted());

    // Rejected plan updates are never cloned into a gateway event. This keeps
    // oversized model JSON out of a second allocation and off the client bus.
    let gateway_event = if plan_outcome.suppress_gateway_event() {
        None
    } else {
        crate::events::convert_stream_event(
            event.clone(),
            &ctx.agent_id,
            &ctx.conversation_id,
            &ctx.session_id,
            &ctx.execution_id,
        )
    };
    if let Some(event) = gateway_event.as_ref() {
        persist_gateway_surface(&ctx.state_service, event);
    }

    let response_delta = extract_response_delta(&gateway_event);
    (gateway_event, response_delta)
}

/// Project gateway-owned plan data into the initial native work-surface
/// catalog. This uses the `update_plan` tool's structured payload rather than
/// interpreting arbitrary assistant text as UI.
fn publish_projected_surface(
    ctx: &StreamContext,
    event: &StreamEvent,
    accepted_plan: Option<&SessionPlanSnapshot>,
) {
    let Some(snapshot) = accepted_plan else {
        return;
    };
    // A plan belongs to the session, not an individual root/subagent
    // execution. Research replaces surfaces by this stable identifier.
    let surface_id = plan_surface_id(&ctx.session_id);
    let is_update = ctx
        .surface_ids
        .lock()
        .map(|mut ids| !ids.insert(surface_id.clone()))
        .unwrap_or(false);
    let surface = build_plan_surface(surface_id, snapshot);
    if let Some(surface_event) = crate::events::convert_stream_event(
        if is_update {
            StreamEvent::WorkSurfaceUpdated {
                timestamp: event.timestamp(),
                surface,
            }
        } else {
            StreamEvent::WorkSurface {
                timestamp: event.timestamp(),
                surface,
            }
        },
        &ctx.agent_id,
        &ctx.conversation_id,
        &ctx.session_id,
        &ctx.execution_id,
    ) {
        persist_gateway_surface(&ctx.state_service, &surface_event);
        ctx.event_bus.publish_sync(surface_event);
    }
}

/// Persist an already-validated gateway surface event when the live setting is
/// enabled. Failure is supplementary and never blocks event publication.
pub(crate) fn persist_gateway_surface(
    state_service: &execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>,
    event: &GatewayEvent,
) {
    if !state_service.surface_persistence_enabled() {
        return;
    }
    let result = match event {
        GatewayEvent::SurfaceCreated {
            session_id,
            execution_id,
            surface,
        }
        | GatewayEvent::SurfaceUpdated {
            session_id,
            execution_id,
            surface,
        } if is_persistable_surface(surface) => serde_json::to_string(surface)
            .map_err(|error| error.to_string())
            .and_then(|surface_json| {
                state_service.save_session_surface(
                    session_id,
                    execution_id,
                    &surface.surface_id,
                    &surface_json,
                )
            }),
        GatewayEvent::SurfaceDeleted {
            session_id,
            surface_id,
            ..
        } => state_service
            .delete_session_surface(session_id, surface_id)
            .map(|_| ()),
        GatewayEvent::SurfaceCreated {
            session_id,
            surface,
            ..
        }
        | GatewayEvent::SurfaceUpdated {
            session_id,
            surface,
            ..
        } => state_service
            .delete_session_surface(session_id, &surface.surface_id)
            .map(|_| ()),
        _ => return,
    };
    if result.is_err() {
        tracing::warn!(
            event = "surface_persistence_failed",
            "could not update saved work surface"
        );
    }
}

/// Result of plan processing before generic stream-event conversion.
enum PlanUpdateOutcome {
    NotPlan,
    Accepted(SessionPlanSnapshot),
    Rejected,
}

impl PlanUpdateOutcome {
    fn accepted(&self) -> Option<&SessionPlanSnapshot> {
        match self {
            Self::Accepted(snapshot) => Some(snapshot),
            Self::NotPlan | Self::Rejected => None,
        }
    }

    const fn suppress_gateway_event(&self) -> bool {
        matches!(self, Self::Rejected)
    }
}

/// Save a valid plan before publishing its native Research surface. Rejections
/// are expected model-output outcomes and never expose model text in logs.
fn persist_current_plan(ctx: &StreamContext, event: &StreamEvent) -> PlanUpdateOutcome {
    let StreamEvent::ActionPlanUpdate {
        plan,
        explanation,
        timestamp,
    } = event
    else {
        return PlanUpdateOutcome::NotPlan;
    };

    let step_count = plan.as_array().map_or(0, Vec::len).min(1_000);
    if let Err(reason) = SessionPlanInput::preflight_update(plan, explanation.as_deref()) {
        log_plan_rejection(ctx, reason.code(), *timestamp, step_count);
        return PlanUpdateOutcome::Rejected;
    }
    match ctx.state_service.save_session_plan(
        &ctx.session_id,
        &ctx.execution_id,
        plan.clone(),
        explanation.clone(),
        *timestamp,
    ) {
        Ok(SessionPlanSaveOutcome::Accepted(snapshot)) => PlanUpdateOutcome::Accepted(snapshot),
        Ok(SessionPlanSaveOutcome::Rejected(reason)) => {
            log_plan_rejection(ctx, reason.code(), *timestamp, step_count);
            PlanUpdateOutcome::Rejected
        }
        Err(_) => {
            tracing::warn!(
                target: "zbot_plan",
                reason = "persistence_failed",
                session_id = %ctx.session_id,
                execution_id = %ctx.execution_id,
                source_event_timestamp = *timestamp,
                step_count,
                "could not persist current plan update"
            );
            PlanUpdateOutcome::Rejected
        }
    }
}

fn log_plan_rejection(ctx: &StreamContext, reason: &str, timestamp: u64, step_count: usize) {
    tracing::warn!(
        target: "zbot_plan",
        reason,
        session_id = %ctx.session_id,
        execution_id = %ctx.execution_id,
        source_event_timestamp = timestamp,
        step_count,
        "rejected current plan update"
    );
}

fn build_plan_surface(surface_id: String, snapshot: &SessionPlanSnapshot) -> WorkSurface {
    WorkSurface {
        surface_id,
        catalog_id: ZBOT_WORK_SURFACE_CATALOG.to_owned(),
        components: vec![SurfaceComponent {
            id: "plan".to_owned(),
            component_type: ComponentType::PlanChecklist,
            props: BTreeMap::from([
                (
                    "title".to_owned(),
                    serde_json::Value::String("Plan".to_owned()),
                ),
                (
                    "plan_path".to_owned(),
                    serde_json::Value::String("/plan".to_owned()),
                ),
            ]),
        }],
        data: serde_json::json!({ "plan": snapshot.plan }),
    }
}

/// Build the stable session plan surface outside stream-event handling.
/// Lifecycle completion uses this after reconciling a terminal plan state.
pub(crate) fn build_session_plan_surface(
    session_id: &str,
    snapshot: &SessionPlanSnapshot,
) -> WorkSurface {
    build_plan_surface(plan_surface_id(session_id), snapshot)
}

fn plan_surface_id(session_id: &str) -> String {
    format!("plan-{session_id}")
}

/// Broadcast a gateway event synchronously to preserve token ordering.
///
/// Uses `publish_sync` (non-blocking `broadcast::Sender::send`) instead of
/// spawning async tasks, which would destroy insertion order between tokens.
pub fn broadcast_event(event_bus: Arc<EventBus>, event: GatewayEvent) {
    event_bus.publish_sync(event);
}

fn handle_artifact_declarations(ctx: &StreamContext, event: &StreamEvent) {
    if let StreamEvent::ActionRespond { ref artifacts, .. } = event {
        if !artifacts.is_empty() {
            // Fetch ward_id from the session record (persisted by WardChanged events)
            let ward_id = ctx
                .state_service
                .get_session(&ctx.session_id)
                .ok()
                .flatten()
                .and_then(|s| s.ward_id);

            crate::artifacts::process_artifact_declarations(
                artifacts,
                &ctx.session_id,
                &ctx.execution_id,
                &ctx.agent_id,
                ward_id.as_deref(),
                &ctx.vault_dir,
                &ctx.state_service,
            );
        }
    }
}

fn handle_delegation_event(ctx: &StreamContext, event: &StreamEvent) {
    if let StreamEvent::ActionDelegate {
        agent_id: child_agent,
        task,
        context,
        max_iterations,
        output_schema,
        skills,
        capability_assignment,
        planning_capability_catalog,
        complexity,
        mode,
        parallel,
        child_execution_id,
        ..
    } = event
    {
        handle_delegation(
            ctx,
            child_agent,
            task,
            context,
            *max_iterations,
            output_schema,
            skills,
            capability_assignment,
            planning_capability_catalog,
            complexity,
            mode,
            *parallel,
            child_execution_id.as_deref(),
        );
    }
}

fn handle_side_effects(ctx: &StreamContext, event: &StreamEvent) {
    match event {
        StreamEvent::TokenUpdate {
            tokens_in,
            tokens_out,
            ..
        } => {
            handle_token_update(ctx, *tokens_in, *tokens_out);
        }
        StreamEvent::ToolCallStart {
            tool_id,
            tool_name,
            args,
            ..
        } => {
            log_tool_call(ctx, tool_id, tool_name, args);
            trace_tool_call(ctx, tool_id, tool_name, args);
        }
        StreamEvent::ToolResult {
            tool_id,
            result,
            error,
            duration_ms,
            ..
        } => {
            log_tool_result(ctx, tool_id, result, error, *duration_ms);
            trace_tool_result(ctx, tool_id, result, error, *duration_ms);
        }
        StreamEvent::Error { error, .. } => {
            log_error(ctx, error);
        }
        StreamEvent::WardChanged { ward_id, .. } => {
            handle_ward_changed(ctx, ward_id);
        }
        StreamEvent::SessionTitleChanged { ref title, .. } => {
            handle_session_title_changed(ctx, title);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Trace emission — full-fidelity events streamed to traces/<session>.jsonl.gz
// via the BatchWriter sink (additive alongside the slim execution_logs).
// ---------------------------------------------------------------------------

fn emit_trace(ctx: &StreamContext, event: zbot_trace::TraceEvent) {
    if let Some(writer) = &ctx.batch_writer {
        writer.trace_event(&ctx.session_id, event);
    }
}

fn trace_tool_call(ctx: &StreamContext, tool_id: &str, tool_name: &str, args: &serde_json::Value) {
    emit_trace(
        ctx,
        zbot_trace::TraceEvent {
            trace_id: ctx.session_id.clone(),
            span_id: tool_id.to_string(),
            session_id: ctx.session_id.clone(),
            execution_id: ctx.execution_id.clone(),
            agent_id: ctx.agent_id.clone(),
            parent_session_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: "info".into(),
            category: "tool_call".into(),
            message: format!("Calling tool: {tool_name}"),
            duration_ms: None,
            tool_name: Some(tool_name.to_string()),
            payload: Some(args.clone()),
            usage: None,
            model: None,
        },
    );
}

fn trace_tool_result(
    ctx: &StreamContext,
    tool_id: &str,
    result: &str,
    error: &Option<String>,
    duration_ms: Option<i64>,
) {
    // Full, untruncated result (the .jsonl.gz is the full-fidelity source;
    // execution_logs.metadata carries only the truncated/scalar preview).
    let level = if error.is_some() { "error" } else { "info" };
    emit_trace(
        ctx,
        zbot_trace::TraceEvent {
            trace_id: ctx.session_id.clone(),
            span_id: tool_id.to_string(),
            session_id: ctx.session_id.clone(),
            execution_id: ctx.execution_id.clone(),
            agent_id: ctx.agent_id.clone(),
            parent_session_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level.into(),
            category: "tool_result".into(),
            message: format!(
                "Tool result ({tool_id}): {}",
                error.as_deref().unwrap_or("ok")
            ),
            duration_ms,
            // tool_name is not carried on StreamEvent::ToolResult; the
            // tool_call event (same span_id) carries it. Analytics joins on
            // span_id (refined in T8/T14).
            tool_name: None,
            payload: Some(serde_json::Value::String(result.to_string())),
            usage: None,
            model: None,
        },
    );
}

fn handle_ward_changed(ctx: &StreamContext, ward_id: &str) {
    // Persist ward_id to session so it survives across continuations
    if let Err(e) = ctx
        .state_service
        .update_session_ward(&ctx.session_id, ward_id)
    {
        tracing::warn!("Failed to update session ward: {}", e);
    }

    // Scaffold ward structure from RECOMMENDED skills only (not all skills on disk).
    // This prevents life-os directories appearing in financial-analysis wards, etc.
    let ward_dir = ctx.vault_dir.join("wards").join(ward_id);
    if ward_dir.exists() {
        let skills_dir = ctx.vault_dir.join("skills");
        let setups = if ctx.recommended_skills.is_empty() {
            // No intent analysis (simple approach or fallback) — use coding skill only
            collect_ward_setup_for_skill(&skills_dir, "coding")
        } else {
            collect_ward_setups_for_skills(&skills_dir, &ctx.recommended_skills)
        };
        if !setups.is_empty() {
            crate::middleware::ward_scaffold::scaffold_ward(&ward_dir, ward_id, &setups);
            tracing::info!(ward = %ward_id, skills = ?ctx.recommended_skills, "Ward scaffolded from recommended skills");
        }

        // AGENTS.md is curated manually by the agent after ward creation;
        // the runtime no longer auto-rewrites it here.
    }
}

fn handle_session_title_changed(ctx: &StreamContext, title: &str) {
    // Persist title to session
    if let Err(e) = ctx
        .state_service
        .update_session_title(&ctx.session_id, title)
    {
        tracing::warn!("Failed to update session title: {}", e);
    }
}

fn extract_response_delta(gateway_event: &Option<GatewayEvent>) -> Option<String> {
    // Note: Token events stream incrementally during final response (when no tool calls)
    // TurnComplete contains the final response and is used as fallback marker
    match gateway_event {
        Some(GatewayEvent::Token { delta, .. }) => Some(delta.clone()),
        Some(GatewayEvent::Respond { message, .. }) => Some(format!("\n\n{}", message)),
        // TurnComplete is handled specially - marked with prefix so accumulator can detect fallback
        Some(GatewayEvent::TurnComplete { message, .. }) if !message.is_empty() => {
            Some(format!("{}{}", TURN_COMPLETE_MARKER, message))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_logs::LogService;
    use execution_state::{
        AgentExecution, DelegationType, SessionPlanStep, SessionPlanStepStatus, StateService,
    };
    use gateway_events::EventBus;
    use gateway_services::VaultPaths;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use zbot_runtime_sqlite::DatabaseManager;

    struct Harness {
        _temp: TempDir,
        paths: Arc<VaultPaths>,
        state: Arc<StateService<DatabaseManager>>,
        logs: Arc<LogService<DatabaseManager>>,
        bus: Arc<EventBus>,
        session_id: String,
        root_execution: AgentExecution,
    }

    fn setup() -> Harness {
        let temp = TempDir::new().expect("temp vault");
        let paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("vault directories");
        let db = Arc::new(DatabaseManager::new(paths.clone()).expect("database"));
        let state = Arc::new(StateService::new(db.clone()));
        let logs = Arc::new(LogService::new(db));
        let (session, root_execution) = state.create_session("root-agent").expect("session");
        Harness {
            _temp: temp,
            paths,
            state,
            logs,
            bus: Arc::new(EventBus::new()),
            session_id: session.id,
            root_execution,
        }
    }

    fn context(harness: &Harness, execution: &AgentExecution) -> StreamContext {
        let (delegation_tx, _delegation_rx) = mpsc::unbounded_channel();
        StreamContext::new(
            execution.agent_id.clone(),
            harness.session_id.clone(),
            harness.session_id.clone(),
            execution.id.clone(),
            harness.bus.clone(),
            harness.logs.clone(),
            harness.state.clone(),
            delegation_tx,
            harness.paths.vault_dir().clone(),
        )
    }

    #[test]
    fn projected_plan_surface_has_one_plan_checklist_and_no_open_loops() {
        let snapshot = SessionPlanSnapshot {
            execution_id: "exec-1".to_owned(),
            explanation: Some("Compare the options".to_owned()),
            plan: vec![SessionPlanStep {
                step: "Inspect the current setup".to_owned(),
                status: SessionPlanStepStatus::InProgress,
            }],
            updated_at: "2026-07-14T12:00:00Z".to_owned(),
            source_event_timestamp: 1,
            source_event_sequence: 1,
        };

        let surface = build_plan_surface(plan_surface_id("sess-1"), &snapshot);

        assert_eq!(surface.components.len(), 1);
        assert_eq!(
            surface.components[0].component_type,
            ComponentType::PlanChecklist
        );
        assert!(surface.data.get("open_loops").is_none());
        assert_eq!(surface.data["plan"][0]["status"], "in_progress");
    }

    #[test]
    fn surface_events_persist_only_when_enabled_and_reject_actionable_components() {
        // STUB: AC2/AC3/AC5 — publication is independent of durable storage.
        let harness = setup();
        let snapshot = SessionPlanSnapshot {
            execution_id: harness.root_execution.id.clone(),
            explanation: None,
            plan: vec![SessionPlanStep {
                step: "Inspect".to_owned(),
                status: SessionPlanStepStatus::Pending,
            }],
            updated_at: "2026-07-28T00:00:00Z".to_owned(),
            source_event_timestamp: 1,
            source_event_sequence: 1,
        };
        let surface = build_plan_surface("surface-persist".to_owned(), &snapshot);
        let created = GatewayEvent::SurfaceCreated {
            session_id: harness.session_id.clone(),
            execution_id: harness.root_execution.id.clone(),
            surface: surface.clone(),
        };

        persist_gateway_surface(&harness.state, &created);
        assert!(harness
            .state
            .list_session_surfaces(&harness.session_id)
            .unwrap()
            .is_empty());

        harness.state.set_surface_persistence_enabled(true);
        persist_gateway_surface(&harness.state, &created);
        assert_eq!(
            harness
                .state
                .list_session_surfaces(&harness.session_id)
                .unwrap()
                .len(),
            1
        );

        let mut actionable = surface;
        actionable.components[0].component_type = ComponentType::ApprovalGate;
        persist_gateway_surface(
            &harness.state,
            &GatewayEvent::SurfaceUpdated {
                session_id: harness.session_id.clone(),
                execution_id: harness.root_execution.id.clone(),
                surface: actionable,
            },
        );
        assert_eq!(
            harness
                .state
                .list_session_surfaces(&harness.session_id)
                .unwrap()
                .len(),
            0,
            "a non-persistable update must evict the older saved descriptor"
        );

        persist_gateway_surface(&harness.state, &created);
        assert_eq!(
            harness
                .state
                .list_session_surfaces(&harness.session_id)
                .unwrap()
                .len(),
            1
        );

        persist_gateway_surface(
            &harness.state,
            &GatewayEvent::SurfaceDeleted {
                session_id: harness.session_id.clone(),
                execution_id: harness.root_execution.id.clone(),
                surface_id: "surface-persist".to_owned(),
            },
        );
        assert!(harness
            .state
            .list_session_surfaces(&harness.session_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn accepted_plan_persists_before_one_session_scoped_research_surface() {
        let harness = setup();
        let context = context(&harness, &harness.root_execution);
        let mut events = harness.bus.subscribe_all();

        process_stream_event(
            &context,
            &StreamEvent::ActionPlanUpdate {
                timestamp: 10,
                plan: serde_json::json!([{"step": "Inspect configuration", "status": "pending"}]),
                explanation: None,
            },
        );

        let detail = harness
            .state
            .get_mission_control_session_tokens(&harness.session_id)
            .expect("load tokens")
            .expect("session exists");
        assert_eq!(
            detail.current_plan.expect("persisted plan").plan[0].step,
            "Inspect configuration"
        );

        let GatewayEvent::SurfaceCreated { surface, .. } =
            events.try_recv().expect("surface event")
        else {
            panic!("accepted plan should create a surface");
        };
        assert_eq!(surface.surface_id, plan_surface_id(&harness.session_id));
        assert_eq!(surface.components.len(), 1);
    }

    #[test]
    fn rejected_plan_keeps_prior_snapshot_and_emits_no_surface() {
        let harness = setup();
        let context = context(&harness, &harness.root_execution);
        let mut events = harness.bus.subscribe_all();

        let (gateway_event, _) = process_stream_event(
            &context,
            &StreamEvent::ActionPlanUpdate {
                timestamp: 10,
                plan: serde_json::json!([{"step": "not persisted", "status": "unknown"}]),
                explanation: Some("must not be logged or stored".to_owned()),
            },
        );

        let detail = harness
            .state
            .get_mission_control_session_tokens(&harness.session_id)
            .expect("load tokens")
            .expect("session exists");
        assert!(detail.current_plan.is_none());
        assert!(
            gateway_event.is_none(),
            "rejected plan must not be rebroadcast"
        );
        assert!(
            events.try_recv().is_err(),
            "rejected plan must not create a surface"
        );
    }

    #[test]
    fn oversized_plan_is_not_cloned_or_broadcast_after_preflight_rejection() {
        let harness = setup();
        let context = context(&harness, &harness.root_execution);
        let mut events = harness.bus.subscribe_all();
        let plan = serde_json::Value::Array(
            (0..21)
                .map(|index| {
                    serde_json::json!({
                        "step": format!("Step {index}"),
                        "status": "pending",
                    })
                })
                .collect(),
        );

        let (gateway_event, _) = process_stream_event(
            &context,
            &StreamEvent::ActionPlanUpdate {
                timestamp: 10,
                plan,
                explanation: None,
            },
        );

        assert!(
            gateway_event.is_none(),
            "preflight-rejected plans must not be converted or rebroadcast"
        );
        assert!(
            events.try_recv().is_err(),
            "preflight-rejected plans must not create a surface"
        );
    }

    #[test]
    fn root_and_subagent_plan_updates_share_one_research_surface_identifier() {
        let harness = setup();
        let child = AgentExecution::new_delegated(
            &harness.session_id,
            "researcher-agent",
            &harness.root_execution.id,
            DelegationType::Sequential,
            "research",
        );
        harness
            .state
            .create_execution(&child)
            .expect("child execution");
        let root_context = context(&harness, &harness.root_execution);
        let child_context = context(&harness, &child);
        let mut events = harness.bus.subscribe_all();

        for (ctx, timestamp, step) in [
            (&root_context, 10, "Root plan"),
            (&child_context, 11, "Subagent plan"),
        ] {
            process_stream_event(
                ctx,
                &StreamEvent::ActionPlanUpdate {
                    timestamp,
                    plan: serde_json::json!([{"step": step, "status": "in_progress"}]),
                    explanation: None,
                },
            );
        }

        let mut surface_ids = Vec::new();
        while let Ok(GatewayEvent::SurfaceCreated { surface, .. }) = events.try_recv() {
            surface_ids.push(surface.surface_id);
        }
        assert_eq!(surface_ids, vec![plan_surface_id(&harness.session_id); 2]);
    }

    #[test]
    fn artifact_declaration_uses_the_persisted_session_ward() {
        let harness = setup();
        let ward_id = "financial-analysis";
        let artifact_path = harness
            .paths
            .wards_dir()
            .join(ward_id)
            .join("output/report.md");
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(&artifact_path, "# Report").unwrap();
        harness
            .state
            .update_session_ward(&harness.session_id, ward_id)
            .unwrap();

        let context = context(&harness, &harness.root_execution);
        process_stream_event(
            &context,
            &StreamEvent::ActionRespond {
                timestamp: 1,
                message: "Done".to_string(),
                format: "text".to_string(),
                conversation_id: None,
                session_id: None,
                artifacts: vec![agent_primitives::event::ArtifactDeclaration {
                    path: "output/report.md".to_string(),
                    label: Some("Report".to_string()),
                    is_goal_artifact: true,
                }],
            },
        );

        let artifacts = harness
            .state
            .list_artifacts_by_session(&harness.session_id)
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].ward_id.as_deref(), Some(ward_id));
        assert_eq!(
            artifacts[0].file_path,
            artifact_path.to_string_lossy(),
            "artifact path must resolve under the persisted ward, not scratch"
        );
    }
}
