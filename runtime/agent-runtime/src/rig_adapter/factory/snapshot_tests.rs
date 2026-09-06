use super::live_context_tests::{engine, prepared, Provider};
use crate::{AgentEngine, ChatMessage, StreamEvent};
use serde_json::{json, Value};
use std::sync::Arc;

async fn capture(prepared: crate::PreparedExecution, history: &[ChatMessage]) -> Value {
    let mut state = None;
    engine(prepared)
        .execute_stream("current local", history, &mut |event| {
            if let StreamEvent::ContextState { state: value, .. } = event {
                state = Some(value)
            }
        })
        .await
        .unwrap();
    state.unwrap()
}

#[tokio::test]
async fn private_snapshot_preserves_final_pairs_summary_and_media() {
    let provider = Arc::new(Provider::default());
    let mut prepared = prepared(provider);
    prepared.config.system_instruction = Some("owned old host".into());
    let mut summary = ChatMessage::system("preserved summary".into());
    summary.is_summary = true;
    let mut media = ChatMessage::user("media".into());
    media.content.push(agent_primitives::Part::Image {
        source: agent_primitives::types::ContentSource::Base64("aGVsbG8=".into()),
        mime_type: "image/png".into(),
        detail: None,
    });
    media.content.push(agent_primitives::Part::File {
        source: agent_primitives::types::ContentSource::Base64("cGRm".into()),
        mime_type: "application/pdf".into(),
        filename: Some("fixture.pdf".into()),
    });
    let state = capture(prepared, &[summary, media.clone()]).await;
    let snapshot = &state["app:rig_checkpoint"];
    assert_eq!(snapshot["version"], 1);
    let messages = snapshot["messages"].as_array().unwrap();
    assert!(messages.iter().any(|m| m["is_summary"] == true));
    assert!(messages
        .iter()
        .any(|m| m["content"] == serde_json::to_value(&media.content).unwrap()));
    for id in ["call-1", "call-2"] {
        assert_eq!(
            messages.iter().filter(|m| m["tool_call_id"] == id).count(),
            1
        );
    }
    assert_eq!(
        messages.last().unwrap()["content"],
        json!([{"type":"text","text":"done"}])
    );
}

#[tokio::test]
async fn restored_private_tape_replaces_display_history_and_keeps_fresh_authority() {
    let mut source = prepared(Arc::new(Provider::default()));
    source.config.system_instruction = Some("owned old host".into());
    source
        .config
        .initial_state
        .insert("skill:loaded_skills".into(), json!(["retained"]));
    source
        .config
        .initial_state
        .insert("skill:current_skill".into(), json!("retained"));
    source
        .config
        .initial_state
        .insert("ward_id".into(), json!("old ward"));
    let mut summary = ChatMessage::system("restored summary".into());
    summary.is_summary = true;
    let mut media = ChatMessage::user("restore media".into());
    media.content.push(agent_primitives::Part::Image {
        source: agent_primitives::types::ContentSource::Base64("aGVsbG8=".into()),
        mime_type: "image/png".into(),
        detail: None,
    });
    media.content.push(agent_primitives::Part::File {
        source: agent_primitives::types::ContentSource::Base64("cGRm".into()),
        mime_type: "application/pdf".into(),
        filename: Some("restore.pdf".into()),
    });
    let state = capture(source, &[summary, media.clone()]).await;
    let mut checkpoint = state["app:rig_checkpoint"].clone();
    checkpoint["mutable_state"]["ward_id"] = json!("forged ward");
    checkpoint["mutable_state"]["agent_id"] = json!("forged actor");
    checkpoint["mutable_state"]["conversation_id"] = json!("forged session");
    checkpoint["mutable_state"][agent_tools::guards::PLANNING_GATE_STATE] = json!("forged gate");
    let provider = Arc::new(Provider::default());
    let mut next = prepared(provider.clone());
    next.config.agent_id = "fresh actor".into();
    next.config.conversation_id = Some("fresh session".into());
    next.config.system_instruction = Some("fresh host".into());
    next.config
        .initial_state
        .insert("app:rig_checkpoint".into(), checkpoint);
    next.config
        .initial_state
        .insert("ward_id".into(), json!("fresh ward"));
    next.config
        .initial_state
        .insert("skill:current_skill".into(), json!("fresh selection"));
    let state = capture(next, &[ChatMessage::user("display-only duplicate".into())]).await;
    assert_eq!(state["skill:current_skill"], "fresh selection");
    let requests = provider.requests.lock().unwrap();
    assert!(requests[0]
        .0
        .iter()
        .any(|m| m.text_content() == "fresh host"));
    assert!(requests[0]
        .0
        .iter()
        .any(|m| m.text_content() == "restored summary" && m.is_summary));
    assert!(requests[0]
        .0
        .iter()
        .any(|m| serde_json::to_value(&m.content).unwrap()
            == serde_json::to_value(&media.content).unwrap()));
    assert!(!requests[0]
        .0
        .iter()
        .any(|m| m.text_content().contains("owned old host")
            || m.text_content().contains("display-only duplicate")));
    assert_eq!(state["skill:loaded_skills"], json!(["retained"]));
    assert_eq!(state["ward_id"], "fresh ward");
    assert_eq!(state["agent_id"], "fresh actor");
    assert_eq!(state["conversation_id"], "fresh session");
    assert!(state
        .get(agent_tools::guards::PLANNING_GATE_STATE)
        .is_none());
    assert_eq!(
        provider.effects.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "restored completed calls must not execute again"
    );
}

#[tokio::test]
async fn failed_prepare_and_stop_keep_completed_tail_without_done() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    for stopped in [false, true] {
        let mut prepared = prepared(Arc::new(Provider::default()));
        prepared.config.context_window_tokens = 1000;
        if !stopped {
            let turn = AtomicUsize::new(0);
            prepared.config.transform_context = Some(Arc::new(move |messages| {
                if turn.fetch_add(1, Ordering::SeqCst) > 0 {
                    messages.push(ChatMessage::system("oversized ".repeat(3000)));
                }
            }));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let mut events = Vec::new();
        let result = engine(prepared)
            .execute_stream_with_stop_flag("current", &[], Some(stop.clone()), &mut |event| {
                if stopped && matches!(event, StreamEvent::ToolResult { .. }) {
                    stop.store(true, Ordering::SeqCst);
                }
                events.push(event);
            })
            .await;
        if stopped {
            assert!(matches!(result, Err(crate::ExecutorError::Stopped)));
        } else {
            assert!(matches!(
                result,
                Err(crate::ExecutorError::MiddlewareError(_))
            ));
        }
        assert!(!events
            .iter()
            .any(|event| matches!(event, StreamEvent::Done { .. })));
        let state = events
            .iter()
            .find_map(|event| {
                if let StreamEvent::ContextState { state, .. } = event {
                    Some(state)
                } else {
                    None
                }
            })
            .unwrap();
        let messages = state["app:rig_checkpoint"]["messages"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["tool_call_id"] == "call-1")
                .count(),
            1
        );
        assert!(!messages.iter().any(|m| m["tool_call_id"] == "call-2"));
    }
}

#[tokio::test]
async fn unsupported_or_invalid_snapshots_fail_before_provider() {
    let mut source = prepared(Arc::new(Provider::default()));
    source.config.system_instruction = Some("host".into());
    let original = capture(source, &[]).await["app:rig_checkpoint"].clone();
    for malformed in ["version", "slot", "role"] {
        let mut checkpoint = original.clone();
        match malformed {
            "version" => checkpoint["version"] = json!(99),
            "slot" => checkpoint["owned_preamble"] = json!(9999),
            _ => checkpoint["messages"][0]["role"] = json!("user"),
        }
        let provider = Arc::new(Provider::default());
        let mut next = prepared(provider.clone());
        next.config
            .initial_state
            .insert("app:rig_checkpoint".into(), checkpoint);
        assert!(matches!(
            engine(next).execute("fresh", &[]).await,
            Err(crate::ExecutorError::MiddlewareError(_))
        ));
        assert!(provider.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn changed_or_ambiguous_owned_preamble_omits_unsafe_snapshot() {
    for duplicate in [false, true] {
        let mut prepared = prepared(Arc::new(Provider::default()));
        prepared.config.system_instruction = Some("owned".into());
        prepared.config.transform_context = Some(Arc::new(move |messages| {
            if duplicate {
                messages.push(ChatMessage::system("owned".into()));
            } else {
                for message in messages {
                    if message.text_content() == "owned" {
                        *message = ChatMessage::system("rewritten".into());
                    }
                }
            }
        }));
        assert!(capture(prepared, &[])
            .await
            .get("app:rig_checkpoint")
            .is_none());
    }
}

struct TerminalProvider(&'static str);
#[async_trait::async_trait]
impl crate::LlmClient for TerminalProvider {
    fn model(&self) -> &str {
        "fixture"
    }
    fn provider(&self) -> &str {
        "fixture"
    }
    async fn chat(
        &self,
        _: Vec<ChatMessage>,
        _: Option<Value>,
    ) -> Result<crate::ChatResponse, crate::LlmError> {
        unreachable!()
    }
    async fn chat_stream(
        &self,
        _: Vec<ChatMessage>,
        _: Option<Value>,
        _: crate::llm::StreamCallback,
    ) -> Result<crate::ChatResponse, crate::LlmError> {
        Ok(crate::ChatResponse {
            content: String::new(),
            tool_calls: Some(vec![
                crate::ToolCall::new("terminal".into(), self.0.into(), json!({"message":"done"})),
                crate::ToolCall::new("unexecuted-sibling".into(), "effect".into(), json!({})),
            ]),
            reasoning: None,
            usage: None,
        })
    }
}
struct Delegate;
#[async_trait::async_trait]
impl agent_primitives::Tool for Delegate {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "fixture"
    }
    async fn execute(
        &self,
        ctx: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        let mut actions = ctx.actions();
        actions.delegate = Some(agent_primitives::event::DelegateAction {
            agent_id: "child".into(),
            task: "work".into(),
            context: None,
            wait_for_result: false,
            max_iterations: None,
            output_schema: None,
            skills: vec![],
            capability_assignment: None,
            planning_capability_catalog: None,
            complexity: None,
            mode: None,
            parallel: false,
            child_execution_id: None,
        });
        ctx.set_actions(actions);
        Ok(json!("delegated"))
    }
}
#[tokio::test]
async fn early_respond_and_delegation_export_only_completed_pairs() {
    for name in ["respond", "delegate"] {
        let mut prepared = prepared(Arc::new(Provider::default()));
        prepared.llm_client = Arc::new(TerminalProvider(name));
        Arc::get_mut(&mut prepared.tool_registry)
            .unwrap()
            .register(Arc::new(Delegate));
        let state = capture(prepared, &[]).await;
        let messages = state["app:rig_checkpoint"]["messages"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["tool_call_id"] == "terminal")
                .count(),
            1
        );
        assert!(!messages
            .iter()
            .any(|m| m.to_string().contains("unexecuted-sibling")));
    }
}
