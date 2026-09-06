//! Control contracts exercise the production provider bridge, not a Rig-only stub.
use super::*;
use crate::{
    llm::{ChatMessage, ChatResponse, LlmError, StreamCallback},
    mcp::{McpClient, McpError, McpTool},
    rig_adapter::RigModelConfig,
    AgentEngine, ExecutorConfig, ExecutorError, LlmClient, LlmConfig, McpManager,
    MiddlewarePipeline, StreamEvent, ToolRegistry,
};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct PendingProvider {
    calls: AtomicUsize,
    entered: Notify,
    dropped: Arc<Notify>,
}
struct OnDrop(Arc<Notify>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
#[async_trait::async_trait]
impl LlmClient for PendingProvider {
    fn model(&self) -> &str {
        "fixture-model"
    }
    fn provider(&self) -> &str {
        "fixture-provider"
    }
    async fn chat(&self, _: Vec<ChatMessage>, _: Option<Value>) -> Result<ChatResponse, LlmError> {
        unreachable!()
    }
    async fn chat_stream(
        &self,
        _: Vec<ChatMessage>,
        _: Option<Value>,
        _: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let _drop = OnDrop(self.dropped.clone());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        futures::future::pending().await
    }
}

fn engine(
    provider: Arc<dyn LlmClient>,
    manager: Arc<McpManager>,
) -> RigAgentEngine<LlmCompletionModel> {
    let cfg = ExecutorConfig::new(
        "actor".into(),
        "fixture-provider".into(),
        "fixture-model".into(),
    );
    let prepared = PreparedExecution::new(
        cfg,
        provider,
        Arc::new(ToolRegistry::new()),
        manager,
        Arc::new(MiddlewarePipeline::new()),
    );
    // Optional adapter config may predate resolved execution inputs.
    let rig = RigAgentConfig::new(
        "stale-actor",
        "Actor",
        "fixture-provider",
        "",
        RigModelConfig::from_llm_config(
            &LlmConfig::new(
                "http://unused".into(),
                String::new(),
                "stale-model".into(),
                "stale-provider".into(),
            ),
            8192,
        ),
    );
    build_engine(prepared, rig)
}

#[tokio::test]
async fn preset_stop_makes_zero_provider_calls_and_no_done() {
    let provider = Arc::new(PendingProvider::default());
    let engine = engine(provider.clone(), Arc::new(McpManager::new()));
    let mut events = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        engine.execute_stream_with_stop_flag(
            "hi",
            &[],
            Some(Arc::new(AtomicBool::new(true))),
            &mut |event| events.push(event),
        ),
    )
    .await
    .expect("pre-set stop is immediate");
    assert!(matches!(result, Err(ExecutorError::Stopped)));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(
        matches!(events.first(), Some(StreamEvent::Metadata { agent_id, model, provider, .. }) if agent_id == "actor" && model == "fixture-model" && provider == "fixture-provider")
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event, StreamEvent::Done { .. })));
}

struct SlowClose {
    entered: Notify,
    release: Notify,
    finished: Notify,
}
#[async_trait::async_trait]
impl McpClient for SlowClose {
    fn name(&self) -> &str {
        "cleanup-fixture"
    }
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        Ok(Vec::new())
    }
    async fn call_tool(&self, _: &str, _: Value) -> Result<Value, McpError> {
        unreachable!()
    }
    async fn close(&self) {
        self.entered.notify_one();
        self.release.notified().await;
        self.finished.notify_one();
    }
}

#[tokio::test]
async fn pending_provider_stop_is_prompt_and_cleanup_finishes_independently() {
    let provider = Arc::new(PendingProvider::default());
    let manager = Arc::new(McpManager::new());
    let client = Arc::new(SlowClose {
        entered: Notify::new(),
        release: Notify::new(),
        finished: Notify::new(),
    });
    manager.insert_test_client("cleanup", client.clone()).await;
    let engine = engine(provider.clone(), manager);
    let stop = Arc::new(AtomicBool::new(false));
    let mut events = Vec::new();
    let mut sink = |event| events.push(event);
    let run = engine.execute_stream_with_stop_flag("hi", &[], Some(stop.clone()), &mut sink);
    tokio::pin!(run);
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! { result = &mut run => panic!("provider did not stall: {result:?}"), () = provider.entered.notified() => {} }
    }).await.unwrap();
    stop.store(true, Ordering::Release);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), &mut run)
            .await
            .expect("stop interrupts pending provider"),
        Err(ExecutorError::Stopped)
    ));
    tokio::time::timeout(Duration::from_secs(1), provider.dropped.notified())
        .await
        .expect("provider request dropped");
    tokio::time::timeout(Duration::from_secs(1), client.entered.notified())
        .await
        .expect("cleanup scheduled even while engine lives");
    client.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), client.finished.notified())
        .await
        .expect("cleanup completes");
}

#[tokio::test]
async fn pending_provider_emits_ten_second_heartbeat_after_metadata() {
    let provider = Arc::new(PendingProvider::default());
    let engine = engine(provider.clone(), Arc::new(McpManager::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let started = tokio::time::Instant::now();
    let mut events = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(12),
        engine.execute_stream_with_stop_flag("hi", &[], Some(stop.clone()), &mut |event| {
            if matches!(event, StreamEvent::Heartbeat { .. }) {
                assert!(started.elapsed() >= Duration::from_secs(10));
                stop.store(true, Ordering::Release);
            }
            events.push(event);
        }),
    )
    .await
    .expect("heartbeat while provider is pending");
    assert!(matches!(result, Err(ExecutorError::Stopped)));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        events.as_slice(),
        [StreamEvent::Metadata { .. }, StreamEvent::Heartbeat { .. },StreamEvent::ContextState {state,..}]
        if state[crate::engine::snapshot::CHECKPOINT_KEY]["version"]==1
    ));
}

struct TwoCalls;
#[async_trait::async_trait]
impl LlmClient for TwoCalls {
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
        _: Vec<ChatMessage>,
        _: Option<Value>,
        _: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: Some(
                (0..2)
                    .map(|i| {
                        crate::ToolCall::new(
                            format!("call-{i}"),
                            "effect".into(),
                            serde_json::json!({}),
                        )
                    })
                    .collect(),
            ),
            reasoning: None,
            usage: None,
        })
    }
}

struct FiniteCalls {
    requests: AtomicUsize,
    finish_after: usize,
}
#[async_trait::async_trait]
impl LlmClient for FiniteCalls {
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
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        if self.requests.fetch_add(1, Ordering::SeqCst) < self.finish_after {
            TwoCalls.chat_stream(messages, tools, callback).await
        } else {
            callback(crate::llm::StreamChunk::Token("done".into()));
            Ok(ChatResponse {
                content: "done".into(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
    }
}

#[tokio::test]
async fn configured_hard_turn_limits_and_single_action_reach_rig() {
    // Legacy ticks before checking >= hardmax: N permits N-1 model turns.
    // Zero disables that limit; 53 calls proves no hidden Rig default of 50.
    for (limit, single_action, expected_requests, expected_effects) in [
        (1, false, 0, 0),
        (2, false, 1, 2),
        (4, false, 3, 6),
        (0, false, 53, 104),
        (0, true, 53, 52),
    ] {
        let provider = Arc::new(FiniteCalls {
            requests: AtomicUsize::new(0),
            finish_after: 52,
        });
        let effects = Arc::new(AtomicUsize::new(0));
        let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
        cfg.max_turns = limit;
        cfg.single_action_mode = single_action;
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(Effect(effects.clone())));
        let prepared = PreparedExecution::new(
            cfg,
            provider.clone(),
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
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
        let engine = build_engine(prepared, rig);
        let mut events = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(2),
            engine.execute_stream("hi", &[], &mut |event| events.push(event)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            provider.requests.load(Ordering::SeqCst),
            expected_requests,
            "hardmax={limit}"
        );
        assert_eq!(
            effects.load(Ordering::SeqCst),
            expected_effects,
            "single_action={single_action}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done { .. }))
                .count(),
            1
        );
        if limit > 0 {
            assert!(events.iter().any(|event| matches!(event, StreamEvent::Done { final_message, .. } if final_message.contains(&format!("after {limit} iterations")))));
        }
    }
}
struct Effect(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl agent_primitives::Tool for Effect {
    fn name(&self) -> &str {
        "effect"
    }
    fn description(&self) -> &str {
        "count real dispatch"
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(serde_json::json!({"type":"object","properties":{}}))
    }
    async fn execute(
        &self,
        _: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!("effect"))
    }
}

#[tokio::test]
async fn observed_stop_prevents_next_authoritative_tool_call() {
    assert_tool_stop(false).await;
    assert_tool_stop(true).await;
}

async fn assert_tool_stop(stop_before_dispatch: bool) {
    let effects = Arc::new(AtomicUsize::new(0));
    let cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Effect(effects.clone())));
    let prepared = PreparedExecution::new(
        cfg,
        Arc::new(TwoCalls),
        Arc::new(registry),
        Arc::new(McpManager::new()),
        Arc::new(MiddlewarePipeline::new()),
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
    let engine = build_engine(prepared, rig);
    let stop = Arc::new(AtomicBool::new(false));
    let mut events = Vec::new();
    let result = engine
        .execute_stream_with_stop_flag("hi", &[], Some(stop.clone()), &mut |event| {
            if matches!(event, StreamEvent::ToolResult { .. })
                || (stop_before_dispatch && matches!(event, StreamEvent::ToolCallStart { .. }))
            {
                stop.store(true, Ordering::Release);
            }
            events.push(event);
        })
        .await;
    assert!(matches!(result, Err(ExecutorError::Stopped)));
    assert_eq!(
        effects.load(Ordering::SeqCst),
        usize::from(!stop_before_dispatch)
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolCallStart { .. }))
            .count(),
        1
    );
    assert!(!events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
}
