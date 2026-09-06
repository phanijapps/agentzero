//! Shared external-event visibility for peer-influenced tool execution.
use crate::types::ChatMessage;
use agent_primitives::types::Part;
use serde_json::{json, Value};

/// Tool-call events, traces, and conversation rows are externally observable.
/// Peer text belongs only in the durable work payload and the recipient's
/// bounded steering envelope, so expose identifiers while replacing content.
pub(crate) fn externally_visible_tool_args(
    tool_name: &str,
    args: &Value,
    peer_content_in_context: bool,
) -> Value {
    if peer_content_in_context {
        return json!({"redacted": true, "reason": "peer_influenced"});
    }
    fn canonical_id(args: &Value, field: &str, prefix: &str) -> Value {
        let Some(value) = args.get(field).and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(raw) = value.strip_prefix(prefix) else {
            return Value::String("[REDACTED]".to_owned());
        };
        let Ok(uuid) = uuid::Uuid::parse_str(raw) else {
            return Value::String("[REDACTED]".to_owned());
        };
        if uuid.hyphenated().to_string() != raw {
            return Value::String("[REDACTED]".to_owned());
        }
        Value::String(value.to_owned())
    }

    match tool_name {
        "message_agent" => json!({
            "execution_id": canonical_id(args, "execution_id", "exec-"),
            "message": "[REDACTED]"
        }),
        "reply_to_agent" => json!({
            "reply_to": canonical_id(args, "reply_to", "work-"),
            "message": "[REDACTED]"
        }),
        _ => args.clone(),
    }
}

pub(crate) fn externally_visible_tool_result(
    peer_content_in_context: bool,
    result: String,
    context_result: Option<String>,
    error: Option<String>,
) -> (String, Option<String>, Option<String>) {
    if peer_content_in_context {
        (
            "[REDACTED: peer-influenced tool result]".to_owned(),
            context_result.map(|_| "[REDACTED: peer-influenced tool context]".to_owned()),
            error.map(|_| "peer_influenced_tool_error".to_owned()),
        )
    } else {
        (result, context_result, error)
    }
}

pub(crate) fn contains_persisted_peer_result(messages: &[ChatMessage]) -> bool {
    let last_local_user = messages.iter().rposition(|message| message.role == "user");
    let last_peer_result = messages.iter().rposition(|message| {
        message.role == "system"
            && message.content.iter().any(|part| {
                matches!(
                    part,
                    Part::Text { text }
                        if text.starts_with("[REMOTE ZBOT RESULT — UNTRUSTED DATA]")
                )
            })
    });
    last_peer_result.is_some_and(|peer| last_local_user.is_none_or(|user| peer > user))
}
