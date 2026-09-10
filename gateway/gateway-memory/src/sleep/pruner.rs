//! Pruner — soft-deletes orphan candidates via the `__pruned__` sentinel on
//! `kg_entities.compressed_into`, then records each prune in `kg_compactions`.
//!
//! Keeping the row (rather than hard-deleting) preserves referential
//! integrity with episodes and distillations that may still point at the
//! entity id. The Archiver can hard-delete later.

use std::sync::Arc;

use knowledge_graph::kg_trait::kg_types::EntityId;
use knowledge_graph::kg_trait::KnowledgeGraphStore;
use zbot_stores_traits::CompactionStore;

use crate::sleep::decay::PruneCandidate;

/// Counts emitted from a single prune pass.
#[derive(Debug, Default, Clone)]
pub struct PruneStats {
    pub pruned: u64,
    pub failed: u64,
}

/// Soft-deletes the candidates produced by `DecayEngine`.
///
/// Phase D3: trait-routed. Both `kg_store` and `compaction_store`
/// abstract over the backend so the prune cycle runs.
pub struct Pruner {
    kg_store: Arc<dyn KnowledgeGraphStore>,
    compaction_store: Arc<dyn CompactionStore>,
}

impl Pruner {
    pub fn new(
        kg_store: Arc<dyn KnowledgeGraphStore>,
        compaction_store: Arc<dyn CompactionStore>,
    ) -> Self {
        Self {
            kg_store,
            compaction_store,
        }
    }

    /// Soft-delete every candidate and log each outcome under `run_id`.
    pub async fn prune(&self, run_id: &str, candidates: &[PruneCandidate]) -> PruneStats {
        let mut stats = PruneStats::default();
        for c in candidates {
            let eid = EntityId::from(c.entity_id.clone());
            match self.kg_store.mark_entity_pruned(&eid).await {
                Ok(()) => {
                    stats.pruned += 1;
                    if let Err(e) = self
                        .compaction_store
                        .record_prune(run_id, Some(&c.entity_id), None, &c.reason)
                        .await
                    {
                        tracing::warn!(
                            entity = %c.entity_id,
                            error = %e,
                            "record_prune failed"
                        );
                    }
                }
                Err(e) => {
                    stats.failed += 1;
                    tracing::warn!(entity = %c.entity_id, error = %e, "mark_entity_pruned failed");
                }
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::decay::{DecayConfig, DecayEngine};
    use crate::sleep::test_support;
    use knowledge_graph::kg_trait::ExtractedKnowledge;
    use knowledge_graph::{Entity, EntityType};

    #[tokio::test]
    async fn pruner_soft_deletes_orphan_and_records_audit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let agent_id = "agent-prune";

        // One orphan, old entity.
        let mut orphan = Entity::new(
            agent_id.to_string(),
            EntityType::Concept,
            "Abandoned Concept".to_string(),
        );
        orphan.last_seen_at = chrono::Utc::now() - chrono::Duration::days(90);
        orphan.first_seen_at = orphan.last_seen_at;
        let orphan_id = orphan.id.clone();

        let kg_store = test_support::kg_store(&tmp);
        kg_store
            .store_knowledge(
                agent_id,
                ExtractedKnowledge {
                    entities: vec![orphan],
                    relationships: vec![],
                },
            )
            .await
            .expect("store");

        let engine = DecayEngine::new(
            kg_store.clone(),
            DecayConfig {
                min_age_days: 30,
                limit: 100,
            },
        );
        let candidates = engine.list_prune_candidates(agent_id).await;
        assert!(
            !candidates.is_empty(),
            "decay engine must produce a candidate"
        );

        let compaction_store = test_support::compaction_store(&tmp);
        let pruner = Pruner::new(kg_store.clone(), compaction_store);
        let stats = pruner.prune("run-prune-test", &candidates).await;

        assert!(stats.pruned >= 1, "expected prunes, got {stats:?}");
        assert_eq!(stats.failed, 0);

        // Behavioral prune verification (matches the KG-maintenance
        // conformance contract): the pruned entity no longer resolves and
        // leaves prune candidacy.
        let resolved = kg_store
            .resolve_entity(agent_id, &EntityType::Concept, "Abandoned Concept", None)
            .await
            .expect("resolve after prune");
        assert!(
            matches!(resolved, knowledge_graph::kg_trait::ResolveOutcome::NoMatch),
            "pruned entity must not resolve"
        );
        let after = engine.list_prune_candidates(agent_id).await;
        assert!(
            after.iter().all(|c| c.entity_id != orphan_id),
            "pruned entity leaves prune candidacy"
        );
    }
}
