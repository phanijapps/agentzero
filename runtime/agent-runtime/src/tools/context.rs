// ============================================================================
// TOOL CONTEXT
// Execution context for tool operations
// ============================================================================

use agent_primitives::event::EventActions;
use agent_primitives::types::Content;
use agent_primitives::CallbackContext;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

/// Context for tool execution
///
/// Provides tools with necessary information about the execution environment
/// including conversation scoping and available resources.
///
/// Implements `agent_primitives::ToolContext` trait for compatibility with tools.
///
/// This context is designed to be shared across all tool calls in an execution loop
/// via `Arc<ToolContext>`. State set by one tool (e.g., loaded skills) persists and
/// can be accessed by subsequent tools and middleware.
pub struct ToolContext {
    /// Optional conversation ID for scoping file operations
    pub conversation_id: Option<String>,

    /// Skills available to the current agent (for `load_skill` tool)
    pub available_skills: Vec<String>,

    /// Agent ID for this execution
    pub agent_id: Option<String>,

    /// Function call ID for the current tool execution.
    /// Uses `RwLock` for interior mutability since context is shared via Arc.
    function_call_id: RwLock<String>,

    /// Key-value state storage
    state: RwLock<HashMap<String, Value>>,

    /// Event actions
    actions: RwLock<EventActions>,

    /// Static empty content for `user_content()`
    empty_content: Content,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            conversation_id: None,
            available_skills: Vec::new(),
            agent_id: None,
            function_call_id: RwLock::new(String::new()),
            state: RwLock::new(HashMap::new()),
            actions: RwLock::new(EventActions::default()),
            empty_content: Content {
                role: "user".to_string(),
                parts: vec![],
            },
        }
    }
}

impl ToolContext {
    /// Create a new empty context
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context with conversation ID
    #[must_use]
    pub fn with_conversation(conversation_id: String) -> Self {
        Self {
            conversation_id: Some(conversation_id),
            ..Default::default()
        }
    }

    /// Create a context with skills
    #[must_use]
    pub fn with_skills(available_skills: Vec<String>) -> Self {
        Self {
            available_skills,
            ..Default::default()
        }
    }

    /// Create a context with conversation and skills
    #[must_use]
    pub fn with_conversation_and_skills(
        conversation_id: String,
        available_skills: Vec<String>,
    ) -> Self {
        Self {
            conversation_id: Some(conversation_id),
            available_skills,
            ..Default::default()
        }
    }

    /// Create a context with agent ID
    #[must_use]
    pub fn with_agent_id(agent_id: String) -> Self {
        let mut state = HashMap::new();
        // Set agent_id in state for tools that need it (like memory tool)
        state.insert("app:agent_id".to_string(), Value::String(agent_id.clone()));
        Self {
            agent_id: Some(agent_id),
            state: RwLock::new(state),
            ..Default::default()
        }
    }

    /// Create a full context with all parameters
    #[must_use]
    pub fn full(
        agent_id: String,
        conversation_id: Option<String>,
        available_skills: Vec<String>,
    ) -> Self {
        Self::full_with_state(agent_id, conversation_id, available_skills, HashMap::new())
    }

    /// Create a full context with all parameters and initial state
    #[must_use]
    pub fn full_with_state(
        agent_id: String,
        conversation_id: Option<String>,
        available_skills: Vec<String>,
        initial_state: HashMap<String, Value>,
    ) -> Self {
        let mut state = initial_state;
        // Set agent_id in state for tools that need it (like memory tool)
        state.insert("app:agent_id".to_string(), Value::String(agent_id.clone()));
        if let Some(ref conv_id) = conversation_id {
            state.insert(
                "app:conversation_id".to_string(),
                Value::String(conv_id.clone()),
            );
        }
        // Also store agent_id and conversation_id without prefix for tools like respond/delegate
        state.insert("agent_id".to_string(), Value::String(agent_id.clone()));
        if let Some(ref conv_id) = conversation_id {
            state.insert(
                "conversation_id".to_string(),
                Value::String(conv_id.clone()),
            );
        }
        Self {
            agent_id: Some(agent_id),
            conversation_id,
            available_skills,
            state: RwLock::new(state),
            ..Default::default()
        }
    }

    /// Set the function call ID (builder pattern for construction)
    #[must_use]
    pub fn with_function_call_id(self, id: String) -> Self {
        if let Ok(mut fcid) = self.function_call_id.write() {
            *fcid = id;
        }
        self
    }

    /// Set the function call ID for the current tool execution.
    /// This is called before each tool execution to track which tool call
    /// is currently being processed.
    pub fn set_function_call_id(&self, id: String) {
        if let Ok(mut fcid) = self.function_call_id.write() {
            *fcid = id;
        }
    }

    /// Get the current function call ID
    pub fn get_function_call_id(&self) -> String {
        self.function_call_id
            .read()
            .map(|id| id.clone())
            .unwrap_or_default()
    }

    /// Export state for checkpoint persistence.
    ///
    /// This serializes all state (including skill tracking) to JSON for saving
    /// in a checkpoint. On session resumption, this state can be restored via
    /// `restore_state()` or by passing to `full_with_state()`.
    ///
    /// Includes skill-related state keys like:
    /// - `skill:graph` - `SkillGraph` with loaded skills and resources
    /// - `skill:loaded_skills` - List of currently loaded skill names
    #[must_use]
    pub fn export_state(&self) -> Value {
        self.state
            .read()
            .map(|state| serde_json::json!(state.clone()))
            .unwrap_or(Value::Null)
    }

    /// Restore state from a checkpoint.
    ///
    /// Merges the checkpoint state into the current state, overwriting any
    /// existing keys. This is typically called after creating a new context
    /// when resuming an execution.
    pub fn restore_state(&self, checkpoint_state: &Value) {
        if let Some(obj) = checkpoint_state.as_object() {
            if let Ok(mut state) = self.state.write() {
                for (key, value) in obj {
                    state.insert(key.clone(), value.clone());
                }
            }
        }
    }

    /// Get skill-related state for middleware consumption.
    ///
    /// Returns the skill graph if available, which contains information about
    /// loaded skills and their resources. This is useful for middleware that
    /// needs to make skill-aware decisions.
    #[must_use]
    pub fn get_skill_state(&self) -> Option<Value> {
        CallbackContext::get_state(self, "skill:graph")
    }

    /// Get the conversation directory if `conversation_id` is set
    ///
    /// This returns None in the library context. The application layer
    /// should provide the actual path resolution.
    #[must_use]
    pub fn conversation_dir(&self) -> Option<PathBuf> {
        // In the library, we don't have access to the actual file system
        // The application layer should override or extend this
        self.conversation_id
            .as_ref()
            .map(|id| PathBuf::from(format!("/conversations/{id}")))
    }
}

// ============================================================================
// IMPLEMENT ZERO_CORE TRAITS
// ============================================================================

impl agent_primitives::ReadonlyContext for ToolContext {
    fn invocation_id(&self) -> &str {
        self.conversation_id.as_deref().unwrap_or("unknown")
    }

    fn agent_name(&self) -> &str {
        self.agent_id.as_deref().unwrap_or("root")
    }

    fn user_id(&self) -> &'static str {
        "default"
    }

    fn app_name(&self) -> &'static str {
        "zbot"
    }

    fn session_id(&self) -> &str {
        self.conversation_id.as_deref().unwrap_or("unknown")
    }

    fn branch(&self) -> &'static str {
        "main"
    }

    fn user_content(&self) -> &Content {
        &self.empty_content
    }
}

impl agent_primitives::CallbackContext for ToolContext {
    fn get_state(&self, key: &str) -> Option<Value> {
        self.state.read().ok()?.get(key).cloned()
    }

    fn set_state(&self, key: String, value: Value) {
        if let Ok(mut state) = self.state.write() {
            state.insert(key, value);
        }
    }

    /// Atomic claim — check and set under a single write lock.
    fn try_claim(&self, key: &str) -> bool {
        if let Ok(mut state) = self.state.write() {
            // Check if already claimed (value is Bool(true))
            if let Some(serde_json::Value::Bool(true)) = state.get(key) {
                return false;
            }
            state.insert(key.to_string(), serde_json::Value::Bool(true));
            true
        } else {
            false
        }
    }
}

impl agent_primitives::ToolContext for ToolContext {
    fn function_call_id(&self) -> String {
        self.get_function_call_id()
    }

    fn actions(&self) -> EventActions {
        self.actions.read().map(|a| a.clone()).unwrap_or_default()
    }

    fn set_actions(&self, actions: EventActions) {
        if let Ok(mut a) = self.actions.write() {
            *a = actions;
        }
    }
}

impl ToolContext {
    /// Atomically read and clear actions. Used for parallel tool execution
    /// to capture each tool's actions without race conditions.
    pub fn take_actions(&self) -> EventActions {
        if let Ok(mut a) = self.actions.write() {
            std::mem::take(&mut *a)
        } else {
            EventActions::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_export_state() {
        let ctx = ToolContext::new();

        // Set some state
        ctx.set_state("key1".to_string(), json!("value1"));
        ctx.set_state("key2".to_string(), json!(42));
        ctx.set_state(
            "skill:graph".to_string(),
            json!({
                "my-skill": {
                    "tool_call_id": "call_1",
                    "loaded_at": 12345,
                    "resources": []
                }
            }),
        );

        // Export state
        let exported = ctx.export_state();

        // Verify exported state
        assert!(exported.is_object());
        let obj = exported.as_object().unwrap();
        assert_eq!(obj.get("key1").unwrap(), "value1");
        assert_eq!(obj.get("key2").unwrap(), 42);
        assert!(obj.contains_key("skill:graph"));
    }

    #[test]
    fn test_restore_state() {
        let ctx = ToolContext::new();

        // Create checkpoint state
        let checkpoint_state = json!({
            "skill:graph": {
                "restored-skill": {
                    "tool_call_id": "call_restored",
                    "loaded_at": 99999,
                    "resources": []
                }
            },
            "skill:loaded_skills": ["restored-skill"],
            "custom_key": "custom_value"
        });

        // Restore state
        ctx.restore_state(&checkpoint_state);

        // Verify state was restored
        let skill_graph = ctx.get_state("skill:graph");
        assert!(skill_graph.is_some());
        let graph = skill_graph.unwrap();
        assert!(graph.get("restored-skill").is_some());

        let loaded_skills = ctx.get_state("skill:loaded_skills");
        assert!(loaded_skills.is_some());

        let custom = ctx.get_state("custom_key");
        assert_eq!(custom, Some(json!("custom_value")));
    }

    #[test]
    fn test_restore_state_merges_with_existing() {
        let ctx = ToolContext::new();

        // Set initial state
        ctx.set_state("existing_key".to_string(), json!("existing_value"));

        // Restore checkpoint (should merge, not replace)
        let checkpoint_state = json!({
            "new_key": "new_value"
        });
        ctx.restore_state(&checkpoint_state);

        // Both keys should exist
        assert_eq!(ctx.get_state("existing_key"), Some(json!("existing_value")));
        assert_eq!(ctx.get_state("new_key"), Some(json!("new_value")));
    }

    #[test]
    fn test_get_skill_state() {
        let ctx = ToolContext::new();

        // Initially no skill state
        assert!(ctx.get_skill_state().is_none());

        // Set skill graph
        ctx.set_state(
            "skill:graph".to_string(),
            json!({
                "test-skill": {"tool_call_id": "call_1"}
            }),
        );

        // Should now return the skill state
        let skill_state = ctx.get_skill_state();
        assert!(skill_state.is_some());
        assert!(skill_state.unwrap().get("test-skill").is_some());
    }

    #[test]
    fn with_conversation_sets_id() {
        let ctx = ToolContext::with_conversation("c1".to_string());
        assert_eq!(ctx.conversation_id.as_deref(), Some("c1"));
        assert!(ctx.available_skills.is_empty());
        assert!(ctx.agent_id.is_none());
    }

    #[test]
    fn with_skills_sets_skill_list() {
        let ctx = ToolContext::with_skills(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(ctx.available_skills, vec!["a".to_string(), "b".to_string()]);
        assert!(ctx.conversation_id.is_none());
    }

    #[test]
    fn with_conversation_and_skills_sets_both() {
        let ctx =
            ToolContext::with_conversation_and_skills("conv".to_string(), vec!["s".to_string()]);
        assert_eq!(ctx.conversation_id.as_deref(), Some("conv"));
        assert_eq!(ctx.available_skills.len(), 1);
    }

    #[test]
    fn with_agent_id_seeds_state() {
        let ctx = ToolContext::with_agent_id("agent-x".to_string());
        assert_eq!(ctx.agent_id.as_deref(), Some("agent-x"));
        // app:agent_id should be present in state
        let v = ctx.get_state("app:agent_id").unwrap();
        assert_eq!(v.as_str(), Some("agent-x"));
    }

    #[test]
    fn full_constructor_seeds_state_keys() {
        let ctx = ToolContext::full(
            "agent".to_string(),
            Some("conv".to_string()),
            vec!["sk".to_string()],
        );
        assert_eq!(ctx.get_state("app:agent_id").unwrap(), "agent");
        assert_eq!(ctx.get_state("app:conversation_id").unwrap(), "conv");
        assert_eq!(ctx.get_state("agent_id").unwrap(), "agent");
        assert_eq!(ctx.get_state("conversation_id").unwrap(), "conv");
    }

    #[test]
    fn full_with_state_no_conversation_omits_conv_keys() {
        let ctx = ToolContext::full("agent".to_string(), None, vec![]);
        assert!(ctx.get_state("app:conversation_id").is_none());
        assert!(ctx.get_state("conversation_id").is_none());
        assert_eq!(ctx.get_state("app:agent_id").unwrap(), "agent");
    }

    #[test]
    fn function_call_id_setter_getter_roundtrip() {
        let ctx = ToolContext::new();
        assert!(ctx.get_function_call_id().is_empty());
        ctx.set_function_call_id("call-42".to_string());
        assert_eq!(ctx.get_function_call_id(), "call-42");
    }

    #[test]
    fn with_function_call_id_builder() {
        let ctx = ToolContext::new().with_function_call_id("c1".to_string());
        assert_eq!(ctx.get_function_call_id(), "c1");
    }

    #[test]
    fn conversation_dir_synthesizes_from_id() {
        let ctx = ToolContext::with_conversation("conv-7".to_string());
        let dir = ctx.conversation_dir().expect("path");
        assert!(dir.to_string_lossy().contains("conv-7"));
        let none_ctx = ToolContext::new();
        assert!(none_ctx.conversation_dir().is_none());
    }

    #[test]
    fn try_claim_atomic() {
        use agent_primitives::CallbackContext;
        let ctx = ToolContext::new();
        // First claim succeeds
        assert!(ctx.try_claim("delegate"));
        // Second claim of the same key fails
        assert!(!ctx.try_claim("delegate"));
        // Different key still works
        assert!(ctx.try_claim("other"));
    }

    #[test]
    fn readonly_context_defaults() {
        use agent_primitives::ReadonlyContext;
        let ctx = ToolContext::new();
        assert_eq!(ctx.invocation_id(), "unknown");
        assert_eq!(ctx.agent_name(), "root");
        assert_eq!(ctx.user_id(), "default");
        assert_eq!(ctx.app_name(), "zbot");
        assert_eq!(ctx.session_id(), "unknown");
        assert_eq!(ctx.branch(), "main");
        let content = ctx.user_content();
        assert_eq!(content.role, "user");
    }

    #[test]
    fn readonly_context_uses_set_values() {
        use agent_primitives::ReadonlyContext;
        let ctx = ToolContext::full("a".to_string(), Some("c".to_string()), vec![]);
        assert_eq!(ctx.invocation_id(), "c");
        assert_eq!(ctx.agent_name(), "a");
        assert_eq!(ctx.session_id(), "c");
    }

    #[test]
    fn actions_round_trip_and_take() {
        use agent_primitives::ToolContext as ZcToolContext;
        let ctx = ToolContext::new();
        // Default actions
        let mut a = ctx.actions();
        a.transfer_to_agent = Some("X".to_string());
        ctx.set_actions(a);
        // Round-trip
        let stored = ctx.actions();
        assert_eq!(stored.transfer_to_agent.as_deref(), Some("X"));
        // Take clears them
        let taken = ctx.take_actions();
        assert_eq!(taken.transfer_to_agent.as_deref(), Some("X"));
        let after = ctx.actions();
        assert!(after.transfer_to_agent.is_none());
    }

    #[test]
    fn function_call_id_via_agent_primitives_trait() {
        use agent_primitives::ToolContext as ZcToolContext;
        let ctx = ToolContext::new();
        ctx.set_function_call_id("zid".to_string());
        assert_eq!(ZcToolContext::function_call_id(&ctx), "zid");
    }

    #[test]
    fn test_full_with_state_restores_checkpoint() {
        // This tests the flow: checkpoint.context_state -> initial_state -> ToolContext
        let initial_state = {
            let mut state = std::collections::HashMap::new();
            state.insert(
                "skill:graph".to_string(),
                json!({
                    "my-skill": {"tool_call_id": "call_1", "resources": []}
                }),
            );
            state.insert("skill:loaded_skills".to_string(), json!(["my-skill"]));
            state
        };

        let ctx = ToolContext::full_with_state(
            "agent-1".to_string(),
            Some("conv-1".to_string()),
            vec!["skill-a".to_string()],
            initial_state,
        );

        // Verify the skill state was restored
        let skill_graph = ctx.get_state("skill:graph");
        assert!(skill_graph.is_some());

        let loaded_skills = ctx.get_state("skill:loaded_skills");
        assert!(loaded_skills.is_some());
        let skills: Vec<String> = serde_json::from_value(loaded_skills.unwrap()).unwrap();
        assert!(skills.contains(&"my-skill".to_string()));
    }
}
