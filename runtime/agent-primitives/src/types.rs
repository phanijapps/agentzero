//! # Core Types
//!
//! Core data structures used across zbot agent execution.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Content represents a message with role and parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    /// The role of the content creator (user, assistant, system, tool)
    pub role: String,

    /// The parts that make up this content
    pub parts: Vec<Part>,
}

impl Content {
    /// Create a new user content.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            parts: vec![Part::Text { text: text.into() }],
        }
    }

    /// Create a new assistant content.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            parts: vec![Part::Text { text: text.into() }],
        }
    }

    /// Create a new system content.
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            parts: vec![Part::Text { text: text.into() }],
        }
    }

    /// Create a new tool response content.
    pub fn tool_response(tool_call_id: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            parts: vec![Part::FunctionResponse {
                id: tool_call_id.into(),
                response: response.into(),
            }],
        }
    }

    /// Get the text content if this is a text part.
    pub fn text(&self) -> Option<&str> {
        self.parts.iter().find_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
    }
}

/// Source of multimodal content (image, file).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ContentSource {
    /// Remote URL or data: URI — stored as-is in DB
    #[serde(rename = "url")]
    Url(String),

    /// Raw base64 encoded bytes — ephemeral, never persisted to DB
    #[serde(rename = "base64")]
    Base64(String),

    /// Local file path — what DB stores after flushing Base64 to disk
    #[serde(rename = "file_ref")]
    FileRef(String),
}

/// Image detail level for vision models.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Low,
    High,
    Auto,
}

/// Part represents a single piece of content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Part {
    /// Plain text content
    #[serde(rename = "text")]
    Text { text: String },

    /// Function call (tool call)
    #[serde(rename = "function_call")]
    FunctionCall {
        name: String,
        args: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },

    /// Function response (tool result)
    #[serde(rename = "function_response")]
    FunctionResponse { id: String, response: String },

    /// Image content (screenshots, photos, diagrams)
    #[serde(rename = "image")]
    Image {
        source: ContentSource,
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },

    /// File content (PDFs, documents, etc.)
    #[serde(rename = "file")]
    File {
        source: ContentSource,
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

impl Part {
    /// Create a text part.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create a function call part.
    pub fn function_call(name: impl Into<String>, args: Value) -> Self {
        Self::FunctionCall {
            name: name.into(),
            args,
            id: None,
        }
    }

    /// Create a function call part with ID.
    pub fn function_call_with_id(
        name: impl Into<String>,
        args: Value,
        id: impl Into<String>,
    ) -> Self {
        Self::FunctionCall {
            name: name.into(),
            args,
            id: Some(id.into()),
        }
    }

    /// Returns the type name of this part variant.
    pub fn type_name(&self) -> &'static str {
        match self {
            Part::Text { .. } => "text",
            Part::FunctionCall { .. } => "function_call",
            Part::FunctionResponse { .. } => "function_response",
            Part::Image { .. } => "image",
            Part::File { .. } => "file",
        }
    }

    /// Returns true if this part contains multimodal content (image or file).
    pub fn is_multimodal(&self) -> bool {
        matches!(self, Part::Image { .. } | Part::File { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_user() {
        let content = Content::user("Hello");
        assert_eq!(content.role, "user");
        assert_eq!(content.text(), Some("Hello"));
    }

    #[test]
    fn test_content_assistant() {
        let content = Content::assistant("Hi there");
        assert_eq!(content.role, "assistant");
    }

    #[test]
    fn test_content_system() {
        let content = Content::system("You are helpful");
        assert_eq!(content.role, "system");
    }

    #[test]
    fn test_part_text() {
        let part = Part::text("Hello");
        match part {
            Part::Text { text } => assert_eq!(text, "Hello"),
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn test_part_function_call() {
        let part = Part::function_call("search", serde_json::json!({"query": "test"}));
        match part {
            Part::FunctionCall { name, args, .. } => {
                assert_eq!(name, "search");
                assert_eq!(args["query"], "test");
            }
            _ => panic!("Expected FunctionCall part"),
        }
    }

    #[test]
    fn test_content_serialization() {
        let content = Content::user("Hello");
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_part_image_base64() {
        let part = Part::Image {
            source: ContentSource::Base64("iVBOR...".to_string()),
            mime_type: "image/png".to_string(),
            detail: Some(ImageDetail::Auto),
        };
        match &part {
            Part::Image {
                source,
                mime_type,
                detail,
            } => {
                assert!(matches!(source, ContentSource::Base64(_)));
                assert_eq!(mime_type, "image/png");
                assert!(matches!(detail, Some(ImageDetail::Auto)));
            }
            _ => panic!("Expected Image part"),
        }
    }

    #[test]
    fn test_part_image_url() {
        let part = Part::Image {
            source: ContentSource::Url("https://example.com/img.png".to_string()),
            mime_type: "image/png".to_string(),
            detail: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("image"));
        assert!(json.contains("https://example.com/img.png"));
    }

    #[test]
    fn test_part_image_fileref() {
        let part = Part::Image {
            source: ContentSource::FileRef("/tmp/attachments/abc123.png".to_string()),
            mime_type: "image/jpeg".to_string(),
            detail: Some(ImageDetail::High),
        };
        match &part {
            Part::Image { source, .. } => {
                assert!(matches!(source, ContentSource::FileRef(_)));
            }
            _ => panic!("Expected Image part"),
        }
    }

    #[test]
    fn test_part_file() {
        let part = Part::File {
            source: ContentSource::Url("https://example.com/doc.pdf".to_string()),
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
        };
        match &part {
            Part::File {
                mime_type,
                filename,
                ..
            } => {
                assert_eq!(mime_type, "application/pdf");
                assert_eq!(filename.as_deref(), Some("report.pdf"));
            }
            _ => panic!("Expected File part"),
        }
    }

    #[test]
    fn test_content_source_serialization_roundtrip() {
        let sources = vec![
            ContentSource::Url("https://example.com/img.png".to_string()),
            ContentSource::Base64("aGVsbG8=".to_string()),
            ContentSource::FileRef("/tmp/file.png".to_string()),
        ];
        for source in &sources {
            let json = serde_json::to_string(source).unwrap();
            let deserialized: ContentSource = serde_json::from_str(&json).unwrap();
            match (source, &deserialized) {
                (ContentSource::Url(a), ContentSource::Url(b)) => assert_eq!(a, b),
                (ContentSource::Base64(a), ContentSource::Base64(b)) => assert_eq!(a, b),
                (ContentSource::FileRef(a), ContentSource::FileRef(b)) => assert_eq!(a, b),
                _ => panic!("Roundtrip mismatch"),
            }
        }
    }

    #[test]
    fn test_image_detail_serialization() {
        let details = vec![ImageDetail::Low, ImageDetail::High, ImageDetail::Auto];
        for detail in &details {
            let json = serde_json::to_string(detail).unwrap();
            let deserialized: ImageDetail = serde_json::from_str(&json).unwrap();
            assert_eq!(
                serde_json::to_string(detail).unwrap(),
                serde_json::to_string(&deserialized).unwrap(),
            );
        }
    }

    #[test]
    fn test_part_type_name() {
        assert_eq!(Part::text("hello").type_name(), "text");
        let img = Part::Image {
            source: ContentSource::Base64("x".to_string()),
            mime_type: "image/png".to_string(),
            detail: None,
        };
        assert_eq!(img.type_name(), "image");
        let file = Part::File {
            source: ContentSource::Url("https://x.com/f.pdf".to_string()),
            mime_type: "application/pdf".to_string(),
            filename: None,
        };
        assert_eq!(file.type_name(), "file");
    }

    #[test]
    fn test_existing_text_and_function_still_work() {
        let text = Part::text("hello");
        assert!(matches!(text, Part::Text { .. }));

        let fc = Part::function_call("search", serde_json::json!({"q": "test"}));
        assert!(matches!(fc, Part::FunctionCall { .. }));

        let content = Content::user("test");
        assert_eq!(content.text(), Some("test"));

        let json = serde_json::to_string(&content).unwrap();
        let roundtrip: Content = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.text(), Some("test"));
    }
}
