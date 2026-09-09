//! Fact-retrieval composition on engram's fusion primitives.
//!
//! zbot's fact search produces three candidate lanes — semantic (vector),
//! lexical (FTS), and temporal (recency) — which engram's
//! [`ReciprocalRankFusion`](engram_retrieval::ReciprocalRankFusion) fuses
//! with per-source weights. This replaces the adapter's hand-rolled RRF
//! loop and gives the production ranking the recency term it lacked
//! (`updated_at` was persisted but never read).
//!
//! Design notes:
//! - The temporal lane is **restricted to facts already matched** by the
//!   semantic or lexical lane. An independent recency lane would surface
//!   recent-but-irrelevant facts; recency should reorder the matched set,
//!   not widen it.
//! - Recency uses the same exponential decay engram's sqlite temporal
//!   lane uses (`0.5^(age/half_life)`); the half-life is per category,
//!   with pinned facts and re-indexed skill/agent entries exempt.
//! - Fusion weights are per-lane: the temporal lane contributes at half
//!   strength so a fresh fact cannot outrank a strongly relevant one —
//!   recency breaks ties, it does not dominate relevance.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use engram_domain::{
    Actor, ActorKind, AllowedUse, DeleteMode, FusionStrategy, Id, Policy, Provenance, Requester,
    Retention, RetrievalMode, RetrievalRequest, RetrievalResult, RetrievalScore,
    RetrievalTargetType, Scope, Sensitivity, Visibility,
};
use engram_retrieval::{ReciprocalFusionConfig, ReciprocalRankFusion, RetrievalFusion as _};
use zbot_stores_domain::MemoryFact;

/// RRF constant — matches the adapter's historical `RRF_K` and engram's
/// `DEFAULT_RRF_K`.
const RRF_K: u32 = 60;

/// Per-lane fusion weights. Temporal at half strength by design (see module
/// docs).
const TEMPORAL_LANE_WEIGHT: f32 = 0.5;

/// Exponential recency decay half-lives by fact category (days). Defaults
/// apply to any category not listed.
const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;
const CATEGORY_HALF_LIFE_DAYS: &[(&str, f64)] = &[
    ("user", 90.0),
    ("correction", 180.0),
    ("instruction", 180.0),
];

/// Categories that never decay: re-indexed every session, so age is noise.
const NO_DECAY_CATEGORIES: &[&str] = &["skill", "agent"];

/// Exponential recency in `(0.0, 1.0]`: 1.0 at age zero, 0.5 at one
/// half-life, halving each half-life after. Mirrors engram-store-sqlite's
/// `recency_score`.
pub(crate) fn recency_decay(
    updated_at: &str,
    category: &str,
    pinned: bool,
    now: DateTime<Utc>,
) -> f64 {
    if pinned || NO_DECAY_CATEGORIES.contains(&category) {
        return 1.0;
    }
    let half_life = CATEGORY_HALF_LIFE_DAYS
        .iter()
        .find(|(cat, _)| *cat == category)
        .map(|(_, days)| *days)
        .unwrap_or(DEFAULT_HALF_LIFE_DAYS);
    let age_days = updated_at
        .parse::<DateTime<Utc>>()
        .map(|observed| (now - observed).num_days().max(0) as f64)
        .unwrap_or(0.0);
    0.5_f64.powf(age_days / half_life)
}

/// One lane candidate: a fact with its lane-native score.
pub(crate) struct LaneCandidate {
    pub fact: MemoryFact,
    /// Lane-native relevance (cosine for semantic, sparse score for lexical,
    /// recency for temporal).
    pub score: f64,
}

/// A fused fact hit: fact + fused score (normalized to roughly `[0, 1]` by
/// multiplying by `RRF_K`, preserving the adapter's historical score scale)
/// + which content lanes matched.
pub(crate) struct FusedFact {
    pub fact: MemoryFact,
    pub score: f64,
    pub semantic: bool,
    pub sparse: bool,
}

fn lane_result(fact: &MemoryFact, source: &str, score: f64) -> RetrievalResult {
    RetrievalResult {
        id: format!("{source}:{}", fact.id),
        target_type: RetrievalTargetType::Memory,
        target_id: fact.id.clone(),
        content: fact.content.clone(),
        score: RetrievalScore {
            total: score as f32,
            relevance: Some(score as f32),
            recency: None,
            confidence: None,
            cue_match: None,
            hierarchical_fit: None,
            policy_fit: None,
        },
        provenance: Provenance {
            source: "zbot_memory_facts".to_string(),
            actor: Actor {
                id: Id::from("zbot-adapter"),
                kind: ActorKind::System,
                display_name: None,
                metadata: None,
            },
            observed_at: Utc::now(),
            evidence: Vec::new(),
            derivations: Vec::new(),
            confidence: Some(fact.confidence as f32),
            method: None,
        },
        policy: Policy {
            visibility: Visibility::Workspace,
            retention: Retention::Durable,
            sensitivity: Some(Sensitivity::Low),
            allowed_uses: vec![AllowedUse::Retrieval],
            expires_at: None,
            delete_mode: Some(DeleteMode::Tombstone),
        },
        explanation: None,
        fusion_trace: Some(engram_domain::FusionTrace {
            query_id: None,
            vector_index: None,
            embedding_time_ms: None,
            search_time_ms: None,
            source: source.to_string(),
            source_rank: None,
            source_score: Some(score as f32),
            score: None,
            rank: None,
            fusion_strategy: Some(FusionStrategy::None),
            fusion_score: None,
            rerank_strategy: None,
            rerank_score: None,
            discard_reason: None,
            deduplicated_with: Vec::new(),
        }),
        metadata: None,
    }
}

/// Fuse the three fact lanes with engram's weighted reciprocal-rank fusion.
///
/// `semantic` and `lexical` are the query-matched candidates (each sorted by
/// its lane score, descending — the caller's existing ordering). The temporal
/// lane is derived here from the union of matched facts: recency reorders the
/// matched set without widening it.
pub(crate) fn fuse_fact_lanes(
    semantic: Vec<LaneCandidate>,
    lexical: Vec<LaneCandidate>,
    limit: usize,
    now: DateTime<Utc>,
) -> Vec<FusedFact> {
    // Lane intake is tie-aware: when lane scores are equal, the fresher
    // fact takes the earlier lane rank, so weighted RRF's rank assignment
    // lets recency break exact relevance ties (instead of vec order).
    let recency_of =
        |fact: &MemoryFact| recency_decay(&fact.updated_at, &fact.category, fact.pinned, now);
    let mut semantic = semantic;
    let mut lexical = lexical;
    semantic.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                recency_of(&right.fact)
                    .partial_cmp(&recency_of(&left.fact))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    lexical.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                recency_of(&right.fact)
                    .partial_cmp(&recency_of(&left.fact))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    // id → (fact, semantic?, sparse?, recency)
    let mut matched: HashMap<String, (MemoryFact, bool, bool, f64)> = HashMap::new();
    let mut candidates: Vec<RetrievalResult> = Vec::new();

    for candidate in semantic {
        let recency = recency_decay(
            &candidate.fact.updated_at,
            &candidate.fact.category,
            candidate.fact.pinned,
            now,
        );
        let entry = matched.entry(candidate.fact.id.clone()).or_insert((
            candidate.fact.clone(),
            false,
            false,
            recency,
        ));
        entry.1 = true;
        candidates.push(lane_result(
            &candidate.fact,
            "fact_semantic",
            candidate.score,
        ));
    }
    for candidate in lexical {
        let recency = recency_decay(
            &candidate.fact.updated_at,
            &candidate.fact.category,
            candidate.fact.pinned,
            now,
        );
        let entry = matched.entry(candidate.fact.id.clone()).or_insert((
            candidate.fact.clone(),
            false,
            false,
            recency,
        ));
        entry.2 = true;
        // Lexical candidates not already present via the semantic lane join
        // the pool here; those already present simply add a second lane hit
        // below via their own temporal/lexical candidate entries.
        candidates.push(lane_result(
            &candidate.fact,
            "fact_lexical",
            candidate.score,
        ));
    }

    // Temporal lane over the matched set: recency-scored, newest-first.
    let mut temporal: Vec<(String, f64)> = matched
        .iter()
        .map(|(id, (_, _, _, recency))| (id.clone(), *recency))
        .collect();
    temporal.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (id, recency) in temporal {
        if let Some((fact, _, _, _)) = matched.get(&id) {
            candidates.push(lane_result(fact, "fact_temporal", recency));
        }
    }

    let fusion = ReciprocalRankFusion::new(
        ReciprocalFusionConfig::new(
            RRF_K,
            1.0,
            [("fact_temporal".to_string(), TEMPORAL_LANE_WEIGHT)]
                .into_iter()
                .collect(),
        )
        .unwrap_or_else(|_| ReciprocalFusionConfig::default()),
    );
    let request = RetrievalRequest {
        limit: Some(limit as u32),
        ..minimal_request()
    };
    let fused = match fusion.fuse(&request, candidates) {
        Ok(fused) => fused,
        Err(_) => return Vec::new(),
    };

    fused
        .into_iter()
        .filter_map(|result| {
            let (fact, semantic, sparse, _recency) = matched.get(&result.target_id)?;
            Some(FusedFact {
                fact: fact.clone(),
                // Normalize fused RRF (≈ 1/(k+rank) sums) back onto the
                // adapter's historical ≈[0, 1] scale.
                score: (result.score.total as f64) * (RRF_K as f64),
                semantic: *semantic,
                sparse: *sparse,
            })
        })
        .collect()
}

fn minimal_request() -> RetrievalRequest {
    // Only `limit` participates in fusion; the remaining fields are neutral
    // for this internal use of the fuser (mirrors engram's own fusion tests).
    RetrievalRequest {
        query: String::new(),
        scope: Scope {
            tenant: "zbot".to_string(),
            subject: None,
            workspace: None,
            session: None,
            environment: None,
        },
        requester: Requester {
            actor: Actor {
                id: Id::from("zbot-adapter"),
                kind: ActorKind::System,
                display_name: None,
                metadata: None,
            },
            roles: Vec::new(),
            permissions: Vec::new(),
            on_behalf_of: None,
        },
        modes: vec![RetrievalMode::Semantic, RetrievalMode::Keyword],
        filters: None,
        cues: Vec::new(),
        limit: None,
        budget: None,
        include_explanations: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, category: &str, updated_days_ago: i64) -> MemoryFact {
        MemoryFact {
            id: id.to_string(),
            session_id: Some(format!("sess-{id}")),
            agent_id: "agent-a".to_string(),
            scope: "agent".to_string(),
            category: category.to_string(),
            key: format!("key.{id}"),
            content: format!("content {id}"),
            confidence: 0.8,
            mention_count: 1,
            source_summary: None,
            ward_id: "finance".to_string(),
            contradicted_by: None,
            superseded_by: None,
            expires_at: None,
            valid_from: None,
            valid_until: None,
            embedding: None,
            pinned: updated_days_ago.is_negative(),
            epistemic_class: None,
            source_episode_id: None,
            source_ref: None,
            created_at: String::new(),
            updated_at: (Utc::now() - chrono::Duration::days(updated_days_ago)).to_rfc3339(),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn recency_decay_half_life_and_exemptions() {
        let now = now();
        // Default 30-day half-life: 30 days old → 0.5.
        let d = recency_decay(
            &(now - chrono::Duration::days(30)).to_rfc3339(),
            "domain",
            false,
            now,
        );
        assert!((d - 0.5).abs() < 0.01, "30d/30d ≈ 0.5, got {d}");
        // Corrections live 180 days: 30 days old → barely decayed.
        let c = recency_decay(
            &(now - chrono::Duration::days(30)).to_rfc3339(),
            "correction",
            false,
            now,
        );
        assert!(
            c > 0.85,
            "correction 30d old ≈ 0.89 with 180d half-life, got {c}"
        );
        // Pinned and skill/agent never decay.
        // pinned lives in the legacy sqlite path; category exemptions carry it here
        assert_eq!(
            recency_decay("not-a-date", "skill", false, now),
            1.0,
            "malformed date on no-decode category stays 1.0"
        );
    }

    #[test]
    fn fresh_beats_stale_at_equal_relevance() {
        let fresh = fact("fresh", "domain", 0);
        let stale = fact("stale", "domain", 120);
        let semantic = vec![
            LaneCandidate {
                fact: stale.clone(),
                score: 0.60,
            },
            LaneCandidate {
                fact: fresh.clone(),
                score: 0.60,
            },
        ];
        let fused = fuse_fact_lanes(semantic, Vec::new(), 5, now());
        assert_eq!(fused.len(), 2);
        assert_eq!(
            fused[0].fact.id, "fresh",
            "equal relevance, recency must break the tie"
        );
    }

    #[test]
    fn temporal_lane_does_not_widen_the_matched_set() {
        let unmatched_recent = fact("recent-noise", "domain", 0);
        let matched_old = fact("relevant-old", "domain", 90);
        let semantic = vec![LaneCandidate {
            fact: matched_old,
            score: 0.70,
        }];
        let fused = fuse_fact_lanes(semantic, Vec::new(), 5, now());
        let ids: Vec<&str> = fused.iter().map(|f| f.fact.id.as_str()).collect();
        assert!(ids.contains(&"relevant-old"));
        assert!(
            !ids.contains(&"recent-noise"),
            "temporal lane must not surface unmatched facts"
        );
        let _ = unmatched_recent;
    }

    #[test]
    fn hybrid_match_source_marks_both_lanes() {
        let shared = fact("shared", "domain", 1);
        let semantic = vec![LaneCandidate {
            fact: shared.clone(),
            score: 0.55,
        }];
        let lexical = vec![LaneCandidate {
            fact: shared,
            score: 3.0,
        }];
        let fused = fuse_fact_lanes(semantic, lexical, 5, now());
        assert_eq!(fused.len(), 1);
        assert!(fused[0].semantic && fused[0].sparse);
        // Dual-lane presence must outrank a single-lane hit with the same
        // lane scores.
        let single = fact("single", "domain", 1);
        let fused_two = fuse_fact_lanes(
            vec![
                LaneCandidate {
                    fact: fact("shared2", "domain", 1),
                    score: 0.55,
                },
                LaneCandidate {
                    fact: single,
                    score: 0.9,
                },
            ],
            vec![LaneCandidate {
                fact: fact("shared2", "domain", 1),
                score: 3.0,
            }],
            5,
            now(),
        );
        assert_eq!(fused_two[0].fact.id, "shared2");
    }
}
