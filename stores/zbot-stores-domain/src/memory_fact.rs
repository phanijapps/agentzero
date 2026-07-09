//! `MemoryFact` and related domain types.
//!
//! Shared domain shape so any backend impl can round-trip the type without
//! depending on the SQLite-coupled crate.

use serde::{Deserialize, Serialize};

/// A structured memory fact extracted from session distillation or manual save.
///
/// This is the persistence-layer shape. HTTP responses derive from it
/// via `MemoryFactResponse` in the gateway crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFact {
    pub id: String,
    pub session_id: Option<String>,
    pub agent_id: String,
    pub scope: String,
    pub category: String,
    pub key: String,
    pub content: String,
    pub confidence: f64,
    pub mention_count: i32,
    pub source_summary: Option<String>,
    /// Raw f32 embedding. Always `None` when loaded from a backend that
    /// stores embeddings out-of-row (e.g. the SQLite `memory_facts_index`
    /// vec0 table). Callers may set this to `Some(v)` prior to upsert to
    /// have the vector persisted alongside the row — vectors MUST be
    /// L2-normalized by the caller.
    #[serde(skip)]
    pub embedding: Option<Vec<f32>>,
    /// Ward (sandbox) this fact belongs to. `"__global__"` means shared across all wards.
    pub ward_id: String,
    /// If set, the key of the newer fact that contradicts this one.
    pub contradicted_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: Option<String>,
    /// ISO-8601 timestamp from which this fact is valid.
    pub valid_from: Option<String>,
    /// ISO-8601 timestamp after which this fact is no longer current (superseded).
    pub valid_until: Option<String>,
    /// Key of the newer fact that replaced this one.
    pub superseded_by: Option<String>,
    /// Pinned facts can't be overwritten by distillation. User-authored facts are pinned.
    #[serde(default)]
    pub pinned: bool,
    /// Epistemic classification governing lifecycle behavior:
    /// - `archival` — historical records, never decay
    /// - `current` — volatile observed state, decays when superseded
    /// - `convention` — rules/preferences, stable until explicitly replaced
    /// - `procedural` — learned patterns, evolve via success counts
    ///
    /// Defaults to `"current"` when not specified.
    #[serde(default)]
    pub epistemic_class: Option<String>,

    /// FK to the extraction event (e.g. `kg_episodes.id`) that produced this fact.
    #[serde(default)]
    pub source_episode_id: Option<String>,

    /// Human-readable pointer to source (e.g., `"research_notes.pdf:page_42"`).
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// A memory fact with a computed relevance score from hybrid search.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredFact {
    pub fact: MemoryFact,
    pub score: f64,
}

/// Result of `MemoryFactStore::find_strategy_fact_by_similarity`.
/// Captures only what the Synthesizer needs to decide whether to bump
/// or insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyFactMatch {
    pub fact_id: String,
    /// Comma-separated csv of episode ids previously attributed to
    /// this fact, or `None` if the column is null.
    pub source_episode_id: Option<String>,
}

/// Request shape for `MemoryFactStore::insert_strategy_fact`. Flat
/// field set chosen so backends construct their canonical fact row
/// without callers needing to know the full `MemoryFact` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyFactInsert {
    pub agent_id: String,
    pub key: String,
    pub content: String,
    pub confidence: f64,
    pub source_summary: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub source_episode_id: Option<String>,
}
