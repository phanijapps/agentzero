//! Pure `MultiTurnStreamItem` → `StreamEvent` mapping functions.
//!
//! Extracted from the engine loop so each arm is unit-testable without a
//! live engine and the loop itself reads as orchestration. Every function
//! takes explicit parameters — no engine borrow — and preserves the exact
//! event sequence, field values, and policy side effects the inline code
//! produced (the golden turn traces pin this contract).

use std::collections::HashMap;

use rig::completion::message::ToolResult as RigToolResult;

use super::context_policy::ContextPolicy;
use super::tool_results::SharedToolResults;
use crate::engine::StreamEventSink;
use crate::tool_visibility::{externally_visible_tool_args, externally_visible_tool_result};
use crate::types::events::current_timestamp;
use crate::types::StreamEvent;
use serde_json::Value;
use std::sync::Arc;

/// Assistant text delta → `Token` emit, policy text tracking, and message
/// accumulation.
pub(super) fn map_assistant_text(
    policy: Option<&Arc<ContextPolicy>>,
    text: &str,
    final_message: &mut String,
    on_event: &mut StreamEventSink<'_>,
) {
    if let Some(policy) = policy {
        policy.text(text);
    }
    final_message.push_str(text);
    on_event(StreamEvent::Token {
        timestamp: current_timestamp(),
        content: text.to_string(),
    });
}

/// Assistant tool-call delta → `ToolCallStart` emit plus call-id/name
/// registration for later result correlation.
pub(super) fn map_assistant_tool_call(
    tool_call: &rig::completion::message::ToolCall,
    tool_names_by_call_id: &mut HashMap<String, (String, serde_json::Value)>,
    peer_influenced: bool,
    on_event: &mut StreamEventSink<'_>,
) {
    let tool_id = tool_call
        .call_id
        .clone()
        .unwrap_or_else(|| tool_call.id.clone());
    tool_names_by_call_id.insert(
        tool_id.clone(),
        (
            tool_call.function.name.clone(),
            tool_call.function.arguments.clone(),
        ),
    );
    on_event(StreamEvent::ToolCallStart {
        timestamp: current_timestamp(),
        tool_id,
        tool_name: tool_call.function.name.clone(),
        args: externally_visible_tool_args(
            &tool_call.function.name,
            &tool_call.function.arguments,
            peer_influenced,
        ),
    });
}

/// Reasoning delta → `Reasoning` emit.
pub(super) fn map_reasoning(reasoning: String, on_event: &mut StreamEventSink<'_>) {
    on_event(StreamEvent::Reasoning {
        timestamp: current_timestamp(),
        content: reasoning,
    });
}

/// The user-side tool-result arm: correlation, policy recording, the
/// delegate/respond action emission (returning the loop signal), the
/// surface/ward/plan marker parses, and the `ToolResult`/`ToolCallEnd`
/// pair. Returns the signal the loop should switch to (`Continue` when
/// the turn keeps going).
pub(super) fn map_tool_result(
    tool_result: &RigToolResult,
    results: &SharedToolResults,
    policy: Option<&Arc<ContextPolicy>>,
    shared_context: &super::SharedToolContext,
    tool_names_by_call_id: &mut HashMap<String, (String, serde_json::Value)>,
    on_event: &mut StreamEventSink<'_>,
) -> super::turn_signal::TurnSignal {
    use super::turn_signal::TurnSignal;

    let outcome = results.take();
    let context_text = outcome
        .context
        .unwrap_or_else(|| tool_result_text(tool_result));
    let result_text = outcome.raw.unwrap_or_else(|| context_text.clone());
    if let Some((name, args)) = &outcome.rejected_call {
        // Rig's invalid-call recovery omits a dispatch-start item because
        // no tool ran; retain our attempt trace.
        on_event(StreamEvent::ToolCallStart {
            timestamp: current_timestamp(),
            tool_id: tool_result.id.clone(),
            tool_name: name.clone(),
            args: externally_visible_tool_args(name, args, results.peer_influenced()),
        });
    }
    let tool_info = tool_names_by_call_id
        .remove(&tool_result.id)
        .or(outcome.rejected_call);
    let is_surface_tool = tool_info
        .as_ref()
        .is_some_and(|(name, _)| name == "present_surface");
    if let (Some(policy), Some((name, args))) = (policy, &tool_info) {
        policy.record_tool(name, args, outcome.error.as_deref());
        policy.completed(&tool_result.id, name, args, &context_text);
    }
    let (event_result, event_context, event_error) = externally_visible_tool_result(
        results.peer_influenced(),
        result_text.clone(),
        Some(context_text),
        outcome.error,
    );
    // Surface tool side-effects set on the shared context
    // (delegate/respond), mirroring the legacy executor. Without
    // ActionDelegate, delegate_to_agent would not spawn a child and
    // wait_agent would hang forever on the Rig path.
    let actions = shared_context.take_actions();
    let mut signal = TurnSignal::Continue;
    if let Some(delegate) = actions.delegate {
        if !delegate.parallel {
            signal = TurnSignal::DelegationYield;
        }
        on_event(StreamEvent::ActionDelegate {
            timestamp: current_timestamp(),
            agent_id: delegate.agent_id,
            task: delegate.task,
            context: delegate.context,
            wait_for_result: delegate.wait_for_result,
            max_iterations: delegate.max_iterations,
            output_schema: delegate.output_schema,
            skills: delegate.skills,
            capability_assignment: delegate.capability_assignment,
            planning_capability_catalog: delegate.planning_capability_catalog,
            complexity: delegate.complexity,
            mode: delegate.mode,
            parallel: delegate.parallel,
            child_execution_id: delegate.child_execution_id,
        });
    }
    if let Some(respond) = actions.respond {
        signal = TurnSignal::Responded;
        if let Some(policy) = policy {
            policy.record_respond();
            // In-session reflexion: discharge recovered failures at respond
            // (event-only in the runtime; the gateway persists them).
            let recovered = policy.drain_recovered();
            if !recovered.is_empty() {
                on_event(StreamEvent::RecoveredFailures {
                    timestamp: current_timestamp(),
                    items: recovered
                        .into_iter()
                        .map(|item| crate::RecoveredFailureItem {
                            tool: item.tool,
                            last_error: item.last_error,
                        })
                        .collect(),
                });
            }
        }
        on_event(StreamEvent::ActionRespond {
            timestamp: current_timestamp(),
            message: respond.message,
            format: respond.format,
            conversation_id: respond.conversation_id,
            session_id: respond.session_id,
            artifacts: respond.artifacts,
        });
    }
    emit_result_markers(&result_text, is_surface_tool, on_event);
    on_event(StreamEvent::ToolResult {
        timestamp: current_timestamp(),
        tool_id: tool_result.id.clone(),
        result: event_result,
        context_result: event_context,
        error: event_error,
        duration_ms: Some(outcome.duration_ms),
    });
    if let Some((tool_name, args)) = tool_info {
        on_event(StreamEvent::ToolCallEnd {
            timestamp: current_timestamp(),
            tool_id: tool_result.id.clone(),
            args: externally_visible_tool_args(&tool_name, &args, results.peer_influenced()),
            tool_name,
        });
    }
    signal
}

/// Marker parses from tool result JSON: `__work_surface*` (surface tools
/// only), `__session_title_changed__`, `__ward_changed__`, `__plan_update`.
/// Producers signal via their return JSON; without this the corresponding
/// stream events never publish.
fn emit_result_markers(
    result_text: &str,
    is_surface_tool: bool,
    on_event: &mut StreamEventSink<'_>,
) {
    let Ok(parsed) = serde_json::from_str::<Value>(result_text) else {
        return;
    };
    if is_surface_tool {
        if parsed
            .get("__work_surface")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(surface) = parsed
                .get("surface")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
            {
                on_event(StreamEvent::WorkSurface {
                    timestamp: current_timestamp(),
                    surface,
                });
            }
        }
        if parsed
            .get("__work_surface_updated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(surface) = parsed
                .get("surface")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
            {
                on_event(StreamEvent::WorkSurfaceUpdated {
                    timestamp: current_timestamp(),
                    surface,
                });
            }
        }
        if parsed
            .get("__work_surface_deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(surface_id) = parsed
                .get("surface_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 128)
            {
                on_event(StreamEvent::WorkSurfaceDeleted {
                    timestamp: current_timestamp(),
                    surface_id: surface_id.to_owned(),
                });
            }
        }
    }
    if parsed
        .get("__session_title_changed__")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if let Some(title) = parsed.get("title").and_then(Value::as_str) {
            on_event(StreamEvent::SessionTitleChanged {
                timestamp: current_timestamp(),
                title: title.to_string(),
            });
        }
    }
    if parsed
        .get("__ward_changed__")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if let Some(ward_id) = parsed.get("ward_id").and_then(Value::as_str) {
            on_event(StreamEvent::WardChanged {
                timestamp: current_timestamp(),
                ward_id: ward_id.to_string(),
            });
        }
    }
    if parsed
        .get("__plan_update")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let plan = parsed
            .get("plan")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let explanation = parsed
            .get("explanation")
            .and_then(Value::as_str)
            .map(std::string::ToString::to_string);
        on_event(StreamEvent::ActionPlanUpdate {
            timestamp: current_timestamp(),
            plan,
            explanation,
        });
    }
}

/// Completion-call usage → cumulative `TokenUpdate` emit.
pub(super) fn map_completion_call(
    input_tokens: u64,
    output_tokens: u64,
    total_input: &mut u64,
    total_output: &mut u64,
    on_event: &mut StreamEventSink<'_>,
) {
    // Emit token usage (cumulative) so the gateway records per-execution
    // token counts via TokenUpdate → batch_writer.
    *total_input += input_tokens;
    *total_output += output_tokens;
    on_event(StreamEvent::TokenUpdate {
        timestamp: current_timestamp(),
        tokens_in: *total_input,
        tokens_out: *total_output,
    });
}

/// Extract the model-visible text from a rig tool result.
pub(super) fn tool_result_text(tool_result: &RigToolResult) -> String {
    use rig::completion::message::ToolResultContent;
    tool_result
        .content
        .iter()
        .filter_map(|content| match content {
            ToolResultContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
