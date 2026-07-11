//! # Stream Event Processor
//!
//! Processes individual stream events: handles side-effects, converts to gateway events,
//! and extracts response deltas for accumulation.

use agent_runtime::StreamEvent;
use agent_surfaces::{ComponentType, SurfaceComponent, WorkSurface, ZBOT_WORK_SURFACE_CATALOG};
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
    publish_projected_surface(ctx, event);

    // Convert to gateway event (may return None for internal events)
    let gateway_event = crate::events::convert_stream_event(
        event.clone(),
        &ctx.agent_id,
        &ctx.conversation_id,
        &ctx.session_id,
        &ctx.execution_id,
    );

    let response_delta = extract_response_delta(&gateway_event);
    (gateway_event, response_delta)
}

/// Project gateway-owned plan data into the initial native work-surface
/// catalog. This uses the `update_plan` tool's structured payload rather than
/// interpreting arbitrary assistant text as UI.
fn publish_projected_surface(ctx: &StreamContext, event: &StreamEvent) {
    let StreamEvent::ActionPlanUpdate { plan, .. } = event else {
        return;
    };
    let surface_id = format!("plan-{}", ctx.execution_id);
    let is_update = ctx
        .surface_ids
        .lock()
        .map(|mut ids| !ids.insert(surface_id.clone()))
        .unwrap_or(false);
    let surface = WorkSurface {
        surface_id,
        catalog_id: ZBOT_WORK_SURFACE_CATALOG.to_owned(),
        components: vec![
            SurfaceComponent {
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
            },
            SurfaceComponent {
                id: "open-loops".to_owned(),
                component_type: ComponentType::OpenLoops,
                props: BTreeMap::from([
                    (
                        "title".to_owned(),
                        serde_json::Value::String("Open loops".to_owned()),
                    ),
                    (
                        "items_path".to_owned(),
                        serde_json::Value::String("/open_loops".to_owned()),
                    ),
                ]),
            },
        ],
        data: serde_json::json!({ "plan": plan, "open_loops": plan }),
    };
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
        ctx.event_bus.publish_sync(surface_event);
    }
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
