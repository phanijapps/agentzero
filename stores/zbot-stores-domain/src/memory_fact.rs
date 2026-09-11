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

    /// When the agent last retrieved this fact into a final recall packet.
    /// Access-based reinforcement (ACT-R base-level activation): each
    /// retrieval slows the fact's recency decay. Distinct from
    /// `updated_at` (content change) — touch updates only this field
    /// plus `mention_count`. Absent on pre-reinforcement records.
    #[serde(default)]
    pub last_accessed: Option<String>,

    /// Long-term importance, 0.0–1.0 — the Generative-Agents retrieval
    /// triple's third term (relevance × recency × importance). Absent on
    /// pre-importance records; resolved to a category prior at read time
    /// via [`importance_of`]. The distiller may override with an LLM-scored
    /// value; everyone else gets the prior.
    #[serde(default)]
    pub importance: Option<f64>,
}

/// Category priors for fact importance when no explicit value was set.
/// Corrections and user preferences are the highest-value classes (they
/// guard future behavior); domain observations decay in value; indexed
/// capability entries (skill/agent) are lookup rows, not judgment.
///
/// Backward compatible: pre-importance records deserialize with
/// `importance: None` and resolve through this table.
#[must_use]
pub fn importance_of(fact: &MemoryFact) -> f64 {
    if fact.pinned {
        return 1.0;
    }
    let explicit = fact
        .importance
        .filter(|value| (0.0..=1.0).contains(value))
        .unwrap_or_else(|| category_importance_prior(&fact.category));
    explicit.clamp(0.0, 1.0)
}

/// Default importance by fact category. Unknown categories fall back to
/// the mid prior — neither boosted nor suppressed.
#[must_use]
pub fn category_importance_prior(category: &str) -> f64 {
    match category {
        "correction" | "instruction" => 0.9,
        "user" => 0.85,
        "pattern" | "strategy" => 0.7,
        "domain" => 0.6,
        "ctx" | "skill" | "agent" | "schema" | "primitive" => 0.5,
        _ => 0.6,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(category: &str, importance: Option<f64>, pinned: bool) -> MemoryFact {
        MemoryFact {
            id: "f".into(),
            session_id: None,
            agent_id: "a".into(),
            scope: "agent".into(),
            category: category.into(),
            key: "k".into(),
            content: "c".into(),
            confidence: 0.8,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "__global__".into(),
            contradicted_by: None,
            created_at: String::new(),
            updated_at: String::new(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned,
            epistemic_class: None,
            source_episode_id: None,
            source_ref: None,
            last_accessed: None,
            importance,
        }
    }

    #[test]
    fn priors_by_category() {
        assert_eq!(category_importance_prior("correction"), 0.9);
        assert_eq!(category_importance_prior("user"), 0.85);
        assert_eq!(category_importance_prior("pattern"), 0.7);
        assert_eq!(category_importance_prior("domain"), 0.6);
        assert_eq!(category_importance_prior("skill"), 0.5);
        assert_eq!(category_importance_prior("mystery"), 0.6, "unknown → mid");
    }

    #[test]
    fn explicit_importance_wins_and_clamps() {
        assert_eq!(importance_of(&fact("domain", Some(0.95), false)), 0.95);
        assert_eq!(
            importance_of(&fact("correction", Some(2.0), false)),
            0.9,
            "out-of-range falls back to prior"
        );
        assert_eq!(
            importance_of(&fact("user", Some(-1.0), false)),
            0.85,
            "out-of-range falls back to prior"
        );
        // out-of-range explicit falls back to the prior
        assert_eq!(importance_of(&fact("user", None, false)), 0.85);
    }

    #[test]
    fn pinned_resolves_to_max() {
        assert_eq!(importance_of(&fact("ctx", None, true)), 1.0);
        assert_eq!(importance_of(&fact("domain", Some(0.2), true)), 1.0);
    }
}
