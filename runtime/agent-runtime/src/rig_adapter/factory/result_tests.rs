//! Real provider→Rig result contracts: events retain raw data, model gets context.
use super::*;
use crate::{
    llm::{ChatMessage, ChatResponse, LlmError, StreamCallback},
    rig_adapter::RigModelConfig,
    AgentEngine, ExecutorConfig, LlmClient, LlmConfig, McpManager, MiddlewarePipeline, StreamEvent,
    ToolRegistry,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
struct Script {
    requests: Mutex<Vec<Vec<ChatMessage>>>,
    requested_name: Option<&'static str>,
}
#[async_trait::async_trait]
impl LlmClient for Script {
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
        _: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(messages);
        let call = match requests.len() {
            1 => crate::ToolCall::new(
                "effect-call".into(),
                self.requested_name.unwrap_or("effect").into(),
                if self.requested_name == Some("message_agent") {
                    json!({"message":"peer-message-canary","execution_id":"exec-00000000-0000-0000-0000-000000000001"})
                } else {
                    json!({"input":"raw-argument"})
                },
            ),
            2 => crate::ToolCall::new(
                "respond-call".into(),
                "respond".into(),
                json!({"message":"recovered answer"}),
            ),
            _ => panic!("respond should terminate without another request"),
        };
        let mut tool_calls = vec![call];
        if self.requested_name == Some("unknown_tool") && requests.len() == 1 {
            tool_calls.push(crate::ToolCall::new(
                "discarded-sibling".into(),
                "effect".into(),
                json!({}),
            ));
        }
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: Some(tool_calls),
            reasoning: None,
            usage: Some(crate::llm::TokenUsage {
                prompt_tokens: 7,
                completion_tokens: 3,
                total_tokens: 10,
                cached_prompt_tokens: None,
            }),
        })
    }
}
struct Effect {
    output: Result<String, String>,
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl agent_primitives::Tool for Effect {
    fn name(&self) -> &str {
        "effect"
    }
    fn description(&self) -> &str {
        "fixture effect"
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({"type":"object","properties":{"input":{"type":"string"}}}))
    }
    async fn execute(
        &self,
        _: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        self.output
            .clone()
            .map(Value::String)
            .map_err(agent_primitives::error::AgentError::Tool)
    }
}

async fn run(
    config: ExecutorConfig,
    output: Result<String, String>,
    history: &[ChatMessage],
) -> (Vec<StreamEvent>, Arc<Script>, usize) {
    run_requested(config, output, history, None).await
}

async fn run_requested(
    config: ExecutorConfig,
    output: Result<String, String>,
    history: &[ChatMessage],
    requested_name: Option<&'static str>,
) -> (Vec<StreamEvent>, Arc<Script>, usize) {
    let provider = Arc::new(Script {
        requested_name,
        ..Script::default()
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Effect {
        output,
        calls: calls.clone(),
    }));
    registry.register(Arc::new(crate::RespondTool::new()));
    let prepared = PreparedExecution::new(
        config,
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
    engine
        .execute_stream("execute", history, &mut |event| events.push(event))
        .await
        .unwrap();
    (events, provider, calls.load(Ordering::SeqCst))
}

// ---- test hooks ----
struct RewriteEffectHook {
    seen: std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>>,
}
#[async_trait::async_trait]
impl crate::EngineHook for RewriteEffectHook {
    async fn after_tool(
        &self,
        name: &str,
        args: &Value,
        output: &str,
        succeeded: bool,
    ) -> Option<String> {
        if name != "effect" {
            return None;
        }
        assert_eq!(args["input"], "raw-argument");
        self.seen
            .lock()
            .unwrap()
            .push((output.to_owned(), succeeded));
        Some("rewritten-context".into())
    }
}

struct SafeContextEffectHook;
#[async_trait::async_trait]
impl crate::EngineHook for SafeContextEffectHook {
    async fn after_tool(
        &self,
        name: &str,
        _args: &Value,
        _output: &str,
        _ok: bool,
    ) -> Option<String> {
        (name == "effect").then(|| "safe context".into())
    }
}

struct DenyEffectHook;
#[async_trait::async_trait]
impl crate::EngineHook for DenyEffectHook {
    async fn before_tool(&self, name: &str, _args: &Value) -> crate::ToolDecision {
        if name == "effect" {
            crate::ToolDecision::Block {
                reason: "denied \"quoted\" reason".into(),
            }
        } else {
            crate::ToolDecision::Allow
        }
    }
}

struct CountEffectAfterHook {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait::async_trait]
impl crate::EngineHook for CountEffectAfterHook {
    async fn after_tool(
        &self,
        name: &str,
        _args: &Value,
        _output: &str,
        _ok: bool,
    ) -> Option<String> {
        if name == "effect" {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        None
    }
}

struct RecoverFailureHook;
#[async_trait::async_trait]
impl crate::EngineHook for RecoverFailureHook {
    async fn after_tool(
        &self,
        name: &str,
        _args: &Value,
        output: &str,
        succeeded: bool,
    ) -> Option<String> {
        if name != "effect" {
            return None;
        }
        assert!(!succeeded);
        assert!(serde_json::from_str::<Value>(output).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("raw-failure"));
        Some("safe recovery instructions".into())
    }
}

#[tokio::test]
async fn tool_error_is_recoverable_and_carries_error_duration_and_context() {
    let cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    let (events, provider, calls) = run(cfg, Err("recoverable-failure".into()), &[]).await;
    assert_eq!(calls, 1);
    let result_index = events.iter().position(|event| matches!(event, StreamEvent::ToolResult { tool_id, .. } if tool_id == "effect-call")).unwrap();
    assert!(
        matches!(&events[result_index], StreamEvent::ToolResult { result, context_result: Some(context), error: Some(error), duration_ms: Some(duration), .. } if result.is_empty() && context.contains("recoverable-failure") && error.contains("recoverable-failure") && *duration >= 2)
    );
    assert!(
        matches!(&events[result_index+1], StreamEvent::ToolCallEnd { tool_id, .. } if tool_id == "effect-call")
    );
    assert!(events.iter().any(|event| matches!(event, StreamEvent::ActionRespond { message, .. } if message == "recovered answer")));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::ContextState { .. })
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::TokenUpdate {
            tokens_in: 14,
            tokens_out: 6,
            ..
        }
    )));
    let requests = provider.requests.lock().unwrap();
    let context = requests[1]
        .iter()
        .find(|message| message.role == "tool")
        .unwrap()
        .text_content();
    assert!(serde_json::from_str::<Value>(&context).unwrap()["error"]
        .as_str()
        .unwrap()
        .contains("recoverable-failure"));
}

#[tokio::test]
async fn successful_raw_output_is_distinct_from_processed_after_hook_context() {
    let raw = "raw-output-".repeat(100);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hook_seen = seen.clone();
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.max_tool_result_chars = 100;
    cfg.hooks
        .add(Arc::new(RewriteEffectHook { seen: hook_seen }));
    let (events, provider, calls) = run(cfg, Ok(raw.clone()), &[]).await;
    assert_eq!(calls, 1);
    assert!(
        matches!(&seen.lock().unwrap()[..], [(output, true)] if output.len() < raw.len() && output.contains("TRUNCATED"))
    );
    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolResult { tool_id, result, context_result: Some(context), error: None, duration_ms: Some(duration), .. } if tool_id == "effect-call" && result == &raw && context == "rewritten-context" && *duration >= 2)));
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests[1]
            .iter()
            .find(|m| m.role == "tool")
            .unwrap()
            .text_content(),
        "rewritten-context"
    );
}

#[tokio::test]
async fn blocked_hook_has_zero_effects_zero_duration_and_valid_model_json() {
    let after_calls = Arc::new(AtomicUsize::new(0));
    let after = after_calls.clone();
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.hooks.add(Arc::new(DenyEffectHook));
    cfg.hooks
        .add(Arc::new(CountEffectAfterHook { calls: after }));
    let (events, provider, calls) = run(cfg, Ok("should-not-run".into()), &[]).await;
    assert_eq!(calls, 0);
    assert_eq!(after_calls.load(Ordering::SeqCst), 0);
    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolResult { result, context_result: Some(context), error: Some(error), duration_ms: Some(0), .. } if result == "[blocked by hook]" && error == "blocked_by_hook" && serde_json::from_str::<Value>(context).unwrap()["blocked"] == true)));
    let requests = provider.requests.lock().unwrap();
    let context = requests[1]
        .iter()
        .find(|m| m.role == "tool")
        .unwrap()
        .text_content();
    assert_eq!(
        serde_json::from_str::<Value>(&context).unwrap()["reason"],
        "denied \"quoted\" reason"
    );
}

#[tokio::test]
async fn failed_tool_after_hook_receives_false_and_rewrites_error_context_only() {
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.hooks.add(Arc::new(RecoverFailureHook));
    let (events, provider, _) = run(cfg, Err("raw-failure".into()), &[]).await;
    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolResult { result, context_result: Some(context), error: Some(error), .. } if result.is_empty() && context == "safe recovery instructions" && error.contains("raw-failure"))));
    assert_eq!(
        provider.requests.lock().unwrap()[1]
            .iter()
            .find(|m| m.role == "tool")
            .unwrap()
            .text_content(),
        "safe recovery instructions"
    );
}

#[tokio::test]
async fn large_successful_result_is_offloaded_before_after_hook_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let raw = "large-raw-output-".repeat(100);
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.offload_large_results = true;
    cfg.offload_threshold_chars = 100;
    cfg.offload_dir = Some(dir.path().to_path_buf());
    let (events, provider, _) = run(cfg, Ok(raw.clone()), &[]).await;
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read_to_string(files[0].path()).unwrap(), raw);
    let requests = provider.requests.lock().unwrap();
    let context = requests[1]
        .iter()
        .find(|m| m.role == "tool")
        .unwrap()
        .text_content();
    assert!(context.contains(files[0].path().to_str().unwrap()));
    assert!(!context.contains(&raw));
    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolResult { result, context_result: Some(model_context), .. } if result == &raw && model_context == &context)));
}

#[derive(Clone, Default)]
pub(super) struct Capture(pub(super) Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn actual_rig_diagnostics_do_not_bypass_runtime_payload_filter() {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;
    let captured = Capture::default();
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
    let mut cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    cfg.hooks.add(Arc::new(SafeContextEffectHook));
    async {
        tracing::info!("application-log-sentinel");
        run(cfg, Ok("peer-result-canary".into()), &[]).await;
    }
    .with_subscriber(subscriber)
    .await;
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("application-log-sentinel"));
    assert!(!logs.contains("raw-argument"), "tool args escaped: {logs}");
    assert!(
        !logs.contains("peer-result-canary"),
        "raw result escaped: {logs}"
    );
}

#[tokio::test]
async fn host_peer_outcome_plumbing_blocks_real_rig_effects_before_dispatch() {
    use super::super::{
        tool_hook::RigExecutionHook,
        tool_results::{SharedToolResults, ToolResults},
        SharedToolContext,
    };
    use futures::StreamExt;
    use rig::{
        agent::{AgentBuilder, MultiTurnStreamItem},
        streaming::StreamingChat,
        tool::ToolCallExtensions,
    };
    // Plumbing only: T6's post-policy model boundary supplies this host-owned
    // state. A fresh local user must not inherit historical peer taint.
    let outcomes = ToolResults::default();
    outcomes.mark_peer_influenced();
    let outcomes = Arc::new(outcomes);
    let context = Arc::new(crate::tools::ToolContext::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(Script::default());
    let agent = AgentBuilder::new(LlmCompletionModel::new(provider.clone(), "fixture"))
        .tools(vec![
            RigToolAdapter::boxed(Arc::new(Effect {
                output: Ok("peer-result-canary".into()),
                calls: calls.clone(),
            })),
            RigToolAdapter::boxed(Arc::new(crate::RespondTool::new())),
        ])
        .add_hook(RigExecutionHook {
            ctx: context.clone(),
            hooks: std::sync::Arc::new(crate::HookSet::new()),
            results: outcomes.clone(),
            context_config: Default::default(),
        })
        .build();
    let mut extensions = ToolCallExtensions::new();
    extensions.insert::<SharedToolContext>(context);
    extensions.insert::<SharedToolResults>(outcomes.clone());
    let mut stream = agent
        .stream_chat("execute", Vec::<rig::completion::Message>::new())
        .tool_extensions(extensions)
        .multi_turn(3)
        .tool_concurrency(1)
        .await;
    let mut result_count = 0;
    while let Some(item) = stream.next().await {
        if matches!(item.unwrap(), MultiTurnStreamItem::StreamUserItem(_)) {
            result_count += 1;
            let outcome = outcomes.take();
            if result_count == 1 {
                assert_eq!(outcome.error.as_deref(), Some("blocked_by_hook"));
                assert_eq!(outcome.duration_ms, 0);
                assert!(outcome
                    .context
                    .as_ref()
                    .unwrap()
                    .contains("peer_data_authority_boundary"));
            } else {
                break;
            }
        }
    }
    assert_eq!(result_count, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let args = crate::tool_visibility::externally_visible_tool_args(
        "effect",
        &json!({"secret":"peer-args-canary"}),
        true,
    );
    assert_eq!(args, json!({"redacted":true,"reason":"peer_influenced"}));
    let (raw, context, error) = crate::tool_visibility::externally_visible_tool_result(
        true,
        "peer-result-canary".into(),
        Some("peer-context-canary".into()),
        Some("peer-error-canary".into()),
    );
    assert!(!raw.contains("canary"));
    assert!(!context.unwrap().contains("canary"));
    assert_eq!(error.as_deref(), Some("peer_influenced_tool_error"));
}

#[tokio::test]
async fn unregistered_tool_has_typed_error_zero_effects_and_model_recovery() {
    let cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    let (events, provider, calls) =
        run_requested(cfg, Ok("must not run".into()), &[], Some("unknown_tool")).await;
    assert_eq!(calls, 0);
    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolResult { tool_id, result, context_result: Some(context), error: Some(error), duration_ms: Some(0), .. } if tool_id == "effect-call" && result.is_empty() && context.contains("not found") && error.contains("not found"))));
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let history = &requests[1];
    let tool = history.iter().find(|m| m.role == "tool").unwrap();
    assert_eq!(tool.tool_call_id.as_deref(), Some("effect-call"));
    assert!(
        serde_json::from_str::<Value>(&tool.text_content()).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
    assert!(history
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .any(|call| call.id == "effect-call"));
    assert!(!serde_json::to_string(history)
        .unwrap()
        .contains("discarded-sibling"));
    assert!(!events.iter().any(
        |e| matches!(e, StreamEvent::ToolResult { tool_id, .. } if tool_id == "discarded-sibling")
    ));
}

#[tokio::test]
async fn peer_message_arguments_are_redacted_in_real_rig_attempt_events() {
    let cfg = ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into());
    let (events, _, calls) =
        run_requested(cfg, Ok("must not run".into()), &[], Some("message_agent")).await;
    assert_eq!(calls, 0);
    let args: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallStart {
                tool_name, args, ..
            }
            | StreamEvent::ToolCallEnd {
                tool_name, args, ..
            } if tool_name == "message_agent" => Some(args),
            _ => None,
        })
        .collect();
    assert_eq!(args.len(), 2);
    for args in args {
        assert_eq!(args["message"], "[REDACTED]");
        assert_eq!(
            args["execution_id"],
            "exec-00000000-0000-0000-0000-000000000001"
        );
        assert!(!args.to_string().contains("peer-message-canary"));
    }
}
