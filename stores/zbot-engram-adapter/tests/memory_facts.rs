use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rusqlite::params;
use serde_json::json;
use zbot_engram_adapter::{
    mapping::memory::{
        memory_fact_to_record, memory_fact_to_record_with_governance, memory_record_to_fact,
    },
    AdapterConfig, AdapterErrorKind, AdapterFeature, CapabilityReport, EmbeddingMode,
    EngramMemoryFactStore, EngramProvider, GovernanceOverlay, GovernancePolicy,
    GovernanceSelection, ZBOT_BASE_ONTOLOGY_ID, ZBOT_GENERAL_SCHEME_ID,
};
use zbot_stores_traits::{
    EmbeddingQueryIdentity, MemoryFact, MemoryFactStore, MemoryFactWriteRequest,
};

fn engram_config(root: &tempfile::TempDir) -> AdapterConfig {
    AdapterConfig::engram_for_data_root(root.path(), "engram.db")
}

fn engram_config_with_embedding_identity(
    root: &tempfile::TempDir,
    model: &str,
    dimensions: u32,
) -> AdapterConfig {
    let mut config = engram_config(root);
    config.embedding_provider.model = model.to_string();
    config.embedding_provider.dimensions = dimensions;
    config
}

fn query_identity(model: &str, dimensions: u32) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: "fastembed".to_string(),
        model: model.to_string(),
        dimensions,
        prompt_profile: "query".to_string(),
        normalization: None,
    }
}

fn sample_fact() -> MemoryFact {
    MemoryFact {
        id: "fact-lossless-1".to_string(),
        session_id: Some("sess-1".to_string()),
        agent_id: "agent-a".to_string(),
        scope: "session".to_string(),
        category: "preference".to_string(),
        key: "user.favorite_language".to_string(),
        content: "User prefers Rust".to_string(),
        confidence: 0.92,
        mention_count: 3,
        source_summary: Some("manual note".to_string()),
        embedding: Some(vec![0.25, 0.5, 0.75]),
        ward_id: "ward-alpha".to_string(),
        contradicted_by: None,
        created_at: "2026-07-06T10:00:00Z".to_string(),
        updated_at: "2026-07-06T10:05:00Z".to_string(),
        expires_at: Some("2027-01-01T00:00:00Z".to_string()),
        valid_from: Some("2026-07-01T00:00:00Z".to_string()),
        valid_until: None,
        superseded_by: None,
        pinned: true,
        epistemic_class: Some("current".to_string()),
        source_episode_id: Some("episode-1".to_string()),
        source_ref: Some("manual://memory/fact-lossless-1".to_string()),
    }
}

struct MutableIdentityEmbedder {
    model: Mutex<String>,
    calls: AtomicUsize,
}

#[async_trait]
impl EmbeddingClient for MutableIdentityEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.calls.fetch_add(texts.len(), Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![1.0_f32, 0.0]).collect())
    }

    fn dimensions(&self) -> usize {
        2
    }

    fn model_name(&self) -> String {
        self.model.lock().expect("model lock").clone()
    }

    fn provider_type(&self) -> String {
        "fastembed".to_string()
    }
}

struct FailingEmbedder;

#[async_trait]
impl EmbeddingClient for FailingEmbedder {
    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Err(EmbeddingError::ModelError("forced failure".to_string()))
    }

    fn dimensions(&self) -> usize {
        384
    }

    fn model_name(&self) -> String {
        "BAAI/bge-small-en-v1.5".to_string()
    }

    fn provider_type(&self) -> String {
        "fastembed".to_string()
    }
}

#[test]
fn memory_fact_maps_to_engram_record_losslessly() {
    let config = AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db");
    let mapper = config.scope_mapper().expect("scope mapper");
    let fact = sample_fact();

    let record = memory_fact_to_record(&fact, &mapper, EmbeddingMode::PreserveBytes)
        .expect("record mapping");
    let round_trip = memory_record_to_fact(&record).expect("fact mapping");

    assert_eq!(record.id.as_str(), fact.id);
    assert_eq!(record.scope.tenant, "agentzero");
    assert_eq!(record.scope.workspace.as_deref(), Some("ward-alpha"));
    assert_eq!(record.scope.session.as_deref(), Some("sess-1"));
    assert_eq!(record.provenance.confidence, Some(0.92));
    assert_eq!(record.provenance.source, "manual://memory/fact-lossless-1");
    assert_eq!(
        record.created_at,
        Utc.with_ymd_and_hms(2026, 7, 6, 10, 0, 0).unwrap()
    );
    assert_eq!(
        record.updated_at,
        Some(Utc.with_ymd_and_hms(2026, 7, 6, 10, 5, 0).unwrap())
    );
    assert_eq!(
        record
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("embeddingMode")),
        Some(&json!("preserve_bytes"))
    );
    assert_eq!(
        record
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("importance")),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(round_trip.id, fact.id);
    assert_eq!(round_trip.agent_id, fact.agent_id);
    assert_eq!(round_trip.scope, fact.scope);
    assert_eq!(round_trip.category, fact.category);
    assert_eq!(round_trip.key, fact.key);
    assert_eq!(round_trip.content, fact.content);
    assert_eq!(round_trip.confidence, fact.confidence);
    assert_eq!(round_trip.mention_count, fact.mention_count);
    assert_eq!(round_trip.source_summary, fact.source_summary);
    assert_eq!(round_trip.ward_id, fact.ward_id);
    assert_eq!(round_trip.created_at, fact.created_at);
    assert_eq!(round_trip.updated_at, fact.updated_at);
    assert_eq!(round_trip.valid_from, fact.valid_from);
    assert_eq!(round_trip.source_ref, fact.source_ref);
    assert_eq!(round_trip.embedding, None);
}

#[test]
fn memory_fact_mapping_persists_the_scoped_governance_classification() {
    let config = AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db");
    let mapper = config.scope_mapper().expect("scope mapper");
    let fact = sample_fact();
    let governance = GovernancePolicy {
        default_selection: GovernanceSelection {
            ontology_ids: vec!["ontology.default:v1".to_string()],
            taxonomy_scheme_ids: vec!["taxonomy.default:v1".to_string()],
        },
        overlays: vec![GovernanceOverlay {
            session_id: Some("sess-1".to_string()),
            selection: GovernanceSelection {
                ontology_ids: vec![ZBOT_BASE_ONTOLOGY_ID.to_string()],
                taxonomy_scheme_ids: vec![ZBOT_GENERAL_SCHEME_ID.to_string()],
            },
            ..GovernanceOverlay::default()
        }],
        ..GovernancePolicy::default()
    };

    let record = memory_fact_to_record_with_governance(
        &fact,
        &mapper,
        EmbeddingMode::PreserveBytes,
        Some(&governance),
    )
    .expect("record mapping");
    let metadata = record.metadata.expect("governance metadata");

    assert_eq!(
        metadata.get("governanceOntologyIds"),
        Some(&json!([ZBOT_BASE_ONTOLOGY_ID]))
    );
    assert_eq!(
        metadata.get("governanceTaxonomySchemeIds"),
        Some(&json!([ZBOT_GENERAL_SCHEME_ID]))
    );
    assert_eq!(
        metadata.get("governanceTaxonomyConceptIds"),
        Some(&json!([format!("{ZBOT_GENERAL_SCHEME_ID}:concept:memory")]))
    );
}

#[tokio::test]
async fn save_count_list_get_delete_and_archive_round_trip() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .save_fact(
            "agent-a",
            "correction",
            "tone",
            "Use concise responses",
            0.9,
            Some("sess-a"),
            None,
        )
        .await
        .expect("save");

    assert_eq!(
        store.count_all_facts(Some("agent-a")).await.expect("count"),
        1
    );

    let listed = store
        .list_memory_facts(Some("agent-a"), Some("correction"), Some("agent"), 10, 0)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["content"], "Use concise responses");

    let id = listed[0]["id"].as_str().expect("id").to_string();
    let fetched = store
        .get_memory_fact_by_id(&id)
        .await
        .expect("get")
        .expect("fact");
    assert_eq!(fetched["session_id"], "sess-a");

    assert!(store.archive_fact(&id).await.expect("archive"));
    assert!(store
        .list_memory_facts(Some("agent-a"), None, None, 10, 0)
        .await
        .expect("list after archive")
        .is_empty());
    assert!(store
        .get_memory_fact_by_id(&id)
        .await
        .expect("get archived")
        .is_some());

    assert!(store.delete_memory_fact(&id).await.expect("delete"));
    assert!(store
        .get_memory_fact_by_id(&id)
        .await
        .expect("get deleted")
        .is_none());
}

#[tokio::test]
async fn context_aware_save_keeps_ward_session_and_writer_provenance() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .save_fact_with_context(MemoryFactWriteRequest {
            agent_id: "agent-a".to_string(),
            category: "domain".to_string(),
            key: "architecture.memory".to_string(),
            content: "Memory writes retain the active execution scope.".to_string(),
            confidence: 0.9,
            session_id: Some("sess-scoped".to_string()),
            ward_id: Some("ward-scoped".to_string()),
            source_ref: Some("agentzero.memory_tool".to_string()),
            valid_from: None,
        })
        .await
        .expect("scoped save");

    let facts = store
        .get_memory_facts("agent-a", Some("global"), 10)
        .await
        .expect("facts");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].session_id.as_deref(), Some("sess-scoped"));
    assert_eq!(facts[0].ward_id, "ward-scoped");
    assert_eq!(
        facts[0].source_ref.as_deref(),
        Some("agentzero.memory_tool")
    );
}

#[tokio::test]
async fn context_aware_save_does_not_overwrite_a_matching_key_in_another_session() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    for session_id in ["sess-one", "sess-two"] {
        store
            .save_fact_with_context(MemoryFactWriteRequest {
                agent_id: "agent-a".to_string(),
                category: "domain".to_string(),
                key: "architecture.memory".to_string(),
                content: format!("Scoped write from {session_id}."),
                confidence: 0.9,
                session_id: Some(session_id.to_string()),
                ward_id: Some("ward-scoped".to_string()),
                source_ref: Some("agentzero.memory_tool".to_string()),
                valid_from: None,
            })
            .await
            .expect("scoped save");
    }

    let facts = store
        .get_memory_facts("agent-a", Some("global"), 10)
        .await
        .expect("facts");
    assert_eq!(facts.len(), 2);
    let mut session_ids = facts
        .iter()
        .filter_map(|fact| fact.session_id.clone())
        .collect::<Vec<_>>();
    session_ids.sort();
    assert_eq!(session_ids, vec!["sess-one", "sess-two"]);
}

#[tokio::test]
async fn typed_upsert_preserves_embedding_sidecar() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-typed-1".to_string();
    fact.session_id = None;
    fact.scope = "global".to_string();

    store
        .upsert_typed_fact(fact.clone(), Some(vec![0.1, 0.2, 0.3]))
        .await
        .expect("upsert");

    let fetched = store
        .get_memory_fact_by_id("fact-typed-1")
        .await
        .expect("get")
        .expect("fact");
    assert_eq!(fetched["agent_id"], "agent-a");
    assert_eq!(
        store
            .get_fact_embedding("fact-typed-1")
            .await
            .expect("embedding"),
        Some(vec![0.1, 0.2, 0.3])
    );
}

#[tokio::test]
async fn session_ctx_facts_are_excluded_from_public_search_candidates() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-ctx-secret".to_string();
    fact.agent_id = "__ctx__".to_string();
    fact.scope = "session".to_string();
    fact.category = "ctx".to_string();
    fact.key = "ctx.sess-secret.intent".to_string();
    fact.content = "secret session intent should not appear in public recall".to_string();
    fact.ward_id = "ward-alpha".to_string();
    fact.embedding = None;

    store
        .upsert_typed_fact(fact.clone(), None)
        .await
        .expect("upsert ctx");

    let hits = store
        .search_memory_facts_hybrid_typed(
            None,
            "ctx.sess-secret.intent",
            "fts",
            10,
            None,
            None,
            None,
        )
        .await
        .expect("search");

    assert!(
        hits.is_empty(),
        "session ctx facts must not be public search candidates"
    );
}

#[tokio::test]
async fn supported_search_respects_agent_ward_and_as_of_filters() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut agent_a = sample_fact();
    agent_a.id = "fact-agent-a".to_string();
    agent_a.agent_id = "agent-a".to_string();
    agent_a.ward_id = "ward-a".to_string();
    agent_a.scope = "agent".to_string();
    agent_a.session_id = None;
    agent_a.content = "Alpha project uses Rust".to_string();
    let mut agent_b = agent_a.clone();
    agent_b.id = "fact-agent-b".to_string();
    agent_b.agent_id = "agent-b".to_string();
    agent_b.ward_id = "ward-b".to_string();
    agent_b.content = "Alpha project uses TypeScript".to_string();
    let mut expired = agent_a.clone();
    expired.id = "fact-expired".to_string();
    expired.content = "Alpha project used Python".to_string();
    expired.valid_until = Some("2026-01-01T00:00:00Z".to_string());

    for fact in [&agent_a, &agent_b, &expired] {
        store
            .upsert_typed_fact(fact.clone(), None)
            .await
            .expect("upsert");
    }

    let hits = store
        .search_memory_facts_hybrid(
            Some("agent-a"),
            "Alpha Rust",
            "fts",
            10,
            Some("ward-a"),
            None,
            Some(Utc.with_ymd_and_hms(2026, 7, 6, 0, 0, 0).unwrap()),
        )
        .await
        .expect("search");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], "fact-agent-a");
    assert_eq!(hits[0]["match_source"], "fts");
}

#[test]
fn memory_fact_support_can_be_enabled_without_enabling_other_features() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config(&root);

    let default_report = CapabilityReport::from_config(&config);
    assert!(!default_report.supports(AdapterFeature::MemoryFacts));

    let report = CapabilityReport::from_verified_features(&config, [AdapterFeature::MemoryFacts]);

    assert!(report.supports(AdapterFeature::MemoryFacts));
    assert!(!report.supports(AdapterFeature::Beliefs));
    assert!(!report.supports(AdapterFeature::KnowledgeGraph));
}

#[tokio::test]
async fn current_sqlite_mode_does_not_open_memory_fact_store() {
    let err = match EngramMemoryFactStore::open(AdapterConfig::default()) {
        Ok(_) => panic!("current sqlite mode should not open an Engram memory fact store"),
        Err(error) => error,
    };

    assert_eq!(err.kind(), AdapterErrorKind::UnsupportedFeature);
}

#[tokio::test]
async fn recall_prioritized_without_embedder_returns_structured_degraded_reason() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-amd-generic".to_string();
    fact.key = "finance.amd.valuation_methodology".to_string();
    fact.category = "domain".to_string();
    fact.content = "AMD valuation analysis uses relative valuation methodology".to_string();
    fact.embedding = Some(vec![0.0, 1.0]);
    store
        .upsert_typed_fact(fact.clone(), fact.embedding.clone())
        .await
        .expect("upsert");

    let recalled = store
        .recall_facts_prioritized(
            "agent-a",
            "academic paper review methodology research analysis critical evaluation",
            5,
            None,
        )
        .await
        .expect("recall");
    assert_eq!(recalled["degraded"], json!(true));
    assert_eq!(recalled["reason"], "embedding_client_unavailable");
    assert_eq!(recalled["count"], json!(0));
    assert!(
        recalled["results"].as_array().is_some_and(Vec::is_empty),
        "hybrid recall must not return generic lexical hits when the query was not embedded: {recalled:?}"
    );
}

#[tokio::test]
async fn recall_embedding_error_degrades_without_fuzzy_lexical_results() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config(&root);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let store = EngramMemoryFactStore::from_provider_with_embedding_client(
        config,
        &provider,
        Some(Arc::new(FailingEmbedder)),
    )
    .expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-domain-category".to_string();
    fact.key = "finance.amd.valuation_methodology".to_string();
    fact.category = "domain".to_string();
    fact.content = "AMD valuation analysis uses relative valuation methodology".to_string();
    store
        .upsert_typed_fact(fact.clone(), None)
        .await
        .expect("upsert");

    let recalled = store
        .recall_facts_prioritized("agent-a", "domain methodology analysis", 5, None)
        .await
        .expect("recall");
    assert_eq!(recalled["degraded"], json!(true));
    assert_eq!(recalled["reason"], "embedding_error");
    assert!(
        recalled["results"].as_array().is_some_and(Vec::is_empty),
        "degraded recall must not infer broad category/generic lexical matches: {recalled:?}"
    );
}

#[tokio::test]
async fn generic_academic_tokens_do_not_return_unrelated_domain_fact() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut amd = sample_fact();
    amd.id = "fact-amd-methodology".to_string();
    amd.key = "finance.amd.valuation_methodology".to_string();
    amd.category = "domain".to_string();
    amd.content = "AMD valuation analysis uses relative valuation methodology".to_string();
    store
        .upsert_typed_fact(amd.clone(), None)
        .await
        .expect("upsert");

    let recalled = store
        .recall_facts_prioritized(
            "agent-a",
            "academic paper review methodology research analysis critical evaluation",
            5,
            None,
        )
        .await
        .expect("recall");
    let rows = recalled["results"].as_array().expect("results array");
    assert!(
        rows.iter().all(|row| row["id"] != "fact-amd-methodology"),
        "generic academic/task tokens must not recall unrelated AMD facts: {rows:?}"
    );
}

#[tokio::test]
async fn scope_and_category_do_not_bypass_content_relevance_gate() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-same-scope-category".to_string();
    fact.ward_id = "academic-research".to_string();
    fact.category = "domain".to_string();
    fact.content = "AMD valuation analysis uses relative valuation methodology".to_string();
    store
        .upsert_typed_fact(fact.clone(), None)
        .await
        .expect("upsert");

    let hits = store
        .search_memory_facts_hybrid(
            Some("agent-a"),
            "academic paper review methodology research analysis critical evaluation",
            "hybrid",
            5,
            Some("academic-research"),
            None,
            None,
        )
        .await
        .expect("search");
    assert!(
        hits.is_empty(),
        "same scope/category cannot bypass the content relevance gate: {hits:?}"
    );
}

#[tokio::test]
async fn rrf_fuses_dense_and_specific_sparse_ahead_of_newer_generic_hit() {
    let root = tempfile::tempdir().expect("root");
    let store =
        EngramMemoryFactStore::open(engram_config_with_embedding_identity(&root, "test-2d", 2))
            .expect("store");

    let mut semantic_specific = sample_fact();
    semantic_specific.id = "fact-arxiv-specific".to_string();
    semantic_specific.key = "arxiv.2602.03315.paper_under_review".to_string();
    semantic_specific.content = "arxiv 2602.03315".to_string();
    semantic_specific.updated_at = "2026-07-07T04:00:00Z".to_string();
    semantic_specific.embedding = Some(vec![1.0, 0.0]);

    let mut newer_generic = sample_fact();
    newer_generic.id = "fact-newer-generic".to_string();
    newer_generic.key = "generic.research_review".to_string();
    newer_generic.content =
        "arxiv 2602.03315 critical paper review methodology analysis research".to_string();
    newer_generic.updated_at = "2026-07-07T05:00:00Z".to_string();
    newer_generic.embedding = Some(vec![0.0, 1.0]);

    for fact in [&semantic_specific, &newer_generic] {
        store
            .upsert_typed_fact(fact.clone(), fact.embedding.clone())
            .await
            .expect("upsert");
    }

    let identity = query_identity("test-2d", 2);
    let hits = store
        .search_memory_facts_hybrid_with_identity(
            Some("agent-a"),
            "arxiv 2602.03315 critical paper review",
            "hybrid",
            2,
            None,
            Some(&[1.0, 0.0]),
            Some(&identity),
            None,
        )
        .await
        .expect("search");
    assert_eq!(
        hits.first().and_then(|row| row["id"].as_str()),
        Some("fact-arxiv-specific"),
        "RRF should rank semantic+specific evidence above a newer generic lexical hit: {hits:?}"
    );
    assert!(
        hits.first()
            .and_then(|row| row["score"].as_f64())
            .is_some_and(|score| score >= 0.3),
        "adapter must normalize RRF-scale scores before returning them across the trait boundary: {hits:?}"
    );
}

#[tokio::test]
async fn recency_noise_does_not_hide_older_exact_match_before_ranking() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    let mut exact = sample_fact();
    exact.id = "fact-older-exact".to_string();
    exact.key = "arxiv.2602.03315.paper_under_review".to_string();
    exact.content = "Paper under review: arxiv 2602.03315".to_string();
    exact.updated_at = "2026-07-06T01:00:00Z".to_string();
    store
        .upsert_typed_fact(exact.clone(), None)
        .await
        .expect("upsert exact");

    for idx in 0..25 {
        let mut noise = sample_fact();
        noise.id = format!("fact-recent-noise-{idx}");
        noise.key = format!("noise.recent.{idx}");
        noise.content = format!("Unrelated recent memory row {idx}");
        noise.updated_at = format!("2026-07-07T05:{idx:02}:00Z");
        store
            .upsert_typed_fact(noise.clone(), None)
            .await
            .expect("upsert noise");
    }

    let hits = store
        .search_memory_facts_hybrid(Some("agent-a"), "2602.03315", "fts", 2, None, None, None)
        .await
        .expect("search");

    assert!(
        hits.iter().any(|row| row["id"] == "fact-older-exact"),
        "candidate generation must not let recent noise hide older exact matches: {hits:?}"
    );
}

#[tokio::test]
async fn scoped_fact_tie_breaks_before_newer_global_fact() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    let mut scoped = sample_fact();
    scoped.id = "fact-scoped-old".to_string();
    scoped.agent_id = "agent-a".to_string();
    scoped.scope = "agent".to_string();
    scoped.key = "project.alpha.pin".to_string();
    scoped.content = "Project alpha pin".to_string();
    scoped.updated_at = "2026-07-06T00:00:00Z".to_string();
    store
        .upsert_typed_fact(scoped.clone(), None)
        .await
        .expect("upsert scoped");

    let mut global = scoped.clone();
    global.id = "fact-global-new".to_string();
    global.agent_id = "other-agent".to_string();
    global.scope = "global".to_string();
    global.updated_at = "2026-07-07T00:00:00Z".to_string();
    store
        .upsert_typed_fact(global.clone(), None)
        .await
        .expect("upsert global");

    let hits = store
        .search_memory_facts_hybrid(
            Some("agent-a"),
            "project.alpha.pin",
            "hybrid",
            2,
            None,
            None,
            None,
        )
        .await
        .expect("search");

    assert_eq!(
        hits.first().and_then(|row| row["id"].as_str()),
        Some("fact-scoped-old"),
        "scope should tie-break before recency for equally relevant facts: {hits:?}"
    );
}

#[tokio::test]
async fn degraded_recall_allows_exact_identifier_not_generic_category_inference() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");
    let mut arxiv = sample_fact();
    arxiv.id = "fact-arxiv-id".to_string();
    arxiv.key = "arxiv.2602.03315.paper_under_review".to_string();
    arxiv.category = "domain".to_string();
    arxiv.content = "Paper under review: arxiv 2602.03315".to_string();
    store
        .upsert_typed_fact(arxiv.clone(), None)
        .await
        .expect("upsert");

    let exact = store
        .recall_facts_prioritized("agent-a", "2602.03315", 5, None)
        .await
        .expect("recall exact");
    let exact_rows = exact["results"].as_array().expect("results array");
    assert_eq!(exact_rows.len(), 1);
    assert_eq!(exact_rows[0]["match_source"], "exact_degraded");
    assert_eq!(exact_rows[0]["degraded"], json!(true));
    assert_eq!(
        exact_rows[0]["degraded_reason"],
        "embedding_client_unavailable"
    );

    let category_inferred = store
        .recall_facts_prioritized("agent-a", "domain", 5, None)
        .await
        .expect("recall category");
    assert_eq!(category_inferred["degraded"], json!(true));
    assert_eq!(category_inferred["reason"], "embedding_client_unavailable");
    assert!(
        category_inferred["results"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "degraded recall must not infer category filters from generic query tokens: {category_inferred:?}"
    );
}

#[tokio::test]
async fn embedding_identity_mismatch_skips_vectors_with_reindex_blocker() {
    let root = tempfile::tempdir().expect("root");
    let store =
        EngramMemoryFactStore::open(engram_config_with_embedding_identity(&root, "test-2d-a", 2))
            .expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-model-mismatch".to_string();
    fact.embedding = Some(vec![1.0, 0.0]);
    store
        .upsert_typed_fact(fact.clone(), fact.embedding.clone())
        .await
        .expect("upsert");

    let connection =
        rusqlite::Connection::open(root.path().join("engram.db").join("engram_data.db"))
            .expect("sidecar db");
    connection
        .execute(
            "UPDATE memory_facts SET embedding_identity_json = ?1 WHERE id = ?2",
            params![
                json!({
                    "providerType": "fastembed",
                    "model": "test-2d-b",
                    "dimensions": 2,
                    "promptProfile": "query",
                    "normalization": null
                })
                .to_string(),
                "fact-model-mismatch"
            ],
        )
        .expect("mutate stored identity");
    let identity = query_identity("test-2d-a", 2);
    let hits = store
        .search_memory_facts_hybrid_with_identity(
            Some("agent-a"),
            "Rust",
            "semantic",
            5,
            None,
            Some(&[1.0, 0.0]),
            Some(&identity),
            None,
        )
        .await
        .expect("search");
    assert!(
        hits.is_empty(),
        "provider/model identity-mismatched stored vectors must be skipped until reindex compatibility is proven"
    );
}

#[tokio::test]
async fn external_query_embedding_without_identity_fails_closed() {
    let root = tempfile::tempdir().expect("root");
    let store =
        EngramMemoryFactStore::open(engram_config_with_embedding_identity(&root, "test-2d-a", 2))
            .expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-no-query-identity".to_string();
    fact.embedding = Some(vec![1.0, 0.0]);
    store
        .upsert_typed_fact(fact.clone(), fact.embedding.clone())
        .await
        .expect("upsert");

    let hits = store
        .search_memory_facts_hybrid(
            Some("agent-a"),
            "Rust",
            "semantic",
            5,
            None,
            Some(&[1.0, 0.0]),
            None,
        )
        .await
        .expect("search");

    assert!(
        hits.is_empty(),
        "external semantic vectors without provider/model identity must not reach vector search"
    );
}

#[tokio::test]
async fn external_query_embedding_identity_mismatch_fails_closed() {
    let root = tempfile::tempdir().expect("root");
    let store =
        EngramMemoryFactStore::open(engram_config_with_embedding_identity(&root, "test-2d-a", 2))
            .expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-query-identity-mismatch".to_string();
    fact.embedding = Some(vec![1.0, 0.0]);
    store
        .upsert_typed_fact(fact.clone(), fact.embedding.clone())
        .await
        .expect("upsert");

    let identity = query_identity("test-2d-b", 2);
    let hits = store
        .search_memory_facts_hybrid_with_identity(
            Some("agent-a"),
            "Rust",
            "semantic",
            5,
            None,
            Some(&[1.0, 0.0]),
            Some(&identity),
            None,
        )
        .await
        .expect("search");

    assert!(
        hits.is_empty(),
        "same-dimension query vectors from another provider/model identity must not reach vector search"
    );

    let same_basename = query_identity("other/test-2d-a", 2);
    let hits = store
        .search_memory_facts_hybrid_with_identity(
            Some("agent-a"),
            "Rust",
            "semantic",
            5,
            None,
            Some(&[1.0, 0.0]),
            Some(&same_basename),
            None,
        )
        .await
        .expect("same-basename search");
    assert!(
        hits.is_empty(),
        "same-basename model identities must not be treated as compatible"
    );
}

#[tokio::test]
async fn live_embedding_identity_mismatch_degrades_before_vector_search() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config_with_embedding_identity(&root, "test-2d-a", 2);
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let embedder = Arc::new(MutableIdentityEmbedder {
        model: Mutex::new("test-2d-a".to_string()),
        calls: AtomicUsize::new(0),
    });
    let embedding_client: Arc<dyn EmbeddingClient> = embedder.clone();
    let store = EngramMemoryFactStore::from_provider_with_embedding_client(
        config,
        &provider,
        Some(embedding_client),
    )
    .expect("store");
    let mut fact = sample_fact();
    fact.id = "fact-live-model-drift".to_string();
    fact.embedding = Some(vec![1.0, 0.0]);
    store
        .upsert_typed_fact(fact.clone(), fact.embedding.clone())
        .await
        .expect("upsert");

    *embedder.model.lock().expect("model lock") = "test-2d-b".to_string();
    let recalled = store
        .recall_facts_prioritized("agent-a", "Rust preferences", 5, None)
        .await
        .expect("recall");

    assert_eq!(recalled["degraded"], json!(true));
    assert_eq!(recalled["reason"], "embedding_identity_mismatch");
    assert_eq!(
        embedder.calls.load(Ordering::SeqCst),
        0,
        "identity mismatch must fail closed before embedding/search"
    );

    let mut drifted_fact = sample_fact();
    drifted_fact.id = "fact-live-model-drift-write".to_string();
    drifted_fact.embedding = Some(vec![1.0, 0.0]);
    let err = store
        .upsert_typed_fact(drifted_fact.clone(), drifted_fact.embedding.clone())
        .await
        .expect_err("model drift must block vector writes");
    assert!(err.to_string().contains("embedding_identity_mismatch"));
}
