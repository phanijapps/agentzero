//! Ingestion queue — producers signal "wake up, work exists"; N workers
//! race to `claim_next_pending`, one wins, others resleep. Work lives in
//! the DB so worker restarts recover in-flight pending episodes.
//!
//! Phase B2: backend-agnostic. `episode_store` is the trait surface,
//! `kg_store` is the trait surface. SQLite (and any future alternate backend) both work.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

use crate::ingest::extractor::Extractor;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_traits::KgEpisodeStore;

const WAKE_CHANNEL_CAPACITY: usize = 256;
const CLAIM_FAILURE_BACKOFF: Duration = Duration::from_millis(500);

pub struct IngestionQueue {
    tx: mpsc::Sender<()>,
}

impl IngestionQueue {
    /// Spawn `workers` background tasks; return a handle producers can use
    /// to signal new work.
    pub fn start(
        workers: usize,
        episode_store: Arc<dyn KgEpisodeStore>,
        kg_store: Arc<dyn KnowledgeGraphStore>,
        extractor: Arc<dyn Extractor>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<()>(WAKE_CHANNEL_CAPACITY);
        let notify = Arc::new(Notify::new());

        // Fan-out: a small dispatcher drains the mpsc and fires notify_waiters()
        // so every idle worker wakes. Workers race to claim; only one wins,
        // others re-sleep.
        {
            let notify = notify.clone();
            tokio::spawn(async move {
                while rx.recv().await.is_some() {
                    notify.notify_waiters();
                }
            });
        }

        for worker_idx in 0..workers {
            let episode_store = episode_store.clone();
            let kg_store = kg_store.clone();
            let extractor = extractor.clone();
            let notify = notify.clone();
            tokio::spawn(async move {
                worker_loop(worker_idx, episode_store, kg_store, extractor, notify).await;
            });
        }

        Self { tx }
    }

    /// Signal workers that work exists. Non-blocking; if the wake channel
    /// is full, workers will pick up on their next claim attempt anyway.
    pub fn notify(&self) {
        let _ = self.tx.try_send(());
    }
}

/// Pull the episode id (string) out of a Value row from the trait.
/// Both backend impls emit the canonical `KgEpisode` JSON shape with
/// the id at the top-level `id` key (already prefix-stripped).
fn episode_id_from_value(v: &serde_json::Value) -> Option<String> {
    v.get("id").and_then(|s| s.as_str()).map(|s| s.to_string())
}

async fn worker_loop(
    worker_idx: usize,
    episode_store: Arc<dyn KgEpisodeStore>,
    kg_store: Arc<dyn KnowledgeGraphStore>,
    extractor: Arc<dyn Extractor>,
    notify: Arc<Notify>,
) {
    tracing::info!(worker_idx, "ingestion worker started");
    loop {
        let claim = episode_store.claim_next_pending().await;

        let episode_value = match claim {
            Ok(Some(v)) => v,
            Ok(None) => {
                notify.notified().await;
                continue;
            }
            Err(e) => {
                tracing::warn!(worker_idx, error = %e, "claim_next_pending failed");
                tokio::time::sleep(CLAIM_FAILURE_BACKOFF).await;
                continue;
            }
        };

        let episode_id = match episode_id_from_value(&episode_value) {
            Some(id) => id,
            None => {
                tracing::warn!(worker_idx, "claimed episode has no id field — skipping");
                continue;
            }
        };

        let chunk_text = match episode_store.get_payload(&episode_id).await {
            Ok(Some(text)) => text,
            Ok(None) => {
                tracing::warn!(
                    worker_idx,
                    episode_id = %episode_id,
                    "episode has no payload — marking failed",
                );
                let _ = episode_store
                    .mark_failed(&episode_id, "payload missing")
                    .await;
                continue;
            }
            Err(e) => {
                tracing::warn!(worker_idx, error = %e, "get_payload failed");
                tokio::time::sleep(CLAIM_FAILURE_BACKOFF).await;
                continue;
            }
        };

        let result = extractor.process(&episode_id, &chunk_text, &kg_store).await;

        let finish = match result {
            Ok(()) => episode_store.mark_done(&episode_id).await,
            Err(err_msg) => {
                tracing::warn!(
                    worker_idx,
                    episode_id = %episode_id,
                    error = %err_msg,
                    "extractor failed; marking episode failed",
                );
                episode_store.mark_failed(&episode_id, &err_msg).await
            }
        };

        if let Err(db_err) = finish {
            tracing::warn!(error = %db_err, "finish status update DB error");
        }
    }
}
