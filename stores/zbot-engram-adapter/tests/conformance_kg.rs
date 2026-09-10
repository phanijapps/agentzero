//! KnowledgeGraphStore conformance for the engram adapter.
//!
//! Tier E1-a parity proof: every KG scenario the sqlite KG suite exercised
//! must hold on the production (engram) KG store before the sqlite crate
//! retires. Governance-sensitive admission (ungoverned entity types /
//! predicates are rejected) is the adapter's documented behavior; the
//! scenarios below use governed types/person names, which the default
//! governance selection classifies.

use zbot_engram_adapter::{AdapterConfig, EngramKnowledgeGraphStore, EngramProvider};
use zbot_stores_conformance as conf;

fn kg_store(root: &tempfile::TempDir) -> EngramKnowledgeGraphStore {
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    EngramKnowledgeGraphStore::from_provider(config, &provider).expect("kg store opens")
}

macro_rules! kg_conformance {
    ($(#[$meta:meta])* $name:ident, $scenario:path) => {
        $(#[$meta])*
        #[tokio::test]
        async fn $name() {
            let root = tempfile::tempdir().expect("root");
            $scenario(&kg_store(&root)).await;
        }
    };
}

kg_conformance!(entity_round_trip, conf::entity_round_trip);
kg_conformance!(
    upsert_increments_mention_count,
    conf::upsert_increments_mention_count
);
kg_conformance!(
    bump_mention_increases_count,
    conf::bump_mention_increases_count
);
kg_conformance!(resolve_exact_match, conf::resolve_exact_match);
kg_conformance!(resolve_via_alias, conf::resolve_via_alias);
kg_conformance!(resolve_no_match, conf::resolve_no_match);
kg_conformance!(relationship_round_trip, conf::relationship_round_trip);
kg_conformance!(
    store_knowledge_writes_both,
    conf::store_knowledge_writes_both
);
kg_conformance!(neighbors_outgoing, conf::neighbors_outgoing);
kg_conformance!(neighbors_incoming, conf::neighbors_incoming);
kg_conformance!(traverse_respects_max_hops, conf::traverse_respects_max_hops);
kg_conformance!(fts_finds_match, conf::fts_finds_match);
kg_conformance!(
    reindex_idempotent_when_dim_matches,
    conf::reindex_idempotent_when_dim_matches
);
kg_conformance!(stats_reflects_writes, conf::stats_reflects_writes);
kg_conformance!(graph_stats_per_agent, conf::graph_stats_per_agent);
kg_conformance!(mark_archival_sets_class, conf::mark_archival_sets_class);
kg_conformance!(
    list_entities_respects_agent,
    conf::list_entities_respects_agent
);

// ---- KG-maintenance (KG-lane port) ----------------------------------------

#[tokio::test]
async fn kg_decay_entity_confidence() {
    let root = tempfile::tempdir().expect("root");
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    let identity = zbot_stores_traits::EmbeddingQueryIdentity {
        provider_type: config.embedding_provider.provider_type.clone(),
        model: config.embedding_provider.model.clone(),
        dimensions: config.embedding_provider.dimensions,
        prompt_profile: config.embedding_provider.prompt_profile.clone(),
        normalization: config.embedding_provider.normalization.clone(),
    };
    conf::kg_decay_entity_confidence(&kg_store(&root), &identity).await;
}
kg_conformance!(
    kg_find_duplicate_candidates,
    conf::kg_find_duplicate_candidates
);
kg_conformance!(kg_orphan_candidates, conf::kg_orphan_candidates);
kg_conformance!(kg_merge_entity_into, conf::kg_merge_entity_into);
kg_conformance!(kg_mark_entity_pruned, conf::kg_mark_entity_pruned);
#[tokio::test]
async fn kg_confidence_multiplier() {
    let root = tempfile::tempdir().expect("root");
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    let identity = zbot_stores_traits::EmbeddingQueryIdentity {
        provider_type: config.embedding_provider.provider_type.clone(),
        model: config.embedding_provider.model.clone(),
        dimensions: config.embedding_provider.dimensions,
        prompt_profile: config.embedding_provider.prompt_profile.clone(),
        normalization: config.embedding_provider.normalization.clone(),
    };
    conf::kg_confidence_multiplier(&kg_store(&root), &identity).await;
}
