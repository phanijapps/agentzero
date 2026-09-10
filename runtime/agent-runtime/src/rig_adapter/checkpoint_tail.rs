//! Completed run tail not yet absorbed by the next canonical request.
use crate::{ChatMessage, ToolCall};
use serde_json::Value;
#[derive(Clone, Default)]
pub(super) struct CheckpointTail {
    pub messages: Vec<ChatMessage>,
    pending_text: String,
}
impl CheckpointTail {
    pub fn text(&mut self, text: &str) {
        self.pending_text.push_str(text);
    }
    pub fn completed(&mut self, id: &str, name: &str, args: &Value, result: &str) {
        if self
            .messages
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some(id))
        {
            return;
        }
        let mut assistant = ChatMessage::assistant(std::mem::take(&mut self.pending_text));
        assistant.tool_calls = Some(vec![ToolCall::new(id.into(), name.into(), args.clone())]);
        self.messages.push(assistant);
        self.messages
            .push(ChatMessage::tool_result(id.into(), result.into()));
    }
}
