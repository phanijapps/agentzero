//! T11 parity harness — proves the Rig-backed execution path produces the
//! gateway event sequence the old-engine baseline expects, with **no live
//! default flip**.
//!
//! Each scenario drives `RigAgentEngine` (through the real `LlmCompletionModel`
//! bridge over a scripted stub `LlmClient`), captures its runtime `StreamEvent`s,
//! runs them through the gateway's `convert_stream_event`, and asserts the
//! resulting `GatewayEvent` sequence matches the runtime-derived subset of the
//! synthetic fixture's `expected_events`.
//!
//! Scope: the Rig-path-derived scenarios — `simple_chat`, `tool_call_result`,
//! `error`, `stop_cancel`. `delegation_continuation` is gateway-owned
//! (`DelegationStarted`/`DelegationCompleted` come from the delegation registry,
//! not the engine) and is verified in T8. Gateway-lifecycle events
//! (`AgentStarted`/`AgentCompleted`/`AgentStopped`/`SessionCancelled`) are emitted
//! by the gateway runner around the executor, not by `RigAgentEngine`, and are
//! covered by the T10 wire suites.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use agent_runtime::executor::ExecutorError;
use agent_runtime::llm::{ChatResponse, LlmClient, LlmError, StreamCallback, StreamChunk};
use agent_runtime::rig_adapter::engine::RigAgentEngine;
use agent_runtime::rig_adapter::model::LlmCompletionModel;
use agent_runtime::rig_adapter::RigToolAdapter;
use agent_runtime::tools::ToolContext;
use agent_runtime::types::{ChatMessage, StreamEvent};
use agent_runtime::AgentEngine;
use agent_runtime::{RigAgentConfig, RigModelConfig, ToolCall as AgentToolCall};
use async_trait::async_trait;
use gateway_events::GatewayEvent;
use gateway_execution::convert_stream_event;
use serde_json::{json, Value};

const AGENT_ID: &str = "agent-1";
const CONVERSATION_ID: &str = "conv-1";
const SESSION_ID: &str = "sess-1";
const EXECUTION_ID: &str = "exec-1";

fn sample_config() -> RigAgentConfig {
    RigAgentConfig::new(
        AGENT_ID,
        "Agent",
        "parity test agent",
        "You are a test agent.".to_string(),
        RigModelConfig {
            provider_id: "p".into(),
            base_url: "https://llm.local/v1".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            temperature: 0.0,
            max_tokens: 100,
            context_window_tokens: 4096,
            thinking_enabled: false,
            provider_params: None,
        },
    )
}

/// Scripted AgentZero LlmClient. First turn streams `chunks` and optionally
/// returns a tool call; later turns stream `chunks` and finalize. If `error`
/// is set, every call returns that error.
struct ScriptedLlm {
    chunks: Vec<String>,
    first_turn_tool_calls: Vec<AgentToolCall>,
    error: Option<String>,
    turn: AtomicU32,
}

#[async_trait]
impl LlmClient for ScriptedLlm {
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
        if let Some(err) = &self.error {
            return Err(LlmError::ApiError(err.clone()));
        }
        Ok(ChatResponse {
            content: self.chunks.join(""),
            tool_calls: None,
            reasoning: None,
            usage: None,
        })
    }
    async fn chat_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        if let Some(err) = &self.error {
            return Err(LlmError::ApiError(err.clone()));
        }
        for chunk in &self.chunks {
            callback(StreamChunk::Token(chunk.clone()));
        }
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        let tool_calls = if turn == 0 && !self.first_turn_tool_calls.is_empty() {
            Some(self.first_turn_tool_calls.clone())
        } else {
            None
        };
        Ok(ChatResponse {
            content: self.chunks.join(""),
            tool_calls,
            reasoning: None,
            usage: None,
        })
    }
}

fn convert_all(events: &[StreamEvent]) -> Vec<GatewayEvent> {
    events
        .iter()
        .filter_map(|event| {
            convert_stream_event(
                event.clone(),
                AGENT_ID,
                CONVERSATION_ID,
                SESSION_ID,
                EXECUTION_ID,
            )
        })
        .collect()
}

fn variant_names(events: &[GatewayEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            GatewayEvent::Token { .. } => "Token",
            GatewayEvent::ToolCall { .. } => "ToolCall",
            GatewayEvent::ToolResult { .. } => "ToolResult",
            GatewayEvent::TurnComplete { .. } => "TurnComplete",
            GatewayEvent::Error { .. } => "Error",
            _ => "Other",
        })
        .collect()
}

#[tokio::test]
async fn parity_simple_chat() {
    // Fixture expected_events (runtime-derived): Token, TurnComplete.
    // (AgentStarted/AgentCompleted are gateway-lifecycle.)
    let client: Arc<dyn LlmClient> = Arc::new(ScriptedLlm {
        chunks: vec!["hel".to_string(), "lo".to_string()],
        first_turn_tool_calls: vec![],
        error: None,
        turn: AtomicU32::new(0),
    });
    // Empty tool list: the element type is inferred from `RigAgentEngine::new`'s
    // parameter, so the Rig ToolDyn type need not be named here (Rig is
    // agent-runtime-only; gateway-execution does not depend on it).
    let engine = RigAgentEngine::new(
        sample_config(),
        LlmCompletionModel::new(client, "stub"),
        Vec::new(),
        Arc::new(ToolContext::default()),
    );

    let mut events = Vec::new();
    engine
        .execute_stream("hi", &[], &mut |event| events.push(event))
        .await
        .expect("run");

    let names = variant_names(&convert_all(&events));
    assert!(
        names.contains(&"Token"),
        "simple chat must yield Token events: {names:?}"
    );
    assert!(
        names.contains(&"TurnComplete"),
        "Done must convert to TurnComplete: {names:?}"
    );
    assert!(
        !names.contains(&"ToolCall") && !names.contains(&"Error"),
        "simple chat must not produce tool/error events: {names:?}"
    );
}

#[tokio::test]
async fn parity_tool_call_result() {
    // Fixture expected_events: ToolCall, ToolResult, TurnComplete (in order).
    let client: Arc<dyn LlmClient> = Arc::new(ScriptedLlm {
        chunks: vec!["think".to_string()],
        first_turn_tool_calls: vec![AgentToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            arguments: json!({"msg": "hi"}),
        }],
        error: None,
        turn: AtomicU32::new(0),
    });
    let engine = RigAgentEngine::new(
        sample_config(),
        LlmCompletionModel::new(client, "stub"),
        vec![RigToolAdapter::boxed(Arc::new(EchoTool))],
        Arc::new(ToolContext::default()),
    );

    let mut events = Vec::new();
    engine
        .execute_stream("use echo", &[], &mut |event| events.push(event))
        .await
        .expect("run");

    let names = variant_names(&convert_all(&events));
    let tool_call = names.iter().position(|n| *n == "ToolCall");
    let tool_result = names.iter().position(|n| *n == "ToolResult");
    let turn_complete = names.iter().position(|n| *n == "TurnComplete");
    assert!(
        tool_call.is_some() && tool_result.is_some() && turn_complete.is_some(),
        "tool path must produce ToolCall+ToolResult+TurnComplete: {names:?}"
    );
    assert!(
        tool_call < tool_result && tool_result < turn_complete,
        "order must be ToolCall -> ToolResult -> TurnComplete: {names:?}"
    );
}

#[tokio::test]
async fn parity_error() {
    // Fixture expected_events: Error. The legacy executor returns
    // ExecutorError::LlmError on LLM errors (it does not emit StreamEvent::Error
    // either); the gateway converts the Err into the Error gateway event. The
    // Rig path must match: surface LLM errors as Err.
    let client: Arc<dyn LlmClient> = Arc::new(ScriptedLlm {
        chunks: vec![],
        first_turn_tool_calls: vec![],
        error: Some("boom".to_string()),
        turn: AtomicU32::new(0),
    });
    let engine = RigAgentEngine::new(
        sample_config(),
        LlmCompletionModel::new(client, "stub"),
        Vec::new(),
        Arc::new(ToolContext::default()),
    );

    let mut events = Vec::new();
    let result = engine
        .execute_stream("hi", &[], &mut |event| events.push(event))
        .await;

    let err = result.expect_err("LLM error must surface as Err");
    assert!(
        matches!(err, ExecutorError::LlmError(_)),
        "expected LlmError for parity with the legacy executor, got {err:?}"
    );
    assert!(
        events.is_empty(),
        "no StreamEvents should be emitted before the error"
    );
}

#[tokio::test]
async fn parity_stop_cancel() {
    // Fixture expected_events: AgentStopped, SessionCancelled (gateway-lifecycle).
    // Rig-path parity: the stop flag halts streaming cleanly and the run
    // finalizes (Done -> TurnComplete) without orphaned tokens.
    let client: Arc<dyn LlmClient> = Arc::new(ScriptedLlm {
        chunks: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        first_turn_tool_calls: vec![],
        error: None,
        turn: AtomicU32::new(0),
    });
    let engine = RigAgentEngine::new(
        sample_config(),
        LlmCompletionModel::new(client, "stub"),
        Vec::new(),
        Arc::new(ToolContext::default()),
    );

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_callback = stop.clone();
    let mut events = Vec::new();
    engine
        .execute_stream_with_stop_flag("hi", &[], Some(stop), &mut |event| {
            let is_token = matches!(event, StreamEvent::Token { .. });
            events.push(event);
            if is_token {
                stop_for_callback.store(true, Ordering::Release);
            }
        })
        .await
        .expect("stopped run should finalize");

    let gateway_events = convert_all(&events);
    let token_count = gateway_events
        .iter()
        .filter(|event| matches!(event, GatewayEvent::Token { .. }))
        .count();
    assert_eq!(
        token_count, 1,
        "stop must halt after the first token: {gateway_events:?}"
    );
    assert!(
        gateway_events
            .iter()
            .any(|event| matches!(event, GatewayEvent::TurnComplete { .. })),
        "stopped run must still finalize (Done -> TurnComplete): {gateway_events:?}"
    );
}

/// Minimal AgentZero tool used by the tool_call_result parity scenario.
struct EchoTool;

#[async_trait]
impl agent_primitives::Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes its arguments back"
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn agent_primitives::ToolContext>,
        args: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        Ok(args)
    }
}
