//! `BeliefContradiction` domain type — Belief Network Phase B-2.
//!
//! A `BeliefContradiction` is a pair-wise relationship row between two
//! `Belief`s that the LLM judge classified as `logical_contradiction` or
//! `tension`. The detector enforces canonical pair ordering
//! (`belief_a_id` is always the lexicographically smaller of the two)
//! so `UNIQUE(belief_a_id, belief_b_id)` in `kg_belief_contradictions`
//! prevents double-detection across cycles.
//!
//! Storage shape is `kg_belief_contradictions` — see migration v28 for
//! the SQL schema.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Classification of a contradiction.
///
/// - `Logical`: A and B cannot both be true at the same time (e.g. two
///   different current employers).
/// - `Tension`: Different facets of the same subject; could both be true
///   with context (e.g. "prefers dark mode" + "prefers light mode").
/// - `Temporal`: Reserved for future use — captures cases where two
///   beliefs share a subject but disagree on the time interval. Not
///   produced by the B-2 detector yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContradictionType {
    #[serde(rename = "logical")]
    Logical,
    #[serde(rename = "tension")]
    Tension,
    #[serde(rename = "temporal")]
    Temporal,
}

/// Resolution applied to a contradiction by the operator (or by a future
/// auto-resolver). `Unresolved` and `None`-in-storage both mean "not
/// resolved yet"; resolution rows in B-2 are always written `None` and
/// later updated by a UI / tool path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Resolution {
    #[serde(rename = "a_won")]
    AWon,
    #[serde(rename = "b_won")]
    BWon,
    #[serde(rename = "compatible")]
    Compatible,
    #[serde(rename = "unresolved")]
    Unresolved,
}

/// One pair-wise contradiction between two beliefs.
///
/// `belief_a_id` is always the lexicographically smaller of the two
/// belief IDs — canonical pair ordering enforced at insert time.
/// `severity` is the LLM judge's confidence in the classification
/// (0.0..1.0), NOT the severity of disagreement. `judge_reasoning` is
/// the LLM's one-sentence explanation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefContradiction {
    pub id: String,
    pub belief_a_id: String,
    pub belief_b_id: String,
    pub contradiction_type: ContradictionType,
    pub severity: f64,
    pub judge_reasoning: Option<String>,
    pub detected_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution: Option<Resolution>,
}
