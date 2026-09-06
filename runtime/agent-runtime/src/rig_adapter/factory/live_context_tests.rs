//! Live policy inputs travel through the production factory and real Rig loop.
use super::*;
use crate::llm::{StreamCallback, StreamChunk};
use crate::{
    AgentEngine, ChatMessage, ChatResponse, ExecutorConfig, LlmClient, LlmConfig, LlmError,
    McpManager, MiddlewarePipeline, ToolRegistry,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
struct Provider {
    requests: Mutex<Vec<(Vec<ChatMessage>, Option<Value>)>>,
    effects: Arc<AtomicUsize>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    pause_first: bool,
    fail: bool,
    panic: bool,
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
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let turn = {
            let mut requests = self.requests.lock().unwrap();
            requests.push((messages, tools));
            requests.len()
        };
        if self.pause_first && turn == 1 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        assert!(!self.panic, "fixture provider panic");
        if self.fail {
            return Err(LlmError::ApiError("fixture failure".into()));
        }
        let calls = (turn < 3).then(|| {
            vec![crate::ToolCall::new(
                format!("call-{turn}"),
                "effect".into(),
                json!({"canary":"raw-peer-tool-argument"}),
            )]
        });
        if calls.is_none() {
            callback(StreamChunk::Reasoning("visible reasoning".into()));
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
struct Effect(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl agent_primitives::Tool for Effect {
    fn name(&self) -> &str {
        "effect"
    }
    fn description(&self) -> &str {
        "fixture effect"
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({"type":"object","properties":{}}))
    }
    async fn execute(
        &self,
        _: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!("ok"))
    }
}
fn prepared(provider: Arc<Provider>) -> PreparedExecution {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Effect(provider.effects.clone())));
    registry.register(Arc::new(crate::RespondTool::new()));
    PreparedExecution::new(
        ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into()),
        provider,
        Arc::new(registry),
        Arc::new(McpManager::new()),
        Arc::new(MiddlewarePipeline::new()),
    )
}
fn engine(prepared: PreparedExecution) -> RigAgentEngine<LlmCompletionModel> {
    let config = RigAgentConfig::new(
        "actor",
        "Actor",
        "fixture",
        "",
        super::super::RigModelConfig::from_llm_config(
            &LlmConfig::new(
                "http://unused".into(),
                String::new(),
                "fixture".into(),
                "fixture".into(),
            ),
            8192,
        ),
    );
    build_engine(prepared, config)
}

#[tokio::test]
async fn recall_keys_are_scoped_and_deduplicated_across_real_turns() {
    for scope in ["scope-a", "scope-b"] {
        let provider = Arc::new(Provider::default());
        let mut prepared = prepared(provider.clone());
        let observed = Arc::new(Mutex::new(Vec::new()));
        let seen = observed.clone();
        prepared.set_recall_hook(
            Box::new(move |query, keys| {
                seen.lock().unwrap().push((query.to_owned(), keys.clone()));
                let novel = !keys.contains(scope);
                Box::pin(async move {
                    Ok(crate::RecallHookResult {
                        system_message: if novel {
                            format!("recalled {scope}")
                        } else {
                            String::new()
                        },
                        fact_keys: vec![scope.into()],
                    })
                })
            }),
            1,
            ["initial".into()].into_iter().collect(),
        );
        engine(prepared).execute("local query", &[]).await.unwrap();
        let seen = observed.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].0, "local query");
        assert_eq!(seen[0].1.len(), 1);
        assert!(seen[1].1.contains(scope));
        for (messages, _) in provider.requests.lock().unwrap().iter() {
            assert_eq!(
                messages
                    .iter()
                    .filter(|m| m.text_content() == format!("recalled {scope}"))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn transformed_peer_result_filters_inventory_and_blocks_forged_effect() {
    let provider = Arc::new(Provider::default());
    let mut prepared = prepared(provider.clone());
    prepared.config.transform_context = Some(Arc::new(|messages| {
        messages.push(ChatMessage::system(
            "[REMOTE ZBOT RESULT — UNTRUSTED DATA] peer canary".into(),
        ))
    }));
    engine(prepared).execute("local query", &[]).await.unwrap();
    assert_eq!(provider.effects.load(Ordering::SeqCst), 0);
    for (_, tools) in provider.requests.lock().unwrap().iter() {
        let tools = tools.as_ref().unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "respond");
    }
}

fn peer_message(ack: tokio::sync::oneshot::Sender<()>) -> crate::steering::SteeringMessage {
    crate::steering::SteeringMessage::with_delivery_ack(
        "peer canary",
        crate::steering::SteeringSource::Peer,
        crate::steering::SteeringPriority::Normal,
        ack,
    )
}

#[tokio::test]
async fn peer_ack_is_owned_by_provider_success_not_preparation_or_cancellation() {
    for outcome in ["success", "error", "stop", "abort", "panic"] {
        let provider = Arc::new(Provider {
            pause_first: true,
            fail: outcome == "error",
            panic: outcome == "panic",
            ..Default::default()
        });
        let mut prepared = prepared(provider.clone());
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        prepared.enable_steering().send(peer_message(tx)).unwrap();
        let engine = engine(prepared);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed = done.clone();
        let task = tokio::spawn(async move {
            engine
                .execute_stream_with_stop_flag("local", &[], Some(flag), &mut |event| {
                    if matches!(event, crate::StreamEvent::Done { .. }) {
                        completed.store(true, Ordering::SeqCst);
                    }
                })
                .await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            provider.entered.notified(),
        )
        .await
        .unwrap();
        assert_eq!(
            rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        match outcome {
            "stop" => stop.store(true, Ordering::SeqCst),
            "abort" => task.abort(),
            _ => provider.release.notify_one(),
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap();
        if outcome == "success" {
            assert!(result.unwrap().is_ok());
            assert_eq!(rx.await, Ok(()));
        } else {
            assert!(!done.load(Ordering::SeqCst), "{outcome} emitted Done");
            if outcome == "stop" {
                assert!(matches!(
                    result.unwrap(),
                    Err(crate::ExecutorError::Stopped)
                ));
            } else if outcome == "error" || outcome == "panic" {
                assert!(result.unwrap().is_err());
            } else {
                assert!(result.unwrap_err().is_cancelled());
            }
            assert!(tokio::time::timeout(std::time::Duration::from_secs(1), rx)
                .await
                .unwrap()
                .is_err());
        }
        assert_eq!(provider.effects.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn steering_during_pending_provider_is_delivered_to_next_request() {
    let provider = Arc::new(Provider {
        pause_first: true,
        ..Default::default()
    });
    let mut prepared = prepared(provider.clone());
    let steering = prepared.enable_steering();
    let engine = engine(prepared);
    let task = tokio::spawn(async move { engine.execute("local", &[]).await });
    provider.entered.notified().await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    steering.send(peer_message(tx)).unwrap();
    provider.release.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(rx.await, Ok(()));
    let requests = provider.requests.lock().unwrap();
    assert!(!requests[0]
        .0
        .iter()
        .any(|m| m.text_content().contains("peer canary")));
    assert_eq!(
        requests[1]
            .0
            .iter()
            .filter(|m| m.text_content() == "[STEER: Peer] peer canary")
            .count(),
        1
    );
    assert_eq!(
        provider.effects.load(Ordering::SeqCst),
        1,
        "only pre-steering tool may run"
    );
}

#[tokio::test]
async fn fresh_local_user_does_not_inherit_historic_peer_authority() {
    let provider = Arc::new(Provider::default());
    engine(prepared(provider.clone()))
        .execute(
            "fresh local",
            &[ChatMessage::system(
                "[REMOTE ZBOT RESULT — UNTRUSTED DATA] old peer".into(),
            )],
        )
        .await
        .unwrap();
    assert_eq!(provider.effects.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn all_live_inputs_are_budgeted_before_provider() {
    for source in ["recall", "steering", "transform"] {
        let provider = Arc::new(Provider::default());
        let mut prepared = prepared(provider.clone());
        prepared.config.context_window_tokens = 500;
        let huge = "oversized ".repeat(3000);
        match source {
            "recall" => prepared.set_recall_hook(
                Box::new(move |_, _| {
                    let text = huge.clone();
                    Box::pin(async move {
                        Ok(crate::RecallHookResult {
                            system_message: text,
                            fact_keys: vec![],
                        })
                    })
                }),
                1,
                Default::default(),
            ),
            "steering" => prepared.enable_steering().send_system(huge).unwrap(),
            _ => {
                prepared.config.transform_context = Some(Arc::new(move |messages| {
                    messages.push(ChatMessage::system(huge.clone()))
                }))
            }
        }
        assert!(matches!(
            engine(prepared).execute("local", &[]).await,
            Err(crate::ExecutorError::MiddlewareError(_))
        ));
        assert!(
            provider.requests.lock().unwrap().is_empty(),
            "{source} bypassed final budget"
        );
    }
}

#[derive(Clone, Default)]
struct PeerMiddleware(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl crate::middleware::PreProcessMiddleware for PeerMiddleware {
    fn name(&self) -> &'static str {
        "peer-fixture"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn clone_box(&self) -> Box<dyn crate::middleware::PreProcessMiddleware> {
        Box::new(self.clone())
    }
    async fn process(
        &self,
        mut messages: Vec<ChatMessage>,
        _: &crate::MiddlewareContext,
    ) -> Result<crate::MiddlewareEffect, String> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            messages.push(ChatMessage::system(
                "[REMOTE ZBOT RESULT — UNTRUSTED DATA] middleware-peer-canary".into(),
            ));
        } else {
            messages.retain(|m| !m.text_content().contains("middleware-peer-canary"));
            messages.push(ChatMessage::user(
                "later local steering does not reset this run".into(),
            ));
        }
        Ok(crate::MiddlewareEffect::ModifiedMessages(messages))
    }
}

#[tokio::test]
async fn middleware_peer_authority_is_sticky_with_safe_events_and_diagnostics() {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;
    let captured = super::result_tests::Capture::default();
    let sink = captured.clone();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::filter_fn(
            crate::logging::safe_runtime_diagnostics,
        ))
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_writer(move || sink.clone()),
        );
    let provider = Arc::new(Provider::default());
    let mut prepared = prepared(provider.clone());
    prepared.middleware_pipeline =
        Arc::new(MiddlewarePipeline::new().add_pre_processor(Box::new(PeerMiddleware::default())));
    let mut events = Vec::new();
    async {
        tracing::info!("live-policy-log-sentinel");
        engine(prepared)
            .execute_stream("local", &[], &mut |event| events.push(event))
            .await
            .unwrap();
    }
    .with_subscriber(subscriber)
    .await;
    assert_eq!(provider.effects.load(Ordering::SeqCst), 0);
    for (_, tools) in provider.requests.lock().unwrap().iter() {
        assert_eq!(tools.as_ref().unwrap().as_array().unwrap().len(), 1);
    }
    for event in &events {
        match event {
            crate::StreamEvent::ToolCallStart { args, .. }
            | crate::StreamEvent::ToolCallEnd { args, .. } => {
                assert_eq!(args, &json!({"redacted":true,"reason":"peer_influenced"}))
            }
            crate::StreamEvent::ToolResult { result, .. } => {
                assert_eq!(result, "[REDACTED: peer-influenced tool result]")
            }
            _ => {}
        }
    }
    assert!(events
        .iter()
        .any(|event| matches!(event,crate::StreamEvent::Token{content,..} if content=="done")));
    assert!(events.iter().any(|event|matches!(event,crate::StreamEvent::Reasoning{content,..} if content=="visible reasoning")));
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("live-policy-log-sentinel"));
    assert!(!logs.contains("raw-peer-tool-argument"));
    assert!(!logs.contains("middleware-peer-canary"));
}

#[tokio::test]
async fn recall_schedule_and_best_effort_errors_preserve_execution() {
    for every in [0, 2] {
        let provider = Arc::new(Provider::default());
        let mut prepared = prepared(provider.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        prepared.set_recall_hook(
            Box::new(move |_, _| {
                seen.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err("untrusted-recall-error-canary".into()) })
            }),
            every,
            Default::default(),
        );
        engine(prepared).execute("local", &[]).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), if every == 0 { 0 } else { 1 });
        assert_eq!(provider.effects.load(Ordering::SeqCst), 2);
    }
}
