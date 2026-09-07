//! The engine hook framework — one trait, many slots, ordered composition.
//!
//! Extend by implementing [`EngineHook`] and adding to a [`HookSet`]. New
//! hook points are added as defaulted trait methods: existing implementors
//! keep compiling, the set fans out in registration order.
//!
//! Semantics:
//! - `before_tool`: the first `Block` wins; otherwise `Allow`.
//! - `after_tool`: replacements chain — the last `Some` wins.
//! - `transform_context`: applied in registration order.
//! - `recall`: the first hook returning a non-empty packet wins.

use crate::types::ChatMessage;
use serde_json::Value;
use std::collections::HashSet;

/// Fresh memory context produced by a recall hook.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecallPacket {
    /// Formatted system message to inject (empty if nothing novel).
    pub system_message: String,
    /// Keys of the included facts, for dedup tracking.
    pub fact_keys: Vec<String>,
}

impl RecallPacket {
    pub fn is_empty(&self) -> bool {
        self.system_message.is_empty()
    }
}

/// A tool-call gate decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDecision {
    /// Allow the tool call to proceed.
    Allow,
    /// Block it; the reason is returned to the model as the tool result.
    Block { reason: String },
}

/// Error surfaced by a hook; typed, never a bare string.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HookError {
    pub message: String,
}

impl HookError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// One engine hook. All methods are defaulted no-ops — implement only what
/// you need. Object-safe; stored as `Arc<dyn EngineHook>` in a [`HookSet`].
#[async_trait::async_trait]
pub trait EngineHook: Send + Sync {
    /// Before a tool executes. Return [`ToolDecision::Block`] to veto; the
    /// first veto across the set wins.
    async fn before_tool(&self, _name: &str, _args: &Value) -> ToolDecision {
        ToolDecision::Allow
    }

    /// After a tool completed. Return `Some(replacement)` to rewrite the
    /// result shown to the model; the last replacement across the set wins.
    async fn after_tool(
        &self,
        _name: &str,
        _args: &Value,
        _result: &str,
        _ok: bool,
    ) -> Option<String> {
        None
    }

    /// Before each model call; may mutate the outgoing message list.
    async fn transform_context(&self, _messages: &mut Vec<ChatMessage>) {}

    /// Periodic memory refresh. Called on the recall schedule; return a
    /// non-empty packet to inject novel facts.
    async fn recall(
        &self,
        _query: &str,
        _already_injected: &HashSet<String>,
    ) -> Result<RecallPacket, HookError> {
        Ok(RecallPacket::default())
    }
}

/// An ordered set of hooks fanned out by the engine.
#[derive(Default)]
pub struct HookSet {
    slots: Vec<std::sync::Arc<dyn EngineHook>>,
}

impl HookSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a hook. Runs after previously registered hooks.
    pub fn add(&mut self, hook: std::sync::Arc<dyn EngineHook>) -> &mut Self {
        self.slots.push(hook);
        self
    }

    /// Register a hook, consuming the builder for chained calls.
    pub fn with(mut self, hook: std::sync::Arc<dyn EngineHook>) -> Self {
        self.slots.push(hook);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// First `Block` wins.
    pub async fn before_tool(&self, name: &str, args: &Value) -> ToolDecision {
        for hook in &self.slots {
            if let ToolDecision::Block { reason } = hook.before_tool(name, args).await {
                return ToolDecision::Block { reason };
            }
        }
        ToolDecision::Allow
    }

    /// Last replacement wins.
    pub async fn after_tool(
        &self,
        name: &str,
        args: &Value,
        result: &str,
        ok: bool,
    ) -> Option<String> {
        let mut replacement = None;
        for hook in &self.slots {
            if let Some(next) = hook.after_tool(name, args, result, ok).await {
                replacement = Some(next);
            }
        }
        replacement
    }

    /// Registration order.
    pub async fn transform_context(&self, messages: &mut Vec<ChatMessage>) {
        for hook in &self.slots {
            hook.transform_context(messages).await;
        }
    }

    /// First non-empty packet wins.
    pub async fn recall(
        &self,
        query: &str,
        already_injected: &HashSet<String>,
    ) -> Result<RecallPacket, HookError> {
        for hook in &self.slots {
            let packet = hook.recall(query, already_injected).await?;
            if !packet.is_empty() {
                return Ok(packet);
            }
        }
        Ok(RecallPacket::default())
    }
}

impl Clone for HookSet {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Gate(&'static str);
    #[async_trait::async_trait]
    impl EngineHook for Gate {
        async fn before_tool(&self, name: &str, _args: &Value) -> ToolDecision {
            if name == self.0 {
                ToolDecision::Block {
                    reason: "denied".into(),
                }
            } else {
                ToolDecision::Allow
            }
        }
    }

    struct Rewrite(&'static str);
    #[async_trait::async_trait]
    impl EngineHook for Rewrite {
        async fn after_tool(
            &self,
            _name: &str,
            _args: &Value,
            _result: &str,
            _ok: bool,
        ) -> Option<String> {
            Some(self.0.to_string())
        }
    }

    struct Recall(&'static str);
    #[async_trait::async_trait]
    impl EngineHook for Recall {
        async fn recall(
            &self,
            _query: &str,
            _injected: &HashSet<String>,
        ) -> Result<RecallPacket, HookError> {
            Ok(RecallPacket {
                system_message: self.0.to_string(),
                fact_keys: vec![self.0.to_string()],
            })
        }
    }

    #[tokio::test]
    async fn first_block_wins_and_allow_passes() {
        let mut set = HookSet::new();
        set.add(std::sync::Arc::new(Gate("shell")));
        assert_eq!(
            set.before_tool("shell", &json!({})).await,
            ToolDecision::Block {
                reason: "denied".into()
            }
        );
        assert_eq!(
            set.before_tool("read", &json!({})).await,
            ToolDecision::Allow
        );
    }

    #[tokio::test]
    async fn last_replacement_wins() {
        let set = HookSet::new()
            .with(std::sync::Arc::new(Rewrite("first")))
            .with(std::sync::Arc::new(Rewrite("second")));
        assert_eq!(
            set.after_tool("t", &json!({}), "raw", true).await,
            Some("second".to_string())
        );
    }

    #[tokio::test]
    async fn first_non_empty_recall_wins() {
        let set = HookSet::new().with(std::sync::Arc::new(Recall("novel")));
        let packet = set.recall("q", &HashSet::new()).await.unwrap();
        assert_eq!(packet.system_message, "novel");
    }

    #[tokio::test]
    async fn defaulted_hooks_are_noops() {
        let set = HookSet::new();
        assert!(set.is_empty());
        assert_eq!(
            set.before_tool("x", &json!({})).await,
            ToolDecision::Allow
        );
        assert_eq!(set.after_tool("x", &json!({}), "", false).await, None);
        let empty = set.recall("q", &HashSet::new()).await.unwrap();
        assert!(empty.is_empty());
    }
}
