//! `Goal` domain type.

use serde::{Deserialize, Serialize};

/// A goal row — agent intents with lifecycle state and decomposition edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub agent_id: String,
    pub ward_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    /// `active` / `blocked` / `satisfied` / `abandoned`.
    pub state: String,
    pub parent_goal_id: Option<String>,
    /// JSON-encoded slot map.
    pub slots: Option<String>,
    /// JSON-encoded filled-slot values.
    pub filled_slots: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}
