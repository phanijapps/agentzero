//! # Context Traits
//!
//! Context types for agent and tool execution.

use crate::event::EventActions;
use crate::types::Content;
use serde_json::Value;

/// Readonly context provides read access to execution context.
pub trait ReadonlyContext: Send + Sync {
    /// Get the invocation ID.
    fn invocation_id(&self) -> &str;

    /// Get the agent name.
    fn agent_name(&self) -> &str;

    /// Get the user ID.
    fn user_id(&self) -> &str;

    /// Get the app name.
    fn app_name(&self) -> &str;

    /// Get the session ID.
    fn session_id(&self) -> &str;

    /// Get the branch.
    fn branch(&self) -> &str;

    /// Get the user's content/message.
    fn user_content(&self) -> &Content;
}

/// Callback context provides access to artifacts and other callback-specific data.
pub trait CallbackContext: ReadonlyContext {
    /// Get state value.
    fn get_state(&self, key: &str) -> Option<Value>;

    /// Set state value.
    fn set_state(&self, key: String, value: Value);

    /// Atomically claim a key — returns true if this caller was first, false if already claimed.
    /// Used to prevent concurrent tool calls from racing (e.g., only first delegate_to_agent proceeds).
    /// Default implementation uses get+set (not truly atomic), override for lock-based atomicity.
    fn try_claim(&self, key: &str) -> bool {
        if self.get_state(key).is_some() {
            return false;
        }
        self.set_state(key.to_string(), Value::Bool(true));
        true
    }
}

/// Tool context provides additional context for tool execution.
pub trait ToolContext: CallbackContext {
    /// Get the function call ID for this tool execution.
    /// Returns a String to allow implementations using interior mutability (RwLock).
    fn function_call_id(&self) -> String;

    /// Get the current event actions.
    fn actions(&self) -> EventActions;

    /// Set event actions (e.g., to trigger escalation).
    fn set_actions(&self, actions: EventActions);
}
