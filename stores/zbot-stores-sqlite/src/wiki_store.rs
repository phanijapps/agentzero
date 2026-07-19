// ============================================================================
// GATEWAY WIKI STORE
// SQLite-backed implementation of the WikiStore trait. Wraps
// WardWikiRepository so the SQLite-coupled storage logic stays here and
// the gateway/runtime sees only the trait.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use zbot_stores_domain::{WikiArticle, WikiHit};
use zbot_stores_traits::{WikiStats, WikiStore};

use crate::wiki_repository::WardWikiRepository;

pub struct GatewayWikiStore {
    repo: Arc<WardWikiRepository>,
}

impl GatewayWikiStore {
    pub fn new(repo: Arc<WardWikiRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl WikiStore for GatewayWikiStore {
    async fn list_articles(&self, ward_id: &str) -> Result<Vec<Value>, String> {
        let articles = self.repo.list_articles(ward_id)?;
        articles
            .into_iter()
            .map(|a| serde_json::to_value(a).map_err(|e| e.to_string()))
            .collect()
    }

    async fn get_article(&self, ward_id: &str, title: &str) -> Result<Option<Value>, String> {
        match self.repo.get_article(ward_id, title)? {
            Some(a) => Ok(Some(serde_json::to_value(a).map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    async fn upsert_article(
        &self,
        article: Value,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        let mut typed: WikiArticle =
            serde_json::from_value(article).map_err(|e| format!("decode WikiArticle: {e}"))?;
        if embedding.is_some() {
            typed.embedding = embedding;
        }
        self.repo.upsert_article(&typed)
    }

    async fn delete_article(&self, ward_id: &str, title: &str) -> Result<bool, String> {
        self.repo.delete_article(ward_id, title)
    }

    async fn search_wiki_hybrid(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<Value>, String> {
        let hits =
            self.repo
                .search_hybrid(query, ward_id, query_embedding.map(|e| e.to_vec()), limit)?;
        Ok(hits
            .into_iter()
            .map(|h| {
                serde_json::json!({
                    "article": h.article,
                    "score": h.score,
                    "match_source": h.match_source,
                })
            })
            .collect())
    }

    async fn search_wiki_hybrid_typed(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<WikiHit>, String> {
        // Override the trait default to skip the JSON round-trip — the
        // SQLite repo already returns the typed shape we want.
        self.repo
            .search_hybrid(query, ward_id, query_embedding.map(|e| e.to_vec()), limit)
    }

    async fn wiki_stats(&self) -> Result<WikiStats, String> {
        // The repo only counts per-ward; cross-ward aggregate isn't a method
        // it exposes. Return zero (the trait contract is "best-effort
        // snapshot"). Backends that track this differently can override.
        Ok(WikiStats::default())
    }

    async fn search_wiki_by_similarity_typed(
        &self,
        ward_id: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(WikiArticle, f64)>, String> {
        self.repo.search_by_similarity(ward_id, embedding, limit)
    }
}
