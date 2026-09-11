//! Context-atom projection for unified recall results.
//!
//! This module is intentionally one-way and side-effect free: recall still
//! ranks `ScoredItem`s exactly as before, and later context-packet assembly can
//! consume these typed atoms without reparsing rendered prompt text.

use agent_runtime::{ContextActorKind, ContextAtom, ContextRenderPolicy, DroppedContextCandidate};
use chrono::{DateTime, Utc};
use zbot_stores_domain::{MemoryFact, RouteHint, RouteSourceKind, ScoredFact};

use crate::recall::{ItemKind, Provenance, ScoredItem};

/// Project a scored recall list into context atoms.
pub fn scored_items_to_context_atoms(items: &[ScoredItem]) -> Vec<ContextAtom> {
    items.iter().map(scored_item_to_context_atom).collect()
}

/// Project one scored recall item into a typed context atom.
pub fn scored_item_to_context_atom(item: &ScoredItem) -> ContextAtom {
    ContextAtom {
        id: item.id.clone(),
        kind: item_kind_name(&item.kind).to_string(),
        content: item.content.clone(),
        score: item.score,
        confidence: confidence_from_score(item.score),
        source: item.provenance.source.clone(),
        source_id: Some(item.provenance.source_id.clone()),
        provenance: provenance_handles(&item.provenance),
        valid_from: None,
        valid_until: None,
        visibility: default_visibility(),
        route_hint: item
            .route_hint
            .clone()
            .and_then(|hint| serde_json::to_value(hint).ok()),
        token_estimate: estimate_tokens(&item.content),
        render_policy: render_policy_for_kind(&item.kind),
    }
}

/// Project a scored memory fact into a context atom with fact-native validity
/// and contradiction/supersession provenance.
pub fn scored_fact_to_context_atom(scored: &ScoredFact) -> ContextAtom {
    let fact = &scored.fact;
    ContextAtom {
        id: fact.id.clone(),
        kind: "memory_fact".to_string(),
        content: fact.content.clone(),
        score: scored.score,
        confidence: fact.confidence.clamp(0.0, 1.0),
        source: "memory_facts".to_string(),
        source_id: Some(fact.id.clone()),
        provenance: fact_provenance_handles(fact),
        valid_from: parse_rfc3339(fact.valid_from.as_deref()),
        valid_until: parse_rfc3339(fact.valid_until.as_deref()),
        visibility: default_visibility(),
        route_hint: serde_json::to_value(
            RouteHint::new(fact.ward_id.clone(), RouteSourceKind::Fact)
                .with_memory_id(fact.id.clone())
                .with_session_id(fact.session_id.clone()),
        )
        .ok(),
        token_estimate: estimate_tokens(&fact.content),
        render_policy: ContextRenderPolicy::Inline,
    }
}

/// Return a dropped-candidate record for facts that recall should omit because
/// a newer fact superseded them.
pub fn dropped_candidate_for_superseded_fact(fact: &MemoryFact) -> Option<DroppedContextCandidate> {
    fact.superseded_by
        .as_ref()
        .map(|newer| DroppedContextCandidate {
            id: fact.id.clone(),
            reason: format!("superseded_by:{newer}"),
        })
}

fn item_kind_name(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Fact => "memory_fact",
        ItemKind::Wiki => "wiki",
        ItemKind::Procedure => "procedure",
        ItemKind::GraphNode => "graph_node",
        ItemKind::Goal => "goal",
        ItemKind::Episode => "episode",
        ItemKind::Belief => "belief",
        ItemKind::HierEntity => "hierarchy_entity",
        ItemKind::HierRelation => "hierarchy_relation",
    }
}

fn render_policy_for_kind(kind: &ItemKind) -> ContextRenderPolicy {
    match kind {
        ItemKind::Fact | ItemKind::Belief | ItemKind::Goal => ContextRenderPolicy::Inline,
        ItemKind::Wiki
        | ItemKind::Procedure
        | ItemKind::GraphNode
        | ItemKind::Episode
        | ItemKind::HierEntity
        | ItemKind::HierRelation => ContextRenderPolicy::Summary,
    }
}

fn default_visibility() -> Vec<ContextActorKind> {
    vec![
        ContextActorKind::Root,
        ContextActorKind::DelegatedExecutor,
        ContextActorKind::WardAgent,
    ]
}

fn provenance_handles(provenance: &Provenance) -> Vec<String> {
    let mut handles = vec![format!("{}:{}", provenance.source, provenance.source_id)];
    if let Some(session_id) = &provenance.session_id {
        handles.push(format!("session:{session_id}"));
    }
    if let Some(ward_id) = &provenance.ward_id {
        handles.push(format!("ward:{ward_id}"));
    }
    handles
}

fn fact_provenance_handles(fact: &MemoryFact) -> Vec<String> {
    let mut handles = vec![format!("memory_facts:{}", fact.id)];
    if let Some(session_id) = &fact.session_id {
        handles.push(format!("session:{session_id}"));
    }
    if !fact.ward_id.is_empty() {
        handles.push(format!("ward:{}", fact.ward_id));
    }
    if let Some(contradicted_by) = &fact.contradicted_by {
        handles.push(format!("penalty:contradicted_by:{contradicted_by}"));
    }
    if let Some(superseded_by) = &fact.superseded_by {
        handles.push(format!("drop:superseded_by:{superseded_by}"));
    }
    handles
}

fn parse_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn confidence_from_score(score: f64) -> f64 {
    if score.is_finite() {
        score.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn estimate_tokens(content: &str) -> u32 {
    let chars = content.chars().count();
    chars.div_ceil(4).max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbot_stores_domain::{RouteHint, RouteSourceKind};

    fn item(kind: ItemKind, id: &str, score: f64) -> ScoredItem {
        ScoredItem {
            kind,
            id: id.to_string(),
            content: format!("content for {id}"),
            score,
            provenance: Provenance {
                source: "test_source".to_string(),
                source_id: id.to_string(),
                session_id: Some("sess-1".to_string()),
                ward_id: Some("ward-1".to_string()),
            },
            route_hint: None,
        }
    }

    fn memory_fact(id: &str) -> MemoryFact {
        MemoryFact {
            id: id.to_string(),
            session_id: Some("sess-1".to_string()),
            agent_id: "agent-1".to_string(),
            scope: "agent".to_string(),
            category: "domain".to_string(),
            key: format!("fact.{id}"),
            content: "fact content".to_string(),
            confidence: 0.82,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "finance".to_string(),
            contradicted_by: None,
            created_at: "2026-07-07T12:00:00Z".to_string(),
            updated_at: "2026-07-07T12:00:00Z".to_string(),
            expires_at: None,
            valid_from: Some("2026-07-01T00:00:00Z".to_string()),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
            last_accessed: None,
            importance: None,
        }
    }

    #[test]
    fn maps_all_recall_item_kinds_to_context_atom_kinds() {
        let items = vec![
            item(ItemKind::Fact, "fact", 0.8),
            item(ItemKind::Wiki, "wiki", 0.7),
            item(ItemKind::Procedure, "procedure", 0.6),
            item(ItemKind::GraphNode, "graph", 0.5),
            item(ItemKind::Goal, "goal", 1.2),
            item(ItemKind::Episode, "episode", 0.4),
            item(ItemKind::Belief, "belief", 0.9),
            item(ItemKind::HierEntity, "hier-entity", 0.3),
            item(ItemKind::HierRelation, "hier-relation", 0.2),
        ];

        let atoms = scored_items_to_context_atoms(&items);
        let kinds: Vec<_> = atoms.iter().map(|atom| atom.kind.as_str()).collect();

        assert_eq!(
            kinds,
            vec![
                "memory_fact",
                "wiki",
                "procedure",
                "graph_node",
                "goal",
                "episode",
                "belief",
                "hierarchy_entity",
                "hierarchy_relation",
            ]
        );
        assert_eq!(atoms[4].confidence, 1.0, "confidence is bounded");
        assert_eq!(atoms[0].render_policy, ContextRenderPolicy::Inline);
        assert_eq!(atoms[1].render_policy, ContextRenderPolicy::Summary);
    }

    #[test]
    fn maps_provenance_visibility_route_hint_and_token_estimate() {
        let mut scored = item(ItemKind::Fact, "fact-1", f64::NAN);
        scored.content = "abcd efgh".to_string();
        scored.route_hint =
            Some(RouteHint::new("ward-1", RouteSourceKind::Fact).with_memory_id("fact-1"));

        let atom = scored_item_to_context_atom(&scored);

        assert_eq!(atom.confidence, 0.0, "non-finite confidence is bounded");
        assert_eq!(atom.source, "test_source");
        assert_eq!(atom.source_id.as_deref(), Some("fact-1"));
        assert_eq!(
            atom.provenance,
            vec![
                "test_source:fact-1".to_string(),
                "session:sess-1".to_string(),
                "ward:ward-1".to_string(),
            ]
        );
        assert_eq!(
            atom.visibility,
            vec![
                ContextActorKind::Root,
                ContextActorKind::DelegatedExecutor,
                ContextActorKind::WardAgent,
            ]
        );
        assert_eq!(atom.token_estimate, 3);

        let route_hint = atom.route_hint.as_ref().expect("route hint serialized");
        assert_eq!(route_hint["ward_id"], "ward-1");
        assert_eq!(route_hint["source_kind"], "fact");
        assert_eq!(route_hint["memory_id"], "fact-1");
        assert!(serde_json::to_string(&atom)
            .expect("atom serializes")
            .find("embedding")
            .is_none());
    }

    #[test]
    fn scored_fact_projection_preserves_validity_and_penalty_provenance() {
        let mut fact = memory_fact("fact-1");
        fact.contradicted_by = Some("fact-2".to_string());
        fact.valid_until = Some("2026-07-05T00:00:00Z".to_string());
        let scored = ScoredFact { fact, score: 0.42 };

        let atom = scored_fact_to_context_atom(&scored);

        assert_eq!(atom.id, "fact-1");
        assert_eq!(atom.kind, "memory_fact");
        assert_eq!(atom.confidence, 0.82);
        assert_eq!(
            atom.valid_from.unwrap().to_rfc3339(),
            "2026-07-01T00:00:00+00:00"
        );
        assert_eq!(
            atom.valid_until.unwrap().to_rfc3339(),
            "2026-07-05T00:00:00+00:00"
        );
        assert!(atom
            .provenance
            .contains(&"penalty:contradicted_by:fact-2".to_string()));
        assert_eq!(atom.route_hint.as_ref().unwrap()["source_kind"], "fact");
    }

    #[test]
    fn superseded_fact_can_emit_dropped_candidate_reason() {
        let mut fact = memory_fact("fact-old");
        fact.superseded_by = Some("fact-new".to_string());

        let dropped = dropped_candidate_for_superseded_fact(&fact).expect("superseded fact drops");

        assert_eq!(dropped.id, "fact-old");
        assert_eq!(dropped.reason, "superseded_by:fact-new");

        let scored = ScoredFact { fact, score: 0.1 };
        let atom = scored_fact_to_context_atom(&scored);
        assert!(atom
            .provenance
            .contains(&"drop:superseded_by:fact-new".to_string()));
    }
}
