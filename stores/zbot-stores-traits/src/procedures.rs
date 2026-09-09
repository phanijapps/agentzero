//! `ProcedureStore` trait — backend-agnostic interface for learned procedures.

use crate::error::{StoreError, StoreResult};
use crate::memory_facts::EmbeddingQueryIdentity;
use async_trait::async_trait;
use serde_json::Value;
// Domain types live in `zbot-stores-domain`; re-export here so the
// trait surface keeps working for callers that import from this crate.
pub use zbot_stores_domain::{PatternProcedureInsert, PatternStep, Procedure, ProcedureSummary};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcedureStats {
    pub total: i64,
}

#[async_trait]
pub trait ProcedureStore: Send + Sync {
    /// List procedures for a ward, capped at `limit`. Default empty.
    async fn list_by_ward(&self, _ward_id: &str, _limit: usize) -> StoreResult<Vec<Value>> {
        Ok(Vec::new())
    }

    /// List `(name, home ward)` pairs for an agent across ALL wards, capped
    /// at `limit`. The global name index for deterministic macro matching —
    /// wards organize context, not callables, so procedure names are never
    /// ward-scoped. Default empty.
    async fn list_procedure_names(
        &self,
        _agent_id: &str,
        _limit: usize,
    ) -> StoreResult<Vec<(String, Option<String>)>> {
        Ok(Vec::new())
    }

    /// Upsert a procedure; `embedding` is optional.
    async fn upsert_procedure(
        &self,
        procedure: Procedure,
        embedding: Option<Vec<f32>>,
    ) -> StoreResult<()> {
        let _ = (procedure, embedding);
        Err(StoreError::Unavailable(
            "upsert_procedure not implemented for this store".into(),
        ))
    }

    /// Vector-similarity search scoped to an agent (and optional ward).
    /// Each row carries a `procedure` field + `score` (cosine ∈ [0, 1]).
    async fn search_procedures_by_similarity(
        &self,
        _embedding: &[f32],
        _agent_id: &str,
        _ward_id: Option<&str>,
        _limit: usize,
    ) -> StoreResult<Vec<Value>> {
        Ok(Vec::new())
    }

    /// Identity-aware variant of `search_procedures_by_similarity`.
    async fn search_procedures_by_similarity_with_identity(
        &self,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<Value>> {
        let _ = query_identity;
        self.search_procedures_by_similarity(embedding, agent_id, ward_id, limit)
            .await
    }

    /// Typed variant of `search_procedures_by_similarity` returning
    /// `(Procedure, score)` pairs directly. Default deserialises the
    /// Value-based result for backends that haven't overridden.
    async fn search_procedures_by_similarity_typed(
        &self,
        embedding: &[f32],
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<(Procedure, f64)>> {
        self.search_procedures_by_similarity_typed_with_identity(
            embedding, None, agent_id, ward_id, limit,
        )
        .await
    }

    /// Identity-aware typed variant of `search_procedures_by_similarity_typed`.
    async fn search_procedures_by_similarity_typed_with_identity(
        &self,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<(Procedure, f64)>> {
        let rows = self
            .search_procedures_by_similarity_with_identity(
                embedding,
                query_identity,
                agent_id,
                ward_id,
                limit,
            )
            .await?;
        rows.into_iter()
            .map(|row| {
                let proc_v = row
                    .get("procedure")
                    .cloned()
                    .ok_or_else(|| "missing procedure field".to_string())?;
                let score = row.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let p: Procedure =
                    serde_json::from_value(proc_v).map_err(|e| format!("decode Procedure: {e}"))?;
                Ok((p, score))
            })
            .collect()
    }

    /// Bump success/failure counts after a run. No-op default.
    async fn increment_success(
        &self,
        _id: &str,
        _duration_ms: Option<i64>,
        _token_cost: Option<i64>,
    ) -> StoreResult<()> {
        Ok(())
    }

    async fn increment_failure(&self, _id: &str) -> StoreResult<()> {
        Ok(())
    }

    async fn procedure_stats(&self) -> StoreResult<ProcedureStats> {
        Ok(ProcedureStats::default())
    }

    // ---- Sleep-time pattern extraction (Phase D4) ----------------------

    /// Look up a procedure by `(agent_id, name)`. Returns just the
    /// dedup-relevant fields (id + success_count) so callers don't pay
    /// for hydrating the full row. Used by `PatternExtractor` to skip
    /// candidates whose name is already locked-in by a successful
    /// existing procedure. Default: not found.
    async fn get_procedure_summary_by_name(
        &self,
        _agent_id: &str,
        _name: &str,
    ) -> StoreResult<Option<ProcedureSummary>> {
        Ok(None)
    }

    /// Look up a full procedure row by `(agent_id, name)`. Returns the
    /// complete `Procedure` so callers (e.g., `RunProcedureTool`) can access
    /// `steps`, `parameters`, etc. Default: not implemented.
    async fn get_procedure_by_name(
        &self,
        _agent_id: &str,
        _name: &str,
    ) -> StoreResult<Option<Procedure>> {
        Ok(None)
    }

    /// Dedupe procedures by `(agent_id, name)`. For each group with 2+
    /// rows, keeps the highest-`success_count` row (ties broken by most
    /// recent `created_at`) and deletes the rest. Returns the number of
    /// rows deleted.
    ///
    /// Maintenance routine for cleaning up months of mining duplicates —
    /// PatternExtractor without dedup floored every cycle to keep
    /// generating new rows. The dedup floor (`success_count >= 2` in the
    /// extractor) prevents new duplicates going forward; this method
    /// retroactively collapses the existing pile. Vec-index rows are
    /// cleaned up alongside the procedure rows so similarity search
    /// stays consistent.
    async fn dedupe_procedures_by_name(&self) -> StoreResult<usize> {
        Ok(0)
    }

    /// Insert a synthesised procedure pattern. Pre-built from the
    /// LLM's structured response by `PatternExtractor`. Returns the
    /// procedure id used. Default: no-op error so misuse is loud.
    async fn insert_pattern_procedure(&self, _req: PatternProcedureInsert) -> StoreResult<String> {
        Err(StoreError::Unavailable(
            "insert_pattern_procedure not implemented for this store".into(),
        ))
    }
}
