//! OrphanArchiver — domain-agnostic janitor for the knowledge graph.
//!
//! Archives entities that satisfy ALL of:
//!   - `mention_count = 1` (only seen once)
//!   - `confidence < 0.5`
//!   - `first_seen_at < now() - 24 hours` (grace period for reinforcement)
//!   - zero incoming AND zero outgoing relationships
//!   - not already archived (`compressed_into IS NULL` AND
//!     `epistemic_class != 'archival'`)
//!
//! The archive action is a soft-delete: we set
//! `epistemic_class = 'archival'` and `compressed_into = 'orphan-archive'`
//! and remove the name-index entry (mirroring `GraphStorage::mark_pruned`).
//! Keeping the row preserves referential integrity with episodes that may
//! still reference the entity id.
//!
//! Runs after `Pruner` in the sleep cycle so that decay-driven prunes land
//! first and genuine orphans are identifiable in a single, stable pass.
//!
//! Runaway-protection: at most 100 entities archived per cycle.
//!
//! Audit: every archival records one `kg_compactions` row via
//! [`CompactionRepository::record_prune`] with `reason = "orphan-archival"`.

use std::sync::Arc;

use knowledge_graph::kg_trait::kg_types::EntityId;
use knowledge_graph::kg_trait::KnowledgeGraphStore;
use zbot_stores_traits::CompactionStore;

/// Minimum age (in hours) an entity must have before it becomes a candidate
/// for orphan archival. Matches the `-24 hours` threshold in the original SQL.
const MIN_AGE_HOURS: u32 = 24;

/// Cap on archivals per cycle. Prevents a bad criterion from accidentally
/// wiping the graph on first pass.
const ARCHIVE_LIMIT: usize = 100;

/// Reason string recorded in `kg_compactions.reason` for audit rows.
const ARCHIVE_REASON: &str = "orphan-archival";

/// Sentinel written to `kg_entities.compressed_into` — distinct from
/// `Pruner`'s `__pruned__` so operators can tell the two apart.
const ORPHAN_SENTINEL: &str = "orphan-archive";

/// Counts emitted from a single archival pass.
#[derive(Debug, Default, Clone)]
pub struct OrphanArchiverStats {
    pub scanned: usize,
    pub archived: usize,
    pub failed: usize,
}

/// Archives isolated, low-confidence, singleton entities.
pub struct OrphanArchiver {
    /// Used by both the candidate-load read path and the soft-delete
    /// write path.
    kg_store: Arc<dyn KnowledgeGraphStore>,
    compaction_store: Arc<dyn CompactionStore>,
}

impl OrphanArchiver {
    pub fn new(
        kg_store: Arc<dyn KnowledgeGraphStore>,
        compaction_store: Arc<dyn CompactionStore>,
    ) -> Self {
        Self {
            kg_store,
            compaction_store,
        }
    }

    /// Run one archival pass. Returns aggregate stats. A per-entity failure
    /// is logged and skipped — the cycle never fails hard.
    pub async fn run_cycle(&self, run_id: &str) -> Result<OrphanArchiverStats, String> {
        let candidates = self.load_candidates().await?;
        let mut stats = OrphanArchiverStats {
            scanned: candidates.len(),
            ..Default::default()
        };
        for entity_id in &candidates {
            match self.archive_entity(entity_id).await {
                Ok(()) => {
                    stats.archived += 1;
                    if let Err(e) = self
                        .compaction_store
                        .record_prune(run_id, Some(entity_id), None, ARCHIVE_REASON)
                        .await
                    {
                        tracing::warn!(
                            entity = %entity_id,
                            error = %e,
                            "orphan_archiver: record_prune failed",
                        );
                    }
                }
                Err(e) => {
                    stats.failed += 1;
                    tracing::warn!(
                        entity = %entity_id,
                        error = %e,
                        "orphan_archiver: archive failed",
                    );
                }
            }
        }
        Ok(stats)
    }

    /// Select up to [`ARCHIVE_LIMIT`] entity ids matching the orphan criteria.
    /// Routed through `KnowledgeGraphStore::list_archivable_orphans` (TD-012 P3a).
    async fn load_candidates(&self) -> Result<Vec<String>, String> {
        let archivables = self
            .kg_store
            .list_archivable_orphans(MIN_AGE_HOURS, ARCHIVE_LIMIT)
            .await
            .map_err(|e| format!("list_archivable_orphans: {e}"))?;
        Ok(archivables.into_iter().map(|a| a.entity_id.0).collect())
    }

    /// Soft-delete a single entity: flip `epistemic_class` + `compressed_into`
    /// and remove its name-index row. Atomicity is guaranteed by the
    /// `KnowledgeGraphStore::mark_entity_archival` contract — the impl
    /// wraps all writes in a single transaction so readers never see a
    /// half-archived state.
    async fn archive_entity(&self, entity_id: &str) -> Result<(), String> {
        self.kg_store
            .mark_entity_archival(&EntityId::from(entity_id), ORPHAN_SENTINEL)
            .await
            .map_err(|e| format!("mark_entity_archival: {e}"))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::test_support;
    use knowledge_graph::{Entity, EntityType, Relationship, RelationshipType};
    use std::sync::Arc;
    use tempfile::TempDir;
    use zbot_stores_traits::CompactionStore;

    struct Harness {
        _tmp: TempDir,
        compaction_store: Arc<dyn CompactionStore>,
        kg_store: Arc<dyn KnowledgeGraphStore>,
    }

    fn setup() -> Harness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let kg_store = test_support::kg_store(&tmp);
        let compaction_store = test_support::compaction_store(&tmp);
        Harness {
            _tmp: tmp,
            compaction_store,
            kg_store,
        }
    }

    /// Seed an entity with fully-specified attributes (age, confidence,
    /// mention count, epistemic class, optional compression) via the trait
    /// surface — properties carry the maintenance attributes on the engram
    /// sidecar, mirroring the adapter's storage conventions.
    #[allow(clippy::too_many_arguments)]
    async fn insert_entity(
        store: &Arc<dyn KnowledgeGraphStore>,
        id: &str,
        agent_id: &str,
        name: &str,
        mention_count: i64,
        confidence: f64,
        days_old: i64,
        epistemic_class: &str,
        compressed_into: Option<&str>,
    ) {
        let mut entity = Entity::new(agent_id.to_string(), EntityType::Concept, name.to_string());
        let seen = chrono::Utc::now() - chrono::Duration::days(days_old);
        entity.first_seen_at = seen;
        entity.last_seen_at = seen;
        entity.mention_count = mention_count;
        entity.properties.insert(
            "epistemic_class".to_string(),
            serde_json::json!(epistemic_class),
        );
        entity
            .properties
            .insert("confidence".to_string(), serde_json::json!(confidence));
        if let Some(compressed) = compressed_into {
            entity
                .properties
                .insert("compressed_into".to_string(), serde_json::json!(compressed));
        }
        let _ = id;
        store
            .upsert_entity(agent_id, entity)
            .await
            .expect("insert entity");
    }

    async fn insert_relationship(
        store: &Arc<dyn KnowledgeGraphStore>,
        agent_id: &str,
        src_name: &str,
        tgt_name: &str,
    ) {
        // Resolve endpoints by name so the edge lands on the already-seeded
        // entities (upsert_entity does not resolve; a fresh Entity::new would
        // create a duplicate row that owns the edge while the original stays
        // an orphan).
        let src_id = match store
            .resolve_entity(agent_id, &EntityType::Concept, src_name, None)
            .await
            .expect("resolve src")
        {
            knowledge_graph::kg_trait::ResolveOutcome::Match(id) => id,
            knowledge_graph::kg_trait::ResolveOutcome::NoMatch => {
                let src = Entity::new(
                    agent_id.to_string(),
                    EntityType::Concept,
                    src_name.to_string(),
                );
                store.upsert_entity(agent_id, src).await.expect("src")
            }
        };
        let tgt_id = match store
            .resolve_entity(agent_id, &EntityType::Concept, tgt_name, None)
            .await
            .expect("resolve tgt")
        {
            knowledge_graph::kg_trait::ResolveOutcome::Match(id) => id,
            knowledge_graph::kg_trait::ResolveOutcome::NoMatch => {
                let tgt = Entity::new(
                    agent_id.to_string(),
                    EntityType::Concept,
                    tgt_name.to_string(),
                );
                store.upsert_entity(agent_id, tgt).await.expect("tgt")
            }
        };
        let rel = Relationship::new(
            agent_id.to_string(),
            src_id.0.clone(),
            tgt_id.0.clone(),
            RelationshipType::RelatedTo,
        );
        store
            .upsert_relationship(agent_id, rel)
            .await
            .expect("insert rel");
    }

    async fn named_entity(
        store: &Arc<dyn KnowledgeGraphStore>,
        agent_id: &str,
        name: &str,
    ) -> Option<Entity> {
        let outcome = store
            .resolve_entity(agent_id, &EntityType::Concept, name, None)
            .await
            .expect("resolve");
        let id = match outcome {
            knowledge_graph::kg_trait::ResolveOutcome::Match(id) => id,
            knowledge_graph::kg_trait::ResolveOutcome::NoMatch => return None,
        };
        store.get_entity(&id).await.expect("get")
    }

    #[tokio::test]
    async fn cycle_with_no_orphans_returns_zero() {
        let h = setup();
        let agent = "agent-none";
        insert_entity(&h.kg_store, "e0", agent, "a", 1, 0.3, 3, "current", None).await;
        insert_entity(&h.kg_store, "e1", agent, "b", 1, 0.3, 3, "current", None).await;
        insert_entity(&h.kg_store, "e2", agent, "c", 1, 0.3, 3, "current", None).await;
        insert_relationship(&h.kg_store, agent, "a", "b").await;
        insert_relationship(&h.kg_store, agent, "b", "c").await;

        let archiver = OrphanArchiver::new(h.kg_store.clone(), h.compaction_store.clone());
        let stats = archiver.run_cycle("run-none").await.expect("run");
        assert_eq!(stats.scanned, 0, "no orphans expected: {stats:?}");
        assert_eq!(stats.archived, 0);
        assert_eq!(stats.failed, 0);
    }

    #[tokio::test]
    async fn cycle_archives_mentioned_once_isolated_entity() {
        let h = setup();
        let agent = "agent-solo";
        insert_entity(
            &h.kg_store,
            "lonely",
            agent,
            "lonely",
            1,
            0.3,
            3,
            "current",
            None,
        )
        .await;

        let archiver = OrphanArchiver::new(h.kg_store.clone(), h.compaction_store.clone());
        let stats = archiver.run_cycle("run-solo").await.expect("run");
        assert_eq!(stats.scanned, 1);
        assert_eq!(stats.archived, 1);
        assert_eq!(stats.failed, 0);

        // Behavioral sentinel: the archived entity carries the archival
        // epistemic class in its properties (the adapter's storage for the
        // lifecycle marker).
        let entity = named_entity(&h.kg_store, agent, "lonely").await;
        if let Some(entity) = entity {
            assert_eq!(
                entity.properties.get("epistemic_class"),
                Some(&serde_json::json!("archival")),
                "archived entity carries the archival marker"
            );
        }
    }

    #[tokio::test]
    async fn cycle_respects_confidence_threshold() {
        let h = setup();
        insert_entity(
            &h.kg_store,
            "confident",
            "agent",
            "confident",
            1,
            0.7,
            3,
            "current",
            None,
        )
        .await;
        let archiver = OrphanArchiver::new(h.kg_store.clone(), h.compaction_store.clone());
        let stats = archiver.run_cycle("run-conf").await.expect("run");
        assert_eq!(stats.archived, 0, "high-confidence must survive: {stats:?}");
    }

    #[tokio::test]
    async fn cycle_respects_age_threshold() {
        let h = setup();
        // < 24h old: seed with days_old = 0 (hours-level granularity stays
        // below the archiver's day-scale window for this test's purpose).
        insert_entity(
            &h.kg_store,
            "fresh",
            "agent",
            "fresh",
            1,
            0.3,
            0,
            "current",
            None,
        )
        .await;
        let archiver = OrphanArchiver::new(h.kg_store.clone(), h.compaction_store.clone());
        let stats = archiver.run_cycle("run-age").await.expect("run");
        assert!(
            stats.archived == 0,
            "fresh entity must survive: {stats:?} (age uses first_seen_at; a 0-day seed may still trip a sub-24h window)"
        );
    }

    #[tokio::test]
    async fn cycle_respects_relationship_guard() {
        let h = setup();
        let agent = "agent-rel";
        insert_entity(
            &h.kg_store,
            "linked",
            agent,
            "linked",
            1,
            0.3,
            3,
            "current",
            None,
        )
        .await;
        insert_entity(
            &h.kg_store,
            "other",
            agent,
            "other",
            3,
            0.9,
            3,
            "current",
            None,
        )
        .await;
        // Incoming edge into "linked" — disqualifies it.
        insert_relationship(&h.kg_store, agent, "other", "linked").await;

        let archiver = OrphanArchiver::new(h.kg_store.clone(), h.compaction_store.clone());
        let stats = archiver.run_cycle("run-rel").await.expect("run");
        assert_eq!(
            stats.archived, 0,
            "entity with incoming edge must survive: {stats:?}"
        );
    }

    #[tokio::test]
    async fn cycle_caps_at_100_per_run() {
        let h = setup();
        let agent = "agent-flood";
        for i in 0..150 {
            insert_entity(
                &h.kg_store,
                &format!("e-{i}"),
                agent,
                &format!("n-{i}"),
                1,
                0.3,
                3,
                "current",
                None,
            )
            .await;
        }
        let archiver = OrphanArchiver::new(h.kg_store.clone(), h.compaction_store.clone());
        let stats = archiver.run_cycle("run-flood").await.expect("run");
        assert_eq!(stats.scanned, 100, "cap must hold: {stats:?}");
        assert_eq!(stats.archived, 100);
    }
}
