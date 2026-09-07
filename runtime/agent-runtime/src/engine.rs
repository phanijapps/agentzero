//! Gateway-facing execution facade.
//!
//! This trait is the boundary Rig will implement behind AgentZero runtime
//! types. Gateway crates should depend on this surface and `StreamEvent`, not
//! Rig-native agents, streams, tools, or messages.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;

pub mod config;
pub mod error;
pub mod hooks;
pub mod prepared;
pub mod snapshot;

use crate::types::{ChatMessage, StreamEvent};
pub use config::ExecutorConfig;
pub use error::ExecutorError;
pub use hooks::{
    AfterToolCallHook, BeforeToolCallHook, RecallHook, RecallHookResult, ToolCallDecision,
    ToolExecutionMode, TransformContextHook,
};
pub use prepared::PreparedExecution;

/// Event sink used by execution engines.
pub type StreamEventSink<'a> = dyn FnMut(StreamEvent) + Send + 'a;

/// Boxed execution engine.
pub type BoxedAgentEngine = Box<dyn AgentEngine>;

/// Narrow execution boundary consumed by gateway execution.
#[async_trait]
pub trait AgentEngine: Send + Sync {
    /// Execute the agent with streaming events.
    async fn execute_stream(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError>;

    /// Execute the agent with a cooperative stop flag.
    async fn execute_stream_with_stop_flag(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        stop_flag: Option<Arc<AtomicBool>>,
        on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError>;

    /// Execute the agent and return concatenated text output.
    async fn execute(
        &self,
        user_message: &str,
        history: &[ChatMessage],
    ) -> Result<String, ExecutorError>;

    /// Identifier for which engine implementation is driving — for
    /// observability. The default covers loop-free boundary fakes.
    fn engine_name(&self) -> &'static str {
        "engine"
    }
}
