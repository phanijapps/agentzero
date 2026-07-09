use zbot_engram_adapter::{
    AdapterConfig, EngramTaxonomyRecallExpander, GovernancePolicy, GovernanceSelection,
    ZBOT_GENERAL_SCHEME_ID,
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

#[tokio::test]
async fn skos_expansion_uses_alt_labels_and_bounded_relations() {
    let root = tempfile::tempdir().expect("root");
    let expander =
        EngramTaxonomyRecallExpander::open(governed_config(&root)).expect("taxonomy expander");

    let expansion = expander
        .expand_recall_query(RecallTaxonomyExpansionRequest {
            query: "kg recall".to_string(),
            ward_id: None,
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
