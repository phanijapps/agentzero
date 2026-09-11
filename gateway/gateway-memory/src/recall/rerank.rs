//! Cross-encoder rerank stage for unified recall (the precision lever).
//!
//! Wires `engram-rerank-cross-encoder` between weighted-RRF fusion and
//! MMR: the top `pool` fused candidates are scored query-vs-content by
//! an LLM-backed [`RerankScorer`] and reordered best-first. Fail-open by
//! contract — any error, timeout, or unwired scorer keeps the fused
//! order; recall must never break because the reranker did.
//!
//! The scorer implements engram's port trait synchronously
//! (`score(&self, query, candidate) -> CoreResult<f32>`); the LLM call
//! inside is async, bridged with `block_in_place` (the daemon runs the
//! multi-thread runtime). The stage wraps the whole rerank in
//! [`tokio::time::timeout`] so a hung scorer cannot stall recall past
//! `timeout_ms`.

use std::sync::Arc;

use engram_domain::{
    Actor, ActorKind, AllowedUse, DeleteMode, Policy, Provenance, Retention, RetrievalResult,
    RetrievalScore, RetrievalTargetType, Sensitivity, Visibility,
};
use engram_rerank_cross_encoder::CrossEncoderReranker;
use engram_runtime::CoreResult;

use crate::recall::scored_item::ScoredItem;

/// Configuration for the rerank stage, threaded like [`crate::MmrConfig`]
/// (`memory.rerank.*` in settings.json).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RerankConfig {
    /// Master switch. Default on — the stage is the production precision
    /// lever and fail-opens on any scorer problem. Set `memory.rerank.enabled
    /// = false` to skip the stage entirely (fused order preserved).
    pub enabled: bool,
    /// How many fused candidates to rerank (top-N). Bounded so one LLM
    /// scoring session stays cheap. Default 20.
    pub pool: usize,
    /// Wall-clock budget for the whole stage. Default 2000ms.
    pub timeout_ms: u64,
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            pool: 20,
            timeout_ms: 2000,
        }
    }
}

/// The rerank stage: engram's `CrossEncoderReranker` plus the config it
/// runs under. Constructed by the bootstrap only when a scoring client
/// is available; absent → stage skipped.
pub struct RerankStage {
    reranker: Arc<CrossEncoderReranker>,
    pub config: RerankConfig,
}

impl RerankStage {
    pub fn new(
        scorer: Arc<dyn engram_rerank_cross_encoder::RerankScorer>,
        config: RerankConfig,
    ) -> Self {
        Self {
            reranker: Arc::new(CrossEncoderReranker::new(scorer)),
            config,
        }
    }

    /// Will this stage run for a fused list of this length?
    pub fn will_run(&self, fused_len: usize) -> bool {
        self.config.enabled && fused_len > 1
    }

    /// Rerank the top `pool` items of `fused` (query-aware, stable on
    /// ties), keeping the remainder in fused order. Fail-open: any
    /// scorer error or timeout returns the input unchanged.
    pub async fn apply(&self, query: &str, fused: Vec<ScoredItem>) -> Vec<ScoredItem> {
        if !self.will_run(fused.len()) {
            return fused;
        }
        let pool_len = self.config.pool.min(fused.len());
        let (head, tail) = fused.split_at(pool_len);

        let head_results: Vec<RetrievalResult> = head.iter().map(item_to_result).collect();
        let reranked =
            match tokio::time::timeout(std::time::Duration::from_millis(self.config.timeout_ms), {
                let reranker = self.reranker.clone();
                let query = query.to_string();
                async move { reranker.rerank(&query, head_results, None) }
            })
            .await
            {
                Ok(Ok(reranked)) => reranked,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "cross-encoder rerank failed; keeping fused order");
                    let mut out: Vec<ScoredItem> = head.to_vec();
                    out.extend(tail.to_vec());
                    return out;
                }
                Err(_) => {
                    tracing::warn!("cross-encoder rerank timed out; keeping fused order");
                    let mut out: Vec<ScoredItem> = head.to_vec();
                    out.extend(tail.to_vec());
                    return out;
                }
            };

        // Map reranked order back onto the original ScoredItems by id.
        let by_id: std::collections::HashMap<String, &ScoredItem> =
            head.iter().map(|item| (item.id.clone(), item)).collect();
        let mut out: Vec<ScoredItem> = Vec::with_capacity(fused.len());
        let mut taken = std::collections::HashSet::new();
        for result in &reranked {
            if let Some(item) = by_id.get(&result.target_id) {
                if taken.insert(result.target_id.clone()) {
                    out.push((*item).clone());
                }
            }
        }
        // Any item the scorer dropped (should not happen — rerank preserves
        // candidates) is appended in fused order so nothing is lost.
        for item in head {
            if !taken.contains(&item.id) {
                out.push(item.clone());
            }
        }
        out.extend(tail.to_vec());
        out
    }
}

/// Minimal `RetrievalResult` projection for the reranker — only `content`
/// participates in scoring; identity round-trips via `target_id`.
fn item_to_result(item: &ScoredItem) -> RetrievalResult {
    RetrievalResult {
        id: format!("rerank:{}", item.id),
        target_type: RetrievalTargetType::Memory,
        target_id: item.id.clone(),
        content: item.content.clone(),
        score: RetrievalScore {
            total: item.score as f32,
            relevance: Some(item.score as f32),
            recency: None,
            confidence: None,
            cue_match: None,
            hierarchical_fit: None,
            policy_fit: None,
        },
        provenance: Provenance {
            source: "zbot_unified_recall".to_string(),
            actor: Actor {
                id: engram_domain::Id::from("zbot-recall"),
                kind: ActorKind::System,
                display_name: None,
                metadata: None,
            },
            observed_at: chrono::Utc::now(),
            evidence: Vec::new(),
            derivations: Vec::new(),
            confidence: None,
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
        fusion_trace: None,
        metadata: None,
    }
}

/// LLM-backed scorer over the shared cached client. One tiny prompt per
/// (query, content) pair; parse a 0–1 float; any failure is an error the
/// stage converts into fail-open.
pub struct LlmRerankScorer {
    client: Arc<crate::CachedLlmClient>,
}

impl LlmRerankScorer {
    pub fn new(client: Arc<crate::CachedLlmClient>) -> Self {
        Self { client }
    }
}

impl engram_rerank_cross_encoder::RerankScorer for LlmRerankScorer {
    fn score(&self, query: &str, candidate_text: &str) -> CoreResult<f32> {
        // The port trait is sync; the LLM call is async. Bridge with
        // block_in_place on the multi-thread runtime the daemon runs.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = self
                    .client
                    .get()
                    .await
                    .map_err(|error| engram_runtime::CoreError::InvalidRequest {
                        reason: format!("rerank client unavailable: {error}"),
                    })?;
                let response = client
                    .chat(
                        vec![
                            agent_runtime::llm::ChatMessage::system(
                                "Rate how relevant the CANDIDATE is to the QUERY for answering it. \
                                 Reply with ONLY a number between 0.0 (irrelevant) and 1.0 (directly answers)."
                                    .to_string(),
                            ),
                            agent_runtime::llm::ChatMessage::user(format!(
                                "QUERY: {query}\n\nCANDIDATE:\n{}",
                                truncate(candidate_text, 2000)
                            )),
                        ],
                        None,
                    )
                    .await
                    .map_err(|error| engram_runtime::CoreError::InvalidRequest {
                        reason: format!("rerank score call failed: {error}"),
                    })?;
                let text = response.content;
                // Models occasionally append punctuation or prose; strip a
                // trailing sentence period and take the LAST numeric token
                // (reasoning models may lead with text).
                let trimmed = last_numeric_token(&text);
                parsed_score(trimmed)
            })
        })
    }
}

fn last_numeric_token(text: &str) -> &str {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    tokens
        .iter()
        .rev()
        .find(|token| token.chars().any(|c| c.is_ascii_digit()))
        .copied()
        .unwrap_or_else(|| text.trim())
}

fn parsed_score(text: &str) -> CoreResult<f32> {
    let value: f32 = text.trim().trim_end_matches('.').parse().map_err(|_| {
        engram_runtime::CoreError::InvalidRequest {
            reason: format!("rerank score was not a number: {text:?}"),
        }
    })?;
    if !(0.0..=1.0).contains(&value) {
        return Err(engram_runtime::CoreError::InvalidRequest {
            reason: format!("rerank score out of [0,1]: {value}"),
        });
    }
    Ok(value)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let head: String = text.chars().take(limit).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recall::scored_item::{ItemKind, Provenance};

    fn item(id: &str, content: &str, score: f64) -> ScoredItem {
        ScoredItem {
            kind: ItemKind::Fact,
            id: id.to_string(),
            content: content.to_string(),
            score,
            provenance: Provenance {
                source: "test".to_string(),
                source_id: id.to_string(),
                session_id: None,
                ward_id: None,
            },
            route_hint: None,
        }
    }

    struct RankScorer;
    impl engram_rerank_cross_encoder::RerankScorer for RankScorer {
        fn score(&self, _query: &str, candidate_text: &str) -> CoreResult<f32> {
            // Score by the first char of the candidate: 'z' > 'y' > 'x'.
            let rank = candidate_text.chars().next().unwrap_or('a');
            Ok(match rank {
                'z' => 0.9,
                'y' => 0.5,
                _ => 0.1,
            })
        }
    }

    struct ErrorScorer;
    impl engram_rerank_cross_encoder::RerankScorer for ErrorScorer {
        fn score(&self, _query: &str, _candidate_text: &str) -> CoreResult<f32> {
            Err(engram_runtime::CoreError::InvalidRequest {
                reason: "boom".to_string(),
            })
        }
    }

    fn stage(scorer: Arc<dyn engram_rerank_cross_encoder::RerankScorer>) -> RerankStage {
        RerankStage::new(
            scorer,
            RerankConfig {
                enabled: true,
                pool: 20,
                timeout_ms: 2000,
            },
        )
    }

    #[tokio::test]
    async fn rerank_reorders_when_scores_distinct() {
        let fused = vec![
            item("a", "xxx low", 0.9),
            item("b", "yyy mid", 0.8),
            item("c", "zzz high", 0.7),
        ];
        let out = stage(Arc::new(RankScorer)).apply("query", fused).await;
        let ids: Vec<&str> = out.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "b", "a"], "cross-encoder order wins");
    }

    #[tokio::test]
    async fn fail_open_on_scorer_error_keeps_fused_order() {
        let fused = vec![item("a", "one", 0.9), item("b", "two", 0.8)];
        let out = stage(Arc::new(ErrorScorer)).apply("query", fused).await;
        let ids: Vec<&str> = out.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "error must not reorder or drop");
    }

    #[tokio::test]
    async fn pool_truncates_head_only_tail_kept_fused_order() {
        let mut fused = Vec::new();
        for index in 0..10 {
            fused.push(item(&format!("i{index}"), "xxx", 1.0 - index as f64 * 0.01));
        }
        let small_pool = RerankStage::new(
            Arc::new(RankScorer),
            RerankConfig {
                enabled: true,
                pool: 3,
                timeout_ms: 2000,
            },
        );
        let out = small_pool.apply("query", fused).await;
        assert_eq!(out.len(), 10, "no items lost");
        // All pool candidates are 'x'-scored (0.1) so fused order persists;
        // the tail (i3..i9) must follow in original order.
        let ids: Vec<&str> = out.iter().map(|i| i.id.as_str()).collect();
        let tail: Vec<&str> = ids.iter().skip(3).copied().collect();
        assert_eq!(tail, vec!["i3", "i4", "i5", "i6", "i7", "i8", "i9"]);
    }

    #[tokio::test]
    async fn kill_switch_skips_stage() {
        let cfg = RerankConfig {
            enabled: false,
            ..RerankConfig::default()
        };
        let stag = RerankStage::new(Arc::new(ErrorScorer), cfg);
        assert!(!stag.will_run(5));
        let out = stag
            .apply("query", vec![item("a", "x", 1.0), item("b", "y", 0.9)])
            .await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "a", "disabled stage is a no-op");
    }

    #[test]
    fn parses_scores_and_rejects_garbage() {
        assert!((parsed_score("0.75").unwrap() - 0.75).abs() < 1e-6);
        assert!(parsed_score("0.9.").is_ok());
        assert!(parsed_score("high").is_err());
        assert!(parsed_score("1.5").is_err());
        assert!(parsed_score("-0.2").is_err());
    }
}
