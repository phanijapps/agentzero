//! AgentZero-owned policy callbacks, independent of the execution loop.

use crate::types::ChatMessage;
use serde_json::Value;
use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc};

/// Result returned by the mid-session recall hook.
///
/// Contains novel facts formatted as a system message and the keys of those
/// facts so the caller can track already-injected keys.
#[derive(Debug, Clone)]
pub struct RecallHookResult {
    /// Formatted system message to inject (empty if nothing novel)
    pub system_message: String,
    /// Keys of the facts that were included (for dedup tracking)
    pub fact_keys: Vec<String>,
}

/// A callback invoked by the executor every N turns to refresh memory recall.
///
/// The hook receives:
/// - `latest_user_message`: the most recent user message for query context
/// - `already_injected_keys`: keys of facts already injected in this session
///
/// Returns a `RecallHookResult` with a formatted message and new keys.
pub type RecallHook = Box<
    dyn Fn(
            &str,
            &HashSet<String>,
        ) -> Pin<Box<dyn Future<Output = Result<RecallHookResult, String>> + Send>>
        + Send
        + Sync,
>;

/// Decision from beforeToolCall hook.
#[derive(Debug, Clone)]
pub enum ToolCallDecision {
    /// Allow the tool call to proceed.
    Allow,
    /// Block the tool call. The reason is returned to the LLM as the tool result.
    Block { reason: String },
}

/// Tool execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ToolExecutionMode {
    /// Execute all tools concurrently (current behavior).
    #[default]
    Parallel,
    /// Execute tools one at a time, in order.
    Sequential,
}

/// Type alias for beforeToolCall hook.
/// Receives (`tool_name`, args). Returns Allow or Block.
pub type BeforeToolCallHook = Arc<dyn Fn(&str, &Value) -> ToolCallDecision + Send + Sync>;

/// Type alias for afterToolCall hook.
/// Receives (`tool_name`, args, result, succeeded). Returns optional replacement result.
pub type AfterToolCallHook = Arc<dyn Fn(&str, &Value, &str, bool) -> Option<String> + Send + Sync>;

/// Type alias for transformContext hook.
/// Called before every LLM call. Can modify the message list in place.
pub type TransformContextHook = Arc<dyn Fn(&mut Vec<ChatMessage>) + Send + Sync>;
