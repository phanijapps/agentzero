use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use rusqlite::Connection;
use serde_json::json;
use zbot_engram_adapter::{
    run_migration_dry_run, AdapterConfig, AllowUnclassifiedPolicy, EngramKnowledgeGraphStore,
    EngramProvider, EngramTaxonomyRecallExpander, GovernanceCapabilityHealth, GovernancePolicy,
    GovernanceSelection, MigrationInput, MigrationMode, MigrationSource, SkosExpansionPolicy,
    ValidationMode, ZBOT_BASE_ONTOLOGY_ID, ZBOT_GENERAL_SCHEME_ID,
};
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_traits::{RecallTaxonomyExpander, RecallTaxonomyExpansionRequest};

fn governed_config(root: &tempfile::TempDir, mode: MigrationMode) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.migration_mode = mode;
    config.governance = GovernancePolicy {
        default_selection: GovernanceSelection {
            ontology_ids: vec![ZBOT_BASE_ONTOLOGY_ID.to_string()],
            taxonomy_scheme_ids: vec![ZBOT_GENERAL_SCHEME_ID.to_string()],
        },
        validation_mode: ValidationMode::Advisory,
        allow_unclassified: AllowUnclassifiedPolicy::Warn,
        skos_expansion: SkosExpansionPolicy {
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8,
        },
        ..GovernancePolicy::default()
    };
    config
}

fn create_empty_source(path: &std::path::Path) {
    let connection = Connection::open(path).expect("source db");
    connection
        .execute_batch(
            r#"
            CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);
            INSERT INTO schema_version (version, applied_at) VALUES (42, '2026-07-08T00:00:00Z');
            "#,
        )
        .expect("empty source schema");
}

#[tokio::test]
async fn configured_governance_fixture_covers_cutover_surfaces() {
    let root = tempfile::tempdir().expect("root");
    let config = governed_config(&root, MigrationMode::DryRun);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let bootstrap = provider
        .governance_bootstrap()
        .expect("governance bootstrap");
    assert_eq!(bootstrap.ontology_id, ZBOT_BASE_ONTOLOGY_ID);
    assert_eq!(bootstrap.taxonomy_scheme_id, ZBOT_GENERAL_SCHEME_ID);
    assert!(bootstrap.class_count > 0);
    assert!(bootstrap.concept_count > 0);

    let graph =
        EngramKnowledgeGraphStore::from_provider(config.clone(), &provider).expect("graph store");
    let mut source = Entity::new("agent-a".into(), EntityType::Project, "ZBot".into());
    source.id = "entity-cutover-project".to_string();
    source
        .properties
        .insert("ward_id".to_string(), json!("ward-cutover"));
    let mut target = Entity::new("agent-a".into(), EntityType::Person, "Alice".into());
    target.id = "entity-cutover-person".to_string();
    target
        .properties
        .insert("ward_id".to_string(), json!("ward-cutover"));
    let source_id = graph
        .upsert_entity("agent-a", source)
        .await
        .expect("source");
    let target_id = graph
        .upsert_entity("agent-a", target)
        .await
        .expect("target");

    let mut relationship = Relationship::new(
        "agent-a".into(),
        source_id.0,
        target_id.0,
        RelationshipType::WorksFor,
    );
    relationship.id = "rel-cutover-domain-range".to_string();
    relationship
        .properties
        .insert("ward_id".to_string(), json!("ward-cutover"));
    graph
        .upsert_relationship("agent-a", relationship)
        .await
        .expect("relationship");
    let findings = graph
        .list_governance_findings(Some("agent-a"), 10)
        .expect("findings");
    let codes = findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"domain_mismatch"));
    assert!(codes.contains(&"range_mismatch"));

    let expander =
        EngramTaxonomyRecallExpander::from_provider(config.clone(), &provider).expect("taxonomy");
    let expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "kg recall".to_string(),
            ward_id: None,
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8,
        })
        .await
        .expect("expansion");
    assert!(expansion.expanded_query.contains("Knowledge Graph"));
    assert!(expansion
        .candidates
        .iter()
        .any(|candidate| candidate.scheme_id == ZBOT_GENERAL_SCHEME_ID));

    let health = GovernanceCapabilityHealth::from_config_and_findings(
        &config,
        provider.governance_bootstrap(),
        &findings,
    );
    assert!(health.supported);
    assert_eq!(health.ontology_ids, vec![ZBOT_BASE_ONTOLOGY_ID]);
    assert_eq!(health.taxonomy_scheme_ids, vec![ZBOT_GENERAL_SCHEME_ID]);
    assert_eq!(health.finding_count, findings.len());
    assert!(health
        .finding_codes
        .iter()
        .any(|code| code == "domain_mismatch"));

    let source_db = root.path().join("knowledge.db");
    create_empty_source(&source_db);
    let report = run_migration_dry_run(&MigrationInput::new(
        config,
        vec![MigrationSource::knowledge(&source_db)],
    ))
    .expect("dry run");
    assert_eq!(
        report.manifest.governance_ontology_ids,
        vec![ZBOT_BASE_ONTOLOGY_ID.to_string()]
    );
    assert_eq!(
        report.manifest.governance_taxonomy_scheme_ids,
        vec![ZBOT_GENERAL_SCHEME_ID.to_string()]
    );
    assert!(!report.manifest.governance_config_fingerprint.is_empty());
}
