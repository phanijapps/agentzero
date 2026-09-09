//! Test-only construction of the production (engram) fact/procedure stores.
//!
//! Unit-test fixtures that used to build the sqlite `GatewayMemoryFactStore`
//! construct the real engram adapter stack instead — the same wiring the
//! daemon uses (via `persistence_factory`), on a throwaway directory. The
//! adapter's own test suite opens one provider per test in ~milliseconds,
//! so per-test setup stays cheap while exercising production behavior.

use std::sync::Arc;

use zbot_engram_adapter::{
    AdapterConfig, EngramMemoryFactStore, EngramProvider, EngramSidecarStores,
};
use zbot_stores_traits::{MemoryFactStore, ProcedureStore};

fn adapter_config(root: &std::path::Path) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root, "engram.db");
    config.embedding_provider.provider_type = "gateway-memory-test".to_string();
    config.embedding_provider.model = "gateway-memory-test".to_string();
    config.embedding_provider.dimensions = 8;
    config
}

/// An engram-backed fact store for unit-test fixtures (no embedder wired —
/// matches the fixtures that previously passed `None`).
pub(crate) fn fact_store(tmp: &tempfile::TempDir) -> Arc<dyn MemoryFactStore> {
    let root = tmp.path().join("engram-facts");
    std::fs::create_dir_all(&root).expect("engram test root");
    let config = adapter_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    Arc::new(
        EngramMemoryFactStore::from_provider_with_embedding_client(config, &provider, None)
            .expect("fact store opens"),
    )
}

/// An engram-backed procedure store for unit-test fixtures.
pub(crate) fn procedure_store(tmp: &tempfile::TempDir) -> Arc<dyn ProcedureStore> {
    let root = tmp.path().join("engram-procedures");
    std::fs::create_dir_all(&root).expect("engram test root");
    let config = adapter_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    Arc::new(EngramSidecarStores::from_provider(config, &provider).expect("procedure store opens"))
}
