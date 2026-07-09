//! # Tool Trait
//!
//! Tool execution interface for zbot agent execution.
//!
//! ## Permissions
//!
//! Tools can optionally declare their permissions via the `permissions()` method.
//! This enables the orchestrator to make informed routing decisions and
//! show appropriate warnings to users.
//!
//! ```ignore
//! use agent_primitives::{Tool, ToolPermissions, ToolRiskLevel};
//!
//! // Inside an `impl Tool for MyTool` block — override permissions() to
//! // declare risk level and requirements:
//! fn permissions(&self) -> ToolPermissions {
//!     ToolPermissions::moderate(vec!["network:http".into()])
//! }
//! ```

use crate::context::ToolContext;
use crate::error::Result;
use crate::policy::ToolPermissions;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Core tool trait.
///
/// All tools must implement this trait. Tools receive a context and
/// arguments, and return a JSON-serializable result.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool's name.
    fn name(&self) -> &str;

    /// Get the tool's description.
    fn description(&self) -> &str;

    /// Get the JSON Schema for the tool's parameters.
    fn parameters_schema(&self) -> Option<Value> {
        None
    }

    /// Get the JSON Schema for the tool's response.
    fn response_schema(&self) -> Option<Value> {
        None
    }

    /// Get the tool's permission requirements.
    ///
    /// Override this to declare risk level, required capabilities,
    /// and resource limits. Default is safe with no requirements.
    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::default()
    }

    /// Validate arguments before execution.
    ///
    /// Override this to perform custom validation. Called before execute().
    /// Default implementation does nothing (accepts all arguments).
    fn validate(&self, _args: &Value) -> Result<()> {
        Ok(())
    }

    /// Execute the tool with the given context and arguments.
    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{CallbackContext, ReadonlyContext, ToolContext};
    use crate::event::EventActions;
    use crate::types::Content;

    struct TestTool {
        name: String,
        description: String,
    }

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            &self.description
        }

        async fn execute(&self, _ctx: Arc<dyn ToolContext>, _args: Value) -> Result<Value> {
            Ok(serde_json::json!({"result": "ok"}))
        }
    }

    #[tokio::test]
    async fn test_tool_trait() {
        let tool = TestTool {
            name: "test".to_string(),
            description: "Test tool".to_string(),
        };

        assert_eq!(tool.name(), "test");
        assert_eq!(tool.description(), "Test tool");

        let result = tool
            .execute(Arc::new(MockContext), serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    struct MockContext;

    impl ReadonlyContext for MockContext {
        fn invocation_id(&self) -> &str {
            "test"
        }
        fn agent_name(&self) -> &str {
            "test"
        }
        fn user_id(&self) -> &str {
            "test"
        }
        fn app_name(&self) -> &str {
            "test"
        }
        fn session_id(&self) -> &str {
            "test"
        }
        fn branch(&self) -> &str {
            "test"
        }
        fn user_content(&self) -> &Content {
            use std::sync::LazyLock;
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for MockContext {
        fn get_state(&self, _key: &str) -> Option<Value> {
            None
        }
        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for MockContext {
        fn function_call_id(&self) -> String {
            "test".to_string()
        }
        fn actions(&self) -> EventActions {
            EventActions::default()
        }
        fn set_actions(&self, _actions: EventActions) {}
    }

    unsafe impl Send for MockContext {}
    unsafe impl Sync for MockContext {}
}
