//! Rig-backed execution engine behind the [`AgentEngine`] facade.
//!
//! Drives a Rig [`Agent`](rig::agent::Agent) built from a [`RigAgentConfig`],
//! a [`CompletionModel`], and a set of bridged [`ToolDyn`] tools, and maps its
//! multi-turn stream onto AgentZero [`StreamEvent`]s. The existing
//! [`AgentExecutor`](crate::executor::AgentExecutor) stays the live engine;
//! `RigAgentEngine` is the T7 path that will replace it once parity is proven
//! (T11). It is generic over the model so a stub model can drive it in tests
//! without the LLM-client bridge (which lands as a separate T7a step).
//!
//! ## Status (T7)
//!
//! Wired and tested: agent construction, multi-turn stream driving via
//! `stream_chat` (with `ChatMessage`→`Message` history), cooperative stop,
//! streaming-error mapping, and mapping of `MultiTurnStreamItem` onto
//! `StreamEvent` — text→`Token`, reasoning→`Reasoning`, tool-call→
//! `ToolCallStart`, tool-result→`ToolResult`, terminal→`Done`. The
//! `LlmCompletionModel` bridge (`model.rs`) lets Rig drive the real
//! OpenAI-compatible `LlmClient`.
//!
//! Still deferred (see TODOs):
//! - token-usage accounting through the bridge (`TokenUpdate`);
//! - the raw/context/persisted/UI result distinction on `ToolResult`
//!   (currently the model-visible text only) and tool-role history conversion.
//!
//! T7c is wired: [`RigExecutionHook`] surfaces `before_tool_call`
//! (`Block`→`Flow::Skip`) and `after_tool_call` (→`Flow::RewriteResult`), and
//! sets the per-call `function_call_id` from `StepEvent::ToolCall`.
//! `tool_concurrency(1)` keeps the shared context race-free.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use agent_primitives::CallbackContext;
use futures::StreamExt;
use rig::agent::{
    Agent, AgentBuilder, AgentHook, Flow, MultiTurnStreamItem, StepEvent, StreamingError,
};
use rig::completion::message::{ToolResult as RigToolResult, ToolResultContent};
use rig::completion::{CompletionModel, Message};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat};
use rig::tool::{ToolCallExtensions, ToolDyn};
use serde_json::Value;

use crate::engine::{AgentEngine, StreamEventSink};
use crate::executor::{AfterToolCallHook, BeforeToolCallHook, ExecutorError, ToolCallDecision};
use crate::rig_adapter::{RigAgentConfig, SharedToolContext};
use crate::types::events::current_timestamp;
use crate::types::{ChatMessage, StreamEvent};

const DEFAULT_MAX_TURNS: usize = 50;

/// Rig-backed implementation of the gateway-facing [`AgentEngine`] facade.
///
/// The agent is built once at construction (tools baked in) and reused across
/// `execute_*` calls; per-request hidden context is threaded through Rig's
/// `ToolCallExtensions` each run.
pub struct RigAgentEngine<M: CompletionModel> {
    #[allow(dead_code)]
    config: RigAgentConfig,
    agent: Agent<M>,
    shared_context: SharedToolContext,
    max_turns: usize,
}

impl<M: CompletionModel + Send + Sync + 'static> RigAgentEngine<M> {
    /// Build a Rig agent behind the facade.
    ///
    /// `tools` are the already actor-filtered, bridged [`ToolDyn`] set; this
    /// engine performs no executable filtering of its own (that stays in
    /// gateway-execution's actor gating, per AC7).
    #[must_use]
    pub fn new(
        config: RigAgentConfig,
        model: M,
        tools: Vec<Box<dyn ToolDyn>>,
        shared_context: SharedToolContext,
    ) -> Self {
        Self::with_max_turns(config, model, tools, shared_context, DEFAULT_MAX_TURNS)
    }

    /// Same as [`Self::new`] with an explicit multi-turn cap.
    #[must_use]
    pub fn with_max_turns(
        config: RigAgentConfig,
        model: M,
        tools: Vec<Box<dyn ToolDyn>>,
        shared_context: SharedToolContext,
        max_turns: usize,
    ) -> Self {
        Self::build(config, model, tools, shared_context, max_turns, None, None)
    }

    /// Same as [`Self::new`] with before/after-tool hooks (T7c). The hooks map
    /// onto Rig's `Flow` model: `before_tool_call` returning [`ToolCallDecision::Block`]
    /// becomes `Flow::Skip` (the reason is returned to the model as the tool
    /// result), and `after_tool_call` returning a replacement becomes
    /// `Flow::RewriteResult`. The hook also sets the per-call `function_call_id`
    /// on the shared context from `StepEvent::ToolCall`, resolving the race
    /// noted in the T6 review.
    #[must_use]
    pub fn with_tool_hooks(
        config: RigAgentConfig,
        model: M,
        tools: Vec<Box<dyn ToolDyn>>,
        shared_context: SharedToolContext,
        before: Option<BeforeToolCallHook>,
        after: Option<AfterToolCallHook>,
    ) -> Self {
        Self::build(
            config,
            model,
            tools,
            shared_context,
            DEFAULT_MAX_TURNS,
            before,
            after,
        )
    }

    fn build(
        config: RigAgentConfig,
        model: M,
        tools: Vec<Box<dyn ToolDyn>>,
        shared_context: SharedToolContext,
        max_turns: usize,
        before: Option<BeforeToolCallHook>,
        after: Option<AfterToolCallHook>,
    ) -> Self {
        let hook = RigExecutionHook::<M>::new(shared_context.clone(), before, after);
        let agent = AgentBuilder::new(model)
            .preamble(&config.instructions)
            .tools(tools)
            .default_max_turns(max_turns)
            .add_hook(hook)
            .build();
        Self {
            config,
            agent,
            shared_context,
            max_turns,
        }
    }

    /// Drive the Rig agent stream and map it onto [`StreamEvent`]s.
    ///
    /// `stop_flag` enables cooperative cancellation: when set, the loop breaks
    /// after the current item and finalizes with whatever was accumulated.
    async fn run(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        stop_flag: Option<Arc<AtomicBool>>,
        on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        let prompt = Message::user(user_message.to_string());
        let chat_history = convert_history(history);

        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(self.shared_context.clone());

        // Awaiting the `StreamingPromptRequest` IntoFuture yields the agent
        // stream directly: `Stream<Item = Result<MultiTurnStreamItem, _>>`.
        // `tool_concurrency(1)` keeps tool execution sequential within a turn,
        // matching the legacy executor and keeping the shared `ToolContext`'s
        // per-call state (e.g. function_call_id) race-free until T7c moves it
        // onto a proper per-call carrier.
        let mut stream = self
            .agent
            .stream_chat(prompt, chat_history)
            .tool_extensions(extensions)
            .multi_turn(self.max_turns)
            .tool_concurrency(1)
            .await;

        let mut final_message = String::new();
        let mut total_input: u64 = 0;
        let mut total_output: u64 = 0;
        while let Some(item) = stream.next().await {
            if let Some(flag) = &stop_flag {
                if flag.load(Ordering::Acquire) {
                    break;
                }
            }
            let item = match item {
                Ok(item) => item,
                Err(error) => return Err(map_streaming_error(error)),
            };
            match item {
                MultiTurnStreamItem::StreamAssistantItem(content) => match content {
                    StreamedAssistantContent::Text(text) => {
                        final_message.push_str(&text.text);
                        on_event(StreamEvent::Token {
                            timestamp: current_timestamp(),
                            content: text.text,
                        });
                    }
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        let tool_id = tool_call
                            .call_id
                            .clone()
                            .unwrap_or_else(|| tool_call.id.clone());
                        on_event(StreamEvent::ToolCallStart {
                            timestamp: current_timestamp(),
                            tool_id,
                            tool_name: tool_call.function.name.clone(),
                            args: tool_call.function.arguments.clone(),
                        });
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        on_event(StreamEvent::Reasoning {
                            timestamp: current_timestamp(),
                            content: reasoning,
                        });
                    }
                    other => {
                        // Deltas / complete-reasoning / unknown low-level items
                        // are folded into the aggregated assistant message by
                        // Rig; full surfacing rides on the AgentHook (T7c).
                        let _ = other;
                    }
                },
                MultiTurnStreamItem::StreamUserItem(user_content) => match user_content {
                    StreamedUserContent::ToolResult { tool_result, .. } => {
                        let result_text = tool_result_text(&tool_result);
                        on_event(StreamEvent::ToolResult {
                            timestamp: current_timestamp(),
                            tool_id: tool_result.id.clone(),
                            result: result_text.clone(),
                            context_result: None,
                            error: None,
                            duration_ms: None,
                        });
                        // Surface tool side-effects set on the shared context
                        // (delegate/respond), mirroring the legacy executor. Without
                        // ActionDelegate, delegate_to_agent would not spawn a child
                        // and wait_agent would hang forever on the Rig path.
                        let actions = self.shared_context.take_actions();
                        if let Some(delegate) = actions.delegate {
                            on_event(StreamEvent::ActionDelegate {
                                timestamp: current_timestamp(),
                                agent_id: delegate.agent_id,
                                task: delegate.task,
                                context: delegate.context,
                                wait_for_result: delegate.wait_for_result,
                                max_iterations: delegate.max_iterations,
                                output_schema: delegate.output_schema,
                                skills: delegate.skills,
                                complexity: delegate.complexity,
                                mode: delegate.mode,
                                parallel: delegate.parallel,
                                child_execution_id: delegate.child_execution_id,
                            });
                        }
                        if let Some(respond) = actions.respond {
                            on_event(StreamEvent::ActionRespond {
                                timestamp: current_timestamp(),
                                message: respond.message,
                                format: respond.format,
                                conversation_id: respond.conversation_id,
                                session_id: respond.session_id,
                                artifacts: respond.artifacts,
                            });
                        }
                        // Surface result-value markers. Ward/update_plan/title
                        // marker producers signal via their return JSON
                        // (`__ward_changed__`/`__plan_update`/`__session_title_changed__`
                        // + payload fields); the legacy executor parses the tool
                        // output. Without this, legacy marker producers never
                        // publish the corresponding stream events.
                        if let Ok(parsed) = serde_json::from_str::<Value>(&result_text) {
                            if parsed
                                .get("__session_title_changed__")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                if let Some(title) = parsed.get("title").and_then(Value::as_str) {
                                    on_event(StreamEvent::SessionTitleChanged {
                                        timestamp: current_timestamp(),
                                        title: title.to_string(),
                                    });
                                }
                            }
                            if parsed
                                .get("__ward_changed__")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                if let Some(ward_id) = parsed.get("ward_id").and_then(Value::as_str)
                                {
                                    on_event(StreamEvent::WardChanged {
                                        timestamp: current_timestamp(),
                                        ward_id: ward_id.to_string(),
                                    });
                                }
                            }
                            if parsed
                                .get("__plan_update")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                let plan = parsed
                                    .get("plan")
                                    .cloned()
                                    .unwrap_or_else(|| Value::Array(Vec::new()));
                                let explanation = parsed
                                    .get("explanation")
                                    .and_then(Value::as_str)
                                    .map(std::string::ToString::to_string);
                                on_event(StreamEvent::ActionPlanUpdate {
                                    timestamp: current_timestamp(),
                                    plan,
                                    explanation,
                                });
                            }
                        }
                    }
                },
                MultiTurnStreamItem::CompletionCall(cc) => {
                    // Emit token usage (cumulative) so the gateway records
                    // per-execution token counts via TokenUpdate → batch_writer.
                    total_input += cc.usage.input_tokens;
                    total_output += cc.usage.output_tokens;
                    on_event(StreamEvent::TokenUpdate {
                        timestamp: current_timestamp(),
                        tokens_in: total_input,
                        tokens_out: total_output,
                    });
                }
                MultiTurnStreamItem::FinalResponse(_) => {
                    // Terminal; final_message accumulated from tokens above.
                }
                // `MultiTurnStreamItem` is #[non_exhaustive]; future variants
                // are ignored until the full mapping lands.
                _ => {}
            }
        }

        on_event(StreamEvent::Done {
            timestamp: current_timestamp(),
            final_message,
            token_count: (total_input + total_output) as usize,
        });
        Ok(())
    }
}

/// Convert AgentZero chat history into rig `Message`s (text-first).
///
/// Tool-result (`role: "tool"`) and multimodal content are not yet converted;
/// that fidelity rides on the remaining T7 work.
fn convert_history(history: &[ChatMessage]) -> Vec<Message> {
    history
        .iter()
        .filter_map(|message| match message.role.as_str() {
            "user" => Some(Message::user(message.text_content())),
            "assistant" => Some(Message::assistant(message.text_content())),
            "system" => Some(Message::system(message.text_content())),
            _ => None,
        })
        .collect()
}

/// Extract the model-visible text from a rig tool result.
fn tool_result_text(tool_result: &RigToolResult) -> String {
    tool_result
        .content
        .iter()
        .filter_map(|content| match content {
            ToolResultContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait::async_trait]
impl<M: CompletionModel + Send + Sync + 'static> AgentEngine for RigAgentEngine<M> {
    async fn execute_stream(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        self.run(user_message, history, None, on_event).await
    }

    async fn execute_stream_with_stop_flag(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        stop_flag: Option<Arc<AtomicBool>>,
        on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        self.run(user_message, history, stop_flag, on_event).await
    }

    async fn execute(
        &self,
        user_message: &str,
        history: &[ChatMessage],
    ) -> Result<String, ExecutorError> {
        let mut accumulated = String::new();
        self.run(user_message, history, None, &mut |event| {
            if let StreamEvent::Token { content, .. } = &event {
                accumulated.push_str(content);
            }
        })
        .await?;
        Ok(accumulated)
    }

    fn engine_name(&self) -> &'static str {
        "rig"
    }
}

/// Map a Rig streaming error onto the AgentZero executor error.
///
/// Kept coarse for the first slice: tool/completion/prompt failures all surface
/// as `ExecutorError::LlmError`. T7c may split these once the engine handles
/// tool errors distinctly.
fn map_streaming_error(error: StreamingError) -> ExecutorError {
    ExecutorError::LlmError(error.to_string())
}

/// AgentZero execution hook bridging before/after-tool behavior onto Rig's
/// [`Flow`] model and threading the per-call function-call id onto the shared
/// [`ToolContext`].
///
/// - `StepEvent::ToolCall` sets `function_call_id` (resolving the T6 race —
///   `tool_concurrency(1)` keeps the shared context safe), then applies
///   `before_tool_call`; [`ToolCallDecision::Block`] becomes [`Flow::skip`].
/// - `StepEvent::ToolResult` applies `after_tool_call`; a returned replacement
///   becomes [`Flow::rewrite_result`] (model-visible only — the real result
///   still ran).
struct RigExecutionHook<M: CompletionModel> {
    ctx: SharedToolContext,
    before: Option<BeforeToolCallHook>,
    after: Option<AfterToolCallHook>,
    _marker: PhantomData<M>,
}

impl<M: CompletionModel> RigExecutionHook<M> {
    fn new(
        ctx: SharedToolContext,
        before: Option<BeforeToolCallHook>,
        after: Option<AfterToolCallHook>,
    ) -> Self {
        Self {
            ctx,
            before,
            after,
            _marker: PhantomData,
        }
    }
}

impl<M: CompletionModel> AgentHook<M> for RigExecutionHook<M> {
    async fn on_event(&self, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::ToolCall {
                tool_name,
                tool_call_id,
                args,
                ..
            } => {
                if let Some(id) = tool_call_id.filter(|id: &&str| !id.is_empty()) {
                    self.ctx.set_function_call_id((*id).to_string());
                }
                if let Some(before) = &self.before {
                    let args_value = serde_json::from_str::<Value>(args).unwrap_or(Value::Null);
                    if let ToolCallDecision::Block { reason } = before(tool_name, &args_value) {
                        return Flow::skip(reason);
                    }
                }
                Flow::cont()
            }
            StepEvent::CompletionCall { .. } => {
                // Mirror the legacy executor's per-turn reset of the
                // delegation claim. Without this, the first delegation's
                // `app:delegation_active=true` is never released on the Rig
                // path, so every subsequent `delegate_to_agent` is blocked
                // with "You already have an active delegation" and the root
                // deadlocks looping on queued delegations that never spawn.
                self.ctx
                    .set_state("app:delegation_active".to_string(), Value::Bool(false));
                Flow::cont()
            }
            StepEvent::ToolResult {
                tool_name,
                args,
                result,
                ..
            } => {
                if let Some(after) = &self.after {
                    let args_value = serde_json::from_str::<Value>(args).unwrap_or(Value::Null);
                    // rig's ToolResult fires for completed calls; the legacy
                    // executor calls after_tool_call with succeeded=true on
                    // this path, so we match it.
                    if let Some(replacement) = after(tool_name, &args_value, result, true) {
                        return Flow::rewrite_result(replacement);
                    }
                }
                Flow::cont()
            }
            _ => Flow::cont(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rig_adapter::RigToolAdapter;
    use rig::completion::{
        AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
        Usage,
    };
    use rig::one_or_many::OneOrMany;
    use rig::streaming::{RawStreamingChoice, StreamingCompletionResponse};
    use std::sync::atomic::AtomicU32;
    use std::sync::Mutex;

    /// Stub completion model that streams a fixed sequence of text chunks then
    /// a final-response marker. `type Response = type StreamingResponse = ()`
    /// because `()` already implements [`rig::completion::GetTokenUsage`].
    #[derive(Clone)]
    struct StubModel {
        chunks: Vec<String>,
    }

    impl StubModel {
        fn text(chunks: &[&str]) -> Self {
            Self {
                chunks: chunks.iter().map(|c| (*c).to_string()).collect(),
            }
        }
    }

    impl CompletionModel for StubModel {
        type Response = ();
        type StreamingResponse = ();
        type Client = ();

        fn make(_: &Self::Client, _: impl Into<String>) -> Self {
            Self { chunks: Vec::new() }
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            Ok(CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text(self.chunks.join(""))),
                usage: Usage::new(),
                raw_response: (),
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            let mut choices: Vec<Result<RawStreamingChoice<()>, CompletionError>> = self
                .chunks
                .iter()
                .map(|c| Ok(RawStreamingChoice::Message(c.clone())))
                .collect();
            // Terminal marker so the agent loop finalizes cleanly.
            choices.push(Ok(RawStreamingChoice::FinalResponse(())));
            let stream = futures::stream::iter(choices);
            Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
        }
    }

    fn sample_config() -> RigAgentConfig {
        use crate::rig_adapter::RigModelConfig;
        RigAgentConfig::new(
            "agent-1",
            "Agent",
            "test agent",
            "You are a test agent.".to_string(),
            RigModelConfig {
                provider_id: "p".into(),
                base_url: "https://llm.local/v1".into(),
                api_key: "sk-test".into(),
                model: "m".into(),
                temperature: 0.0,
                max_tokens: 100,
                context_window_tokens: 1_000,
                thinking_enabled: false,
                provider_params: None,
            },
        )
    }

    async fn collect_events(engine: &RigAgentEngine<StubModel>, prompt: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        engine
            .execute_stream(prompt, &[], &mut |event| events.push(event))
            .await
            .expect("engine stream should complete");
        events
    }

    #[tokio::test]
    async fn streams_tokens_then_done_for_simple_chat() {
        let engine = RigAgentEngine::new(
            sample_config(),
            StubModel::text(&["hel", "lo", " world"]),
            Vec::new(),
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let events = collect_events(&engine, "hi").await;
        assert!(!events.is_empty());

        // Token events preserve model order.
        let tokens: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Token { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tokens,
            vec!["hel".to_string(), "lo".to_string(), " world".to_string()]
        );

        // Terminal Done carries the concatenated text.
        let done = events.iter().rev().find(|e| e.is_terminal());
        assert!(
            matches!(done, Some(StreamEvent::Done { final_message, .. }) if final_message == "hello world"),
            "expected terminal Done with concatenated text, got {done:?}"
        );
    }

    #[tokio::test]
    async fn empty_model_still_finalizes() {
        let engine = RigAgentEngine::new(
            sample_config(),
            StubModel::text(&[]),
            Vec::new(),
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let events = collect_events(&engine, "hi").await;
        assert!(events.iter().any(
            |e| matches!(e, StreamEvent::Done { final_message, .. } if final_message.is_empty())
        ));
    }

    #[tokio::test]
    async fn stop_flag_breaks_after_current_item() {
        let engine = RigAgentEngine::new(
            sample_config(),
            StubModel::text(&["a", "b", "c"]),
            Vec::new(),
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_closure = stop.clone();
        let mut events = Vec::new();
        engine
            .execute_stream_with_stop_flag("hi", &[], Some(stop.clone()), &mut |event| {
                let is_token = matches!(event, StreamEvent::Token { .. });
                events.push(event);
                // Stop after the first token lands.
                if is_token {
                    stop_for_closure.store(true, Ordering::Release);
                }
            })
            .await
            .expect("stopped run should still finalize");

        let tokens: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Token { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        // The stop flag is checked before polling the next item, so exactly one
        // token is emitted before the loop breaks and finalizes.
        assert_eq!(tokens, vec!["a".to_string()]);
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
    }

    // End-to-end: the real LlmCompletionModel bridge over a stub AgentZero
    // LlmClient, driven through RigAgentEngine. Proves LlmClient -> Rig agent
    // loop -> StreamEvent without any real network call.
    #[tokio::test]
    async fn llm_completion_model_drives_engine_end_to_end() {
        use crate::llm::{ChatResponse, LlmClient, LlmError, StreamCallback, StreamChunk};
        use crate::rig_adapter::model::LlmCompletionModel;
        use serde_json::Value;

        struct StubLlm {
            chunks: Vec<String>,
        }
        #[async_trait::async_trait]
        impl LlmClient for StubLlm {
            fn model(&self) -> &str {
                "stub"
            }
            fn provider(&self) -> &str {
                "stub"
            }
            async fn chat(
                &self,
                _messages: Vec<ChatMessage>,
                _tools: Option<Value>,
            ) -> Result<ChatResponse, LlmError> {
                Ok(ChatResponse {
                    content: self.chunks.join(""),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
            async fn chat_stream(
                &self,
                _messages: Vec<ChatMessage>,
                _tools: Option<Value>,
                callback: StreamCallback,
            ) -> Result<ChatResponse, LlmError> {
                for chunk in &self.chunks {
                    callback(StreamChunk::Token(chunk.clone()));
                }
                Ok(ChatResponse {
                    content: self.chunks.join(""),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
        }

        let client: Arc<dyn LlmClient> = Arc::new(StubLlm {
            chunks: vec!["ri".to_string(), "gged".to_string()],
        });
        let model = LlmCompletionModel::new(client, "stub");
        let engine = RigAgentEngine::new(
            sample_config(),
            model,
            Vec::new(),
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let mut events = Vec::new();
        engine
            .execute_stream("hi", &[], &mut |event| events.push(event))
            .await
            .expect("engine should complete");

        let tokens: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Token { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, vec!["ri".to_string(), "gged".to_string()]);
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::Done { final_message, .. } if final_message == "rigged"
        )));
    }

    // T7c: the AgentHook surfaces before_tool_call (Block -> Skip, tool not run)
    // and threads the per-call function_call_id.
    #[tokio::test]
    async fn before_tool_call_block_prevents_execution() {
        use std::sync::atomic::AtomicU32;

        let calls = Arc::new(AtomicU32::new(0));
        let tool = RigToolAdapter::boxed(Arc::new(RecordingTool::new("recorder", &calls)));
        let before: BeforeToolCallHook = Arc::new(|_name, _args| ToolCallDecision::Block {
            reason: "blocked".to_string(),
        });

        let engine = RigAgentEngine::with_tool_hooks(
            sample_config(),
            ToolCallModel::new("recorder"),
            vec![tool],
            Arc::new(crate::tools::context::ToolContext::default()),
            Some(before),
            None,
        );

        let mut events = Vec::new();
        engine
            .execute_stream("hi", &[], &mut |event| events.push(event))
            .await
            .expect("run");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a blocked tool call must not execute"
        );
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
    }

    #[tokio::test]
    async fn hook_runs_tool_and_sets_call_id_when_allowed() {
        use std::sync::atomic::AtomicU32;

        let calls = Arc::new(AtomicU32::new(0));
        let fcid = Arc::new(Mutex::new(None));
        let tool = RigToolAdapter::boxed(Arc::new(RecordingTool::with_fcid(
            "recorder", &calls, &fcid,
        )));

        let engine = RigAgentEngine::new(
            sample_config(),
            ToolCallModel::new("recorder"),
            vec![tool],
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let mut events = Vec::new();
        engine
            .execute_stream("hi", &[], &mut |event| events.push(event))
            .await
            .expect("run");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "allowed tool should execute once"
        );
        assert_eq!(
            *fcid.lock().unwrap(),
            Some("call_7".to_string()),
            "function_call_id should be set from the ToolCall hook"
        );
        // The bridged tool result surfaces as a ToolResult event.
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolResult { .. })));
    }

    /// Stub model that emits one tool call (to `tool_name`) on its first turn,
    /// then plain text on subsequent turns. The `emitted` flag is shared across
    /// clones so it survives Rig's model cloning between turns.
    #[derive(Clone)]
    struct ToolCallModel {
        tool_name: String,
        emitted: Arc<AtomicU32>,
    }

    impl ToolCallModel {
        fn new(tool_name: &str) -> Self {
            Self {
                tool_name: tool_name.to_string(),
                emitted: Arc::new(AtomicU32::new(0)),
            }
        }
    }

    impl CompletionModel for ToolCallModel {
        type Response = ();
        type StreamingResponse = ();
        type Client = ();

        fn make(_: &Self::Client, _: impl Into<String>) -> Self {
            Self {
                tool_name: String::new(),
                emitted: Arc::new(AtomicU32::new(0)),
            }
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            Ok(CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text("ok")),
                usage: Usage::new(),
                raw_response: (),
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            use rig::streaming::RawStreamingToolCall;
            // Emit the tool call only on the first turn.
            let first_turn = self.emitted.fetch_or(1, Ordering::SeqCst) == 0;
            let mut choices: Vec<Result<RawStreamingChoice<()>, CompletionError>> = Vec::new();
            if first_turn {
                choices.push(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall {
                    id: "call_7".to_string(),
                    internal_call_id: "call_7".to_string(),
                    call_id: Some("call_7".to_string()),
                    name: self.tool_name.clone(),
                    arguments: serde_json::json!({}),
                    signature: None,
                    additional_params: None,
                })));
            }
            choices.push(Ok(RawStreamingChoice::Message("done".to_string())));
            choices.push(Ok(RawStreamingChoice::FinalResponse(())));
            Ok(StreamingCompletionResponse::stream(Box::pin(
                futures::stream::iter(choices),
            )))
        }
    }

    /// AgentZero tool that records whether it ran and the function_call_id it saw.
    struct RecordingTool {
        name: String,
        calls: Arc<AtomicU32>,
        fcid: Arc<Mutex<Option<String>>>,
    }

    impl RecordingTool {
        fn new(name: &str, calls: &Arc<AtomicU32>) -> Self {
            Self {
                name: name.to_string(),
                calls: calls.clone(),
                fcid: Arc::new(Mutex::new(None)),
            }
        }
        fn with_fcid(
            name: &str,
            calls: &Arc<AtomicU32>,
            fcid: &Arc<Mutex<Option<String>>>,
        ) -> Self {
            Self {
                name: name.to_string(),
                calls: calls.clone(),
                fcid: fcid.clone(),
            }
        }
    }

    #[async_trait::async_trait]
    impl agent_primitives::Tool for RecordingTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "records execution"
        }
        async fn execute(
            &self,
            ctx: Arc<dyn agent_primitives::ToolContext>,
            _args: Value,
        ) -> Result<Value, agent_primitives::error::AgentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.fcid.lock().unwrap() = Some(ctx.function_call_id());
            Ok(serde_json::json!({"ok": true}))
        }
    }

    // T9 / AC13: the Rig path is a faithful conduit for the gateway-owned
    // conversation tape — it forwards history to the LlmClient without silent
    // compaction or loss (live context control stays in AgentZero runtime).
    #[tokio::test]
    async fn rig_engine_forwards_history_to_llm_unchanged() {
        use crate::llm::{ChatResponse, LlmClient, LlmError, StreamCallback, StreamChunk};
        use crate::rig_adapter::model::LlmCompletionModel;
        use serde_json::Value;

        let sent: Arc<Mutex<Vec<Vec<ChatMessage>>>> = Arc::new(Mutex::new(Vec::new()));
        struct RecordingLlm {
            sent: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
        }
        #[async_trait::async_trait]
        impl LlmClient for RecordingLlm {
            fn model(&self) -> &str {
                "stub"
            }
            fn provider(&self) -> &str {
                "stub"
            }
            async fn chat(
                &self,
                messages: Vec<ChatMessage>,
                _tools: Option<Value>,
            ) -> Result<ChatResponse, LlmError> {
                self.sent.lock().unwrap().push(messages);
                Ok(ChatResponse {
                    content: "ok".to_string(),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
            async fn chat_stream(
                &self,
                messages: Vec<ChatMessage>,
                _tools: Option<Value>,
                callback: StreamCallback,
            ) -> Result<ChatResponse, LlmError> {
                self.sent.lock().unwrap().push(messages);
                callback(StreamChunk::Token("ok".to_string()));
                Ok(ChatResponse {
                    content: "ok".to_string(),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            }
        }

        let client: Arc<dyn LlmClient> = Arc::new(RecordingLlm { sent: sent.clone() });
        let engine = RigAgentEngine::new(
            sample_config(),
            LlmCompletionModel::new(client, "stub"),
            Vec::new(),
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let history = vec![
            ChatMessage::user("hello".to_string()),
            ChatMessage::assistant("hi there".to_string()),
        ];
        let mut events = Vec::new();
        engine
            .execute_stream("next", &history, &mut |event| events.push(event))
            .await
            .expect("run");

        let received = sent.lock().unwrap().clone();
        assert_eq!(received.len(), 1, "LlmClient should be called once");
        let texts: Vec<String> = received[0].iter().map(|m| m.text_content()).collect();
        let joined = texts.join(" | ");
        assert!(
            joined.contains("hello"),
            "history user msg forwarded; got {joined}"
        );
        assert!(
            joined.contains("hi there"),
            "history assistant msg forwarded; got {joined}"
        );
        assert!(
            joined.contains("next"),
            "current prompt forwarded; got {joined}"
        );
    }

    // T8 / AC21: a child executor's delegation mode (initial state seeded by the
    // gateway) reaches a bridged tool through the Rig path's SharedToolContext.
    #[tokio::test]
    async fn delegation_mode_flows_to_tool_through_rig_path() {
        use std::collections::HashMap;

        let seen = Arc::new(Mutex::new(None));
        let tool = RigToolAdapter::boxed(Arc::new(ModeProbeTool {
            seen_mode: seen.clone(),
        }));

        // Gateway seeds the child's context with a delegation mode.
        let mut state = HashMap::new();
        state.insert(
            "app:delegation_mode".to_string(),
            serde_json::json!("ward_backed_build"),
        );
        let shared = Arc::new(crate::tools::context::ToolContext::full_with_state(
            "child-agent".to_string(),
            None,
            Vec::new(),
            state,
        ));

        let engine = RigAgentEngine::new(
            sample_config(),
            ToolCallModel::new("mode_probe"),
            vec![tool],
            shared,
        );
        let mut events = Vec::new();
        engine
            .execute_stream("hi", &[], &mut |event| events.push(event))
            .await
            .expect("run");

        assert_eq!(
            *seen.lock().unwrap(),
            Some("ward_backed_build".to_string()),
            "delegation mode must reach the tool through the Rig path"
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolResult { .. })));
    }

    /// Tool that records the `app:delegation_mode` it sees in its context.
    struct ModeProbeTool {
        seen_mode: Arc<Mutex<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl agent_primitives::Tool for ModeProbeTool {
        fn name(&self) -> &str {
            "mode_probe"
        }
        fn description(&self) -> &str {
            "reads delegation mode"
        }
        async fn execute(
            &self,
            ctx: Arc<dyn agent_primitives::ToolContext>,
            _args: Value,
        ) -> Result<Value, agent_primitives::error::AgentError> {
            let mode = agent_primitives::CallbackContext::get_state(&*ctx, "app:delegation_mode");
            *self.seen_mode.lock().unwrap() = mode.and_then(|v| v.as_str().map(str::to_string));
            Ok(serde_json::json!({"ok": true}))
        }
    }

    // T7/Rig-cutover: a tool that sets a delegate action (like delegate_to_agent)
    // must surface ActionDelegate, or the gateway never spawns the child and a
    // later wait_agent hangs forever.
    #[tokio::test]
    async fn action_events_surface_after_tool_runs() {
        struct DelegatingTool;
        #[async_trait::async_trait]
        impl agent_primitives::Tool for DelegatingTool {
            fn name(&self) -> &str {
                "delegate_to_agent"
            }
            fn description(&self) -> &str {
                "delegate"
            }
            async fn execute(
                &self,
                ctx: Arc<dyn agent_primitives::ToolContext>,
                _args: Value,
            ) -> Result<Value, agent_primitives::error::AgentError> {
                let mut actions = ctx.actions();
                actions.delegate = Some(agent_primitives::event::DelegateAction {
                    agent_id: "ward:x".to_string(),
                    task: "do thing".to_string(),
                    context: None,
                    wait_for_result: false,
                    max_iterations: None,
                    output_schema: None,
                    skills: vec![],
                    complexity: None,
                    mode: None,
                    parallel: false,
                    child_execution_id: None,
                });
                ctx.set_actions(actions);
                Ok(serde_json::json!({"delegated": true}))
            }
        }

        let engine = RigAgentEngine::new(
            sample_config(),
            ToolCallModel::new("delegate_to_agent"),
            vec![RigToolAdapter::boxed(Arc::new(DelegatingTool))],
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let mut events = Vec::new();
        engine
            .execute_stream("hi", &[], &mut |event| events.push(event))
            .await
            .expect("run");

        assert!(
            events.iter().any(|event| matches!(
                event,
                StreamEvent::ActionDelegate { agent_id, task, .. }
                    if agent_id == "ward:x" && task == "do thing"
            )),
            "ActionDelegate must surface after the tool runs; got {events:?}"
        );
    }

    // A legacy title marker producer can return
    // `{"__session_title_changed__": true, "title": ...}`; the engine must
    // surface SessionTitleChanged so the gateway persists the title.
    #[tokio::test]
    async fn session_title_marker_surfaces() {
        struct TitleTool;
        #[async_trait::async_trait]
        impl agent_primitives::Tool for TitleTool {
            fn name(&self) -> &str {
                "legacy_title_marker"
            }
            fn description(&self) -> &str {
                "set title"
            }
            async fn execute(
                &self,
                _ctx: Arc<dyn agent_primitives::ToolContext>,
                _args: Value,
            ) -> Result<Value, agent_primitives::error::AgentError> {
                Ok(serde_json::json!({"__session_title_changed__": true, "title": "My Session"}))
            }
        }

        let engine = RigAgentEngine::new(
            sample_config(),
            ToolCallModel::new("legacy_title_marker"),
            vec![RigToolAdapter::boxed(Arc::new(TitleTool))],
            Arc::new(crate::tools::context::ToolContext::default()),
        );

        let mut events = Vec::new();
        engine
            .execute_stream("hi", &[], &mut |event| events.push(event))
            .await
            .expect("run");

        assert!(
            events.iter().any(|event| matches!(
                event,
                StreamEvent::SessionTitleChanged { title, .. } if title == "My Session"
            )),
            "SessionTitleChanged must surface; got {events:?}"
        );
    }
}
