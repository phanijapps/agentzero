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
}
impl Default for ProgressPolicy {
    fn default() -> Self {
        Self {
            tracker: ProgressTracker::new(0),
            prompt_tokens: 0,
            turn_warned: false,
            context_warned: false,
            stuck_warned: false,
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
        self.tracker.record_tool_call(name, args, error.is_none());
        if let Some(error) = error.filter(|error| *error != "blocked_by_hook") {
            self.tracker.record_error(error);
        }
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
}
