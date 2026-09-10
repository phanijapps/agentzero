//! ExecutionError — the one typed error for gateway-execution.
//!
//! Every fallible operation in this crate returns `Result<T, ExecutionError>`
//! instead of `Result<T, String>`. Variants carry raw context for logs
//! (Display); [`ExecutionError::client_message`] returns the safe,
//! client-visible constant used on the event bus and crash paths.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ExecutionError {
    /// Agent/provider/tool settings resolution failed before execution.
    #[error("configuration: {0}")]
    Config(String),

    /// LLM client construction or provider interaction failed.
    #[error("provider: {0}")]
    Provider(String),

    /// Conversation/state/log store read or write failed.
    #[error("store: {0}")]
    Store(String),

    /// Session lifecycle: reactivate, root-execution lookup, identity.
    #[error("session: {0}")]
    Session(String),

    /// Delegation spawn, callback, or registry failure.
    #[error("delegation: {0}")]
    Delegation(String),

    /// Continuation composition, checkpoint restore, or recovery failure.
    #[error("continuation: {0}")]
    Continuation(String),

    /// Resource/artifact/ward layout access failure.
    #[error("resource: {0}")]
    Resource(String),
}

impl ExecutionError {
    /// The client-visible message — a safe constant per variant. Raw context
    /// stays in [`Display`](std::fmt::Display) for logs only.
    #[must_use]
    pub fn client_message(&self) -> String {
        match self {
            Self::Config(_) => "Unable to start this request".to_string(),
            Self::Provider(_) => "The AI provider is unavailable".to_string(),
            Self::Store(_) => "A storage operation failed".to_string(),
            Self::Session(_) => "The session could not be continued".to_string(),
            Self::Delegation(_) => "A delegated agent could not run".to_string(),
            Self::Continuation(_) => "Unable to resume this session".to_string(),
            Self::Resource(_) => "A required resource is unavailable".to_string(),
        }
    }
}

impl From<String> for ExecutionError {
    /// Default classification for untyped messages: configuration. This
    /// exists so `.map_err(ExecutionError::from)` works at store/service
    /// boundaries without naming a variant at every site.
    fn from(message: String) -> Self {
        Self::Config(message)
    }
}

impl From<&str> for ExecutionError {
    fn from(message: &str) -> Self {
        Self::Config(message.to_string())
    }
}
