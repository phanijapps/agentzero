// ============================================================================
// OUTBOX STORE TRAIT
// Backend-agnostic interface for the bridge outbox
// ============================================================================
//
// The outbox sits on the conversations side and stays SQLite forever per the
// design (`docs/architecture/future-state/persistence-readiness-design.md`). This
// trait exists for hygiene — no leakage of `rusqlite::Connection` through the
// public API. The concrete impl in `gateway-bridge/src/outbox.rs` implements
// this trait.
//
// Note on signatures:
// - All methods are synchronous to mirror the existing `OutboxRepository`
//   public surface exactly (the underlying `DatabaseManager::with_connection`
//   call is blocking). Making them async would force `block_on` shenanigans
//   in the impl that subtly change behaviour.
// - Errors are returned as `String` (the trait can't pull in `BridgeError`
//   without a circular dependency on `gateway-bridge`). Impls map their
//   typed errors to strings on the trait boundary; consumers that need the
//   typed errors continue to call the concrete type directly. This trait is
//   hygiene scaffold, not a migration target.
// - Trait surface is intentionally trimmed to the core lifecycle methods.
//   `OutboxRepository` methods deliberately omitted from the trait:
//     * `mark_failed(id, error, retry_after: Option<DateTime<Utc>>)` —
//       carries `chrono::DateTime<Utc>` in the signature; pulling chrono
//       into this dependency-light traits crate just for one method isn't
//       worth it.
//     * `get_unacked` / `get_since` / `get_retryable` — return
//       `Vec<OutboxItem>` whose row type lives in `gateway-bridge`.
//       Hoisting `OutboxItem` here is deferred until a consumer needs
//       trait-erased reads.
//     * `reset_all_inflight` / `cleanup_sent` — startup / janitorial
//       primitives only used directly by gateway today; can be promoted
//       into the trait when a cross-cutting consumer appears.

use serde_json::Value;

/// Backend-agnostic interface for the bridge outbox.
///
/// Mirrors a subset of `gateway_bridge::OutboxRepository`'s public surface
/// (insert + status transitions + adapter-scoped reset). Errors are
/// flattened to `String` at the trait boundary; concrete impls retain
/// their richer error types on direct calls.
///
/// One row in the outbox transitions through `pending` → `inflight` →
/// `sent` (or `failed`). See `OutboxItem` on the concrete impl for the
/// row shape.
pub trait OutboxStore: Send + Sync {
    /// Insert a new outbox item. Returns the generated ID (`obx-<uuid>`).
    fn insert_item(
        &self,
        adapter_id: &str,
        capability: &str,
        payload: &Value,
        session_id: Option<&str>,
        thread_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<String, String>;

    /// Mark an item as inflight (being sent to worker).
    fn mark_inflight(&self, id: &str) -> Result<(), String>;

    /// Mark an item as sent (ACK received from worker).
    fn mark_sent(&self, id: &str) -> Result<(), String>;

    /// Reset all inflight items for an adapter back to pending (on disconnect).
    /// Returns the number of rows updated.
    fn reset_inflight(&self, adapter_id: &str) -> Result<usize, String>;
}
