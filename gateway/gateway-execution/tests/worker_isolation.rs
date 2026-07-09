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

use gateway_execution::ingest::{extractor::Extractor, IngestionQueue};
use gateway_services::VaultPaths;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_sqlite::kg::storage::GraphStorage;
use zbot_stores_sqlite::{
    GatewayKgEpisodeStore, KgEpisodeRepository, KnowledgeDatabase, SqliteKgStore,
};
use zbot_stores_traits::KgEpisodeStore;

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
    ) -> Result<(), String> {
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
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
    let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
    let repo = Arc::new(KgEpisodeRepository::new(db.clone()));
    let graph_storage = Arc::new(GraphStorage::new(db.clone()).unwrap());
    let episode_store: Arc<dyn KgEpisodeStore> = Arc::new(GatewayKgEpisodeStore::new(repo.clone()));
    let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph_storage));

    let invocations = Arc::new(AtomicU64::new(0));
    let extractor = Arc::new(PanicExtractor {
        invocations: invocations.clone(),
        panic_on: 2, // second invocation panics
    });

    let queue = IngestionQueue::start(2, episode_store, kg_store, extractor);

    // Enqueue 5 episodes with payloads.
    for i in 0..5 {
        let id = repo
            .upsert_pending("test", &format!("src#{i}"), &format!("h{i}"), None, "root")
            .unwrap();
        repo.set_payload(&id, &format!("chunk {i}")).unwrap();
    }
    queue.notify();

    // Wait up to 5 seconds for most episodes to finish.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let counts = repo.status_counts_for_source("src#").unwrap();
        if counts.done + counts.failed >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let counts = repo.status_counts_for_source("src#").unwrap();
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
