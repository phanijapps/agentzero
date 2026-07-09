//! Previous-episode chain adapter.
//!
//! When a new session starts inside a ward that has prior successful or partial
//! session episodes, we inject the most recent 3 as [`ScoredItem`]s into the
//! unified recall pool so the agent can continue the chain of work rather than
//! starting cold. This is the Memory v2 Phase 6 "episode chain" wiring.
//!
//! Phase E6c: backend-agnostic — takes `Arc<dyn EpisodeStore>` so the
//! same recall path works on the configured backend.

use crate::recall::scored_item::{ItemKind, Provenance, ScoredItem};
use std::sync::Arc;
use zbot_stores_domain::{RouteHint, RouteSourceKind};
use zbot_stores_traits::{EpisodeStore, SessionEpisode};

/// Adapter that projects a ward's recent successful/partial episodes into
/// [`ScoredItem`]s suitable for [`rrf_merge`](crate::recall::rrf_merge).
pub struct PreviousEpisodesAdapter {
    store: Arc<dyn EpisodeStore>,
}

impl PreviousEpisodesAdapter {
    /// Create a new adapter wired to the given episode store.
    pub fn new(store: Arc<dyn EpisodeStore>) -> Self {
        Self { store }
    }

    /// Fetch up to 3 prior episodes for `ward_id` (most recent first) and
    /// return them as [`ScoredItem`]s with `kind = Episode`.
    ///
    /// The per-item score is `1.0 / (rank + 1)` — i.e. `1.0, 0.5, 0.333…`
    /// for 3 results. RRF later re-ranks these against the other pools.
    pub async fn fetch(&self, ward_id: &str) -> Result<Vec<ScoredItem>, String> {
        let episodes = self
            .store
            .fetch_recent_successful_by_ward(ward_id, 3)
            .await?;
        Ok(episodes
            .iter()
            .enumerate()
            .map(|(rank, ep)| episode_to_item(ep, rank))
            .collect())
    }
}

/// Project a [`SessionEpisode`] into a [`ScoredItem`] with rank-based score.
pub fn episode_to_item(ep: &SessionEpisode, rank: usize) -> ScoredItem {
    let rank_one = (rank as f64) + 1.0;
    let score = 1.0 / (rank_one + 1.0);
    let mut content = format!("[{}, {}] {}", ep.outcome, ep.created_at, ep.task_summary);
    if let Some(learnings) = ep.key_learnings.as_ref() {
        if !learnings.is_empty() {
            content.push_str("\nLearnings: ");
            content.push_str(learnings);
        }
    }
    ScoredItem {
        kind: ItemKind::Episode,
        id: ep.id.clone(),
        content,
        score,
        provenance: Provenance {
            source: "session_episodes".to_string(),
            source_id: ep.id.clone(),
            session_id: Some(ep.session_id.clone()),
            ward_id: Some(ep.ward_id.clone()),
        },
        route_hint: Some(
            RouteHint::new(ep.ward_id.clone(), RouteSourceKind::Episode)
                .with_memory_id(ep.id.clone())
                .with_session_id(Some(ep.session_id.clone())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recall::scored_item::ItemKind;
    use gateway_services::VaultPaths;
    use zbot_stores_sqlite::{
        EpisodeRepository, GatewayEpisodeStore, KnowledgeDatabase, SqliteVecIndex,
    };

    fn setup() -> (
        tempfile::TempDir,
        Arc<EpisodeRepository>,
        Arc<dyn EpisodeStore>,
    ) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let vec_index = Arc::new(
            SqliteVecIndex::new(db.clone(), "session_episodes_index", "episode_id")
                .expect("vec index init"),
        );
        let repo = Arc::new(EpisodeRepository::new(db, vec_index));
        let store: Arc<dyn EpisodeStore> = Arc::new(GatewayEpisodeStore::new(repo.clone()));
        (tmp, repo, store)
    }

    fn insert_ep(repo: &EpisodeRepository, id: &str, ward: &str, outcome: &str, created_at: &str) {
        let ep = SessionEpisode {
            id: id.to_string(),
            session_id: format!("sess-{id}"),
            agent_id: "agent-a".to_string(),
            ward_id: ward.to_string(),
            task_summary: format!("task for {id}"),
            outcome: outcome.to_string(),
            strategy_used: None,
            key_learnings: Some(format!("learn-{id}")),
            token_cost: None,
            embedding: None,
            created_at: created_at.to_string(),
        };
        repo.insert(&ep).expect("insert");
    }

    fn now_offset_days(days: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
    }

    #[test]
    fn episode_to_item_formats_content_and_score() {
        let ep = SessionEpisode {
            id: "ep-x".into(),
            session_id: "s-x".into(),
            agent_id: "a".into(),
            ward_id: "finance".into(),
            task_summary: "summarize Q3".into(),
            outcome: "success".into(),
            strategy_used: None,
            key_learnings: Some("use the docs".into()),
            token_cost: None,
            embedding: None,
            created_at: "2026-04-01T00:00:00Z".into(),
        };
        let item = episode_to_item(&ep, 0);
        assert_eq!(item.kind, ItemKind::Episode);
        assert_eq!(item.id, "ep-x");
        assert!(item.content.contains("success"));
        assert!(item.content.contains("summarize Q3"));
        assert!(item.content.contains("Learnings: use the docs"));
        assert!((item.score - 0.5).abs() < 1e-9, "rank 0 → 1/2");
        assert_eq!(item.provenance.source, "session_episodes");
        assert_eq!(item.provenance.ward_id.as_deref(), Some("finance"));
        assert_eq!(item.provenance.session_id.as_deref(), Some("s-x"));
    }

    #[tokio::test]
    async fn fetch_returns_top3_newest_first_filtered_by_ward_and_window() {
        let (_tmp, repo, store) = setup();
        // 3 successful in ward, created oldest → newest.
        insert_ep(&repo, "ep-old", "finance", "success", &now_offset_days(10));
        insert_ep(&repo, "ep-mid", "finance", "partial", &now_offset_days(5));
        insert_ep(&repo, "ep-new", "finance", "success", &now_offset_days(1));
        // 1 outside the 14-day window.
        insert_ep(
            &repo,
            "ep-stale",
            "finance",
            "success",
            &now_offset_days(30),
        );
        // 1 in a different ward.
        insert_ep(&repo, "ep-other", "hr", "success", &now_offset_days(1));
        // 1 failed — excluded.
        insert_ep(&repo, "ep-fail", "finance", "failed", &now_offset_days(1));

        let adapter = PreviousEpisodesAdapter::new(store);
        let items = adapter.fetch("finance").await.expect("fetch");

        assert_eq!(items.len(), 3, "exactly 3 in-window finance ep/partial");
        assert_eq!(items[0].id, "ep-new", "newest first");
        assert_eq!(items[1].id, "ep-mid");
        assert_eq!(items[2].id, "ep-old");

        // Scores are 1/(rank+1): rank 0 → 1/2, rank 1 → 1/3, rank 2 → 1/4.
        assert!((items[0].score - 0.5).abs() < 1e-9);
        assert!((items[1].score - (1.0 / 3.0)).abs() < 1e-9);
        assert!((items[2].score - 0.25).abs() < 1e-9);
        for item in &items {
            assert_eq!(item.kind, ItemKind::Episode);
        }
    }

    #[tokio::test]
    async fn fetch_empty_when_ward_has_no_episodes() {
        let (_tmp, _repo, store) = setup();
        let adapter = PreviousEpisodesAdapter::new(store);
        let items = adapter.fetch("ghost-ward").await.expect("fetch");
        assert!(items.is_empty());
    }
}
