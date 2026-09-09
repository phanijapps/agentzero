//! Previous-episode chain adapter.
//!
//! When a new session starts inside a ward that has prior successful or partial
//! session episodes, we inject the most recent 3 as [`ScoredItem`]s into the
//! unified recall pool so the agent can continue the chain of work rather than
//! starting cold. This is the Memory v2 Phase 6 "episode chain" wiring.
//!
//! Failed episodes with recorded learnings join the pool as an avoid-list:
//! the agent sees what already failed in this ward and why, so it doesn't
//! repeat a known-bad approach.
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

    /// Fetch up to 3 prior successful/partial episodes for `ward_id`
    /// (most recent first) plus up to 2 failed episodes with recorded
    /// learnings, and return them as [`ScoredItem`]s — chain items with
    /// `kind = Episode`, avoid items with `kind = Episode` and an
    /// `[AVOID]`-prefixed content.
    ///
    /// The per-item score is `1.0 / (rank + 1)` — i.e. `1.0, 0.5, 0.333…`
    /// for 3 results. RRF later re-ranks these against the other pools.
    /// Avoid items start one rank down so chain context outranks failures.
    pub async fn fetch(&self, agent_id: &str, ward_id: &str) -> Result<Vec<ScoredItem>, String> {
        let episodes = self
            .store
            .fetch_recent_successful_by_ward(ward_id, 3)
            .await?;
        let mut items: Vec<ScoredItem> = episodes
            .iter()
            .filter(|episode| episode.agent_id == agent_id)
            .enumerate()
            .map(|(rank, ep)| episode_to_item(ep, rank))
            .collect();

        let failed = self.store.fetch_recent_failed_by_ward(ward_id, 2).await?;
        let base = items.len();
        items.extend(
            failed
                .iter()
                .filter(|episode| episode.agent_id == agent_id)
                .filter(|episode| {
                    episode
                        .key_learnings
                        .as_deref()
                        .map(|l| !l.trim().is_empty())
                        .unwrap_or(false)
                })
                .enumerate()
                .map(|(rank, ep)| failed_episode_to_item(ep, base + rank)),
        );

        Ok(items)
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
            // This adapter has already selected a record by authenticated
            // agent and ward. The originating session is audit provenance,
            // not a permission boundary for a durable episode summary.
            session_id: Some("__global__".to_string()),
            ward_id: Some(ep.ward_id.clone()),
        },
        route_hint: Some(
            RouteHint::new(ep.ward_id.clone(), RouteSourceKind::Episode)
                .with_memory_id(ep.id.clone())
                .with_session_id(Some(ep.session_id.clone())),
        ),
    }
}

/// Project a failed [`SessionEpisode`] into an avoid-list [`ScoredItem`].
///
/// Content is prefixed `[AVOID]` and leads with the learnings — the point
/// is "this failed; don't do it again" — so downstream prompt assembly
/// can style avoid items distinctly from chain items.
pub fn failed_episode_to_item(ep: &SessionEpisode, rank: usize) -> ScoredItem {
    let learnings = ep.key_learnings.as_deref().unwrap_or("").trim();
    let content = format!(
        "[AVOID] [{}, {}] {} — this approach failed. Learnings: {}",
        ep.outcome, ep.created_at, ep.task_summary, learnings
    );
    let rank_one = (rank as f64) + 1.0;
    let score = 1.0 / (rank_one + 1.0);
    ScoredItem {
        kind: ItemKind::Episode,
        id: format!("{}#avoid", ep.id),
        content,
        score,
        provenance: Provenance {
            source: "session_episodes".to_string(),
            source_id: ep.id.clone(),
            session_id: Some("__global__".to_string()),
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
        assert_eq!(item.provenance.session_id.as_deref(), Some("__global__"));
    }

    #[test]
    fn failed_episode_to_item_marks_avoid_and_leads_with_learnings() {
        let ep = SessionEpisode {
            id: "ep-f".into(),
            session_id: "s-f".into(),
            agent_id: "a".into(),
            ward_id: "finance".into(),
            task_summary: "pull earnings data".into(),
            outcome: "failed".into(),
            strategy_used: None,
            key_learnings: Some("API rejects bulk ranges; fetch per-ticker".into()),
            token_cost: None,
            embedding: None,
            created_at: "2026-04-01T00:00:00Z".into(),
        };
        let item = failed_episode_to_item(&ep, 0);
        assert_eq!(item.id, "ep-f#avoid");
        assert!(item.content.starts_with("[AVOID]"));
        assert!(item.content.contains("pull earnings data"));
        assert!(item.content.contains("API rejects bulk ranges"));
        assert!((item.score - 0.5).abs() < 1e-9);
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
        // 1 failed with learnings — surfaces as an avoid item.
        insert_ep(&repo, "ep-fail", "finance", "failed", &now_offset_days(1));

        let adapter = PreviousEpisodesAdapter::new(store);
        let items = adapter.fetch("agent-a", "finance").await.expect("fetch");

        assert_eq!(items.len(), 4, "3 chain + 1 avoid");
        assert_eq!(items[0].id, "ep-new", "newest first");
        assert_eq!(items[1].id, "ep-mid");
        assert_eq!(items[2].id, "ep-old");
        let avoid = &items[3];
        assert_eq!(avoid.id, "ep-fail#avoid");
        assert!(avoid.content.starts_with("[AVOID]"));
        assert!(avoid.content.contains("Learnings: learn-ep-fail"));

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
        let items = adapter.fetch("agent-a", "ghost-ward").await.expect("fetch");
        assert!(items.is_empty());
    }
}
