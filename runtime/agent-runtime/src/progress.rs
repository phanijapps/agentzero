// ============================================================================
// PROGRESS TRACKER MODULE
// Tracks execution progress to distinguish productive work from stuck loops.
// ============================================================================

use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

/// Tracks execution progress to distinguish productive work from stuck loops.
///
/// Scores each iteration based on tool diversity, success rate, and
/// repetition patterns. Drives stuck-detection nudges and planning nudges.
pub(crate) struct ProgressTracker {
    /// Recent tool calls as (name, `args_hash`) for repetition detection
    pub(crate) recent_tool_calls: VecDeque<(String, u64)>,
    /// Recent error messages for repeated-error detection
    pub(crate) recent_errors: VecDeque<String>,
    /// Unique tool names used during this scoring window
    pub(crate) unique_tools_used: HashSet<String>,
    /// Cumulative progress score for the current window
    pub(crate) score: i32,
    /// Total iterations consumed across all windows
    pub(crate) total_iterations: u32,
    /// Rolling window of tool names (last 20 calls) for diversity tracking
    pub(crate) tool_name_window: VecDeque<String>,
    /// Count of tool calls in current scoring window (for periodic diversity scoring)
    pub(crate) window_tool_calls: u32,
    /// Whether the agent has created a plan via update_plan.
    pub(crate) has_plan: bool,
    /// Number of plan items the agent has added.
    pub(crate) plan_items_created: u32,
    /// Number of plan items completed via update_plan.
    pub(crate) plan_items_completed: u32,
    /// Whether the planning nudge has been injected (max 1)
    pub(crate) planning_nudge_sent: bool,
    /// Non-planning tool calls made before first update_plan.
    pub(crate) tool_calls_before_plan: u32,
    /// Per-path count of `write_file` overwrites. Catches the "rewrite the
    /// same file over and over" loop — each rewrite carries different
    /// content (so the args-hash repeat check misses it) and every write
    /// succeeds (so the failed-call diversity check misses it too). That
    /// loop shape burned 600K-1.8M tokens per runaway builder.
    pub(crate) write_target_repeats: HashMap<String, u32>,
}

impl ProgressTracker {
    pub(crate) fn new() -> Self {
        Self {
            recent_tool_calls: VecDeque::with_capacity(10),
            recent_errors: VecDeque::with_capacity(5),
            unique_tools_used: HashSet::new(),
            score: 0,
            total_iterations: 0,
            tool_name_window: VecDeque::with_capacity(20),
            window_tool_calls: 0,
            has_plan: false,
            plan_items_created: 0,
            plan_items_completed: 0,
            planning_nudge_sent: false,
            tool_calls_before_plan: 0,
            write_target_repeats: HashMap::new(),
        }
    }

    fn hash_args(args: &Value) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let s = serde_json::to_string(args).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    /// Record a tool call and update the progress score.
    pub(crate) fn record_tool_call(&mut self, name: &str, args: &Value, succeeded: bool) {
        // Planning enforcement: detect update_plan usage.
        if name == "update_plan" && succeeded {
            // update_plan uses {plan: [{step, status}]} — lightweight, fire-and-forget.
            if let Some(plan) = args.get("plan").and_then(|v| v.as_array()) {
                let step_count = plan.len() as u32;
                let completed_count = plan
                    .iter()
                    .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("completed"))
                    .count() as u32;
                if !self.has_plan {
                    self.plan_items_created = step_count;
                    self.has_plan = true;
                    self.score += 3 + step_count.min(5) as i32;
                }
                // Reward completed steps.
                if completed_count > self.plan_items_completed {
                    let new_completions = completed_count - self.plan_items_completed;
                    self.plan_items_completed = completed_count;
                    self.score += (new_completions * 2) as i32;
                }
            }
        }

        if !self.has_plan && name != "update_plan" {
            self.tool_calls_before_plan += 1;
        }

        let args_hash = Self::hash_args(args);

        // Exact repetition detection — same tool+args in last 5 calls
        // Only penalize FAILED exact repeats. Successful calls with same args
        // are legitimate workflow patterns (e.g. polling, iterating a task list).
        let is_exact_repeat = self
            .recent_tool_calls
            .iter()
            .any(|(n, h)| n == name && *h == args_hash);
        if is_exact_repeat && !succeeded {
            self.score -= 3;
        }

        // Same-file rewrite loop. A builder that overwrites the SAME path
        // again and again — each time with slightly different content
        // (shrinking a file to fit a size gate, re-fixing its own output) —
        // evades both guards above: the args-hash repeat check (content
        // differs => hash differs) and the failed-call diversity check
        // (every write succeeds). This is the exact shape of the runaway
        // builders that burned 600K-1.8M tokens. 3 free overwrites cover
        // normal write→test→fix debugging; the 4th+ overwrite of one path
        // takes a hard penalty so `score` crosses the stuck thresholds
        // (-5 advisory nudge, -12 hard stop) within ~2 more iterations.
        if name == "write_file" {
            if let Some(path) = args
                .get("path")
                .or_else(|| args.get("file_path"))
                .and_then(Value::as_str)
            {
                let n = self
                    .write_target_repeats
                    .entry(path.to_string())
                    .or_insert(0);
                *n += 1;
                if *n >= 4 {
                    self.score -= 10;
                }
            }
        }

        // Tool diversity scoring via rolling window
        // Only track FAILED calls for diversity scoring. Subagents with a
        // narrow toolset (shell, write_file, load_skill, respond) naturally
        // have low diversity ratios even when productive. Penalizing low
        // diversity on successful calls would kill legitimate iterative workflows.
        if !succeeded {
            self.tool_name_window.push_back(name.to_string());
            if self.tool_name_window.len() > 20 {
                self.tool_name_window.pop_front();
            }
        }

        // Score diversity every 10 FAILED calls (not total calls)
        self.window_tool_calls += 1;
        if !succeeded
            && self.tool_name_window.len() >= 10
            && self.tool_name_window.len().is_multiple_of(5)
        {
            let distinct: HashSet<&str> = self
                .tool_name_window
                .iter()
                .map(std::string::String::as_str)
                .collect();
            let ratio = distinct.len() as f32 / self.tool_name_window.len() as f32;

            if ratio <= 0.15 {
                // Same tool failing repeatedly — definitely stuck
                self.score -= 8;
            } else if ratio <= 0.25 {
                self.score -= 3;
            }
            // No positive score for diversity — success bonus handles that
        }

        // First-ever use of a tool gets a small bonus
        if self.unique_tools_used.insert(name.to_string()) {
            self.score += 1;
        }

        // Successful tool calls get a small bonus to offset any accidental penalties.
        // This keeps productive agents alive. Stuck agents still die because
        // failures accumulate penalties faster than successes add bonuses.
        if succeeded {
            self.score += 1;
        }

        // Track for exact-repetition detection (keep last 5)
        self.recent_tool_calls
            .push_back((name.to_string(), args_hash));
        if self.recent_tool_calls.len() > 5 {
            self.recent_tool_calls.pop_front();
        }
    }

    /// Record a tool error for repeated-error detection.
    pub(crate) fn record_error(&mut self, error: &str) {
        // Check if this exact error appeared 3+ times recently
        let repeat_count = self
            .recent_errors
            .iter()
            .filter(|e| e.as_str() == error)
            .count();
        if repeat_count >= 2 {
            self.score -= 5; // Definitely stuck
        }

        self.recent_errors.push_back(error.to_string());
        if self.recent_errors.len() > 5 {
            self.recent_errors.pop_front();
        }
    }

    /// Record that a respond action was emitted — agent is finishing.
    pub(crate) fn record_respond(&mut self) {
        self.score += 10;
    }

    /// Record one iteration consumed.
    pub(crate) fn tick(&mut self) {
        self.total_iterations += 1;
    }

    /// Check if the agent is clearly stuck and should stop early (before window boundary).
    /// Returns true if score has gone negative after at least 10 tool calls in this window.
    /// Threshold lowered from 15/-10 to 10/-5 because success bonus was removed.
    pub(crate) fn is_clearly_stuck(&self) -> bool {
        self.window_tool_calls >= 10 && self.score <= -5
    }

    /// Returns true once when agent should be nudged to create a plan.
    pub(crate) fn needs_planning_nudge(&mut self) -> bool {
        if !self.has_plan && !self.planning_nudge_sent && self.tool_calls_before_plan >= 5 {
            self.planning_nudge_sent = true;
            true
        } else {
            false
        }
    }

    /// Build a human-readable diagnosis of the current state.
    pub(crate) fn diagnosis(&self) -> String {
        let plan_status = if self.has_plan {
            format!(
                ", plan: {}/{} items done",
                self.plan_items_completed, self.plan_items_created
            )
        } else {
            ", no plan created".to_string()
        };

        if self.score <= -10 {
            format!(
                "Stuck in loop: {} repeated tool calls detected (score: {}){}",
                self.recent_tool_calls.len(),
                self.score,
                plan_status
            )
        } else if self.score <= 0 {
            format!(
                "No progress detected after {} iterations (score: {}){}",
                self.total_iterations, self.score, plan_status
            )
        } else {
            format!(
                "Making progress: {} unique tools used (score: {}){}",
                self.unique_tools_used.len(),
                self.score,
                plan_status
            )
        }
    }

}

#[cfg(test)]
mod progress_tracker_tests {
    use super::*;
    use crate::context_management::compact_messages;
    use crate::engine::ExecutorConfig;
    use crate::types::ChatMessage;
    use agent_primitives::types::Part;
    use serde_json::json;

    #[test]
    fn test_diagnosis_stuck() {
        let mut tracker = ProgressTracker::new();
        let args = json!({"path": "/same"});
        for _ in 0..6 {
            tracker.record_tool_call("read", &args, false);
        }
        let diagnosis = tracker.diagnosis();
        assert!(
            diagnosis.contains("loop") || diagnosis.contains("No progress"),
            "Got: {diagnosis}"
        );
    }

    #[test]
    fn test_diagnosis_progress() {
        let mut tracker = ProgressTracker::new();
        // Use enough diverse tools to stay positive
        tracker.record_tool_call("read", &json!({}), true);
        tracker.record_tool_call("write", &json!({}), true);
        tracker.record_tool_call("shell", &json!({}), true);
        let diagnosis = tracker.diagnosis();
        assert!(diagnosis.contains("progress"), "Got: {diagnosis}");
    }

    #[test]
    fn test_executor_config_defaults() {
        let config = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.max_extensions, 3);
        assert_eq!(config.extension_size, 25);
        assert_eq!(config.turn_budget, 25);
        assert_eq!(config.max_turns, 50);
    }

    #[test]
    fn test_low_diversity_loop_detected() {
        let mut tracker = ProgressTracker::new();
        // Simulate write+shell loop for 20 iterations, all failed (different args each time)
        for i in 0..20 {
            let tool = if i % 2 == 0 { "write" } else { "shell" };
            tracker.record_tool_call(tool, &json!({"i": i}), false);
        }
        // After 20 failed calls:
        // 2 unique tools (+1 each = +2)
        // At 10 failed calls: diversity = 2/10 = 0.20 <= 0.25 → -3
        // At 15 failed calls: diversity = 2/15 = 0.13 <= 0.15 → -8
        // At 20 failed calls: diversity = 2/20 = 0.10 <= 0.15 → -8
        // Total: +2 - 3 - 8 - 8 = -17
        assert!(
            tracker.score < 0,
            "Low-diversity loop should have negative score, got: {}",
            tracker.score
        );
    }

    #[test]
    fn test_high_diversity_extends() {
        let mut tracker = ProgressTracker::new();
        // Use 10 unique tools in 10 calls (all succeed)
        let tools = [
            "read",
            "write",
            "shell",
            "edit",
            "glob",
            "memory",
            "update_plan",
            "ward",
            "respond",
            "load_skill",
        ];
        for (i, tool) in tools.iter().enumerate() {
            tracker.record_tool_call(tool, &json!({"i": i}), true);
        }
        // 10 unique tools: +1 each = 10, +1 success each = 10
        // No diversity check (only tracks failed calls, none here)
        // Total: 10 + 10 = 20
        assert!(
            tracker.score > 0,
            "High diversity should produce positive score, got: {}",
            tracker.score
        );
    }

    #[test]
    fn test_early_stop_deeply_stuck() {
        let mut tracker = ProgressTracker::new();
        let args = json!({"path": "/same"});
        // Same exact tool+args repeated, all failed — triggers repetition and diversity penalties
        // Call 1: +1 (unique) = 1
        // Calls 2-20: -3 (repeat) each = -57
        // At 10 failed calls: diversity = 1/10 ≤ 0.15 → -8
        // At 15 failed calls: diversity = 1/15 ≤ 0.15 → -8
        // At 20 failed calls: diversity = 1/20 ≤ 0.15 → -8
        // Total: 1 - 57 - 8 - 8 - 8 = -80
        for _ in 0..20 {
            tracker.record_tool_call("read", &args, false);
        }
        // With 10+ window_tool_calls and deeply negative score, should be stuck
        assert!(
            tracker.window_tool_calls >= 10,
            "Should have 20 window_tool_calls, got: {}",
            tracker.window_tool_calls
        );
        assert!(
            tracker.score <= -5,
            "Score should be <= -5 with exact-repeat loop, got: {}",
            tracker.score
        );
        assert!(
            tracker.is_clearly_stuck(),
            "Should be clearly stuck with score {} after {} calls",
            tracker.score,
            tracker.window_tool_calls
        );
    }

    // ========================================================================
    // PLANNING ENFORCEMENT TESTS
    // ========================================================================

    #[test]
    fn test_update_plan_sets_has_plan() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "pending"}]}),
            true,
        );
        assert!(tracker.has_plan);
        assert_eq!(tracker.plan_items_created, 1);
    }

    #[test]
    fn test_update_plan_counts_items() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [
                {"step": "step 1", "status": "pending"},
                {"step": "step 2", "status": "pending"},
                {"step": "step 3", "status": "pending"}
            ]}),
            true,
        );
        assert!(tracker.has_plan);
        assert_eq!(tracker.plan_items_created, 3);
    }

    #[test]
    fn test_update_plan_boosts_score() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [
                {"step": "step 1", "status": "pending"},
                {"step": "step 2", "status": "pending"}
            ]}),
            true,
        );
        // +3 base + 2 items + 1 unique tool + 1 success = 7
        assert_eq!(tracker.score, 7);
    }

    #[test]
    fn test_update_plan_completed_boosts_score() {
        let mut tracker = ProgressTracker::new();
        // First add a plan so we have context.
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "pending"}]}),
            true,
        );
        let score_after_add = tracker.score;
        // Complete the item.
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "completed"}]}),
            true,
        );
        // +2 completion bonus + 1 success (unique tool bonus already used)
        assert_eq!(tracker.score, score_after_add + 3);
        assert_eq!(tracker.plan_items_completed, 1);
    }

    #[test]
    fn test_update_plan_incomplete_no_completion_bonus() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "pending"}]}),
            true,
        );
        // +3 plan base + 1 item + 1 unique tool + 1 success = 6, no completion bonus.
        assert_eq!(tracker.score, 6);
        assert_eq!(tracker.plan_items_completed, 0);
    }

    #[test]
    fn test_failed_update_plan_not_counted() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "pending"}]}),
            false,
        );
        assert!(!tracker.has_plan);
        assert_eq!(tracker.plan_items_created, 0);
    }

    #[test]
    fn test_tool_calls_before_plan_counted() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call("read", &json!({"path": "/a"}), true);
        tracker.record_tool_call("write", &json!({"path": "/b"}), true);
        assert_eq!(tracker.tool_calls_before_plan, 2);

        // Create plan
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "pending"}]}),
            true,
        );
        assert_eq!(tracker.tool_calls_before_plan, 2); // Frozen

        // More tool calls after plan — counter should not increase
        tracker.record_tool_call("shell", &json!({"cmd": "ls"}), true);
        assert_eq!(tracker.tool_calls_before_plan, 2);
    }

    #[test]
    fn test_needs_planning_nudge_at_threshold() {
        let mut tracker = ProgressTracker::new();
        for i in 0..5 {
            tracker.record_tool_call("read", &json!({"path": format!("/{}", i)}), true);
        }
        assert_eq!(tracker.tool_calls_before_plan, 5);
        assert!(tracker.needs_planning_nudge());
    }

    #[test]
    fn test_needs_planning_nudge_only_once() {
        let mut tracker = ProgressTracker::new();
        for i in 0..6 {
            tracker.record_tool_call("read", &json!({"path": format!("/{}", i)}), true);
        }
        assert!(tracker.needs_planning_nudge());
        assert!(
            !tracker.needs_planning_nudge(),
            "Nudge should fire only once"
        );
    }

    #[test]
    fn test_no_nudge_if_plan_exists() {
        let mut tracker = ProgressTracker::new();
        // Create plan first.
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [{"step": "step 1", "status": "pending"}]}),
            true,
        );
        // Then do 10 tool calls
        for i in 0..10 {
            tracker.record_tool_call("read", &json!({"path": format!("/{}", i)}), true);
        }
        assert!(!tracker.needs_planning_nudge());
    }

    #[test]
    fn test_diagnosis_includes_plan_status() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [
                {"step": "a", "status": "pending"},
                {"step": "b", "status": "pending"}
            ]}),
            true,
        );
        tracker.record_tool_call(
            "update_plan",
            &json!({"plan": [
                {"step": "a", "status": "completed"},
                {"step": "b", "status": "pending"}
            ]}),
            true,
        );
        let diagnosis = tracker.diagnosis();
        assert!(
            diagnosis.contains("plan: 1/2 items done"),
            "Expected plan status in diagnosis, got: {diagnosis}"
        );
    }

    #[test]
    fn test_diagnosis_shows_no_plan() {
        let mut tracker = ProgressTracker::new();
        tracker.record_tool_call("read", &json!({}), true);
        let diagnosis = tracker.diagnosis();
        assert!(
            diagnosis.contains("no plan created"),
            "Expected 'no plan created' in diagnosis, got: {diagnosis}"
        );
    }

    // ========================================================================
    // STUCK DETECTION THRESHOLD TESTS (post-deflation)
    // ========================================================================

    #[test]
    fn test_is_clearly_stuck_requires_10_calls() {
        let mut tracker = ProgressTracker::new();
        let args = json!({"path": "/same"});
        // 9 repeated failed calls — not enough window_tool_calls to trigger
        for _ in 0..9 {
            tracker.record_tool_call("read", &args, false);
        }
        assert!(
            !tracker.is_clearly_stuck(),
            "Should not be stuck with only {} calls (need 10), score: {}",
            tracker.window_tool_calls,
            tracker.score
        );
        // 10th call pushes over the threshold
        tracker.record_tool_call("read", &args, false);
        assert!(
            tracker.is_clearly_stuck(),
            "Should be stuck at {} calls with score {}",
            tracker.window_tool_calls,
            tracker.score
        );
    }

    #[test]
    fn test_safety_valve_at_negative_12() {
        let mut tracker = ProgressTracker::new();
        let args = json!({"path": "/same"});
        for _ in 0..15 {
            tracker.record_tool_call("read", &args, false);
        }
        // Score: +1(unique) - 14*3(repeats) - 8(div@10) - 8(div@15) = 1-42-8-8 = -57
        assert!(
            tracker.score <= -12,
            "Score should be <= -12 after 15 exact repeats, got: {}",
            tracker.score
        );
    }

    // ========================================================================
    // SAME-FILE REWRITE LOOP DETECTION
    // ========================================================================

    #[test]
    fn same_path_rewrite_loop_trips_stuck() {
        // The runaway-builder shape: overwrite ONE file again and again,
        // each time with different content, every write succeeding. The
        // old scoring rewarded every success (+1) and never tripped — this
        // is the regression that let builders burn 600K-1.8M tokens.
        let mut tracker = ProgressTracker::new();
        for i in 0..10 {
            // Different content each call → different args hash → the
            // exact-repeat guard does NOT fire. Same path → our new guard does.
            let args = json!({"path": "report.html", "content": format!("v{i}")});
            tracker.record_tool_call("write_file", &args, true);
        }
        assert!(
            tracker.is_clearly_stuck(),
            "10 overwrites of the same file must register as stuck (score: {})",
            tracker.score
        );
    }

    #[test]
    fn distinct_path_writes_do_not_trip_stuck() {
        // Writing 10 DIFFERENT files is productive work, not a loop. The
        // per-path penalty must not fire — no path is overwritten 4+ times.
        let mut tracker = ProgressTracker::new();
        for i in 0..10 {
            let args = json!({"path": format!("file_{i}.py"), "content": "x"});
            tracker.record_tool_call("write_file", &args, true);
        }
        assert!(
            !tracker.is_clearly_stuck(),
            "writing 10 distinct files is productive, not stuck (score: {})",
            tracker.score
        );
    }

    #[test]
    fn three_rewrites_of_same_file_still_allowed() {
        // 3 overwrites = normal write→test→fix debugging. The 4th is where
        // the penalty kicks in, so 3 must stay penalty-free.
        let mut tracker = ProgressTracker::new();
        for i in 0..3 {
            let args = json!({"path": "x.py", "content": format!("v{i}")});
            tracker.record_tool_call("write_file", &args, true);
        }
        assert!(
            tracker.score > 0,
            "3 overwrites must not be penalized (score: {})",
            tracker.score
        );
    }

    // ========================================================================
    // COMPACTION TESTS
    // ========================================================================

    #[test]
    fn test_compact_messages_preserves_original_request() {
        let mut messages = Vec::new();
        // System message
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: vec![Part::Text {
                text: "You are an assistant.".to_string(),
            }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        });
        // Original user request
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: vec![Part::Text {
                text: "Build a trinomial cheat sheet.".to_string(),
            }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        });
        // Add 30 filler messages so compaction kicks in
        for i in 0..30 {
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: format!("Step {i}"),
                }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            });
        }

        let compacted = compact_messages(messages);

        // Should contain: system + original user request + compaction notice + last 20
        assert!(
            compacted
                .iter()
                .any(|m| m.text_content().contains("trinomial cheat sheet")),
            "Compacted messages should preserve the original user request"
        );
        assert!(
            compacted
                .iter()
                .any(|m| m.text_content().contains("Context compacted")),
            "Compacted messages should include compaction notice"
        );
        assert!(
            compacted
                .iter()
                .any(|m| m.text_content().contains("original request")),
            "Compaction notice should reference the preserved original request"
        );
    }

    #[test]
    fn test_compact_messages_no_op_when_short() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: vec![Part::Text {
                    text: "system".to_string(),
                }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "user".to_string(),
                content: vec![Part::Text {
                    text: "hello".to_string(),
                }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            },
        ];
        let compacted = compact_messages(messages.clone());
        assert_eq!(compacted.len(), messages.len());
    }

    // ========================================================================
    // E2E-STYLE LOOP DETECTOR TESTS (moved from e2e_ward_pipeline_tests)
    // ========================================================================

    /// Successful tool calls should not tank the progress score.
    #[test]
    fn test_loop_detector_productive_agent_survives() {
        let mut tracker = ProgressTracker::new();

        // Simulate a productive iterative workflow:
        // shell(get task) -> write_file(create file) -> shell(verify) -> shell(mark done)
        // All successful — score should stay positive
        for i in 0..5 {
            tracker.record_tool_call(
                "shell",
                &json!({"command": format!("get_task {}", i)}),
                true,
            );
            tracker.record_tool_call(
                "write_file",
                &json!({"path": format!("core/mod{}.py", i)}),
                true,
            );
            tracker.record_tool_call(
                "shell",
                &json!({"command": format!("python3 -c 'import core.mod{}'", i)}),
                true,
            );
            tracker.record_tool_call(
                "shell",
                &json!({"command": format!("mark_done {}", i)}),
                true,
            );
        }

        assert!(
            !tracker.is_clearly_stuck(),
            "Productive agent with 20 successful calls should NOT be stuck. Score: {}",
            tracker.score
        );
        assert!(
            tracker.score > 0,
            "Score should be positive for productive work, got: {}",
            tracker.score
        );
    }

    /// Failed repeated tool calls should tank the score.
    #[test]
    fn test_loop_detector_stuck_agent_dies() {
        let mut tracker = ProgressTracker::new();

        // Simulate a stuck agent: same shell command failing repeatedly
        for _ in 0..15 {
            tracker.record_tool_call("shell", &json!({"command": "cat nonexistent.py"}), false);
        }

        assert!(
            tracker.is_clearly_stuck(),
            "Agent with 15 repeated failures should be stuck. Score: {}",
            tracker.score
        );
    }

    /// Mixed success/failure: productive work with occasional errors should survive.
    #[test]
    fn test_loop_detector_mixed_survives() {
        let mut tracker = ProgressTracker::new();

        // 8 successes, 2 failures — should be fine
        for i in 0..10 {
            let succeeded = i % 5 != 3; // fail on iteration 3 and 8
            tracker.record_tool_call(
                if i % 2 == 0 { "shell" } else { "write_file" },
                &json!({"arg": format!("call_{}", i)}),
                succeeded,
            );
        }

        assert!(
            !tracker.is_clearly_stuck(),
            "Agent with 80% success rate should NOT be stuck. Score: {}",
            tracker.score
        );
    }
}
