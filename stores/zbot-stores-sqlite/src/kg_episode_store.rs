// ============================================================================
// GATEWAY KG EPISODE STORE
// SQLite-backed implementation of the KgEpisodeStore trait.
// Wraps KgEpisodeRepository so the existing knowledge.db-coupled storage
// logic stays here and the gateway/runtime/queue sees only the trait.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use zbot_stores_traits::{KgEpisodeStatusCounts, KgEpisodeStore, StoreError, StoreResult};

use crate::kg_episode_repository::{KgEpisode, KgEpisodeRepository};

/// SQLite-backed `KgEpisodeStore`. Delegates to the concrete
/// `KgEpisodeRepository` so the existing schema, indexes, and
/// transaction semantics serve both the legacy concrete callers and
/// the trait-routed paths.
pub struct GatewayKgEpisodeStore {
    repo: Arc<KgEpisodeRepository>,
}

impl GatewayKgEpisodeStore {
    pub fn new(repo: Arc<KgEpisodeRepository>) -> Self {
        Self { repo }
    }
}

/// Translate the SQLite `KgEpisode` struct to the canonical JSON shape
/// the trait surface emits. Field names match the trait doc comment.
fn episode_to_value(ep: KgEpisode) -> Value {
    json!({
        "id": ep.id,
        "source_type": ep.source_type,
        "source_ref": ep.source_ref,
        "content_hash": ep.content_hash,
        "session_id": ep.session_id,
        "agent_id": ep.agent_id,
        "status": ep.status,
        "retry_count": ep.retry_count,
        "error": ep.error,
        "created_at": ep.created_at,
        "started_at": ep.started_at,
        "completed_at": ep.completed_at })
}

#[async_trait]
impl KgEpisodeStore for GatewayKgEpisodeStore {
    async fn get_episode(&self, id: &str) -> StoreResult<Option<Value>> {
        Ok(self.repo.get(id)?.map(episode_to_value))
    }

    async fn get_by_content_hash(
        &self,
        source_type: &str,
        content_hash: &str,
    ) -> StoreResult<Option<Value>> {
        Ok(self
            .repo
            .get_by_content_hash(source_type, content_hash)?
            .map(episode_to_value))
    }

    async fn list_by_session(&self, session_id: &str) -> StoreResult<Vec<Value>> {
        Ok(self
            .repo
            .list_by_session(session_id)?
            .into_iter()
            .map(episode_to_value)
            .collect())
    }

    async fn status_counts_for_source(
        &self,
        source_ref_prefix: &str,
    ) -> StoreResult<KgEpisodeStatusCounts> {
        let c = self.repo.status_counts_for_source(source_ref_prefix)?;
        Ok(KgEpisodeStatusCounts {
            pending: c.pending,
            running: c.running,
            done: c.done,
            failed: c.failed,
        })
    }

    async fn count_pending_global(&self) -> StoreResult<u64> {
        self.repo.count_pending_global().map_err(StoreError::from)
    }

    async fn count_pending_for_source(&self, source_ref_prefix: &str) -> StoreResult<u64> {
        self.repo
            .count_pending_for_source(source_ref_prefix)
            .map_err(StoreError::from)
    }

    async fn upsert_pending(
        &self,
        source_type: &str,
        source_ref: &str,
        content_hash: &str,
        session_id: Option<&str>,
        agent_id: &str,
    ) -> StoreResult<String> {
        self.repo
            .upsert_pending(source_type, source_ref, content_hash, session_id, agent_id)
            .map_err(StoreError::from)
    }

    async fn claim_next_pending(&self) -> StoreResult<Option<Value>> {
        Ok(self.repo.claim_next_pending()?.map(episode_to_value))
    }

    async fn mark_done(&self, id: &str) -> StoreResult<()> {
        self.repo.mark_done(id).map_err(StoreError::from)
    }

    async fn mark_failed(&self, id: &str, error: &str) -> StoreResult<()> {
        self.repo.mark_failed(id, error).map_err(StoreError::from)
    }

    async fn retry_if_eligible(&self, id: &str, max_retries: u32) -> StoreResult<bool> {
        self.repo
            .retry_if_eligible(id, max_retries)
            .map_err(StoreError::from)
    }

    async fn set_payload(&self, id: &str, text: &str) -> StoreResult<()> {
        self.repo.set_payload(id, text).map_err(StoreError::from)
    }

    async fn get_payload(&self, id: &str) -> StoreResult<Option<String>> {
        self.repo.get_payload(id).map_err(StoreError::from)
    }
}
