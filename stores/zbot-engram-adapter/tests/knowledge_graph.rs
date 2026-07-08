use chrono::Utc;
use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use serde_json::json;
use zbot_engram_adapter::{
    mapping::knowledge::{entity_to_knowledge_entity, knowledge_entity_to_entity},
    AdapterConfig, AdapterFeature, CapabilityReport, EngramKnowledgeGraphStore, EngramWikiStore,
    GovernancePolicy, GovernanceSelection, ZBOT_BASE_ONTOLOGY_ID,
};
use zbot_stores::{types::Direction, KnowledgeGraphStore};
use zbot_stores_domain::WikiArticle;
use zbot_stores_traits::{EmbeddingQueryIdentity, WikiStore};

fn engram_config(root: &tempfile::TempDir) -> AdapterConfig {
    AdapterConfig::engram_for_data_root(root.path(), "engram.db")
}

fn engram_config_with_embedding_dimensions(
    root: &tempfile::TempDir,
    dimensions: u32,
) -> AdapterConfig {
    let mut config = engram_config(root);
    config.embedding_provider.dimensions = dimensions;
    config
}

fn governed_engram_config(root: &tempfile::TempDir) -> AdapterConfig {
    let mut config = engram_config(root);
    config.governance = GovernancePolicy {
        default_selection: GovernanceSelection {
            ontology_ids: vec![ZBOT_BASE_ONTOLOGY_ID.to_string()],
            taxonomy_scheme_ids: Vec::new(),
        },
        ..GovernancePolicy::default()
    };
    config
}

fn query_identity(dimensions: u32) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: "fastembed".to_string(),
        model: "BAAI/bge-small-en-v1.5".to_string(),
        dimensions,
        prompt_profile: "query".to_string(),
        normalization: None,
    }
}

fn article(id: &str, ward_id: &str, title: &str, content: &str) -> WikiArticle {
    let now = Utc::now().to_rfc3339();
    WikiArticle {
        id: id.to_string(),
        ward_id: ward_id.to_string(),
        agent_id: "agent-a".to_string(),
        title: title.to_string(),
        content: content.to_string(),
        tags: Some("rust,engram".to_string()),
        source_fact_ids: Some("fact-1,fact-2".to_string()),
        embedding: None,
        version: 1,
        created_at: now.clone(),
        updated_at: now,
    }
}

#[test]
fn knowledge_mapping_preserves_dynamic_policy_metadata() {
    let config = AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db");
    let mapper = config.scope_mapper().expect("scope mapper");
    let mut entity = Entity::new(
        "agent-a".to_string(),
        EntityType::Concept,
        "Dynamic Ontology".to_string(),
    );
    entity.id = "entity-policy-1".to_string();
    entity
        .properties
        .insert("ward_id".to_string(), json!("ward-alpha"));
    entity
        .properties
        .insert("ontology_id".to_string(), json!("ontology.dynamic"));
    entity
        .properties
        .insert("taxonomy_id".to_string(), json!("taxonomy.runtime"));

    let mapped = entity_to_knowledge_entity(&entity, &mapper).expect("entity mapping");
    let round_trip = knowledge_entity_to_entity(&mapped).expect("entity round trip");

    assert_eq!(mapped.scope.workspace.as_deref(), Some("ward-alpha"));
    assert_eq!(
        mapped
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("ontologyId")),
        Some(&json!("ontology.dynamic"))
    );
    assert_eq!(
        mapped
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("taxonomyId")),
        Some(&json!("taxonomy.runtime"))
    );
    assert_eq!(round_trip.id, entity.id);
    assert_eq!(round_trip.properties["ontology_id"], "ontology.dynamic");
}

#[tokio::test]
async fn wiki_articles_round_trip_through_engram_knowledge() {
    let root = tempfile::tempdir().expect("root");
    let store =
        EngramWikiStore::open(engram_config_with_embedding_dimensions(&root, 3)).expect("store");
    let mut article = article(
        "wiki-1",
        "ward-alpha",
        "Compaction Notes",
        "Rig style compaction keeps boundaries explicit.",
    );

    store
        .upsert_article(
            serde_json::to_value(&article).expect("article json"),
            Some(vec![0.25, 0.5, 0.75]),
        )
        .await
        .expect("upsert");

    let listed = store
        .list_articles("ward-alpha")
        .await
        .expect("list articles");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["title"], "Compaction Notes");

    let fetched = store
        .get_article("ward-alpha", "Compaction Notes")
        .await
        .expect("get")
        .expect("article");
    assert_eq!(fetched["content"], article.content);
    assert_eq!(
        store.get_article_embedding("wiki-1").expect("embedding"),
        Some(vec![0.25, 0.5, 0.75])
    );
    assert!(store
        .search_wiki_by_similarity_typed("ward-alpha", &[0.25, 0.5, 0.75], 10)
        .await
        .expect("legacy wiki vector search")
        .is_empty());
    assert_eq!(
        store
            .search_wiki_by_similarity_typed_with_identity(
                "ward-alpha",
                &[0.25, 0.5, 0.75],
                Some(&query_identity(3)),
                10
            )
            .await
            .expect("wiki vector search")
            .len(),
        1
    );
    let mut same_basename = query_identity(3);
    same_basename.model = "other/bge-small-en-v1.5".to_string();
    assert!(store
        .search_wiki_by_similarity_typed_with_identity(
            "ward-alpha",
            &[0.25, 0.5, 0.75],
            Some(&same_basename),
            10
        )
        .await
        .expect("same-basename wiki mismatch")
        .is_empty());

    article.content = "Updated content about Engram knowledge mapping.".to_string();
    store
        .upsert_article(serde_json::to_value(&article).expect("article json"), None)
        .await
        .expect("update");
    let updated = store
        .get_article("ward-alpha", "Compaction Notes")
        .await
        .expect("get")
        .expect("article");
    assert_eq!(updated["version"], 2);

    let hits = store
        .search_wiki_hybrid(Some("ward-alpha"), "Engram mapping", 10, None)
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["article"]["id"], "wiki-1");
    assert_eq!(hits[0]["match_source"], "fts");

    assert!(store
        .delete_article("ward-alpha", "Compaction Notes")
        .await
        .expect("delete"));
    assert!(store
        .list_articles("ward-alpha")
        .await
        .expect("list after delete")
        .is_empty());
}

#[tokio::test]
async fn graph_entities_relationships_and_read_models_round_trip() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config_with_embedding_dimensions(&root, 3))
        .expect("store");
    let mut alice = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    alice.id = "entity-alice".to_string();
    alice.name_embedding = Some(vec![1.0, 0.0, 0.0]);
    let mut project = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    project.id = "entity-zbot".to_string();
    project.name_embedding = Some(vec![0.0, 1.0, 0.0]);

    let alice_id = store
        .upsert_entity("agent-a", alice.clone())
        .await
        .expect("alice");
    let project_id = store
        .upsert_entity("agent-a", project.clone())
        .await
        .expect("project");
    store.add_alias(&alice_id, "Ada").await.expect("alias");
    assert!(matches!(
        store
            .resolve_entity("agent-a", &EntityType::Person, "Ada", None)
            .await
            .expect("resolve"),
        zbot_stores::types::ResolveOutcome::Match(found) if found == alice_id
    ));

    let mut rel = Relationship::new(
        "agent-a".into(),
        alice_id.0.clone(),
        project_id.0.clone(),
        RelationshipType::Created,
    );
    rel.id = "rel-created".to_string();
    let rel_id = store
        .upsert_relationship("agent-a", rel)
        .await
        .expect("relationship");

    let neighbors = store
        .get_neighbors(&alice_id, Direction::Outgoing, 10)
        .await
        .expect("neighbors");
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].entity_id, project_id);

    let full = store
        .get_neighbors_full("agent-a", "entity-alice", Direction::Outgoing, 10)
        .await
        .expect("full neighbors");
    assert_eq!(full[0].entity.name, "ZBot");

    let traversal = store.traverse(&alice_id, 2, 10).await.expect("traverse");
    assert!(traversal.iter().any(|hit| hit.entity_id == project_id));

    let graph_stats = store.graph_stats("agent-a").await.expect("graph stats");
    assert_eq!(graph_stats.entity_count, 2);
    assert_eq!(graph_stats.relationship_count, 1);

    let subgraph = store
        .get_subgraph("agent-a", "entity-alice", 2)
        .await
        .expect("subgraph");
    assert_eq!(subgraph.center, "entity-alice");
    assert_eq!(subgraph.relationships.len(), 1);

    assert_eq!(store.count_all_entities().await.expect("entity count"), 2);
    assert_eq!(
        store
            .search_entities_by_name("agent-a", "zbot", 10)
            .await
            .expect("search")[0]
            .id,
        "entity-zbot"
    );
    assert!(store
        .search_entities_by_name_embedding("agent-a", &[1.0, 0.0, 0.0], 1)
        .await
        .expect("legacy embedding search")
        .is_empty());
    assert_eq!(
        store
            .search_entities_by_name_embedding_with_identity(
                "agent-a",
                &[1.0, 0.0, 0.0],
                Some(&query_identity(3)),
                1
            )
            .await
            .expect("embedding search")[0]
            .id,
        "entity-alice"
    );
    let mut same_basename = query_identity(3);
    same_basename.model = "other/bge-small-en-v1.5".to_string();
    assert!(store
        .search_entities_by_name_embedding_with_identity(
            "agent-a",
            &[1.0, 0.0, 0.0],
            Some(&same_basename),
            1
        )
        .await
        .expect("same-basename mismatch")
        .is_empty());

    store
        .delete_relationship(&rel_id)
        .await
        .expect("delete rel");
    assert_eq!(
        store
            .list_relationships("agent-a", None, 10, 0)
            .await
            .expect("rels")
            .len(),
        0
    );
}

#[tokio::test]
async fn advisory_governance_findings_do_not_block_relationship_writes() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(governed_engram_config(&root)).expect("store");
    let mut source = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    source.id = "entity-governed-alice".to_string();
    let mut target = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    target.id = "entity-governed-zbot".to_string();
    let source_id = store
        .upsert_entity("agent-a", source)
        .await
        .expect("source");
    let target_id = store
        .upsert_entity("agent-a", target)
        .await
        .expect("target");
    let mut relationship = Relationship::new(
        "agent-a".into(),
        source_id.0.clone(),
        target_id.0.clone(),
        RelationshipType::Custom("indexes_secret_path".to_string()),
    );
    relationship.id = "rel-governance-custom".to_string();
    relationship.properties.insert(
        "raw_context".to_string(),
        json!("absolute path /home/example/Documents/zbot/providers.json with placeholder TOKEN_VALUE"),
    );

    let relationship_id = store
        .upsert_relationship("agent-a", relationship)
        .await
        .expect("advisory write succeeds");

    assert_eq!(
        store
            .list_relationships("agent-a", None, 10, 0)
            .await
            .expect("relationships")
            .len(),
        1
    );
    let findings = store
        .list_governance_findings(Some("agent-a"), 10)
        .expect("findings");
    assert_eq!(relationship_id.0, "rel-governance-custom");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "unknown_predicate");
    let finding_json = serde_json::to_string(&findings[0]).expect("finding json");
    assert!(!finding_json.contains("/home/example"));
    assert!(!finding_json.contains("providers.json"));
    assert!(!finding_json.contains("TOKEN_VALUE"));
    assert!(!finding_json.contains("raw_context"));
}

#[tokio::test]
async fn advisory_governance_records_domain_and_range_mismatches() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(governed_engram_config(&root)).expect("store");
    let mut source = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    source.id = "entity-source-project".to_string();
    let mut target = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    target.id = "entity-target-person".to_string();
    let source_id = store
        .upsert_entity("agent-a", source)
        .await
        .expect("source");
    let target_id = store
        .upsert_entity("agent-a", target)
        .await
        .expect("target");
    let mut relationship = Relationship::new(
        "agent-a".into(),
        source_id.0,
        target_id.0,
        RelationshipType::WorksFor,
    );
    relationship.id = "rel-governance-mismatch".to_string();

    store
        .upsert_relationship("agent-a", relationship)
        .await
        .expect("advisory write succeeds");

    let mut codes = store
        .list_governance_findings(Some("agent-a"), 10)
        .expect("findings")
        .into_iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    codes.sort();
    assert_eq!(codes, vec!["domain_mismatch", "range_mismatch"]);
}

#[tokio::test]
async fn hierarchy_aggregate_summary_and_path_round_trip() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let a = store
        .upsert_entity(
            "agent-a",
            Entity {
                id: "entity-a".to_string(),
                ..Entity::new("agent-a".into(), EntityType::Concept, "A".into())
            },
        )
        .await
        .expect("a");
    let b = store
        .upsert_entity(
            "agent-a",
            Entity {
                id: "entity-b".to_string(),
                ..Entity::new("agent-a".into(), EntityType::Concept, "B".into())
            },
        )
        .await
        .expect("b");
    let c = store
        .upsert_entity(
            "agent-a",
            Entity {
                id: "entity-c".to_string(),
                ..Entity::new("agent-a".into(), EntityType::Concept, "C".into())
            },
        )
        .await
        .expect("c");

    let aggregate = store
        .promote_cluster_to_aggregate(
            "agent-a",
            1,
            &[a.clone(), b.clone()],
            "AB Cluster",
            "A and B are clustered",
            Some(vec![0.1, 0.9]),
        )
        .await
        .expect("aggregate");
    let other = store
        .promote_cluster_to_aggregate(
            "agent-a",
            1,
            std::slice::from_ref(&c),
            "C Cluster",
            "C is alone",
            None,
        )
        .await
        .expect("other aggregate");
    store
        .write_inter_cluster_relation("agent-a", 1, &aggregate, &other, "related-via")
        .await
        .expect("inter relation");

    let summary = store
        .hierarchy_summary("agent-a", 10)
        .await
        .expect("summary");
    assert!(summary
        .layer_counts
        .iter()
        .any(|(layer, count)| { *layer == 1 && *count >= 2 }));
    assert_eq!(summary.inter_cluster_relations, 1);
    assert_eq!(summary.top_aggregates[0].id, aggregate.0);

    let embeddings = store
        .list_entities_with_embeddings_at_layer("agent-a", 1, 10)
        .await
        .expect("layer embeddings");
    assert!(embeddings.iter().any(|row| row.id == aggregate));

    let path = store
        .compute_lca_path("agent-a", &[a, b])
        .await
        .expect("lca");
    assert_eq!(path.lca, Some(aggregate.clone()));
    assert!(path.path_entities.contains(&aggregate));

    let inter = store
        .list_inter_cluster_relations("agent-a", &[aggregate.clone(), other])
        .await
        .expect("inter");
    assert_eq!(inter[0].relationship_type, "related-via");
}

#[test]
fn knowledge_capabilities_can_be_enabled_independently() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config(&root);

    let report = CapabilityReport::from_verified_features(
        &config,
        [
            AdapterFeature::Wiki,
            AdapterFeature::KnowledgeGraph,
            AdapterFeature::Hierarchy,
        ],
    );

    assert!(report.supports(AdapterFeature::Wiki));
    assert!(report.supports(AdapterFeature::KnowledgeGraph));
    assert!(report.supports(AdapterFeature::Hierarchy));
    assert!(!report.supports(AdapterFeature::Beliefs));
    assert!(!report.supports(AdapterFeature::Recall));
    assert!(!report.supports(AdapterFeature::Migration));
}
