//! Smoke test: enqueue a few pending episodes, start a 2-worker queue with
//! NoopExtractor, verify they get drained to 'done' within a timeout.

use std::sync::Arc;
use std::time::Duration;
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
async fn queue_drains_pending_episodes() {
    let tmp = tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
    let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
    let repo = Arc::new(KgEpisodeRepository::new(db.clone()));
    let graph_storage = Arc::new(GraphStorage::new(db.clone()).unwrap());
    let episode_store: Arc<dyn KgEpisodeStore> = Arc::new(GatewayKgEpisodeStore::new(repo.clone()));
    let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph_storage));

    // Enqueue 5 episodes with payloads.
    for i in 0..5 {
        let id = repo
            .upsert_pending(
                "test",
                &format!("src#{i}"),
                &format!("hash{i}"),
                None,
                "root",
            )
            .unwrap();
        repo.set_payload(&id, &format!("chunk {i} text")).unwrap();
    }

    let extractor = Arc::new(NoopExtractor::new());
    let queue = IngestionQueue::start(2, episode_store.clone(), kg_store, extractor.clone());
    queue.notify();

    // Poll until all 5 are done or timeout.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let counts = repo.status_counts_for_source("src#").unwrap();
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
