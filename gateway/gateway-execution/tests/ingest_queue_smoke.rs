//! Smoke test: enqueue a few pending episodes, start a 2-worker queue with
//! NoopExtractor, verify they get drained to 'done' within a timeout.

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

mod common;

use gateway_execution::ingest::{IngestionQueue, NoopExtractor};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_drains_pending_episodes() {
    let tmp = tempdir().unwrap();
    let (kg_store, episode_store) = common::engram_stores::kg_and_episode_stores(&tmp);

    // Enqueue 5 episodes with payloads.
    for i in 0..5 {
        let id = episode_store
            .upsert_pending(
                "test",
                &format!("src#{i}"),
                &format!("hash{i}"),
                None,
                "root",
            )
            .await
            .unwrap();
        episode_store
            .set_payload(&id, &format!("chunk {i} text"))
            .await
            .unwrap();
    }

    let extractor = Arc::new(NoopExtractor::new());
    let queue = IngestionQueue::start(2, episode_store.clone(), kg_store, extractor.clone());
    queue.notify();

    // Poll until all 5 are done or timeout.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let counts = episode_store
            .status_counts_for_source("src#")
            .await
            .unwrap();
        if counts.done == 5 {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "queue did not drain in 5s: pending={} running={} done={} failed={}",
                counts.pending, counts.running, counts.done, counts.failed,
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // NoopExtractor should have observed all 5 ids.
    let seen = extractor.seen.lock().await;
    assert_eq!(seen.len(), 5);
}
