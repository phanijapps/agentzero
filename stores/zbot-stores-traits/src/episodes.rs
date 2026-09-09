//! `EpisodeStore` trait — backend-agnostic interface for session episodes.

use crate::memory_facts::EmbeddingQueryIdentity;
use async_trait::async_trait;
use serde_json::Value;
// Domain types live in `zbot-stores-domain`; re-export here so the
// trait surface keeps working for callers that import from this crate.
pub use zbot_stores_domain::{SessionEpisode, SuccessfulEpisode};

/// Aggregate health metrics returned alongside episode reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpisodeStats {
    /// Total session_episodes rows.
    pub total: i64,
}

/// Backend-agnostic interface for the session-episode subsystem.
///
/// Each row carries the `SessionEpisode` JSON shape from
/// `zbot-stores-domain` so the trait surface stays free of SQLite-specific
/// types. Methods that return `Vec<Value>` emit one Value per episode in
/// the canonical shape; callers deserialize via `serde_json::from_value`.
///
/// All methods have safe defaults (empty Vec / Ok with zero count) so
/// impls that don't yet support a method gracefully degrade rather than
/// panic — matching the pattern used by `MemoryFactStore`.
#[async_trait]
pub trait EpisodeStore: Send + Sync {
    /// List episodes for a ward, capped at `limit`. Used by the ward
    /// content endpoint and Observatory views.
    async fn list_by_ward(&self, _ward_id: &str, _limit: usize) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    /// Insert an episode. `embedding` is the optional L2-normalized
    /// vector to persist alongside. Returns the persisted row id.
    async fn insert_episode(
        &self,
        episode: SessionEpisode,
        embedding: Option<Vec<f32>>,
    ) -> Result<String, String> {
        let _ = (episode, embedding);
        Err("insert_episode not implemented for this store".to_string())
    }

    /// Vector-similarity search for episodes scoped to an agent.
    /// Each returned Value carries an `episode` field (SessionEpisode shape)
    /// and a `score` field (cosine similarity ∈ [0, 1]).
    async fn search_episodes_by_similarity(
        &self,
        _agent_id: &str,
        _embedding: &[f32],
        _threshold: f32,
        _limit: usize,
    ) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    /// Identity-aware variant of `search_episodes_by_similarity`.
    async fn search_episodes_by_similarity_with_identity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let _ = query_identity;
        self.search_episodes_by_similarity(agent_id, embedding, threshold, limit)
            .await
    }

    /// Typed variant of `search_episodes_by_similarity` returning
    /// `(SessionEpisode, score)` pairs directly. Default deserialises
    /// the Value-based result so backends only need to implement the
    /// Value-returning variant.
    async fn search_episodes_by_similarity_typed(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<(SessionEpisode, f64)>, String> {
        self.search_episodes_by_similarity_typed_with_identity(
            agent_id, embedding, None, threshold, limit,
        )
        .await
    }

    /// Identity-aware typed variant of `search_episodes_by_similarity_typed`.
    async fn search_episodes_by_similarity_typed_with_identity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<(SessionEpisode, f64)>, String> {
        let rows = self
            .search_episodes_by_similarity_with_identity(
                agent_id,
                embedding,
                query_identity,
                threshold,
                limit,
            )
            .await?;
        rows.into_iter()
            .map(|row| {
                let ep_v = row
                    .get("episode")
                    .cloned()
                    .ok_or_else(|| "missing episode field".to_string())?;
                let score = row.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let ep: SessionEpisode = serde_json::from_value(ep_v)
                    .map_err(|e| format!("decode SessionEpisode: {e}"))?;
                Ok((ep, score))
            })
            .collect()
    }

    /// Plain LIKE-style keyword search across `task_summary` /
    /// `key_learnings`. Used by the unified-search FTS fallback when no
    /// FTS5 partner table exists for episodes. Default returns empty so
    /// backends without a keyword path degrade gracefully.
    async fn keyword_search_episodes(
        &self,
        _query: &str,
        _ward_id: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        Ok(Vec::new())
    }

    /// Recent successful episodes for a ward. Used by the previous-episodes
    /// recall path to seed agent context with what worked last time.
    /// Returns typed [`SessionEpisode`] (via `zbot-stores-domain`) so
    /// callers don't pay the round-trip-through-Value tax.
    async fn fetch_recent_successful_by_ward(
        &self,
        _ward_id: &str,
        _limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        Ok(Vec::new())
    }

    /// Most recent failed episodes for a ward, newest first. Consumers
    /// surface these as an avoid-list at session start so the agent
    /// doesn't repeat known failures. Backends that don't track
    /// outcomes gracefully degrade to empty.
    async fn fetch_recent_failed_by_ward(
        &self,
        _ward_id: &str,
        _limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        Ok(Vec::new())
    }

    /// Aggregate counts. Default returns zero so backends that don't
    /// track this gracefully degrade.
    async fn episode_stats(&self) -> Result<EpisodeStats, String> {
        Ok(EpisodeStats::default())
    }

    // ---- Sleep-time pattern + synthesis (Phase D4) ----------------------
    //
    // Reads needed by the `Synthesizer` and `PatternExtractor` ops.
    // Default impls return empty so backends that haven't implemented
    // yet make the synthesis/pattern cycle a quiet no-op rather than
    // crashing.

    /// Recent successful episodes (within `lookback_days`) with their
    /// task-summary embedding loaded. Used by `PatternExtractor` to
    /// surface candidate pairs by semantic similarity. `embedding` is
    /// `None` for rows that aren't indexed yet. Default: empty.
    async fn list_successful_episodes_with_embedding(
        &self,
        _lookback_days: i64,
        _limit: usize,
    ) -> Result<Vec<SuccessfulEpisode>, String> {
        Ok(Vec::new())
    }

    /// Task summaries for a set of session ids. Used by `Synthesizer`
    /// to build per-candidate LLM context. Order is unspecified;
    /// duplicates per session are allowed if a session has multiple
    /// episodes. Default: empty.
    async fn task_summaries_for_sessions(
        &self,
        _session_ids: &[String],
    ) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}
