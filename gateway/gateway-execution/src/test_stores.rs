//! Test-only construction of the production (engram) stores for
//! gateway-execution unit-test fixtures — the same wiring the daemon uses,
//! on throwaway directories. Mirrors gateway-memory's sleep::test_support.

use std::sync::Arc;

use zbot_engram_adapter::{
    AdapterConfig, EngramKnowledgeGraphStore, EngramProvider, EngramSidecarStores,
};
use zbot_stores_traits::KgEpisodeStore;

fn open_provider(root: &std::path::Path, subdir: &str) -> (AdapterConfig, EngramProvider) {
    let dir = root.join(subdir);
    std::fs::create_dir_all(&dir).expect("engram test root");
    let mut config = AdapterConfig::engram_for_data_root(&dir, "engram.db");
    config.embedding_provider.provider_type = "gateway-execution-test".to_string();
    config.embedding_provider.model = "gateway-execution-test".to_string();
    config.embedding_provider.dimensions = 8;
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    (config, provider)
}

/// An engram-backed KG-ingestion episode store for unit-test fixtures.
pub(crate) fn kg_episode_store(tmp: &tempfile::TempDir) -> Arc<dyn KgEpisodeStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-kg-episodes");
    Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("kg episode store opens"))
}

/// An engram-backed knowledge-graph store for unit-test fixtures.
pub(crate) fn kg_store(tmp: &tempfile::TempDir) -> Arc<dyn zbot_stores::KnowledgeGraphStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-kg");
    Arc::new(EngramKnowledgeGraphStore::from_provider(config, &provider).expect("kg store opens"))
}
