//! Worker-panic isolation: a panicking Extractor kills only its own worker
//! task; siblings continue draining.
//!
//! Known limitation: tokio task panic leaves the claimed episode in `running`
//! state (no cleanup path). Phase 6 can add a claim-lease-timeout. For Phase 5
//! this test asserts sibling liveness, not perfect cleanup.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tempfile::tempdir;

mod common;

use gateway_execution::ingest::{extractor::Extractor, IngestionQueue};
use knowledge_graph::kg_trait::KnowledgeGraphStore;

struct PanicExtractor {
    invocations: Arc<AtomicU64>,
    panic_on: u64,
}

#[async_trait]
impl Extractor for PanicExtractor {
    async fn process(
        &self,
        _episode_id: &str,
        _chunk_text: &str,
        _kg_store: &Arc<dyn KnowledgeGraphStore>,
    ) -> Result<(), gateway_execution::errors::ExecutionError> {
        let n = self.invocations.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.panic_on {
            panic!("simulated extractor panic (invocation {n})");
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_panic_does_not_kill_siblings() {
    let tmp = tempdir().unwrap();
    let (kg_store, episode_store) = common::engram_stores::kg_and_episode_stores(&tmp);

    let invocations = Arc::new(AtomicU64::new(0));
    let extractor = Arc::new(PanicExtractor {
        invocations: invocations.clone(),
        panic_on: 2, // second invocation panics
    });

    let queue = IngestionQueue::start(2, episode_store.clone(), kg_store, extractor);

    // Enqueue 5 episodes with payloads.
    for i in 0..5 {
        let id = episode_store
            .upsert_pending("test", &format!("src#{i}"), &format!("h{i}"), None, "root")
            .await
            .unwrap();
        episode_store
            .set_payload(&id, &format!("chunk {i}"))
            .await
            .unwrap();
    }
    queue.notify();

    // Wait up to 5 seconds for most episodes to finish.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let counts = episode_store
            .status_counts_for_source("src#")
            .await
            .unwrap();
        if counts.done + counts.failed >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let counts = episode_store
        .status_counts_for_source("src#")
        .await
        .unwrap();
    // Siblings must have kept going — we expect at least 3 successfully done
    // (5 total − 1 panic victim − 1 possibly-stuck). Accept some stuck in
    // `running` since tokio panic has no cleanup hook.
    assert!(
        counts.done >= 3,
        "expected at least 3 done; got pending={} running={} done={} failed={}",
        counts.pending,
        counts.running,
        counts.done,
        counts.failed,
    );
}
