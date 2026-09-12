//! Free recall helpers — pure functions moved verbatim from mod.rs.
//! Query bounding, visibility markers, taxonomy tracing, fusion glue,
//! and the trait-fact score normalizer.

use super::*;

pub(super) fn recall_embedding_queries(text: &str) -> Vec<String> {
    let primary = bounded_recall_embedding_query(text, MAX_RECALL_EMBED_QUERY_CHARS);
    let retry = bounded_recall_embedding_query(text, RETRY_RECALL_EMBED_QUERY_CHARS);
    if primary == retry {
        vec![primary]
    } else {
        vec![primary, retry]
    }
}

pub(super) fn profile_fact_key_sets_for_query(query: &str) -> Vec<&'static [&'static str]> {
    let normalized = query.to_ascii_lowercase();
    let mut key_sets = Vec::new();
    if ["my name", "who am i", "who i am", "my identity", "call me"]
        .iter()
        .any(|cue| normalized.contains(cue))
    {
        key_sets.push(IDENTITY_PROFILE_KEYS);
    }
    if [
        "weather",
        "forecast",
        "temperature",
        "air quality",
        "aqi",
        "near me",
        "nearby",
        "where am i",
        "where i live",
    ]
    .iter()
    .any(|cue| normalized.contains(cue))
    {
        key_sets.push(LOCATION_PROFILE_KEYS);
    }
    key_sets
}

pub(super) fn apply_scoped_candidate_visibility(
    items: &mut Vec<ScoredItem>,
    visible: &(dyn Fn(&ScoredItem) -> bool + Send + Sync),
) {
    // Source adapters must attach an explicit, trusted scope before a
    // candidate reaches this point. Missing metadata is not a synonym for the
    // current request scope or a global record: the gateway predicate denies
    // it before telemetry, traversal, RRF, MMR, and model output.
    items.retain(|item| visible(item));
}

pub(super) fn mark_durable_global_session(mut item: ScoredItem) -> ScoredItem {
    item.provenance.session_id = Some(GLOBAL_SESSION_SCOPE.to_string());
    item
}

pub(super) fn mark_agent_global_scope(mut item: ScoredItem) -> ScoredItem {
    item.provenance.ward_id = Some(GLOBAL_SESSION_SCOPE.to_string());
    item.provenance.session_id = Some(GLOBAL_SESSION_SCOPE.to_string());
    item
}

pub(super) fn source_status(
    configured: bool,
    requires_embedding: bool,
    embedding_available: bool,
    count: usize,
    unavailable: bool,
) -> UnifiedRecallSourceStatus {
    if !configured {
        UnifiedRecallSourceStatus::not_configured()
    } else if unavailable {
        UnifiedRecallSourceStatus::unavailable(count)
    } else if requires_embedding && !embedding_available {
        UnifiedRecallSourceStatus::embedding_unavailable()
    } else {
        UnifiedRecallSourceStatus::used_or_empty(count)
    }
}

pub(super) fn taxonomy_outcome_trace(
    expansion: &zbot_stores_traits::RecallTaxonomyExpansion,
) -> UnifiedRecallTaxonomyTrace {
    UnifiedRecallTaxonomyTrace {
        retrieval_query: bounded_recall_embedding_query(
            &expansion.expanded_query,
            MAX_RECALL_EMBED_QUERY_CHARS,
        ),
        candidates: expansion
            .candidates
            .iter()
            .map(|candidate| UnifiedRecallTaxonomyCandidate {
                scheme_id: bounded_outcome_text(
                    &candidate.scheme_id,
                    MAX_OUTCOME_TAXONOMY_FIELD_CHARS,
                ),
                concept_id: bounded_outcome_text(
                    &candidate.concept_id,
                    MAX_OUTCOME_TAXONOMY_FIELD_CHARS,
                ),
                label: bounded_outcome_text(&candidate.label, MAX_OUTCOME_TAXONOMY_FIELD_CHARS),
                relation: normalized_taxonomy_relation(candidate),
            })
            .collect(),
    }
}

pub(super) fn normalized_taxonomy_relation(
    candidate: &RecallTaxonomyExpansionCandidate,
) -> UnifiedRecallTaxonomyRelation {
    match candidate.relation.as_deref() {
        Some("pref_label") => UnifiedRecallTaxonomyRelation::PrefLabel,
        Some("alt_label") => UnifiedRecallTaxonomyRelation::AltLabel,
        Some("broader") => UnifiedRecallTaxonomyRelation::Broader,
        Some("narrower") => UnifiedRecallTaxonomyRelation::Narrower,
        Some("related") => UnifiedRecallTaxonomyRelation::Related,
        _ if candidate.matched_label == candidate.label => UnifiedRecallTaxonomyRelation::PrefLabel,
        _ => UnifiedRecallTaxonomyRelation::AltLabel,
    }
}

pub(super) fn bounded_outcome_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub(super) fn bounded_recall_embedding_query(text: &str, max_chars: usize) -> String {
    let compact = compact_recall_embedding_query(text);
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect()
}

pub(super) fn compact_recall_embedding_query(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn taxonomy_trace(
    candidates: &[RecallTaxonomyExpansionCandidate],
) -> Vec<serde_json::Value> {
    candidates
        .iter()
        .map(|candidate| {
            serde_json::json!({
                "schemeId": candidate.scheme_id,
                "conceptId": candidate.concept_id,
                "label": candidate.label,
                "matchedLabel": candidate.matched_label,
                "relation": candidate.relation,
                "depth": candidate.depth })
        })
        .collect()
}

pub(super) fn is_embedding_context_length_error(error: &EmbeddingError) -> bool {
    let msg = error.to_string().to_ascii_lowercase();
    (msg.contains("input length") && msg.contains("context length"))
        || msg.contains("maximum context")
        || msg.contains("too many tokens")
        || msg.contains("context window")
}

/// Fuse the per-source candidate lists with engram's weighted Reciprocal
/// Rank Fusion (`ReciprocalRankFusion`). Replaces gateway-memory's own
/// Weighted fusion over the unified source lists: per-source weights encode source trust (goals steer
/// hardest, facts/procedures lead content, traversal/hierarchy are
/// supporting context), and engram's `FusionTrace` keeps every fused item
/// explainable. Fused scores are normalized back onto the ≈[0, 1) scale
/// the packet builders expect (multiply by `RRF_K`, then saturate).
pub(super) fn fuse_source_lists(
    lists: Vec<Vec<ScoredItem>>,
    budget: usize,
    intent_boosted: bool,
) -> Vec<ScoredItem> {
    use engram_domain::{
        Actor, ActorKind, AllowedUse, DeleteMode, FusionStrategy, Id, Policy, Provenance,
        Retention, RetrievalMode, RetrievalRequest, RetrievalResult, RetrievalScore,
        RetrievalTargetType, Scope, Sensitivity, Visibility,
    };
    use engram_retrieval::{ReciprocalFusionConfig, ReciprocalRankFusion, RetrievalFusion as _};
    use std::collections::BTreeMap;

    const RRF_K: u32 = 60;
    let source_weight = |kind: &ItemKind| -> f32 {
        match kind {
            ItemKind::Goal => 1.2,
            ItemKind::Procedure => 1.1,
            ItemKind::Fact => 1.0,
            ItemKind::Wiki | ItemKind::Belief => 0.9,
            ItemKind::GraphNode | ItemKind::Episode => 0.8,
            ItemKind::HierEntity | ItemKind::HierRelation => 0.7,
        }
    };

    // Group lists by dominant kind (lists are homogeneous by construction)
    // and record per-item weights for the trace-driven weight lookup.
    let mut candidates: Vec<RetrievalResult> = Vec::new();
    let mut items_by_id: std::collections::HashMap<String, ScoredItem> =
        std::collections::HashMap::new();
    let mut source_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for list in lists {
        let Some(first) = list.first() else {
            continue;
        };
        let kind_name = match first.kind {
            ItemKind::Fact => "facts",
            ItemKind::Wiki => "wiki",
            ItemKind::Procedure => "procedures",
            ItemKind::GraphNode => "graph",
            ItemKind::Goal => "goals",
            ItemKind::Episode => "episodes",
            ItemKind::Belief => "beliefs",
            ItemKind::HierEntity | ItemKind::HierRelation => "hierarchy",
        };
        // Traversal items share ItemKind::GraphNode with ANN seeds; their
        // provenance separates them (kg_traversal vs kg_name_index).
        let kind_weight = source_weight(&first.kind);
        for (rank, item) in list.into_iter().enumerate() {
            let source = match item.provenance.source.as_str() {
                "kg_traversal" => "traversal".to_string(),
                _ => kind_name.to_string(),
            };
            let weight = if source == "traversal" {
                0.7
            } else {
                kind_weight
            };
            let id = item.id.clone();
            items_by_id.insert(id.clone(), item);
            source_names.insert(id.clone(), source.clone());
            let score = items_by_id[&id].score;
            candidates.push(RetrievalResult {
                id: format!("{source}:{id}"),
                target_type: RetrievalTargetType::Memory,
                target_id: id.clone(),
                content: String::new(),
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
                    source: "zbot_unified_recall".to_string(),
                    actor: Actor {
                        id: Id::from("gateway-memory"),
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
                fusion_trace: Some(engram_domain::FusionTrace {
                    query_id: None,
                    vector_index: None,
                    embedding_time_ms: None,
                    search_time_ms: None,
                    source,
                    source_rank: Some((rank + 1) as u32),
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
            });
            let _ = weight;
        }
    }

    // Per-source weights for the fuser: lookup by trace source name.
    let source_weights: BTreeMap<String, f32> = [
        ("goals".to_string(), 1.2),
        ("procedures".to_string(), 1.1),
        ("facts".to_string(), 1.0),
        ("wiki".to_string(), 0.9),
        ("beliefs".to_string(), 0.9),
        ("graph".to_string(), 0.8),
        ("episodes".to_string(), 0.8),
        ("traversal".to_string(), 0.7),
        ("hierarchy".to_string(), 0.7),
    ]
    .into_iter()
    .collect();

    let fusion = ReciprocalRankFusion::new(
        ReciprocalFusionConfig::new(RRF_K, 1.0, source_weights)
            .unwrap_or_else(|_| ReciprocalFusionConfig::default()),
    );
    let request = RetrievalRequest {
        limit: Some(budget as u32),
        query: String::new(),
        scope: Scope {
            tenant: "zbot".to_string(),
            subject: None,
            workspace: None,
            session: None,
            environment: None,
        },
        requester: engram_domain::Requester {
            actor: Actor {
                id: Id::from("gateway-memory"),
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
        budget: None,
        include_explanations: None,
    };

    let Ok(fused) = fusion.fuse(&request, candidates) else {
        return Vec::new();
    };
    let _ = intent_boosted;
    fused
        .into_iter()
        .filter_map(|result| {
            let mut item = items_by_id.remove(&result.target_id)?;
            let scaled = (result.score.total as f64) * (RRF_K as f64);
            item.score = scaled / (1.0 + scaled);
            Some(item)
        })
        .collect()
}

pub(super) fn normalized_trait_fact_score(value: &serde_json::Value) -> f64 {
    let raw = value
        .get("score")
        .and_then(|score| score.as_f64())
        .unwrap_or(0.5);
    let match_source = value
        .get("match_source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if raw > 0.0 && raw < 0.3 && matches!(match_source, "hybrid" | "vec" | "fts") {
        (raw * 60.0).min(1.0)
    } else {
        raw
    }
}
