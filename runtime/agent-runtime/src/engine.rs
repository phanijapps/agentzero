//! Gateway-facing execution facade.
//!
//! This trait is the boundary Rig will implement behind AgentZero runtime
//! types. Gateway crates should depend on this surface and `StreamEvent`, not
//! Rig-native agents, streams, tools, or messages.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;

use crate::executor::{AgentExecutor, ExecutorError};
use crate::types::{ChatMessage, StreamEvent};

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

    /// Identifier for which engine implementation is driving — for observability
    /// and for testing the Rig cutover selector.
    fn engine_name(&self) -> &'static str {
        "agent-executor"
    }
}

#[async_trait]
impl AgentEngine for AgentExecutor {
    async fn execute_stream(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        mut on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        AgentExecutor::execute_stream(self, user_message, history, &mut on_event).await
    }

    async fn execute_stream_with_stop_flag(
        &self,
        user_message: &str,
        history: &[ChatMessage],
        stop_flag: Option<Arc<AtomicBool>>,
        on_event: &mut StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        AgentExecutor::execute_stream_with_stop_flag(
            self,
            user_message,
            history,
            stop_flag,
            |event| {
                on_event(event);
            },
        )
        .await
    }

    async fn execute(
        &self,
        user_message: &str,
        history: &[ChatMessage],
    ) -> Result<String, ExecutorError> {
        AgentExecutor::execute(self, user_message, history).await
    }
}
