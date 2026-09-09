/// Marker prefix for TurnComplete fallback content.
pub const TURN_COMPLETE_MARKER: &str = "\x00TURN_COMPLETE\x00";

/// Accumulator for building the final response from stream events.
#[derive(Default)]
pub struct ResponseAccumulator {
    content: String,
    turn_complete_fallback: Option<String>,
}

/// Resolve the assistant content persisted for one tool-call turn.
///
/// A `respond` action transports the terminal answer in tool-call arguments,
/// not in streamed assistant tokens. Both normal executions and delegated
/// continuations must use this resolver so a reload can recover that answer.
pub(crate) fn assistant_turn_content(
    turn_text: &mut String,
    tool_calls: &[serde_json::Value],
) -> String {
    if !turn_text.is_empty() {
        return std::mem::take(turn_text);
    }

    for tool_call in tool_calls {
        if tool_call.get("tool_name").and_then(|value| value.as_str()) != Some("respond") {
            continue;
        }
        let Some(args) = tool_call.get("args") else {
            continue;
        };
        let message = args
            .get("text")
            .or_else(|| args.get("message"))
            .and_then(|value| value.as_str());
        if let Some(message) = message.filter(|message| !message.is_empty()) {
            return message.to_owned();
        }
    }

    "[tool calls]".to_owned()
}

impl ResponseAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, content: &str) {
        if let Some(message) = content.strip_prefix(TURN_COMPLETE_MARKER) {
            self.turn_complete_fallback = Some(message.to_string());
            return;
        }
        self.content.push_str(content);
    }

    pub fn into_response(self) -> String {
        let trimmed = self.content.trim();
        if !trimmed.is_empty() {
            trimmed.to_string()
        } else if let Some(fallback) = self.turn_complete_fallback {
            fallback.trim().to_string()
        } else {
            String::new()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content.trim().is_empty() && self.turn_complete_fallback.is_none()
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_accumulator() {
        let mut acc = ResponseAccumulator::new();
        assert!(acc.is_empty());
        acc.append("Hello");
        assert!(!acc.is_empty());
        assert_eq!(acc.content(), "Hello");
        acc.append(" World");
        assert_eq!(acc.content(), "Hello World");
        assert_eq!(acc.into_response(), "Hello World");
    }

    #[test]
    fn test_response_accumulator_with_respond_tool() {
        let mut acc = ResponseAccumulator::new();
        acc.append("Initial response");
        acc.append("\n\nFrom respond tool");
        assert_eq!(acc.into_response(), "Initial response\n\nFrom respond tool");
    }

    #[test]
    fn turn_complete_used_as_fallback_when_no_tokens() {
        let mut acc = ResponseAccumulator::new();
        acc.append(&format!("{}final message", TURN_COMPLETE_MARKER));
        assert_eq!(acc.into_response(), "final message");
    }

    #[test]
    fn token_content_wins_over_turn_complete_fallback() {
        let mut acc = ResponseAccumulator::new();
        acc.append("token content");
        acc.append(&format!("{}turn complete", TURN_COMPLETE_MARKER));
        assert_eq!(acc.into_response(), "token content");
    }

    #[test]
    fn assistant_turn_content_persists_a_respond_message_without_tokens() {
        let mut text = String::new();
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "message": "terminal answer" } })];

        assert_eq!(assistant_turn_content(&mut text, &calls), "terminal answer");
    }

    #[test]
    fn assistant_turn_content_prefers_streamed_text_over_a_respond_argument() {
        let mut text = "progress text".to_owned();
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "message": "terminal answer" } })];

        assert_eq!(assistant_turn_content(&mut text, &calls), "progress text");
        assert!(text.is_empty());
    }

    #[test]
    fn assistant_turn_content_supports_legacy_text_and_uses_the_first_respond() {
        let mut text = String::new();
        let calls = vec![
            serde_json::json!({
                "tool_name": "graph_query",
                "args": { "query": "memory" } }),
            serde_json::json!({
                "tool_name": "respond",
                "args": { "text": "first terminal answer", "message": "newer shape" } }),
            serde_json::json!({
                "tool_name": "respond",
                "args": { "message": "second terminal answer" } }),
        ];

        assert_eq!(
            assistant_turn_content(&mut text, &calls),
            "first terminal answer"
        );
    }

    #[test]
    fn assistant_turn_content_uses_placeholder_without_a_nonempty_respond_argument() {
        let mut text = String::new();
        let calls = vec![serde_json::json!({
            "tool_name": "respond",
            "args": { "message": "" } })];

        assert_eq!(assistant_turn_content(&mut text, &calls), "[tool calls]");
    }
}
