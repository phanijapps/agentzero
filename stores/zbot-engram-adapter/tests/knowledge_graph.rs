use chrono::Utc;
use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use serde_json::json;
use zbot_engram_adapter::{
    mapping::knowledge::{entity_to_knowledge_entity, knowledge_entity_to_entity},
    AdapterConfig, AdapterFeature, CapabilityReport, EngramKnowledgeGraphStore, EngramWikiStore,
    GovernancePolicy, GovernanceSelection, ZBOT_BASE_ONTOLOGY_ID, ZBOT_GENERAL_SCHEME_ID,
};
use zbot_stores::{types::Direction, ExtractedKnowledge, KnowledgeGraphStore};
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
    governed_engram_config_with_ids(root, ZBOT_BASE_ONTOLOGY_ID, ZBOT_GENERAL_SCHEME_ID)
}

fn governed_engram_config_with_ids(
    root: &tempfile::TempDir,
    ontology_id: &str,
    taxonomy_scheme_id: &str,
) -> AdapterConfig {
    let mut config = engram_config(root);
    if ontology_id != ZBOT_BASE_ONTOLOGY_ID || taxonomy_scheme_id != ZBOT_GENERAL_SCHEME_ID {
        let config_root = root.path().join("config");
        let governance_dir = config_root.join("governance");
        std::fs::create_dir_all(&governance_dir).expect("governance directory");
        std::fs::write(
            governance_dir.join("changed-ontology.json"),
            format!(
                r#"{{
                  "kind": "zbot.ontology",
                  "schemaVersion": 1,
                  "ontologyId": "{ontology_id}",
                  "label": "Changed Ontology",
                  "entityClasses": [
                    {{ "id": "person", "label": "Person" }},
                    {{ "id": "project", "label": "Project" }}
                  ],
                  "relationshipProperties": [
                    {{ "id": "created", "label": "Created" }}
                  ]
                }}"#
            ),
        )
        .expect("ontology definition");
        std::fs::write(
            governance_dir.join("changed-taxonomy.json"),
            format!(
                r#"{{
                  "kind": "zbot.skos_taxonomy",
                  "schemaVersion": 1,
                  "schemeId": "{taxonomy_scheme_id}",
                  "label": "Changed Taxonomy",
                  "concepts": [{{ "id": "memory", "prefLabel": "Memory" }}]
                }}"#
            ),
        )
        .expect("taxonomy definition");
        config = config.with_trusted_config_root(config_root);
        config.governance.ontology_definition_paths =
            vec!["governance/changed-ontology.json".into()];
        config.governance.taxonomy_definition_paths =
            vec!["governance/changed-taxonomy.json".into()];
    }
    config.governance = GovernancePolicy {
        ontology_definition_paths: config.governance.ontology_definition_paths,
        taxonomy_definition_paths: config.governance.taxonomy_definition_paths,
        default_selection: GovernanceSelection {
            ontology_ids: vec![ontology_id.to_string()],
            taxonomy_scheme_ids: vec![taxonomy_scheme_id.to_string()],
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
        .upsert_article(article.clone(), Some(vec![0.25, 0.5, 0.75]))
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
        .upsert_article(article.clone(), None)
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
async fn normalized_entity_lookup_is_exact_case_and_whitespace_insensitive() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let mut entity = Entity::new(
        "agent-normalized".into(),
        EntityType::Organization,
        "AgentZero".into(),
    );
    entity.id = "entity-agentzero".to_string();
    store
        .upsert_entity("agent-normalized", entity)
        .await
        .expect("store entity");

    // These would fill the old `search_entities_by_name(..., 16)` ranked
    // window, leaving the exact match below it. The normalized lookup must
    // remain an exact query instead of regressing to ranked substring search.
    for index in 0..16 {
        let mut distractor = Entity::new(
            "agent-normalized".into(),
            EntityType::Organization,
            format!("agentzero-distractor-{index}"),
        );
        distractor.id = format!("entity-agentzero-distractor-{index}");
        let distractor_id = store
            .upsert_entity("agent-normalized", distractor)
            .await
            .expect("store distractor");
        store
            .bump_entity_mention(&distractor_id)
            .await
            .expect("promote distractor");
    }

    let found = store
        .get_entity_by_normalized_name("agent-normalized", "  agentzero  ")
        .await
        .expect("normalized lookup")
        .expect("case variant resolves");

    assert_eq!(found.id, "entity-agentzero");
    assert!(
        store
            .get_entity_by_normalized_name("agent-normalized", "agentzero-missing")
            .await
            .expect("missing normalized lookup")
            .is_none(),
        "normalized lookup is exact, not a substring search"
    );
}

#[tokio::test]
async fn graph_entities_relationships_and_read_models_round_trip() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config_with_embedding_dimensions(&root, 3))
        .expect("store");
    let mut alice = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    alice.id = "entity-alice".to_string();
    alice.name_embedding = Some(vec![1.0, 0.0, 0.0]);
    alice
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    let mut project = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    project.id = "entity-zbot".to_string();
    project.name_embedding = Some(vec![0.0, 1.0, 0.0]);
    project
        .properties
        .insert("ward_id".to_string(), json!("ward-b"));

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
    let all_entities = store
        .list_all_entities(None, None, 1)
        .await
        .expect("all entities");
    assert_eq!(all_entities.len(), 1);
    assert_eq!(
        all_entities[0].id, "entity-alice",
        "aggregate entity list should preserve mention-count ordering"
    );
    let project_entities = store
        .list_all_entities(None, Some("project"), 10)
        .await
        .expect("project entities");
    assert_eq!(project_entities.len(), 1);
    assert_eq!(project_entities[0].id, "entity-zbot");
    let ward_entities = store
        .list_all_entities(Some("ward-a"), None, 10)
        .await
        .expect("ward entities");
    assert_eq!(ward_entities.len(), 1);
    assert_eq!(ward_entities[0].id, "entity-alice");
    let all_relationships = store
        .list_all_relationships(10)
        .await
        .expect("all relationships");
    assert_eq!(all_relationships.len(), 1);
    assert_eq!(all_relationships[0].id, "rel-created");
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

/// Aggregate counters must only read indexed scalar columns.  A malformed
/// JSON payload is deliberately inserted to prove the counter never
/// deserializes graph records as the old implementation did.
#[tokio::test]
async fn aggregate_counts_do_not_deserialize_graph_payloads() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config(&root);
    let sidecar_path = config
        .compatibility_store_path("zbot-knowledge-graph.sqlite")
        .expect("sidecar path");
    let store = EngramKnowledgeGraphStore::open(config).expect("store");
    let connection = rusqlite::Connection::open(sidecar_path).expect("sidecar connection");
    let now = Utc::now().to_rfc3339();

    connection
        .execute(
            "INSERT INTO kg_entities \
                (id, agent_id, entity_type, name, first_seen_at, last_seen_at, mention_count, \
                 properties_json, entity_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "invalid-count-entity",
                "agent-a",
                "person",
                "Invalid payload",
                now,
                now,
                1_i64,
                "{}",
                "not valid entity json",
            ],
        )
        .expect("insert entity");
    connection
        .execute(
            "INSERT INTO kg_relationships \
                (id, agent_id, source_entity_id, target_entity_id, relationship_type, \
                 first_seen_at, last_seen_at, mention_count, properties_json, relationship_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                "invalid-count-relationship",
                "agent-a",
                "source",
                "target",
                "uses",
                now,
                now,
                1_i64,
                "{}",
                "not valid relationship json",
            ],
        )
        .expect("insert relationship");

    assert_eq!(store.count_all_entities().await.expect("entity count"), 1);
    assert_eq!(
        store
            .count_all_relationships()
            .await
            .expect("relationship count"),
        1
    );
}

/// Hierarchy health is fetched whenever the Observatory opens. Its summary
/// must therefore aggregate indexed columns without loading every graph JSON
/// payload when hierarchy is enabled on a large workspace.
#[tokio::test]
async fn hierarchy_summary_does_not_deserialize_graph_payloads() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config(&root);
    let sidecar_path = config
        .compatibility_store_path("zbot-knowledge-graph.sqlite")
        .expect("sidecar path");
    let store = EngramKnowledgeGraphStore::open(config).expect("store");
    let connection = rusqlite::Connection::open(sidecar_path).expect("sidecar connection");
    let now = Utc::now().to_rfc3339();

    connection
        .execute(
            "INSERT INTO kg_entities \
                (id, agent_id, entity_type, name, first_seen_at, last_seen_at, mention_count, \
                 properties_json, entity_json, layer) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                "invalid-hierarchy-entity",
                "agent-a",
                "concept",
                "Aggregate",
                now,
                now,
                1_i64,
                r#"{"member_count": 42, "description": "A bounded summary."}"#,
                "not valid entity json",
                1_i64,
            ],
        )
        .expect("insert aggregate");
    connection
        .execute(
            "INSERT INTO kg_relationships \
                (id, agent_id, source_entity_id, target_entity_id, relationship_type, \
                 first_seen_at, last_seen_at, mention_count, properties_json, relationship_json, \
                 is_inter_cluster) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "invalid-hierarchy-relationship",
                "agent-a",
                "source",
                "target",
                "related_to",
                now,
                now,
                1_i64,
                "{}",
                "not valid relationship json",
                1_i64,
            ],
        )
        .expect("insert inter-cluster relation");

    let summary = store
        .hierarchy_summary("agent-a", 10)
        .await
        .expect("hierarchy summary");
    assert_eq!(summary.layer_counts, vec![(1, 1)]);
    assert_eq!(summary.inter_cluster_relations, 1);
    assert_eq!(summary.top_aggregates.len(), 1);
    assert_eq!(summary.top_aggregates[0].id, "invalid-hierarchy-entity");
    assert_eq!(summary.top_aggregates[0].member_count, 42);
}

#[tokio::test]
async fn duplicate_relationships_collapse_by_normalized_governed_key() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let mut source = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    source.id = "entity-dedup-alice".to_string();
    source
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    let mut target = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    target.id = "entity-dedup-zbot".to_string();
    target
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    let source_id = store
        .upsert_entity("agent-a", source)
        .await
        .expect("source");
    let target_id = store
        .upsert_entity("agent-a", target)
        .await
        .expect("target");

    let mut first = Relationship::new(
        "agent-a".into(),
        source_id.0.clone(),
        target_id.0.clone(),
        RelationshipType::Created,
    );
    first.id = "rel-dedup-created-a".to_string();
    first
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    first
        .properties
        .insert("evidence".to_string(), json!(["trace-a"]));

    let mut duplicate = Relationship::new(
        "agent-a".into(),
        source_id.0.clone(),
        target_id.0.clone(),
        RelationshipType::Custom("CREATED".to_string()),
    );
    duplicate.id = "rel-dedup-created-b".to_string();
    duplicate
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    duplicate
        .properties
        .insert("evidence".to_string(), json!(["trace-b"]));

    let first_id = store
        .upsert_relationship("agent-a", first)
        .await
        .expect("first relationship");
    let duplicate_id = store
        .upsert_relationship("agent-a", duplicate)
        .await
        .expect("duplicate relationship");

    assert_eq!(first_id.0, "rel-dedup-created-a");
    assert_eq!(duplicate_id.0, "rel-dedup-created-a");
    let relationships = store
        .list_relationships("agent-a", None, 10, 0)
        .await
        .expect("relationships");
    assert_eq!(relationships.len(), 1);
    assert_eq!(relationships[0].id, "rel-dedup-created-a");
    assert_eq!(relationships[0].mention_count, 2);
    assert_eq!(
        relationships[0].properties.get("evidence"),
        Some(&json!(["trace-a", "trace-b"]))
    );
    assert_eq!(
        store.count_all_relationships().await.expect("count"),
        1,
        "read models should expose one durable connection"
    );
    assert_eq!(
        store
            .graph_stats("agent-a")
            .await
            .expect("graph stats")
            .relationship_count,
        1
    );
}

#[tokio::test]
async fn relationship_dedup_respects_scope_and_visibility_boundaries() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let mut source = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    source.id = "entity-boundary-alice".to_string();
    let mut target = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    target.id = "entity-boundary-zbot".to_string();
    let source_id = store
        .upsert_entity("agent-a", source)
        .await
        .expect("source");
    let target_id = store
        .upsert_entity("agent-a", target)
        .await
        .expect("target");

    for (id, ward_id, visibility) in [
        ("rel-boundary-base", "ward-a", "workspace"),
        ("rel-boundary-other-ward", "ward-b", "workspace"),
        ("rel-boundary-public", "ward-a", "public"),
    ] {
        let mut relationship = Relationship::new(
            "agent-a".into(),
            source_id.0.clone(),
            target_id.0.clone(),
            RelationshipType::Uses,
        );
        relationship.id = id.to_string();
        relationship
            .properties
            .insert("ward_id".to_string(), json!(ward_id));
        relationship
            .properties
            .insert("visibility".to_string(), json!(visibility));
        store
            .upsert_relationship("agent-a", relationship)
            .await
            .expect("relationship");
    }

    let relationships = store
        .list_relationships("agent-a", None, 10, 0)
        .await
        .expect("relationships");
    assert_eq!(relationships.len(), 3);
    assert_eq!(
        store.count_all_relationships().await.expect("count"),
        3,
        "scope and visibility boundaries must not be collapsed"
    );
}

#[tokio::test]
async fn admission_gate_rejects_unclassified_entity_and_predicate_before_persisting() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(governed_engram_config(&root)).expect("store");
    let mut unclassified = Entity::new(
        "agent-a".into(),
        EntityType::Custom("generated_label".into()),
        "Generated Label".into(),
    );
    unclassified.id = "entity-unclassified".to_string();
    assert!(store
        .upsert_entity("agent-a", unclassified)
        .await
        .expect_err("custom entity type must be rejected")
        .to_string()
        .contains("built-in ontology class"));
    assert!(store
        .get_entity(&zbot_stores::types::EntityId("entity-unclassified".into()))
        .await
        .expect("read rejected entity")
        .is_none());

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
    relationship.properties.insert(
        "governance_ontology_ids".to_string(),
        json!(["untrusted.ontology:v1"]),
    );

    let error = store
        .upsert_relationship("agent-a", relationship)
        .await
        .expect_err("custom predicate must be rejected");
    assert!(error.to_string().contains("built-in ontology predicate"));

    let relationships = store
        .list_relationships("agent-a", None, 10, 0)
        .await
        .expect("relationships");
    assert!(relationships.is_empty());
    let findings = store
        .list_governance_findings(Some("agent-a"), 10)
        .expect("findings");
    assert!(findings.is_empty());
}

#[tokio::test]
async fn admission_gate_rejects_cross_agent_entity_id_takeover_direct_and_in_batch() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let mut owned = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    owned.id = "entity-shared-id".to_string();
    store
        .upsert_entity("agent-a", owned)
        .await
        .expect("seed owner entity");

    let mut takeover = Entity::new("agent-b".into(), EntityType::Person, "Mallory".into());
    takeover.id = "entity-shared-id".to_string();
    let direct_error = store
        .upsert_entity("agent-b", takeover.clone())
        .await
        .expect_err("direct cross-agent takeover must fail");
    assert!(direct_error
        .to_string()
        .contains("entity id belongs to another agent"));
    let retained = store
        .get_entity(&zbot_stores::types::EntityId("entity-shared-id".into()))
        .await
        .expect("read owner")
        .expect("owner retained");
    assert_eq!(retained.agent_id, "agent-a");
    assert_eq!(retained.name, "Alice");

    let mut fresh = Entity::new("agent-b".into(), EntityType::Project, "Fresh".into());
    fresh.id = "entity-fresh-before-collision".to_string();
    let batch_error = store
        .store_knowledge(
            "agent-b",
            ExtractedKnowledge {
                entities: vec![fresh, takeover],
                relationships: vec![],
            },
        )
        .await
        .expect_err("batch cross-agent takeover must fail before mutation");
    assert!(batch_error
        .to_string()
        .contains("entity id belongs to another agent"));
    assert!(store
        .get_entity(&zbot_stores::types::EntityId(
            "entity-fresh-before-collision".into()
        ))
        .await
        .expect("read fresh")
        .is_none());
}

#[tokio::test]
async fn concurrent_cross_agent_entity_claim_has_exactly_one_owner() {
    let root = tempfile::tempdir().expect("root");
    let store =
        std::sync::Arc::new(EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store"));
    let mut alice = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    alice.id = "entity-concurrent-claim".to_string();
    let mut mallory = Entity::new("agent-b".into(), EntityType::Person, "Mallory".into());
    mallory.id = "entity-concurrent-claim".to_string();

    let (alice_result, mallory_result) = tokio::join!(
        store.upsert_entity("agent-a", alice),
        store.upsert_entity("agent-b", mallory)
    );

    assert_ne!(
        alice_result.is_ok(),
        mallory_result.is_ok(),
        "exactly one agent must acquire a fresh entity ID"
    );
    let retained = store
        .get_entity(&zbot_stores::types::EntityId(
            "entity-concurrent-claim".into(),
        ))
        .await
        .expect("read winner")
        .expect("winner persisted");
    if alice_result.is_ok() {
        assert_eq!(retained.agent_id, "agent-a");
        assert_eq!(retained.name, "Alice");
        assert!(mallory_result
            .expect_err("Mallory loses")
            .to_string()
            .contains("entity id belongs to another agent"));
    } else {
        assert_eq!(retained.agent_id, "agent-b");
        assert_eq!(retained.name, "Mallory");
        assert!(alice_result
            .expect_err("Alice loses")
            .to_string()
            .contains("entity id belongs to another agent"));
    }
}

#[tokio::test]
async fn admission_gate_rejects_dangling_relationships() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let mut relationship = Relationship::new(
        "agent-a".into(),
        "missing-source".into(),
        "missing-target".into(),
        RelationshipType::RelatedTo,
    );
    relationship.id = "rel-dangling".to_string();

    assert!(store
        .upsert_relationship("agent-a", relationship)
        .await
        .expect_err("relationship endpoints must exist")
        .to_string()
        .contains("source entity does not exist"));
    assert!(store
        .list_relationships("agent-a", None, 10, 0)
        .await
        .expect("relationships")
        .is_empty());
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
async fn duplicate_relationship_replaces_stale_governance_with_the_active_selection() {
    let root = tempfile::tempdir().expect("root");
    let mut source = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    source.id = "entity-stale-governance-source".to_string();
    source
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    let mut target = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    target.id = "entity-stale-governance-target".to_string();
    target
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));

    {
        let store = EngramKnowledgeGraphStore::open(governed_engram_config(&root)).expect("A");
        store
            .upsert_entity("agent-a", source)
            .await
            .expect("source");
        store
            .upsert_entity("agent-a", target)
            .await
            .expect("target");
        let mut relationship = Relationship::new(
            "agent-a".into(),
            "entity-stale-governance-source".into(),
            "entity-stale-governance-target".into(),
            RelationshipType::Created,
        );
        relationship.id = "rel-governance-a".to_string();
        relationship
            .properties
            .insert("ward_id".to_string(), json!("ward-a"));
        store
            .upsert_relationship("agent-a", relationship)
            .await
            .expect("first relationship");
    }

    let store = EngramKnowledgeGraphStore::open(governed_engram_config_with_ids(
        &root,
        "ontology.changed:v1",
        "taxonomy.changed:v1",
    ))
    .expect("B");
    let mut duplicate = Relationship::new(
        "agent-a".into(),
        "entity-stale-governance-source".into(),
        "entity-stale-governance-target".into(),
        RelationshipType::Created,
    );
    duplicate.id = "rel-governance-b".to_string();
    duplicate
        .properties
        .insert("ward_id".to_string(), json!("ward-a"));
    duplicate.properties.insert(
        "governance_ontology_ids".to_string(),
        json!(["untrusted.ontology:v1"]),
    );
    store
        .upsert_relationship("agent-a", duplicate)
        .await
        .expect("duplicate relationship");

    let relationships = store
        .list_relationships("agent-a", None, 10, 0)
        .await
        .expect("relationships");
    assert_eq!(relationships.len(), 1);
    assert_eq!(
        relationships[0].properties.get("governance_ontology_ids"),
        Some(&json!(["ontology.changed:v1"]))
    );
    assert_eq!(
        relationships[0]
            .properties
            .get("governance_taxonomy_scheme_ids"),
        Some(&json!(["taxonomy.changed:v1"]))
    );
}

#[tokio::test]
async fn unconfigured_relationship_governance_removes_reserved_input_properties() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramKnowledgeGraphStore::open(engram_config(&root)).expect("store");
    let mut source = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    source.id = "entity-unconfigured-source".to_string();
    let mut target = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    target.id = "entity-unconfigured-target".to_string();
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
        RelationshipType::Created,
    );
    relationship.id = "rel-unconfigured".to_string();
    relationship.properties.insert(
        "governance_ontology_ids".to_string(),
        json!(["untrusted.ontology:v1"]),
    );
    relationship.properties.insert(
        "governance_taxonomy_scheme_ids".to_string(),
        json!(["untrusted.taxonomy:v1"]),
    );
    relationship
        .properties
        .insert("governance_record_kind".to_string(), json!("relationship"));

    store
        .upsert_relationship("agent-a", relationship)
        .await
        .expect("relationship");
    let relationship = store
        .list_relationships("agent-a", None, 10, 0)
        .await
        .expect("relationships")
        .pop()
        .expect("relationship row");
    for key in [
        "governance_ontology_ids",
        "governance_taxonomy_scheme_ids",
        "governance_record_kind",
    ] {
        assert!(
            !relationship.properties.contains_key(key),
            "{key} must be removed without a configured selection"
        );
    }
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
    assert_eq!(inter[0].relationship_type, "related_to");
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
