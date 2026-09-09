//! MemoryFactStore + ProcedureStore conformance for the engram adapter.
//!
//! Tier D parity proof: these are the scenarios the sqlite fact/procedure
//! stores exercised in their own suites. Before those backends retire,
//! the production (engram) path must pass the same behavioral contracts.

use std::sync::Arc;

use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
use async_trait::async_trait;
use zbot_engram_adapter::{
    AdapterConfig, EngramMemoryFactStore, EngramProvider, EngramSidecarStores,
};
use zbot_stores_conformance as conf;
use zbot_stores_traits::ProcedureStore;

/// Deterministic 384-dim embedder — same shape the adapter's own tests use,
/// so hybrid scenarios exercise the real (embedding-aware) paths.
const DIM: usize = 384;

fn hash_embed(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; DIM];
    for token in text.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        if token.len() < 3 {
            continue;
        }
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in token.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        vector[(hash % DIM as u64) as usize] += 1.0;
    }
    let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for v in &mut vector {
            *v /= norm;
        }
    }
    vector
}

struct ConfEmbedder;

#[async_trait]
impl EmbeddingClient for ConfEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|t| hash_embed(t)).collect())
    }
    fn dimensions(&self) -> usize {
        DIM
    }
    fn model_name(&self) -> String {
        "conf-hash-384".to_string()
    }
    fn provider_type(&self) -> String {
        "conf-hash".to_string()
    }
}

fn adapter_config(root: &tempfile::TempDir) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    config.embedding_provider.provider_type = "conf-hash".to_string();
    config.embedding_provider.model = "conf-hash-384".to_string();
    config.embedding_provider.dimensions = DIM as u32;
    config
}

fn fact_store(root: &tempfile::TempDir) -> EngramMemoryFactStore {
    let config = adapter_config(root);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    EngramMemoryFactStore::from_provider_with_embedding_client(
        config,
        &provider,
        Some(Arc::new(ConfEmbedder)),
    )
    .expect("fact store opens")
}

#[tokio::test]
async fn conformance_memory_save_and_count() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_save_and_count(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_recall_finds_match() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_recall_finds_match(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_recall_respects_agent_isolation() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_recall_respects_agent_isolation(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_list_facts_filters_and_paginates() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_list_facts_filters_and_paginates(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_get_by_id_round_trip() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_get_by_id_round_trip(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_delete_fact_removes_it() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_delete_fact_removes_it(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_archive_fact_hides_from_listing() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_archive_fact_hides_from_listing(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_supersede_fact_succeeds() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_supersede_fact_succeeds(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_upsert_typed_fact_round_trip() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_upsert_typed_fact_round_trip(&fact_store(&root)).await;
}

#[tokio::test]
async fn conformance_memory_hybrid_search_finds_match() {
    let root = tempfile::tempdir().expect("root");
    conf::memory_hybrid_search_finds_match(&fact_store(&root)).await;
}

/// Procedures: upsert + similarity-search round trip with identity-consistent
/// embeddings — the surface `run_procedure` and recall depend on.
#[tokio::test]
async fn conformance_procedure_upsert_and_similarity_round_trip() {
    let root = tempfile::tempdir().expect("root");
    let config = adapter_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let store = EngramSidecarStores::from_provider(config, &provider).expect("sidecars");

    let procedure = zbot_stores_domain::Procedure {
        id: "conf-p1".into(),
        agent_id: "conf-agent".into(),
        ward_id: Some("conf-ward".into()),
        name: "deploy_via_research".into(),
        description: "Delegate data gathering to research agent, then publish.".into(),
        trigger_pattern: Some("research then publish".into()),
        steps: r#"["gather via research agent","publish findings"]"#.into(),
        parameters: None,
        success_count: 3,
        failure_count: 1,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: "2026-09-01T00:00:00Z".into(),
        updated_at: "2026-09-01T00:00:00Z".into(),
    };
    store
        .upsert_procedure(procedure, Some(hash_embed("research publish delegate")))
        .await
        .expect("upsert");

    let identity = zbot_stores_traits::EmbeddingQueryIdentity {
        provider_type: "conf-hash".into(),
        model: "conf-hash-384".into(),
        dimensions: DIM as u32,
        prompt_profile: "query".into(),
        normalization: None,
    };
    let hits = store
        .search_procedures_by_similarity_typed_with_identity(
            &hash_embed("research publish delegate"),
            Some(&identity),
            "conf-agent",
            Some("conf-ward"),
            5,
        )
        .await
        .expect("similarity search");
    assert!(
        hits.iter().any(|(p, _score)| p.id == "conf-p1"),
        "seeded procedure must be retrievable by similar query: {hits:?}"
    );

    // Agent isolation: another agent's procedures never surface.
    let other = store
        .search_procedures_by_similarity_typed_with_identity(
            &hash_embed("research publish delegate"),
            Some(&identity),
            "conf-other-agent",
            None,
            5,
        )
        .await
        .expect("other-agent search");
    assert!(
        !other.iter().any(|(p, _)| p.id == "conf-p1"),
        "procedure must be agent-scoped"
    );
}
