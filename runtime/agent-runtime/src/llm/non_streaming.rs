//! # Non-Streaming LLM Client Wrapper
//!
//! Wraps an `LlmClient` and converts `chat_stream()` calls into `chat()` calls.
//! Used for subagents where streaming adds no value and causes reliability issues.

use super::{
    ChatMessage, ChatResponse, LlmClient, LlmError, StreamCallback, StreamChunk, ToolCallChunk,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Wraps an `LlmClient` to disable streaming — `chat_stream()` calls `chat()` internally.
pub struct NonStreamingLlmClient {
    inner: Arc<dyn LlmClient>,
}

impl NonStreamingLlmClient {
    /// Create a new non-streaming wrapper around an existing LLM client.
    pub fn new(inner: Arc<dyn LlmClient>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl LlmClient for NonStreamingLlmClient {
    fn model(&self) -> &str {
        self.inner.model()
    }
    fn provider(&self) -> &str {
        self.inner.provider()
    }
    fn supports_tools(&self) -> bool {
        self.inner.supports_tools()
    }
    fn supports_reasoning(&self) -> bool {
        self.inner.supports_reasoning()
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        self.inner.chat(messages, tools).await
    }

    async fn chat_with_schema(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        self.inner
            .chat_with_schema(messages, tools, output_schema)
            .await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        tracing::debug!("NonStreamingLlmClient: using chat() instead of chat_stream()");

        let response = self.inner.chat(messages, tools).await?;

        // Emit content as a single token chunk
        if !response.content.is_empty() {
            callback(StreamChunk::Token(response.content.clone()));
        }

        // Emit tool call chunks
        if let Some(ref tool_calls) = response.tool_calls {
            for tc in tool_calls {
                callback(StreamChunk::ToolCall(ToolCallChunk {
                    id: Some(tc.id.clone()),
                    name: Some(tc.name.clone()),
                    arguments: tc.arguments.to_string(),
                }));
            }
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct StubLlm {
        response: Mutex<Option<ChatResponse>>,
    }

    #[async_trait]
    impl LlmClient for StubLlm {
        fn model(&self) -> &str {
            "stub-model"
        }
        fn provider(&self) -> &str {
            "stub-provider"
        }
        fn supports_tools(&self) -> bool {
            true
        }
        fn supports_reasoning(&self) -> bool {
            true
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            self.response
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| LlmError::ApiError("no response staged".to_string()))
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            unreachable!("non_streaming should not call inner.chat_stream")
        }
    }

    fn stub(resp: ChatResponse) -> Arc<StubLlm> {
        Arc::new(StubLlm {
            response: Mutex::new(Some(resp)),
        })
    }

    #[tokio::test]
    async fn passthrough_metadata() {
        let inner = stub(ChatResponse {
            content: String::new(),
            tool_calls: None,
            reasoning: None,
            usage: None,
        });
        let wrapped = NonStreamingLlmClient::new(inner);
        assert_eq!(wrapped.model(), "stub-model");
        assert_eq!(wrapped.provider(), "stub-provider");
        assert!(wrapped.supports_tools());
        assert!(wrapped.supports_reasoning());
    }

    #[tokio::test]
    async fn chat_forwards_to_inner() {
        let inner = stub(ChatResponse {
            content: "hello".to_string(),
            tool_calls: None,
            reasoning: None,
            usage: None,
        });
        let wrapped = NonStreamingLlmClient::new(inner);
        let resp = wrapped.chat(vec![], None).await.unwrap();
        assert_eq!(resp.content, "hello");
    }

    #[tokio::test]
    async fn chat_stream_emits_token_then_tool_calls_then_returns() {
        let inner = stub(ChatResponse {
            content: "hello world".to_string(),
            tool_calls: Some(vec![
                ToolCall::new(
                    "id-1".to_string(),
                    "search".to_string(),
                    serde_json::json!({"q": "rust"}),
                ),
                ToolCall::new(
                    "id-2".to_string(),
                    "calc".to_string(),
                    serde_json::json!({}),
                ),
            ]),
            reasoning: None,
            usage: None,
        });
        let wrapped = NonStreamingLlmClient::new(inner);

        let calls = Arc::new(AtomicUsize::new(0));
        let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let collected_cl = Arc::clone(&collected);
        let calls_cl = Arc::clone(&calls);
        let cb: StreamCallback = Box::new(move |chunk: StreamChunk| {
            calls_cl.fetch_add(1, Ordering::SeqCst);
            let label = match chunk {
                StreamChunk::Token(t) => format!("tok:{t}"),
                StreamChunk::Reasoning(_) => "reasoning".to_string(),
                StreamChunk::ToolCall(tc) => {
                    format!(
                        "tc:{}:{}:{}",
                        tc.id.unwrap_or_default(),
                        tc.name.unwrap_or_default(),
                        tc.arguments
                    )
                }
            };
            collected_cl.lock().unwrap().push(label);
        });

        let resp = wrapped.chat_stream(vec![], None, cb).await.unwrap();
        assert_eq!(resp.content, "hello world");
        let labels = collected.lock().unwrap().clone();
        assert_eq!(labels.len(), 3);
        assert!(labels[0].starts_with("tok:hello world"));
        assert!(labels[1].contains("id-1"));
        assert!(labels[2].contains("id-2"));
    }

    #[tokio::test]
    async fn chat_stream_skips_token_emit_when_content_empty() {
        let inner = stub(ChatResponse {
            content: String::new(),
            tool_calls: None,
            reasoning: None,
            usage: None,
        });
        let wrapped = NonStreamingLlmClient::new(inner);
        let count = Arc::new(AtomicUsize::new(0));
        let count_cl = Arc::clone(&count);
        let cb: StreamCallback = Box::new(move |_chunk| {
            count_cl.fetch_add(1, Ordering::SeqCst);
        });
        let resp = wrapped.chat_stream(vec![], None, cb).await.unwrap();
        assert!(resp.content.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
