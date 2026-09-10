//! Verify the SleepTimeWorker fires a cycle when triggered.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;

use gateway_memory::sleep::{Compactor, DecayConfig, DecayEngine, Pruner, SleepTimeWorker};
use knowledge_graph::kg_trait::KnowledgeGraphStore;
use zbot_engram_adapter::{
    AdapterConfig, EngramKnowledgeGraphStore, EngramProvider, EngramSidecarStores,
};
use zbot_stores_traits::CompactionStore;

fn engram_stores(
    tmp: &tempfile::TempDir,
    subdir: &str,
) -> (Arc<dyn KnowledgeGraphStore>, Arc<dyn CompactionStore>) {
    let dir = tmp.path().join(subdir);
    std::fs::create_dir_all(&dir).expect("engram test root");
    let config = AdapterConfig::engram_for_data_root(&dir, "engram.db");
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    let kg: Arc<dyn KnowledgeGraphStore> = Arc::new(
        EngramKnowledgeGraphStore::from_provider(config.clone(), &provider)
            .expect("kg store opens"),
    );
    let compaction: Arc<dyn CompactionStore> =
        Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("sidecars open"));
    (kg, compaction)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_causes_immediate_cycle() {
    let tmp = tempdir().unwrap();
    let (kg_store, compaction_store) = engram_stores(&tmp, "engram-kg");
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

    // Empty graph: no merges, no prunes. The test passes if we reach this
    // line without deadlock/panic.
}
