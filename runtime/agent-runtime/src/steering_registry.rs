//! Registry mapping execution IDs to live SteeringHandles.
//!
//! Created once at server startup. Passed to both the executor builder
//! (for SteerAgentTool) and spawn_delegated_agent (to store handles).

use crate::steering::{SteeringHandle, SteeringMessage, SteeringPriority, SteeringSource};
use std::collections::HashMap;
use std::sync::RwLock;

/// Result of a steer attempt.
#[derive(Debug, PartialEq)]
pub enum SteerResult {
    /// Message delivered to the running agent.
    Delivered,
    /// No agent found with that execution_id (completed, failed, or unknown).
    AgentNotRunning,
}

/// Thread-safe map from execution_id to SteeringHandle.
#[derive(Default)]
pub struct SteeringRegistry {
    parent_handles: RwLock<HashMap<String, SteeringHandle>>,
    peer_handles: RwLock<HashMap<String, SteeringHandle>>,
}

impl SteeringRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handle when a subagent starts.
    pub fn register(&self, execution_id: &str, handle: SteeringHandle) {
        self.parent_handles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(execution_id.to_string(), handle.clone());
        self.peer_handles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(execution_id.to_string(), handle);
    }

    /// Register a root/continuation handle for durable peer replies without
    /// making that execution a target of the parent-only `steer_agent` path.
    pub fn register_peer_only(&self, execution_id: &str, handle: SteeringHandle) {
        self.peer_handles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(execution_id.to_string(), handle);
    }

    /// Return whether an execution currently has a live peer-delivery slot.
    /// This is an identity/lifecycle probe only; it does not expose the handle.
    pub fn has_peer_handle(&self, execution_id: &str) -> bool {
        self.peer_handles
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(execution_id)
    }

    /// Remove a handle when a subagent completes.
    pub fn remove(&self, execution_id: &str) {
        self.parent_handles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(execution_id);
        self.peer_handles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(execution_id);
    }

    /// Send a parent-sourced steering message to a running subagent.
    ///
    /// Returns `AgentNotRunning` if no handle exists or the channel is closed.
    pub fn steer(&self, execution_id: &str, message: impl Into<String>) -> SteerResult {
        self.steer_with_source(execution_id, message, SteeringSource::Parent)
    }

    /// Send an explicitly peer-sourced data message to a running execution.
    ///
    /// Unlike ordinary steering, durable peer delivery is acknowledged only
    /// after the executor has included the message in a successful LLM call.
    pub async fn steer_peer(&self, execution_id: &str, message: impl Into<String>) -> SteerResult {
        let handle = self
            .peer_handles
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(execution_id)
            .cloned();
        let Some(handle) = handle else {
            return SteerResult::AgentNotRunning;
        };
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let message = SteeringMessage::with_delivery_ack(
            message,
            SteeringSource::Peer,
            SteeringPriority::Normal,
            ack_tx,
        );
        if handle.send(message).is_err() {
            return SteerResult::AgentNotRunning;
        }
        match ack_rx.await {
            Ok(()) => SteerResult::Delivered,
            Err(_) => SteerResult::AgentNotRunning,
        }
    }

    fn steer_with_source(
        &self,
        execution_id: &str,
        message: impl Into<String>,
        source: SteeringSource,
    ) -> SteerResult {
        let handles = self
            .parent_handles
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match handles.get(execution_id) {
            None => SteerResult::AgentNotRunning,
            Some(handle) => {
                let msg = SteeringMessage::new(message, source, SteeringPriority::Normal);
                match handle.send(msg) {
                    Ok(()) => SteerResult::Delivered,
                    Err(_) => SteerResult::AgentNotRunning, // channel closed — agent done
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steering::SteeringQueue;

    #[test]
    fn steer_unknown_id_returns_not_running() {
        let registry = SteeringRegistry::new();
        assert_eq!(
            registry.steer("exec-unknown", "hello"),
            SteerResult::AgentNotRunning
        );
    }

    #[test]
    fn register_then_steer_delivers_message() {
        let (mut queue, handle) = SteeringQueue::new();
        let registry = SteeringRegistry::new();
        registry.register("exec-123", handle);

        let result = registry.steer("exec-123", "pivot to approach B");
        assert_eq!(result, SteerResult::Delivered);

        let messages = queue.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "pivot to approach B");
        assert_eq!(messages[0].source, SteeringSource::Parent);
    }

    #[tokio::test]
    async fn steer_peer_waits_for_confirmed_peer_delivery() {
        let (mut queue, handle) = SteeringQueue::new();
        let registry = SteeringRegistry::new();
        registry.register("exec-peer", handle);

        let delivery = registry.steer_peer("exec-peer", "peer data");
        let acknowledge = async {
            tokio::task::yield_now().await;
            let mut messages = queue.drain();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].source, SteeringSource::Peer);
            messages[0].acknowledge_delivery();
        };
        let (result, ()) = tokio::join!(delivery, acknowledge);
        assert_eq!(result, SteerResult::Delivered);
    }

    #[tokio::test]
    async fn peer_only_registration_cannot_be_used_by_parent_steering() {
        let (mut queue, handle) = SteeringQueue::new();
        let registry = SteeringRegistry::new();
        registry.register_peer_only("exec-root", handle);

        assert_eq!(
            registry.steer("exec-root", "unauthorized parent steering"),
            SteerResult::AgentNotRunning
        );
        let delivery = registry.steer_peer("exec-root", "authorized peer reply");
        let acknowledge = async {
            tokio::task::yield_now().await;
            let mut messages = queue.drain();
            assert_eq!(messages.len(), 1);
            messages[0].acknowledge_delivery();
        };
        let (result, ()) = tokio::join!(delivery, acknowledge);
        assert_eq!(result, SteerResult::Delivered);
    }

    #[test]
    fn remove_then_steer_returns_not_running() {
        let (_queue, handle) = SteeringQueue::new();
        let registry = SteeringRegistry::new();
        registry.register("exec-456", handle);
        registry.remove("exec-456");

        assert_eq!(
            registry.steer("exec-456", "too late"),
            SteerResult::AgentNotRunning
        );
    }

    #[test]
    fn steer_dropped_channel_returns_not_running() {
        let (queue, handle) = SteeringQueue::new();
        let registry = SteeringRegistry::new();
        registry.register("exec-789", handle);
        drop(queue); // drop receiver — channel closed

        assert_eq!(
            registry.steer("exec-789", "no one listening"),
            SteerResult::AgentNotRunning
        );
    }
}
