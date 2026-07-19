// ============================================================================
// MIDDLEWARE TRAITS
// Core traits for middleware implementation
// ============================================================================

//! # Middleware Traits
//!
//! Core traits for middleware implementation.

use std::collections::HashMap;

use crate::types::{ChatMessage, StreamEvent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// EXECUTION STATE
// State passed from the executor to middleware for context-aware processing
// ============================================================================

/// Information about a loaded skill for middleware consumption
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillInfo {
    /// Name of the skill
    pub name: String,
    /// Tool call ID when this skill's SKILL.md was loaded
    pub tool_call_id: String,
    /// Tool call IDs for all resources loaded under this skill
    pub resource_tool_call_ids: Vec<String>,
}

/// Execution state passed to middleware.
///
/// Contains information about the current execution that middleware
/// can use to make context-aware decisions (e.g., skill-aware compaction).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExecutionState {
    /// Currently loaded skills with their `tool_call_ids`.
    /// Key is skill name, value is skill info.
    pub loaded_skills: HashMap<String, SkillInfo>,
}

impl ExecutionState {
    /// Build execution state by scanning conversation messages for skill-related tool calls.
    ///
    /// This extracts skill information from the message history, allowing middleware
    /// to identify which tool results are skill loads vs regular tool calls.
    #[must_use]
    pub fn from_messages(messages: &[crate::types::ChatMessage]) -> Self {
        let mut loaded_skills: HashMap<String, SkillInfo> = HashMap::new();

        // Scan for assistant messages with tool calls
        for (idx, message) in messages.iter().enumerate() {
            if let Some(tool_calls) = &message.tool_calls {
                for tool_call in tool_calls {
                    if tool_call.name == "load_skill" {
                        // Extract skill name from arguments
                        if let Some(skill_name) =
                            tool_call.arguments.get("skill").and_then(|v| v.as_str())
                        {
                            // This is a main skill load
                            let entry =
                                loaded_skills
                                    .entry(skill_name.to_string())
                                    .or_insert_with(|| SkillInfo {
                                        name: skill_name.to_string(),
                                        tool_call_id: tool_call.id.clone(),
                                        resource_tool_call_ids: vec![],
                                    });
                            // Update tool_call_id if this is a newer load
                            entry.tool_call_id = tool_call.id.clone();
                        } else if let Some(file_path) =
                            tool_call.arguments.get("file").and_then(|v| v.as_str())
                        {
                            // This is a resource file load - try to extract skill name
                            let skill_name =
                                Self::extract_skill_from_file_arg(file_path, messages, idx);
                            if let Some(name) = skill_name {
                                let entry =
                                    loaded_skills.entry(name.clone()).or_insert_with(|| {
                                        SkillInfo {
                                            name,
                                            tool_call_id: String::new(),
                                            resource_tool_call_ids: vec![],
                                        }
                                    });
                                entry.resource_tool_call_ids.push(tool_call.id.clone());
                            }
                        }
                    }
                }
            }
        }

        Self { loaded_skills }
    }

    /// Extract skill name from a file argument.
    /// Handles formats like "@skill:skill-name/path" or uses current skill context.
    fn extract_skill_from_file_arg(
        file_path: &str,
        _messages: &[crate::types::ChatMessage],
        _current_idx: usize,
    ) -> Option<String> {
        if let Some(path) = file_path.strip_prefix("@skill:") {
            // Skip "@skill:"
            if path.contains('/') {
                let parts: Vec<&str> = path.splitn(2, '/').collect();
                return Some(parts[0].to_string());
            }
            // Just skill name without path
            let has_extension = [
                ".md", ".txt", ".json", ".yaml", ".yml", ".toml", ".rs", ".py", ".js", ".ts",
            ]
            .iter()
            .any(|ext| path.ends_with(ext));
            if !has_extension {
                return Some(path.to_string());
            }
        }
        // TODO: Could search backwards through messages to find current skill context
        None
    }
}

/// Context passed to middleware during execution
#[derive(Clone, Debug)]
pub struct MiddlewareContext {
    /// Agent ID
    pub agent_id: String,
    /// Conversation ID (if available)
    pub conversation_id: Option<String>,
    /// Provider ID
    pub provider_id: String,
    /// Model name
    pub model: String,
    /// Current message count in conversation
    pub message_count: usize,
    /// Estimated token count
    pub estimated_tokens: usize,
    /// Additional metadata
    pub metadata: Value,
    /// Execution state from the tool context (skills, etc.)
    /// This allows middleware to make skill-aware decisions during compaction.
    pub execution_state: ExecutionState,
    /// Current `app:plan` session state, if the agent has called
    /// `update_plan` during this session. Shape:
    /// `{ explanation?: String, plan: [{ step, status }] }`. Populated by
    /// the executor from `ctx.get_state("app:plan")` when building the
    /// context. `None` when no plan exists yet.
    ///
    /// Consumed by the plan-block middleware to inject a pinned
    /// structured anchor just after the system prompt — the anchor
    /// survives context-editing compaction so long-running tool loops
    /// don't lose sight of the goal + checklist.
    pub plan_state: Option<Value>,
}

impl MiddlewareContext {
    /// Create a new middleware context
    #[must_use]
    pub fn new(
        agent_id: String,
        conversation_id: Option<String>,
        provider_id: String,
        model: String,
    ) -> Self {
        Self {
            agent_id,
            conversation_id,
            provider_id,
            model,
            message_count: 0,
            estimated_tokens: 0,
            metadata: Value::Object(Default::default()),
            execution_state: ExecutionState::default(),
            plan_state: None,
        }
    }

    /// Set the current plan state (what the agent's last `update_plan`
    /// call produced). Populated by the executor before running
    /// middleware, consumed by [`super::plan_block::PlanBlockMiddleware`]
    /// to render a pinned anchor.
    #[must_use]
    pub fn with_plan_state(mut self, plan_state: Option<Value>) -> Self {
        self.plan_state = plan_state;
        self
    }

    /// Set message and token counts
    #[must_use]
    pub fn with_counts(mut self, message_count: usize, estimated_tokens: usize) -> Self {
        self.message_count = message_count;
        self.estimated_tokens = estimated_tokens;
        self
    }

    /// Set additional metadata
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set execution state (loaded skills, etc.)
    ///
    /// This allows middleware to access skill information for context-aware
    /// processing, such as leaving meaningful placeholders when compacting
    /// skill-related tool results.
    #[must_use]
    pub fn with_execution_state(mut self, execution_state: ExecutionState) -> Self {
        self.execution_state = execution_state;
        self
    }
}

/// Effect returned by preprocess middleware
#[derive(Debug)]
pub enum MiddlewareEffect {
    /// Continue with modified messages
    ModifiedMessages(Vec<ChatMessage>),
    /// Continue without modification
    Proceed,
    /// Emit an event and continue
    EmitEvent(StreamEvent),
    /// Emit event AND modify messages
    EmitAndModify {
        event: StreamEvent,
        messages: Vec<ChatMessage>,
    },
}

/// Trait for middleware that pre-processes messages before LLM execution
///
/// Implement this trait for middleware that:
/// - Summarizes conversation history
/// - Edits context (removes old tool outputs)
/// - Filters/transforms messages
/// - Validates/limits input
///
/// Note: This trait uses a different approach to avoid dyn-compatibility issues.
/// Middleware returns events in the effect rather than calling callbacks.
#[async_trait]
pub trait PreProcessMiddleware: Send + Sync {
    /// Get the unique name of this middleware
    fn name(&self) -> &'static str;

    /// Clone the middleware (needed for enum wrapper)
    fn clone_box(&self) -> Box<dyn PreProcessMiddleware>;

    /// Process messages before they are sent to the LLM
    ///
    /// # Arguments
    /// * `messages` - Current conversation messages
    /// * `context` - Execution context with metadata
    ///
    /// # Returns
    /// * `MiddlewareEffect` - The effect to apply (modify messages, emit event, etc.)
    async fn process(
        &self,
        messages: Vec<ChatMessage>,
        context: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String>;

    /// Whether this middleware is enabled
    fn enabled(&self) -> bool {
        true
    }
}

/// Trait for middleware that reacts to events during execution
///
/// Implement this trait for middleware that:
/// - Logs/traces execution
/// - Collects metrics
/// - Implements rate limiting
/// - Detects PII
/// - Builds todo lists
///
/// Note: This trait is NOT dyn-compatible due to async trait bounds.
/// Middleware must be stored as concrete types, not trait objects.
#[async_trait]
pub trait EventMiddleware: Send + Sync {
    /// Get the unique name of this middleware
    fn name(&self) -> &'static str;

    /// Clone the middleware (needed for enum wrapper)
    fn clone_box(&self) -> Box<dyn EventMiddleware>;

    /// Called when any stream event is emitted
    async fn on_event(
        &self,
        event: &StreamEvent,
        context: &MiddlewareContext,
    ) -> Result<(), String>;

    /// Whether this middleware is enabled
    fn enabled(&self) -> bool {
        true
    }
}

/// Helper trait for middleware that needs state
pub trait StatefulMiddleware {
    /// Get the current state as JSON
    fn get_state(&self) -> Result<Value, String>;

    /// Reset state (e.g., clear counters)
    fn reset(&mut self) -> Result<(), String>;
}

// Implement Clone for Box<dyn PreProcessMiddleware>
impl Clone for Box<dyn PreProcessMiddleware> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

// Implement Clone for Box<dyn EventMiddleware>
impl Clone for Box<dyn EventMiddleware> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;
    use agent_primitives::types::Part;
    use serde_json::json;

    fn user_msg(s: &str) -> ChatMessage {
        ChatMessage::user(s.to_string())
    }

    fn assistant_with_tool_calls(calls: Vec<ToolCall>) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: vec![Part::Text {
                text: String::new(),
            }],
            tool_calls: Some(calls),
            tool_call_id: None,
            is_summary: false,
        }
    }

    #[test]
    fn execution_state_default_has_no_skills() {
        let state = ExecutionState::default();
        assert!(state.loaded_skills.is_empty());
    }

    #[test]
    fn execution_state_from_empty_messages_is_empty() {
        let state = ExecutionState::from_messages(&[]);
        assert!(state.loaded_skills.is_empty());

        let only_user = vec![user_msg("hi")];
        let state2 = ExecutionState::from_messages(&only_user);
        assert!(state2.loaded_skills.is_empty());
    }

    #[test]
    fn execution_state_picks_up_load_skill_call() {
        let messages = vec![
            user_msg("please load"),
            assistant_with_tool_calls(vec![ToolCall::new(
                "call-load-1".to_string(),
                "load_skill".to_string(),
                json!({"skill": "my-skill"}),
            )]),
        ];
        let state = ExecutionState::from_messages(&messages);
        let info = state.loaded_skills.get("my-skill").expect("skill present");
        assert_eq!(info.name, "my-skill");
        assert_eq!(info.tool_call_id, "call-load-1");
        assert!(info.resource_tool_call_ids.is_empty());
    }

    #[test]
    fn execution_state_updates_tool_call_id_on_relead() {
        let messages = vec![
            assistant_with_tool_calls(vec![ToolCall::new(
                "first".to_string(),
                "load_skill".to_string(),
                json!({"skill": "alpha"}),
            )]),
            assistant_with_tool_calls(vec![ToolCall::new(
                "second".to_string(),
                "load_skill".to_string(),
                json!({"skill": "alpha"}),
            )]),
        ];
        let state = ExecutionState::from_messages(&messages);
        let info = state.loaded_skills.get("alpha").unwrap();
        assert_eq!(info.tool_call_id, "second"); // newer load overrides id
    }

    #[test]
    fn execution_state_resource_load_via_at_skill_path() {
        let messages = vec![assistant_with_tool_calls(vec![ToolCall::new(
            "resource-1".to_string(),
            "load_skill".to_string(),
            json!({"file": "@skill:beta/docs.md"}),
        )])];
        let state = ExecutionState::from_messages(&messages);
        let info = state.loaded_skills.get("beta").unwrap();
        assert_eq!(info.resource_tool_call_ids, vec!["resource-1".to_string()]);
        // tool_call_id stays empty when only a resource was loaded
        assert!(info.tool_call_id.is_empty());
    }

    #[test]
    fn execution_state_resource_load_via_at_skill_no_path() {
        let messages = vec![assistant_with_tool_calls(vec![ToolCall::new(
            "r".to_string(),
            "load_skill".to_string(),
            json!({"file": "@skill:gamma"}),
        )])];
        let state = ExecutionState::from_messages(&messages);
        assert!(state.loaded_skills.contains_key("gamma"));
    }

    #[test]
    fn execution_state_resource_load_with_extension_is_ignored() {
        // No "/" and ends with ".md" — extract_skill returns None, no entry created.
        let messages = vec![assistant_with_tool_calls(vec![ToolCall::new(
            "r".to_string(),
            "load_skill".to_string(),
            json!({"file": "@skill:something.md"}),
        )])];
        let state = ExecutionState::from_messages(&messages);
        assert!(state.loaded_skills.is_empty());
    }

    #[test]
    fn execution_state_ignores_non_load_skill_tool_calls() {
        let messages = vec![assistant_with_tool_calls(vec![ToolCall::new(
            "x".to_string(),
            "search".to_string(),
            json!({"skill": "ignored"}),
        )])];
        let state = ExecutionState::from_messages(&messages);
        assert!(state.loaded_skills.is_empty());
    }

    #[test]
    fn middleware_context_new_has_empty_metadata_and_default_state() {
        let ctx = MiddlewareContext::new(
            "agent-x".to_string(),
            Some("conv-1".to_string()),
            "openai".to_string(),
            "gpt-4o-mini".to_string(),
        );
        assert_eq!(ctx.agent_id, "agent-x");
        assert_eq!(ctx.conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(ctx.provider_id, "openai");
        assert_eq!(ctx.model, "gpt-4o-mini");
        assert_eq!(ctx.message_count, 0);
        assert_eq!(ctx.estimated_tokens, 0);
        assert!(ctx.plan_state.is_none());
        assert!(ctx.execution_state.loaded_skills.is_empty());
    }

    #[test]
    fn middleware_context_builders_set_fields() {
        let mut state = ExecutionState::default();
        state.loaded_skills.insert(
            "s".to_string(),
            SkillInfo {
                name: "s".to_string(),
                tool_call_id: "c".to_string(),
                resource_tool_call_ids: vec![],
            },
        );
        let ctx =
            MiddlewareContext::new("agent".to_string(), None, "p".to_string(), "m".to_string())
                .with_counts(7, 1234)
                .with_metadata(json!({"k": "v"}))
                .with_execution_state(state)
                .with_plan_state(Some(json!({"plan": []})));

        assert_eq!(ctx.message_count, 7);
        assert_eq!(ctx.estimated_tokens, 1234);
        assert_eq!(ctx.metadata.get("k").and_then(|v| v.as_str()), Some("v"));
        assert!(ctx.execution_state.loaded_skills.contains_key("s"));
        assert!(ctx.plan_state.is_some());
    }

    #[test]
    fn skill_info_default_is_empty() {
        let info = SkillInfo::default();
        assert!(info.name.is_empty());
        assert!(info.tool_call_id.is_empty());
        assert!(info.resource_tool_call_ids.is_empty());
    }

    // Exercise PreProcessMiddleware/EventMiddleware default `enabled()` and
    // Box<dyn ...>::clone via trivial concrete impls.

    struct Trivial;

    #[async_trait]
    impl PreProcessMiddleware for Trivial {
        fn name(&self) -> &'static str {
            "trivial"
        }
        fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
            Box::new(Trivial)
        }
        async fn process(
            &self,
            _msgs: Vec<ChatMessage>,
            _ctx: &MiddlewareContext,
        ) -> Result<MiddlewareEffect, String> {
            Ok(MiddlewareEffect::Proceed)
        }
    }

    struct TrivialEvent;

    #[async_trait]
    impl EventMiddleware for TrivialEvent {
        fn name(&self) -> &'static str {
            "evt"
        }
        fn clone_box(&self) -> Box<dyn EventMiddleware> {
            Box::new(TrivialEvent)
        }
        async fn on_event(&self, _e: &StreamEvent, _c: &MiddlewareContext) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn default_enabled_true_for_trivial() {
        let p: Box<dyn PreProcessMiddleware> = Box::new(Trivial);
        assert!(p.enabled());
        assert_eq!(p.name(), "trivial");
        let cloned = p.clone();
        assert_eq!(cloned.name(), "trivial");

        let e: Box<dyn EventMiddleware> = Box::new(TrivialEvent);
        assert!(e.enabled());
        let cloned_e = e.clone();
        assert_eq!(cloned_e.name(), "evt");
    }

    // StatefulMiddleware helper trait
    struct Stateful {
        counter: u32,
    }

    impl StatefulMiddleware for Stateful {
        fn get_state(&self) -> Result<Value, String> {
            Ok(json!({"counter": self.counter}))
        }
        fn reset(&mut self) -> Result<(), String> {
            self.counter = 0;
            Ok(())
        }
    }

    #[test]
    fn stateful_middleware_get_and_reset() {
        let mut s = Stateful { counter: 5 };
        let state = s.get_state().unwrap();
        assert_eq!(state.get("counter").and_then(|v| v.as_u64()), Some(5));
        s.reset().unwrap();
        assert_eq!(
            s.get_state().unwrap().get("counter").unwrap().as_u64(),
            Some(0)
        );
    }
}
