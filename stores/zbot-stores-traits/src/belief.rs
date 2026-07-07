//! `BeliefStore` trait — persistence interface for the Belief Network.
//!
//! Beliefs are aggregates over one or more `MemoryFact`s about a single
//! subject. This trait is dep-light by design so the `agent-tools` crate
//! can call it without dragging in `zbot-stores-sqlite`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use zbot_stores_domain::{Belief, ScoredBelief};

use crate::memory_facts::EmbeddingQueryIdentity;

/// Abstract interface for durable belief storage.
///
/// Implementations can wrap a SQLite repository, a remote API, or an
/// in-memory store for testing. All methods are async to allow blocking
/// backends to be wrapped on a runtime executor without leaking shape.
#[async_trait]
pub trait BeliefStore: Send + Sync {
    /// Get the active belief for a subject at `as_of` (default: `Utc::now()`).
    /// Returns `None` if no belief exists for this subject.
    async fn get_belief(
        &self,
        partition_id: &str,
        subject: &str,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Option<Belief>, String>;

    /// List recent beliefs for a partition, ordered by `updated_at`
    /// descending.
    async fn list_beliefs(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String>;

    /// Insert or update a belief. UPSERT keyed on
    /// `(partition_id, subject, valid_from)`.
    async fn upsert_belief(&self, belief: &Belief) -> Result<(), String>;

    /// Mark a belief superseded by another. Mirrors `supersede_fact`
    /// semantics: the old belief's `valid_until` closes at
    /// `transition_time` and its `superseded_by` is set to `new_id`.
    async fn supersede_belief(
        &self,
        old_id: &str,
        new_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String>;

    /// Mark a belief stale — the next synthesizer cycle re-derives it
    /// from its (remaining) source facts. B-3 propagation calls this on
    /// a multi-source belief whose source fact was invalidated.
    async fn mark_stale(&self, belief_id: &str) -> Result<(), String>;

    /// Retract a belief — set its `valid_until` to `transition_time`.
    /// B-3 propagation calls this on a sole-source belief whose only
    /// fact was invalidated. Differs from `supersede_belief` in that no
    /// replacement belief id is recorded.
    async fn retract_belief(
        &self,
        belief_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String>;

    /// Find beliefs whose `source_fact_ids` JSON array contains the given
    /// fact_id and that are still active (valid_until IS NULL). Returns
    /// belief ids only — callers load the full `Belief` on demand via
    /// [`BeliefStore::get_belief_by_id`].
    async fn beliefs_referencing_fact(&self, fact_id: &str) -> Result<Vec<String>, String>;

    /// Load a belief by its primary-key id. Returns `None` when the id
    /// is unknown. Used by B-3 propagation to read `source_fact_ids`
    /// without needing to know the belief's partition.
    async fn get_belief_by_id(&self, belief_id: &str) -> Result<Option<Belief>, String>;

    /// List stale beliefs in a partition, oldest-first by `updated_at`.
    /// Used by the synthesizer to pick up re-synthesis candidates at the
    /// top of each cycle.
    async fn list_stale(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String>;

    /// Clear the stale flag on a belief. Called by the synthesizer right
    /// after a successful re-synthesis pass.
    async fn clear_stale(&self, belief_id: &str) -> Result<(), String>;

    /// Search beliefs by semantic similarity to a query embedding
    /// (Phase B-4 — recall integration).
    ///
    /// Returns up to `limit` beliefs in `partition_id` scored by cosine
    /// similarity, sorted descending. Filters out:
    /// - superseded beliefs (`superseded_by IS NOT NULL`)
    /// - retracted / historical beliefs whose interval doesn't cover "now"
    ///   (`valid_until <= now`)
    /// - beliefs with NULL embedding (cannot be scored semantically)
    ///
    /// Implementations may load all live beliefs in the partition and
    /// score them in-memory — belief count is bounded by design.
    async fn search_beliefs(
        &self,
        partition_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<ScoredBelief>, String>;

    /// Identity-aware variant of [`BeliefStore::search_beliefs`].
    /// Backends with persisted embedding identities should override this and
    /// fail closed when `query_identity` is missing or mismatched.
    async fn search_beliefs_with_identity(
        &self,
        partition_id: &str,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        limit: usize,
    ) -> Result<Vec<ScoredBelief>, String> {
        let _ = query_identity;
        self.search_beliefs(partition_id, query_embedding, limit)
            .await
    }
}
