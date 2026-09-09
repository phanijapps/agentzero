//! Shared engram-backed store fixtures for gateway-execution integration
//! tests — the same adapter wiring production uses, on tempdirs.
//! (Integration tests cannot import the lib's `#[cfg(test)]` module.)

use std::sync::Arc;

use zbot_engram_adapter::{
    AdapterConfig, EngramKnowledgeGraphStore, EngramProvider, EngramSidecarStores,
};

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

/// One shared provider for a test's KG + KG-episode stores (they may share
/// a database safely — separate tables).
pub fn kg_and_episode_stores(
    tmp: &tempfile::TempDir,
) -> (
    Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>,
    Arc<dyn zbot_stores_traits::KgEpisodeStore>,
) {
    let (config, provider) = open_provider(tmp.path(), "engram-ingest");
    let kg: Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore> = Arc::new(
        EngramKnowledgeGraphStore::from_provider(config.clone(), &provider)
            .expect("kg store opens"),
    );
    let episodes: Arc<dyn zbot_stores_traits::KgEpisodeStore> = Arc::new(
        EngramSidecarStores::from_provider(config, &provider).expect("episode store opens"),
    );
    (kg, episodes)
}

/// The engram sidecar bundle typed as the session-episode store (the same
/// construction also implements `KgEpisodeStore`).
pub fn episode_store(tmp: &tempfile::TempDir) -> Arc<dyn zbot_stores_traits::EpisodeStore> {
    let (config, provider) = open_provider(tmp.path(), "engram-episodes");
    Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("episode store opens"))
}
