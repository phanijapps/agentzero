//! Maximal Marginal Relevance (MMR) diversity reranking.
//!
//! Given a ranked candidate list with relevance scores and embeddings, MMR
//! iteratively selects candidates that balance relevance against diversity
//! from already-selected items. The classic formulation:
//!
//! ```text
//! next = argmax over remaining of:
//!     lambda * relevance(c) - (1 - lambda) * max(sim(c, s) for s in Selected)
//! ```
//!
//! - `lambda = 1.0` → pure relevance ordering (degenerate to identity sort).
//! - `lambda = 0.0` → pure diversity (selects most-different remaining).
//! - `lambda = 0.6` (default) → balanced.
//!
//! Candidates with `None` embeddings degrade gracefully: their diversity
//! penalty contributes 0 (treated as orthogonal to everything), so their
//! relevance still applies and they remain selectable.

use crate::recall::ScoredItem;
use agent_primitives::vec_math::cosine_f64;

/// Input row for [`mmr_select`]. Pairs a [`ScoredItem`] with its optional
/// embedding vector.
pub struct MmrInput<'a> {
    pub item: &'a ScoredItem,
    pub embedding: Option<&'a [f32]>,
}

/// Select up to `target_count` items from `candidates` using MMR.
///
/// Returns indices into the input vector, in selection order. The first
/// pick is pure argmax over relevance (diversity term is `0` when the
/// `Selected` set is empty); subsequent picks apply the full formula.
///
/// Ties on the MMR score are broken by original input order (the first
/// candidate to reach the maximum wins), which preserves stability when
/// scores are equal — important so that callers see deterministic
/// results across runs.
///
/// When `lambda == 1.0`, the function short-circuits to a stable
/// relevance-only sort. When `target_count >= candidates.len()`, every
/// index is returned in MMR-selected order — the algorithm still runs so
/// the order reflects diversity even if no items are dropped.
pub fn mmr_select(candidates: Vec<MmrInput<'_>>, lambda: f64, target_count: usize) -> Vec<usize> {
    if candidates.is_empty() || target_count == 0 {
        return Vec::new();
    }

    let k = target_count.min(candidates.len());

    // Short-circuit: pure-relevance, stable sort.
    if lambda >= 1.0 {
        let mut indexed: Vec<(usize, f64)> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.item.score))
            .collect();
        indexed.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        return indexed.into_iter().take(k).map(|(i, _)| i).collect();
    }

    let mut selected: Vec<usize> = Vec::with_capacity(k);
    let mut chosen = vec![false; candidates.len()];
    let relevance_scale = candidates
        .iter()
        .map(|candidate| candidate.item.score)
        .filter(|score| score.is_finite() && *score > 0.0)
        .fold(0.0_f64, f64::max);

    while selected.len() < k {
        let mut best_idx: Option<usize> = None;
        let mut best_score: f64 = f64::NEG_INFINITY;

        for (idx, cand) in candidates.iter().enumerate() {
            if chosen[idx] {
                continue;
            }
            // RRF emits scores around 0.01 while cosine similarity is in
            // [0, 1]. Normalize relevance per candidate set so lambda keeps
            // the same relevance/diversity trade-off regardless of score
            // representation or number of fused sources.
            let relevance = if relevance_scale > 0.0 {
                cand.item.score / relevance_scale
            } else {
                0.0
            };
            let diversity_penalty = if selected.is_empty() {
                0.0
            } else {
                max_similarity_to_selected(cand.embedding, &candidates, &selected)
            };
            let mmr_score = lambda * relevance - (1.0 - lambda) * diversity_penalty;

            // Strictly greater — first encountered wins ties for input-order stability.
            if mmr_score > best_score {
                best_score = mmr_score;
                best_idx = Some(idx);
            }
        }

        match best_idx {
            Some(i) => {
                chosen[i] = true;
                selected.push(i);
            }
            // Defensive: all candidates exhausted — shouldn't trigger given
            // the `while` condition, but stay panic-free.
            None => break,
        }
    }

    selected
}

/// Maximum cosine similarity between `candidate_emb` and any embedding in
/// `selected`. Returns `0.0` when:
/// - `candidate_emb` is `None` (candidate has no embedding — treat as orthogonal),
/// - every selected item has `None` embedding (no defined similarity).
///
/// A selected item with `None` embedding contributes `0.0` to the max.
fn max_similarity_to_selected(
    candidate_emb: Option<&[f32]>,
    candidates: &[MmrInput<'_>],
    selected: &[usize],
) -> f64 {
    let Some(cand_emb) = candidate_emb else {
        return 0.0;
    };
    let mut max_sim = 0.0_f64;
    for &sel_idx in selected {
        let sel_emb = match candidates[sel_idx].embedding {
            Some(e) => e,
            None => continue,
        };
        let sim = cosine_f64(cand_emb, sel_emb);
        if sim > max_sim {
            max_sim = sim;
        }
    }
    max_sim
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recall::{ItemKind, Provenance};

    fn mk_item(id: &str, score: f64) -> ScoredItem {
        ScoredItem {
            kind: ItemKind::Fact,
            id: id.to_string(),
            content: id.to_string(),
            score,
            provenance: Provenance {
                source: "test".into(),
                source_id: id.into(),
                session_id: None,
                ward_id: None,
            },
            route_hint: None,
        }
    }

    #[test]
    fn empty_candidates_returns_empty() {
        let out = mmr_select(Vec::new(), 0.6, 5);
        assert!(out.is_empty());
    }

    #[test]
    fn zero_target_returns_empty() {
        let item = mk_item("a", 1.0);
        let emb = vec![1.0_f32, 0.0, 0.0];
        let candidates = vec![MmrInput {
            item: &item,
            embedding: Some(&emb),
        }];
        let out = mmr_select(candidates, 0.6, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn near_duplicate_diversifies_picks_one_first_then_distant() {
        // Items 0 and 1 are near-duplicates (same direction); item 2 is
        // orthogonal but with lower relevance. With lambda = 0.5, the
        // diversity penalty should push item 2 above item 1.
        let i0 = mk_item("a", 1.0);
        let i1 = mk_item("b", 0.95); // near-dup of a
        let i2 = mk_item("c", 0.6); // orthogonal, lower relevance

        let e0 = vec![1.0_f32, 0.0, 0.0];
        let e1 = vec![1.0_f32, 0.0001, 0.0]; // ~identical to e0
        let e2 = vec![0.0_f32, 1.0, 0.0]; // orthogonal

        let candidates = vec![
            MmrInput {
                item: &i0,
                embedding: Some(&e0),
            },
            MmrInput {
                item: &i1,
                embedding: Some(&e1),
            },
            MmrInput {
                item: &i2,
                embedding: Some(&e2),
            },
        ];

        let out = mmr_select(candidates, 0.5, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0, "highest relevance picked first");
        assert_eq!(
            out[1], 2,
            "orthogonal item beats near-duplicate due to diversity penalty"
        );
    }

    #[test]
    fn selection_is_invariant_to_positive_relevance_scale() {
        let low_high = mk_item("high", 0.020);
        let low_near = mk_item("near", 0.019);
        let low_distant = mk_item("distant", 0.001);
        let scaled_high = mk_item("high", 2.0);
        let scaled_near = mk_item("near", 1.9);
        let scaled_distant = mk_item("distant", 0.1);
        let high = vec![1.0_f32, 0.0];
        let distant = vec![0.0_f32, 1.0];

        let low = mmr_select(
            vec![
                MmrInput {
                    item: &low_high,
                    embedding: Some(&high),
                },
                MmrInput {
                    item: &low_near,
                    embedding: Some(&high),
                },
                MmrInput {
                    item: &low_distant,
                    embedding: Some(&distant),
                },
            ],
            0.6,
            2,
        );
        let scaled = mmr_select(
            vec![
                MmrInput {
                    item: &scaled_high,
                    embedding: Some(&high),
                },
                MmrInput {
                    item: &scaled_near,
                    embedding: Some(&high),
                },
                MmrInput {
                    item: &scaled_distant,
                    embedding: Some(&distant),
                },
            ],
            0.6,
            2,
        );

        assert_eq!(low, scaled, "MMR selection must be score-scale invariant");
        assert_eq!(low, vec![0, 1]);
    }

    #[test]
    fn all_orthogonal_returned_in_relevance_order() {
        let i0 = mk_item("a", 1.0);
        let i1 = mk_item("b", 0.8);
        let i2 = mk_item("c", 0.6);

        let e0 = vec![1.0_f32, 0.0, 0.0];
        let e1 = vec![0.0_f32, 1.0, 0.0];
        let e2 = vec![0.0_f32, 0.0, 1.0];

        let candidates = vec![
            MmrInput {
                item: &i0,
                embedding: Some(&e0),
            },
            MmrInput {
                item: &i1,
                embedding: Some(&e1),
            },
            MmrInput {
                item: &i2,
                embedding: Some(&e2),
            },
        ];

        let out = mmr_select(candidates, 0.6, 3);
        assert_eq!(out, vec![0, 1, 2]);
    }

    #[test]
    fn lambda_one_is_pure_relevance_sort() {
        let i0 = mk_item("a", 0.5);
        let i1 = mk_item("b", 1.0);
        let i2 = mk_item("c", 0.7);

        // Identical embeddings — diversity penalty would dominate at lambda < 1,
        // but lambda = 1.0 must ignore diversity entirely.
        let e = vec![1.0_f32, 0.0, 0.0];

        let candidates = vec![
            MmrInput {
                item: &i0,
                embedding: Some(&e),
            },
            MmrInput {
                item: &i1,
                embedding: Some(&e),
            },
            MmrInput {
                item: &i2,
                embedding: Some(&e),
            },
        ];

        let out = mmr_select(candidates, 1.0, 3);
        assert_eq!(out, vec![1, 2, 0], "purely sorted by descending relevance");
    }

    #[test]
    fn lambda_zero_picks_most_diverse() {
        // With lambda = 0, second pick should be the item most different
        // from the first. Items 0 (relevance 1.0) and 1 (relevance 0.99)
        // are near-duplicates; item 2 (relevance 0.5) is orthogonal.
        let i0 = mk_item("a", 1.0);
        let i1 = mk_item("b", 0.99);
        let i2 = mk_item("c", 0.5);

        let e0 = vec![1.0_f32, 0.0, 0.0];
        let e1 = vec![1.0_f32, 0.0, 0.0]; // identical to e0
        let e2 = vec![0.0_f32, 1.0, 0.0]; // orthogonal

        let candidates = vec![
            MmrInput {
                item: &i0,
                embedding: Some(&e0),
            },
            MmrInput {
                item: &i1,
                embedding: Some(&e1),
            },
            MmrInput {
                item: &i2,
                embedding: Some(&e2),
            },
        ];

        let out = mmr_select(candidates, 0.0, 2);
        // First pick: lambda * relevance - 0 = 0 for everyone (lambda=0
        // zeros out the relevance term too), but the formula evaluates to
        // 0.0 for all and ties break by input order → idx 0 wins.
        assert_eq!(out[0], 0);
        // Second pick: 0 - 1 * sim. sim(idx 1, idx 0) = 1.0 → -1.0;
        // sim(idx 2, idx 0) = 0.0 → 0.0. idx 2 wins by being more diverse.
        assert_eq!(out[1], 2);
    }

    #[test]
    fn null_embedding_still_selectable_no_panic() {
        // A candidate with None embedding should not crash; its diversity
        // term is 0, so its relevance score competes directly.
        let i0 = mk_item("a", 1.0);
        let i1 = mk_item("b", 0.8); // no embedding
        let i2 = mk_item("c", 0.6);

        let e0 = vec![1.0_f32, 0.0, 0.0];
        let e2 = vec![0.0_f32, 1.0, 0.0];

        let candidates = vec![
            MmrInput {
                item: &i0,
                embedding: Some(&e0),
            },
            MmrInput {
                item: &i1,
                embedding: None,
            },
            MmrInput {
                item: &i2,
                embedding: Some(&e2),
            },
        ];

        let out = mmr_select(candidates, 0.6, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 0, "highest relevance picks first");
        // After idx 0 is chosen, both idx 1 (no embedding → penalty=0,
        // relevance 0.8) and idx 2 (orthogonal → sim=0, penalty=0,
        // relevance 0.6) compete. At lambda=0.6, idx 1 wins
        // (0.6*0.8 = 0.48 vs 0.6*0.6 - 0.4*0 = 0.36).
        assert_eq!(out[1], 1, "null-embedding item still ranks by relevance");
        assert_eq!(out[2], 2);
    }

    #[test]
    fn target_exceeds_candidates_returns_all() {
        let i0 = mk_item("a", 1.0);
        let i1 = mk_item("b", 0.5);
        let e0 = vec![1.0_f32, 0.0];
        let e1 = vec![0.0_f32, 1.0];
        let candidates = vec![
            MmrInput {
                item: &i0,
                embedding: Some(&e0),
            },
            MmrInput {
                item: &i1,
                embedding: Some(&e1),
            },
        ];
        let out = mmr_select(candidates, 0.6, 100);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn cosine_similarity_handles_mismatched_lengths() {
        assert_eq!(cosine_f64(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_similarity_handles_zero_vectors() {
        assert_eq!(cosine_f64(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine_f64(&[1.0, 0.0], &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_similarity_identical_is_one() {
        let s = cosine_f64(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0]);
        assert!((s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let s = cosine_f64(&[1.0, 0.0], &[0.0, 1.0]);
        assert!(s.abs() < 1e-9);
    }
}
