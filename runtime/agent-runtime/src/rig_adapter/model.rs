//! Rig `CompletionModel` adapter over AgentZero's existing `LlmClient`.
//!
//! This is the T7a provider bridge: Rig owns the agent loop, tool dispatch,
//! hooks, and stream aggregation; AgentZero keeps owning the OpenAI-compatible
//! HTTP transport (with its retry and rate-limiter wrappers). The adapter
//! implements Rig's [`CompletionModel`] by driving
//! [`LlmClient::chat_stream`](crate::llm::LlmClient::chat_stream) and bridging
//! its callback-based chunks onto Rig's stream-of-`RawStreamingChoice`.
//!
//! ## Design choices
//!
//! - **Text streams token-by-token; tool calls arrive complete.** AgentZero's
//!   `chat_stream` emits partial `ToolCall` argument fragments during streaming,
//!   but the authoritative complete calls come back in the final
//!   [`ChatResponse`]. Re-accumulating fragments would duplicate provider
//!   parsing, so the bridge forwards `Token`/`Reasoning` chunks live and emits
//!   complete `RawStreamingChoice::ToolCall`s once `chat_stream` resolves.
//! - **`futures::mpsc::unbounded` carries chunks out of the callback.** The
//!   callback is a synchronous `Fn` invoked from inside the async `chat_stream`,
//!   where `tokio::mpsc::blocking_send` would panic; an unbounded channel sends
//!   synchronously without blocking.
//! - **Usage travels in the final response.** Rig reads provider token counts
//!   through `GetTokenUsage` when completing each model call.
//! - **The stream owns the provider task.** Dropping it aborts an outstanding
//!   request, including one that has not produced its first token.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures::Stream;
use rig::completion::message::ToolResultContent;
use rig::completion::{
    AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
    GetTokenUsage, Message, ToolDefinition, Usage,
};
use rig::one_or_many::OneOrMany;
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse};
use serde_json::{json, Value};

use crate::llm::{
    ChatMessage, ChatResponse, LlmClient, LlmError, StreamCallback, StreamChunk, TokenUsage,
};
use crate::types::ToolCall as AgentToolCall;

/// Bridge response type carrying token usage from the AgentZero LlmClient.
/// Implements [`GetTokenUsage`] so Rig's agent loop populates `CompletionCall.usage`.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmCompletionResponse {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
}

impl LlmCompletionResponse {
    fn from_usage(usage: Option<&TokenUsage>) -> Self {
        usage
            .map(|u| Self {
                input_tokens: u.prompt_tokens as u64,
                output_tokens: u.completion_tokens as u64,
                total_tokens: u.total_tokens as u64,
                cached_input_tokens: u.cached_prompt_tokens.unwrap_or(0) as u64,
            })
            .unwrap_or_default()
    }
}

impl GetTokenUsage for LlmCompletionResponse {
    fn token_usage(&self) -> Usage {
        Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            cache_creation_input_tokens: 0,
            tool_use_prompt_tokens: 0,
            reasoning_tokens: 0,
        }
    }
}

/// Rig completion model backed by an AgentZero [`LlmClient`].
#[derive(Clone)]
pub struct LlmCompletionModel {
    client: Arc<dyn LlmClient>,
    #[allow(dead_code)]
    model_id: String,
    single_action_mode: bool,
}

/// Bind callback-driven provider work to the lifetime of its consuming stream.
struct ProviderStream {
    receiver:
        mpsc::UnboundedReceiver<Result<RawStreamingChoice<LlmCompletionResponse>, CompletionError>>,
    task: tokio::task::JoinHandle<()>,
}

impl Stream for ProviderStream {
    type Item = Result<RawStreamingChoice<LlmCompletionResponse>, CompletionError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl Drop for ProviderStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LlmCompletionModel {
    /// Wrap an AgentZero LLM client for use as a Rig completion model.
    #[must_use]
    pub fn new(client: Arc<dyn LlmClient>, model_id: impl Into<String>) -> Self {
        Self {
            client,
            model_id: model_id.into(),
            single_action_mode: false,
        }
    }

    /// Constrain authoritative complete calls before Rig can dispatch siblings.
    pub(super) fn with_single_action_mode(mut self, enabled: bool) -> Self {
        self.single_action_mode = enabled;
        self
    }
}

impl CompletionModel for LlmCompletionModel {
    type Response = LlmCompletionResponse;
    type StreamingResponse = LlmCompletionResponse;
    type Client = super::client::LlmCompletionClient;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(client.client.clone(), model)
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let messages = convert_messages(&request)?;
        let tools = convert_tools(&request.tools);
        let output_schema = request
            .output_schema
            .as_ref()
            .map(|schema| schema.as_value().clone());
        let mut response = self
            .client
            .chat_with_schema(messages, tools, output_schema)
            .await
            .map_err(llm_error_to_completion)?;
        if self.single_action_mode {
            if let Some(calls) = &mut response.tool_calls {
                calls.truncate(1);
            }
        }

        let choice = if let Some(calls) = nonempty_tool_calls(&response) {
            OneOrMany::many(calls).map_err(|_| {
                CompletionError::ProviderError("tool call list was empty".to_string())
            })?
        } else {
            OneOrMany::one(AssistantContent::text(response.content))
        };

        Ok(CompletionResponse {
            choice,
            usage: LlmCompletionResponse::from_usage(response.usage.as_ref()).token_usage(),
            raw_response: LlmCompletionResponse::from_usage(response.usage.as_ref()),
            message_id: None,
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let messages = convert_messages(&request)?;
        let tools = convert_tools(&request.tools);
        let client = self.client.clone();
        let single_action_mode = self.single_action_mode;

        let (tx, rx) =
            mpsc::unbounded::<Result<RawStreamingChoice<LlmCompletionResponse>, CompletionError>>();
        let sender = tx.clone();

        // The callback is a synchronous Fn invoked from inside the async
        // `chat_stream`; it forwards text/reasoning chunks onto the channel.
        let callback: StreamCallback = Box::new(move |chunk| match chunk {
            StreamChunk::Token(text) => {
                let _ = sender.unbounded_send(Ok(RawStreamingChoice::Message(text)));
            }
            StreamChunk::Reasoning(text) => {
                let _ = sender.unbounded_send(Ok(RawStreamingChoice::ReasoningDelta {
                    id: None,
                    reasoning: text,
                }));
            }
            // Partial tool-call fragments are ignored; complete calls are
            // emitted from the resolved ChatResponse below.
            StreamChunk::ToolCall(_) => {}
        });

        let task = tokio::spawn(async move {
            let result = client.chat_stream(messages, tools, callback).await;
            match result {
                Ok(response) => {
                    let mut calls = response.tool_calls.unwrap_or_default();
                    if single_action_mode {
                        calls.truncate(1);
                    }
                    for call in calls {
                        let _ = tx.unbounded_send(Ok(raw_tool_call(call)));
                    }
                    let _ = tx.unbounded_send(Ok(RawStreamingChoice::FinalResponse(
                        LlmCompletionResponse::from_usage(response.usage.as_ref()),
                    )));
                }
                Err(error) => {
                    let _ = tx.unbounded_send(Err(llm_error_to_completion(error)));
                }
            }
            // Dropping `tx` ends the stream.
        });

        Ok(StreamingCompletionResponse::stream(Box::pin(
            ProviderStream { receiver: rx, task },
        )))
    }
}

/// Convert a Rig [`CompletionRequest`] into AgentZero chat messages.
///
/// Preamble is already folded into a leading system message by Rig's request
/// builder, so it needs no separate handling here. Assistant tool calls and
/// tool results are bridged faithfully: a rig assistant `ToolCall` becomes an
/// AgentZero assistant message with `tool_calls`, and a rig user `ToolResult`
/// becomes an AgentZero `role:"tool"` message whose `tool_call_id` matches the
/// originating call. Without this, OpenAI-compatible providers (DeepSeek, GLM,
/// OpenAI) reject the orphaned tool call as a malformed prompt.
pub(crate) fn convert_messages(
    request: &CompletionRequest,
) -> Result<Vec<ChatMessage>, CompletionError> {
    use agent_primitives::types::Part;
    use rig::completion::message::{AssistantContent, UserContent};

    let mut out = Vec::new();
    for message in request.chat_history.iter() {
        match message {
            Message::System { content } => out.push(ChatMessage::system(content.clone())),

            Message::User { content } => {
                // A rig user message may carry text and/or tool results. Each
                // tool result becomes its own `role:"tool"` message (OpenAI shape),
                // paired by id with the assistant tool call that requested it.
                for part in content.iter() {
                    match part {
                        UserContent::Text(t) => out.push(ChatMessage::user(t.text.clone())),
                        UserContent::ToolResult(tool_result) => {
                            let text = tool_result
                                .content
                                .iter()
                                .filter_map(|c| match c {
                                    ToolResultContent::Text(t) => Some(t.text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            let id = tool_result
                                .call_id
                                .clone()
                                .unwrap_or_else(|| tool_result.id.clone());
                            out.push(ChatMessage::tool_result(id, text));
                        }
                        // Images / audio / documents are not yet bridged onto the wire.
                        _ => {}
                    }
                }
            }

            Message::Assistant { content, .. } => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<AgentToolCall> = Vec::new();
                for part in content.iter() {
                    match part {
                        AssistantContent::Text(t) => text_parts.push(t.text.clone()),
                        AssistantContent::ToolCall(tc) => {
                            tool_calls.push(AgentToolCall {
                                id: tc.call_id.clone().unwrap_or_else(|| tc.id.clone()),
                                name: tc.function.name.clone(),
                                arguments: tc.function.arguments.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                // Providers reject empty assistant content; only emit a content
                // part when there is text. A tool-call-only turn yields an
                // assistant message with empty content + tool_calls.
                let content_parts: Vec<Part> = text_parts
                    .into_iter()
                    .map(|t| Part::Text { text: t })
                    .collect();
                if content_parts.is_empty() && tool_calls.is_empty() {
                    continue;
                }
                out.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: content_parts,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                    is_summary: false,
                });
            }
        }
    }
    Ok(out)
}

/// Convert Rig tool definitions into the OpenAI-compatible `tools` payload the
/// AgentZero client expects.
pub(crate) fn convert_tools(tools: &[ToolDefinition]) -> Option<Value> {
    if tools.is_empty() {
        return None;
    }
    Some(Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect(),
    ))
}

fn nonempty_tool_calls(response: &ChatResponse) -> Option<Vec<AssistantContent>> {
    let calls = response.tool_calls.as_ref()?;
    if calls.is_empty() {
        return None;
    }
    Some(
        calls
            .iter()
            .map(|call| {
                AssistantContent::ToolCall(rig::completion::message::ToolCall::new(
                    call.id.clone(),
                    rig::completion::message::ToolFunction::new(
                        call.name.clone(),
                        call.arguments.clone(),
                    ),
                ))
            })
            .collect(),
    )
}

fn raw_tool_call(call: AgentToolCall) -> RawStreamingChoice<LlmCompletionResponse> {
    RawStreamingChoice::ToolCall(RawStreamingToolCall {
        id: call.id.clone(),
        // Rig correlates tool results by `internal_call_id`; reuse the
        // provider call id so the result round-trips match.
        internal_call_id: call.id.clone(),
        // Rig's tool hook exposes this field, not `id`. Preserve the same
        // authoritative ID for hidden context and provider result correlation.
        call_id: Some(call.id.clone()),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        signature: None,
        additional_params: None,
    })
}

fn llm_error_to_completion(error: LlmError) -> CompletionError {
    CompletionError::ProviderError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::StreamExt;
    use rig::streaming::StreamedAssistantContent;
    use std::sync::Mutex;

    type SeenMessages = Arc<Mutex<Vec<Vec<ChatMessage>>>>;

    /// Stub AgentZero LlmClient that streams canned text then resolves.
    struct StubLlm {
        chunks: Vec<String>,
        final_text: String,
        tool_calls: Vec<AgentToolCall>,
        seen: SeenMessages,
        seen_schema: Arc<Mutex<Vec<Option<Value>>>>,
    }

    impl StubLlm {
        fn text(chunks: &[&str]) -> (Arc<Self>, SeenMessages) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let stub = Arc::new(Self {
                chunks: chunks.iter().map(|c| (*c).to_string()).collect(),
                final_text: chunks.join(""),
                tool_calls: Vec::new(),
                seen: seen.clone(),
                seen_schema: Arc::new(Mutex::new(Vec::new())),
            });
            (stub, seen)
        }
    }

    #[async_trait]
    impl LlmClient for StubLlm {
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
            self.seen.lock().unwrap().push(messages);
            self.seen_schema.lock().unwrap().push(None);
            Ok(ChatResponse {
                content: self.final_text.clone(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        async fn chat_with_schema(
            &self,
            messages: Vec<ChatMessage>,
            _tools: Option<Value>,
            output_schema: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            self.seen.lock().unwrap().push(messages);
            self.seen_schema.lock().unwrap().push(output_schema);
            Ok(ChatResponse {
                content: self.final_text.clone(),
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
            self.seen.lock().unwrap().push(messages);
            for chunk in &self.chunks {
                callback(StreamChunk::Token(chunk.clone()));
            }
            Ok(ChatResponse {
                content: self.final_text.clone(),
                tool_calls: if self.tool_calls.is_empty() {
                    None
                } else {
                    Some(self.tool_calls.clone())
                },
                reasoning: None,
                usage: None,
            })
        }
    }

    fn agent_tool_call(id: &str, name: &str, args: Value) -> AgentToolCall {
        AgentToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        }
    }

    #[tokio::test]
    async fn bridge_streams_text_tokens_then_final() {
        let (stub, _seen) = StubLlm::text(&["hel", "lo"]);
        let model = LlmCompletionModel::new(stub as Arc<dyn LlmClient>, "stub");

        let stream = model
            .stream(rig_request("hi"))
            .await
            .expect("stream should build");

        let mut text = String::new();
        let mut saw_final = false;
        let mut s = stream;
        while let Some(item) = s.next().await {
            match item.expect("chunk") {
                StreamedAssistantContent::Text(t) => text.push_str(&t.text),
                StreamedAssistantContent::Final(_) => saw_final = true,
                _ => {}
            }
        }
        assert_eq!(text, "hello");
        assert!(saw_final);
    }

    #[tokio::test]
    async fn bridge_forwards_complete_tool_calls_after_text() {
        let stub = Arc::new(StubLlm {
            chunks: vec!["think".to_string()],
            final_text: "think".to_string(),
            tool_calls: vec![agent_tool_call("call_1", "calculator", json!({"x": 1}))],
            seen: Arc::new(Mutex::new(Vec::new())),
            seen_schema: Arc::new(Mutex::new(Vec::new())),
        });
        let model = LlmCompletionModel::new(stub as Arc<dyn LlmClient>, "stub");

        let stream = model.stream(rig_request("use tool")).await.expect("stream");
        let mut tool_calls = Vec::new();
        let mut s = stream;
        while let Some(item) = s.next().await {
            if let StreamedAssistantContent::ToolCall { tool_call, .. } = item.expect("chunk") {
                tool_calls.push((
                    tool_call.function.name,
                    tool_call.function.arguments,
                    tool_call.id,
                    tool_call.call_id,
                ));
            }
        }
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].0, "calculator");
        assert_eq!(tool_calls[0].1, json!({"x": 1}));
        assert_eq!(tool_calls[0].2, "call_1");
        assert_eq!(tool_calls[0].3.as_deref(), Some("call_1"));
    }

    #[tokio::test]
    async fn bridge_surfaces_llm_errors_as_provider_error() {
        struct ErrorLlm;
        #[async_trait]
        impl LlmClient for ErrorLlm {
            fn model(&self) -> &str {
                "err"
            }
            fn provider(&self) -> &str {
                "err"
            }
            async fn chat(
                &self,
                _: Vec<ChatMessage>,
                _: Option<Value>,
            ) -> Result<ChatResponse, LlmError> {
                Err(LlmError::ApiError("boom".to_string()))
            }
            async fn chat_stream(
                &self,
                _: Vec<ChatMessage>,
                _: Option<Value>,
                _: StreamCallback,
            ) -> Result<ChatResponse, LlmError> {
                Err(LlmError::ApiError("boom".to_string()))
            }
        }
        let model = LlmCompletionModel::new(Arc::new(ErrorLlm) as Arc<dyn LlmClient>, "err");
        let mut stream = model.stream(rig_request("hi")).await.expect("stream");
        match stream.next().await.expect("an item") {
            Err(CompletionError::ProviderError(msg)) => assert!(msg.contains("boom")),
            other => panic!("expected provider error, got {other:?}"),
        }
    }

    fn rig_request(prompt: &str) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::one(Message::user(prompt.to_string())),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    #[tokio::test]
    async fn dropping_bridge_stream_aborts_pending_provider() {
        struct PendingLlm {
            entered: Arc<tokio::sync::Notify>,
            dropped: Arc<tokio::sync::Notify>,
        }
        struct DropNotice(Arc<tokio::sync::Notify>);
        impl Drop for DropNotice {
            fn drop(&mut self) {
                self.0.notify_one();
            }
        }
        #[async_trait]
        impl LlmClient for PendingLlm {
            fn model(&self) -> &str {
                "pending"
            }
            fn provider(&self) -> &str {
                "fixture"
            }
            async fn chat(
                &self,
                _: Vec<ChatMessage>,
                _: Option<Value>,
            ) -> Result<ChatResponse, LlmError> {
                panic!("streaming fixture");
            }
            async fn chat_stream(
                &self,
                _: Vec<ChatMessage>,
                _: Option<Value>,
                _: StreamCallback,
            ) -> Result<ChatResponse, LlmError> {
                let _notice = DropNotice(self.dropped.clone());
                self.entered.notify_one();
                futures::future::pending().await
            }
        }
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(tokio::sync::Notify::new());
        let model = LlmCompletionModel::new(
            Arc::new(PendingLlm {
                entered: entered.clone(),
                dropped: dropped.clone(),
            }),
            "pending",
        );
        let stream = model.stream(rig_request("hello")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), entered.notified())
            .await
            .expect("provider task must actually start");
        drop(stream);
        tokio::time::timeout(std::time::Duration::from_secs(1), dropped.notified())
            .await
            .expect("stream owns and cancels the pending provider task");
    }

    fn invalid_tool_request() -> CompletionRequest {
        let mut request = rig_request("hello");
        request.tools = vec![ToolDefinition {
            name: "x".repeat(65),
            description: "invalid provider tool name".to_string(),
            parameters: json!({"type": "object"}),
        }];
        request
    }

    #[tokio::test]
    async fn completion_and_stream_reject_invalid_tools_before_network_io() {
        let config = crate::llm::LlmConfig::new(
            "https://api.example.invalid".to_string(),
            "test-key".to_string(),
            "gpt-4-turbo".to_string(),
            "openai".to_string(),
        );
        let client = crate::llm::OpenAiClient::new(config).expect("test OpenAI client");
        let model = LlmCompletionModel::new(Arc::new(client) as Arc<dyn LlmClient>, "gpt-4-turbo");

        let completion = model
            .completion(invalid_tool_request())
            .await
            .expect_err("Rig completion must surface local validation");
        assert!(completion
            .to_string()
            .contains("tool_schema_rule=function_name"));

        let mut stream = model
            .stream(invalid_tool_request())
            .await
            .expect("Rig stream should initialize");
        let stream_error = stream
            .next()
            .await
            .expect("Rig stream must surface an error")
            .expect_err("Rig stream must reject the invalid tool");
        assert!(stream_error
            .to_string()
            .contains("tool_schema_rule=function_name"));
    }

    #[test]
    fn convert_messages_bridges_tool_calls_and_results() {
        // Regression: a history with an assistant tool call + its tool result
        // must produce a valid OpenAI message chain (assistant.tool_calls paired
        // with a role:"tool" message whose tool_call_id matches). Dropping the
        // tool result made strict providers (DeepSeek/GLM) reject the request.
        use rig::completion::message::Text as RigText;
        use rig::completion::message::{
            AssistantContent, ToolCall as RigToolCall, ToolFunction, ToolResult as RigToolResult,
            UserContent,
        };

        let assistant_call = Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(RigToolCall::new(
                "call_1".to_string(),
                ToolFunction::new("echo".to_string(), json!({"x": 1})),
            ))),
        };
        let tool_result = Message::User {
            content: OneOrMany::one(UserContent::ToolResult(RigToolResult {
                id: "call_1".to_string(),
                call_id: Some("call_1".to_string()),
                content: OneOrMany::one(ToolResultContent::Text(RigText::new("echo-result"))),
            })),
        };
        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(vec![
                Message::user("please echo".to_string()),
                assistant_call,
                tool_result,
            ])
            .expect("non-empty history"),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        };

        let msgs = convert_messages(&request).expect("convert");

        let assistant = msgs
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant message present");
        let tool_calls = assistant.tool_calls.as_ref().expect("assistant tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].name, "echo");
        assert_eq!(tool_calls[0].arguments, json!({"x": 1}));

        let tool = msgs
            .iter()
            .find(|m| m.role == "tool")
            .expect("tool result message present");
        // The tool result's tool_call_id must match the assistant's tool call id.
        assert_eq!(tool.tool_call_id.as_deref(), Some("call_1"));
        assert!(tool.text_content().contains("echo-result"));
    }

    #[tokio::test]
    async fn completion_forwards_output_schema_to_llm_client() {
        let (stub, _seen) = StubLlm::text(&[r#"{"value":"ok"}"#]);
        let schema = json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            },
            "required": ["value"]
        });
        let model = LlmCompletionModel::new(stub.clone() as Arc<dyn LlmClient>, "stub");
        let mut request = rig_request("typed");
        request.output_schema = Some(schemars::Schema::try_from(schema.clone()).unwrap());

        let _ = model.completion(request).await.expect("completion");

        let seen_schema = stub.seen_schema.lock().unwrap();
        assert_eq!(seen_schema.as_slice(), &[Some(schema)]);
    }
}
