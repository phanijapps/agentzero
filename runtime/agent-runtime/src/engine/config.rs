//! Execution policy shared by runtime adapters.

use super::hooks::{
    AfterToolCallHook, BeforeToolCallHook, ToolExecutionMode, TransformContextHook,
};
use serde_json::Value;
use std::{collections::HashSet, fmt};

/// Configuration for agent executor
#[derive(Clone)]
pub struct ExecutorConfig {
    /// Agent identifier
    pub agent_id: String,

    /// Provider identifier
    pub provider_id: String,

    /// Model to use
    pub model: String,

    /// Temperature for generation (0.0 - 1.0)
    pub temperature: f64,

    /// Maximum tokens to generate
    pub max_tokens: u32,

    /// Enable reasoning/thinking
    pub thinking_enabled: bool,

    /// System instruction
    pub system_instruction: Option<String>,

    /// Enable tools
    pub tools_enabled: bool,

    /// Registered tools that remain executable internally but are not offered
    /// in the model-visible tool schema.
    pub model_hidden_tools: HashSet<String>,

    /// MCP servers to use
    pub mcps: Vec<String>,

    /// Skills to use
    pub skills: Vec<String>,

    /// Conversation ID for scoping
    pub conversation_id: Option<String>,

    /// Initial state to inject into tool context.
    /// This allows passing hook context, delegation context, etc.
    #[allow(dead_code)]
    pub initial_state: std::collections::HashMap<String, Value>,

    /// Maximum characters for a tool result in context (default: 30000 chars ≈ 7500 tokens).
    /// Results exceeding this are truncated to head + tail with a notice.
    /// Set to 0 to disable truncation.
    pub max_tool_result_chars: usize,

    /// Offload large tool results to filesystem instead of keeping in context.
    pub offload_large_results: bool,

    /// Character threshold for offloading (default: 20000 chars ≈ 5000 tokens).
    pub offload_threshold_chars: usize,

    /// Directory to save offloaded tool results.
    pub offload_dir: Option<std::path::PathBuf>,

    /// Maximum LLM loop iterations before checking for progress (default: 50).
    /// Kept for diagnostics — no longer a hard stop. Set to 0 to disable diagnostics.
    pub max_iterations: u32,

    /// Maximum times auto-extension can be granted (default: 3, so 50 + 3*25 = 125 max).
    /// Legacy field — iteration limits are now advisory.
    pub max_extensions: u32,

    /// Additional iterations granted per auto-extension (default: 25).
    /// Legacy field — iteration limits are now advisory.
    pub extension_size: u32,

    /// Context window size for the model in tokens.
    /// Set to 0 to disable context-budget warnings.
    pub context_window_tokens: u64,

    /// Percentage of context window at which to inject a context memory flush warning.
    /// Default: 80. Chat mode sets this to 70 so the nudge fires before the middleware prunes.
    pub compaction_warn_pct: u64,

    /// Soft turn budget: inject a "wrap up" nudge after this many tool-calling iterations.
    /// Set to 0 to disable.
    pub turn_budget: u32,

    /// Hard turn limit: forcibly stop execution after this many iterations.
    /// Set to 0 to disable.
    pub max_turns: u32,

    /// Hook called before each tool execution. Can block the call.
    /// Default: None (all tools allowed).
    pub before_tool_call: Option<BeforeToolCallHook>,

    /// Hook called after each tool execution. Can transform the result.
    /// Default: None (results passed through unchanged).
    pub after_tool_call: Option<AfterToolCallHook>,

    /// Tool execution mode: parallel (default) or sequential.
    pub tool_execution_mode: ToolExecutionMode,

    /// Hook called before every LLM call to transform the message context.
    /// Default: None (messages passed through unchanged).
    pub transform_context: Option<TransformContextHook>,

    /// Task complexity level: "S", "M", "L", "XL".
    /// When set, applies complexity-based iteration budgets:
    /// S=15, M=30, L=50, XL=100.
    pub complexity: Option<String>,

    /// When true, only the first tool call per LLM response is executed.
    /// Extra tool calls are dropped with a log message.
    /// Default: false. Set true for orchestrator agents (root).
    pub single_action_mode: bool,
}

impl ExecutorConfig {
    /// Create a new executor config
    #[must_use]
    pub fn new(agent_id: String, provider_id: String, model: String) -> Self {
        Self {
            agent_id,
            provider_id,
            model,
            temperature: 0.7,
            max_tokens: 8192,
            thinking_enabled: false,
            system_instruction: None,
            tools_enabled: true,
            model_hidden_tools: HashSet::new(),
            mcps: Vec::new(),
            skills: Vec::new(),
            conversation_id: None,
            initial_state: std::collections::HashMap::new(),
            max_tool_result_chars: 30_000, // ~7500 tokens
            offload_large_results: false,
            offload_threshold_chars: 20_000, // ~5000 tokens
            offload_dir: None,
            max_iterations: 50,
            max_extensions: 3,
            extension_size: 25,
            context_window_tokens: 128_000, // Default to 128K context
            compaction_warn_pct: 80,        // Warn at 80% by default
            turn_budget: 25,                // Soft nudge at 25 turns
            max_turns: 50,                  // Hard stop at 50 turns
            before_tool_call: None,
            after_tool_call: None,
            tool_execution_mode: ToolExecutionMode::default(),
            transform_context: None,
            complexity: None,
            single_action_mode: false,
        }
    }

    /// Add initial state that will be injected into tool context
    #[must_use]
    pub fn with_initial_state(mut self, key: impl Into<String>, value: Value) -> Self {
        self.initial_state.insert(key.into(), value);
        self
    }

    /// Hide registered tools from model-visible schemas while preserving
    /// executor-internal dispatch for procedures, hooks, and compatibility.
    #[must_use]
    pub fn with_model_hidden_tools<I, S>(mut self, tool_names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.model_hidden_tools = tool_names.into_iter().map(Into::into).collect();
        self
    }
}

impl fmt::Debug for ExecutorConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutorConfig")
            .field("agent_id", &self.agent_id)
            .field("provider_id", &self.provider_id)
            .field("model", &self.model)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("thinking_enabled", &self.thinking_enabled)
            .field("system_instruction", &self.system_instruction)
            .field("tools_enabled", &self.tools_enabled)
            .field("model_hidden_tools", &self.model_hidden_tools)
            .field("mcps", &self.mcps)
            .field("skills", &self.skills)
            .field("conversation_id", &self.conversation_id)
            .field("initial_state", &self.initial_state)
            .field("max_tool_result_chars", &self.max_tool_result_chars)
            .field("offload_large_results", &self.offload_large_results)
            .field("offload_threshold_chars", &self.offload_threshold_chars)
            .field("offload_dir", &self.offload_dir)
            .field("max_iterations", &self.max_iterations)
            .field("max_extensions", &self.max_extensions)
            .field("extension_size", &self.extension_size)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("compaction_warn_pct", &self.compaction_warn_pct)
            .field("turn_budget", &self.turn_budget)
            .field("max_turns", &self.max_turns)
            .field(
                "before_tool_call",
                &self.before_tool_call.as_ref().map(|_| "<hook>"),
            )
            .field(
                "after_tool_call",
                &self.after_tool_call.as_ref().map(|_| "<hook>"),
            )
            .field("tool_execution_mode", &self.tool_execution_mode)
            .field(
                "transform_context",
                &self.transform_context.as_ref().map(|_| "<hook>"),
            )
            .field("complexity", &self.complexity)
            .field("single_action_mode", &self.single_action_mode)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ToolCallDecision;
    use serde_json::json;
    use std::sync::Arc;
    // ------------- ExecutorConfig builder + Debug -------------
    #[test]
    fn config_with_initial_state_records_value() {
        let cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into())
            .with_initial_state("k", json!("v"))
            .with_initial_state("k2", json!(42));
        assert_eq!(cfg.initial_state.get("k").unwrap(), "v");
        assert_eq!(cfg.initial_state.get("k2").unwrap(), 42);
    }

    #[test]
    fn config_debug_renders_hooks_as_placeholders() {
        let mut cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        cfg.before_tool_call = Some(Arc::new(|_, _| ToolCallDecision::Allow));
        cfg.after_tool_call = Some(Arc::new(|_, _, _, _| None));
        cfg.transform_context = Some(Arc::new(|_| {}));
        let s = format!("{cfg:?}");
        assert!(s.contains("<hook>"));
        assert!(s.contains("agent_id"));
    }

    #[test]
    fn tool_execution_mode_default_parallel() {
        assert_eq!(ToolExecutionMode::default(), ToolExecutionMode::Parallel);
    }

    // ------------- AgentExecutor builder methods -------------
}
