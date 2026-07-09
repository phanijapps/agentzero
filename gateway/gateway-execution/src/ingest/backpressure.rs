//! Backpressure and per-source rate limiting for ingestion.
//!
//! Two gates, both checked before enqueue:
//! - Global queue depth (pending + running episodes across all sources)
//! - Per-source pending count (protects against one book starving sessions)
//!
//! Violations return Err; the HTTP layer converts those to 429.
//!
//! Phase B2: backend-agnostic — uses the `KgEpisodeStore` trait so
//! the configured backend shares the same gating logic as SQLite.

use std::sync::Arc;

use zbot_stores_traits::KgEpisodeStore;

#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    pub max_queue_depth: u64,
    pub max_per_source: u64,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            max_queue_depth: 5_000,
            max_per_source: 500,
        }
    }
}

pub struct Backpressure {
    config: BackpressureConfig,
    episode_store: Arc<dyn KgEpisodeStore>,
}

impl Backpressure {
    pub fn new(config: BackpressureConfig, episode_store: Arc<dyn KgEpisodeStore>) -> Self {
        Self {
            config,
            episode_store,
        }
    }

    /// Returns Err(String) with a human-readable reason if either gate
    /// rejects the source. The HTTP caller maps this to 429 Too Many Requests.
    pub async fn check(&self, source_ref_prefix: &str) -> Result<(), String> {
        let global_pending = self.episode_store.count_pending_global().await.unwrap_or(0);
        if global_pending >= self.config.max_queue_depth {
            return Err(format!(
                "ingestion queue full ({global_pending} pending+running, limit {})",
                self.config.max_queue_depth
            ));
        }
        let per_source_pending = self
            .episode_store
            .count_pending_for_source(source_ref_prefix)
            .await
            .unwrap_or(0);
        if per_source_pending >= self.config.max_per_source {
            return Err(format!(
                "source backpressure ({per_source_pending} pending+running for '{source_ref_prefix}', limit {})",
                self.config.max_per_source
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::VaultPaths;
    use zbot_stores_sqlite::{GatewayKgEpisodeStore, KgEpisodeRepository, KnowledgeDatabase};

    fn setup() -> (
        tempfile::TempDir,
        Arc<KgEpisodeRepository>,
        Arc<dyn KgEpisodeStore>,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        let repo = Arc::new(KgEpisodeRepository::new(db));
        let store: Arc<dyn KgEpisodeStore> = Arc::new(GatewayKgEpisodeStore::new(repo.clone()));
        (tmp, repo, store)
    }

    #[tokio::test]
    async fn allows_when_queue_is_empty() {
        let (_tmp, _repo, store) = setup();
        let bp = Backpressure::new(BackpressureConfig::default(), store);
        assert!(bp.check("anything").await.is_ok());
    }

    #[tokio::test]
    async fn rejects_over_global_cap() {
        let (_tmp, repo, store) = setup();
        // Seed exactly the cap number of pending episodes.
        let cap = 3;
        for i in 0..cap {
            repo.upsert_pending("t", &format!("x#{i}"), &format!("h{i}"), None, "root")
                .unwrap();
        }
        let bp = Backpressure::new(
            BackpressureConfig {
                max_queue_depth: cap as u64,
                max_per_source: 1_000,
            },
            store,
        );
        assert!(bp.check("x").await.is_err());
    }

    #[tokio::test]
    async fn rejects_over_per_source_cap() {
        let (_tmp, repo, store) = setup();
        for i in 0..5 {
            repo.upsert_pending("t", &format!("book#{i}"), &format!("h{i}"), None, "root")
                .unwrap();
        }
        let bp = Backpressure::new(
            BackpressureConfig {
                max_queue_depth: 1_000,
                max_per_source: 5,
            },
            store,
        );
        assert!(bp.check("book").await.is_err());
        // Different source still allowed.
        assert!(bp.check("other").await.is_ok());
    }
}
