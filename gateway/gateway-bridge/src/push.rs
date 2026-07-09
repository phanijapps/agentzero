//! # Outbox Push
//!
//! Drains pending outbox items to connected workers and runs a background retry loop.

use crate::outbox::{OutboxItem, OutboxRepository};
use crate::protocol::BridgeServerMessage;
use crate::registry::BridgeRegistry;
use std::sync::Arc;

/// Push all pending items for an adapter to the connected worker.
///
/// Called after Hello (replay) and after new items are inserted.
pub async fn push_pending(
    adapter_id: &str,
    outbox_repo: &OutboxRepository,
    registry: &BridgeRegistry,
) {
    let items = match outbox_repo.get_unacked(adapter_id) {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(adapter_id = %adapter_id, "Failed to get pending items: {}", e);
            return;
        }
    };

    for item in &items {
        push_single_item(outbox_repo, registry, adapter_id, item).await;
    }
}

/// Push a single outbox item to a worker.
pub async fn push_single_item(
    outbox_repo: &OutboxRepository,
    registry: &BridgeRegistry,
    adapter_id: &str,
    item: &OutboxItem,
) {
    // Parse the payload back from JSON string
    let payload: serde_json::Value = match serde_json::from_str(&item.payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(outbox_id = %item.id, "Failed to parse payload: {}", e);
            return;
        }
    };

    // Mark as inflight before sending
    if let Err(e) = outbox_repo.mark_inflight(&item.id) {
        tracing::warn!(outbox_id = %item.id, "Failed to mark inflight: {}", e);
        return;
    }

    let msg = BridgeServerMessage::OutboxItem {
        outbox_id: item.id.clone(),
        capability: item.capability.clone(),
        payload,
    };

    if let Err(e) = registry.send(adapter_id, msg).await {
        tracing::warn!(
            adapter_id = %adapter_id,
            outbox_id = %item.id,
            "Failed to push to worker: {}",
            e
        );
        // Reset back to pending
        let _ = outbox_repo.reset_inflight(adapter_id);
    }
}

/// Spawn a background retry loop that periodically pushes retryable items.
///
/// Returns a join handle that can be aborted on shutdown.
pub fn spawn_retry_loop(
    registry: Arc<BridgeRegistry>,
    outbox_repo: Arc<OutboxRepository>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));

        loop {
            interval.tick().await;

            let adapter_ids = registry.list().await;
            for adapter_id in adapter_ids {
                // Push any retryable failed items
                let retryable = match outbox_repo.get_retryable(&adapter_id) {
                    Ok(items) => items,
                    Err(_) => continue,
                };

                for item in &retryable {
                    // Reset status to pending for retry
                    push_single_item(&outbox_repo, &registry, &adapter_id, item).await;
                }

                // Also push any pending items that may have been missed
                push_pending(&adapter_id, &outbox_repo, &registry).await;
            }
        }
    })
}

/// Insert an outbox item and push it immediately if the worker is connected.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_and_push(
    adapter_id: &str,
    capability: &str,
    payload: &serde_json::Value,
    session_id: Option<&str>,
    thread_id: Option<&str>,
    agent_id: Option<&str>,
    outbox_repo: &OutboxRepository,
    registry: &BridgeRegistry,
) -> Result<String, crate::error::BridgeError> {
    let id = outbox_repo.insert(
        adapter_id, capability, payload, session_id, thread_id, agent_id,
    )?;

    if registry.is_connected(adapter_id).await {
        push_pending(adapter_id, outbox_repo, registry).await;
    }

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending_requests::PendingRequests;
    use crate::protocol::BridgeServerMessage;
    use tokio::sync::mpsc;

    fn setup() -> (Arc<OutboxRepository>, Arc<BridgeRegistry>) {
        use gateway_services::VaultPaths;

        let dir = tempfile::TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(dir.path().to_path_buf()));
        let db = Arc::new(zbot_stores_sqlite::DatabaseManager::new(paths).unwrap());
        let outbox = Arc::new(OutboxRepository::new(db));
        let registry = Arc::new(BridgeRegistry::new());
        (outbox, registry)
    }

    #[tokio::test]
    async fn test_push_pending_to_connected() {
        let (outbox, registry) = setup();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pending = Arc::new(PendingRequests::new());

        registry
            .register("w1".to_string(), vec![], vec![], tx, pending)
            .await
            .unwrap();

        outbox
            .insert(
                "w1",
                "send_msg",
                &serde_json::json!({"text": "hi"}),
                None,
                None,
                None,
            )
            .unwrap();

        push_pending("w1", &outbox, &registry).await;

        // Should receive the outbox item
        let msg = rx.recv().await.unwrap();
        match msg {
            BridgeServerMessage::OutboxItem {
                capability,
                payload,
                ..
            } => {
                assert_eq!(capability, "send_msg");
                assert_eq!(payload["text"], "hi");
            }
            _ => panic!("Expected OutboxItem"),
        }
    }

    #[tokio::test]
    async fn test_push_pending_not_connected() {
        let (outbox, registry) = setup();

        outbox
            .insert("w1", "send_msg", &serde_json::json!({}), None, None, None)
            .unwrap();

        // Should not panic — just skip
        push_pending("w1", &outbox, &registry).await;

        // Item should still be pending
        let items = outbox.get_unacked("w1").unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn test_enqueue_and_push() {
        let (outbox, registry) = setup();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pending = Arc::new(PendingRequests::new());

        registry
            .register("w1".to_string(), vec![], vec![], tx, pending)
            .await
            .unwrap();

        let id = enqueue_and_push(
            "w1",
            "respond",
            &serde_json::json!({"message": "hello"}),
            Some("sess-1"),
            None,
            Some("root"),
            &outbox,
            &registry,
        )
        .await
        .unwrap();

        assert!(id.starts_with("obx-"));

        // Should receive the message
        let msg = rx.recv().await.unwrap();
        assert!(matches!(msg, BridgeServerMessage::OutboxItem { .. }));
    }

    #[tokio::test]
    async fn test_enqueue_when_disconnected() {
        let (outbox, registry) = setup();

        let id = enqueue_and_push(
            "w1",
            "respond",
            &serde_json::json!({"message": "hello"}),
            None,
            None,
            None,
            &outbox,
            &registry,
        )
        .await
        .unwrap();

        assert!(id.starts_with("obx-"));

        // Item should be pending (not pushed since worker is offline)
        let items = outbox.get_unacked("w1").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "pending");
    }
}
