//! # Agent Callbacks
//!
//! Callback types for agent lifecycle hooks.

use crate::{CallbackContext, Content};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Callback invoked before an agent runs.
///
/// Can optionally return additional content to prepend to the user's message.
pub type BeforeAgentCallback = Arc<
    dyn Fn(Arc<dyn CallbackContext>) -> Pin<Box<dyn Future<Output = Option<Content>> + Send>>
        + Send
        + Sync,
>;

/// Callback invoked after an agent completes.
///
/// Can optionally return additional content to append to the agent's response.
pub type AfterAgentCallback = Arc<
    dyn Fn(Arc<dyn CallbackContext>) -> Pin<Box<dyn Future<Output = Option<Content>> + Send>>
        + Send
        + Sync,
>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_before_callback() {
        let callback: BeforeAgentCallback =
            Arc::new(|_ctx| Box::pin(async move { Some(Content::user("Preprocessed content")) }));

        let result = callback(Arc::new(MockCallbackContext)).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().role, "user");
    }

    #[tokio::test]
    async fn test_after_callback() {
        let callback: AfterAgentCallback = Arc::new(|_ctx| {
            Box::pin(async move { Some(Content::assistant("Postprocessed content")) })
        });

        let result = callback(Arc::new(MockCallbackContext)).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().role, "assistant");
    }

    // Mock context for testing
    struct MockCallbackContext;

    impl crate::ReadonlyContext for MockCallbackContext {
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
            static CONTENT: Content = Content {
                role: String::new(),
                parts: Vec::new(),
            };
            &CONTENT
        }
    }

    impl crate::CallbackContext for MockCallbackContext {
        fn get_state(&self, _key: &str) -> Option<serde_json::Value> {
            None
        }
        fn set_state(&self, _key: String, _value: serde_json::Value) {}
    }
}
