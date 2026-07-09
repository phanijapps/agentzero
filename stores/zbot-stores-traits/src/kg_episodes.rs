//! `KgEpisodeStore` trait — backend-agnostic interface for the
//! knowledge-graph ingestion pipeline.
//!
//! Each row tracks one extraction event (a chunk of source text staged
//! for ingest) with status `pending` → `running` → `done` | `failed`.
//! The `KgEpisode` JSON shape (see `zbot_stores_sqlite::KgEpisode` /
//! the Surreal `kg_ingestion_episode` table) is used as the canonical
//! over-the-wire format.
//!
//! Consumers:
//! - `gateway/src/http/ingest.rs`: the `/api/graph/ingest` endpoints
//! - `gateway-execution/invoke/ingest_adapter.rs`: the `ingest` agent tool
//! - `gateway-execution/ingest/queue.rs`: background processor (polls
//!   `claim_next_pending` and runs LLM extraction)
//! - `gateway-execution/ingest/backpressure.rs`: rate gate
//! - `gateway-execution/tool_result_extractor.rs`: auto-ingest from tool results
//! - `gateway-execution/ward_artifact_indexer`: per-ward reindex pipeline
//! - `gateway-execution/sleep/embedding_reindex`: dim-change reindex
//!
//! Backend-agnostic by construction: the trait surface is primitives +
//! `serde_json::Value` for the row shape. Adding a new datastore
//! (Postgres / MongoDB / etc.) means implementing the trait and adding
//! a build branch in `persistence_factory.rs` — zero changes to consumers.

use async_trait::async_trait;
use serde_json::Value;

/// Aggregate counts of ingestion episodes by status, used by the
/// progress endpoint and backpressure gate.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KgEpisodeStatusCounts {
    pub pending: u64,
    pub running: u64,
    pub done: u64,
    pub failed: u64,
}

/// Backend-agnostic interface for the kg_ingestion_episode subsystem.
///
/// Each row in the canonical `KgEpisode` shape:
/// ```ignore
/// {
///   "id":            "ep-<uuid>",
///   "source_type":   "tool_result" | "ward_file" | "session" | "distillation" | "user_input",
///   "source_ref":    "<source>#chunk-<n>",
///   "content_hash":  "<sha256>",
///   "session_id":    "<sess-uuid>" | null,
///   "agent_id":      "<agent>",
///   "status":        "pending" | "running" | "done" | "failed",
///   "retry_count":   0,
///   "error":         "<message>" | null,
///   "created_at":    "<iso-8601>",
///   "started_at":    "<iso-8601>" | null,
///   "completed_at":  "<iso-8601>" | null
/// }
/// ```
///
/// Defaults match `MemoryFactStore` / `EpisodeStore` style — read paths
/// return empty / None, write paths return loud errors so unsupported
/// methods don't silently swallow data.
#[async_trait]
pub trait KgEpisodeStore: Send + Sync {
    // ---- Read paths -------------------------------------------------

    /// Look up an episode by exact id. Returns the `KgEpisode` JSON
    /// shape or `None` if the id is unknown.
    async fn get_episode(&self, _id: &str) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// Dedup probe — find the existing episode for a given
    /// (source_type, content_hash) pair, if any. The ingest pipeline
    /// uses this to avoid re-processing identical chunks.
    async fn get_by_content_hash(
        &self,
        _source_type: &str,
        _content_hash: &str,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// All episodes attributable to a session (across sources).
    async fn list_by_session(&self, _session_id: &str) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    /// Aggregate counts for `source_id` (matched by `source_ref` prefix).
    async fn status_counts_for_source(
        &self,
        _source_ref_prefix: &str,
    ) -> Result<KgEpisodeStatusCounts, String> {
        Ok(KgEpisodeStatusCounts::default())
    }

    /// Global pending count — used by backpressure to enforce a hard cap
    /// across all sources.
    async fn count_pending_global(&self) -> Result<u64, String> {
        Ok(0)
    }

    /// Pending count scoped to one source — used by backpressure to
    /// enforce a per-source quota.
    async fn count_pending_for_source(&self, _source_ref_prefix: &str) -> Result<u64, String> {
        Ok(0)
    }

    // ---- Write paths ------------------------------------------------

    /// Create a `pending` episode (or return the existing id if a row
    /// with matching `(source_type, content_hash)` already exists).
    /// Returns the persisted id either way.
    async fn upsert_pending(
        &self,
        _source_type: &str,
        _source_ref: &str,
        _content_hash: &str,
        _session_id: Option<&str>,
        _agent_id: &str,
    ) -> Result<String, String> {
        Err("upsert_pending not implemented for this store".to_string())
    }

    /// Atomically claim the next pending episode for processing.
    /// Returns `None` when the queue is empty. The status transitions
    /// to `running` and `started_at` is stamped.
    async fn claim_next_pending(&self) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// Mark an episode `done`. Idempotent.
    async fn mark_done(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    /// Mark an episode `failed` with an error message. Idempotent.
    async fn mark_failed(&self, _id: &str, _error: &str) -> Result<(), String> {
        Ok(())
    }

    /// Reset a failed episode back to `pending` if `retry_count` is
    /// below `max_retries`. Returns `true` when the retry was queued,
    /// `false` when the episode is over the retry budget.
    async fn retry_if_eligible(&self, _id: &str, _max_retries: u32) -> Result<bool, String> {
        Ok(false)
    }

    /// Attach the chunk's text payload. Stored separately from the
    /// metadata row so large payloads don't bloat status queries.
    async fn set_payload(&self, _id: &str, _text: &str) -> Result<(), String> {
        Err("set_payload not implemented for this store".to_string())
    }

    /// Read back the payload — used by the queue processor before
    /// running LLM extraction.
    async fn get_payload(&self, _id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
}
