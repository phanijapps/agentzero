use zbot_engram_adapter::{
    AdapterConfig, EngramProvider, EngramTaxonomyRecallExpander, GovernancePolicy,
    GovernanceSelection, ZBOT_GENERAL_SCHEME_ID,
};
use zbot_stores_traits::{RecallTaxonomyExpander, RecallTaxonomyExpansionRequest};

fn governed_config(root: &tempfile::TempDir) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    config.governance = GovernancePolicy {
        default_selection: GovernanceSelection {
            ontology_ids: Vec::new(),
            taxonomy_scheme_ids: vec![ZBOT_GENERAL_SCHEME_ID.to_string()],
        },
        ..GovernancePolicy::default()
    };
    config
}

fn configured_taxonomy_config(root: &tempfile::TempDir) -> AdapterConfig {
    let config_root = root.path().join("config");
    let governance_dir = config_root.join("governance");
    std::fs::create_dir_all(&governance_dir).expect("governance dir");
    std::fs::write(
        governance_dir.join("project-taxonomy.json"),
        r#"{
          "kind": "zbot.skos_taxonomy",
          "schemaVersion": 1,
          "schemeId": "zbot.projects:v1",
          "label": "Project Taxonomy",
          "concepts": [
            { "id": "work", "prefLabel": "Work" },
            {
              "id": "planning",
              "prefLabel": "Planning",
              "altLabels": ["roadmap"],
              "broader": ["work"],
              "related": ["execution"]
            },
            { "id": "execution", "prefLabel": "Execution" }
          ]
        }"#,
    )
    .expect("taxonomy definition");

    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db")
        .with_trusted_config_root(config_root);
    config.governance = GovernancePolicy {
        taxonomy_definition_paths: vec!["governance/project-taxonomy.json".into()],
        default_selection: GovernanceSelection {
            ontology_ids: Vec::new(),
            taxonomy_scheme_ids: vec!["zbot.projects:v1".to_string()],
        },
        ..GovernancePolicy::default()
    };
    config
}

fn session_overlay_config(root: &tempfile::TempDir) -> AdapterConfig {
    let config_root = root.path().join("config");
    let governance_dir = config_root.join("governance");
    std::fs::create_dir_all(&governance_dir).expect("governance dir");
    std::fs::write(
        governance_dir.join("default-taxonomy.json"),
        r#"{
          "kind": "zbot.skos_taxonomy", "schemaVersion": 1,
          "schemeId": "zbot.default:v1", "label": "Default Taxonomy",
          "concepts": [{"id":"default","prefLabel":"Default","altLabels":["shared"]}]
        }"#,
    )
    .expect("default taxonomy");
    std::fs::write(
        governance_dir.join("session-taxonomy.json"),
        r#"{
          "kind": "zbot.skos_taxonomy", "schemaVersion": 1,
          "schemeId": "zbot.session:v1", "label": "Session Taxonomy",
          "concepts": [{"id":"private","prefLabel":"Session Private","altLabels":["session-secret"]}]
        }"#,
    )
    .expect("session taxonomy");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db")
        .with_trusted_config_root(config_root);
    config.governance = GovernancePolicy {
        taxonomy_definition_paths: vec![
            "governance/default-taxonomy.json".into(),
            "governance/session-taxonomy.json".into(),
        ],
        default_selection: GovernanceSelection {
            ontology_ids: Vec::new(),
            taxonomy_scheme_ids: vec!["zbot.default:v1".to_string()],
        },
        overlays: vec![zbot_engram_adapter::GovernanceOverlay {
            session_id: Some("sess-private".to_string()),
            selection: GovernanceSelection {
                ontology_ids: Vec::new(),
                taxonomy_scheme_ids: vec!["zbot.session:v1".to_string()],
            },
            ..zbot_engram_adapter::GovernanceOverlay::default()
        }],
        ..GovernancePolicy::default()
    };
    config
}

#[tokio::test]
async fn skos_expansion_uses_alt_labels_and_bounded_relations() {
    let root = tempfile::tempdir().expect("root");
    let expander =
        EngramTaxonomyRecallExpander::open(governed_config(&root)).expect("taxonomy expander");

    let expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "kg recall".to_string(),
            ward_id: None,
            session_id: None,
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 4,
        })
        .await
        .expect("expand query");

    assert_eq!(expansion.candidates.len(), 4);
    assert_eq!(expansion.candidates[0].matched_label, "kg");
    assert_eq!(expansion.candidates[0].label, "Knowledge Graph");
    assert_eq!(expansion.candidates[0].relation, None);
    assert!(expansion
        .candidates
        .iter()
        .any(|candidate| candidate.relation.as_deref() == Some("broader") && candidate.depth == 1));
    assert!(expansion
        .candidates
        .iter()
        .any(|candidate| candidate.relation.as_deref() == Some("related") && candidate.depth == 1));
    assert!(expansion.expanded_query.contains("Knowledge Graph"));
}

#[tokio::test]
async fn skos_expansion_ignores_deeper_cycles_when_depth_is_zero() {
    let root = tempfile::tempdir().expect("root");
    let expander =
        EngramTaxonomyRecallExpander::open(governed_config(&root)).expect("taxonomy expander");

    let expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "agent engine".to_string(),
            ward_id: None,
            session_id: None,
            max_depth: 0,
            max_fan_out: 8,
            max_candidates: 16,
        })
        .await
        .expect("expand query");

    assert_eq!(expansion.candidates.len(), 1);
    assert_eq!(expansion.candidates[0].label, "Agent Runtime");
    assert_eq!(expansion.candidates[0].depth, 0);
}

#[tokio::test]
async fn configured_taxonomy_is_bootstrapped_and_expands_its_own_relations() {
    let root = tempfile::tempdir().expect("root");
    let config = configured_taxonomy_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let bootstrap = provider.governance_bootstrap().expect("bootstrap");
    assert_eq!(bootstrap.taxonomy_scheme_ids, vec!["zbot.projects:v1"]);
    assert_eq!(bootstrap.concept_count, 3);
    assert_eq!(bootstrap.relation_count, 2);

    let expander =
        EngramTaxonomyRecallExpander::from_provider(config, &provider).expect("taxonomy expander");
    let expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "review the roadmap".to_string(),
            ward_id: None,
            session_id: None,
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8,
        })
        .await
        .expect("expand query");

    assert!(expansion.expanded_query.contains("Planning"));
    assert!(expansion.expanded_query.contains("Work"));
    assert!(expansion.expanded_query.contains("Execution"));
    assert!(expansion.candidates.iter().any(|candidate| {
        candidate.scheme_id == "zbot.projects:v1"
            && candidate.label == "Work"
            && candidate.relation.as_deref() == Some("broader")
    }));
    assert!(expansion.candidates.iter().any(|candidate| {
        candidate.scheme_id == "zbot.projects:v1"
            && candidate.label == "Execution"
            && candidate.relation.as_deref() == Some("related")
    }));
}

#[test]
fn malformed_configured_taxonomy_fails_without_leaking_the_definition_path() {
    let root = tempfile::tempdir().expect("root");
    let config_root = root.path().join("config");
    let governance_dir = config_root.join("governance");
    std::fs::create_dir_all(&governance_dir).expect("governance dir");
    let definition_path = governance_dir.join("private-taxonomy.json");
    std::fs::write(
        &definition_path,
        r#"{
          "kind": "zbot.ontology",
          "schemaVersion": 1,
          "schemeId": "zbot.projects:v1",
          "label": "Wrong Kind"
        }"#,
    )
    .expect("taxonomy definition");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db")
        .with_trusted_config_root(config_root);
    config.governance = GovernancePolicy {
        taxonomy_definition_paths: vec!["governance/private-taxonomy.json".into()],
        default_selection: GovernanceSelection {
            ontology_ids: Vec::new(),
            taxonomy_scheme_ids: vec!["zbot.projects:v1".to_string()],
        },
        ..GovernancePolicy::default()
    };

    let error = EngramProvider::open(config)
        .err()
        .expect("wrong definition type must fail");
    let message = error.to_string();
    assert!(message.contains("governance_taxonomy_definition"));
    assert!(!message.contains("private-taxonomy.json"));
    assert!(!message.contains(&root.path().display().to_string()));
}

#[tokio::test]
async fn session_governance_overlay_selects_only_the_trusted_session_taxonomy() {
    let root = tempfile::tempdir().expect("root");
    let config = session_overlay_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let expander =
        EngramTaxonomyRecallExpander::from_provider(config, &provider).expect("taxonomy expander");

    assert!(expander.is_configured_for(None, Some("sess-private")));
    let expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "session-secret".to_string(),
            ward_id: None,
            session_id: Some("sess-private".to_string()),
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8,
        })
        .await
        .expect("session expansion");
    assert!(expansion
        .candidates
        .iter()
        .all(|candidate| candidate.scheme_id == "zbot.session:v1"));
    assert!(expansion.expanded_query.contains("Session Private"));

    let default_expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "session-secret shared".to_string(),
            ward_id: None,
            session_id: Some("sess-other".to_string()),
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8,
        })
        .await
        .expect("default expansion");
    assert!(default_expansion
        .candidates
        .iter()
        .all(|candidate| candidate.scheme_id == "zbot.default:v1"));
}

#[tokio::test]
async fn restart_filters_removed_concepts_to_the_current_definition_snapshot() {
    let root = tempfile::tempdir().expect("root");
    let config = configured_taxonomy_config(&root);
    let first_provider = EngramProvider::open(config.clone()).expect("first provider");
    let first_expander =
        EngramTaxonomyRecallExpander::from_provider(config.clone(), &first_provider)
            .expect("first expander");
    assert!(first_expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "roadmap".to_string(),
            ward_id: None,
            session_id: None,
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8
        })
        .await
        .expect("first expansion")
        .candidates
        .iter()
        .any(|candidate| candidate.label == "Planning"));

    std::fs::write(
        root.path().join("config/governance/project-taxonomy.json"),
        r#"{
          "kind": "zbot.skos_taxonomy", "schemaVersion": 1,
          "schemeId": "zbot.projects:v1", "label": "Project Taxonomy",
          "concepts": [{"id":"current","prefLabel":"Current","altLabels":["fresh"]}]
        }"#,
    )
    .expect("updated taxonomy definition");
    let second_provider = EngramProvider::open(config.clone()).expect("second provider");
    let second_expander = EngramTaxonomyRecallExpander::from_provider(config, &second_provider)
        .expect("second expander");
    let stale = second_expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "roadmap".to_string(),
            ward_id: None,
            session_id: None,
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 8,
        })
        .await
        .expect("stale expansion");
    assert!(stale.candidates.is_empty());
    assert_eq!(stale.expanded_query, "roadmap");
}

#[test]
fn provider_rejects_an_expander_config_from_a_different_governance_snapshot() {
    let root = tempfile::tempdir().expect("root");
    let config = configured_taxonomy_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let mut mismatched = config;
    mismatched.governance.default_selection.taxonomy_scheme_ids = vec!["zbot.other:v1".to_string()];

    let error = EngramTaxonomyRecallExpander::from_provider(mismatched, &provider)
        .err()
        .expect("mismatched provider config must fail");
    assert!(error.to_string().contains("does not match"));
}
