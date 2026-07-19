//! Concurrency invariant: while a 500-chunk ingestion is in-flight, unrelated
//! reads against `knowledge.db` stay responsive (<200ms p95).

use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::tempdir;

use gateway_execution::ingest::{IngestionQueue, NoopExtractor};
use gateway_services::VaultPaths;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_sqlite::kg::storage::GraphStorage;
use zbot_stores_sqlite::{
    GatewayKgEpisodeStore, KgEpisodeRepository, KnowledgeDatabase, SqliteKgStore,
};
use zbot_stores_traits::KgEpisodeStore;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unrelated_reads_stay_under_200ms_p95_during_ingestion() {
    let tmp = tempdir().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
    let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
    let episode_repo = Arc::new(KgEpisodeRepository::new(db.clone()));
    let graph_storage = Arc::new(GraphStorage::new(db.clone()).expect("graph"));
    let episode_store: Arc<dyn KgEpisodeStore> =
        Arc::new(GatewayKgEpisodeStore::new(episode_repo.clone()));
    let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph_storage));
    let extractor = Arc::new(NoopExtractor::new());

    let queue = IngestionQueue::start(2, episode_store, kg_store, extractor);

    // Seed 500 pending episodes.
    for i in 0..500 {
        let id = episode_repo
            .upsert_pending(
                "document",
                &format!("stress#{i}"),
                &format!("h{i}"),
                None,
                "root",
            )
            .expect("upsert");
        episode_repo
            .set_payload(&id, &format!("chunk {i} text"))
            .expect("payload");
    }
    queue.notify();

    // In parallel, issue 100 unrelated reads against knowledge.db.
    let db_clone = db.clone();
    let read_handle = tokio::spawn(async move {
        let mut durations = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = Instant::now();
            let _ = db_clone.with_connection(|conn| {
                let _: i64 =
                    conn.query_row("SELECT COUNT(*) FROM kg_entities", [], |r| r.get(0))?;
                Ok(())
            });
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
