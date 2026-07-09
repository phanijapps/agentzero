// ============================================================================
// PLAN-BLOCK MIDDLEWARE
// Inject a pinned structured anchor (goal + checklist) after the system
// prompt so long tool loops don't lose sight of the task.
// ============================================================================

//! # Plan-block middleware
//!
//! Reads `MiddlewareContext::plan_state` (populated by the executor from
//! the agent's `app:plan` session state) and injects a formatted anchor
//! right after the last system message. Runs every turn — replaces any
//! prior plan block so the anchor is always current.
//!
//! Ordering: run AFTER `ContextEditingMiddleware` so tool-result
//! clearing is done before the anchor is (re)inserted, and the anchor
//! itself survives any future compaction because:
//!
//!   - `ContextEditingMiddleware` clears `tool` messages, not `system`
//!   - `SummarizationMiddleware` (when/if it fires) will not touch
//!     messages flagged `is_summary = true` — the block is flagged as
//!     `is_summary` because it IS a compacted representation of the
//!     agent's intent, and re-summarizing a summary is the documented
//!     anti-pattern the flag was introduced for.
//!
//! Research context: the "Layer 1 pinned scratchpad" from
//! `docs/architecture/future-state/compaction-strategy.md` §4 and from the
//! Manus `todo.md` / Claude Code `claude-progress.txt` / Devin
//! progress-file pattern. This is the minimum useful version —
//! no subagent rewrite, no post-turn executor hook, just take what
//! `update_plan` already wrote and render it.

use super::traits::{MiddlewareContext, MiddlewareEffect, PreProcessMiddleware};
use crate::types::ChatMessage;
use agent_primitives::types::Part;
use async_trait::async_trait;
use serde_json::Value;

/// Scan a conversation tape backwards for the most recent `update_plan`
/// tool call and return its arguments as the plan state. This mirrors
/// [`super::traits::ExecutionState::from_messages`] — the plan info
/// lives in the messages themselves (since `update_plan` tool call
/// arguments carry the same `{plan, explanation}` shape the tool
/// stashes in `app:plan`), so the middleware doesn't need a separate
/// session-state accessor plumbed through the executor.
///
/// Returns `None` when no `update_plan` call has happened this
/// conversation — the agent hasn't authored a plan yet.
#[must_use]
pub fn extract_plan_state(messages: &[ChatMessage]) -> Option<Value> {
    for msg in messages.iter().rev() {
        if msg.role != "assistant" {
            continue;
        }
        let Some(tool_calls) = msg.tool_calls.as_ref() else {
            continue;
        };
        for tc in tool_calls.iter().rev() {
            if tc.name == "update_plan" {
                return Some(tc.arguments.clone());
            }
        }
    }
    None
}

/// Sentinel embedded in the rendered block so we can find and replace it
/// idempotently on each turn. Not stable wire format — can change.
const PLAN_BLOCK_MARKER: &str = "<!-- plan-block:v1 -->";

/// Middleware that injects a pinned plan-state anchor after the system
/// prompt. See module docs for motivation + ordering requirements.
#[derive(Debug, Default)]
pub struct PlanBlockMiddleware {
    enabled: bool,
}

impl PlanBlockMiddleware {
    /// Construct the middleware in the default (enabled) state.
    #[must_use]
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// Disable — the middleware becomes a no-op. Useful for tests.
    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Render a plan state `Value` (the thing `update_plan` writes to
    /// `app:plan`) into a human-readable markdown anchor. Shape expected:
    /// `{ explanation?: String, plan: [{ step, status }] }`. Unknown
    /// shapes degrade to "raw JSON" so we never silently drop data.
    fn render_plan_block(plan: &Value) -> String {
        let mut out = String::new();
        out.push_str(PLAN_BLOCK_MARKER);
        out.push_str("\n[Plan — current task state]\n");

        if let Some(explanation) = plan.get("explanation").and_then(Value::as_str) {
            if !explanation.is_empty() {
                out.push_str(&format!("Goal: {explanation}\n"));
            }
        }

        if let Some(steps) = plan.get("plan").and_then(Value::as_array) {
            for (idx, step) in steps.iter().enumerate() {
                let text = step
                    .get("step")
                    .and_then(Value::as_str)
                    .unwrap_or("<no step text>");
                let status = step
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("pending");
                let marker = match status {
                    "completed" => "[x]",
                    "in_progress" => "[~]",
                    "failed" => "[!]",
                    _ => "[ ]",
                };
                out.push_str(&format!("{marker} {}. {text}\n", idx + 1));
            }
        } else {
            // Unknown shape — embed raw JSON so information isn't lost.
            out.push_str(&plan.to_string());
            out.push('\n');
        }

        out
    }

    /// True iff this message is the plan-block we previously injected.
    /// Used to replace instead of stacking duplicates.
    fn is_plan_block(msg: &ChatMessage) -> bool {
        msg.role == "system" && msg.is_summary && msg.text_content().starts_with(PLAN_BLOCK_MARKER)
    }

    /// Build the plan-block message itself. Flagged `is_summary = true`
    /// because it's a compacted representation; `Summarization` and any
    /// future compaction pass must treat it as a pinned, non-summarizable
    /// anchor (the `is_summary` flag guard in context-editing etc.
    /// short-circuits these).
    fn build_block_message(plan: &Value) -> ChatMessage {
        ChatMessage {
            role: "system".to_string(),
            content: vec![Part::Text {
                text: Self::render_plan_block(plan),
            }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: true,
        }
    }
}

#[async_trait]
impl PreProcessMiddleware for PlanBlockMiddleware {
    fn name(&self) -> &'static str {
        "plan_block"
    }

    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(Self {
            enabled: self.enabled,
        })
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    async fn process(
        &self,
        mut messages: Vec<ChatMessage>,
        context: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        // No plan state → no-op. Agent hasn't called `update_plan` yet.
        let Some(plan_state) = context.plan_state.as_ref() else {
            return Ok(MiddlewareEffect::Proceed);
        };

        // Remove any prior plan-block so we don't stack duplicates.
        let prior_count = messages.len();
        messages.retain(|m| !Self::is_plan_block(m));
        let removed = prior_count - messages.len();

        // Insert the fresh plan block immediately after the last system
        // message. A well-formed conversation has system messages at the
        // head; walk forward to find the first non-system index.
        let insert_at = messages
            .iter()
            .position(|m| m.role != "system")
            .unwrap_or(messages.len());
        messages.insert(insert_at, Self::build_block_message(plan_state));

        tracing::debug!(
            agent_id = %context.agent_id,
            insert_at = insert_at,
            removed_prior = removed,
            "plan_block injected"
        );

        Ok(MiddlewareEffect::ModifiedMessages(messages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_ctx(plan_state: Option<Value>) -> MiddlewareContext {
        MiddlewareContext::new(
            "agent-test".to_string(),
            None,
            "openai".to_string(),
            "gpt-4o-mini".to_string(),
        )
        .with_plan_state(plan_state)
    }

    fn conversation_with_system_and_user() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("You are a helpful assistant.".to_string()),
            ChatMessage::user("do the thing".to_string()),
        ]
    }

    #[tokio::test]
    async fn no_plan_state_is_noop() {
        let mw = PlanBlockMiddleware::new();
        let ctx = make_ctx(None);
        let before = conversation_with_system_and_user();
        let effect = mw.process(before.clone(), &ctx).await.unwrap();
        assert!(matches!(effect, MiddlewareEffect::Proceed));
    }

    #[tokio::test]
    async fn injects_after_last_system_message() {
        let mw = PlanBlockMiddleware::new();
        let plan = json!({
            "explanation": "Debug the stock analyzer",
            "plan": [
                { "step": "Load spec file", "status": "completed" },
                { "step": "Run tests", "status": "in_progress" },
                { "step": "Fix regressions", "status": "pending" },
            ]
        });
        let ctx = make_ctx(Some(plan));
        let effect = mw
            .process(conversation_with_system_and_user(), &ctx)
            .await
            .unwrap();

        let MiddlewareEffect::ModifiedMessages(out) = effect else {
            panic!("expected ModifiedMessages");
        };

        // Order: system, plan-block (system+is_summary), user
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, "system");
        assert!(!out[0].is_summary, "original system must stay non-summary");
        assert_eq!(out[1].role, "system");
        assert!(out[1].is_summary, "plan block must be flagged is_summary");
        let block_text = out[1].text_content();
        assert!(block_text.contains("Debug the stock analyzer"));
        assert!(block_text.contains("[x] 1. Load spec file"));
        assert!(block_text.contains("[~] 2. Run tests"));
        assert!(block_text.contains("[ ] 3. Fix regressions"));
        assert_eq!(out[2].role, "user");
    }

    #[tokio::test]
    async fn re_run_replaces_prior_block_without_stacking() {
        let mw = PlanBlockMiddleware::new();

        let plan_v1 = json!({
            "plan": [
                { "step": "one", "status": "in_progress" }
            ]
        });
        let plan_v2 = json!({
            "plan": [
                { "step": "one", "status": "completed" },
                { "step": "two", "status": "in_progress" }
            ]
        });

        let first = mw
            .process(
                conversation_with_system_and_user(),
                &make_ctx(Some(plan_v1)),
            )
            .await
            .unwrap();
        let MiddlewareEffect::ModifiedMessages(after_v1) = first else {
            panic!();
        };
        assert_eq!(after_v1.len(), 3);

        let second = mw
            .process(after_v1, &make_ctx(Some(plan_v2)))
            .await
            .unwrap();
        let MiddlewareEffect::ModifiedMessages(after_v2) = second else {
            panic!();
        };

        // Still 3 messages — old block replaced, not stacked.
        assert_eq!(after_v2.len(), 3);
        let block_count = after_v2
            .iter()
            .filter(|m| PlanBlockMiddleware::is_plan_block(m))
            .count();
        assert_eq!(block_count, 1);
        let block_text = after_v2[1].text_content();
        assert!(block_text.contains("[x] 1. one"));
        assert!(block_text.contains("[~] 2. two"));
    }

    #[tokio::test]
    async fn unknown_plan_shape_degrades_gracefully() {
        let mw = PlanBlockMiddleware::new();
        // No `plan` array — raw JSON should be embedded, no panic.
        let plan = json!({ "something_else": "entirely" });
        let ctx = make_ctx(Some(plan));
        let effect = mw
            .process(conversation_with_system_and_user(), &ctx)
            .await
            .unwrap();
        let MiddlewareEffect::ModifiedMessages(out) = effect else {
            panic!("expected ModifiedMessages");
        };
        assert_eq!(out.len(), 3);
        let block_text = out[1].text_content();
        assert!(block_text.contains("something_else"));
    }

    #[tokio::test]
    async fn empty_conversation_appends_block() {
        let mw = PlanBlockMiddleware::new();
        let plan = json!({
            "plan": [{ "step": "do it", "status": "pending" }]
        });
        let ctx = make_ctx(Some(plan));
        let effect = mw.process(vec![], &ctx).await.unwrap();
        let MiddlewareEffect::ModifiedMessages(out) = effect else {
            panic!();
        };
        assert_eq!(out.len(), 1);
        assert!(PlanBlockMiddleware::is_plan_block(&out[0]));
    }

    #[tokio::test]
    async fn disabled_enabled_flag_signalled_correctly() {
        let enabled = PlanBlockMiddleware::new();
        assert!(enabled.enabled());
        let disabled = PlanBlockMiddleware::new().with_enabled(false);
        assert!(!disabled.enabled());
    }
}
