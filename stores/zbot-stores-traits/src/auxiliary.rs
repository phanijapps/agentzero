//! Auxiliary store traits batched in Phase D5.
//!
//! Smaller surface areas than the main MemoryFactStore / KnowledgeGraphStore;
//! grouped here to keep the file count tractable while still giving each one
//! a clean trait. Each is sized to its actual usage in the runtime.

use async_trait::async_trait;
use serde_json::Value;

// ============================================================================
// GoalStore
// ============================================================================

/// Backend-agnostic interface for goals — agent intents with lifecycle.
#[async_trait]
pub trait GoalStore: Send + Sync {
    /// Get a goal by id.
    async fn get_goal(&self, _goal_id: &str) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// Active goals for an agent — used for intent boost in unified recall.
    async fn list_active_goals(&self, _agent_id: &str) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    /// Create a new goal. Returns the persisted id.
    async fn create_goal(&self, _goal: Value) -> Result<String, String> {
        Err("create_goal not implemented for this store".to_string())
    }

    /// Move a goal to a new state (active / blocked / satisfied / abandoned).
    async fn update_goal_state(&self, _goal_id: &str, _new_state: &str) -> Result<(), String> {
        Ok(())
    }

    /// Update the JSON-serialised `filled_slots` payload on a goal.
    /// Used by the agent runtime to record progress against the goal's
    /// declared slots. Default: no-op so backends without slot tracking
    /// degrade gracefully.
    async fn update_goal_filled_slots(
        &self,
        _goal_id: &str,
        _filled_slots_json: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

// ============================================================================
// RecallLogStore
// ============================================================================

/// Per-session log of which fact-keys were recalled. Drives predictive
/// recall (don't re-surface the same fact every turn).
#[async_trait]
pub trait RecallLogStore: Send + Sync {
    async fn log_recall(&self, _session_id: &str, _fact_key: &str) -> Result<(), String> {
        Ok(())
    }

    async fn get_keys_for_session(&self, _session_id: &str) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    async fn get_keys_for_sessions(&self, _session_ids: &[String]) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}

// ============================================================================
// DistillationStore
// ============================================================================

/// Distillation run lifecycle: tracks which sessions have been distilled,
/// retry counts, and per-run statistics.
#[async_trait]
pub trait DistillationStore: Send + Sync {
    /// Insert a new distillation run row.
    async fn insert_run(&self, _run: Value) -> Result<(), String> {
        Err("insert_run not implemented for this store".to_string())
    }

    /// Get the run row for a session (if any).
    async fn get_run_by_session(&self, _session_id: &str) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// Bump the retry counter for a failed run.
    async fn update_retry(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }

    /// Mark a run successful. `summary` is a free-form post-mortem string.
    async fn update_success(
        &self,
        _session_id: &str,
        _summary: Option<String>,
    ) -> Result<(), String> {
        Ok(())
    }

    // ---- Richer lifecycle methods used by SessionDistiller --------------
    //
    // The Value-based methods above predate Phase E6c; they're kept for
    // legacy callers. The methods below carry the typed counts +
    // duration that the live SessionDistiller actually wants, matching
    // the SQLite `DistillationRepository` signatures one-for-one. New
    // callers should prefer these.

    /// Record a pending/skipped/failed run insertion. `status` is the
    /// terminal label ("pending", "skipped", "failed"); `error` is an
    /// optional human-readable note. Idempotent on `session_id` —
    /// backends with a unique index ON CONFLICT update; backends
    /// without one may produce duplicate rows (acceptable; later
    /// success update collapses).
    async fn record_distillation_pending(
        &self,
        _session_id: &str,
        _status: &str,
        _error: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Mark a session's run successful with extraction counts.
    /// `episode_created` is `true` when the distiller emitted a
    /// session_episode row alongside the fact upserts.
    async fn record_distillation_success(
        &self,
        _session_id: &str,
        _facts: i32,
        _entities: i32,
        _relationships: i32,
        _episode_created: bool,
        _duration_ms: i64,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Mark a session's run failed/retry. `retry_count` is the
    /// post-update value (0 on first failure). `error` is the
    /// human-readable message.
    async fn record_distillation_failure(
        &self,
        _session_id: &str,
        _status: &str,
        _retry_count: i32,
        _error: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }
}
