// ============================================================================
// UPDATE PLAN TOOL
// Lightweight fire-and-forget plan tracking.
// No persistence, no UUIDs — just a status checklist for the model to track progress.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext};

use crate::tools::guards::{has_placeholder_specs, planning_gate_awaits_ward};

// ============================================================================
// UPDATE PLAN TOOL
// ============================================================================

/// Lightweight plan tool that accepts a checklist of steps with statuses.
/// Returns "Plan updated" immediately — fire-and-forget.
pub struct UpdatePlanTool;

impl UpdatePlanTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpdatePlanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Track task progress with a lightweight checklist. Each step has a status: pending, in_progress, completed, or failed. Use for complex tasks (5+ steps). Skip for simple tasks."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "explanation": {
                    "type": "string",
                    "description": "Brief explanation of plan changes (optional)"
                },
                "plan": {
                    "type": "array",
                    "description": "Task checklist with step descriptions and statuses",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": {
                                "type": "string",
                                "description": "Description of the step"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "failed"],
                                "description": "Current status of this step"
                            }
                        },
                        "required": ["step", "status"]
                    }
                }
            },
            "required": ["plan"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        if planning_gate_awaits_ward(ctx.as_ref()) {
            return Ok(json!({
                "status": "redirect",
                "message": "This graph request must establish its ward first. The system will start planner-agent after ward(create/use); do not publish a root checklist before then."
            }));
        }

        if has_placeholder_specs(ctx.as_ref()) {
            return Ok(crate::tools::guards::placeholder_specs_redirect(
                "Delegate to a planning subagent to fill them instead of writing your own plan.",
            ));
        }

        // Check for error markers from truncated/malformed tool calls
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AgentError::Tool(format!("{}: {}", error_type, message)));
        }

        let plan = args
            .get("plan")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AgentError::Tool("Missing 'plan' array parameter".to_string()))?;

        if plan.is_empty() {
            return Err(AgentError::Tool("Plan cannot be empty".to_string()));
        }

        // Part B: Check for plan replacement (existing plan with progress being fully reset)
        let mut replacement_warning = None;
        if let Some(existing) = ctx.get_state("app:plan")
            && let Some(existing_steps) = existing.get("plan").and_then(|p| p.as_array())
        {
            let has_progress = existing_steps.iter().any(|s| {
                let st = s.get("status").and_then(|v| v.as_str()).unwrap_or("");
                st == "completed" || st == "failed"
            });
            if has_progress && let Some(new_steps) = args.get("plan").and_then(|p| p.as_array()) {
                let all_pending = new_steps
                    .iter()
                    .all(|s| s.get("status").and_then(|v| v.as_str()) == Some("pending"));
                if all_pending {
                    replacement_warning = Some(
                        "Warning: You are replacing a plan that had completed/failed steps. \
                                 Update step statuses instead of creating a new plan.",
                    );
                    tracing::warn!("Plan replacement detected — existing plan had progress");
                }
            }
        }

        // Part C: Subagent plan cap — delegated executors limited to 5 steps
        let mut truncation_warning = None;
        let is_delegated = ctx
            .get_state("app:is_delegated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut plan_args = args.clone();
        if is_delegated
            && let Some(plan_array) = plan_args.get_mut("plan").and_then(|p| p.as_array_mut())
            && plan_array.len() > 5
        {
            let original_len = plan_array.len();
            plan_array.truncate(5);
            truncation_warning = Some(format!(
                "Plan truncated from {} to 5 steps. You are a specialist — keep tasks focused.",
                original_len
            ));
            tracing::info!("Subagent plan truncated from {} to 5 steps", original_len);
        }

        // Store the (possibly truncated) plan in session state for UI rendering
        ctx.set_state("app:plan".to_string(), plan_args.clone());

        let final_plan = plan_args
            .get("plan")
            .and_then(|v| v.as_array())
            .unwrap_or(plan);
        tracing::debug!("Plan updated: {} steps", final_plan.len());

        // Build response with optional warnings
        let mut response = json!({
            "__plan_update": true,
            "plan": final_plan,
            "message": "Plan updated"
        });

        if let Some(warning) = replacement_warning {
            response["replacement_warning"] = json!(warning);
        }
        if let Some(warning) = truncation_warning {
            response["truncation_warning"] = json!(warning);
        }

        // Return response — fire-and-forget
        Ok(response)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::event::EventActions;
    use agent_primitives::types::Content;
    use agent_primitives::{CallbackContext, ReadonlyContext};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct GateContext {
        state: Mutex<HashMap<String, Value>>,
        content: Content,
    }

    impl GateContext {
        fn cold_graph() -> Self {
            let mut state = HashMap::new();
            state.insert(
                crate::tools::guards::PLANNING_GATE_STATE.to_string(),
                serde_json::to_value(crate::tools::guards::PlanningGate::awaiting_ward(
                    "Plan this request",
                ))
                .unwrap(),
            );
            Self {
                state: Mutex::new(state),
                content: Content::user(""),
            }
        }
    }

    impl ReadonlyContext for GateContext {
        fn invocation_id(&self) -> &str {
            "test"
        }
        fn agent_name(&self) -> &str {
            "root"
        }
        fn user_id(&self) -> &str {
            "test"
        }
        fn app_name(&self) -> &str {
            "test"
        }
        fn session_id(&self) -> &str {
            "test"
        }
        fn branch(&self) -> &str {
            "test"
        }
        fn user_content(&self) -> &Content {
            &self.content
        }
    }

    impl CallbackContext for GateContext {
        fn get_state(&self, key: &str) -> Option<Value> {
            self.state.lock().ok()?.get(key).cloned()
        }

        fn set_state(&self, key: String, value: Value) {
            if let Ok(mut state) = self.state.lock() {
                state.insert(key, value);
            }
        }
    }

    impl ToolContext for GateContext {
        fn function_call_id(&self) -> String {
            "test".to_string()
        }
        fn actions(&self) -> EventActions {
            EventActions::default()
        }
        fn set_actions(&self, _actions: EventActions) {}
    }

    #[test]
    fn test_update_plan_schema() {
        let tool = UpdatePlanTool::new();
        assert_eq!(tool.name(), "update_plan");
        let schema = tool.parameters_schema().unwrap();
        assert!(schema.get("properties").unwrap().get("plan").is_some());
    }

    #[tokio::test]
    async fn cold_graph_gate_redirects_root_checklist() {
        let tool = UpdatePlanTool::new();
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::cold_graph());

        let result = tool
            .execute(
                ctx.clone(),
                json!({"plan": [{"step": "Skip planner", "status": "in_progress"}]}),
            )
            .await
            .expect("planning gate returns a redirect");

        assert_eq!(result["status"], "redirect");
        assert!(ctx.get_state("app:plan").is_none());
    }
}
