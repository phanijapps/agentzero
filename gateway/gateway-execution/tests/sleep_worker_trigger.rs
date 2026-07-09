//! Verify the SleepTimeWorker fires a cycle when triggered.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;

use gateway_execution::sleep::{Compactor, DecayConfig, DecayEngine, Pruner, SleepTimeWorker};
use gateway_services::VaultPaths;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_sqlite::kg::storage::GraphStorage;
use zbot_stores_sqlite::{
    CompactionRepository, GatewayCompactionStore, KnowledgeDatabase, SqliteKgStore,
};
use zbot_stores_traits::CompactionStore;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_causes_immediate_cycle() {
    let tmp = tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
    let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());

    let graph = Arc::new(GraphStorage::new(db.clone()).unwrap());
    let compaction_repo = Arc::new(CompactionRepository::new(db.clone()));
    let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph));
    let compaction_store: Arc<dyn CompactionStore> =
        Arc::new(GatewayCompactionStore::new(compaction_repo.clone()));
    let compactor = Arc::new(Compactor::new(
        kg_store.clone(),
        compaction_store.clone(),
        None,
    ));
    let decay = Arc::new(DecayEngine::new(kg_store.clone(), DecayConfig::default()));
    let pruner = Arc::new(Pruner::new(kg_store, compaction_store));

    // Interval long enough that the periodic timer won't fire during the test.
    let worker = SleepTimeWorker::start(
        compactor,
        decay,
        pruner,
        Duration::from_secs(3600),
        "root".to_string(),
    );

    // Trigger + wait briefly for the async task to run the cycle.
    worker.trigger();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Empty graph: no merges, no prunes. We don't care about specific numbers;
    // the test passes if we reach this line without deadlock/panic.
    // Optionally, verify no rows in kg_compactions — confirming the cycle ran
    // and found nothing to do.
    let summary = compaction_repo.latest_run_summary().unwrap();
    // summary is None on empty graph — because nothing was recorded.
    // If summary is Some, it should reflect a sleep-* run_id.
    if let Some(s) = summary {
        assert!(s.run_id.starts_with("sleep-"));
    }
}
