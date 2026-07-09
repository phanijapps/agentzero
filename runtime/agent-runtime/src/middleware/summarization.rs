// ============================================================================
// SUMMARIZATION MIDDLEWARE
// Automatically summarize conversation history when approaching token limits
//
// Inspired by LangChain's summarization middleware:
// https://docs.langchain.com/oss/javascript/langchain/middleware/built-in#summarization
// ============================================================================

//! # Summarization Middleware
//!
//! Automatically summarize conversation history when approaching token limits.

use super::config::SummarizationConfig;
use super::token_counter::{estimate_total_tokens, get_model_context_window};
use super::traits::{MiddlewareContext, MiddlewareEffect, PreProcessMiddleware};
use crate::llm::openai::OpenAiClient;
use crate::llm::{LlmClient, LlmConfig};
use crate::types::{ChatMessage, StreamEvent};
use std::sync::Arc;

/// Summarization middleware
///
/// Compresses older conversation messages into a summary when token limits
/// are approached, while keeping recent messages intact.
pub struct SummarizationMiddleware {
    /// Configuration
    config: SummarizationConfig,
    /// LLM client for generating summaries
    summary_client: Arc<dyn LlmClient>,
}

impl SummarizationMiddleware {
    /// Create a new summarization middleware
    ///
    /// # Arguments
    /// * `config` - Middleware configuration
    /// * `summary_client` - LLM client to use for summarization
    pub fn new(config: SummarizationConfig, summary_client: Arc<dyn LlmClient>) -> Self {
        Self {
            config,
            summary_client,
        }
    }

    /// Create from config with LLM client creation
    ///
    /// # Arguments
    /// * `config` - Middleware configuration
    /// * `provider_id` - Provider ID for the summary model (config provider or agent provider)
    /// * `summary_model` - Model from config, if specified (None = use agent model)
    /// * `agent_model` - The agent's model (used as fallback)
    /// * `api_key` - API key for the provider
    /// * `base_url` - Base URL for the provider API
    pub async fn from_config(
        config: SummarizationConfig,
        provider_id: &str,
        summary_model: Option<String>,
        agent_model: &str,
        api_key: String,
        base_url: String,
    ) -> Result<Self, String> {
        // Use configured summary model, or fall back to agent's model
        let model = summary_model.unwrap_or_else(|| agent_model.to_string());

        let llm_config = LlmConfig {
            provider_id: provider_id.to_string(),
            api_key,
            base_url,
            model,
            temperature: 0.3, // Lower temperature for more consistent summaries
            max_tokens: 1000,
            thinking_enabled: false,
            provider_params: None,
        };

        let summary_client = Arc::new(
            OpenAiClient::new(llm_config)
                .map_err(|e| format!("Failed to create LLM client: {e}"))?,
        );

        Ok(Self::new(config, summary_client))
    }

    /// Generate a summary of the given messages
    async fn summarize(&self, messages: &[ChatMessage]) -> Result<String, String> {
        // Build conversation text for summarization
        let conversation_text = self.build_conversation_text(messages);

        // Build summarization prompt
        let prompt = if let Some(custom_prompt) = &self.config.summary_prompt {
            custom_prompt.replace("{messages}", &conversation_text)
        } else {
            format!(
                "Summarize the following conversation concisely. \
                 Preserve key information, decisions, and context:\n\n{conversation_text}"
            )
        };

        // Call LLM to generate summary
        let response = self
            .summary_client
            .chat(
                vec![ChatMessage::user(prompt)],
                None, // No tools needed for summarization
            )
            .await
            .map_err(|e| format!("Summarization failed: {e}"))?;

        Ok(response.content)
    }

    /// Build conversation text from messages
    fn build_conversation_text(&self, messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .map(|msg| format!("{}: {}", msg.role, msg.text_content()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn is_summarizable_prose(message: &ChatMessage) -> bool {
        !message.is_summary
            && message.tool_calls.is_none()
            && message.tool_call_id.is_none()
            && matches!(message.role.as_str(), "user" | "assistant")
    }

    /// Split messages into keep and summarize groups.
    ///
    /// Summarization only consumes old prose messages. System messages,
    /// plan-block summaries, assistant tool-call messages, tool responses,
    /// and existing summaries are retained verbatim.
    fn split_messages(
        &self,
        messages: &[ChatMessage],
        context_window: usize,
    ) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
        let to_keep = self
            .config
            .keep
            .to_keep_count(messages.len(), context_window);

        let keep_boundary = messages.len().saturating_sub(to_keep);
        let mut keep = Vec::new();
        let mut summarize = Vec::new();

        for (idx, message) in messages.iter().enumerate() {
            if idx < keep_boundary && Self::is_summarizable_prose(message) {
                summarize.push(message.clone());
            } else {
                keep.push(message.clone());
            }
        }

        (keep, summarize)
    }
}

#[async_trait::async_trait]
impl PreProcessMiddleware for SummarizationMiddleware {
    fn name(&self) -> &'static str {
        "summarization"
    }

    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(Self {
            config: self.config.clone(),
            summary_client: Arc::clone(&self.summary_client),
        })
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    async fn process(
        &self,
        messages: Vec<ChatMessage>,
        context: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        // Validate configuration
        if !self.config.trigger.is_valid() {
            return Ok(MiddlewareEffect::Proceed);
        }

        if !self.config.keep.is_valid() {
            return Ok(MiddlewareEffect::Proceed);
        }

        // Estimate current token count
        let current_tokens = estimate_total_tokens(&messages, &context.model);
        let message_count = messages.len();
        let context_window = get_model_context_window(&context.model);

        // Check if we should trigger summarization
        if !self
            .config
            .trigger
            .should_trigger(current_tokens, message_count, context_window)
        {
            return Ok(MiddlewareEffect::Proceed);
        }

        // Split messages into keep and summarize groups
        let (keep_messages, to_summarize) = self.split_messages(&messages, context_window);

        if to_summarize.is_empty() {
            // Nothing to summarize
            return Ok(MiddlewareEffect::Proceed);
        }

        // Generate summary
        let summary = self.summarize(&to_summarize).await?;

        // Create event about summarization
        let event = StreamEvent::Token {
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            content: format!(
                "{}\n[Summarized {} messages into {} characters]",
                self.config.summary_prefix,
                to_summarize.len(),
                summary.len()
            ),
        };

        // Build new message list with summary
        let mut new_messages = Vec::new();

        let mut summary_message = ChatMessage::system(format!(
            "{}\n\nSummary of previous conversation:\n{}",
            self.config.summary_prefix, summary
        ));
        summary_message.is_summary = true;
        let insert_at = keep_messages
            .iter()
            .position(|message| message.role != "system")
            .unwrap_or(keep_messages.len());
        new_messages.extend(keep_messages[..insert_at].iter().cloned());
        new_messages.push(summary_message);
        new_messages.extend(keep_messages[insert_at..].iter().cloned());

        Ok(MiddlewareEffect::EmitAndModify {
            event,
            messages: new_messages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::client::{ChatResponse, LlmError, StreamCallback};
    use crate::llm::LlmConfig;
    use crate::middleware::config::{KeepPolicy, TriggerCondition};
    use crate::types::ToolCall;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn create_test_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("You are a helpful assistant.".to_string()),
            ChatMessage::user("Hello!".to_string()),
            ChatMessage::assistant("Hi there!".to_string()),
        ]
    }

    /// Stub LLM that returns canned summaries and counts calls.
    struct StubSummaryClient {
        canned: String,
        calls: Arc<AtomicUsize>,
    }

    impl StubSummaryClient {
        fn new(canned: &str) -> (Arc<Self>, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Arc::new(Self {
                    canned: canned.to_string(),
                    calls: Arc::clone(&calls),
                }),
                calls,
            )
        }
    }

    struct FailingSummaryClient;

    #[async_trait]
    impl LlmClient for FailingSummaryClient {
        fn model(&self) -> &str {
            "stub"
        }
        fn provider(&self) -> &str {
            "stub"
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            Err(LlmError::ApiError("summary unavailable".to_string()))
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            self.chat(_msgs, _tools).await
        }
    }

    #[async_trait]
    impl LlmClient for StubSummaryClient {
        fn model(&self) -> &str {
            "stub"
        }
        fn provider(&self) -> &str {
            "stub"
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: self.canned.clone(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            // Not used by summarization
            self.chat(_msgs, _tools).await
        }
    }

    fn make_middleware(
        config: SummarizationConfig,
        client: Arc<dyn LlmClient>,
    ) -> SummarizationMiddleware {
        SummarizationMiddleware::new(config, client)
    }

    fn make_ctx() -> MiddlewareContext {
        MiddlewareContext::new(
            "agent".to_string(),
            None,
            "stub".to_string(),
            "gpt-4o-mini".to_string(),
        )
    }

    fn permissive_config() -> SummarizationConfig {
        SummarizationConfig {
            enabled: true,
            trigger: TriggerCondition {
                tokens: None,
                messages: Some(2),
                fraction: None,
            },
            keep: KeepPolicy {
                messages: Some(1),
                tokens: None,
                fraction: None,
            },
            ..SummarizationConfig::default()
        }
    }

    #[test]
    fn test_split_messages() {
        let config = SummarizationConfig::default();
        let middleware = SummarizationMiddleware {
            config,
            // Would need a mock client for full testing
            summary_client: Arc::new(
                OpenAiClient::new(LlmConfig {
                    provider_id: "test".to_string(),
                    api_key: "test".to_string(),
                    base_url: "https://test.com".to_string(),
                    model: "gpt-4o-mini".to_string(),
                    temperature: 0.3,
                    max_tokens: 1000,
                    thinking_enabled: false,
                    provider_params: None,
                })
                .unwrap(),
            ),
        };

        let messages = create_test_messages();
        let (keep, summarize) = middleware.split_messages(&messages, 128_000);

        // All messages should be kept (3 total, default keep is 10)
        assert_eq!(keep.len(), 3); // all messages kept
        assert_eq!(summarize.len(), 0); // nothing to summarize
    }

    #[test]
    fn name_and_clone_box() {
        let (stub, _calls) = StubSummaryClient::new("");
        let mw = make_middleware(SummarizationConfig::default(), stub);
        assert_eq!(mw.name(), "summarization");
        let cloned = mw.clone_box();
        assert_eq!(cloned.name(), "summarization");
    }

    #[test]
    fn enabled_reflects_config() {
        let (stub, _) = StubSummaryClient::new("");
        let cfg = SummarizationConfig {
            enabled: false,
            ..SummarizationConfig::default()
        };
        let mw = make_middleware(cfg, Arc::clone(&stub) as Arc<dyn LlmClient>);
        assert!(!mw.enabled());

        let cfg2 = SummarizationConfig {
            enabled: true,
            ..SummarizationConfig::default()
        };
        let mw2 = make_middleware(cfg2, stub as Arc<dyn LlmClient>);
        assert!(mw2.enabled());
    }

    #[tokio::test]
    async fn process_proceeds_when_trigger_invalid() {
        let (stub, calls) = StubSummaryClient::new("");
        let cfg = SummarizationConfig {
            enabled: true,
            trigger: TriggerCondition::default(), // no fields set => invalid
            ..SummarizationConfig::default()
        };
        let mw = make_middleware(cfg, stub);
        let result = mw
            .process(create_test_messages(), &make_ctx())
            .await
            .unwrap();
        assert!(matches!(result, MiddlewareEffect::Proceed));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_proceeds_when_keep_invalid() {
        let (stub, calls) = StubSummaryClient::new("");
        let cfg = SummarizationConfig {
            enabled: true,
            trigger: TriggerCondition {
                messages: Some(1),
                ..TriggerCondition::default()
            },
            keep: KeepPolicy {
                messages: None,
                tokens: None,
                fraction: None,
            },
            ..SummarizationConfig::default()
        };
        let mw = make_middleware(cfg, stub);
        let result = mw
            .process(create_test_messages(), &make_ctx())
            .await
            .unwrap();
        assert!(matches!(result, MiddlewareEffect::Proceed));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_proceeds_when_trigger_not_met() {
        let (stub, calls) = StubSummaryClient::new("");
        let cfg = SummarizationConfig {
            enabled: true,
            trigger: TriggerCondition {
                messages: Some(100), // way above
                ..TriggerCondition::default()
            },
            keep: KeepPolicy {
                messages: Some(1),
                tokens: None,
                fraction: None,
            },
            ..SummarizationConfig::default()
        };
        let mw = make_middleware(cfg, stub);
        let result = mw
            .process(create_test_messages(), &make_ctx())
            .await
            .unwrap();
        assert!(matches!(result, MiddlewareEffect::Proceed));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_emits_and_modifies_when_triggered() {
        let (stub, calls) = StubSummaryClient::new("brief recap");
        let cfg = permissive_config();
        let mw = make_middleware(cfg, stub);

        // Leading system messages include the pinned plan block and stay before the summary.
        let mut plan_block = ChatMessage::system("<!-- plan-block:v1 -->\n[Plan]".to_string());
        plan_block.is_summary = true;
        let messages = vec![
            ChatMessage::system("sys".to_string()),
            plan_block,
            ChatMessage::user("u1".to_string()),
            ChatMessage::assistant("a1".to_string()),
            ChatMessage::user("u2".to_string()),
        ];
        let result = mw.process(messages, &make_ctx()).await.unwrap();
        match result {
            MiddlewareEffect::EmitAndModify { event, messages } => {
                assert!(matches!(event, StreamEvent::Token { .. }));
                assert_eq!(messages[0].text_content(), "sys");
                assert!(messages[1].text_content().contains("plan-block"));
                assert!(messages[1].is_summary);
                assert_eq!(messages[2].role, "system");
                assert!(messages[2].is_summary);
                assert!(messages[2].text_content().contains("brief recap"));
            }
            other => panic!("expected EmitAndModify, got {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn process_proceeds_when_nothing_to_summarize() {
        let (stub, calls) = StubSummaryClient::new("nope");
        // trigger=1 (always), but keep is large enough to keep everything
        let cfg = SummarizationConfig {
            enabled: true,
            trigger: TriggerCondition {
                messages: Some(1),
                ..TriggerCondition::default()
            },
            keep: KeepPolicy {
                messages: Some(100),
                tokens: None,
                fraction: None,
            },
            ..SummarizationConfig::default()
        };
        let mw = make_middleware(cfg, stub);
        let result = mw
            .process(create_test_messages(), &make_ctx())
            .await
            .unwrap();
        assert!(matches!(result, MiddlewareEffect::Proceed));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_returns_error_when_summary_generation_fails() {
        let cfg = permissive_config();
        let mw = make_middleware(cfg, Arc::new(FailingSummaryClient));
        let err = mw
            .process(
                vec![
                    ChatMessage::user("u1".to_string()),
                    ChatMessage::assistant("a1".to_string()),
                    ChatMessage::user("u2".to_string()),
                ],
                &make_ctx(),
            )
            .await
            .expect_err("summary client failure must be surfaced");
        assert!(err.contains("Summarization failed"));
    }

    #[tokio::test]
    async fn pipeline_does_not_summarize_when_context_editing_drops_below_threshold() {
        use crate::middleware::config::ContextEditingConfig;
        use crate::middleware::{ContextEditingMiddleware, MiddlewarePipeline};
        use agent_primitives::types::Part;

        let (stub, calls) = StubSummaryClient::new("should not be called");
        let raw_messages = vec![
            ChatMessage::user("inspect the old file".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![],
                tool_calls: Some(vec![ToolCall::new(
                    "call_old".to_string(),
                    "read_file".to_string(),
                    serde_json::json!({"path": "old.rs"}),
                )]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "large tool output ".repeat(2_000),
                }],
                tool_calls: None,
                tool_call_id: Some("call_old".to_string()),
                is_summary: false,
            },
            ChatMessage::assistant("I found the relevant code.".to_string()),
            ChatMessage::user("continue".to_string()),
        ];
        let post_edit_messages = vec![
            raw_messages[0].clone(),
            raw_messages[1].clone(),
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "[cleared]".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_old".to_string()),
                is_summary: false,
            },
            raw_messages[3].clone(),
            raw_messages[4].clone(),
        ];
        let threshold = crate::middleware::token_counter::estimate_total_tokens(
            &post_edit_messages,
            "gpt-4o-mini",
        ) + 100;
        assert!(
            crate::middleware::token_counter::estimate_total_tokens(&raw_messages, "gpt-4o-mini")
                > threshold
        );

        let summarization = SummarizationMiddleware::new(
            SummarizationConfig {
                enabled: true,
                trigger: TriggerCondition {
                    tokens: Some(threshold),
                    messages: None,
                    fraction: None,
                },
                keep: KeepPolicy {
                    messages: Some(1),
                    tokens: None,
                    fraction: None,
                },
                ..SummarizationConfig::default()
            },
            stub,
        );
        let pipeline = MiddlewarePipeline::new()
            .add_pre_processor(Box::new(ContextEditingMiddleware::new(
                ContextEditingConfig {
                    enabled: true,
                    trigger_tokens: 1,
                    keep_tool_results: 0,
                    min_reclaim: 0,
                    placeholder: "[cleared]".to_string(),
                    ..Default::default()
                },
            )))
            .add_pre_processor(Box::new(summarization));

        let mut events = Vec::new();
        let out = pipeline
            .process_messages(raw_messages, &make_ctx(), |event| events.push(event))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(events.len(), 1);
        assert!(out.iter().any(|m| {
            m.role == "tool"
                && m.tool_call_id.as_deref() == Some("call_old")
                && m.text_content() == "[cleared]"
        }));
    }

    #[test]
    fn split_messages_excludes_assistant_tool_pair() {
        let (stub, _) = StubSummaryClient::new("");
        let mw = make_middleware(SummarizationConfig::default(), stub);

        // 1 user, 1 assistant w/ tool_calls, 1 tool response, 1 user.
        let messages = vec![
            ChatMessage::user("first".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![],
                tool_calls: Some(vec![ToolCall::new(
                    "c1".to_string(),
                    "x".to_string(),
                    Value::Null,
                )]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![],
                tool_calls: None,
                tool_call_id: Some("c1".to_string()),
                is_summary: false,
            },
            ChatMessage::user("after-pair".to_string()),
        ];

        // Force keep = 1. Only old prose is summarizable; the assistant
        // tool-call message and tool response stay on the tape.
        let mw2 = SummarizationMiddleware::new(
            SummarizationConfig {
                keep: KeepPolicy {
                    messages: Some(1),
                    tokens: None,
                    fraction: None,
                },
                ..SummarizationConfig::default()
            },
            mw.summary_client.clone(),
        );
        let (keep, summarize) = mw2.split_messages(&messages, 128_000);
        assert_eq!(summarize.len(), 1);
        assert_eq!(summarize[0].text_content(), "first");
        assert_eq!(keep.len(), 3);
        assert!(keep.iter().any(|m| m.tool_calls.is_some()));
        assert!(keep
            .iter()
            .any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("c1")));
        assert!(keep.iter().any(|m| m.text_content() == "after-pair"));
    }

    #[test]
    fn split_messages_keeps_tool_responses_when_no_old_prose_is_eligible() {
        let (stub, _) = StubSummaryClient::new("");
        let mw = SummarizationMiddleware::new(
            SummarizationConfig {
                keep: KeepPolicy {
                    messages: Some(2),
                    tokens: None,
                    fraction: None,
                },
                ..SummarizationConfig::default()
            },
            stub,
        );

        // Old assistant tool call and tool responses are ineligible for summarization.
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![],
                tool_calls: Some(vec![ToolCall::new(
                    "c".to_string(),
                    "t".to_string(),
                    Value::Null,
                )]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![],
                tool_calls: None,
                tool_call_id: Some("c".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![],
                tool_calls: None,
                tool_call_id: Some("c".to_string()),
                is_summary: false,
            },
            ChatMessage::user("end".to_string()),
        ];
        let (keep, summarize) = mw.split_messages(&messages, 128_000);
        assert_eq!(keep.len(), 4);
        assert!(summarize.is_empty());
    }

    #[test]
    fn split_messages_excludes_system_plan_tool_and_prior_summary_messages() {
        use agent_primitives::types::Part;

        let (stub, _) = StubSummaryClient::new("");
        let mw = SummarizationMiddleware::new(
            SummarizationConfig {
                keep: KeepPolicy {
                    messages: Some(1),
                    tokens: None,
                    fraction: None,
                },
                ..SummarizationConfig::default()
            },
            stub,
        );
        let mut prior_summary = ChatMessage::assistant("old recap".to_string());
        prior_summary.is_summary = true;
        let plan_block = ChatMessage {
            role: "system".to_string(),
            content: vec![Part::Text {
                text: "<!-- plan-block:v1 -->\n[Plan]".to_string(),
            }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: true,
        };
        let assistant_tool = ChatMessage {
            role: "assistant".to_string(),
            content: vec![],
            tool_calls: Some(vec![ToolCall::new(
                "tool_1".to_string(),
                "read_file".to_string(),
                Value::Null,
            )]),
            tool_call_id: None,
            is_summary: false,
        };
        let tool_result = ChatMessage {
            role: "tool".to_string(),
            content: vec![Part::Text {
                text: "tool body".to_string(),
            }],
            tool_calls: None,
            tool_call_id: Some("tool_1".to_string()),
            is_summary: false,
        };
        let messages = vec![
            ChatMessage::system("stable prefix".to_string()),
            plan_block,
            ChatMessage::user("old prose".to_string()),
            ChatMessage::assistant("old assistant prose".to_string()),
            prior_summary,
            assistant_tool,
            tool_result,
            ChatMessage::user("recent prose".to_string()),
        ];

        let (keep, summarize) = mw.split_messages(&messages, 128_000);

        assert_eq!(summarize.len(), 2);
        assert_eq!(summarize[0].text_content(), "old prose");
        assert_eq!(summarize[1].text_content(), "old assistant prose");
        assert!(keep.iter().any(|m| m.text_content() == "stable prefix"));
        assert!(keep.iter().any(|m| m.text_content().contains("plan-block")));
        assert!(keep.iter().any(|m| m.text_content() == "old recap"));
        assert!(keep.iter().any(|m| m.tool_calls.is_some()));
        assert!(keep
            .iter()
            .any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("tool_1")));
        assert!(keep.iter().any(|m| m.text_content() == "recent prose"));
    }

    #[tokio::test]
    async fn from_config_constructs_with_summary_model_override() {
        let cfg = SummarizationConfig::default();
        let mw = SummarizationMiddleware::from_config(
            cfg,
            "openai",
            Some("gpt-4o-mini".to_string()),
            "gpt-4o",
            "test-key".to_string(),
            "https://api.openai.test".to_string(),
        )
        .await
        .expect("from_config should succeed with valid params");
        assert_eq!(mw.name(), "summarization");
        // Inner config still says enabled=false (default), so middleware reports disabled.
        assert!(!mw.enabled());
    }

    #[tokio::test]
    async fn from_config_falls_back_to_agent_model_when_summary_model_none() {
        let cfg = SummarizationConfig::default();
        let mw = SummarizationMiddleware::from_config(
            cfg,
            "openai",
            None,
            "gpt-4o",
            "test-key".to_string(),
            "https://api.openai.test".to_string(),
        )
        .await
        .expect("from_config should succeed");
        assert_eq!(mw.summary_client.model(), "gpt-4o");
    }

    #[test]
    fn build_conversation_text_joins_role_and_text() {
        let (stub, _) = StubSummaryClient::new("");
        let mw = make_middleware(SummarizationConfig::default(), stub);
        let text = mw.build_conversation_text(&[
            ChatMessage::user("hi".to_string()),
            ChatMessage::assistant("there".to_string()),
        ]);
        assert!(text.contains("user: hi"));
        assert!(text.contains("assistant: there"));
    }
}
