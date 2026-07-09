//! # Event Types
//!
//! Events are the fundamental building blocks of conversation history.
//! Each interaction is recorded as an event, forming an immutable log.

use crate::types::Content;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Event represents a single interaction in a conversation.
///
/// Events form an immutable log that captures the complete execution
/// trace of an agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique identifier for this event
    pub id: String,

    /// Timestamp when this event was created
    pub timestamp: DateTime<Utc>,

    /// Invocation identifier (groups events from a single agent run)
    pub invocation_id: String,

    /// Branch identifier for multi-path conversations
    pub branch: String,

    /// Author of this event (user, agent, tool, system)
    pub author: String,

    /// Content of this event
    pub content: Option<Content>,

    /// Actions triggered by this event
    pub actions: EventActions,

    /// Whether the turn is complete
    #[serde(default)]
    pub turn_complete: bool,

    /// IDs of long-running tools associated with this event
    #[serde(default)]
    pub long_running_tool_ids: Vec<String>,

    /// Additional metadata
    #[serde(flatten)]
    pub metadata: HashMap<String, Value>,
}

impl Event {
    /// Create a new event with a unique ID.
    pub fn new(invocation_id: impl Into<String>) -> Self {
        let invocation_id = invocation_id.into();
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            invocation_id: invocation_id.clone(),
            branch: String::new(),
            author: String::new(),
            content: None,
            actions: EventActions::default(),
            turn_complete: false,
            long_running_tool_ids: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Create an event with a specific ID (for streaming chunks).
    pub fn with_id(id: impl Into<String>, invocation_id: impl Into<String>) -> Self {
        let invocation_id = invocation_id.into();
        Self {
            id: id.into(),
            timestamp: Utc::now(),
            invocation_id: invocation_id.clone(),
            branch: String::new(),
            author: String::new(),
            content: None,
            actions: EventActions::default(),
            turn_complete: false,
            long_running_tool_ids: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Set the content of this event.
    pub fn with_content(mut self, content: Content) -> Self {
        self.content = Some(content);
        self
    }

    /// Set the author of this event.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Mark this turn as complete.
    pub fn with_turn_complete(mut self, complete: bool) -> Self {
        self.turn_complete = complete;
        self
    }

    /// Add a long-running tool ID.
    pub fn with_long_running_tool(mut self, tool_id: impl Into<String>) -> Self {
        self.long_running_tool_ids.push(tool_id.into());
        self
    }
}

/// EventActions represent actions that can be triggered by events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventActions {
    /// State changes to apply
    #[serde(default)]
    pub state_delta: HashMap<String, Value>,

    /// Skip summarization for this event
    #[serde(default)]
    pub skip_summarization: bool,

    /// Transfer to a different agent
    #[serde(default)]
    pub transfer_to_agent: Option<String>,

    /// Escalate to human
    #[serde(default)]
    pub escalate: bool,

    /// Response action from the respond tool
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond: Option<RespondAction>,

    /// Delegation action from the delegate tool
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegate: Option<DelegateAction>,
}

/// A file artifact declared by an agent in its response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactDeclaration {
    /// File path (relative to ward or absolute)
    pub path: String,
    /// Human-readable label
    pub label: Option<String>,
}

/// Action for the respond tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondAction {
    /// The response message.
    pub message: String,

    /// Format of the message (text, markdown, html).
    pub format: String,

    /// Conversation ID for routing.
    pub conversation_id: Option<String>,

    /// Session ID for web hooks.
    pub session_id: Option<String>,

    /// Artifacts produced by this execution
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactDeclaration>,
}

/// Action for the delegate tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegateAction {
    /// Target agent ID to delegate to.
    pub agent_id: String,

    /// Task description for the subagent.
    pub task: String,

    /// Task-scoped context to pass to the subagent.
    pub context: Option<Value>,

    /// Whether to wait for the result.
    pub wait_for_result: bool,

    /// Optional max iterations for the subagent execution loop.
    #[serde(default)]
    pub max_iterations: Option<u32>,

    /// Optional JSON Schema the child agent's response must conform to.
    ///
    /// When provided, the child is instructed to respond with ONLY a JSON
    /// object matching this schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,

    /// Skills to pre-load for the subagent.
    #[serde(default)]
    pub skills: Vec<String>,

    /// Task complexity level: "S", "M", "L", "XL".
    ///
    /// Used for iteration budget enforcement. When set, the executor
    /// applies complexity-based turn budgets instead of the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<String>,

    /// Optional execution posture for the delegated child.
    ///
    /// Gateway validates this wire value and maps it to its internal
    /// DelegationMode enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// Whether to run this delegation in parallel (skip per-session queue).
    ///
    /// When true, the delegation bypasses the sequential queue and runs
    /// immediately, subject only to the global concurrency semaphore.
    #[serde(default)]
    pub parallel: bool,

    /// Pre-generated execution ID for this delegation.
    ///
    /// Set by DelegateTool so the parent agent has a stable handle
    /// for subsequent steer_agent calls. If None, stream.rs generates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_execution_id: Option<String>,
}

impl EventActions {
    /// Create a new empty actions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a state delta.
    pub fn with_state(mut self, key: impl Into<String>, value: Value) -> Self {
        self.state_delta.insert(key.into(), value);
        self
    }

    /// Set skip_summarization flag.
    pub fn with_skip_summarization(mut self, skip: bool) -> Self {
        self.skip_summarization = skip;
        self
    }

    /// Set transfer_to_agent.
    pub fn with_transfer(mut self, agent: impl Into<String>) -> Self {
        self.transfer_to_agent = Some(agent.into());
        self
    }

    /// Set escalate flag.
    pub fn with_escalate(mut self, escalate: bool) -> Self {
        self.escalate = escalate;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_event_new() {
        let event = Event::new("test-invocation");
        assert_eq!(event.invocation_id, "test-invocation");
        assert!(!event.turn_complete);
    }

    #[test]
    fn test_event_with_content() {
        let content = Content::user("Hello");
        let event = Event::new("test").with_content(content);
        assert!(event.content.is_some());
        assert_eq!(event.content.as_ref().unwrap().role, "user");
    }

    #[test]
    fn test_event_with_author() {
        let event = Event::new("test").with_author("user");
        assert_eq!(event.author, "user");
    }

    #[test]
    fn test_event_serialization() {
        let event = Event::new("test")
            .with_author("user")
            .with_content(Content::user("Hello"));

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_event_actions() {
        let actions = EventActions::new()
            .with_state("key", serde_json::json!("value"))
            .with_skip_summarization(true);

        assert_eq!(actions.state_delta.get("key"), Some(&json!("value")));
        assert!(actions.skip_summarization);
    }

    #[test]
    fn test_delegate_action_complexity_field() {
        let action = DelegateAction {
            agent_id: "child".to_string(),
            task: "do work".to_string(),
            context: None,
            wait_for_result: false,
            max_iterations: None,
            output_schema: None,
            skills: vec![],
            complexity: Some("M".to_string()),
            mode: Some("ward_backed_build".to_string()),
            parallel: false,
            child_execution_id: None,
        };
        assert_eq!(action.complexity, Some("M".to_string()));
        assert_eq!(action.mode.as_deref(), Some("ward_backed_build"));
    }

    #[test]
    fn test_delegate_action_complexity_default_none() {
        let json = r#"{"agent_id":"a","task":"t","wait_for_result":false,"skills":[]}"#;
        let action: DelegateAction = serde_json::from_str(json).unwrap();
        assert_eq!(action.complexity, None);
        assert_eq!(action.mode, None);
    }
}
