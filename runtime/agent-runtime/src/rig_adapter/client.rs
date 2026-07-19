//! AgentZero `LlmClient` exposed as a Rig [`CompletionClient`].
//!
//! This is the structured-output companion to [`LlmCompletionModel`]: it lets
//! Rig's higher-level primitives — extractors (`client.extractor::<T>()`),
//! agents — run over the *same* OpenAI-compatible transport (Path A: Rig wraps
//! AgentZero's `LlmClient`, it does not replace it). With this, a consumer that
//! needs typed structured output (intent analysis, distillation, …) can use a
//! Rig [`Extractor`](rig::extractor::Extractor) instead of raw `chat` + manual
//! JSON parsing.

use std::sync::Arc;

use rig::client::CompletionClient;

use super::model::LlmCompletionModel;
use crate::llm::LlmClient;

/// A Rig [`CompletionClient`] backed by an AgentZero [`LlmClient`].
#[derive(Clone)]
pub struct LlmCompletionClient {
    pub(crate) client: Arc<dyn LlmClient>,
}

impl LlmCompletionClient {
    /// Wrap an AgentZero LLM client for use as a Rig completion client.
    #[must_use]
    pub fn new(client: Arc<dyn LlmClient>) -> Self {
        Self { client }
    }
}

impl CompletionClient for LlmCompletionClient {
    type CompletionModel = LlmCompletionModel;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ChatResponse, LlmClient, LlmError, StreamCallback};
    use crate::types::{ChatMessage, ToolCall};
    use async_trait::async_trait;
    use rig::client::CompletionClient;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    // The extractor works via tool-calling: it registers a `submit` tool whose
    // args are T, and the model calls `submit({...T...})`. This test proves the
    // extractor + LlmCompletionClient + LlmCompletionModel bridge end-to-end.
    #[derive(Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
    struct Person {
        name: String,
        age: u32,
    }

    struct StubLlm {
        submitted: Person,
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
            _messages: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            // Immediately "submit" the structured data, as a capable model would.
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: "c1".to_string(),
                    name: "submit".to_string(),
                    arguments: serde_json::to_value(&self.submitted).unwrap(),
                }]),
                reasoning: None,
                usage: None,
            })
        }
        async fn chat_stream(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Option<Value>,
            _callback: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            Err(LlmError::ApiError(
                "chat_stream not used by extractor".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn extractor_returns_typed_struct_via_submit_tool() {
        let client = LlmCompletionClient::new(Arc::new(StubLlm {
            submitted: Person {
                name: "Ada".to_string(),
                age: 36,
            },
        }));
        let extractor = client.extractor::<Person>("stub").build();
        let person = extractor
            .extract("extract the person")
            .await
            .expect("extraction should yield the typed struct via the submit tool");
        assert_eq!(person.name, "Ada");
        assert_eq!(person.age, 36);
    }
}
