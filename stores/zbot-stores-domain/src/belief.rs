//! `Belief` domain type for the Belief Network (Phase B-1).
//!
//! A belief is an aggregate over one or more `MemoryFact`s about a single
//! subject. It carries its own bi-temporal interval (`valid_from` /
//! `valid_until`), confidence (derived from constituents + recency), and
//! provenance (`source_fact_ids`). Beliefs are partition-scoped from day
//! one (`partition_id`) so the future R-series rename of `ward_id` does
//! not need to touch this type.
//!
//! Storage shape is `kg_beliefs` — see migration v27 for the SQL schema.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A synthesized stance the agent maintains about one subject.
///
/// `subject` is the canonical aggregation key (e.g. `user.location`,
/// `domain.finance.acn.valuation_verdict`). `confidence` is the
/// recency-weighted average of constituent fact confidences. `reasoning`
/// is populated only when the synthesis required an LLM call — single-fact
/// beliefs short-circuit and leave `reasoning = None`.
///
/// `stale` (B-3) is set to `true` when a constituent fact is invalidated
/// and the belief has multiple sources — the next `BeliefSynthesizer`
/// cycle picks up stale beliefs first and re-synthesizes them from the
/// remaining valid facts, then clears the flag. Sole-source beliefs are
/// retracted directly (`valid_until` set) instead of marked stale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief {
    pub id: String,
    pub partition_id: String,
    pub subject: String,
    pub content: String,
    pub confidence: f64,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub source_fact_ids: Vec<String>,
    pub synthesizer_version: i32,
    pub reasoning: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub superseded_by: Option<String>,
    #[serde(default)]
    pub stale: bool,
    /// Embedding vector for semantic recall (Phase B-4). Stored on the
    /// belief row as little-endian f32 bytes; `None` means the belief
    /// was synthesized without an available embedding client and won't
    /// surface in `search_beliefs` (only via direct lookup).
    #[serde(default)]
    pub embedding: Option<Vec<u8>>,
}

/// A belief scored by similarity to a recall query (Phase B-4).
///
/// Returned by `BeliefStore::search_beliefs`. `score` is the cosine
/// similarity (`[-1, 1]`) between the query embedding and the belief's
/// stored embedding. Callers project these into `ScoredItem`s with
/// `ItemKind::Belief` for RRF fusion against other recall sources.
#[derive(Debug, Clone)]
pub struct ScoredBelief {
    pub belief: Belief,
    pub score: f64,
}
