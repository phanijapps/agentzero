//! `WikiStore` trait — backend-agnostic interface for ward wiki articles.

use crate::memory_facts::EmbeddingQueryIdentity;
use async_trait::async_trait;
use serde_json::Value;
// `WikiArticle` and `WikiHit` live in `zbot-stores-domain`; re-export
// here so the trait surface keeps working for callers that import from
// this crate.
pub use zbot_stores_domain::{WikiArticle, WikiHit};

/// Aggregate stats for the wiki subsystem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikiStats {
    pub total: i64,
}

/// Backend-agnostic interface for ward wiki articles.
///
/// Each row carries the `WikiArticle` JSON shape from `zbot-stores-domain`.
/// Methods returning `Vec<Value>` emit one row per article in the
/// canonical shape; callers deserialize via `serde_json::from_value`.
#[async_trait]
pub trait WikiStore: Send + Sync {
    /// List all articles for a ward. Default returns empty.
    async fn list_articles(&self, _ward_id: &str) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    /// Get a single article by (ward_id, title). Default returns None.
    async fn get_article(&self, _ward_id: &str, _title: &str) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// Upsert an article; `embedding` is optional.
    async fn upsert_article(
        &self,
        article: WikiArticle,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        let _ = (article, embedding);
        Err("upsert_article not implemented for this store".to_string())
    }

    /// Delete an article. Returns true if a row was removed.
    async fn delete_article(&self, _ward_id: &str, _title: &str) -> Result<bool, String> {
        Ok(false)
    }

    /// Hybrid FTS + vector search across wiki articles.
    /// Each row carries `article` + `score` + `match_source`.
    async fn search_wiki_hybrid(
        &self,
        _ward_id: Option<&str>,
        _query: &str,
        _limit: usize,
        _query_embedding: Option<&[f32]>,
    ) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    /// Identity-aware variant of `search_wiki_hybrid`.
    async fn search_wiki_hybrid_with_identity(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
        query_identity: Option<&EmbeddingQueryIdentity>,
    ) -> Result<Vec<Value>, String> {
        let _ = query_identity;
        self.search_wiki_hybrid(ward_id, query, limit, query_embedding)
            .await
    }

    /// Typed variant of `search_wiki_hybrid` returning `Vec<WikiHit>`
    /// directly. Default deserialises the Value-based result so backends
    /// only need to implement `search_wiki_hybrid`.
    async fn search_wiki_hybrid_typed(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<WikiHit>, String> {
        self.search_wiki_hybrid_typed_with_identity(ward_id, query, limit, query_embedding, None)
            .await
    }

    /// Identity-aware typed variant of `search_wiki_hybrid_typed`.
    async fn search_wiki_hybrid_typed_with_identity(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
        query_identity: Option<&EmbeddingQueryIdentity>,
    ) -> Result<Vec<WikiHit>, String> {
        let rows = self
            .search_wiki_hybrid_with_identity(
                ward_id,
                query,
                limit,
                query_embedding,
                query_identity,
            )
            .await?;
        rows.into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| format!("decode WikiHit: {e}")))
            .collect()
    }

    async fn wiki_stats(&self) -> Result<WikiStats, String> {
        Ok(WikiStats::default())
    }

    /// Pure vector-similarity search scoped to a ward, returning typed
    /// `(WikiArticle, score)` pairs directly. Used by recall paths that
    /// want richer ranking than the hybrid endpoint. Default returns
    /// empty so backends without a dedicated vector index can opt out.
    async fn search_wiki_by_similarity_typed(
        &self,
        _ward_id: &str,
        _embedding: &[f32],
        _limit: usize,
    ) -> Result<Vec<(WikiArticle, f64)>, String> {
        Ok(Vec::new())
    }

    /// Identity-aware variant of `search_wiki_by_similarity_typed`.
    async fn search_wiki_by_similarity_typed_with_identity(
        &self,
        ward_id: &str,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        limit: usize,
    ) -> Result<Vec<(WikiArticle, f64)>, String> {
        let _ = query_identity;
        self.search_wiki_by_similarity_typed(ward_id, embedding, limit)
            .await
    }

    /// Typed variant of `list_articles` returning `Vec<WikiArticle>`
    /// directly. Default deserialises the Value-based result for
    /// backends that haven't overridden.
    async fn list_articles_typed(&self, ward_id: &str) -> Result<Vec<WikiArticle>, String> {
        let rows = self.list_articles(ward_id).await?;
        rows.into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| format!("decode WikiArticle: {e}")))
            .collect()
    }
}
