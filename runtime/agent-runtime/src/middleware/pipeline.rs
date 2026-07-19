// ============================================================================
// MIDDLEWARE PIPELINE
// Orchestrates middleware execution
// ============================================================================

//! # Middleware Pipeline
//!
//! Pipeline that orchestrates middleware execution.

use super::traits::{EventMiddleware, MiddlewareContext, PreProcessMiddleware};
use crate::types::{ChatMessage, StreamEvent};

/// Middleware pipeline that orchestrates preprocessing and event handling
pub struct MiddlewarePipeline {
    /// Pre-process middleware (executed in order before LLM call)
    pre_processors: Vec<Box<dyn PreProcessMiddleware>>,

    /// Event handlers (executed when events are emitted)
    event_handlers: Vec<Box<dyn EventMiddleware>>,
}

impl Default for MiddlewarePipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl MiddlewarePipeline {
    /// Create a new empty pipeline
    #[must_use]
    pub fn new() -> Self {
        Self {
            pre_processors: Vec::new(),
            event_handlers: Vec::new(),
        }
    }

    /// Create a pipeline with pre-process middleware
    #[must_use]
    pub fn with_pre_processors(mut self, middleware: Vec<Box<dyn PreProcessMiddleware>>) -> Self {
        self.pre_processors = middleware;
        self
    }

    /// Create a pipeline with event handlers
    #[must_use]
    pub fn with_event_handlers(mut self, handlers: Vec<Box<dyn EventMiddleware>>) -> Self {
        self.event_handlers = handlers;
        self
    }

    /// Add a pre-process middleware to the pipeline
    #[must_use]
    pub fn add_pre_processor(mut self, middleware: Box<dyn PreProcessMiddleware>) -> Self {
        self.pre_processors.push(middleware);
        self
    }

    /// Add an event handler to the pipeline
    #[must_use]
    pub fn add_event_handler(mut self, handler: Box<dyn EventMiddleware>) -> Self {
        self.event_handlers.push(handler);
        self
    }

    /// Process messages through all pre-process middleware
    ///
    /// # Arguments
    /// * `messages` - Input messages to process
    /// * `context` - Execution context
    /// * `on_event` - Callback to emit events
    ///
    /// # Returns
    /// Processed messages ready for LLM execution
    pub async fn process_messages(
        &self,
        messages: Vec<ChatMessage>,
        context: &MiddlewareContext,
        mut on_event: impl FnMut(StreamEvent),
    ) -> Result<Vec<ChatMessage>, String> {
        let mut current_messages = messages;

        // Process through each pre-processor in order
        for middleware in &self.pre_processors {
            if !middleware.enabled() {
                continue;
            }

            // Clone before passing to middleware — if it returns Proceed,
            // the original messages are preserved. If it returns Modified/EmitAndModify,
            // we use the new messages (clone is discarded).
            let backup = current_messages.clone();
            match middleware
                .process(std::mem::take(&mut current_messages), context)
                .await?
            {
                super::traits::MiddlewareEffect::ModifiedMessages(msgs) => {
                    current_messages = msgs;
                }
                super::traits::MiddlewareEffect::Proceed => {
                    // Middleware didn't modify — restore from backup
                    current_messages = backup;
                }
                super::traits::MiddlewareEffect::EmitEvent(event) => {
                    on_event(event);
                    current_messages = backup;
                }
                super::traits::MiddlewareEffect::EmitAndModify {
                    event,
                    messages: msgs,
                } => {
                    on_event(event);
                    current_messages = msgs;
                }
            }
        }

        Ok(current_messages)
    }

    /// Handle an event through all event handlers
    ///
    /// # Arguments
    /// * `event` - The event that was emitted
    /// * `context` - Execution context
    pub async fn handle_event(
        &self,
        event: &StreamEvent,
        context: &MiddlewareContext,
    ) -> Result<(), String> {
        for handler in &self.event_handlers {
            if !handler.enabled() {
                continue;
            }

            // Continue processing even if one handler fails
            if let Err(e) = handler.on_event(event, context).await {
                tracing::warn!(
                    "Middleware {} failed to handle event: {}",
                    handler.name(),
                    e
                );
            }
        }

        Ok(())
    }

    /// Get the number of pre-process middleware in the pipeline
    #[must_use]
    pub fn pre_processor_count(&self) -> usize {
        self.pre_processors.len()
    }

    /// Get the number of event handlers in the pipeline
    #[must_use]
    pub fn event_handler_count(&self) -> usize {
        self.event_handlers.len()
    }

    /// Get names of all pre-process middleware in configured order.
    #[must_use]
    pub fn pre_processor_names(&self) -> Vec<&'static str> {
        self.pre_processors.iter().map(|m| m.name()).collect()
    }

    /// Get names of all event handlers in configured order.
    #[must_use]
    pub fn event_handler_names(&self) -> Vec<&'static str> {
        self.event_handlers.iter().map(|m| m.name()).collect()
    }

    /// Get names of all enabled pre-process middleware
    #[must_use]
    pub fn enabled_pre_processors(&self) -> Vec<&'static str> {
        self.pre_processors
            .iter()
            .filter(|m| m.enabled())
            .map(|m| m.name())
            .collect()
    }

    /// Get names of all enabled event handlers
    #[must_use]
    pub fn enabled_event_handlers(&self) -> Vec<&'static str> {
        self.event_handlers
            .iter()
            .filter(|m| m.enabled())
            .map(|m| m.name())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock pre-process middleware for testing
    struct MockPreProcessor {
        enabled: bool,
        name: &'static str,
    }

    #[async_trait::async_trait]
    impl PreProcessMiddleware for MockPreProcessor {
        fn name(&self) -> &'static str {
            self.name
        }

        fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
            Box::new(MockPreProcessor {
                enabled: self.enabled,
                name: self.name,
            })
        }

        fn enabled(&self) -> bool {
            self.enabled
        }

        async fn process(
            &self,
            messages: Vec<ChatMessage>,
            _context: &MiddlewareContext,
        ) -> Result<super::super::traits::MiddlewareEffect, String> {
            Ok(super::super::traits::MiddlewareEffect::ModifiedMessages(
                messages,
            ))
        }
    }

    #[test]
    fn test_pipeline_creation() {
        let pipeline = MiddlewarePipeline::new();
        assert_eq!(pipeline.pre_processor_count(), 0);
        assert_eq!(pipeline.event_handler_count(), 0);
    }

    #[test]
    fn test_enabled_middleware() {
        let enabled = Box::new(MockPreProcessor {
            enabled: true,
            name: "test",
        }) as Box<dyn PreProcessMiddleware>;
        let disabled = Box::new(MockPreProcessor {
            enabled: false,
            name: "test2",
        }) as Box<dyn PreProcessMiddleware>;

        let pipeline = MiddlewarePipeline::new()
            .add_pre_processor(enabled)
            .add_pre_processor(disabled);

        assert_eq!(pipeline.enabled_pre_processors().len(), 1);
    }

    #[test]
    fn pre_processor_names_include_disabled_in_order() {
        let enabled = Box::new(MockPreProcessor {
            enabled: true,
            name: "context_editing",
        }) as Box<dyn PreProcessMiddleware>;
        let disabled = Box::new(MockPreProcessor {
            enabled: false,
            name: "summarization",
        }) as Box<dyn PreProcessMiddleware>;

        let pipeline = MiddlewarePipeline::new()
            .add_pre_processor(enabled)
            .add_pre_processor(disabled);

        assert_eq!(
            pipeline.pre_processor_names(),
            vec!["context_editing", "summarization"]
        );
        assert_eq!(pipeline.enabled_pre_processors(), vec!["context_editing"]);
    }

    // ----------------------------------------------------------------------
    // Effect-handling tests
    // ----------------------------------------------------------------------

    use super::super::traits::MiddlewareEffect;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Middleware that returns a configurable effect.
    struct ScriptedMiddleware {
        name: &'static str,
        enabled: bool,
        // Each call dequeues one of these, by index.
        effect_factory: Arc<dyn Fn(Vec<ChatMessage>) -> MiddlewareEffect + Send + Sync>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl PreProcessMiddleware for ScriptedMiddleware {
        fn name(&self) -> &'static str {
            self.name
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
            Box::new(ScriptedMiddleware {
                name: self.name,
                enabled: self.enabled,
                effect_factory: Arc::clone(&self.effect_factory),
                calls: Arc::clone(&self.calls),
            })
        }
        async fn process(
            &self,
            messages: Vec<ChatMessage>,
            _ctx: &MiddlewareContext,
        ) -> Result<MiddlewareEffect, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok((self.effect_factory)(messages))
        }
    }

    fn make_ctx() -> MiddlewareContext {
        MiddlewareContext::new(
            "agent".to_string(),
            None,
            "test".to_string(),
            "test-model".to_string(),
        )
    }

    fn ev() -> StreamEvent {
        StreamEvent::Token {
            timestamp: 1,
            content: "evt".to_string(),
        }
    }

    fn user(t: &str) -> ChatMessage {
        ChatMessage::user(t.to_string())
    }

    #[tokio::test]
    async fn process_messages_proceed_keeps_input_unmodified() {
        let scripted = ScriptedMiddleware {
            name: "proc",
            enabled: true,
            effect_factory: Arc::new(|_msgs| MiddlewareEffect::Proceed),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let pipeline = MiddlewarePipeline::new().add_pre_processor(Box::new(scripted));
        let ctx = make_ctx();
        let mut emitted = Vec::new();
        let out = pipeline
            .process_messages(vec![user("a"), user("b")], &ctx, |e| emitted.push(e))
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text_content(), "a");
        assert!(emitted.is_empty());
    }

    #[tokio::test]
    async fn process_messages_modified_replaces() {
        let scripted = ScriptedMiddleware {
            name: "mod",
            enabled: true,
            effect_factory: Arc::new(|_msgs| {
                MiddlewareEffect::ModifiedMessages(vec![user("rewritten")])
            }),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let pipeline = MiddlewarePipeline::new().add_pre_processor(Box::new(scripted));
        let ctx = make_ctx();
        let mut emitted = Vec::new();
        let out = pipeline
            .process_messages(vec![user("a"), user("b")], &ctx, |e| emitted.push(e))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text_content(), "rewritten");
        assert!(emitted.is_empty());
    }

    #[tokio::test]
    async fn process_messages_emit_event_keeps_messages() {
        let scripted = ScriptedMiddleware {
            name: "emit",
            enabled: true,
            effect_factory: Arc::new(|_msgs| MiddlewareEffect::EmitEvent(ev())),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let pipeline = MiddlewarePipeline::new().add_pre_processor(Box::new(scripted));
        let ctx = make_ctx();
        let mut emitted = Vec::new();
        let out = pipeline
            .process_messages(vec![user("a")], &ctx, |e| emitted.push(e))
            .await
            .unwrap();
        // EmitEvent does not modify messages
        assert_eq!(out.len(), 1);
        assert_eq!(emitted.len(), 1);
        assert!(matches!(emitted[0], StreamEvent::Token { .. }));
    }

    #[tokio::test]
    async fn process_messages_emit_and_modify_does_both() {
        let scripted = ScriptedMiddleware {
            name: "em-mod",
            enabled: true,
            effect_factory: Arc::new(|_msgs| MiddlewareEffect::EmitAndModify {
                event: ev(),
                messages: vec![user("new")],
            }),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let pipeline = MiddlewarePipeline::new().add_pre_processor(Box::new(scripted));
        let ctx = make_ctx();
        let mut emitted = Vec::new();
        let out = pipeline
            .process_messages(vec![user("a")], &ctx, |e| emitted.push(e))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text_content(), "new");
        assert_eq!(emitted.len(), 1);
    }

    #[tokio::test]
    async fn process_messages_skips_disabled_middleware() {
        let calls = Arc::new(AtomicUsize::new(0));
        let scripted = ScriptedMiddleware {
            name: "off",
            enabled: false,
            effect_factory: Arc::new(|_msgs| {
                MiddlewareEffect::ModifiedMessages(vec![user("should-not-run")])
            }),
            calls: Arc::clone(&calls),
        };
        let pipeline = MiddlewarePipeline::new().add_pre_processor(Box::new(scripted));
        let ctx = make_ctx();
        let mut emitted = Vec::new();
        let out = pipeline
            .process_messages(vec![user("a")], &ctx, |e| emitted.push(e))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text_content(), "a");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // Event-handler tests --------------------------------------------------

    struct MockEvent {
        name: &'static str,
        enabled: bool,
        received: Arc<AtomicUsize>,
        // If true, the handler returns Err to exercise the warn branch.
        fail: bool,
    }

    #[async_trait::async_trait]
    impl EventMiddleware for MockEvent {
        fn name(&self) -> &'static str {
            self.name
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn clone_box(&self) -> Box<dyn EventMiddleware> {
            Box::new(MockEvent {
                name: self.name,
                enabled: self.enabled,
                received: Arc::clone(&self.received),
                fail: self.fail,
            })
        }
        async fn on_event(&self, _e: &StreamEvent, _c: &MiddlewareContext) -> Result<(), String> {
            self.received.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err("boom".to_string())
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn handle_event_invokes_only_enabled() {
        let on_count = Arc::new(AtomicUsize::new(0));
        let off_count = Arc::new(AtomicUsize::new(0));
        let pipeline = MiddlewarePipeline::new()
            .add_event_handler(Box::new(MockEvent {
                name: "on",
                enabled: true,
                received: Arc::clone(&on_count),
                fail: false,
            }))
            .add_event_handler(Box::new(MockEvent {
                name: "off",
                enabled: false,
                received: Arc::clone(&off_count),
                fail: false,
            }));
        pipeline.handle_event(&ev(), &make_ctx()).await.unwrap();
        assert_eq!(on_count.load(Ordering::SeqCst), 1);
        assert_eq!(off_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn handle_event_continues_after_failure() {
        let first_count = Arc::new(AtomicUsize::new(0));
        let second_count = Arc::new(AtomicUsize::new(0));
        let pipeline = MiddlewarePipeline::new()
            .add_event_handler(Box::new(MockEvent {
                name: "fail",
                enabled: true,
                received: Arc::clone(&first_count),
                fail: true,
            }))
            .add_event_handler(Box::new(MockEvent {
                name: "ok",
                enabled: true,
                received: Arc::clone(&second_count),
                fail: false,
            }));
        // Should not propagate error
        pipeline.handle_event(&ev(), &make_ctx()).await.unwrap();
        assert_eq!(first_count.load(Ordering::SeqCst), 1);
        assert_eq!(second_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn enabled_event_handlers_filters() {
        let pipeline = MiddlewarePipeline::new().with_event_handlers(vec![
            Box::new(MockEvent {
                name: "on",
                enabled: true,
                received: Arc::new(AtomicUsize::new(0)),
                fail: false,
            }) as Box<dyn EventMiddleware>,
            Box::new(MockEvent {
                name: "off",
                enabled: false,
                received: Arc::new(AtomicUsize::new(0)),
                fail: false,
            }) as Box<dyn EventMiddleware>,
        ]);
        assert_eq!(pipeline.event_handler_count(), 2);
        assert_eq!(pipeline.event_handler_names(), vec!["on", "off"]);
        assert_eq!(pipeline.enabled_event_handlers(), vec!["on"]);
    }

    #[tokio::test]
    async fn runtime_context_controller_clears_tools_then_injects_plan_block() {
        use crate::middleware::config::ContextEditingConfig;
        use crate::middleware::{ContextEditingMiddleware, PlanBlockMiddleware};
        use crate::types::ToolCall;
        use agent_primitives::types::Part;
        use serde_json::json;

        let tool_calls = vec![
            ToolCall::new(
                "call_1".to_string(),
                "read_file".to_string(),
                json!({"path": "old.rs"}),
            ),
            ToolCall::new(
                "call_2".to_string(),
                "read_file".to_string(),
                json!({"path": "recent.rs"}),
            ),
        ];
        let messages = vec![
            ChatMessage::system("base system".to_string()),
            ChatMessage::user("old user prose".to_string()),
            ChatMessage::assistant("old assistant prose should survive".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(tool_calls),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "old tool body that should be cleared".repeat(20),
                }],
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "recent tool body".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_2".to_string()),
                is_summary: false,
            },
            ChatMessage::user("current user".to_string()),
        ];

        let pipeline = MiddlewarePipeline::new()
            .add_pre_processor(Box::new(ContextEditingMiddleware::new(
                ContextEditingConfig {
                    enabled: true,
                    trigger_tokens: 1,
                    keep_tool_results: 1,
                    min_reclaim: 0,
                    clear_tool_inputs: true,
                    placeholder: "[cleared]".to_string(),
                    ..Default::default()
                },
            )))
            .add_pre_processor(Box::new(PlanBlockMiddleware::new()));
        let ctx = make_ctx().with_plan_state(Some(json!({
            "plan": [{"step": "keep the current task visible", "status": "in_progress"}]
        })));

        let mut events = Vec::new();
        let out = pipeline
            .process_messages(messages, &ctx, |event| events.push(event))
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(out[0].text_content(), "base system");
        assert_eq!(out[1].role, "system");
        assert!(out[1].is_summary);
        assert!(out[1]
            .text_content()
            .contains("keep the current task visible"));
        assert!(out
            .iter()
            .any(|m| m.text_content() == "old assistant prose should survive"));
        assert!(out.iter().any(|m| {
            m.role == "tool"
                && m.tool_call_id.as_deref() == Some("call_1")
                && m.text_content() == "[cleared]"
        }));
        assert!(out.iter().any(|m| {
            m.role == "tool"
                && m.tool_call_id.as_deref() == Some("call_2")
                && m.text_content() == "recent tool body"
        }));
        let assistant_with_tools = out
            .iter()
            .find(|m| m.role == "assistant" && m.tool_calls.is_some())
            .expect("assistant tool-call message remains paired");
        let ids: Vec<_> = assistant_with_tools
            .tool_calls
            .as_ref()
            .unwrap()
            .iter()
            .map(|tc| tc.id.as_str())
            .collect();
        assert_eq!(ids, vec!["call_1", "call_2"]);
    }

    #[test]
    fn with_pre_processors_seeds_list() {
        let pre: Vec<Box<dyn PreProcessMiddleware>> = vec![Box::new(MockPreProcessor {
            enabled: true,
            name: "x",
        })];
        let pipeline = MiddlewarePipeline::new().with_pre_processors(pre);
        assert_eq!(pipeline.pre_processor_count(), 1);
    }

    #[test]
    fn default_pipeline_is_empty() {
        let p = MiddlewarePipeline::default();
        assert_eq!(p.pre_processor_count(), 0);
        assert_eq!(p.event_handler_count(), 0);
    }
}
