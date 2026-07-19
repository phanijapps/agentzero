//! Migration parity helpers.
//!
//! This module is intentionally doc-hidden at the crate root. It exists so
//! migration fixtures can exercise the live event conversion functions without
//! turning those functions into general gateway API.

use std::collections::BTreeSet;

use agent_runtime::StreamEvent;
use gateway_events::GatewayEvent;
use gateway_ws_protocol::ServerMessage;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::websocket::handler::gateway_event_to_server_message;

const AGENT_ID: &str = "agent-parity";
const SESSION_ID: &str = "session-parity";
const EXECUTION_ID: &str = "execution-parity";
const CONVERSATION_ID: &str = "conversation-parity";

/// Capture sanitized old-engine event-path signatures.
///
/// The scenarios are deterministic and redacted. They cover the current
/// `StreamEvent -> GatewayEvent -> ServerMessage` path plus gateway-owned and
/// direct websocket messages that are not emitted by `StreamEvent`.
pub fn old_engine_event_signature() -> Value {
    json!({
        "artifact_schema": 1,
        "source": {
            "label": "old-engine-event-path",
            "capture_method": "gateway_execution::convert_stream_event + gateway websocket conversion",
            "input": "deterministic sanitized scenarios"
        },
        "stream_scenarios": stream_scenarios(),
        "gateway_scenarios": gateway_scenarios(),
        "direct_server_messages": direct_server_messages(),
    })
}

fn stream_scenarios() -> Vec<Value> {
    vec![
        stream_scenario(
            "simple_stream",
            vec![
                StreamEvent::Metadata {
                    timestamp: 1,
                    agent_id: AGENT_ID.to_string(),
                    model: "model".to_string(),
                    provider: "provider".to_string(),
                },
                StreamEvent::Token {
                    timestamp: 2,
                    content: "redacted-token".to_string(),
                },
                StreamEvent::Done {
                    timestamp: 3,
                    final_message: "redacted-final".to_string(),
                    token_count: 2,
                },
            ],
        ),
        stream_scenario(
            "thinking_tool_result",
            vec![
                StreamEvent::Reasoning {
                    timestamp: 4,
                    content: "redacted-thinking".to_string(),
                },
                StreamEvent::ToolCallStart {
                    timestamp: 5,
                    tool_id: "tool-call-parity".to_string(),
                    tool_name: "parity_tool".to_string(),
                    args: json!({"path": "redacted", "limit": 1}),
                },
                StreamEvent::ToolResult {
                    timestamp: 6,
                    tool_id: "tool-call-parity".to_string(),
                    result: "redacted-result".to_string(),
                    context_result: Some("redacted-context-result".to_string()),
                    error: None,
                    duration_ms: Some(1),
                },
                StreamEvent::ToolCallEnd {
                    timestamp: 7,
                    tool_id: "tool-call-parity".to_string(),
                    tool_name: "parity_tool".to_string(),
                    args: json!({"path": "redacted", "limit": 1}),
                },
            ],
        ),
        stream_scenario(
            "action_events",
            vec![
                StreamEvent::ActionRespond {
                    timestamp: 8,
                    message: "redacted-response".to_string(),
                    format: "text".to_string(),
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                    session_id: Some(SESSION_ID.to_string()),
                    artifacts: Vec::new(),
                },
                StreamEvent::ActionPlanUpdate {
                    timestamp: 9,
                    plan: json!([{"step": "redacted", "status": "pending"}]),
                    explanation: Some("redacted-explanation".to_string()),
                },
                StreamEvent::ActionDelegate {
                    timestamp: 10,
                    agent_id: "child-agent".to_string(),
                    task: "redacted-task".to_string(),
                    context: Some(json!({"shape": "redacted"})),
                    wait_for_result: true,
                    max_iterations: Some(1),
                    output_schema: None,
                    skills: vec!["skill".to_string()],
                    capability_assignment: None,
                    planning_capability_catalog: None,
                    complexity: Some("low".to_string()),
                    mode: Some("DirectArtifact".to_string()),
                    parallel: false,
                    child_execution_id: Some("child-execution".to_string()),
                },
            ],
        ),
        stream_scenario(
            "state_and_control",
            vec![
                StreamEvent::Heartbeat { timestamp: 11 },
                StreamEvent::ContextState {
                    timestamp: 12,
                    state: json!({"loaded_skills": 1}),
                },
                StreamEvent::WardChanged {
                    timestamp: 13,
                    ward_id: "ward-parity".to_string(),
                },
                StreamEvent::IterationsExtended {
                    timestamp: 14,
                    iterations_used: 3,
                    iterations_added: 2,
                    reason: "redacted-reason".to_string(),
                },
                StreamEvent::SessionTitleChanged {
                    timestamp: 15,
                    title: "redacted-title".to_string(),
                },
                StreamEvent::Error {
                    timestamp: 16,
                    error: "redacted-error".to_string(),
                    recoverable: false,
                },
            ],
        ),
    ]
}

fn gateway_scenarios() -> Vec<Value> {
    vec![
        gateway_scenario(
            "lifecycle",
            vec![
                GatewayEvent::AgentCompleted {
                    agent_id: AGENT_ID.to_string(),
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    result: Some("redacted-result".to_string()),
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                },
                GatewayEvent::AgentStopped {
                    agent_id: AGENT_ID.to_string(),
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    iteration: 2,
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                },
                GatewayEvent::IterationUpdate {
                    agent_id: AGENT_ID.to_string(),
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    current: 1,
                    max: 3,
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                },
                GatewayEvent::ContinuationPrompt {
                    agent_id: AGENT_ID.to_string(),
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    iteration: 3,
                    message: "redacted-continuation".to_string(),
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                },
            ],
        ),
        gateway_scenario(
            "delegation",
            vec![
                GatewayEvent::DelegationStarted {
                    session_id: SESSION_ID.to_string(),
                    parent_execution_id: EXECUTION_ID.to_string(),
                    child_execution_id: "child-execution".to_string(),
                    parent_agent_id: AGENT_ID.to_string(),
                    child_agent_id: "child-agent".to_string(),
                    task: "redacted-task".to_string(),
                    parent_conversation_id: Some(CONVERSATION_ID.to_string()),
                    child_conversation_id: Some("child-conversation".to_string()),
                },
                GatewayEvent::DelegationCompleted {
                    session_id: SESSION_ID.to_string(),
                    parent_execution_id: EXECUTION_ID.to_string(),
                    child_execution_id: "child-execution".to_string(),
                    parent_agent_id: AGENT_ID.to_string(),
                    child_agent_id: "child-agent".to_string(),
                    result: Some("redacted-child-result".to_string()),
                    parent_conversation_id: Some(CONVERSATION_ID.to_string()),
                    child_conversation_id: Some("child-conversation".to_string()),
                },
                GatewayEvent::SessionContinuationReady {
                    session_id: SESSION_ID.to_string(),
                    root_agent_id: AGENT_ID.to_string(),
                    root_execution_id: EXECUTION_ID.to_string(),
                },
            ],
        ),
        gateway_scenario(
            "gateway_owned_updates",
            vec![
                GatewayEvent::MessageAdded {
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    role: "assistant".to_string(),
                    content: "redacted-message".to_string(),
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                },
                GatewayEvent::TokenUsage {
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    tokens_in: 10,
                    tokens_out: 5,
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                },
                GatewayEvent::IntentAnalysisStarted {
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                },
                GatewayEvent::IntentAnalysisComplete {
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                    primary_intent: "redacted-intent".to_string(),
                    hidden_intents: vec!["redacted-hidden-intent".to_string()],
                    recommended_skills: vec!["redacted-skill".to_string()],
                    recommended_agents: vec!["redacted-agent".to_string()],
                    ward_recommendation: json!({"action": "none"}),
                    execution_strategy: json!({"approach": "direct"}),
                },
                GatewayEvent::IntentAnalysisSkipped {
                    session_id: SESSION_ID.to_string(),
                    execution_id: EXECUTION_ID.to_string(),
                },
                GatewayEvent::RecallTrace {
                    agent_id: AGENT_ID.to_string(),
                    conversation_id: Some(CONVERSATION_ID.to_string()),
                    seed_entity_ids: vec!["entity".to_string()],
                    seed_aggregate_ids: vec!["aggregate".to_string()],
                    lca_aggregate_id: Some("aggregate".to_string()),
                    surfaced_item_count: 1,
                    match_sources: vec!["memory_facts".to_string()],
                    ranking_reasons: vec!["reciprocal_rank_fusion".to_string()],
                    degraded_reasons: Vec::new(),
                    embedding_provider_identity: Some(json!({
                        "providerType": "fastembed",
                        "model": "test",
                        "dimensions": 3,
                        "promptProfile": "query",
                        "normalization": null
                    })),
                    taxonomy_expansion: Vec::new(),
                },
            ],
        ),
    ]
}

fn direct_server_messages() -> Vec<Value> {
    [
        ServerMessage::SessionCancelled {
            session_id: SESSION_ID.to_string(),
        },
        ServerMessage::SessionPaused {
            session_id: SESSION_ID.to_string(),
        },
        ServerMessage::SessionResumed {
            session_id: SESSION_ID.to_string(),
        },
        ServerMessage::SessionEnded {
            session_id: SESSION_ID.to_string(),
        },
    ]
    .into_iter()
    .map(|message| {
        json!({
            "variant": variant_name(&message),
            "shape": json_shape(&serde_json::to_value(message).expect("server message serializes")),
        })
    })
    .collect()
}

fn stream_scenario(name: &str, events: Vec<StreamEvent>) -> Value {
    let records: Vec<Value> = events
        .into_iter()
        .map(|event| {
            let stream_variant = variant_name(&event);
            let gateway_event = gateway_execution::convert_stream_event(
                event,
                AGENT_ID,
                CONVERSATION_ID,
                SESSION_ID,
                EXECUTION_ID,
            );
            let gateway_variant = gateway_event.as_ref().map(variant_name);
            let server_message = gateway_event.and_then(gateway_event_to_server_message);
            json!({
                "stream_event": stream_variant,
                "gateway_event": gateway_variant,
                "server_message": server_message_signature(server_message),
            })
        })
        .collect();

    json!({
        "name": name,
        "records": records,
        "sequence_hash": sequence_hash(&records),
    })
}

fn gateway_scenario(name: &str, events: Vec<GatewayEvent>) -> Value {
    let records: Vec<Value> = events
        .into_iter()
        .map(|event| {
            let gateway_variant = variant_name(&event);
            let server_message = gateway_event_to_server_message(event);
            json!({
                "gateway_event": gateway_variant,
                "server_message": server_message_signature(server_message),
            })
        })
        .collect();

    json!({
        "name": name,
        "records": records,
        "sequence_hash": sequence_hash(&records),
    })
}

fn server_message_signature(message: Option<ServerMessage>) -> Value {
    match message {
        Some(message) => json!({
            "variant": variant_name(&message),
            "shape": json_shape(&serde_json::to_value(message).expect("server message serializes")),
        }),
        None => Value::Null,
    }
}

fn variant_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn json_shape(value: &Value) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(items) => {
            let item_shapes = items
                .iter()
                .map(|item| serde_json::to_string(&json_shape(item)).expect("shape serializes"))
                .collect::<BTreeSet<_>>();
            json!({
                "array_len": bucket_number(items.len()),
                "item_shapes": item_shapes,
            })
        }
        Value::Object(map) => {
            let mut fields = Map::new();
            for (key, inner) in map {
                fields.insert(key.clone(), json_shape(inner));
            }
            json!({ "object": fields })
        }
    }
}

fn bucket_number(value: usize) -> &'static str {
    match value {
        0 => "0",
        1..=4 => "1-4",
        5..=16 => "5-16",
        17..=64 => "17-64",
        _ => "65+",
    }
}

fn sequence_hash(records: &[Value]) -> String {
    let rendered = serde_json::to_vec(records).expect("records serialize");
    let digest = Sha256::digest(rendered);
    format!("{digest:x}")[..24].to_string()
}
