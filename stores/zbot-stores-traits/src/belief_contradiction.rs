//! `BeliefContradictionStore` trait — persistence interface for the
//! Belief Network contradiction graph (Phase B-2).
//!
//! Contradictions are pair-wise rows between two beliefs. The trait
//! enforces canonical pair ordering (smaller-id first) at the
//! implementation layer so callers can pass IDs in any order. Backends
//! must reject duplicate `(belief_a_id, belief_b_id)` inserts silently
//! (idempotent) — the detector relies on this to skip already-evaluated
//! pairs without an explicit pre-check race.

use async_trait::async_trait;
use zbot_stores_domain::{BeliefContradiction, Resolution};

/// Abstract interface for durable belief-contradiction storage.
///
/// Implementations should treat `(belief_a_id, belief_b_id)` as an
/// unordered pair and canonicalize to lexicographically-smaller-first
/// before reading or writing.
#[async_trait]
pub trait BeliefContradictionStore: Send + Sync {
    /// Insert a new contradiction. Implementations MUST canonicalize the
    /// pair (smaller id first) and treat conflicts on the unique index
    /// as a no-op (idempotent).
    async fn insert_contradiction(&self, c: &BeliefContradiction) -> Result<(), String>;

    /// List contradictions involving a specific belief — works whether
    /// the belief is on the `belief_a_id` or `belief_b_id` side.
    async fn for_belief(&self, belief_id: &str) -> Result<Vec<BeliefContradiction>, String>;

    /// List recent contradictions in a partition, joined through
    /// `kg_beliefs.partition_id` since contradictions don't carry a
    /// partition column directly.
    async fn list_recent(
        &self,
        partition_id: &str,
        limit: usize,
    ) -> Result<Vec<BeliefContradiction>, String>;

    /// Check if a pair has already been evaluated (any row exists).
    /// Implementations MUST canonicalize the pair before lookup.
    async fn pair_exists(&self, belief_a_id: &str, belief_b_id: &str) -> Result<bool, String>;

    /// Mark a contradiction resolved. Sets `resolution` and `resolved_at`
    /// to "now".
    async fn resolve(&self, contradiction_id: &str, resolution: Resolution) -> Result<(), String>;
}
