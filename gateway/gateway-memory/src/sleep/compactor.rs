//! Compactor — merges near-duplicate entities (same type, cosine >= threshold).
//!
//! Pipeline per entity type:
//!   find_duplicate_candidates -> optional LLM verify -> merge_entity_into ->
//!   CompactionRepository::record_merge.
//!
//! Archival and already-compressed entities are filtered out at
//! `find_duplicate_candidates`, so no extra guard is needed here.

use async_trait::async_trait;
use std::sync::Arc;

use knowledge_graph::{Entity, EntityType};
use zbot_stores::types::EntityId;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_traits::CompactionStore;

/// Default cosine threshold for considering two entities near-duplicates.
const DEFAULT_COSINE_THRESHOLD: f32 = 0.92;
/// Default per-type limit on candidate pairs.
const DEFAULT_PER_TYPE_LIMIT: usize = 50;

/// Counts emitted from a single compaction run.
#[derive(Debug, Default, Clone)]
pub struct CompactionStats {
    /// Total candidate pairs considered across all entity types.
    pub candidates_considered: u64,
    /// Merges successfully committed.
    pub merges_performed: u64,
    /// Pairs rejected by the optional LLM verifier.
    pub merges_skipped_by_verifier: u64,
}

/// Optional LLM-based pairwise verifier. Phase 4 leaves this off by default —
/// threshold-only merges are conservative enough for background operation.
#[async_trait]
pub trait PairwiseVerifier: Send + Sync {
    async fn should_merge(&self, a: &Entity, b: &Entity) -> bool;
}

/// Entity types that see the most duplicates and are worth scanning hourly.
const DEFAULT_TYPES: &[EntityType] = &[
    EntityType::Person,
    EntityType::Organization,
    EntityType::Location,
    EntityType::Event,
    EntityType::Concept,
];

/// Threshold-based duplicate merger.
///
/// Phase D3: trait-routed. Both `kg_store` and `compaction_store` are
/// Arc<dyn ...> so the compactor runs on both SQLite (and any future alternate backend) —
/// each backend implements the underlying queries with its own
/// primitives (SQL transactions on SQLite, RELATE delete+recreate on
/// Surreal).
pub struct Compactor {
    kg_store: Arc<dyn KnowledgeGraphStore>,
    compaction_store: Arc<dyn CompactionStore>,
    verifier: Option<Arc<dyn PairwiseVerifier>>,
    cosine_threshold: f32,
    per_type_limit: usize,
}

impl Compactor {
    /// Build a compactor with default cosine / limit settings.
    pub fn new(
        kg_store: Arc<dyn KnowledgeGraphStore>,
        compaction_store: Arc<dyn CompactionStore>,
        verifier: Option<Arc<dyn PairwiseVerifier>>,
    ) -> Self {
        Self {
            kg_store,
            compaction_store,
            verifier,
            cosine_threshold: DEFAULT_COSINE_THRESHOLD,
            per_type_limit: DEFAULT_PER_TYPE_LIMIT,
        }
    }

    /// Override the cosine threshold (default: 0.92).
    pub fn with_cosine_threshold(mut self, threshold: f32) -> Self {
        self.cosine_threshold = threshold;
        self
    }

    /// Override the per-type candidate pair limit (default: 50).
    pub fn with_per_type_limit(mut self, limit: usize) -> Self {
        self.per_type_limit = limit;
        self
    }

    /// Run one compaction pass for `agent_id`, recording every merge under `run_id`.
    pub async fn run(&self, run_id: &str, agent_id: &str) -> CompactionStats {
        let mut stats = CompactionStats::default();

        for ty in DEFAULT_TYPES {
            self.run_for_type(run_id, agent_id, ty, &mut stats).await;
        }

        stats
    }

    async fn run_for_type(
        &self,
        run_id: &str,
        agent_id: &str,
        ty: &EntityType,
        stats: &mut CompactionStats,
    ) {
        let candidates = match self
            .kg_store
            .find_duplicate_candidates(agent_id, ty, self.cosine_threshold, self.per_type_limit)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(entity_type = ty.as_str(), error = %e, "find_duplicate_candidates failed");
                return;
            }
        };

        if candidates.is_empty() {
            return;
        }

        // Load entities for this agent once per type; the verifier needs
        // them and pick_loser_winner reads mention_count. Use the trait's
        // list_entities with a generous limit (compactor scans are
        // small per-type by design).
        let entities = self
            .kg_store
            .list_entities(agent_id, Some(ty.as_str()), 1_000, 0)
            .await
            .unwrap_or_default();

        for cand in candidates {
            stats.candidates_considered += 1;
            self.process_pair(
                run_id,
                &entities,
                cand.loser_entity_id,
                cand.winner_entity_id,
                cand.cosine_similarity,
                stats,
            )
            .await;
        }
    }

    async fn process_pair(
        &self,
        run_id: &str,
        entities: &[Entity],
        a_id: String,
        b_id: String,
        cosine: f32,
        stats: &mut CompactionStats,
    ) {
        if let Some(verifier) = &self.verifier {
            let a = entities.iter().find(|e| e.id == a_id);
            let b = entities.iter().find(|e| e.id == b_id);
            let (Some(a), Some(b)) = (a, b) else {
                stats.merges_skipped_by_verifier += 1;
                tracing::warn!(
                    candidate_a = %a_id,
                    candidate_b = %b_id,
                    "compactor verifier skipped merge because candidate entity data was unavailable"
                );
                return;
            };
            if !verifier.should_merge(a, b).await {
                stats.merges_skipped_by_verifier += 1;
                return;
            }
        }

        let (loser, winner) = pick_loser_winner(entities, &a_id, &b_id);

        let loser_eid = EntityId::from(loser.clone());
        let winner_eid = EntityId::from(winner.clone());
        match self
            .kg_store
            .merge_entity_into(&loser_eid, &winner_eid)
            .await
        {
            Ok(()) => {
                stats.merges_performed += 1;
                let reason = format!("cosine={cosine:.2}");
                if let Err(e) = self
                    .compaction_store
                    .record_merge(run_id, &loser, &winner, &reason)
                    .await
                {
                    tracing::warn!(
                        loser = %loser,
                        winner = %winner,
                        error = %e,
                        "record_merge failed"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    loser = %loser,
                    winner = %winner,
                    error = %e,
                    "merge_entity_into failed"
                );
            }
        }
    }
}

/// Loser = entity with smaller mention_count. Ties resolved by picking `b`
/// as the loser (arbitrary but deterministic).
fn pick_loser_winner(entities: &[Entity], a: &str, b: &str) -> (String, String) {
    let a_mentions = entities
        .iter()
        .find(|e| e.id == a)
        .map(|e| e.mention_count)
        .unwrap_or(0);
    let b_mentions = entities
        .iter()
        .find(|e| e.id == b)
        .map(|e| e.mention_count)
        .unwrap_or(0);
    if a_mentions < b_mentions {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use gateway_services::VaultPaths;
    use knowledge_graph::{Entity, EntityType, ExtractedKnowledge};
    use std::sync::Arc;
    use zbot_stores_sqlite::kg::storage::GraphStorage;
    use zbot_stores_sqlite::{
        CompactionRepository, GatewayCompactionStore, KnowledgeDatabase, SqliteKgStore,
    };

    struct FixedVerifier(bool);

    #[async_trait]
    impl PairwiseVerifier for FixedVerifier {
        async fn should_merge(&self, _a: &Entity, _b: &Entity) -> bool {
            self.0
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        Arc<GraphStorage>,
        Arc<CompactionRepository>,
    ) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let graph = Arc::new(GraphStorage::new(db.clone()).expect("graph"));
        let repo = Arc::new(CompactionRepository::new(db));
        (tmp, graph, repo)
    }

    /// Two entities with identical L2-normalized embeddings -> cosine 1.0,
    /// so they cross any reasonable threshold.
    fn unit_embedding(seed: f32) -> Vec<f32> {
        // kg_name_index uses 384-dim embeddings (matches production encoder).
        let raw: Vec<f32> = (0..384).map(|i| seed + (i as f32) * 0.001).collect();
        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        raw.iter().map(|x| x / norm).collect()
    }

    #[tokio::test]
    async fn compactor_merges_near_duplicates_and_records_audit() {
        let (_tmp, graph, repo) = setup();
        let agent_id = "agent-compact";

        // Two persons with close-but-not-identical embeddings. The resolver
        // dedups at cosine >= 0.87 on store; we stay below that (~0.75) so both
        // entities land, then run the compactor with a lower threshold.
        let emb_a = unit_embedding(0.1);
        let emb_b = unit_embedding(0.9);
        let mut a = Entity::new(
            agent_id.to_string(),
            EntityType::Person,
            "Alice Smith".to_string(),
        );
        a.name_embedding = Some(emb_a);
        a.mention_count = 1;
        let mut b = Entity::new(
            agent_id.to_string(),
            EntityType::Person,
            "Alice S.".to_string(),
        );
        b.name_embedding = Some(emb_b);
        b.mention_count = 5;

        graph
            .store_knowledge(
                agent_id,
                ExtractedKnowledge {
                    entities: vec![a, b],
                    relationships: vec![],
                },
            )
            .expect("store");

        let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph.clone()));
        let compaction_store: Arc<dyn CompactionStore> =
            Arc::new(GatewayCompactionStore::new(repo.clone()));
        let compactor = Compactor::new(kg_store, compaction_store, None)
            .with_cosine_threshold(0.5)
            .with_per_type_limit(10);

        let run_id = "run-compact-test";
        let stats = compactor.run(run_id, agent_id).await;

        assert!(
            stats.candidates_considered >= 1,
            "expected at least one candidate, got {stats:?}"
        );
        assert!(
            stats.merges_performed >= 1,
            "expected at least one merge, got {stats:?}"
        );

        let rows = repo.list_run(run_id).expect("list run");
        assert!(!rows.is_empty(), "kg_compactions should have a merge row");
        assert_eq!(rows[0].operation, "merge");
    }

    #[tokio::test]
    async fn verifier_enabled_skips_merge_when_candidate_entity_is_missing() {
        let (_tmp, graph, repo) = setup();
        let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph));
        let compaction_store: Arc<dyn CompactionStore> =
            Arc::new(GatewayCompactionStore::new(repo));
        let compactor = Compactor::new(
            kg_store,
            compaction_store,
            Some(Arc::new(FixedVerifier(true))),
        );
        let mut stats = CompactionStats::default();

        compactor
            .process_pair(
                "run-missing-entity",
                &[],
                "missing-a".to_string(),
                "missing-b".to_string(),
                0.99,
                &mut stats,
            )
            .await;

        assert_eq!(stats.merges_performed, 0);
        assert_eq!(stats.merges_skipped_by_verifier, 1);
    }

    #[tokio::test]
    async fn verifier_rejection_skips_merge_and_updates_stats() {
        let (_tmp, graph, repo) = setup();
        let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph));
        let compaction_store: Arc<dyn CompactionStore> =
            Arc::new(GatewayCompactionStore::new(repo));
        let compactor = Compactor::new(
            kg_store,
            compaction_store,
            Some(Arc::new(FixedVerifier(false))),
        );
        let agent_id = "agent-verify";
        let entities = vec![
            Entity::new(agent_id.to_string(), EntityType::Person, "A".to_string()),
            Entity::new(agent_id.to_string(), EntityType::Person, "B".to_string()),
        ];
        let mut stats = CompactionStats::default();

        compactor
            .process_pair(
                "run-reject",
                &entities,
                entities[0].id.clone(),
                entities[1].id.clone(),
                0.99,
                &mut stats,
            )
            .await;

        assert_eq!(stats.merges_performed, 0);
        assert_eq!(stats.merges_skipped_by_verifier, 1);
    }
}
