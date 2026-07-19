// ============================================================================
// GATEWAY COMPACTION STORE
// SQLite-backed implementation of the CompactionStore trait.
// Wraps the existing CompactionRepository so the kg_compactions audit
// table (SQLite-only) keeps its semantics, and the trait-routed
// maintenance ops see a uniform interface.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use zbot_stores_traits::{CompactionRunSummary, CompactionStore};

use crate::compaction_repository::CompactionRepository;

/// SQLite-backed `CompactionStore`. Delegates every method to the
/// concrete `CompactionRepository`. The trait-side methods are async,
/// the repo's are sync — wrapped via direct call (no spawn_blocking;
/// each call is a single fast INSERT against an indexed table).
pub struct GatewayCompactionStore {
    repo: Arc<CompactionRepository>,
}

impl GatewayCompactionStore {
    pub fn new(repo: Arc<CompactionRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl CompactionStore for GatewayCompactionStore {
    async fn record_merge(
        &self,
        run_id: &str,
        loser_entity_id: &str,
        winner_entity_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.repo
            .record_merge(run_id, loser_entity_id, winner_entity_id, reason)
    }

    async fn record_synthesis(
        &self,
        run_id: &str,
        fact_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.repo.record_synthesis(run_id, fact_id, reason)
    }

    async fn record_pattern(
        &self,
        run_id: &str,
        procedure_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.repo.record_pattern(run_id, procedure_id, reason)
    }

    async fn record_prune(
        &self,
        run_id: &str,
        entity_id: Option<&str>,
        relationship_id: Option<&str>,
        reason: &str,
    ) -> Result<String, String> {
        // The SQLite `CompactionRepository::record_prune` only stores
        // `entity_id`. Relationship-only prunes are folded into the
        // reason string so the audit trail still carries the info,
        // matching the pre-trait behavior.
        let eid = entity_id.unwrap_or("");
        let reason_full = match relationship_id {
            Some(rid) => format!("{reason} (relationship {rid})"),
            None => reason.to_string(),
        };
        self.repo.record_prune(run_id, eid, &reason_full)
    }

    async fn record_archival(
        &self,
        run_id: &str,
        entity_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        // SQLite repo doesn't distinguish archival from prune —
        // archival rows go into `kg_compactions` as 'prune' with the
        // reason carrying the archival context. Same wire shape, just
        // a tagged reason. Surreal can choose to keep them separate.
        self.repo.record_prune(run_id, entity_id, reason)
    }

    async fn latest_run_summary(&self) -> Result<Option<CompactionRunSummary>, String> {
        Ok(self
            .repo
            .latest_run_summary()?
            .map(|s| CompactionRunSummary {
                run_id: s.run_id,
                latest_at: s.latest_at,
                merges: s.merges,
                prunes: s.prunes,
            }))
    }
}
