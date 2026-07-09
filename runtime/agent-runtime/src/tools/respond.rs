//! # Respond Tool
//!
//! Tool for sending responses back to the originating hook.
//!
//! This tool routes messages to the correct channel (WebSocket, webhook, etc)
//! based on the `HookContext` that was set when the agent was invoked.

use agent_primitives::{Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool for sending responses back to the originating hook.
///
/// When an agent is invoked through a hook (WebSocket, webhook, CLI, etc),
/// the hook context is stored in the execution state. This tool reads that
/// context and routes the response back to the correct channel.
///
/// # Example
///
/// ```json
/// {
///   "message": "Hello! How can I help you today?",
///   "format": "markdown"
/// }
/// ```
pub struct RespondTool;

impl RespondTool {
    /// Create a new respond tool.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for RespondTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RespondTool {
    fn name(&self) -> &'static str {
        "respond"
    }

    fn description(&self) -> &'static str {
        "Send a response message back to the user through the originating channel. \
         Use this to communicate with the user who initiated the conversation."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The response message to send to the user"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "default": "text",
                    "description": "Format of the message (text, markdown, or html)"
                },
                "artifacts": {
                    "type": "array",
                    "description": "Files produced by this execution. Include any outputs the user would want to see or download.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to the current ward"
                            },
                            "label": {
                                "type": "string",
                                "description": "Human-readable label for this artifact"
                            }
                        },
                        "required": ["path"]
                    }
                }
            },
            "required": ["message"]
        }))
    }

    async fn execute(
        &self,
        ctx: Arc<dyn ToolContext>,
        args: Value,
    ) -> agent_primitives::Result<Value> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| agent_primitives::AgentError::Tool("message is required".to_string()))?;

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let artifacts: Vec<agent_primitives::event::ArtifactDeclaration> = args
            .get("artifacts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Get hook context from state
        let hook_context = ctx.get_state("hook_context");

        // Get conversation ID for response routing
        let conversation_id = ctx
            .get_state("conversation_id")
            .and_then(|v| v.as_str().map(std::string::ToString::to_string));

        // If we have hook context, use it to determine response routing
        let hook_type = hook_context
            .as_ref()
            .and_then(|v| v.get("hook_type"))
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let session_id = hook_context
            .as_ref()
            .and_then(|v| v.get("hook_type"))
            .and_then(|v| v.get("session_id"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        // Set response in actions for the executor to pick up
        let mut actions = ctx.actions();
        actions.respond = Some(agent_primitives::event::RespondAction {
            message: message.to_string(),
            format: format.to_string(),
            conversation_id: conversation_id.clone(),
            session_id: session_id.clone(),
            artifacts,
        });
        ctx.set_actions(actions);

        Ok(json!({
            "status": "sent",
            "hook_type": hook_type,
            "conversation_id": conversation_id,
            "session_id": session_id,
            "message_length": message.len()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_respond_tool_schema() {
        let tool = RespondTool::new();
        assert_eq!(tool.name(), "respond");

        let schema = tool.parameters_schema().unwrap();
        assert!(schema.get("properties").is_some());
        assert!(schema.get("properties").unwrap().get("message").is_some());
    }

    #[test]
    fn default_constructor_matches_new() {
        // Compare via tool name to avoid clippy::default_constructed_unit_structs
        // when calling Default on a unit struct directly.
        fn make_default<T: Default + Tool>() -> T {
            T::default()
        }
        let d: RespondTool = make_default();
        assert_eq!(d.name(), RespondTool::new().name());
    }

    #[test]
    fn description_is_non_empty() {
        let t = RespondTool::new();
        assert!(!t.description().is_empty());
    }

    #[tokio::test]
    async fn execute_missing_message_returns_error() {
        let tool = RespondTool::new();
        let ctx: Arc<dyn ToolContext> = Arc::new(crate::tools::context::ToolContext::new());
        let result = tool.execute(ctx, json!({})).await;
        let err = result.expect_err("must error");
        assert!(matches!(err, agent_primitives::AgentError::Tool(_)));
        assert!(format!("{err}").contains("message is required"));
    }

    #[tokio::test]
    async fn execute_with_minimal_message_returns_status_sent() {
        let tool = RespondTool::new();
        let ctx: Arc<dyn ToolContext> = Arc::new(crate::tools::context::ToolContext::new());
        let res = tool
            .execute(ctx, json!({"message": "hello"}))
            .await
            .unwrap();
        assert_eq!(res.get("status").and_then(|v| v.as_str()), Some("sent"));
        assert_eq!(
            res.get("hook_type").and_then(|v| v.as_str()),
            Some("unknown")
        );
        assert_eq!(res.get("message_length").and_then(|v| v.as_u64()), Some(5));
    }

    #[tokio::test]
    async fn execute_writes_action_with_artifacts_and_hook_context() {
        use agent_primitives::CallbackContext;
        let tool = RespondTool::new();
        let inner = crate::tools::context::ToolContext::full(
            "agent".to_string(),
            Some("conv-1".to_string()),
            vec![],
        );
        // Set hook_context state for routing
        inner.set_state(
            "hook_context".to_string(),
            json!({
                "hook_type": {
                    "type": "websocket",
                    "session_id": "session-7"
                }
            }),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(inner);
        let result = tool
            .execute(
                Arc::clone(&ctx),
                json!({
                    "message": "done!",
                    "format": "markdown",
                    "artifacts": [{"path": "out.txt", "label": "report"}]
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "sent");
        assert_eq!(result["hook_type"], "websocket");
        assert_eq!(result["session_id"], "session-7");
        assert_eq!(result["conversation_id"], "conv-1");

        // Verify the action was set on the context
        let actions = ctx.actions();
        let respond = actions.respond.expect("respond action set");
        assert_eq!(respond.message, "done!");
        assert_eq!(respond.format, "markdown");
        assert_eq!(respond.session_id.as_deref(), Some("session-7"));
        assert_eq!(respond.artifacts.len(), 1);
    }
}
