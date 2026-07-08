//! Conversion from stored conversation rows to runtime chat history.

use agent_primitives::types::Part;
use agent_runtime::{types::ToolCall, ChatMessage};
use zbot_conversation::Message;

/// Convert stored `MessageStore` rows to the runtime LLM chat format.
///
/// Assistant `tool_calls` are parsed best-effort, malformed tool-call JSON is
/// ignored, and tool rows carry `tool_call_id`.
pub fn messages_to_chat_format(messages: &[Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| ChatMessage {
            role: m.role.clone(),
            content: vec![Part::Text {
                text: m.content.clone(),
            }],
            tool_calls: if m.role == "assistant" {
                m.tool_calls.as_deref().and_then(parse_tool_calls_json)
            } else {
                None
            },
            tool_call_id: m.tool_call_id.clone(),
            is_summary: false,
        })
        .collect()
}

fn parse_tool_calls_json(json_str: &str) -> Option<Vec<ToolCall>> {
    let stored: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
    let tool_calls: Vec<ToolCall> = stored
        .into_iter()
        .filter_map(|v| {
            let tool_id = v.get("tool_id")?.as_str()?.to_string();
            let tool_name = v.get("tool_name")?.as_str()?.to_string();
            let args = v.get("args")?.clone();
            Some(ToolCall::new(tool_id, tool_name, args))
        })
        .collect();

    if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            id: format!("msg-{role}"),
            execution_id: Some("exec-1".to_string()),
            session_id: "sess-1".to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: "2026-07-08T00:00:00Z".to_string(),
            token_count: 1,
            tool_calls: None,
            tool_call_id: None,
            seq: 0,
        }
    }

    #[test]
    fn converts_roles_and_tool_call_id() {
        let mut tool = msg("tool", "done");
        tool.tool_call_id = Some("tc-1".to_string());

        let out = messages_to_chat_format(&[msg("user", "hi"), tool]);

        assert_eq!(out[0].role, "user");
        assert_eq!(out[1].role, "tool");
        assert_eq!(out[1].tool_call_id.as_deref(), Some("tc-1"));
        assert!(!out[0].is_summary);
    }

    #[test]
    fn parses_assistant_tool_calls() {
        let mut assistant = msg("assistant", "[tool calls]");
        assistant.tool_calls = Some(
            r#"[{"tool_id":"tc-1","tool_name":"read_file","args":{"path":"src/lib.rs"}}]"#
                .to_string(),
        );

        let out = messages_to_chat_format(&[assistant]);

        let calls = out[0].tool_calls.as_ref().expect("tool calls");
        assert_eq!(calls[0].id, "tc-1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "src/lib.rs");
    }

    #[test]
    fn malformed_tool_calls_are_ignored() {
        let mut assistant = msg("assistant", "[bad]");
        assistant.tool_calls = Some("{not json}".to_string());

        let out = messages_to_chat_format(&[assistant]);

        assert!(out[0].tool_calls.is_none());
    }
}
