//! Test-only construction of the production (engram) stores.
//!
//! Unit-test fixtures that used to build sqlite stores construct the real
//! engram adapter stack instead — the same wiring the daemon uses (via
//! `persistence_factory`), on throwaway directories. The adapter's own test
//! suite opens one provider per test in ~milliseconds, so per-test setup
//! stays cheap while exercising production behavior.

use std::sync::Arc;

use knowledge_graph::kg_trait::KnowledgeGraphStore;
use zbot_engram_adapter::{
    AdapterConfig, EngramBeliefStore, EngramKnowledgeGraphStore, EngramMemoryFactStore,
    EngramProvider, EngramSidecarStores,
};
use zbot_stores_traits::{
    BeliefContradictionStore, BeliefStore, CompactionStore, EpisodeStore, MemoryFactStore,
    ProcedureStore,
};

fn adapter_config(root: &std::path::Path) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root, "engram.db");
    config.embedding_provider.provider_type = "gateway-memory-test".to_string();
    config.embedding_provider.model = "gateway-memory-test".to_string();
    config.embedding_provider.dimensions = 8;
    config
}

fn open_provider(root: &std::path::Path, subdir: &str) -> (AdapterConfig, EngramProvider) {
    let dir = root.join(subdir);
    std::fs::create_dir_all(&dir).expect("engram test root");
    let config = adapter_config(&dir);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    (config, provider)
}

/// An engram-backed fact store for unit-test fixtures (no embedder wired —
/// matches the fixtures that previously passed `None`).
pub(crate) fn fact_store(tmp: &tempfile::TempDir) -> Arc<dyn MemoryFactStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-facts");
    Arc::new(
        EngramMemoryFactStore::from_provider_with_embedding_client(config, &provider, None)
            .expect("fact store opens"),
    )
}

/// An engram-backed procedure store for unit-test fixtures.
pub(crate) fn procedure_store(tmp: &tempfile::TempDir) -> Arc<dyn ProcedureStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-procedures");
    Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("sidecar store opens"))
}

/// An engram-backed episode store for unit-test fixtures.
pub(crate) fn episode_store(tmp: &tempfile::TempDir) -> Arc<dyn EpisodeStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-episodes");
    Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("sidecar store opens"))
}

/// An engram-backed compaction store for unit-test fixtures.
pub(crate) fn compaction_store(tmp: &tempfile::TempDir) -> Arc<dyn CompactionStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-compaction");
    Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("sidecar store opens"))
}

/// An engram-backed knowledge-graph store for unit-test fixtures.
pub(crate) fn kg_store(tmp: &tempfile::TempDir) -> Arc<dyn KnowledgeGraphStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-kg");
    Arc::new(EngramKnowledgeGraphStore::from_provider(config, &provider).expect("kg store opens"))
}

/// Engram-backed belief + contradiction stores sharing one provider — the
/// adapter implements both traits on one type, so a test that needs both
/// passes the two Arcs from the same call.
pub(crate) fn belief_stores(
    tmp: &tempfile::TempDir,
) -> (Arc<dyn BeliefStore>, Arc<dyn BeliefContradictionStore>) {
    let (config, provider) = open_provider(tmp.path(), "engram-beliefs");
    let shared =
        Arc::new(EngramBeliefStore::from_provider(config, &provider).expect("belief store opens"));
    let contradictions: Arc<dyn BeliefContradictionStore> = shared.clone();
    let beliefs: Arc<dyn BeliefStore> = shared;
    (beliefs, contradictions)
}
