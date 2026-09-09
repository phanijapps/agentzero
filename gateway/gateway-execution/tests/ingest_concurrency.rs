//! Concurrency invariant: while a 500-chunk ingestion is in-flight, unrelated
//! reads against `knowledge.db` stay responsive (<200ms p95).

use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::tempdir;

mod common;

use gateway_execution::ingest::{IngestionQueue, NoopExtractor};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unrelated_reads_stay_under_200ms_p95_during_ingestion() {
    let tmp = tempdir().expect("tempdir");
    let (kg_store, episode_store) = common::engram_stores::kg_and_episode_stores(&tmp);
    let extractor = Arc::new(NoopExtractor::new());

    let queue = IngestionQueue::start(2, episode_store.clone(), kg_store.clone(), extractor);

    // Seed 500 pending episodes.
    for i in 0..500 {
        let id = episode_store
            .upsert_pending(
                "document",
                &format!("stress#{i}"),
                &format!("h{i}"),
                None,
                "root",
            )
            .await
            .expect("upsert");
        episode_store
            .set_payload(&id, &format!("chunk {i} text"))
            .await
            .expect("payload");
    }
    queue.notify();

    // In parallel, issue 100 unrelated reads through the store's trait
    // surface (entity count) while ingestion churns the same database.
    let read_store = kg_store.clone();
    let read_handle = tokio::spawn(async move {
        let mut durations = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = Instant::now();
            let _ = read_store.count_all_entities().await;
            durations.push(start.elapsed());
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        durations
    });

    let durations = read_handle.await.expect("reader");
    let mut sorted = durations.clone();
    sorted.sort();
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() * 95) / 100];
    let p99 = sorted[sorted.len() - 1];
    eprintln!("Reader-under-ingestion: p50={p50:?} p95={p95:?} p99={p99:?}");

    assert!(
        p95.as_millis() < 200,
        "unrelated reads must stay <200ms p95 during heavy ingestion, got p95={p95:?}"
    );
}
