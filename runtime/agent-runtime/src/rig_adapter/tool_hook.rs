//! Existing tool policy mapped onto Rig's before/after dispatch hooks.
use super::{
    tool_results::{SharedToolResults, ToolOutcome},
    SharedToolContext,
};
use crate::{AfterToolCallHook, BeforeToolCallHook, ToolCallDecision, ToolResultContextConfig};
use agent_primitives::CallbackContext;
use rig::{
    agent::{AgentHook, Flow, StepEvent},
    completion::CompletionModel,
};
use serde_json::{json, Value};

pub(super) struct RigExecutionHook {
    pub ctx: SharedToolContext,
    pub before: Option<BeforeToolCallHook>,
    pub after: Option<AfterToolCallHook>,
    pub results: SharedToolResults,
    pub context_config: ToolResultContextConfig,
}

impl<M: CompletionModel> AgentHook<M> for RigExecutionHook {
    async fn on_event(&self, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::ToolCall {
                tool_name,
                tool_call_id,
                internal_call_id,
                args,
            } => {
                self.ctx
                    .set_function_call_id(tool_call_id.unwrap_or(internal_call_id).to_owned());
                let args = serde_json::from_str::<Value>(args).unwrap_or(Value::Null);
                let decision = if self.results.peer_influenced() && tool_name != "respond" {
                    ToolCallDecision::Block {
                        reason: "peer_data_authority_boundary".into(),
                    }
                } else {
                    self.before
                        .as_ref()
                        .map_or(ToolCallDecision::Allow, |hook| hook(tool_name, &args))
                };
                if let ToolCallDecision::Block { reason } = decision {
                    let context = json!({"blocked":true,"reason":reason}).to_string();
                    self.results.record(ToolOutcome {
                        raw: Some("[blocked by hook]".into()),
                        error: Some("blocked_by_hook".into()),
                        context: Some(context.clone()),
                        duration_ms: 0,
                        rejected_call: None,
                    });
                    return Flow::skip(context);
                }
                Flow::cont()
            }
            StepEvent::CompletionCall { .. } => {
                self.ctx
                    .set_state("app:delegation_active".into(), Value::Bool(false));
                Flow::cont()
            }
            StepEvent::InvalidToolCall(call) => {
                let args = call
                    .args
                    .as_deref()
                    .and_then(|args| serde_json::from_str::<Value>(args).ok())
                    .unwrap_or(Value::Null);
                let context = self.shape_result(
                    &call.tool_name,
                    &args,
                    ToolOutcome {
                        raw: Some(String::new()),
                        error: Some(format!("Tool not found or not allowed: {}", call.tool_name)),
                        rejected_call: Some((call.tool_name.clone(), args.clone())),
                        ..ToolOutcome::default()
                    },
                );
                // Rig validates against its actual registered/allowed inventory
                // before dispatch. Skip supplies model feedback, never repairs
                // an unauthorized name into an executable capability.
                Flow::skip(context)
            }
            StepEvent::ToolResult {
                tool_name,
                args,
                result,
                ..
            } => {
                let mut outcome = self.results.snapshot();
                outcome.raw.get_or_insert_with(|| result.to_owned());
                let args = serde_json::from_str::<Value>(args).unwrap_or(Value::Null);
                let context = self.shape_result(tool_name, &args, outcome);
                // Always rewrite verbatim: Rig must not reinterpret JSON-shaped
                // context as its own multimodal result protocol.
                Flow::rewrite_result(context)
            }
            _ => Flow::cont(),
        }
    }
}

impl RigExecutionHook {
    fn shape_result(&self, tool_name: &str, args: &Value, mut outcome: ToolOutcome) -> String {
        let succeeded = outcome.error.is_none();
        let context = if succeeded {
            crate::prepare_tool_result_for_context(
                tool_name,
                outcome.raw.clone().unwrap_or_default(),
                &self.context_config,
            )
        } else {
            json!({"error":outcome.error}).to_string()
        };
        let context = self
            .after
            .as_ref()
            .and_then(|hook| hook(tool_name, args, &context, succeeded))
            .unwrap_or(context);
        outcome.context = Some(context.clone());
        self.results.record(outcome);
        context
    }
}
