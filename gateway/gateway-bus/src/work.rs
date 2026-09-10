//! Broker-neutral durable-work coordination.
//!
//! The store remains authoritative. A transport only announces that persisted
//! work may be available and never owns delivery state or execution authority.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use execution_state::{WorkDraft, WorkEnvelope, WorkError, WorkPolicy, WorkStore};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Notify;

/// Closed transport failures; adapters cannot expose raw broker diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkTransportError {
    #[error("transport_unavailable")]
    Unavailable,
    #[error("transport_backpressure")]
    Backpressure,
    #[error("transport_internal")]
    Internal,
}

impl WorkTransportError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "transport_unavailable",
            Self::Backpressure => "transport_backpressure",
            Self::Internal => "transport_internal",
        }
    }
}

/// Best-effort notification port. Publishing does not transfer authority.
#[async_trait]
pub trait WorkTransport: Send + Sync {
    async fn publish(&self, envelope: &WorkEnvelope) -> Result<(), WorkTransportError>;
}

/// Broker-neutral wake signal consumed by a durable worker.
///
/// A wake is only a hint. The worker still claims through [`WorkStore`].
#[async_trait]
pub trait WorkWake: Send + Sync {
    async fn wait(&self);
}

/// In-process, wake-only transport. Tokio retains at most one unconsumed permit.
#[derive(Default)]
pub struct LocalWorkTransport {
    notify: Notify,
}

impl LocalWorkTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wait until a publisher hints that durable work may be available.
    pub async fn notified(&self) {
        self.notify.notified().await;
    }
}

#[async_trait]
impl WorkWake for LocalWorkTransport {
    async fn wait(&self) {
        self.notified().await;
    }
}

#[async_trait]
impl WorkTransport for LocalWorkTransport {
    async fn publish(&self, _envelope: &WorkEnvelope) -> Result<(), WorkTransportError> {
        self.notify.notify_one();
        Ok(())
    }
}

/// Result of a persist-first enqueue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueReceipt {
    work_id: String,
    inserted: bool,
    notification_error: Option<WorkTransportError>,
}

impl EnqueueReceipt {
    pub fn work_id(&self) -> &str {
        &self.work_id
    }

    pub fn inserted(&self) -> bool {
        self.inserted
    }

    pub fn notification_error(&self) -> Option<WorkTransportError> {
        self.notification_error
    }
}

/// Persists authorized work before issuing a best-effort wake notification.
pub struct DurableWorkQueue {
    store: Arc<dyn WorkStore>,
    policy: Arc<dyn WorkPolicy>,
    transport: Arc<dyn WorkTransport>,
}

impl DurableWorkQueue {
    pub fn new(
        store: Arc<dyn WorkStore>,
        policy: Arc<dyn WorkPolicy>,
        transport: Arc<dyn WorkTransport>,
    ) -> Self {
        Self {
            store,
            policy,
            transport,
        }
    }

    pub fn store(&self) -> Arc<dyn WorkStore> {
        self.store.clone()
    }

    pub async fn enqueue(
        &self,
        draft: WorkDraft,
        now: DateTime<Utc>,
    ) -> Result<EnqueueReceipt, WorkError> {
        let envelope = WorkEnvelope::authorize(draft, self.policy.as_ref(), now)?;
        let outcome = match self.store.enqueue(&envelope) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(
                    work_id = %envelope.id(),
                    kind = %envelope.kind(),
                    target = %envelope.target(),
                    reason_code = %error,
                    transition = "enqueue_failed",
                    "durable work coordination failure"
                );
                return Err(error);
            }
        };

        let notification_error = if outcome.inserted() {
            match self.transport.publish(outcome.item().envelope()).await {
                Ok(()) => None,
                Err(error) => {
                    tracing::warn!(
                        work_id = %outcome.item().envelope().id(),
                        kind = %outcome.item().envelope().kind(),
                        target = %outcome.item().envelope().target(),
                        reason_code = error.as_str(),
                        transition = "notification_failed",
                        "durable work coordination failure"
                    );
                    Some(error)
                }
            }
        } else {
            None
        };

        Ok(EnqueueReceipt {
            work_id: outcome.item().envelope().id().to_string(),
            inserted: outcome.inserted(),
            notification_error,
        })
    }
}
