//! Canonical host context contracts through production Rig/provider construction.
use super::*;
use crate::{
    llm::{ChatResponse, LlmError, StreamCallback, StreamChunk},
    middleware::{MiddlewareContext, MiddlewareEffect, PreProcessMiddleware},
    rig_adapter::RigModelConfig,
    AgentEngine, ChatMessage, ExecutorConfig, ExecutorError, LlmClient, LlmConfig, McpManager,
    MiddlewarePipeline, StreamEvent, ToolRegistry,
};
use agent_primitives::{types::ContentSource, Part};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
struct Provider {
    requests: Mutex<Vec<Vec<ChatMessage>>>,
    tool_turns: usize,
    tools_per_turn: usize,
}
#[async_trait::async_trait]
impl LlmClient for Provider {
    fn model(&self) -> &str {
        "fixture"
    }
    fn provider(&self) -> &str {
        "fixture"
    }
    async fn chat(&self, _: Vec<ChatMessage>, _: Option<Value>) -> Result<ChatResponse, LlmError> {
        unreachable!()
    }
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        _: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let mut requests = self.requests.lock().unwrap();
        let turn = requests.len();
        requests.push(messages);
        let calls = (turn < self.tool_turns).then(|| {
            (0..self.tools_per_turn.max(1))
                .map(|index| {
                    crate::ToolCall::new(format!("new-{turn}-{index}"), "effect".into(), json!({}))
                })
                .collect()
        });
        if calls.is_none() {
            callback(StreamChunk::Token("done".into()));
        }
        Ok(ChatResponse {
            content: if calls.is_none() {
                "done".into()
            } else {
                String::new()
            },
            tool_calls: calls,
            reasoning: None,
            usage: None,
        })
    }
}

struct Effect {
    schema_size: usize,
}
#[async_trait::async_trait]
impl agent_primitives::Tool for Effect {
    fn name(&self) -> &str {
        "effect"
    }
    fn description(&self) -> &str {
        "context fixture"
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(
            json!({"type":"object","properties":{"value":{"type":"string","description":"schema ".repeat(self.schema_size)}}}),
        )
    }
    async fn execute(
        &self,
        context: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        context.set_state(
            "app:plan".into(),
            json!({"plan":[{"step":"host plan","status":"in_progress"}]}),
        );
        context.set_state("skill:graph".into(), json!({"alpha":{"tool_call_id":"skill-main","loaded_at":1,"resources":[{"path":"relative.md","tool_call_id":"skill-resource"}]}}));
        Ok(json!("new-tool-result"))
    }
}
fn build(
    config: ExecutorConfig,
    provider: Arc<Provider>,
    middleware: MiddlewarePipeline,
    schema_size: usize,
) -> RigAgentEngine<LlmCompletionModel> {
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(Effect { schema_size }));
    let prepared = PreparedExecution::new(
        config,
        provider,
        Arc::new(tools),
        Arc::new(McpManager::new()),
        Arc::new(middleware),
    );
    let rig = RigAgentConfig::new(
        "actor",
        "Actor",
        "fixture",
        "",
        RigModelConfig::from_llm_config(
            &LlmConfig::new(
                "http://unused".into(),
                String::new(),
                "fixture".into(),
                "fixture".into(),
            ),
            8192,
        ),
    );
    build_engine(prepared, rig)
}

#[derive(Clone, Default)]
struct PruneOnce {
    inputs: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
    contexts: Arc<Mutex<Vec<MiddlewareContext>>>,
}
#[async_trait::async_trait]
impl PreProcessMiddleware for PruneOnce {
    fn name(&self) -> &'static str {
        "fixture-prune"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(self.clone())
    }
    async fn process(
        &self,
        mut messages: Vec<ChatMessage>,
        context: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        let mut inputs = self.inputs.lock().unwrap();
        self.contexts.lock().unwrap().push(context.clone());
        inputs.push(messages.clone());
        if inputs.len() == 1 {
            messages.retain(|m| !m.text_content().starts_with("obsolete"));
        }
        Ok(MiddlewareEffect::ModifiedMessages(messages))
    }
}

#[tokio::test]
async fn full_host_history_and_pruned_context_survive_three_real_rig_requests() {
    let mut summary = ChatMessage::system("preserved summary".into());
    summary.is_summary = true;
    let mut multimodal = ChatMessage::user("multimodal prompt".into());
    multimodal.content.push(Part::Image {
        source: ContentSource::Base64("aGVsbG8=".into()),
        mime_type: "image/png".into(),
        detail: None,
    });
    multimodal.content.push(Part::File {
        source: ContentSource::Base64("cGRm".into()),
        mime_type: "application/pdf".into(),
        filename: Some("fixture.pdf".into()),
    });
    let mut assistant = ChatMessage::assistant("prior assistant".into());
    assistant.tool_calls = Some(vec![crate::ToolCall::new(
        "prior-id".into(),
        "effect".into(),
        json!({}),
    )]);
    let history = vec![
        summary,
        ChatMessage::user("obsolete question".into()),
        ChatMessage::assistant("obsolete answer".into()),
        multimodal.clone(),
        assistant,
        ChatMessage::tool_result("prior-id".into(), "prior-result".into()),
    ];
    let provider = Arc::new(Provider {
        tool_turns: 2,
        tools_per_turn: 2,
        ..Provider::default()
    });
    let middleware = PruneOnce::default();
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.system_instruction = Some("host authority".into());
    let engine = build(
        cfg,
        provider.clone(),
        MiddlewarePipeline::new()
            .add_pre_processor(Box::new(middleware.clone()))
            .add_pre_processor(Box::new(crate::PlanBlockMiddleware::new())),
        0,
    );
    engine.execute("current question", &history).await.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(middleware.inputs.lock().unwrap().len(), 3);
    for (turn, messages) in requests.iter().enumerate() {
        if turn > 0 {
            assert!(messages
                .iter()
                .any(|m| m.text_content().contains("host plan")));
        }
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.role == "system" && m.text_content() == "host authority")
                .count(),
            1
        );
        assert!(messages
            .iter()
            .any(|m| m.is_summary && m.text_content() == "preserved summary"));
        assert!(!messages
            .iter()
            .any(|m| m.text_content().starts_with("obsolete")));
        assert!(messages
            .iter()
            .any(|m| serde_json::to_value(&m.content).unwrap()
                == serde_json::to_value(&multimodal.content).unwrap()));
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.tool_call_id.as_deref() == Some("prior-id"))
                .count(),
            1
        );
        for prior_turn in 0..turn {
            for index in 0..2 {
                assert_eq!(
                    messages
                        .iter()
                        .filter(|m| m.tool_call_id.as_deref()
                            == Some(&format!("new-{prior_turn}-{index}")))
                        .count(),
                    1
                );
            }
        }
    }
    let contexts = middleware.contexts.lock().unwrap();
    assert_eq!(
        contexts[1].plan_state.as_ref().unwrap()["plan"][0]["step"],
        "host plan"
    );
    assert_eq!(
        contexts[1].execution_state.loaded_skills["alpha"].resource_tool_call_ids,
        ["skill-resource"]
    );
}

#[tokio::test]
async fn messages_and_tool_schema_are_budgeted_before_provider_and_zero_disables_limit() {
    for (budget, schema_size, prompt, allowed) in [
        (30, 0, "word ".repeat(100), false),
        (30, 100, "hi".into(), false),
        (0, 100, "word ".repeat(100), true),
        (500, 0, "hi".into(), true),
    ] {
        let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
        cfg.context_window_tokens = budget;
        let provider = Arc::new(Provider::default());
        let engine = build(
            cfg,
            provider.clone(),
            MiddlewarePipeline::new(),
            schema_size,
        );
        let result = engine.execute(&prompt, &[]).await;
        if allowed {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(ExecutorError::MiddlewareError(_))));
        }
        assert_eq!(
            provider.requests.lock().unwrap().len(),
            usize::from(allowed)
        );
    }
}

#[derive(Clone)]
struct Pending {
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl PreProcessMiddleware for Pending {
    fn name(&self) -> &'static str {
        "pending-preprocessor"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(self.clone())
    }
    async fn process(
        &self,
        _: Vec<ChatMessage>,
        _: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        futures::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_interrupts_pending_preprocessor_before_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(Provider::default());
    let engine = build(
        ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into()),
        provider.clone(),
        MiddlewarePipeline::new().add_pre_processor(Box::new(Pending {
            calls: calls.clone(),
        })),
        0,
    );
    let stop = Arc::new(AtomicBool::new(false));
    let mut events = Vec::new();
    let mut sink = |event| events.push(event);
    let run = engine.execute_stream_with_stop_flag("hi", &[], Some(stop.clone()), &mut sink);
    tokio::pin!(run);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            tokio::select! { result = &mut run => panic!("preprocessor not pending: {result:?}"), _ = tokio::task::yield_now() => {} }
            if calls.load(Ordering::SeqCst) > 0 { break; }
        }
    }).await.unwrap();
    stop.store(true, Ordering::Release);
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), run)
            .await
            .unwrap(),
        Err(ExecutorError::Stopped)
    ));
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[derive(Clone)]
struct EmitOrFail {
    fail: bool,
}
#[async_trait::async_trait]
impl PreProcessMiddleware for EmitOrFail {
    fn name(&self) -> &'static str {
        "emit-or-fail"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(self.clone())
    }
    async fn process(
        &self,
        _: Vec<ChatMessage>,
        _: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        if self.fail {
            Err("fixture policy failure".into())
        } else {
            Ok(MiddlewareEffect::EmitEvent(StreamEvent::WardChanged {
                timestamp: 1,
                ward_id: "policy-running".into(),
            }))
        }
    }
}

#[tokio::test]
async fn middleware_events_are_live_while_later_preprocessor_is_pending() {
    let provider = Arc::new(Provider::default());
    let pipeline = MiddlewarePipeline::new()
        .add_pre_processor(Box::new(EmitOrFail { fail: false }))
        .add_pre_processor(Box::new(Pending {
            calls: Arc::new(AtomicUsize::new(0)),
        }));
    let engine = build(
        ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into()),
        provider.clone(),
        pipeline,
        0,
    );
    let stop = Arc::new(AtomicBool::new(false));
    let mut events = Vec::new();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        engine.execute_stream_with_stop_flag("hi", &[], Some(stop.clone()), &mut |event| {
            if matches!(event, StreamEvent::WardChanged { .. }) {
                stop.store(true, Ordering::Release);
            }
            events.push(event);
        }),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(ExecutorError::Stopped)));
    assert!(events.iter().any(|event| matches!(event, StreamEvent::WardChanged { ward_id, .. } if ward_id == "policy-running")));
    assert!(!events
        .iter()
        .any(|event| matches!(event, StreamEvent::Done { .. })));
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failing_middleware_remains_typed_and_makes_zero_provider_calls() {
    let provider = Arc::new(Provider::default());
    let engine = build(
        ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into()),
        provider.clone(),
        MiddlewarePipeline::new().add_pre_processor(Box::new(EmitOrFail { fail: true })),
        0,
    );
    assert!(
        matches!(engine.execute("hi", &[]).await, Err(ExecutorError::MiddlewareError(error)) if error == "fixture policy failure")
    );
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn independently_built_sessions_do_not_share_canonical_context() {
    let first = Arc::new(Provider {
        tool_turns: 2,
        ..Provider::default()
    });
    let second = Arc::new(Provider {
        tool_turns: 2,
        ..Provider::default()
    });
    let first_engine = build(
        ExecutorConfig::new("first".into(), "fixture".into(), "fixture".into()),
        first.clone(),
        MiddlewarePipeline::new(),
        0,
    );
    let second_engine = build(
        ExecutorConfig::new("second".into(), "fixture".into(), "fixture".into()),
        second.clone(),
        MiddlewarePipeline::new(),
        0,
    );
    let (a, b) = tokio::join!(
        first_engine.execute("first-only", &[]),
        second_engine.execute("second-only", &[])
    );
    a.unwrap();
    b.unwrap();
    for (provider, own, other) in [
        (first, "first-only", "second-only"),
        (second, "second-only", "first-only"),
    ] {
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        for request in requests.iter() {
            let text = request
                .iter()
                .map(ChatMessage::text_content)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains(own));
            assert!(!text.contains(other));
        }
    }
}

#[derive(Clone, Default)]
struct GrowSecondRequest(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl PreProcessMiddleware for GrowSecondRequest {
    fn name(&self) -> &'static str {
        "grow-second-request"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(self.clone())
    }
    async fn process(
        &self,
        mut messages: Vec<ChatMessage>,
        _: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 1 {
            messages.push(ChatMessage::system("too large ".repeat(1000)));
        }
        Ok(MiddlewareEffect::ModifiedMessages(messages))
    }
}

#[tokio::test]
async fn later_request_must_pass_budget_after_middleware_changes() {
    let provider = Arc::new(Provider {
        tool_turns: 2,
        ..Provider::default()
    });
    let middleware = GrowSecondRequest::default();
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.context_window_tokens = 500;
    let engine = build(
        cfg,
        provider.clone(),
        MiddlewarePipeline::new().add_pre_processor(Box::new(middleware.clone())),
        0,
    );
    assert!(
        matches!(engine.execute("hi", &[]).await, Err(ExecutorError::MiddlewareError(error)) if error.contains("input token budget"))
    );
    assert_eq!(middleware.0.load(Ordering::SeqCst), 2);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}
