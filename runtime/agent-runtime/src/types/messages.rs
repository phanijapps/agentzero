// ============================================================================
// CHAT MESSAGE TYPES
// Core message types for LLM communication
// ============================================================================

use agent_primitives::types::Part;
use serde::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A chat message in the conversation
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Message role (system, user, assistant, tool)
    pub role: String,

    /// Message content as a list of parts (text, image, file, etc.)
    pub content: Vec<Part>,

    /// Tool calls made by the assistant (optional)
    pub tool_calls: Option<Vec<ToolCall>>,

    /// ID of the tool call this message is responding to (optional)
    pub tool_call_id: Option<String>,

    /// True iff this message was produced by a compaction pass
    /// (summarization, plan-block rewrite, or similar). Consumers use
    /// this flag to avoid re-processing — e.g. the summarizer must
    /// never summarize a message that is itself a summary, and
    /// context-editing must never collapse a pinned plan block.
    ///
    /// Replaces the brittle `text.starts_with("[Turn ")` prefix sniff
    /// that previously served the same purpose. Defaults to `false` —
    /// regular conversation turns are never summaries.
    ///
    /// Not round-tripped on the wire: the flag is an in-process hint
    /// for middleware, not part of the LLM provider payload. Both
    /// `Serialize` and `Deserialize` below omit the field.
    pub is_summary: bool,
}

impl ChatMessage {
    /// Create a new user message
    #[must_use]
    pub fn user(content: String) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![Part::Text { text: content }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        }
    }

    /// Create a new assistant message
    #[must_use]
    pub fn assistant(content: String) -> Self {
        Self {
            role: "assistant".to_string(),
            content: vec![Part::Text { text: content }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        }
    }

    /// Create a new system message
    #[must_use]
    pub fn system(content: String) -> Self {
        Self {
            role: "system".to_string(),
            content: vec![Part::Text { text: content }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        }
    }

    /// Create a new tool result message
    #[must_use]
    pub fn tool_result(tool_call_id: String, content: String) -> Self {
        Self {
            role: "tool".to_string(),
            content: vec![Part::Text { text: content }],
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            is_summary: false,
        }
    }

    /// Get all text content joined as a single string.
    #[must_use]
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Returns true if this message contains any non-text parts.
    #[must_use]
    pub fn has_multimodal_content(&self) -> bool {
        self.content
            .iter()
            .any(agent_primitives::Part::is_multimodal)
    }
}

// Custom serialization: text-only -> plain string, multimodal -> array
impl Serialize for ChatMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let field_count = 2
            + self.tool_calls.as_ref().map_or(0, |_| 1)
            + self.tool_call_id.as_ref().map_or(0, |_| 1);
        let mut s = serializer.serialize_struct("ChatMessage", field_count)?;
        s.serialize_field("role", &self.role)?;
        if self.has_multimodal_content() {
            s.serialize_field("content", &self.content)?;
        } else {
            s.serialize_field("content", &self.text_content())?;
        }
        if let Some(ref tc) = self.tool_calls {
            s.serialize_field("tool_calls", tc)?;
        }
        if let Some(ref id) = self.tool_call_id {
            s.serialize_field("tool_call_id", id)?;
        }
        s.end()
    }
}

// Custom deserialization: accept both string and array content
impl<'de> Deserialize<'de> for ChatMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawMessage {
            role: String,
            content: Value,
            tool_calls: Option<Vec<ToolCall>>,
            tool_call_id: Option<String>,
        }
        let raw = RawMessage::deserialize(deserializer)?;
        let content = match raw.content {
            Value::String(text) => vec![Part::Text { text }],
            Value::Array(_) => {
                serde_json::from_value(raw.content).map_err(serde::de::Error::custom)?
            }
            Value::Null => vec![],
            other => {
                return Err(serde::de::Error::custom(format!(
                    "expected string or array for content, got {other}"
                )))
            }
        };
        Ok(ChatMessage {
            role: raw.role,
            content,
            tool_calls: raw.tool_calls,
            tool_call_id: raw.tool_call_id,
            is_summary: false,
        })
    }
}

/// A tool call made by the LLM
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call
    pub id: String,

    /// Name of the tool to call
    pub name: String,

    /// Arguments to pass to the tool (JSON object)
    pub arguments: Value,
}

// Custom serialization to match OpenAI's format
impl Serialize for ToolCall {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        // OpenAI format: { id, type: "function", function: { name, arguments: "..." } }
        let arguments_str =
            serde_json::to_string(&self.arguments).map_err(serde::ser::Error::custom)?;

        let mut s = serializer.serialize_struct("ToolCall", 3)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("type", "function")?;
        s.serialize_field(
            "function",
            &ToolCallFunctionSerialization {
                name: &self.name,
                arguments: &arguments_str,
            },
        )?;
        s.end()
    }
}

/// Helper struct for serializing the function part of a tool call
#[derive(Serialize)]
struct ToolCallFunctionSerialization<'a> {
    name: &'a str,
    arguments: &'a str,
}

impl ToolCall {
    /// Create a new tool call
    #[must_use]
    pub fn new(id: String, name: String, arguments: Value) -> Self {
        Self {
            id,
            name,
            arguments,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let user_msg = ChatMessage::user("Hello".to_string());
        assert_eq!(user_msg.role, "user");
        assert_eq!(user_msg.text_content(), "Hello");
    }

    #[test]
    fn test_tool_call_creation() {
        let tool_call = ToolCall::new(
            "call_123".to_string(),
            "search".to_string(),
            serde_json::json!({"query": "test"}),
        );
        assert_eq!(tool_call.id, "call_123");
        assert_eq!(tool_call.name, "search");
    }

    #[test]
    fn test_message_text_helper() {
        let msg = ChatMessage::user("Hello".to_string());
        assert_eq!(msg.text_content(), "Hello");
        assert_eq!(msg.content.len(), 1);
    }

    #[test]
    fn test_message_multimodal_content() {
        use agent_primitives::types::{ContentSource, ImageDetail};
        let msg = ChatMessage {
            role: "user".to_string(),
            content: vec![
                Part::Text {
                    text: "What is this?".to_string(),
                },
                Part::Image {
                    source: ContentSource::Base64("aGVsbG8=".to_string()),
                    mime_type: "image/png".to_string(),
                    detail: Some(ImageDetail::Auto),
                },
            ],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        };
        assert_eq!(msg.text_content(), "What is this?");
        assert!(msg.has_multimodal_content());
    }

    #[test]
    fn test_serialization_text_only_is_string() {
        let msg = ChatMessage::user("Hello".to_string());
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["content"], "Hello");
    }

    #[test]
    fn test_serialization_multimodal_is_array() {
        use agent_primitives::types::ContentSource;
        let msg = ChatMessage {
            role: "user".to_string(),
            content: vec![
                Part::Text {
                    text: "Describe".to_string(),
                },
                Part::Image {
                    source: ContentSource::Url("https://example.com/img.png".to_string()),
                    mime_type: "image/png".to_string(),
                    detail: None,
                },
            ],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json["content"].is_array());
    }

    #[test]
    fn test_deserialization_from_string() {
        let json = r#"{"role":"user","content":"Hello"}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text_content(), "Hello");
        assert_eq!(msg.content.len(), 1);
    }

    #[test]
    fn test_deserialization_from_array() {
        let json = r#"{"role":"user","content":[{"type":"text","text":"Hello"}]}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text_content(), "Hello");
    }

    #[test]
    fn assistant_and_system_constructors() {
        let a = ChatMessage::assistant("yo".to_string());
        assert_eq!(a.role, "assistant");
        assert_eq!(a.text_content(), "yo");
        assert!(a.tool_call_id.is_none());

        let s = ChatMessage::system("be nice".to_string());
        assert_eq!(s.role, "system");
        assert_eq!(s.text_content(), "be nice");
        assert!(!s.is_summary);
    }

    #[test]
    fn tool_result_constructor() {
        let m = ChatMessage::tool_result("c1".to_string(), "ok".to_string());
        assert_eq!(m.role, "tool");
        assert_eq!(m.tool_call_id.as_deref(), Some("c1"));
        assert_eq!(m.text_content(), "ok");
    }

    #[test]
    fn deserialization_handles_null_content() {
        let json = r#"{"role":"assistant","content":null}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_empty());
        assert_eq!(msg.role, "assistant");
    }

    #[test]
    fn deserialization_rejects_invalid_content_type() {
        // A number is neither string nor array nor null
        let json = r#"{"role":"user","content":42}"#;
        let res: Result<ChatMessage, _> = serde_json::from_str(json);
        assert!(res.is_err());
    }

    #[test]
    fn serialization_includes_tool_calls_and_id() {
        let mut tc = vec![ToolCall::new(
            "call-1".to_string(),
            "search".to_string(),
            serde_json::json!({"q": "rust"}),
        )];
        // Set a second one for variety
        tc.push(ToolCall::new(
            "call-2".to_string(),
            "calc".to_string(),
            serde_json::json!({}),
        ));
        let msg = ChatMessage {
            role: "assistant".to_string(),
            content: vec![Part::Text {
                text: "hi".to_string(),
            }],
            tool_calls: Some(tc),
            tool_call_id: Some("parent".to_string()),
            is_summary: false,
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "hi");
        assert!(v["tool_calls"].is_array());
        assert_eq!(v["tool_call_id"], "parent");

        // Check ToolCall openai-style serialization
        let first_tc = &v["tool_calls"][0];
        assert_eq!(first_tc["type"], "function");
        assert_eq!(first_tc["function"]["name"], "search");
        // arguments is stringified JSON (OpenAI format)
        assert!(first_tc["function"]["arguments"].is_string());
        let args_str = first_tc["function"]["arguments"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(args_str).unwrap();
        assert_eq!(parsed["q"], "rust");
    }
}
