mod fixtures;

use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
use async_trait::async_trait;
use gateway_services::paths::VaultPaths;
use std::sync::Arc;
use tempfile::TempDir;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_sqlite::kg::storage::GraphStorage;
use zbot_stores_sqlite::knowledge_db::KnowledgeDatabase;
use zbot_stores_sqlite::SqliteKgStore;

struct FixedEmbeddingClient {
    dimensions: usize,
}

#[async_trait]
impl EmbeddingClient for FixedEmbeddingClient {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|_| vec![0.0; self.dimensions]).collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_name(&self) -> String {
        format!("test-{}d", self.dimensions)
    }
}

async fn sqlite_store_with_embedding_dim(dimensions: usize) -> (TempDir, SqliteKgStore) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    let kdb = Arc::new(KnowledgeDatabase::new(paths).expect("kdb"));
    let storage = Arc::new(GraphStorage::new(kdb).expect("storage"));
    let client = Arc::new(FixedEmbeddingClient { dimensions });
    let store = SqliteKgStore::with_embedding_client(storage, client);
    (tmp, store)
}

#[tokio::test]
async fn entity_round_trip() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::entity_round_trip(&store).await;
}

#[tokio::test]
async fn upsert_increments_mention_count() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::upsert_increments_mention_count(&store).await;
}

#[tokio::test]
async fn bump_mention_increases_count() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::bump_mention_increases_count(&store).await;
}

#[tokio::test]
async fn resolve_exact_match() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::resolve_exact_match(&store).await;
}

#[tokio::test]
async fn resolve_via_alias() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::resolve_via_alias(&store).await;
}

#[tokio::test]
async fn resolve_no_match() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::resolve_no_match(&store).await;
}

#[tokio::test]
async fn relationship_round_trip() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::relationship_round_trip(&store).await;
}

#[tokio::test]
async fn store_knowledge_writes_both() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::store_knowledge_writes_both(&store).await;
}

#[tokio::test]
async fn neighbors_outgoing() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::neighbors_outgoing(&store).await;
}

#[tokio::test]
async fn neighbors_incoming() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::neighbors_incoming(&store).await;
}

#[tokio::test]
async fn traverse_respects_max_hops() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::traverse_respects_max_hops(&store).await;
}

#[tokio::test]
async fn fts_finds_match() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::fts_finds_match(&store).await;
}

#[tokio::test]
async fn reindex_idempotent_when_dim_matches() {
    let (_tmp, store) = sqlite_store_with_embedding_dim(1024).await;
    let first = store.reindex_embeddings(1024).await.unwrap();
    assert_eq!(
        first.tables_rebuilt,
        &[
            "memory_facts_index",
            "kg_name_index",
            "session_episodes_index",
            "wiki_articles_index",
            "procedures_index",
        ],
        "first request should report every mismatched SQLite target"
    );

    let second = store.reindex_embeddings(1024).await.unwrap();
    assert!(
        second.tables_rebuilt.is_empty(),
        "matching dimension should be a no-op, got {:?}",
        second.tables_rebuilt
    );
}

#[tokio::test]
async fn stats_reflects_writes() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::stats_reflects_writes(&store).await;
}

#[tokio::test]
async fn graph_stats_per_agent() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::graph_stats_per_agent(&store).await;
}

#[tokio::test]
async fn mark_archival_sets_class() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::mark_archival_sets_class(&store).await;
}

#[tokio::test]
async fn list_entities_respects_agent() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    zbot_stores_conformance::list_entities_respects_agent(&store).await;
}
