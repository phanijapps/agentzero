//! Domain shapes for the distillation pipeline.

use serde::{Deserialize, Serialize};

/// A session that has not yet been distilled, with its root agent ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndistilledSession {
    pub session_id: String,
    pub agent_id: String,
}

/// Aggregate statistics across all distillation runs.
#[derive(Debug, Serialize, Default)]
pub struct DistillationStats {
    pub success_count: i64,
    pub failed_count: i64,
    pub skipped_count: i64,
    pub permanently_failed_count: i64,
    pub total_facts: i64,
    pub total_entities: i64,
    pub total_relationships: i64,
    pub total_episodes: i64,
}
