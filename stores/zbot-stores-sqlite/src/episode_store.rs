// ============================================================================
// GATEWAY EPISODE STORE
// SQLite-backed implementation of the EpisodeStore trait.
// Wraps EpisodeRepository so the SQLite-coupled storage logic stays here
// and the gateway/runtime sees only the trait.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::params;
use serde_json::Value;
use zbot_stores_traits::{EpisodeStats, EpisodeStore, SuccessfulEpisode};

use crate::episode_repository::EpisodeRepository;
use zbot_stores_domain::SessionEpisode;

/// SQLite-backed `EpisodeStore`. Wraps the existing `EpisodeRepository`
/// so the same r2d2 pool + vec0 index serves both legacy concrete
/// callers (still many) and the new trait-routed paths.
pub struct GatewayEpisodeStore {
    repo: Arc<EpisodeRepository>,
}

impl GatewayEpisodeStore {
    pub fn new(repo: Arc<EpisodeRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl EpisodeStore for GatewayEpisodeStore {
    async fn list_by_ward(&self, ward_id: &str, limit: usize) -> Result<Vec<Value>, String> {
        let episodes = self.repo.list_by_ward(ward_id, limit)?;
        episodes
            .into_iter()
            .map(|e| serde_json::to_value(e).map_err(|err| err.to_string()))
            .collect()
    }

    async fn insert_episode(
        &self,
        episode: Value,
        embedding: Option<Vec<f32>>,
    ) -> Result<String, String> {
        let mut typed: SessionEpisode =
            serde_json::from_value(episode).map_err(|e| format!("decode SessionEpisode: {e}"))?;
        if embedding.is_some() {
            typed.embedding = embedding;
        }
        self.repo.insert(&typed)
    }

    async fn search_episodes_by_similarity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let scored =
            self.repo
                .search_by_similarity(agent_id, embedding, threshold as f64, limit)?;
        scored
            .into_iter()
            .map(|(ep, score)| {
                Ok(serde_json::json!({
                    "episode": ep,
                    "score": score,
                }))
            })
            .collect()
    }

    async fn search_episodes_by_similarity_typed(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<(SessionEpisode, f64)>, String> {
        // Skip the JSON round-trip — the repo already returns the typed
        // shape we want.
        self.repo
            .search_by_similarity(agent_id, embedding, threshold as f64, limit)
    }

    async fn keyword_search_episodes(
        &self,
        query: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        self.repo.keyword_search(query, ward_id, limit)
    }

    async fn fetch_recent_successful_by_ward(
        &self,
        ward_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        self.repo.fetch_recent_successful_by_ward(ward_id, limit)
    }

    async fn episode_stats(&self) -> Result<EpisodeStats, String> {
        let total = self.repo.count()?;
        Ok(EpisodeStats { total })
    }

    // ---- Sleep-time pattern + synthesis (Phase D4) -------------------------

    async fn list_successful_episodes_with_embedding(
        &self,
        lookback_days: i64,
        limit: usize,
    ) -> Result<Vec<SuccessfulEpisode>, String> {
        let db = self.repo.db().clone();
        let limit_i64 = limit as i64;
        let date_modifier = format!("-{lookback_days} days");
        let rows: Vec<(String, String, String, String)> = db.with_connection(|conn| {
            let sql = format!(
                "SELECT id, session_id, agent_id, COALESCE(task_summary, '')
                 FROM session_episodes
                 WHERE outcome = 'success'
                   AND task_summary IS NOT NULL
                   AND created_at > datetime('now', '{date_modifier}')
                 ORDER BY created_at DESC
                 LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let collected: Vec<(String, String, String, String)> = stmt
                .query_map(params![limit_i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(collected)
        })?;

        let mut out: Vec<SuccessfulEpisode> = Vec::with_capacity(rows.len());
        for (id, session_id, agent_id, task_summary) in rows {
            let embedding = self.repo.get_episode_embedding(&id).unwrap_or(None);
            out.push(SuccessfulEpisode {
                id,
                session_id,
                agent_id,
                task_summary,
                embedding,
            });
        }
        Ok(out)
    }

    async fn task_summaries_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<String>, String> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.repo.db().clone();
        let placeholders = vec!["?"; session_ids.len()].join(",");
        let sql = format!(
            "SELECT task_summary FROM session_episodes
             WHERE session_id IN ({placeholders}) AND task_summary IS NOT NULL"
        );
        let session_ids = session_ids.to_vec();
        db.with_connection(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let params_vec: Vec<&dyn rusqlite::types::ToSql> = session_ids
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows: Vec<String> = stmt
                .query_map(params_vec.as_slice(), |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}
