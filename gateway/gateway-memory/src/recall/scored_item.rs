//! Unified retrievable item — every recall source (facts, wiki, procedures,
//! graph, goals) projects into `ScoredItem` so they compete in one pool.
//!
//! Each source adapter produces a `Vec<ScoredItem>` ordered by its own
//! scoring. Lists fuse via engram's weighted ReciprocalRankFusion in
//! `fuse_source_lists` (recall/mod.rs): same item across sources sums
//! weighted rank-reciprocal contributions.

use zbot_stores_domain::RouteHint;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Fact,
    Wiki,
    Procedure,
    GraphNode,
    Goal,
    Episode,
    /// Synthesized belief from the Belief Network (Phase B-4).
    /// Surfaces alongside facts in `recall_unified`; the consumer
    /// (`gateway-execution::recall::format_scored_items`) renders these
    /// under a dedicated `## Active Beliefs` heading so the agent can
    /// distinguish aggregated stances from raw facts.
    Belief,
    /// LCA-path entity from the hierarchical-memory builder
    /// (Phase H-4 / LeanRAG). Recall walks `parent_cluster_id` up from
    /// the top-N seed entities to their lowest common ancestor and
    /// emits each ancestor on the path as a `HierEntity`. The consumer
    /// renders these under a dedicated `## Topical Map` heading so the
    /// agent can see the abstraction chain without confusing it with
    /// the base entities.
    HierEntity,
    /// Inter-cluster edge between two aggregate entities at the same
    /// hierarchy layer (Phase H-4 follow-up). Built by the
    /// HierarchyBuilder when λ > τ; surfaced by recall when both
    /// endpoints sit on the LCA path of the active query. The "lean"
    /// part of LeanRAG — the edges that name how abstract concepts
    /// relate. Rendered next to `HierEntity` under the topical map.
    HierRelation,
}

#[derive(Debug, Clone)]
pub struct Provenance {
    pub source: String,
    pub source_id: String,
    pub session_id: Option<String>,
    pub ward_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScoredItem {
    pub kind: ItemKind,
    pub id: String,
    pub content: String,
    pub score: f64,
    pub provenance: Provenance,
    pub route_hint: Option<RouteHint>,
}

/// Lightweight snapshot of an active goal — enough for intent-boost computation.
/// Caller builds this from `GoalRepository::list_active`, extracting unfilled
/// slot names by diffing `slots` ↔ `filled_slots` (JSON parsing happens at
/// caller site; this struct stays simple).
#[derive(Debug, Clone)]
pub struct GoalLite {
    pub id: String,
    pub title: String,
    pub unfilled_slot_names: Vec<String>,
}

/// Boost items whose content mentions any unfilled-goal slot name.
/// MemGuide-style: aligned items get a 1.3× multiplier in place.
///
/// Matching is case-insensitive substring containment — deliberately naive,
/// broadly effective. Phase 4 can promote to embedding-based slot alignment
/// if measurements show false-positive rates matter.
pub fn intent_boost(items: &mut [ScoredItem], active_goals: &[GoalLite]) {
    if active_goals.is_empty() {
        return;
    }
    let tokens: Vec<String> = active_goals
        .iter()
        .flat_map(|g| g.unfilled_slot_names.iter().map(|s| s.to_lowercase()))
        .collect();
    if tokens.is_empty() {
        return;
    }
    for item in items.iter_mut() {
        let lower = item.content.to_lowercase();
        if tokens.iter().any(|t| lower.contains(t)) {
            item.score *= 1.3;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, kind: ItemKind, score: f64) -> ScoredItem {
        ScoredItem {
            kind,
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
    fn intent_boost_multiplies_matching_content() {
        let mut items = vec![
            mk("a", ItemKind::Fact, 1.0),       // content = "a"
            mk("tickers", ItemKind::Fact, 1.0), // content = "tickers"
        ];
        let goals = vec![GoalLite {
            id: "g1".into(),
            title: "portfolio".into(),
            unfilled_slot_names: vec!["tickers".into()],
        }];
        intent_boost(&mut items, &goals);
        let a_score = items.iter().find(|i| i.id == "a").unwrap().score;
        let t_score = items.iter().find(|i| i.id == "tickers").unwrap().score;
        assert!((a_score - 1.0).abs() < 1e-9, "non-matching score unchanged");
        assert!((t_score - 1.3).abs() < 1e-9, "matching score × 1.3");
    }

    #[test]
    fn intent_boost_no_goals_is_noop() {
        let mut items = vec![mk("x", ItemKind::Fact, 1.0)];
        intent_boost(&mut items, &[]);
        assert_eq!(items[0].score, 1.0);
    }

    #[test]
    fn intent_boost_is_case_insensitive() {
        let mut items = vec![mk("x", ItemKind::Fact, 1.0)];
        // Force content to contain mixed-case match.
        items[0].content = "Portfolio of TICKERS".to_string();
        let goals = vec![GoalLite {
            id: "g1".into(),
            title: "t".into(),
            unfilled_slot_names: vec!["tickers".into()],
        }];
        intent_boost(&mut items, &goals);
        assert!((items[0].score - 1.3).abs() < 1e-9);
    }
}
