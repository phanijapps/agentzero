//! `CompactionStore` trait — backend-agnostic audit log for the
//! sleep-time maintenance worker.
//!
//! Each maintenance op (Compactor, DecayEngine, Pruner, Synthesizer,
//! PatternExtractor, OrphanArchiver) records what it did so the
//! Observatory can show "last run merged X entities, pruned Y rows".
//!
//! Audit is observability, not correctness — every method has a
//! default no-op impl returning `Ok(String::new())`. Backends that
//! want audit history (SQLite via `kg_compactions` table, Surreal via
//! a `kg_compaction_run` table) override; minimal/test backends
//! inherit the defaults and the maintenance ops still run.
//!
//! This is the simplification: the trait describes WHAT happened
//! (operation-oriented), each backend stores it however it stores
//! things best, and consumers (the maintenance ops) don't care.

use async_trait::async_trait;

#[async_trait]
pub trait CompactionStore: Send + Sync {
    /// Record an entity merge: `loser_entity_id` was absorbed into
    /// `winner_entity_id`. Returns the audit row id (empty string for
    /// no-op impls).
    async fn record_merge(
        &self,
        _run_id: &str,
        _loser_entity_id: &str,
        _winner_entity_id: &str,
        _reason: &str,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    /// Record a synthesis: a cross-session strategy fact was extracted
    /// into the memory store. `fact_id` is the synthesized fact's id.
    async fn record_synthesis(
        &self,
        _run_id: &str,
        _fact_id: &str,
        _reason: &str,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    /// Record a pattern: a procedure pattern was upserted from session
    /// trajectories. `procedure_id` is the new procedure's id.
    async fn record_pattern(
        &self,
        _run_id: &str,
        _procedure_id: &str,
        _reason: &str,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    /// Record a prune: an entity / relationship row was hard-deleted
    /// because it was archived and old enough.
    async fn record_prune(
        &self,
        _run_id: &str,
        _entity_id: Option<&str>,
        _relationship_id: Option<&str>,
        _reason: &str,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    /// Record an archival: an entity was marked archival (soft-delete)
    /// because it has no edges or low confidence.
    async fn record_archival(
        &self,
        _run_id: &str,
        _entity_id: &str,
        _reason: &str,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    /// Latest-run summary for the Observatory health bar. Default
    /// returns `None` so backends without audit emit "no runs yet".
    async fn latest_run_summary(&self) -> Result<Option<CompactionRunSummary>, String> {
        Ok(None)
    }
}

/// Latest-run summary for the Observatory display.
#[derive(Debug, Clone, Default)]
pub struct CompactionRunSummary {
    pub run_id: String,
    pub latest_at: String,
    pub merges: u64,
    pub prunes: u64,
}
