// ============================================================================
// LLM CLIENT TRAIT
// Abstract interface for LLM providers
// ============================================================================

use std::boxed::Box;

use async_trait::async_trait;
use serde_json::Value;

use crate::types::{ChatMessage, ToolCall};

/// Response from an LLM chat completion
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// The text content of the response
    pub content: String,

    /// Tool calls requested by the LLM
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Reasoning content (for models with thinking enabled)
    pub reasoning: Option<String>,

    /// Token usage information
    pub usage: Option<TokenUsage>,
}

/// Token usage information
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Input tokens
    pub prompt_tokens: u32,

    /// Output tokens
    pub completion_tokens: u32,

    /// Total tokens
    pub total_tokens: u32,

    /// Prompt tokens served from the provider's prompt cache, if the
    /// provider reports them. `None` if unreported. Sources observed:
    ///   - OpenAI: `usage.prompt_tokens_details.cached_tokens`
    ///   - GLM / DeepSeek / z.ai: `usage.prompt_cache_hit_tokens`
    pub cached_prompt_tokens: Option<u32>,
}

/// Callback type for streaming events
pub type StreamCallback = Box<dyn Fn(StreamChunk) + Send + Sync>;

/// Trait for LLM client implementations
///
/// This trait provides a unified interface for interacting with
/// various LLM providers (`OpenAI`, Anthropic, etc.)
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Get the model identifier
    fn model(&self) -> &str;

    /// Get the provider identifier
    fn provider(&self) -> &str;

    /// Send a chat completion request
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError>;

    /// Send a chat completion request with an optional provider-native JSON
    /// Schema output constraint.
    ///
    /// The default preserves existing providers/tests by ignoring the schema.
    /// Wrappers and providers that can carry the constraint should override this.
    async fn chat_with_schema(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        let _ = output_schema;
        self.chat(messages, tools).await
    }

    /// Send a chat completion request with streaming
    ///
    /// The callback receives events as they are generated.
    /// Pass `tools` to enable tool calling during streaming.
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError>;

    /// Check if the model supports tool calling
    fn supports_tools(&self) -> bool {
        true
    }

    /// Check if the model supports reasoning/thinking
    fn supports_reasoning(&self) -> bool {
        false
    }
}

/// A chunk of streamed data from the LLM
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A token from the main response
    Token(String),

    /// Reasoning content
    Reasoning(String),

    /// A tool call being constructed
    ToolCall(ToolCallChunk),
}

/// A partial tool call during streaming
#[derive(Debug, Clone)]
pub struct ToolCallChunk {
    /// Tool call ID
    pub id: Option<String>,

    /// Tool name
    pub name: Option<String>,

    /// Partial arguments (JSON string fragment)
    pub arguments: String,
}

/// Errors from LLM operations
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// Error from the HTTP client
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Error parsing response
    #[error("Parse error: {0}")]
    ParseError(String),

    /// API returned an error
    #[error("API error: {0}")]
    ApiError(String),

    /// Invalid request
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Rate limited
    #[error("Rate limited")]
    RateLimited,

    /// Authentication failed
    #[error("Authentication failed")]
    AuthenticationFailed,

    /// Model not found
    #[error("Model not found: {0}")]
    ModelNotFound(String),
}

/// Trait for models with specific capabilities
pub trait LlmModel: Send + Sync {
    /// Get the model name
    fn model_name(&self) -> &str;

    /// Get maximum context window
    fn max_context_tokens(&self) -> u32;

    /// Get maximum output tokens
    fn max_output_tokens(&self) -> u32;

    /// Check if model supports vision
    fn supports_vision(&self) -> bool {
        false
    }

    /// Check if model supports function calling
    fn supports_function_calling(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 0);
        assert_eq!(usage.cached_prompt_tokens, None);
    }

    #[test]
    fn llm_error_display_messages() {
        let parse = LlmError::ParseError("bad json".to_string());
        assert_eq!(format!("{parse}"), "Parse error: bad json");
        let api = LlmError::ApiError("no".to_string());
        assert_eq!(format!("{api}"), "API error: no");
        let invalid = LlmError::InvalidRequest("oops".to_string());
        assert_eq!(format!("{invalid}"), "Invalid request: oops");
        let rate = LlmError::RateLimited;
        assert_eq!(format!("{rate}"), "Rate limited");
        let auth = LlmError::AuthenticationFailed;
        assert_eq!(format!("{auth}"), "Authentication failed");
        let model = LlmError::ModelNotFound("x".to_string());
        assert_eq!(format!("{model}"), "Model not found: x");
    }

    // Cover LlmModel default trait methods (supports_vision, supports_function_calling).
    struct DummyModel;
    impl LlmModel for DummyModel {
        fn model_name(&self) -> &str {
            "dummy"
        }
        fn max_context_tokens(&self) -> u32 {
            1024
        }
        fn max_output_tokens(&self) -> u32 {
            512
        }
    }

    #[test]
    fn llm_model_defaults() {
        let m = DummyModel;
        assert_eq!(m.model_name(), "dummy");
        assert_eq!(m.max_context_tokens(), 1024);
        assert_eq!(m.max_output_tokens(), 512);
        assert!(!m.supports_vision());
        assert!(m.supports_function_calling());
    }
}
