//! Advisory request policy and the existing stuck-loop safety valve.
use crate::{progress::ProgressTracker, ChatMessage, ExecutorError};
use serde_json::Value;

pub(super) struct ProgressConfig {
    pub turn_budget: u32,
    pub max_turns: u32,
    pub complexity: Option<String>,
    pub input_budget: u64,
    pub warn_pct: u64,
}
pub(super) struct ProgressPolicy {
    tracker: ProgressTracker,
    prompt_tokens: u64,
    turn_warned: bool,
    context_warned: bool,
    stuck_warned: bool,
    /// Last failing call already nudged (tool name, failure count at nudge
    /// time) — re-nudges only when a different call or a higher count wins.
    failure_nudge_key: Option<(String, u32)>,
}
impl Default for ProgressPolicy {
    fn default() -> Self {
        Self {
            tracker: ProgressTracker::new(),
            prompt_tokens: 0,
            turn_warned: false,
            context_warned: false,
            stuck_warned: false,
            failure_nudge_key: None,
        }
    }
}
impl ProgressPolicy {
    pub fn usage(&mut self, prompt: Option<u64>) {
        if let Some(prompt) = prompt {
            self.prompt_tokens = prompt;
        }
    }
    pub fn tool(&mut self, name: &str, args: &Value, error: Option<&str>) {
        let error = error.filter(|error| *error != "blocked_by_hook");
        self.tracker.record_tool_call(name, args, error);
        if let Some(error) = error {
            self.tracker.record_error(error);
        }
    }
    /// A respond action was emitted — the agent is finishing.
    pub fn respond(&mut self) {
        self.tracker.record_respond();
    }

    pub fn prepare(
        &mut self,
        cfg: &ProgressConfig,
        messages: &mut Vec<ChatMessage>,
    ) -> Result<(), ExecutorError> {
        self.tracker.tick();
        let turn = self.tracker.total_iterations;
        if self.tracker.needs_planning_nudge() {
            messages.push(ChatMessage::user(
                "[SYSTEM: You have made several tool calls without creating a plan. \
                 For complex tasks, use the `update_plan` tool to track your steps. \
                 This helps you stay focused and avoid repeating work.]"
                    .into(),
            ));
        }
        if cfg.turn_budget > 0 && turn >= cfg.turn_budget && !self.turn_warned {
            self.turn_warned = true;
            messages.push(ChatMessage::user(format!("[SYSTEM: You have used {turn} of {} tool calls. Wrap up your current work and call `respond` with a summary. Do not start new explorations.]",cfg.max_turns)));
        }
        if let Some(complexity) = &cfg.complexity {
            let (hard, soft) = match complexity.as_str() {
                "S" => (15, 12),
                "M" => (30, 24),
                "L" => (50, 40),
                "XL" => (100, 80),
                _ => (0, 0),
            };
            if hard > 0 && turn >= hard {
                messages.push(ChatMessage::user(format!("[STEER: System] Budget exceeded ({turn}/{hard} iterations for {complexity} task). Respond NOW with what you have. Do not start new work.")));
            } else if hard > 0 && turn == soft {
                messages.push(ChatMessage::user(format!("[STEER: System] You've used {turn}/{hard} iterations for a {complexity} task. Wrap up or simplify your approach.")));
            }
        }
        if let Some((name, count, error)) = self.tracker.top_failing_call() {
            // Nudge once per distinct failing call; re-nudge only when a
            // different call or a higher failure count takes the lead.
            let already_nudged = self
                .failure_nudge_key
                .as_ref()
                .is_some_and(|(n, c)| n == name && *c >= count);
            if !already_nudged {
                self.failure_nudge_key = Some((name.to_string(), count));
                let error_line = if error.is_empty() {
                    String::new()
                } else {
                    format!(" Last error: {error}.")
                };
                messages.push(ChatMessage::user(format!(
                    "[SYSTEM: `{name}` has failed {count} times with these exact arguments.{error_line} \
                     Retrying the identical call will keep failing. Change the arguments, \
                     fix the underlying cause, or take a different approach.]"
                )));
            }
        }
        if self.tracker.is_clearly_stuck() {
            if !self.stuck_warned {
                self.stuck_warned = true;
                messages.push(ChatMessage::user("[SYSTEM: You appear to be repeating similar actions without progress. Step back, re-read the full context, and try a different approach. If you cannot make progress, use the `respond` tool to summarize what you've accomplished and what remains.]".into()));
            } else if self.tracker.score <= -12 {
                return Err(ExecutorError::MaxIterationsNeedsIntervention {
                    iterations_used: turn,
                    reason: self.tracker.diagnosis(),
                });
            }
        }
        if cfg.input_budget > 0
            && self.prompt_tokens > cfg.input_budget.saturating_mul(cfg.warn_pct) / 100
            && !self.context_warned
        {
            self.context_warned = true;
            // Unlike the old continue, this warning reaches a real model turn.
            messages.push(ChatMessage::system("[system] Context is getting full. Save important facts with memory(action=\"save_fact\", scope=\"chat\") before they are pruned.".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_usage_retains_last_prompt_measurement_not_cumulative_total() {
        let mut policy = ProgressPolicy::default();
        policy.usage(Some(400));
        policy.usage(Some(300));
        policy.usage(None);
        assert_eq!(policy.prompt_tokens, 300);
    }

    fn policy_with_failure() -> ProgressPolicy {
        let mut policy = ProgressPolicy::default();
        let args = serde_json::json!({"url": "https://api.example.com/x"});
        policy.tool("fetch", &args, Some("connection refused"));
        policy.tool("fetch", &args, Some("connection refused"));
        policy
    }

    fn cfg() -> ProgressConfig {
        ProgressConfig {
            turn_budget: 0,
            max_turns: 100,
            complexity: None,
            input_budget: 0,
            warn_pct: 80,
        }
    }

    #[test]
    fn repeated_failure_injects_feedback_nudge() {
        let mut policy = policy_with_failure();
        let mut messages = Vec::new();
        policy.prepare(&cfg(), &mut messages).expect("prepare");
        let text: Vec<String> = messages.iter().map(|m| m.text_content()).collect();
        let nudged = text
            .iter()
            .any(|t| t.contains("has failed 2 times") && t.contains("connection refused"));
        assert!(nudged, "nudge should name the call and the error: {text:?}");
    }

    #[test]
    fn failure_nudge_fires_once_per_call() {
        let mut policy = policy_with_failure();
        let mut first = Vec::new();
        policy.prepare(&cfg(), &mut first).expect("prepare");
        let mut second = Vec::new();
        policy.prepare(&cfg(), &mut second).expect("prepare");
        assert!(first
            .iter()
            .any(|m| m.text_content().contains("has failed 2 times")));
        assert!(
            !second
                .iter()
                .any(|m| m.text_content().contains("has failed 2 times")),
            "same count must not re-nudge"
        );
    }
}
