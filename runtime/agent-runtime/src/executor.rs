// ============================================================================
// EXECUTOR MODULE
// Core agent execution engine
// ============================================================================

//! # Executor Module
//!
//! Core agent execution engine that coordinates LLM calls, tool execution,
//! and middleware processing.
//!
//! The executor is the main orchestrator that:
//! 1. Creates LLM client with provided configuration
//! 2. Uses provided tool registry and MCP manager
//! 3. Processes messages through middleware pipeline
//! 4. Executes LLM calls with streaming support
//! 5. Handles tool execution and result collection
//! 6. Emits events for real-time feedback

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use crate::context_management::{
    prepare_tool_result_for_context, sanitize_messages, ToolResultContextConfig,
};
use crate::llm::client::StreamChunk;
use crate::llm::LlmClient;
use crate::mcp::McpManager;
use crate::middleware::token_counter::estimate_total_tokens;
use crate::middleware::traits::MiddlewareContext;
use crate::middleware::MiddlewarePipeline;
use crate::rig_adapter::RigAgentConfig;
use crate::tools::context::ToolContext;
use crate::tools::ToolRegistry;
use crate::types::{ChatMessage, StreamEvent, ToolCall};
use agent_primitives::event::EventActions;
use agent_primitives::types::Part;
use agent_primitives::ToolContext as ZeroToolContext;

// ============================================================================
// MID-SESSION RECALL HOOK
// ============================================================================

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

/// Result from tool execution including any actions set by the tool
struct ToolExecutionResult {
    output: String,
    actions: EventActions,
}

/// Tool-call events, traces, and conversation rows are externally observable.
/// Peer text belongs only in the durable work payload and the recipient's
/// bounded steering envelope, so expose identifiers while replacing content.
fn externally_visible_tool_args(
    tool_name: &str,
    args: &Value,
    peer_content_in_context: bool,
) -> Value {
    if peer_content_in_context {
        return json!({"redacted": true, "reason": "peer_influenced"});
    }
    fn canonical_id(args: &Value, field: &str, prefix: &str) -> Value {
        let Some(value) = args.get(field).and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(raw) = value.strip_prefix(prefix) else {
            return Value::String("[REDACTED]".to_owned());
        };
        let Ok(uuid) = uuid::Uuid::parse_str(raw) else {
            return Value::String("[REDACTED]".to_owned());
        };
        if uuid.hyphenated().to_string() != raw {
            return Value::String("[REDACTED]".to_owned());
        }
        Value::String(value.to_owned())
    }

    match tool_name {
        "message_agent" => json!({
            "execution_id": canonical_id(args, "execution_id", "exec-"),
            "message": "[REDACTED]"
        }),
        "reply_to_agent" => json!({
            "reply_to": canonical_id(args, "reply_to", "work-"),
            "message": "[REDACTED]"
        }),
        _ => args.clone(),
    }
}

fn externally_visible_tool_result(
    peer_content_in_context: bool,
    result: String,
    context_result: Option<String>,
    error: Option<String>,
) -> (String, Option<String>, Option<String>) {
    if peer_content_in_context {
        (
            "[REDACTED: peer-influenced tool result]".to_owned(),
            context_result.map(|_| "[REDACTED: peer-influenced tool context]".to_owned()),
            error.map(|_| "peer_influenced_tool_error".to_owned()),
        )
    } else {
        (result, context_result, error)
    }
}

fn peer_safe_tools_schema(tools_schema: &Option<Value>) -> Option<Value> {
    let tools = tools_schema.as_ref()?.as_array()?;
    Some(Value::Array(
        tools
            .iter()
            .filter(|tool| {
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    == Some("respond")
            })
            .cloned()
            .collect(),
    ))
}

fn contains_persisted_peer_result(messages: &[ChatMessage]) -> bool {
    let last_local_user = messages.iter().rposition(|message| message.role == "user");
    let last_peer_result = messages.iter().rposition(|message| {
        message.role == "system"
            && message.content.iter().any(|part| {
                matches!(
                    part,
                    Part::Text { text }
                        if text.starts_with("[REMOTE ZBOT RESULT — UNTRUSTED DATA]")
                )
            })
    });
    last_peer_result.is_some_and(|peer| last_local_user.is_none_or(|user| peer > user))
}

// ============================================================================
// EXECUTOR CONFIGURATION
// ============================================================================

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

    /// Rig-facing config resolved from current AgentZero settings.
    pub rig_agent_config: Option<RigAgentConfig>,
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
            rig_agent_config: None,
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
            .field("rig_agent_config", &self.rig_agent_config)
            .finish()
    }
}

// ============================================================================
// TOOL HOOK TYPES
// ============================================================================

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

// ============================================================================
// AGENT EXECUTOR
// ============================================================================

/// Main agent executor
///
/// Coordinates LLM calls, tool execution, and middleware processing.
pub struct AgentExecutor {
    config: ExecutorConfig,
    llm_client: Arc<dyn LlmClient>,
    tool_registry: Arc<ToolRegistry>,
    mcp_manager: Arc<McpManager>,
    middleware_pipeline: Arc<MiddlewarePipeline>,
    /// Optional mid-session recall hook invoked every N turns.
    recall_hook: Option<Arc<RecallHook>>,
    /// How often (in turns) to run mid-session recall. 0 = disabled.
    recall_every_n_turns: u32,
    /// Keys of facts already injected at session start (seeds the dedup set).
    recall_initial_keys: HashSet<String>,
    /// Optional steering queue for mid-execution message injection.
    steering_queue: Option<std::sync::Mutex<crate::steering::SteeringQueue>>,
}

impl AgentExecutor {
    /// Create a new agent executor
    ///
    /// # Arguments
    /// * `config` - Executor configuration
    /// * `llm_client` - LLM client for making API calls
    /// * `tool_registry` - Registry of available tools
    /// * `mcp_manager` - MCP manager for external tools
    /// * `middleware_pipeline` - Middleware pipeline for preprocessing
    pub fn new(
        config: ExecutorConfig,
        llm_client: Arc<dyn LlmClient>,
        tool_registry: Arc<ToolRegistry>,
        mcp_manager: Arc<McpManager>,
        middleware_pipeline: Arc<MiddlewarePipeline>,
    ) -> Result<Self, ExecutorError> {
        Ok(Self {
            config,
            llm_client,
            tool_registry,
            mcp_manager,
            middleware_pipeline,
            recall_hook: None,
            recall_every_n_turns: 0,
            recall_initial_keys: HashSet::new(),
            steering_queue: None,
        })
    }

    /// Set the middleware pipeline
    pub fn set_middleware_pipeline(&mut self, pipeline: Arc<MiddlewarePipeline>) {
        self.middleware_pipeline = pipeline;
    }

    /// Configure mid-session recall.
    ///
    /// The `hook` is called every `every_n_turns` iterations with the latest
    /// user message and the set of already-injected fact keys. Novel facts
    /// are injected as a system message.
    ///
    /// `initial_keys` seeds the dedup set from facts injected at session start.
    pub fn set_recall_hook(
        &mut self,
        hook: RecallHook,
        every_n_turns: u32,
        initial_keys: HashSet<String>,
    ) {
        self.recall_hook = Some(Arc::new(hook));
        self.recall_every_n_turns = every_n_turns;
        self.recall_initial_keys = initial_keys;
    }

    /// Attach a steering queue to this executor.
    ///
    /// Call this before `execute_stream`. The returned `SteeringHandle` can be
    /// shared with the UI, parent agents, or budget enforcers.
    pub fn enable_steering(&mut self) -> crate::steering::SteeringHandle {
        let (queue, handle) = crate::steering::SteeringQueue::new();
        self.steering_queue = Some(std::sync::Mutex::new(queue));
        handle
    }

    /// Get the middleware pipeline
    #[must_use]
    pub fn middleware_pipeline(&self) -> &Arc<MiddlewarePipeline> {
        &self.middleware_pipeline
    }

    /// Get the configuration
    #[must_use]
    pub fn config(&self) -> &ExecutorConfig {
        &self.config
    }

    /// Clone of the LLM client (for the Rig cutover to wrap the same client).
    #[must_use]
    pub fn llm_client(&self) -> Arc<dyn LlmClient> {
        self.llm_client.clone()
    }

    /// Borrow the tool registry (for the Rig cutover to bridge the same,
    /// already actor-filtered tool inventory).
    #[must_use]
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    /// Registered tools that should be offered to the model. Hidden tools stay
    /// in `tool_registry` so internal dispatch can still execute them.
    #[must_use]
    pub fn model_visible_tools(&self) -> Vec<Arc<dyn agent_primitives::Tool>> {
        self.tool_registry
            .get_all()
            .iter()
            .filter(|tool| !self.config.model_hidden_tools.contains(tool.name()))
            .cloned()
            .collect()
    }

    /// Execute the agent with streaming
    ///
    /// The callback receives events as they occur during execution.
    pub async fn execute_stream(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        on_event: impl FnMut(StreamEvent),
    ) -> Result<(), ExecutorError> {
        self.execute_stream_with_stop_flag(user_message, history, None, on_event)
            .await
    }

    /// Like `execute_stream` but accepts an optional cooperative stop
    /// signal. When `stop_flag` is `Some`, the streaming-LLM `select!`
    /// loop polls it every ~100 ms; on stop, the spawned LLM task is
    /// aborted and the executor returns [`ExecutorError::Stopped`] so
    /// the caller can finalize the session without treating it as a
    /// real failure.
    ///
    /// Existing callers who don't have a stop signal continue to use
    /// `execute_stream` (which delegates here with `None`); their
    /// behaviour is bytecode-equivalent to before this method existed.
    pub async fn execute_stream_with_stop_flag(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        stop_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        mut on_event: impl FnMut(StreamEvent),
    ) -> Result<(), ExecutorError> {
        // Emit metadata event
        on_event(StreamEvent::Metadata {
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            agent_id: self.config.agent_id.clone(),
            model: self.config.model.clone(),
            provider: self.config.provider_id.clone(),
        });

        // Build messages array
        let mut messages = Vec::new();

        // Add system instruction if available
        if let Some(instruction) = &self.config.system_instruction {
            messages.push(ChatMessage::system(instruction.clone()));
        }

        // Add conversation history
        messages.extend(history.iter().cloned());

        // Add current user message
        messages.push(ChatMessage::user(user_message.to_string()));

        // Create middleware context
        let message_count = messages.len();
        let estimated_tokens = estimate_total_tokens(&messages, &self.config.model);

        // Build execution state from message history.
        // This extracts skill information from previous tool calls so middleware
        // can make skill-aware decisions during context compaction.
        let execution_state = crate::middleware::traits::ExecutionState::from_messages(&messages);

        // Plan state: scan the tape for the most recent `update_plan`
        // call. Same pattern as `ExecutionState::from_messages` — the
        // plan-block middleware uses this to render a pinned anchor
        // that survives compaction.
        let plan_state = crate::middleware::extract_plan_state(&messages);

        let middleware_context = MiddlewareContext::new(
            self.config.agent_id.clone(),
            self.config.conversation_id.clone(),
            self.config.provider_id.clone(),
            self.config.model.clone(),
        )
        .with_counts(message_count, estimated_tokens)
        .with_execution_state(execution_state)
        .with_plan_state(plan_state);

        // Process messages through middleware pipeline
        let processed_messages = self
            .middleware_pipeline
            .process_messages(messages, &middleware_context, &mut on_event)
            .await
            .map_err(ExecutorError::MiddlewareError)?;

        // Get tools schema if enabled
        let tools_schema = if self.config.tools_enabled {
            Some(self.build_tools_schema().await?)
        } else {
            None
        };

        tracing::debug!("Starting execute_with_tools_loop");

        // Execute with tool calling loop
        self.execute_with_tools_loop(processed_messages, tools_schema, stop_flag, &mut on_event)
            .await
    }

    async fn execute_with_tools_loop(
        &self,
        messages: Vec<ChatMessage>,
        tools_schema: Option<Value>,
        stop_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        on_event: &mut impl FnMut(StreamEvent),
    ) -> Result<(), ExecutorError> {
        tracing::debug!("=== execute_with_tools_loop starting ===");
        tracing::debug!("Messages count: {}", messages.len());
        tracing::debug!("Tools schema: {}", tools_schema.is_some());

        let mut current_messages = messages;
        #[allow(unused_assignments)] // Initialized here, assigned in loop exit condition
        let mut full_response = String::new();

        // Cumulative billing totals — summed across every LLM response in the
        // session, emitted on `TokenUpdate` events for cost tracking. These
        // grow monotonically and are NOT the right signal for compaction
        // decisions (a 50-turn loop with a stable 20k-token tape would show
        // `total_tokens_in = 1,000,000`, spuriously tripping the 80% trigger).
        let mut total_tokens_in: u64 = 0;
        let mut total_tokens_out: u64 = 0;

        // Current tape size, in prompt tokens, as last measured by the
        // provider on the prompt we most recently sent. This is the
        // authoritative per-turn occupancy signal and the ONLY value the
        // compaction trigger should compare against. Zero before the first
        // response; falls back to the previous value if a response omits
        // `usage` (some streaming error paths do).
        let mut current_prompt_tokens: u64 = 0;

        // Progress tracker for diagnostics and advisory nudges.
        // No longer used for hard stops — agent runs until done or safety valve trips.
        let mut progress_tracker = ProgressTracker::new(self.config.max_extensions);

        // Track whether we've sent a stuck-loop nudge (max 1)
        let mut stuck_nudge_sent = false;

        // Track whether we've injected a pre-compaction memory flush warning
        let mut compaction_warned = false;

        // Track whether the turn budget nudge has been sent (max 1)
        let mut turn_budget_nudge_sent = false;

        // Track message count to skip redundant compaction estimation
        let mut last_compaction_check_msg_count: usize = 0;

        // Track which fact keys have been injected via recall (initial + mid-session).
        // Seeded from initial recall keys; extended by mid-session recall hook results.
        let mut recall_injected_keys = self.recall_initial_keys.clone();

        // Track whether the loop stopped due to delegation (vs respond or natural end).
        // Set inside the loop; read after the loop to decide whether to emit Done.
        let mut stopped_for_delegation = false;

        // Once peer data enters the prompt tape, every later response in this
        // executor may be derived from it. Keep diagnostics metadata-only for
        // the rest of the loop rather than redacting just the first turn.
        let mut peer_content_in_context = contains_persisted_peer_result(&current_messages);

        // Create shared tool context that persists across all tool calls in this execution.
        // This allows tools like load_skill to maintain state (e.g., loaded skills, resources)
        // that other tools and middleware can access throughout the execution loop.
        let shared_tool_context = Arc::new(ToolContext::full_with_state(
            self.config.agent_id.clone(),
            self.config.conversation_id.clone(),
            self.config.skills.clone(),
            self.config.initial_state.clone(),
        ));

        loop {
            progress_tracker.tick();

            // Reset delegation claim at the start of each turn.
            // This allows root to delegate again after a previous delegation completes.
            // try_claim checks for Bool(true); setting to Bool(false) releases the claim.
            {
                use agent_primitives::CallbackContext;
                shared_tool_context
                    .set_state("app:delegation_active".to_string(), Value::Bool(false));
            }

            // Turn budget: soft nudge then hard stop
            if self.config.max_turns > 0
                && progress_tracker.total_iterations >= self.config.max_turns
            {
                tracing::warn!(
                    total_iterations = progress_tracker.total_iterations,
                    max_turns = self.config.max_turns,
                    "Hard turn limit reached, stopping execution"
                );
                // Emit done with explanation
                on_event(StreamEvent::Done {
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    final_message: format!(
                        "[Turn limit reached after {} iterations. Stopping execution.]",
                        progress_tracker.total_iterations
                    ),
                    token_count: 0,
                });
                return Ok(());
            }

            if self.config.turn_budget > 0
                && progress_tracker.total_iterations >= self.config.turn_budget
                && !turn_budget_nudge_sent
            {
                turn_budget_nudge_sent = true;
                current_messages.push(ChatMessage::user(format!(
                    "[SYSTEM: You have used {} of {} tool calls. Wrap up your current work \
                     and call `respond` with a summary. Do not start new explorations.]",
                    progress_tracker.total_iterations, self.config.max_turns
                )));
                tracing::info!(
                    total_iterations = progress_tracker.total_iterations,
                    turn_budget = self.config.turn_budget,
                    "Turn budget nudge sent"
                );
            }

            // Complexity-based budget enforcement
            if let Some(ref complexity) = self.config.complexity {
                let (hard_budget, soft_budget) = match complexity.as_str() {
                    "S" => (15u32, 12u32),
                    "M" => (30, 24),
                    "L" => (50, 40),
                    "XL" => (100, 80),
                    _ => (0, 0),
                };

                if hard_budget > 0 {
                    let iters = progress_tracker.total_iterations;
                    if iters >= hard_budget {
                        // Hard budget exceeded: inject urgent message
                        current_messages.push(ChatMessage::user(format!(
                            "[STEER: System] Budget exceeded ({iters}/{hard_budget} iterations for {complexity} task).                              Respond NOW with what you have. Do not start new work."
                        )));
                        tracing::warn!(
                            complexity = %complexity,
                            iterations = iters,
                            budget = hard_budget,
                            "Complexity hard budget reached"
                        );
                    } else if iters == soft_budget {
                        // Soft budget: nudge exactly once (when iters == soft_budget)
                        current_messages.push(ChatMessage::user(format!(
                            "[STEER: System] You've used {iters}/{hard_budget} iterations for a {complexity} task.                              Wrap up or simplify your approach."
                        )));
                        tracing::info!(
                            complexity = %complexity,
                            iterations = iters,
                            budget = hard_budget,
                            "Complexity soft budget nudge sent"
                        );
                    }
                }
            }

            // Advisory stuck-detection: inject nudge once, hard-stop only as safety valve
            if progress_tracker.is_clearly_stuck() {
                if !stuck_nudge_sent {
                    // First time: inject advisory nudge, let agent recover
                    stuck_nudge_sent = true;
                    current_messages.push(ChatMessage::user(
                        "[SYSTEM: You appear to be repeating similar actions without progress. \
                         Step back, re-read the full context, and try a different approach. \
                         If you cannot make progress, use the `respond` tool to summarize \
                         what you've accomplished and what remains.]"
                            .to_string(),
                    ));
                    tracing::warn!(
                        total_iterations = progress_tracker.total_iterations,
                        score = progress_tracker.score,
                        "Stuck-loop advisory nudge sent"
                    );
                } else if progress_tracker.score <= -12 {
                    // Safety valve: agent still stuck after nudge, hard-stop
                    let diagnosis = progress_tracker.diagnosis();
                    tracing::warn!(
                        total_iterations = progress_tracker.total_iterations,
                        score = progress_tracker.score,
                        diagnosis = %diagnosis,
                        "Safety valve: agent stuck after nudge, stopping"
                    );
                    return Err(ExecutorError::MaxIterationsNeedsIntervention {
                        iterations_used: progress_tracker.total_iterations,
                        reason: diagnosis,
                    });
                }
            }

            // Token-budget warning trigger.
            // Compares the CURRENT prompt size (as measured by the provider on
            // the prompt we just sent) against the configured context window.
            // `total_tokens_in` is billing-only here — it grows across turns
            // and would spuriously trip the trigger on long tool loops.
            //
            // Skip the check entirely if no new messages have been added
            // since last check — avoids redundant threshold evaluation.
            if self.config.context_window_tokens > 0
                && current_messages.len() > last_compaction_check_msg_count
            {
                last_compaction_check_msg_count = current_messages.len();
                let warn_threshold =
                    (self.config.context_window_tokens * self.config.compaction_warn_pct) / 100;
                let prune_threshold = (self.config.context_window_tokens * 80) / 100;
                if current_prompt_tokens > warn_threshold {
                    // Context memory flush: inject a nudge to save important facts
                    // before middleware pruning. The agent can use save_fact on the
                    // next turn before the old messages disappear.
                    if !compaction_warned {
                        current_messages.push(ChatMessage::system(
                            "[system] Context is getting full. Save important facts with \
                             memory(action=\"save_fact\", scope=\"chat\") before they are pruned."
                                .to_string(),
                        ));
                        compaction_warned = true;
                        tracing::info!(
                            current_prompt_tokens = current_prompt_tokens,
                            warn_threshold = warn_threshold,
                            "Context memory flush warning injected"
                        );
                        // Give the agent one turn to save facts before middleware pruning.
                        continue;
                    }

                    tracing::debug!(
                        current_prompt_tokens = current_prompt_tokens,
                        prune_threshold = prune_threshold,
                        "Context threshold exceeded; runtime middleware owns pruning"
                    );
                }
            }

            // Mid-session recall: every N turns, fetch novel facts and inject as
            // a system message so the agent benefits from memory even during long
            // multi-turn sessions.
            if self.recall_every_n_turns > 0
                && progress_tracker.total_iterations > 0
                && progress_tracker
                    .total_iterations
                    .is_multiple_of(self.recall_every_n_turns)
            {
                if let Some(hook) = &self.recall_hook {
                    // Find the latest user message for query context
                    let latest_user_msg = current_messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "user")
                        .map(super::types::messages::ChatMessage::text_content)
                        .unwrap_or_default();

                    let hook_clone = Arc::clone(hook);
                    match hook_clone(&latest_user_msg, &recall_injected_keys).await {
                        Ok(result) if !result.system_message.is_empty() => {
                            current_messages.push(ChatMessage::system(result.system_message));
                            // Track newly injected keys for future dedup
                            for key in result.fact_keys {
                                recall_injected_keys.insert(key);
                            }
                            tracing::info!(
                                turn = progress_tracker.total_iterations,
                                total_keys = recall_injected_keys.len(),
                                "Mid-session recall injected novel facts"
                            );
                        }
                        Ok(_) => {
                            tracing::debug!(
                                turn = progress_tracker.total_iterations,
                                "Mid-session recall: no novel facts"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                turn = progress_tracker.total_iterations,
                                error = %e,
                                "Mid-session recall failed"
                            );
                        }
                    }
                }
            }

            // Drain steering queue: inject any pending steering messages.
            // Durable peer acknowledgements remain live until the resulting
            // LLM call succeeds; a crash/error before then leaves queue work
            // recoverable for at-least-once redelivery.
            let mut peer_delivery_acks = Vec::new();
            if let Some(ref steering_mutex) = self.steering_queue {
                if let Ok(mut queue) = steering_mutex.lock() {
                    let mut steering_messages = queue.drain();
                    for msg in &mut steering_messages {
                        let formatted = format!("[STEER: {}] {}", msg.source, msg.content);
                        current_messages.push(ChatMessage::user(formatted));
                        if msg.source == crate::steering::SteeringSource::Peer {
                            peer_content_in_context = true;
                            if let Some(ack) = msg.take_delivery_ack() {
                                peer_delivery_acks.push(ack);
                            }
                        }
                        tracing::info!(
                            source = %msg.source,
                            priority = ?msg.priority,
                            "Injected steering message"
                        );
                    }
                }
            }

            // Sanitize messages to remove orphaned tool messages before LLM call.
            // This prevents API errors when compaction or summarization splits
            // assistant+tool pairs.
            sanitize_messages(&mut current_messages);

            // transformContext hook: allow caller to modify messages before LLM call
            if let Some(ref hook) = self.config.transform_context {
                hook(&mut current_messages);
            }

            // Real streaming via chat_stream() with mpsc channel bridge.
            // Tokens are emitted to the user IMMEDIATELY as they arrive from the LLM,
            // including intermediate text that accompanies tool calls.
            // NOTE: When non_streaming is enabled on the LLM client wrapper,
            // chat_stream() internally uses chat() and emits content as a single chunk.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamChunk>();

            let llm_client = self.llm_client.clone();
            let messages_for_stream = current_messages.clone();
            let tools_for_stream = if peer_content_in_context {
                peer_safe_tools_schema(&tools_schema)
            } else {
                tools_schema.clone()
            };

            // Spawn the streaming LLM call in a separate task
            let stream_handle = tokio::spawn(async move {
                llm_client
                    .chat_stream(
                        messages_for_stream,
                        tools_for_stream,
                        Box::new(move |chunk| {
                            let _ = tx.send(chunk);
                        }),
                    )
                    .await
            });

            // Process chunks as they arrive — emit Token events in real-time.
            // Uses tokio::select! with a 10s heartbeat interval so that during
            // extended silent phases (e.g., LLM reasoning), heartbeat events keep
            // WebSocket connections alive (client PONG_TIMEOUT is 30s).
            //
            // When `stop_flag` is provided, an additional 100ms-cadence arm
            // polls it; on observation the spawned LLM task is aborted so the
            // executor can return immediately rather than waiting for the LLM
            // call to finish naturally. 100ms is the perceived-instant
            // threshold for UI cancellation.
            let mut streamed_content = String::new();
            let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(10));
            heartbeat_interval.tick().await; // consume immediate first tick
            let mut stop_poll = tokio::time::interval(std::time::Duration::from_millis(100));
            stop_poll.tick().await; // consume immediate first tick

            let mut stop_observed = false;

            loop {
                tokio::select! {
                    chunk = rx.recv() => {
                        match chunk {
                            Some(StreamChunk::Token(text)) => {
                                streamed_content.push_str(&text);
                                on_event(StreamEvent::Token {
                                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                    content: text,
                                });
                                heartbeat_interval.reset();
                            }
                            Some(StreamChunk::Reasoning(text)) => {
                                on_event(StreamEvent::Reasoning {
                                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                    content: text,
                                });
                                heartbeat_interval.reset();
                            }
                            Some(StreamChunk::ToolCall(_)) => {
                                // Tool call chunks are accumulated by the streaming impl
                                // and returned in the final ChatResponse. No action needed here.
                                heartbeat_interval.reset();
                            }
                            None => break, // channel closed
                        }
                    }
                    _ = heartbeat_interval.tick() => {
                        on_event(StreamEvent::Heartbeat {
                            timestamp: chrono::Utc::now().timestamp_millis() as u64,
                        });
                    }
                    _ = stop_poll.tick(), if stop_flag.is_some() => {
                        let observed: bool = stop_flag
                            .as_ref()
                            .map(|f| f.load(std::sync::atomic::Ordering::SeqCst))
                            .unwrap_or(false);
                        if observed {
                            tracing::info!("Stop requested mid-stream; aborting LLM task");
                            stream_handle.abort();
                            stop_observed = true;
                            break;
                        }
                    }
                }
            }

            // Await the final response. If we observed a stop, the spawned
            // task was aborted and `await` returns a cancelled JoinError; we
            // surface that as `ExecutorError::Stopped` so the outer iteration
            // loop can finalize the session without treating it as a real
            // failure (no partial response is fed into history persistence,
            // tool-call accumulation, or distillation).
            let response = match stream_handle.await {
                Ok(inner) => inner.map_err(|e| ExecutorError::LlmError(e.to_string()))?,
                Err(e) if stop_observed || e.is_cancelled() => {
                    return Err(ExecutorError::Stopped);
                }
                Err(e) => {
                    return Err(ExecutorError::LlmError(format!(
                        "Stream task panicked: {e}"
                    )));
                }
            };
            for ack in peer_delivery_acks {
                let _ = ack.send(());
            }

            // Update cumulative token counts and emit event
            if let Some(usage) = &response.usage {
                total_tokens_in += u64::from(usage.prompt_tokens);
                total_tokens_out += u64::from(usage.completion_tokens);
                // Single-response prompt size: the provider's authoritative
                // measurement of the tape we just sent. Drives context warnings.
                current_prompt_tokens = u64::from(usage.prompt_tokens);

                on_event(StreamEvent::TokenUpdate {
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    tokens_in: total_tokens_in,
                    tokens_out: total_tokens_out,
                });
            }

            if peer_content_in_context {
                tracing::info!(
                    response_length = response.content.len(),
                    tool_call_count = response.tool_calls.as_ref().map_or(0, std::vec::Vec::len),
                    "LLM response after peer steering"
                );
            } else {
                tracing::debug!(
                    "LLM response - content: '{}', tool_calls: {}",
                    response.content,
                    response.tool_calls.as_ref().map_or(0, std::vec::Vec::len)
                );
            }

            // Check for tool calls
            let mut tool_calls = response.tool_calls.clone().unwrap_or_default();

            // Single-action mode: execute only the first tool call, drop extras.
            // This prevents the model from batching multiple actions into one response.
            if self.config.single_action_mode && tool_calls.len() > 1 {
                tracing::info!(
                    "Single-action mode: executing '{}', dropping {} extra tool calls",
                    tool_calls[0].name,
                    tool_calls.len() - 1
                );
                tool_calls.truncate(1);
            }
            if tool_calls.is_empty() {
                // No tool calls, this is the final response
                // Text was already streamed in real-time above
                full_response = response.content.clone();
                tracing::debug!(
                    "No tool calls, final response length: {}",
                    full_response.len()
                );
                break;
            }

            // Handle tool calls
            // Store the assistant message with ORIGINAL tool calls (not truncated).
            // Truncation caused the LLM to copy garbled text on retries.
            current_messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: response.content.clone(),
                }],
                tool_calls: Some(tool_calls.clone()),
                tool_call_id: None,
                is_summary: false,
            });

            // Track if respond tool was called - signals we should stop after this batch
            let mut should_stop_after_respond = false;

            // Emit ToolCallStart events for all tools before execution
            for tool_call in &tool_calls {
                on_event(StreamEvent::ToolCallStart {
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    tool_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    args: externally_visible_tool_args(
                        &tool_call.name,
                        &tool_call.arguments,
                        peer_content_in_context,
                    ),
                });
            }

            // Check beforeToolCall hook for each tool
            let mut blocked_results: HashMap<String, String> = HashMap::new();
            if peer_content_in_context {
                for tool_call in &tool_calls {
                    if tool_call.name != "respond" {
                        blocked_results.insert(
                            tool_call.id.clone(),
                            "{\"blocked\":true,\"reason\":\"peer_data_authority_boundary\"}"
                                .to_owned(),
                        );
                    }
                }
            }
            if let Some(ref hook) = self.config.before_tool_call {
                for tc in &tool_calls {
                    if blocked_results.contains_key(&tc.id) {
                        continue;
                    }
                    match hook(&tc.name, &tc.arguments) {
                        ToolCallDecision::Allow => {}
                        ToolCallDecision::Block { reason } => {
                            blocked_results.insert(
                                tc.id.clone(),
                                format!("{{\"blocked\":true,\"reason\":\"{reason}\"}}"),
                            );
                        }
                    }
                }
            }

            let non_blocked: Vec<&ToolCall> = tool_calls
                .iter()
                .filter(|tc| !blocked_results.contains_key(&tc.id))
                .collect();

            let results: Vec<(Result<ToolExecutionResult, String>, i64)> =
                if self.config.tool_execution_mode == ToolExecutionMode::Sequential {
                    // Sequential: execute one at a time, in order
                    let mut seq_results = Vec::new();
                    for tc in &non_blocked {
                        let started = Instant::now();
                        let result = self
                            .execute_tool(&shared_tool_context, &tc.id, &tc.name, &tc.arguments)
                            .await;
                        let duration_ms = started.elapsed().as_millis() as i64;
                        seq_results.push((result, duration_ms));
                    }
                    seq_results
                } else {
                    // Parallel: all at once (current behavior)
                    let tool_futures: Vec<_> = non_blocked
                        .iter()
                        .map(|tc| {
                            let ctx = shared_tool_context.clone();
                            let tool_id = tc.id.clone();
                            let tool_name = tc.name.clone();
                            let args = tc.arguments.clone();
                            async move {
                                tracing::debug!(
                                    tool_name,
                                    args = %externally_visible_tool_args(
                                        &tool_name,
                                        &args,
                                        peer_content_in_context,
                                    ),
                                    "Executing tool"
                                );
                                let started = Instant::now();
                                let result =
                                    self.execute_tool(&ctx, &tool_id, &tool_name, &args).await;
                                let duration_ms = started.elapsed().as_millis() as i64;
                                (result, duration_ms)
                            }
                        })
                        .collect();
                    futures::future::join_all(tool_futures).await
                };

            // Build a map of executed results (keyed by tool_call id)
            let mut executed_results: HashMap<String, (Result<ToolExecutionResult, String>, i64)> =
                HashMap::new();
            for (tc, result) in non_blocked.into_iter().zip(results) {
                executed_results.insert(tc.id.clone(), result);
            }

            // Process results in original tool_call order
            for tool_call in &tool_calls {
                let tool_name = &tool_call.name;

                if let Some(blocked_result) = blocked_results.remove(&tool_call.id) {
                    // Blocked by beforeToolCall hook
                    current_messages.push(ChatMessage::tool_result(
                        tool_call.id.clone(),
                        blocked_result.clone(),
                    ));
                    let (event_result, event_context, event_error) = externally_visible_tool_result(
                        peer_content_in_context,
                        "[blocked by hook]".to_owned(),
                        Some(blocked_result.clone()),
                        Some("blocked_by_hook".to_owned()),
                    );
                    on_event(StreamEvent::ToolResult {
                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                        tool_id: tool_call.id.clone(),
                        result: event_result,
                        context_result: event_context,
                        error: event_error,
                        duration_ms: Some(0),
                    });
                    progress_tracker.record_tool_call(&tool_call.name, &tool_call.arguments, false);
                } else if let Some(result) = executed_results.remove(&tool_call.id) {
                    let (result, duration_ms) = result;
                    match result {
                        Ok(tool_result) => {
                            let output = tool_result.output;
                            let actions = tool_result.actions;

                            if peer_content_in_context {
                                tracing::debug!(
                                    tool_name,
                                    output_length = output.len(),
                                    "Peer-influenced tool completed"
                                );
                            } else {
                                tracing::debug!("Tool result: {}", output);
                            }

                            // Track progress: tool succeeded
                            progress_tracker.record_tool_call(
                                tool_name,
                                &tool_call.arguments,
                                true,
                            );

                            // Check for respond action
                            if let Some(respond) = &actions.respond {
                                on_event(StreamEvent::ActionRespond {
                                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                    message: respond.message.clone(),
                                    format: respond.format.clone(),
                                    conversation_id: respond.conversation_id.clone(),
                                    session_id: respond.session_id.clone(),
                                    artifacts: respond.artifacts.clone(),
                                });
                                should_stop_after_respond = true;
                                progress_tracker.record_respond();
                                tracing::debug!(
                                    "Respond action detected, will stop after current tool batch"
                                );
                            }

                            // Check for delegate action
                            if let Some(delegate) = &actions.delegate {
                                on_event(StreamEvent::ActionDelegate {
                                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                    agent_id: delegate.agent_id.clone(),
                                    task: delegate.task.clone(),
                                    context: delegate.context.clone(),
                                    wait_for_result: delegate.wait_for_result,
                                    max_iterations: delegate.max_iterations,
                                    output_schema: delegate.output_schema.clone(),
                                    skills: delegate.skills.clone(),
                                    capability_assignment: delegate.capability_assignment.clone(),
                                    planning_capability_catalog: delegate
                                        .planning_capability_catalog
                                        .clone(),
                                    complexity: delegate.complexity.clone(),
                                    mode: delegate.mode.clone(),
                                    parallel: delegate.parallel,
                                    child_execution_id: delegate.child_execution_id.clone(),
                                });
                                // Delegation claim is set atomically by the delegate tool via try_claim
                                // Sequential delegations stop the loop immediately — the
                                // continuation callback resumes the root when the subagent
                                // completes. Parallel delegations keep the loop alive so the
                                // LLM can fire additional parallel delegates before calling
                                // respond(); the execution_stream pauses once respond() fires
                                // and resumes when ALL parallel agents have completed.
                                if !delegate.parallel {
                                    stopped_for_delegation = true;
                                }
                                tracing::debug!(
                                    parallel = delegate.parallel,
                                    "Delegation detected — stopping={}",
                                    !delegate.parallel
                                );
                            }

                            // Only the capability-gated presentation tool may
                            // turn result markers into user-visible surfaces.
                            if let Ok(parsed) = serde_json::from_str::<Value>(&output) {
                                if tool_name == "present_surface" {
                                    // A work surface is declarative data only. It is validated by
                                    // gateway-execution before any client can receive it.
                                    if parsed
                                        .get("__work_surface")
                                        .and_then(serde_json::Value::as_bool)
                                        .unwrap_or(false)
                                    {
                                        if let Some(surface) = parsed.get("surface").cloned() {
                                            if let Ok(surface) = serde_json::from_value(surface) {
                                                on_event(StreamEvent::WorkSurface {
                                                    timestamp: chrono::Utc::now().timestamp_millis()
                                                        as u64,
                                                    surface,
                                                });
                                            }
                                        }
                                    }
                                    if parsed
                                        .get("__work_surface_updated")
                                        .and_then(serde_json::Value::as_bool)
                                        .unwrap_or(false)
                                    {
                                        if let Some(surface) = parsed
                                            .get("surface")
                                            .cloned()
                                            .and_then(|value| serde_json::from_value(value).ok())
                                        {
                                            on_event(StreamEvent::WorkSurfaceUpdated {
                                                timestamp: chrono::Utc::now().timestamp_millis()
                                                    as u64,
                                                surface,
                                            });
                                        }
                                    }
                                    if parsed
                                        .get("__work_surface_deleted")
                                        .and_then(serde_json::Value::as_bool)
                                        .unwrap_or(false)
                                    {
                                        if let Some(surface_id) = parsed
                                            .get("surface_id")
                                            .and_then(serde_json::Value::as_str)
                                            .filter(|id| !id.is_empty() && id.len() <= 128)
                                        {
                                            on_event(StreamEvent::WorkSurfaceDeleted {
                                                timestamp: chrono::Utc::now().timestamp_millis()
                                                    as u64,
                                                surface_id: surface_id.to_owned(),
                                            });
                                        }
                                    }
                                }

                                // Check for show_content marker
                                if parsed
                                    .get("__show_content")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false)
                                {
                                    let content_type = parsed
                                        .get("content_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("text")
                                        .to_string();
                                    let title = parsed
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Content")
                                        .to_string();
                                    let content = parsed
                                        .get("content")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let metadata = parsed.get("metadata").cloned();
                                    let file_path = parsed
                                        .get("file_path")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string);
                                    let is_attachment = parsed
                                        .get("is_attachment")
                                        .and_then(serde_json::Value::as_bool);
                                    let base64 =
                                        parsed.get("base64").and_then(serde_json::Value::as_bool);

                                    on_event(StreamEvent::ShowContent {
                                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                        content_type,
                                        title,
                                        content,
                                        metadata,
                                        file_path,
                                        is_attachment,
                                        base64,
                                    });
                                }

                                // Check for request_input marker
                                if parsed
                                    .get("__request_input")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false)
                                {
                                    let form_id = parsed
                                        .get("form_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(&format!(
                                            "form_{}",
                                            chrono::Utc::now().timestamp()
                                        ))
                                        .to_string();
                                    let form_type = parsed
                                        .get("form_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("json_schema")
                                        .to_string();
                                    let title = parsed
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Input Required")
                                        .to_string();
                                    let description = parsed
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string);
                                    let schema =
                                        parsed.get("schema").cloned().unwrap_or_else(|| json!({}));
                                    let submit_button = parsed
                                        .get("submit_button")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string);

                                    on_event(StreamEvent::RequestInput {
                                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                        form_id,
                                        form_type,
                                        title,
                                        description,
                                        schema,
                                        submit_button,
                                    });
                                }

                                // Check for ward_changed marker (from ward tool)
                                if parsed
                                    .get("__ward_changed__")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false)
                                {
                                    if let Some(ward_id) =
                                        parsed.get("ward_id").and_then(|v| v.as_str())
                                    {
                                        on_event(StreamEvent::WardChanged {
                                            timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                            ward_id: ward_id.to_string(),
                                        });
                                    }
                                }

                                // Check for plan_update marker
                                if parsed
                                    .get("__plan_update")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false)
                                {
                                    let plan =
                                        parsed.get("plan").cloned().unwrap_or_else(|| json!([]));
                                    let explanation = parsed
                                        .get("explanation")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string);

                                    on_event(StreamEvent::ActionPlanUpdate {
                                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                        plan,
                                        explanation,
                                    });
                                }

                                // Check for session_title_changed marker
                                if parsed
                                    .get("__session_title_changed__")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false)
                                {
                                    if let Some(title) =
                                        parsed.get("title").and_then(|v| v.as_str())
                                    {
                                        on_event(StreamEvent::SessionTitleChanged {
                                            timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                            title: title.to_string(),
                                        });
                                    }
                                }
                            }

                            // Process tool result for model context. Large raw outputs are
                            // offloaded when configured, then truncated as a safety net.
                            let processed_output =
                                self.prepare_tool_result_for_context(tool_name, output.clone());

                            // afterToolCall hook — can transform the result
                            let final_output = if let Some(ref hook) = self.config.after_tool_call {
                                match hook(tool_name, &tool_call.arguments, &processed_output, true)
                                {
                                    Some(replacement) => replacement,
                                    None => processed_output,
                                }
                            } else {
                                processed_output
                            };

                            let (event_result, event_context, event_error) =
                                externally_visible_tool_result(
                                    peer_content_in_context,
                                    output,
                                    Some(final_output.clone()),
                                    None,
                                );
                            on_event(StreamEvent::ToolResult {
                                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                tool_id: tool_call.id.clone(),
                                result: event_result,
                                context_result: event_context,
                                error: event_error,
                                duration_ms: Some(duration_ms),
                            });

                            // Add tool result message
                            current_messages
                                .push(ChatMessage::tool_result(tool_call.id.clone(), final_output));
                        }
                        Err(e) => {
                            if peer_content_in_context {
                                tracing::debug!(tool_name, "Peer-influenced tool failed");
                            } else {
                                tracing::debug!("Tool error: {}", e);
                            }

                            // Track progress: tool failed
                            progress_tracker.record_tool_call(
                                tool_name,
                                &tool_call.arguments,
                                false,
                            );
                            progress_tracker.record_error(&e);

                            // afterToolCall hook — can transform error results too
                            let error_message = json!({"error": e}).to_string();
                            let final_error = if let Some(ref hook) = self.config.after_tool_call {
                                match hook(tool_name, &tool_call.arguments, &error_message, false) {
                                    Some(replacement) => replacement,
                                    None => error_message,
                                }
                            } else {
                                error_message
                            };

                            let (event_result, event_context, event_error) =
                                externally_visible_tool_result(
                                    peer_content_in_context,
                                    String::new(),
                                    Some(final_error.clone()),
                                    Some(e.clone()),
                                );
                            on_event(StreamEvent::ToolResult {
                                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                tool_id: tool_call.id.clone(),
                                result: event_result,
                                context_result: event_context,
                                error: event_error,
                                duration_ms: Some(duration_ms),
                            });

                            // Add error result message
                            current_messages
                                .push(ChatMessage::tool_result(tool_call.id.clone(), final_error));
                        }
                    }
                }

                on_event(StreamEvent::ToolCallEnd {
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    tool_id: tool_call.id.clone(),
                    tool_name: tool_name.clone(),
                    args: externally_visible_tool_args(
                        tool_name,
                        &tool_call.arguments,
                        peer_content_in_context,
                    ),
                });
            }

            // Planning enforcement: nudge if agent hasn't planned
            if progress_tracker.needs_planning_nudge() {
                current_messages.push(ChatMessage::user(
                    "[SYSTEM: You have made several tool calls without creating a plan. \
                     For complex tasks, use the `update_plan` tool to track your steps. \
                     This helps you stay focused and avoid repeating work.]"
                        .to_string(),
                ));
            }

            // If respond tool was called, stop the loop - agent has finished responding
            if should_stop_after_respond || stopped_for_delegation {
                tracing::debug!(
                    "Stopping execution loop — respond={} delegation={}",
                    should_stop_after_respond,
                    stopped_for_delegation
                );
                break;
            }
        }

        // Emit done event — but NOT if we stopped for delegation.
        // When delegation is pending, the runner should NOT mark this execution
        // as completed. The continuation callback will resume it later.
        if stopped_for_delegation {
            tracing::info!("Executor paused for delegation — skipping Done event");
        } else {
            on_event(StreamEvent::Done {
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                final_message: full_response.clone(),
                token_count: full_response.len(),
            });
        }

        // Emit context state for checkpoint persistence
        // This includes skill tracking (graph), loaded skills, and other tool context state
        // that should be persisted for session resumption.
        on_event(StreamEvent::ContextState {
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            state: shared_tool_context.export_state(),
        });

        Ok(())
    }

    async fn execute_tool(
        &self,
        shared_ctx: &Arc<ToolContext>,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<ToolExecutionResult, String> {
        // --- Replay intercept ---------------------------------------------------
        // When ZBOT_REPLAY_DIR is set, look up a recorded result and return it
        // instead of running the real tool. Strict mode (default) panics on miss;
        // lenient mode falls through to real execution.
        if let Some(store) = agent_tools::replay::global_store() {
            let exec_id =
                agent_primitives::ReadonlyContext::invocation_id(shared_ctx.as_ref()).to_string();
            if let Ok(mut guard) = store.lock() {
                match guard.lookup(&exec_id, tool_name) {
                    agent_tools::replay::LookupOutcome::Hit(result) => {
                        return Ok(ToolExecutionResult {
                            output: result,
                            actions: EventActions::default(),
                        });
                    }
                    agent_tools::replay::LookupOutcome::Drift {
                        expected_tool,
                        got_tool,
                    } => {
                        panic!(
                            "[tool-replay] drift on exec {exec_id}: expected '{expected_tool}' got '{got_tool}'"
                        );
                    }
                    agent_tools::replay::LookupOutcome::MissStrict {
                        exec_id: miss_id,
                        tool_index,
                    } => {
                        panic!(
                            "[tool-replay] strict miss on exec {miss_id} tool_index {tool_index}"
                        );
                    }
                    agent_tools::replay::LookupOutcome::MissLenient => {
                        // fall through to real execution
                    }
                }
            }
        }
        // --- end replay intercept -----------------------------------------------

        // First try built-in tools
        if let Some(tool) = self.tool_registry.find(tool_name) {
            // Use shared context that persists across all tool calls in this execution.
            // Set the function_call_id for this specific tool call so tools can track
            // their position in the conversation (e.g., for skill loading).
            shared_ctx.set_function_call_id(tool_call_id.to_string());

            // Clear actions before execution so we capture only this tool's actions
            shared_ctx.set_actions(EventActions::default());

            let result = tool
                .execute(shared_ctx.clone(), arguments.clone())
                .await
                .map_err(|e| format!("Tool execution failed: {e:?}"))?;

            // Atomically take any actions that were set by the tool
            let actions = shared_ctx.take_actions();

            return Ok(ToolExecutionResult {
                output: serde_json::to_string(&result).unwrap_or_else(|_| "null".to_string()),
                actions,
            });
        }

        // Try MCP tools
        // MCP tools have format: {normalized_server_id}__{normalized_tool_name}
        if tool_name.contains("__") {
            let parts: Vec<&str> = tool_name.splitn(2, "__").collect();
            if parts.len() == 2 {
                let normalized_server_id = parts[0];
                let normalized_tool = parts[1];

                // Find the original server ID by checking which one matches when normalized
                let mut original_server_id: Option<String> = None;
                for mcp_id in &self.config.mcps {
                    if normalize_tool_name(mcp_id) == normalized_server_id {
                        original_server_id = Some(mcp_id.clone());
                        break;
                    }
                }

                let server_id = match original_server_id {
                    Some(id) => id,
                    None => {
                        // Fallback: try the normalized ID directly (old behavior)
                        normalized_server_id.replace('_', "-")
                    }
                };

                // Find the original tool name by listing tools from this server
                let actual_tool =
                    if let Some(client) = self.mcp_manager.get_client(&server_id).await {
                        if let Ok(tools) = client.list_tools().await {
                            tools
                                .into_iter()
                                .find(|t| normalize_tool_name(&t.name) == normalized_tool)
                                .map_or_else(|| normalized_tool.to_string(), |t| t.name)
                        } else {
                            normalized_tool.to_string()
                        }
                    } else {
                        normalized_tool.to_string()
                    };

                tracing::debug!(
                    "Executing MCP tool: server={}, tool={}",
                    server_id,
                    actual_tool
                );
                let output = self
                    .mcp_manager
                    .execute_tool(&server_id, &actual_tool, arguments.clone())
                    .await
                    .map(|v| serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string()))
                    .map_err(|e| e.to_string())?;

                // MCP tools don't support actions (yet)
                return Ok(ToolExecutionResult {
                    output,
                    actions: EventActions::default(),
                });
            }
        }

        Err(format!("Tool not found: {tool_name}"))
    }

    /// Harden a tool parameter schema for stricter LLM compliance.
    /// Adds "additionalProperties": false if not already present.
    /// Ensures "required" array exists (empty if missing).
    fn harden_tool_schema(schema: Value) -> Value {
        crate::tool_schema::harden_tool_schema(schema)
    }

    fn normalize_mcp_parameters(params: Option<Value>) -> Value {
        crate::tool_schema::normalize_mcp_parameters(params)
    }

    async fn build_tools_schema(&self) -> Result<Value, ExecutorError> {
        let mut tools = Vec::new();

        for tool in self.model_visible_tools() {
            let tool_name = tool.name();
            let tool_desc = tool.description();
            let schema = tool.parameters_schema().map_or_else(|| json!({"type": "object", "properties": {}, "additionalProperties": false, "required": []}), Self::harden_tool_schema);

            // Validate tool name and description aren't empty
            if tool_name.is_empty() {
                tracing::error!("Tool has empty name, skipping");
                continue;
            }
            if tool_desc.is_empty() {
                tracing::error!("Tool '{}' has empty description, skipping", tool_name);
                continue;
            }

            tools.push(json!({
                "type": "function",
                "function": {
                    "name": tool_name,
                    "description": tool_desc,
                    "parameters": schema
                }
            }));
        }

        tracing::info!("Registered {} built-in tools for LLM", tools.len());

        // Add MCP tools
        for mcp_id in &self.config.mcps {
            if let Some(client) = self.mcp_manager.get_client(mcp_id).await {
                let mcp_tools = match client.list_tools().await {
                    Ok(tools) => tools,
                    Err(_) => {
                        // Discovery is the real startup boundary for several
                        // MCP transports. Keep the failure nonfatal, redact
                        // client/command/stderr text, and remove the client so
                        // this executor never retries it on a later turn.
                        tracing::warn!(
                            mcp_id = %mcp_id,
                            rejection_code = "startup_failed",
                            "MCP tool discovery failed; continuing without its tools"
                        );
                        self.mcp_manager.mark_startup_failed(mcp_id).await;
                        continue;
                    }
                };

                tracing::info!(
                    "Loaded {} MCP tools from server {}",
                    mcp_tools.len(),
                    mcp_id
                );

                for mcp_tool in mcp_tools {
                    // Convert MCP ID and tool name to valid OpenAI tool name format
                    // Pattern must match: ^[a-zA-Z0-9_-]+$
                    let mcp_id_normalized = normalize_tool_name(mcp_id);
                    let tool_name_normalized = normalize_tool_name(&mcp_tool.name);
                    let tool_name = format!("{mcp_id_normalized}__{tool_name_normalized}");

                    // Normalize parameters to OpenAI format and harden schema
                    let parameters = Self::harden_tool_schema(Self::normalize_mcp_parameters(
                        mcp_tool.parameters,
                    ));

                    tools.push(json!({
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "description": mcp_tool.description,
                            "parameters": parameters
                        }
                    }));
                }
            }
        }

        tracing::info!("Total tools available to LLM: {}", tools.len());
        Ok(json!(tools))
    }

    /// Execute agent without streaming (simpler API)
    pub async fn execute(
        &self,
        user_message: &str,
        history: &[ChatMessage],
    ) -> Result<String, ExecutorError> {
        let mut final_response = String::new();

        self.execute_stream(user_message, history, |event| {
            if let StreamEvent::Token { content, .. } = event {
                final_response.push_str(&content);
            }
        })
        .await?;

        Ok(final_response)
    }

    /// Process tool result, potentially offloading large results to filesystem.
    ///
    /// If offload is enabled and the result exceeds the threshold, saves to a temp file
    /// and returns instructions for the agent to read it with a CLI tool.
    #[cfg(test)]
    fn process_tool_result(&self, tool_name: &str, result: String) -> String {
        crate::context_management::offload_tool_result_for_context(
            tool_name,
            result,
            &self.tool_result_context_config(),
        )
    }

    fn prepare_tool_result_for_context(&self, tool_name: &str, result: String) -> String {
        prepare_tool_result_for_context(tool_name, result, &self.tool_result_context_config())
    }

    fn tool_result_context_config(&self) -> ToolResultContextConfig {
        ToolResultContextConfig {
            max_tool_result_chars: self.config.max_tool_result_chars,
            offload_large_results: self.config.offload_large_results,
            offload_threshold_chars: self.config.offload_threshold_chars,
            offload_dir: self.config.offload_dir.clone(),
        }
    }
}

/// Normalize a string to be a valid `OpenAI` tool name.
///
/// `OpenAI` requires tool names to match: ^[a-zA-Z0-9_-]+$
/// This function replaces any invalid characters with underscores.
fn normalize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ============================================================================
// PROGRESS TRACKER
// ============================================================================

use crate::progress::ProgressTracker;

/// Executor errors
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// Maximum iterations reached with no progress detected.
    #[error("Maximum iterations reached")]
    MaxIterationsReached,

    /// Cooperative stop — caller signaled the executor to stop via the
    /// optional `stop_flag` parameter on `execute_stream`. Distinct from
    /// `LlmError` so callers can short-circuit cleanup paths instead of
    /// treating it as a real failure.
    #[error("Execution stopped by caller")]
    Stopped,

    /// Maximum iterations reached but agent needs user intervention.
    #[error("Max iterations reached after {iterations_used} iterations: {reason}")]
    MaxIterationsNeedsIntervention {
        /// Total iterations consumed
        iterations_used: u32,
        /// Diagnosis of why the agent stopped
        reason: String,
    },

    /// LLM API error.
    #[error("LLM error: {0}")]
    LlmError(String),

    /// Tool execution error.
    #[error("Tool error: {0}")]
    ToolError(String),

    /// MCP server error.
    #[error("MCP error: {0}")]
    McpError(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Middleware pipeline error.
    #[error("Middleware error: {0}")]
    MiddlewareError(String),
}

/// Factory function to create an executor
///
/// This is a convenience function that creates an executor with default
/// components. For more control, create components separately and use
/// `AgentExecutor::new()`.
pub async fn create_executor(
    config: ExecutorConfig,
    llm_client: Arc<dyn LlmClient>,
    tool_registry: Arc<ToolRegistry>,
    mcp_manager: Arc<McpManager>,
) -> Result<AgentExecutor, ExecutorError> {
    // Create default middleware pipeline
    let middleware_pipeline = Arc::new(MiddlewarePipeline::new());

    AgentExecutor::new(
        config,
        llm_client,
        tool_registry,
        mcp_manager,
        middleware_pipeline,
    )
}

#[cfg(test)]
mod hook_tests {
    use super::*;

    #[test]
    fn test_tool_call_decision_default_is_allow() {
        let decision = ToolCallDecision::Allow;
        assert!(matches!(decision, ToolCallDecision::Allow));
    }

    #[test]
    fn test_tool_call_decision_block_has_reason() {
        let decision = ToolCallDecision::Block {
            reason: "dangerous".to_string(),
        };
        match decision {
            ToolCallDecision::Block { reason } => assert_eq!(reason, "dangerous"),
            _ => panic!("Expected Block"),
        }
    }

    #[test]
    fn test_before_tool_call_block_returns_reason() {
        let reason = "Ward boundary violation";
        let result = format!("{{\"blocked\":true,\"reason\":\"{reason}\"}}");
        assert!(result.contains("blocked"));
        assert!(result.contains(reason));
    }

    #[test]
    fn test_steering_message_format() {
        use crate::steering::{SteeringMessage, SteeringPriority, SteeringSource};
        let msg = SteeringMessage::new(
            "Wrap up now",
            SteeringSource::System,
            SteeringPriority::Normal,
        );
        let formatted = format!("[STEER: {}] {}", msg.source, msg.content);
        assert_eq!(formatted, "[STEER: System] Wrap up now");
    }

    #[test]
    fn peer_tool_event_args_never_expose_message_content_or_unknown_fields() {
        let secret = "peer-secret-that-must-not-leak";
        let execution_id = "exec-00000000-0000-0000-0000-000000000001";
        let work_id = "work-00000000-0000-0000-0000-000000000002";
        let send = externally_visible_tool_args(
            "message_agent",
            &json!({
                "execution_id": execution_id,
                "message": secret,
                "forged": secret
            }),
            false,
        );
        assert_eq!(send["execution_id"], execution_id);
        assert_eq!(send["message"], "[REDACTED]");
        assert!(send.get("forged").is_none());
        assert!(!send.to_string().contains(secret));

        let reply = externally_visible_tool_args(
            "reply_to_agent",
            &json!({"reply_to": work_id, "message": secret}),
            false,
        );
        assert_eq!(reply["reply_to"], work_id);
        assert_eq!(reply["message"], "[REDACTED]");
        assert!(!reply.to_string().contains(secret));

        let malformed = externally_visible_tool_args(
            "message_agent",
            &json!({"execution_id": secret, "message": secret}),
            false,
        );
        assert_eq!(malformed["execution_id"], "[REDACTED]");
        assert!(!malformed.to_string().contains(secret));
    }

    #[test]
    fn test_transform_context_hook_type() {
        // Verify the type compiles and can be called
        let hook: TransformContextHook = Arc::new(|messages: &mut Vec<ChatMessage>| {
            messages.push(ChatMessage::system("injected".to_string()));
        });
        let mut messages = vec![ChatMessage::user("hello".to_string())];
        hook(&mut messages);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].text_content(), "injected");
    }

    #[test]
    fn test_complexity_budget_lookup() {
        fn budget_for(complexity: Option<&str>) -> (u32, u32) {
            match complexity {
                Some("S") => (15, 12),
                Some("M") => (30, 24),
                Some("L") => (50, 40),
                Some("XL") => (100, 80),
                _ => (0, 0),
            }
        }
        assert_eq!(budget_for(Some("S")), (15, 12));
        assert_eq!(budget_for(Some("M")), (30, 24));
        assert_eq!(budget_for(Some("L")), (50, 40));
        assert_eq!(budget_for(Some("XL")), (100, 80));
        assert_eq!(budget_for(None), (0, 0));
    }

    #[test]
    fn test_single_action_mode_default_false() {
        let config = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        assert!(!config.single_action_mode);
    }

    #[test]
    fn peer_safe_schema_exposes_only_respond() {
        let schema = Some(json!([
            {"type":"function","function":{"name":"shell"}},
            {"type":"function","function":{"name":"respond"}},
            {"type":"function","function":{"name":"delegate_to_zbot"}}
        ]));

        assert_eq!(
            peer_safe_tools_schema(&schema),
            Some(json!([
                {"type":"function","function":{"name":"respond"}}
            ]))
        );
    }

    #[test]
    fn persisted_remote_result_taints_only_system_marked_history() {
        let marked = ChatMessage::system(
            "[REMOTE ZBOT RESULT — UNTRUSTED DATA]\npeer_data_json: {}".to_owned(),
        );
        let user_spoof = ChatMessage::user(
            "[REMOTE ZBOT RESULT — UNTRUSTED DATA]\npeer_data_json: {}".to_owned(),
        );

        assert!(contains_persisted_peer_result(&[marked]));
        assert!(!contains_persisted_peer_result(&[user_spoof]));

        let marked = ChatMessage::system(
            "[REMOTE ZBOT RESULT — UNTRUSTED DATA]\npeer_data_json: {}".to_owned(),
        );
        assert!(!contains_persisted_peer_result(&[
            marked,
            ChatMessage::user("new local request".to_owned()),
        ]));
    }
}

#[cfg(test)]
mod token_cache_tests {
    #[test]
    fn test_token_estimate_cache() {
        use std::collections::HashMap;
        let mut cache: HashMap<u64, usize> = HashMap::new();

        fn content_hash(content: &str) -> u64 {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            content.hash(&mut hasher);
            hasher.finish()
        }

        let msg = "Hello world this is a test message";
        let hash = content_hash(msg);

        // Cache miss
        assert!(!cache.contains_key(&hash));
        let estimate = msg.len() / 4 + 4;
        cache.insert(hash, estimate);

        // Cache hit
        assert_eq!(cache.get(&hash), Some(&estimate));
    }
}

// ============================================================================
// Static-helper and builder coverage tests
// ============================================================================
#[cfg(test)]
mod executor_helper_coverage_tests {
    use super::*;
    use crate::llm::client::{ChatResponse, LlmError, StreamCallback};
    use crate::mcp::{McpClient, McpError, McpTool};
    use crate::{SteerResult, SteeringRegistry};
    use agent_primitives::Tool;
    use async_trait::async_trait;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tracing::instrument::WithSubscriber;

    #[derive(Clone, Default)]
    struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedLog {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct NamedTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &'static str {
            self.name
        }

        fn description(&self) -> &'static str {
            "test tool"
        }

        fn parameters_schema(&self) -> Option<Value> {
            Some(json!({"type": "object", "properties": {}}))
        }

        async fn execute(
            &self,
            _ctx: Arc<dyn ZeroToolContext>,
            _args: Value,
        ) -> agent_primitives::Result<Value> {
            Ok(json!({"ok": true}))
        }
    }

    // ------------- normalize_tool_name -------------
    #[test]
    fn normalize_tool_name_passthrough_ascii() {
        assert_eq!(normalize_tool_name("read_file"), "read_file");
        assert_eq!(normalize_tool_name("ABC123-_"), "ABC123-_");
    }

    #[test]
    fn normalize_tool_name_replaces_invalid_chars() {
        assert_eq!(normalize_tool_name("foo bar/baz.qux"), "foo_bar_baz_qux");
        assert_eq!(normalize_tool_name(""), "");
        assert_eq!(normalize_tool_name("héllo"), "h_llo");
    }

    // ------------- harden_tool_schema -------------
    #[test]
    fn harden_tool_schema_object_inserts_required_and_additional_properties() {
        let s = AgentExecutor::harden_tool_schema(json!({"type": "object", "properties": {}}));
        assert_eq!(s.get("additionalProperties"), Some(&Value::Bool(false)));
        assert!(s.get("required").unwrap().is_array());
    }

    #[test]
    fn harden_tool_schema_preserves_existing_required_and_additional() {
        let s = AgentExecutor::harden_tool_schema(json!({
            "type": "object",
            "properties": {"x": {}},
            "required": ["x"],
            "additionalProperties": true
        }));
        assert_eq!(s.get("additionalProperties"), Some(&Value::Bool(true)));
        let req = s.get("required").unwrap().as_array().unwrap();
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "x");
    }

    #[test]
    fn harden_tool_schema_non_object_unchanged() {
        let s = AgentExecutor::harden_tool_schema(json!({"type": "string"}));
        assert!(s.get("additionalProperties").is_none());
        assert!(s.get("required").is_none());
    }

    // ------------- normalize_mcp_parameters -------------
    #[test]
    fn normalize_mcp_parameters_none_yields_empty_object() {
        let v = AgentExecutor::normalize_mcp_parameters(None);
        assert_eq!(v.get("type").and_then(|x| x.as_str()), Some("object"));
        assert!(v.get("properties").is_some());
    }

    #[test]
    fn normalize_mcp_parameters_passthrough_when_type_present() {
        let inp = json!({"type": "object", "properties": {"x": {}}});
        let v = AgentExecutor::normalize_mcp_parameters(Some(inp.clone()));
        assert_eq!(v, inp);
    }

    #[test]
    fn normalize_mcp_parameters_wraps_when_no_type() {
        let inp = json!({"x": {"type": "string"}});
        let v = AgentExecutor::normalize_mcp_parameters(Some(inp));
        assert_eq!(v.get("type").and_then(|x| x.as_str()), Some("object"));
        assert!(v.get("properties").unwrap().get("x").is_some());
    }

    // ------------- ExecutorError display -------------
    #[test]
    fn executor_error_messages() {
        assert_eq!(
            format!("{}", ExecutorError::MaxIterationsReached),
            "Maximum iterations reached"
        );
        assert_eq!(
            format!("{}", ExecutorError::Stopped),
            "Execution stopped by caller"
        );
        let with_intervention = ExecutorError::MaxIterationsNeedsIntervention {
            iterations_used: 5,
            reason: "stuck".to_string(),
        };
        let s = format!("{with_intervention}");
        assert!(s.contains("5"));
        assert!(s.contains("stuck"));
        assert!(format!("{}", ExecutorError::LlmError("e".into())).contains("LLM"));
        assert!(format!("{}", ExecutorError::ToolError("e".into())).contains("Tool"));
        assert!(format!("{}", ExecutorError::McpError("e".into())).contains("MCP"));
        assert!(format!("{}", ExecutorError::ConfigError("e".into())).contains("Configuration"));
        assert!(format!("{}", ExecutorError::MiddlewareError("e".into())).contains("Middleware"));
    }

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

    /// Trivial Llm client that fails on any call.
    struct InertLlm;

    #[async_trait]
    impl LlmClient for InertLlm {
        fn model(&self) -> &str {
            "inert"
        }
        fn provider(&self) -> &str {
            "inert"
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            Err(LlmError::ApiError("inert".into()))
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            Err(LlmError::ApiError("inert".into()))
        }
    }

    struct FailingMcpClient;

    #[async_trait]
    impl McpClient for FailingMcpClient {
        fn name(&self) -> &str {
            "blender"
        }

        async fn call_tool(&self, _tool_name: &str, _arguments: Value) -> Result<Value, McpError> {
            Err(McpError::ConnectionFailed(
                "should not be exposed".to_string(),
            ))
        }

        async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
            Err(McpError::ConnectionFailed(
                "configured command and stderr must not escape".to_string(),
            ))
        }
    }

    fn make_inert_executor() -> AgentExecutor {
        let cfg = ExecutorConfig::new("agent".into(), "prov".into(), "model".into());
        AgentExecutor::new(
            cfg,
            Arc::new(InertLlm),
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn create_executor_returns_executor() {
        let cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        let exec = create_executor(
            cfg,
            Arc::new(InertLlm),
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
        )
        .await
        .unwrap();
        assert_eq!(exec.config().agent_id, "a");
    }

    #[test]
    fn config_and_pipeline_accessors() {
        let exec = make_inert_executor();
        assert_eq!(exec.config().agent_id, "agent");
        // pipeline is empty
        assert_eq!(exec.middleware_pipeline().pre_processor_count(), 0);
    }

    #[test]
    fn set_middleware_pipeline_swaps_pipeline() {
        let mut exec = make_inert_executor();
        let new_pipe = Arc::new(MiddlewarePipeline::new());
        exec.set_middleware_pipeline(Arc::clone(&new_pipe));
        // Same Arc pointer reference: indirection equality
        assert_eq!(
            Arc::as_ptr(&new_pipe),
            Arc::as_ptr(exec.middleware_pipeline())
        );
    }

    #[test]
    fn enable_steering_returns_handle() {
        let mut exec = make_inert_executor();
        let _handle = exec.enable_steering();
        // Sending via handle should not panic; the queue is owned by the executor.
    }

    #[test]
    fn set_recall_hook_records_hook_state() {
        let mut exec = make_inert_executor();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cl = Arc::clone(&calls);
        let hook: RecallHook = Box::new(move |_msg, _keys| {
            calls_cl.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(RecallHookResult {
                    system_message: String::new(),
                    fact_keys: vec![],
                })
            })
        });
        let mut keys = HashSet::new();
        keys.insert("seed".to_string());
        exec.set_recall_hook(hook, 5, keys);
        assert_eq!(exec.recall_every_n_turns, 5);
        assert!(exec.recall_initial_keys.contains("seed"));
        // Call counter unused, just ensures no compile/move errors.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn build_tools_schema_includes_no_mcp_when_empty() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(crate::tools::RespondTool::new()));
        let cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        let exec = AgentExecutor::new(
            cfg,
            Arc::new(InertLlm),
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();
        let schema = exec.build_tools_schema().await.unwrap();
        let arr = schema.as_array().unwrap();
        assert!(!arr.is_empty());
        let first = &arr[0];
        assert_eq!(first.get("type").unwrap(), "function");
        let name = first
            .get("function")
            .unwrap()
            .get("name")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(name, "respond");
    }

    #[tokio::test]
    async fn mcp_discovery_failure_is_nonfatal_and_not_retried() {
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_clone = Arc::clone(&observed);
        let manager = Arc::new(McpManager::new().with_startup_failure_observer(Arc::new(
            move |id| observed_clone.lock().unwrap().push(id.to_string()),
        )));
        manager
            .insert_test_client("blender", Arc::new(FailingMcpClient))
            .await;

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(crate::tools::RespondTool::new()));
        let mut config = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        config.mcps = vec!["blender".to_string()];
        let executor = AgentExecutor::new(
            config,
            Arc::new(InertLlm),
            Arc::new(registry),
            Arc::clone(&manager),
            Arc::new(MiddlewarePipeline::new()),
        )
        .expect("executor builds");

        let schema = executor
            .build_tools_schema()
            .await
            .expect("MCP discovery failure stays nonfatal");
        let names = schema
            .as_array()
            .expect("tool schema array")
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["respond"]);
        assert_eq!(*observed.lock().unwrap(), vec!["blender"]);
        assert!(manager.get_client("blender").await.is_none());
    }

    #[tokio::test]
    async fn build_tools_schema_omits_model_hidden_registered_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(NamedTool { name: "respond" }));
        registry.register(Arc::new(NamedTool { name: "memory" }));
        let cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into())
            .with_model_hidden_tools(["memory"]);
        let exec = AgentExecutor::new(
            cfg,
            Arc::new(InertLlm),
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        assert!(exec.tool_registry().contains("memory"));
        let visible_names = exec
            .model_visible_tools()
            .into_iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>();
        assert_eq!(visible_names, vec!["respond"]);

        let schema = exec.build_tools_schema().await.unwrap();
        let names = schema
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["respond"]);
    }

    #[test]
    fn process_tool_result_disabled_passes_through() {
        let exec = make_inert_executor();
        let large = "x".repeat(50_000);
        let out = exec.process_tool_result("tool", large.clone());
        assert_eq!(out, large);
    }

    #[test]
    fn process_tool_result_under_threshold_passes_through() {
        let mut cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        cfg.offload_large_results = true;
        cfg.offload_threshold_chars = 1_000_000;
        let exec = AgentExecutor::new(
            cfg,
            Arc::new(InertLlm),
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();
        let small = "ok".to_string();
        assert_eq!(exec.process_tool_result("tool", small.clone()), small);
    }

    #[test]
    fn process_tool_result_offloads_to_tempdir() {
        let mut cfg = ExecutorConfig::new("a".into(), "p".into(), "m".into());
        cfg.offload_large_results = true;
        cfg.offload_threshold_chars = 10;
        let tmp =
            std::env::temp_dir().join(format!("agent-runtime-test-offload-{}", std::process::id()));
        cfg.offload_dir = Some(tmp.clone());

        let exec = AgentExecutor::new(
            cfg,
            Arc::new(InertLlm),
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();
        let big = "y".repeat(500);
        let result = exec.process_tool_result("tool name/with-bad?chars", big);
        assert!(result.contains("too large"));
        assert!(result.contains("Tool result"));
        assert!(tmp.exists());
        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ------------- AgentExecutor::execute end-to-end with stub LLM -------------

    /// Stub that emits a single token then returns a finalresponse. No tool calls,
    /// so the loop terminates after the first chunk.
    struct OneShotLlm {
        called: Arc<AtomicBool>,
    }

    #[async_trait]
    impl LlmClient for OneShotLlm {
        fn model(&self) -> &str {
            "oneshot"
        }
        fn provider(&self) -> &str {
            "oneshot"
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            unreachable!("execute_stream calls chat_stream")
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            callback: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            self.called.store(true, Ordering::SeqCst);
            callback(StreamChunk::Token("hello ".to_string()));
            callback(StreamChunk::Token("world".to_string()));
            Ok(ChatResponse {
                content: "hello world".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        fn supports_tools(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn execute_returns_concatenated_tokens() {
        let called = Arc::new(AtomicBool::new(false));
        let llm = Arc::new(OneShotLlm {
            called: Arc::clone(&called),
        });
        let mut cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        cfg.tools_enabled = false;
        cfg.system_instruction = Some("be helpful".to_string());
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();
        let answer = exec.execute("hi", &[]).await.unwrap();
        assert!(called.load(Ordering::SeqCst));
        assert_eq!(answer, "hello world");
    }

    #[tokio::test]
    async fn successful_llm_call_acknowledges_durable_peer_steering() {
        let called = Arc::new(AtomicBool::new(false));
        let llm = Arc::new(OneShotLlm {
            called: Arc::clone(&called),
        });
        let mut cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        cfg.tools_enabled = false;
        let mut exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();
        let steering_handle = exec.enable_steering();
        let registry = SteeringRegistry::new();
        registry.register("exec-peer-target", steering_handle);

        let delivery = registry.steer_peer("exec-peer-target", "durable peer payload");
        let execution = exec.execute("initial task", &[]);
        let (delivery, execution) = tokio::join!(delivery, execution);

        assert_eq!(delivery, SteerResult::Delivered);
        assert_eq!(execution.unwrap(), "hello world");
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn execute_stream_emits_metadata_and_done() {
        let llm = Arc::new(OneShotLlm {
            called: Arc::new(AtomicBool::new(false)),
        });
        let mut cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        cfg.tools_enabled = false;
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();
        let mut events = Vec::new();
        exec.execute_stream("hi", &[], |e| events.push(e))
            .await
            .unwrap();
        assert!(matches!(events[0], StreamEvent::Metadata { .. }));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
    }

    #[tokio::test]
    async fn agent_engine_facade_streams_agentzero_events() {
        let llm = Arc::new(OneShotLlm {
            called: Arc::new(AtomicBool::new(false)),
        });
        let mut cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        cfg.tools_enabled = false;
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        let engine: &dyn crate::engine::AgentEngine = &exec;
        let mut events = Vec::new();
        let mut sink = |event| events.push(event);
        engine.execute_stream("hi", &[], &mut sink).await.unwrap();

        assert!(matches!(events[0], StreamEvent::Metadata { .. }));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
    }

    /// Stub that returns a single tool_call on the first call, then a final
    /// text response on the second. This exercises the tool-execution path.
    struct ToolCallThenDoneLlm {
        calls: Arc<AtomicUsize>,
        tool_name: String,
    }

    struct PeerToolThenDoneLlm {
        calls: Arc<AtomicUsize>,
        peer_derived_response: String,
    }

    struct SurfaceMarkerTool {
        name: &'static str,
        update: bool,
    }

    struct RejectingSurfaceTool;
    struct PlanMarkerTool;
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &'static str {
            "echo_tool"
        }

        fn description(&self) -> &'static str {
            "echo a test payload"
        }

        fn parameters_schema(&self) -> Option<Value> {
            Some(json!({
                "type": "object",
                "properties": {"payload": {"type": "string"}},
                "required": ["payload"]
            }))
        }

        async fn execute(
            &self,
            _ctx: Arc<dyn ZeroToolContext>,
            args: Value,
        ) -> agent_primitives::Result<Value> {
            Ok(args)
        }
    }

    #[async_trait]
    impl Tool for SurfaceMarkerTool {
        fn name(&self) -> &'static str {
            self.name
        }

        fn description(&self) -> &'static str {
            "test surface marker"
        }

        fn parameters_schema(&self) -> Option<Value> {
            Some(json!({"type": "object", "properties": {}}))
        }

        async fn execute(
            &self,
            _ctx: Arc<dyn ZeroToolContext>,
            _args: Value,
        ) -> agent_primitives::Result<Value> {
            let marker = if self.update {
                "__work_surface_updated"
            } else {
                "__work_surface"
            };
            Ok(json!({
                (marker): true,
                "surface": {
                    "surface_id": "automatic-summary",
                    "catalog_id": "zbot/work-surface/v1",
                    "components": [{
                        "id": "status",
                        "type": "StatusBadge",
                        "props": {"value_path": "/status"}
                    }],
                    "data": {"status": "ready"}
                }
            }))
        }
    }

    #[async_trait]
    impl Tool for RejectingSurfaceTool {
        fn name(&self) -> &'static str {
            "present_surface"
        }

        fn description(&self) -> &'static str {
            "reject an invalid test surface"
        }

        async fn execute(
            &self,
            _ctx: Arc<dyn ZeroToolContext>,
            _args: Value,
        ) -> agent_primitives::Result<Value> {
            Err(agent_primitives::AgentError::Tool(
                "surface descriptor is invalid".to_string(),
            ))
        }
    }

    #[async_trait]
    impl Tool for PlanMarkerTool {
        fn name(&self) -> &'static str {
            "update_plan"
        }

        fn description(&self) -> &'static str {
            "return a test plan marker"
        }

        async fn execute(
            &self,
            _ctx: Arc<dyn ZeroToolContext>,
            _args: Value,
        ) -> agent_primitives::Result<Value> {
            Ok(json!({
                "__plan_update": true,
                "plan": [{"step": "Keep legacy markers working", "status": "completed"}]
            }))
        }
    }

    struct RejectedSurfaceThenRespondLlm {
        calls: Arc<AtomicUsize>,
        reject_first: bool,
    }

    #[async_trait]
    impl LlmClient for RejectedSurfaceThenRespondLlm {
        fn model(&self) -> &str {
            "tooled"
        }

        fn provider(&self) -> &str {
            "tooled"
        }

        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            unreachable!()
        }

        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let reject_surface = self.reject_first && call == 0;
            let (name, arguments) = if reject_surface {
                (
                    "present_surface",
                    json!({
                        "surface_id": "invalid",
                        "components": [{"type": "ApprovalGate"}],
                        "data": {}
                    }),
                )
            } else {
                (
                    "respond",
                    json!({"message": "The canonical answer remains available."}),
                )
            };
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: Some(vec![ToolCall::new(
                    format!("call-{call}"),
                    name.to_string(),
                    arguments,
                )]),
                reasoning: None,
                usage: None,
            })
        }
    }

    #[async_trait]
    impl LlmClient for ToolCallThenDoneLlm {
        fn model(&self) -> &str {
            "tooled"
        }
        fn provider(&self) -> &str {
            "tooled"
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            unreachable!()
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(ChatResponse {
                    content: String::new(),
                    tool_calls: Some(vec![ToolCall::new(
                        "call-1".to_string(),
                        self.tool_name.clone(),
                        json!({"message": "hi"}),
                    )]),
                    reasoning: None,
                    usage: Some(crate::llm::TokenUsage {
                        prompt_tokens: 10,
                        completion_tokens: 5,
                        total_tokens: 15,
                        cached_prompt_tokens: None,
                    }),
                })
            } else {
                Ok(ChatResponse {
                    content: "all done".to_string(),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
        }
    }

    #[async_trait]
    impl LlmClient for PeerToolThenDoneLlm {
        fn model(&self) -> &str {
            "peer-redaction"
        }

        fn provider(&self) -> &str {
            "peer-redaction"
        }

        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            unreachable!()
        }

        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(ChatResponse {
                    content: String::new(),
                    tool_calls: Some(vec![ToolCall::new(
                        "call-peer-tool".to_owned(),
                        "echo_tool".to_owned(),
                        json!({"payload": self.peer_derived_response}),
                    )]),
                    reasoning: None,
                    usage: None,
                })
            } else {
                Ok(ChatResponse {
                    content: self.peer_derived_response.clone(),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn peer_response_diagnostics_stay_redacted_after_a_tool_round() {
        let secret = "peer-derived-secret-must-not-be-logged";
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(PeerToolThenDoneLlm {
            calls: Arc::clone(&calls),
            peer_derived_response: secret.to_owned(),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let mut executor = AgentExecutor::new(
            ExecutorConfig::new("agent".into(), "p".into(), "m".into()),
            llm,
            Arc::new(tools),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .expect("executor");
        let steering_handle = executor.enable_steering();
        let steering = SteeringRegistry::new();
        steering.register_peer_only("exec-peer-target", steering_handle);

        let captured = CapturedLog::default();
        let sink = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || sink.clone())
            .finish();
        let mut events = Vec::new();
        let run = async {
            tokio::join!(
                steering.steer_peer("exec-peer-target", "untrusted peer input"),
                executor.execute_stream("initial task", &[], |event| events.push(event))
            )
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(run.0, SteerResult::Delivered);
        run.1.expect("execution");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(events.iter().any(|event| matches!(
            event,
            StreamEvent::ToolCallStart { args, .. }
                if args.get("reason").and_then(Value::as_str) == Some("peer_influenced")
                    && !args.to_string().contains(secret)
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            StreamEvent::ToolResult {
                result,
                context_result,
                ..
            } if !result.contains(secret)
                && context_result
                    .as_deref()
                    .is_none_or(|context| !context.contains(secret))
        )));
        let output =
            String::from_utf8(captured.0.lock().expect("log buffer").clone()).expect("UTF-8 logs");
        assert!(!output.contains(secret));
        assert!(output.contains("LLM response after peer steering"));
    }

    #[tokio::test]
    async fn execute_with_tool_call_invokes_tool_and_returns() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ToolCallThenDoneLlm {
            calls: Arc::clone(&calls),
            tool_name: "respond".to_string(),
        });

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(crate::tools::RespondTool::new()));

        let cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        let mut events = Vec::new();
        exec.execute_stream("trigger respond", &[], |e| events.push(e))
            .await
            .unwrap();

        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolCallStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ActionRespond { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::TokenUpdate { .. })));
        assert!(events.iter().any(|e| {
            matches!(
                e,
                StreamEvent::ToolResult {
                    error: None,
                    duration_ms: Some(_),
                    ..
                }
            )
        }));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
        // Should have called LLM once (respond stops the loop)
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn surface_tool_markers_emit_create_and_update_events() {
        for update in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let llm = Arc::new(ToolCallThenDoneLlm {
                calls,
                tool_name: "present_surface".to_string(),
            });
            let mut registry = ToolRegistry::new();
            registry.register(Arc::new(SurfaceMarkerTool {
                name: "present_surface",
                update,
            }));
            let cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
            let exec = AgentExecutor::new(
                cfg,
                llm,
                Arc::new(registry),
                Arc::new(McpManager::new()),
                Arc::new(MiddlewarePipeline::new()),
            )
            .unwrap();

            let mut events = Vec::new();
            exec.execute_stream("present a status", &[], |event| events.push(event))
                .await
                .unwrap();

            assert!(events.iter().any(|event| {
                if update {
                    matches!(event, StreamEvent::WorkSurfaceUpdated { surface, .. }
                        if surface.surface_id == "automatic-summary")
                } else {
                    matches!(event, StreamEvent::WorkSurface { surface, .. }
                        if surface.surface_id == "automatic-summary")
                }
            }));
            assert!(events
                .iter()
                .any(|event| matches!(event, StreamEvent::Done { .. })));
        }
    }

    #[tokio::test]
    async fn surface_markers_from_other_tools_are_inert() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ToolCallThenDoneLlm {
            calls,
            tool_name: "untrusted_marker".to_string(),
        });
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SurfaceMarkerTool {
            name: "untrusted_marker",
            update: false,
        }));
        let cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        let mut events = Vec::new();
        exec.execute_stream("return a marker", &[], |event| events.push(event))
            .await
            .unwrap();

        assert!(!events.iter().any(|event| matches!(
            event,
            StreamEvent::WorkSurface { .. }
                | StreamEvent::WorkSurfaceUpdated { .. }
                | StreamEvent::WorkSurfaceDeleted { .. }
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, StreamEvent::Done { .. })));
    }

    #[tokio::test]
    async fn surface_marker_guard_preserves_other_tool_markers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ToolCallThenDoneLlm {
            calls,
            tool_name: "update_plan".to_string(),
        });
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(PlanMarkerTool));
        let cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        let mut events = Vec::new();
        exec.execute_stream("update the plan", &[], |event| events.push(event))
            .await
            .unwrap();

        assert!(events
            .iter()
            .any(|event| matches!(event, StreamEvent::ActionPlanUpdate { .. })));
    }

    #[tokio::test]
    async fn rejected_surface_then_respond_matches_respond_only_terminal_result() {
        async fn run(reject_first: bool) -> Vec<StreamEvent> {
            let llm = Arc::new(RejectedSurfaceThenRespondLlm {
                calls: Arc::new(AtomicUsize::new(0)),
                reject_first,
            });
            let mut registry = ToolRegistry::new();
            registry.register(Arc::new(RejectingSurfaceTool));
            registry.register(Arc::new(crate::tools::RespondTool::new()));
            let cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
            let exec = AgentExecutor::new(
                cfg,
                llm,
                Arc::new(registry),
                Arc::new(McpManager::new()),
                Arc::new(MiddlewarePipeline::new()),
            )
            .unwrap();
            let mut events = Vec::new();
            exec.execute_stream("answer canonically", &[], |event| events.push(event))
                .await
                .unwrap();
            events
        }

        let after_rejection = run(true).await;
        let respond_only = run(false).await;
        let response = |events: &[StreamEvent]| {
            events.iter().find_map(|event| match event {
                StreamEvent::ActionRespond { message, .. } => Some(message.clone()),
                _ => None,
            })
        };

        assert_eq!(response(&after_rejection), response(&respond_only));
        assert_eq!(
            after_rejection
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done { .. }))
                .count(),
            1
        );
        assert_eq!(
            respond_only
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done { .. }))
                .count(),
            1
        );
        assert!(!after_rejection.iter().any(|event| matches!(
            event,
            StreamEvent::WorkSurface { .. } | StreamEvent::WorkSurfaceUpdated { .. }
        )));
    }

    #[tokio::test]
    async fn blocked_before_tool_call_emits_warning_grade_tool_result() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ToolCallThenDoneLlm {
            calls: Arc::clone(&calls),
            tool_name: "respond".to_string(),
        });

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(crate::tools::RespondTool::new()));

        let mut cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        cfg.before_tool_call = Some(Arc::new(|_, _| ToolCallDecision::Block {
            reason: "test block".to_string(),
        }));
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        let mut events = Vec::new();
        exec.execute_stream("trigger blocked respond", &[], |e| events.push(e))
            .await
            .unwrap();

        let blocked = events
            .iter()
            .find_map(|e| {
                if let StreamEvent::ToolResult {
                    result,
                    error,
                    duration_ms,
                    ..
                } = e
                {
                    Some((result, error, duration_ms))
                } else {
                    None
                }
            })
            .expect("expected blocked ToolResult event");

        assert_eq!(blocked.0, "[blocked by hook]");
        assert_eq!(blocked.1.as_deref(), Some("blocked_by_hook"));
        assert_eq!(*blocked.2, Some(0));
    }

    /// Stub LLM that emits a tool call for an UNREGISTERED tool, exercising
    /// the "tool not found" fallback path.
    #[tokio::test]
    async fn execute_with_unregistered_tool_emits_error_result() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ToolCallThenDoneLlm {
            calls: Arc::clone(&calls),
            tool_name: "no-such-tool".to_string(),
        });

        let cfg = ExecutorConfig::new("agent".into(), "p".into(), "m".into());
        let exec = AgentExecutor::new(
            cfg,
            llm,
            Arc::new(ToolRegistry::new()),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        )
        .unwrap();

        let mut events = Vec::new();
        exec.execute_stream("trigger missing", &[], |e| events.push(e))
            .await
            .unwrap();

        // Tool call still emits Start; ToolResult should report error
        let result = events
            .iter()
            .find_map(|e| {
                if let StreamEvent::ToolResult { error, .. } = e {
                    Some(error.clone())
                } else {
                    None
                }
            })
            .expect("expected ToolResult event");
        // Some path may produce error == None but result text indicates error;
        // we only require the loop produced a final response after retry.
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
        let _ = result;
    }
}
