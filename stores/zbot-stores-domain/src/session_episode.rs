//! `SessionEpisode` and related domain types.

use serde::{Deserialize, Serialize};

/// A session episode capturing what happened, what worked, and what was learned.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionEpisode {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    /// `'__global__'` or a specific ward name.
    pub ward_id: String,
    pub task_summary: String,
    /// One of: `'success'`, `'partial'`, `'failed'`, `'crashed'`.
    pub outcome: String,
    pub strategy_used: Option<String>,
    pub key_learnings: Option<String>,
    pub token_cost: Option<i64>,
    /// Raw f32 embedding. Always `None` when loaded from a backend that
    /// stores vectors out-of-row. Callers may set this prior to insert
    /// to have the vector persisted alongside.
    #[serde(skip)]
    pub embedding: Option<Vec<f32>>,
    pub created_at: String,
}

/// A session episode with a computed similarity score.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredEpisode {
    pub episode: SessionEpisode,
    pub score: f64,
}

/// One row returned by `EpisodeStore::list_successful_episodes_with_embedding`.
/// Captures the fields the PatternExtractor needs without serialising
/// the full SessionEpisode shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessfulEpisode {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub task_summary: String,
    /// L2-normalised embedding of `task_summary`, if one is indexed.
    pub embedding: Option<Vec<f32>>,
}
