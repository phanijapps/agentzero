//! Adapters: project each recall source into a uniform [`ScoredItem`].
//!
//! Callers construct source-specific results via existing repository search
//! methods and pass them through these pure functions together with a
//! per-source relevance score. The resulting `Vec<ScoredItem>` lists are
//! consumed by `rrf_merge`.

use crate::recall::scored_item::{ItemKind, Provenance, ScoredItem};
use zbot_stores::types::EntityId;
use zbot_stores_domain::{Belief, MemoryFact, Procedure, RouteHint, RouteSourceKind, WikiArticle};

/// Project a [`MemoryFact`] into a [`ScoredItem`].
///
/// The content string combines the category, key, and body so the caller's
/// downstream prompt always has full context without needing extra lookups.
pub fn fact_to_item(fact: &MemoryFact, score: f64) -> ScoredItem {
    ScoredItem {
        kind: ItemKind::Fact,
        id: fact.id.clone(),
        content: format!("[{}] {}: {}", fact.category, fact.key, fact.content),
        score,
        provenance: Provenance {
            source: "memory_facts".to_string(),
            source_id: fact.id.clone(),
            session_id: fact.session_id.clone(),
            ward_id: Some(fact.ward_id.clone()),
        },
        route_hint: Some(
            RouteHint::new(fact.ward_id.clone(), RouteSourceKind::Fact)
                .with_memory_id(fact.id.clone())
                .with_session_id(fact.session_id.clone())
                .with_source_path(fact.source_ref.clone()),
        ),
    }
}

/// Project a [`WikiArticle`] into a [`ScoredItem`].
///
/// The content string uses a Markdown heading so it renders cleanly when
/// injected into a prompt.
pub fn wiki_to_item(article: &WikiArticle, score: f64) -> ScoredItem {
    ScoredItem {
        kind: ItemKind::Wiki,
        id: article.id.clone(),
        content: format!("# {}\n{}", article.title, article.content),
        score,
        provenance: Provenance {
            source: "ward_wiki".to_string(),
            source_id: article.id.clone(),
            session_id: None,
            ward_id: Some(article.ward_id.clone()),
        },
        route_hint: Some(
            RouteHint::new(article.ward_id.clone(), RouteSourceKind::WikiArticle)
                .with_memory_id(article.id.clone()),
        ),
    }
}

/// Project a [`Belief`] into a [`ScoredItem`] (Phase B-4).
///
/// The content carries the belief's confidence inline so the consumer
/// prompt formatter can surface it without re-loading the row. Score
/// is the cosine similarity from `BeliefStore::search_beliefs`.
pub fn belief_to_item(belief: &Belief, score: f64) -> ScoredItem {
    ScoredItem {
        kind: ItemKind::Belief,
        id: belief.id.clone(),
        content: format!(
            "[belief {:.2}] {}: {}",
            belief.confidence, belief.subject, belief.content
        ),
        score,
        provenance: Provenance {
            source: "kg_beliefs".to_string(),
            source_id: belief.id.clone(),
            session_id: None,
            ward_id: Some(belief.partition_id.clone()),
        },
        route_hint: Some(
            RouteHint::new(belief.partition_id.clone(), RouteSourceKind::Belief)
                .with_memory_id(belief.id.clone()),
        ),
    }
}

/// Project a hierarchical-memory path entity into a [`ScoredItem`]
/// (Phase H-4 / LeanRAG LCA recall).
///
/// `score` is a layer-aware default the caller assigns; the recall
/// pipeline multiplies it by the configured category weight before
/// fusion. The content carries the layer + entity id so the consumer
/// formatter can render the topical map without re-querying the DB.
pub fn hier_entity_to_item(id: &EntityId, layer: i64, score: f64) -> ScoredItem {
    ScoredItem {
        kind: ItemKind::HierEntity,
        id: id.0.clone(),
        content: format!("[topic L{layer}] {}", id.0),
        score,
        provenance: Provenance {
            source: "kg_entities.hier".to_string(),
            source_id: id.0.clone(),
            session_id: None,
            ward_id: None,
        },
        route_hint: None,
    }
}

/// Project an inter-cluster relation (Phase H-4 follow-up) into a
/// [`ScoredItem`]. Rendered next to `HierEntity` items under the
/// topical map heading so the agent sees not only the abstraction
/// chain but the edges connecting sibling abstractions.
pub fn hier_relation_to_item(
    id: &str,
    source: &str,
    target: &str,
    relationship_type: &str,
    layer: i64,
    score: f64,
) -> ScoredItem {
    ScoredItem {
        kind: ItemKind::HierRelation,
        id: id.to_string(),
        content: format!("[edge L{layer}] {source} —[{relationship_type}]→ {target}"),
        score,
        provenance: Provenance {
            source: "kg_relationships.inter_cluster".to_string(),
            source_id: id.to_string(),
            session_id: None,
            ward_id: None,
        },
        route_hint: None,
    }
}

/// Project a [`Procedure`] into a [`ScoredItem`].
///
/// Combines name, description, and steps so the agent receives the full
/// how-to text without a second round-trip to the database.
pub fn procedure_to_item(proc: &Procedure, score: f64) -> ScoredItem {
    ScoredItem {
        kind: ItemKind::Procedure,
        id: proc.id.clone(),
        content: format!(
            "Procedure: {}\n{}\nSteps: {}",
            proc.name, proc.description, proc.steps
        ),
        score,
        provenance: Provenance {
            source: "procedures".to_string(),
            source_id: proc.id.clone(),
            session_id: None,
            ward_id: proc.ward_id.clone(),
        },
        route_hint: proc.ward_id.as_ref().map(|ward| {
            RouteHint::new(ward.clone(), RouteSourceKind::Procedure).with_memory_id(proc.id.clone())
        }),
    }
}

use std::sync::Arc;

/// ANN-query `kg_name_index` for entities whose name embedding is closest to
/// `query_embedding`, and project each hit as a [`ScoredItem::GraphNode`].
///
/// The returned `ScoredItem::score` is rank-discounted by cosine similarity
/// so higher-ranked, higher-similarity entities lead — `rrf_merge` re-scores
/// via rank during fusion, so this per-source score only matters for the
/// adapter-local order, which we preserve by sorting by `(rank, cosine)`.
pub async fn graph_ann_to_items(
    kg_store: &Arc<dyn zbot_stores::KnowledgeGraphStore>,
    query_embedding: &[f32],
    top_k: usize,
    agent_id: &str,
) -> Result<Vec<ScoredItem>, String> {
    if query_embedding.is_empty() || top_k == 0 {
        return Ok(Vec::new());
    }
    let results = kg_store
        .search_entities_by_name_embedding(agent_id, query_embedding, top_k)
        .await
        .map_err(|e| format!("graph ANN search failed: {e}"))?;

    let mut out = Vec::with_capacity(results.len());
    for (idx, hit) in results.into_iter().enumerate() {
        // L2-squared on normalized vectors → cosine similarity = 1 - dist/2.
        let cosine = 1.0 - (hit.distance as f64) / 2.0;
        let rank_one = (idx as f64) + 1.0;
        let score = (1.0 / rank_one) * cosine;
        out.push(ScoredItem {
            kind: ItemKind::GraphNode,
            id: format!("graph:{}", hit.name),
            content: format!(
                "Entity: {} [{}] (cosine ~ {cosine:.2})",
                hit.name, hit.entity_type
            ),
            score,
            provenance: Provenance {
                source: "kg_name_index".to_string(),
                source_id: hit.name,
                session_id: None,
                ward_id: None,
            },
            route_hint: None,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fact() -> MemoryFact {
        MemoryFact {
            id: "fact-1".to_string(),
            session_id: Some("sess-42".to_string()),
            agent_id: "agent-a".to_string(),
            scope: "local".to_string(),
            category: "user".to_string(),
            key: "name".to_string(),
            content: "Alice".to_string(),
            confidence: 0.95,
            mention_count: 3,
            source_summary: None,
            embedding: None,
            ward_id: "ward-x".to_string(),
            contradicted_by: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: None,
            source_episode_id: None,
            source_ref: None,
        }
    }

    fn make_article() -> WikiArticle {
        WikiArticle {
            id: "wiki-1".to_string(),
            ward_id: "ward-x".to_string(),
            agent_id: "agent-a".to_string(),
            title: "Project Alpha".to_string(),
            content: "Alpha is the first project.".to_string(),
            tags: None,
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_procedure() -> Procedure {
        Procedure {
            id: "proc-1".to_string(),
            agent_id: "agent-a".to_string(),
            ward_id: Some("ward-x".to_string()),
            name: "Deploy".to_string(),
            description: "Deploy to production.".to_string(),
            trigger_pattern: None,
            steps: "1. Build\n2. Push\n3. Restart".to_string(),
            parameters: None,
            success_count: 5,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn fact_adapter_populates_all_fields() {
        let fact = make_fact();
        let item = fact_to_item(&fact, 0.88);

        assert_eq!(item.kind, ItemKind::Fact);
        assert_eq!(item.id, "fact-1");
        assert!(!item.content.is_empty());
        assert!(item.content.contains("user"));
        assert!(item.content.contains("name"));
        assert!(item.content.contains("Alice"));
        assert!((item.score - 0.88).abs() < f64::EPSILON);
        assert_eq!(item.provenance.source, "memory_facts");
        assert_eq!(item.provenance.source_id, "fact-1");
        assert_eq!(item.provenance.session_id, Some("sess-42".to_string()));
        assert_eq!(item.provenance.ward_id, Some("ward-x".to_string()));
        let hint = item.route_hint.expect("fact route hint");
        assert_eq!(hint.ward_id, "ward-x");
        assert_eq!(hint.source_kind, RouteSourceKind::Fact);
        assert_eq!(hint.memory_id.as_deref(), Some("fact-1"));
        assert_eq!(hint.session_id.as_deref(), Some("sess-42"));
    }

    #[test]
    fn wiki_adapter_populates_all_fields() {
        let article = make_article();
        let item = wiki_to_item(&article, 0.75);

        assert_eq!(item.kind, ItemKind::Wiki);
        assert_eq!(item.id, "wiki-1");
        assert!(!item.content.is_empty());
        assert!(item.content.contains("Project Alpha"));
        assert!(item.content.contains("Alpha is the first project."));
        assert!((item.score - 0.75).abs() < f64::EPSILON);
        assert_eq!(item.provenance.source, "ward_wiki");
        assert_eq!(item.provenance.source_id, "wiki-1");
        assert_eq!(item.provenance.session_id, None);
        assert_eq!(item.provenance.ward_id, Some("ward-x".to_string()));
        let hint = item.route_hint.expect("wiki route hint");
        assert_eq!(hint.ward_id, "ward-x");
        assert_eq!(hint.source_kind, RouteSourceKind::WikiArticle);
        assert_eq!(hint.memory_id.as_deref(), Some("wiki-1"));
    }

    #[test]
    fn procedure_adapter_populates_all_fields() {
        let proc = make_procedure();
        let item = procedure_to_item(&proc, 0.60);

        assert_eq!(item.kind, ItemKind::Procedure);
        assert_eq!(item.id, "proc-1");
        assert!(!item.content.is_empty());
        assert!(item.content.contains("Deploy"));
        assert!(item.content.contains("Deploy to production."));
        assert!(item.content.contains("1. Build"));
        assert!((item.score - 0.60).abs() < f64::EPSILON);
        assert_eq!(item.provenance.source, "procedures");
        assert_eq!(item.provenance.source_id, "proc-1");
        assert_eq!(item.provenance.session_id, None);
        assert_eq!(item.provenance.ward_id, Some("ward-x".to_string()));
        let hint = item.route_hint.expect("procedure route hint");
        assert_eq!(hint.ward_id, "ward-x");
        assert_eq!(hint.source_kind, RouteSourceKind::Procedure);
        assert_eq!(hint.memory_id.as_deref(), Some("proc-1"));
    }
}
