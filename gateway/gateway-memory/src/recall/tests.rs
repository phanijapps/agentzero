//! Recall unit tests — moved verbatim from mod.rs.

use super::*;
use zbot_stores_traits::StoreResult;

fn mk_item(kind: ItemKind, id: &str, content: &str, score: f64) -> ScoredItem {
    ScoredItem {
        kind,
        id: id.to_string(),
        content: content.to_string(),
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

fn make_scored_fact(class: Option<&str>, superseded_by: Option<&str>, _score: f64) -> MemoryFact {
    MemoryFact {
        id: "fact-test".to_string(),
        session_id: None,
        agent_id: "agent-1".to_string(),
        scope: "agent".to_string(),
        category: "misc".to_string(),
        key: "test.key".to_string(),
        content: "test content".to_string(),
        confidence: 0.9,
        mention_count: 1,
        source_summary: None,
        embedding: None,
        ward_id: "__global__".to_string(),
        contradicted_by: None,
        created_at: String::new(),
        updated_at: String::new(),
        expires_at: None,
        valid_from: None,
        valid_until: None,
        superseded_by: superseded_by.map(|s| s.to_string()),
        pinned: false,
        epistemic_class: class.map(|s| s.to_string()),
        source_episode_id: None,
        source_ref: None,
        last_accessed: None,
        importance: None,
    }
}

#[tokio::test]
async fn recall_context_atoms_projects_unified_recall_items() {
    let recall = MemoryRecall::new(None, relaxed_recall_config());
    let goals = vec![GoalLite {
        id: "goal-1".to_string(),
        title: "Analyze AAPL".to_string(),
        unfilled_slot_names: Vec::new(),
    }];

    let atoms = recall
        .recall_context_atoms("agent", "valuation", Some("finance"), &goals, 10)
        .await
        .expect("context atoms");

    assert_eq!(atoms.len(), 1);
    let atom = &atoms[0];
    assert_eq!(atom.id, "goal-1");
    assert_eq!(atom.kind, "goal");
    assert_eq!(atom.source, "kg_goals");
    // Weighted RRF: goals weigh 1.2 at rank 1 → raw 1.2/61, then the
    // saturating normalization keeps it in [0, 1).
    let raw = (1.2_f64 / 61.0) * 60.0;
    let expected = raw / (1.0 + raw);
    assert!(
        (atom.score - expected).abs() < 1e-9,
        "goal atom fused score {} ≈ {expected}",
        atom.score
    );
    assert!((atom.confidence - atom.score).abs() < f64::EPSILON);
    assert_eq!(atom.route_hint.as_ref().unwrap()["source_kind"], "goal");
    assert!(serde_json::to_string(atom)
        .expect("atom serializes")
        .find("embedding")
        .is_none());
}

#[tokio::test]
async fn scoped_unified_outcome_rejects_missing_provenance_before_fusion() {
    let recall = MemoryRecall::new(None, relaxed_recall_config());
    let goals = vec![
        GoalLite {
            id: "allowed-goal".to_string(),
            title: "Authorized goal".to_string(),
            unfilled_slot_names: Vec::new(),
        },
        GoalLite {
            id: "blocked-goal".to_string(),
            title: "Cross-scope goal".to_string(),
            unfilled_slot_names: Vec::new(),
        },
    ];
    let visible = |item: &ScoredItem| {
        item.provenance.ward_id.as_deref() == Some("ward-a")
            && item.provenance.session_id.as_deref() == Some("sess-a")
    };

    let outcome = recall
        .recall_unified_outcome_scoped(
            "agent-a",
            "goal",
            Some("ward-a"),
            &goals,
            5,
            UnifiedRecallScope::new(&visible, true),
        )
        .await
        .expect("scoped outcome");

    assert!(outcome.items.is_empty());
}

#[tokio::test]
async fn scoped_unified_outcome_admits_explicitly_classified_durable_goals() {
    let recall = MemoryRecall::new(None, relaxed_recall_config());
    let goals = vec![GoalLite {
        id: "goal-a".to_string(),
        title: "Authorized durable goal".to_string(),
        unfilled_slot_names: Vec::new(),
    }];
    let visible = |item: &ScoredItem| {
        item.provenance.source == "kg_goals"
            && item.provenance.ward_id.as_deref() == Some("ward-a")
            && item.provenance.session_id.as_deref() == Some("__global__")
    };

    let outcome = recall
        .recall_unified_outcome_scoped(
            "agent-a",
            "goal",
            Some("ward-a"),
            &goals,
            5,
            UnifiedRecallScope::new(&visible, true),
        )
        .await
        .expect("scoped outcome");

    assert_eq!(outcome.items.len(), 1);
    assert_eq!(outcome.items[0].id, "goal-a");
}

#[test]
fn scoped_visibility_only_admits_explicit_authorized_provenance() {
    let mut items = vec![
        mk_item(ItemKind::Fact, "missing", "missing scope", 1.0),
        ScoredItem {
            provenance: Provenance {
                source: "memory_facts".to_string(),
                source_id: "explicit-global".to_string(),
                ward_id: Some("__global__".to_string()),
                session_id: Some("__global__".to_string()),
            },
            ..mk_item(ItemKind::Fact, "explicit-global", "global scope", 0.9)
        },
    ];
    let visible = |item: &ScoredItem| {
        item.provenance.ward_id.as_deref() == Some("__global__")
            && item.provenance.session_id.as_deref() == Some("__global__")
    };

    apply_scoped_candidate_visibility(&mut items, &visible);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "explicit-global");
}

#[tokio::test]
async fn scoped_unified_outcome_hides_taxonomy_when_scope_cannot_be_proven() {
    let recall = MemoryRecall::new(None, relaxed_recall_config());
    let visible = |_item: &ScoredItem| true;

    let outcome = recall
        .recall_unified_outcome_scoped(
            "agent-a",
            "taxonomy",
            Some("ward-a"),
            &[],
            5,
            UnifiedRecallScope::new(&visible, false),
        )
        .await
        .expect("scope denial is a safe partial outcome");

    assert!(outcome.taxonomy_expansion.is_none());
    assert_eq!(
        outcome.source_summary.taxonomy,
        UnifiedRecallSourceStatus::unavailable(0)
    );
}

#[test]
fn recall_facts_retains_only_items_above_min_score() {
    use std::sync::Arc;
    let config = Arc::new(RecallConfig::default()); // default min_score = 0.3

    // Simulate what recall_facts does: sort → retain → truncate
    let mut results = vec![
        mk_item(ItemKind::Fact, "high", "high relevance", 0.9),
        mk_item(ItemKind::Fact, "mid", "borderline", 0.3),
        mk_item(ItemKind::Fact, "low", "chess procedures", 0.1),
    ];
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.retain(|sf| sf.score >= config.min_score);
    results.truncate(10);

    assert_eq!(results.len(), 2, "low-score item should be filtered");
    assert!(results.iter().any(|i| i.id == "high"));
    assert!(results.iter().any(|i| i.id == "mid"));
    assert!(
        !results.iter().any(|i| i.id == "low"),
        "chess procedures should be suppressed"
    );
}

#[test]
fn bounded_recall_embedding_query_caps_oversized_input() {
    let long = "x".repeat(MAX_RECALL_EMBED_QUERY_CHARS + 500);
    let bounded = bounded_recall_embedding_query(&long, MAX_RECALL_EMBED_QUERY_CHARS);

    assert_eq!(bounded.chars().count(), MAX_RECALL_EMBED_QUERY_CHARS);
}

#[test]
fn recall_facts_excludes_superseded() {
    // Sanity test: retain logic drops items whose underlying fact has
    // superseded_by set. Verifies the filter intent directly without the
    // full recall pipeline.
    use zbot_stores_domain::MemoryFact;

    let mk_fact = |id: &str, superseded: Option<&str>| MemoryFact {
        id: id.to_string(),
        session_id: None,
        agent_id: "agent".to_string(),
        scope: "agent".to_string(),
        category: "schema".to_string(),
        key: format!("schema.{id}"),
        content: "content".to_string(),
        confidence: 0.9,
        mention_count: 1,
        source_summary: None,
        embedding: None,
        ward_id: "__global__".to_string(),
        contradicted_by: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        expires_at: None,
        valid_from: None,
        valid_until: None,
        superseded_by: superseded.map(String::from),
        pinned: false,
        epistemic_class: Some("current".to_string()),
        source_episode_id: None,
        source_ref: None,
        last_accessed: None,
        importance: None,
    };

    let facts = vec![
        mk_fact("a", None),
        mk_fact("b", Some("a")),
        mk_fact("c", None),
    ];

    let kept: Vec<_> = facts
        .into_iter()
        .filter(|f| f.superseded_by.is_none())
        .collect();
    assert_eq!(kept.len(), 2);
    assert!(kept.iter().any(|f| f.id == "a"));
    assert!(kept.iter().any(|f| f.id == "c"));
    assert!(!kept.iter().any(|f| f.id == "b"));
}

// ========================================================================

use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
use std::sync::Mutex;
use zbot_stores_domain::{Belief, ScoredBelief};
use zbot_stores_traits::BeliefStore;

/// Test embedder that returns a fixed vector — only used for B-4
/// recall tests where the magnitudes don't matter, only presence.
struct TestEmbed;
#[async_trait]
impl EmbeddingClient for TestEmbed {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|_| vec![1.0_f32, 0.0, 0.0]).collect())
    }
    fn dimensions(&self) -> usize {
        3
    }
    fn model_name(&self) -> String {
        "test".to_string()
    }
}

struct LengthCheckingEmbed {
    max_chars: usize,
    seen: Mutex<Vec<usize>>,
}

impl LengthCheckingEmbed {
    fn new(max_chars: usize) -> Arc<Self> {
        Arc::new(Self {
            max_chars,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen_lengths(&self) -> Vec<usize> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl EmbeddingClient for LengthCheckingEmbed {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        for text in texts {
            let chars = text.chars().count();
            self.seen.lock().unwrap().push(chars);
            if chars > self.max_chars {
                return Err(EmbeddingError::ApiError(format!(
                    "input length exceeds context length: {chars}"
                )));
            }
        }
        Ok(texts.iter().map(|_| vec![1.0_f32, 0.0, 0.0]).collect())
    }

    fn dimensions(&self) -> usize {
        3
    }

    fn model_name(&self) -> String {
        "length-checking".to_string()
    }
}

struct FailingEmbed;

#[async_trait]
impl EmbeddingClient for FailingEmbed {
    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Err(EmbeddingError::ApiError(
            "input length exceeds context length".to_string(),
        ))
    }

    fn dimensions(&self) -> usize {
        384
    }

    fn model_name(&self) -> String {
        "failing".to_string()
    }
}

#[tokio::test]
async fn recall_unified_bounds_query_before_embedding_provider_call() {
    let embed = LengthCheckingEmbed::new(MAX_RECALL_EMBED_QUERY_CHARS);
    let embed_dyn: Arc<dyn EmbeddingClient> = embed.clone();
    let recall = MemoryRecall::new(Some(embed_dyn), Arc::new(RecallConfig::default()));

    let long_query = "memory hygiene ".repeat(400);
    let _ = recall
        .recall_unified("agent", &long_query, None, &[], 5)
        .await
        .unwrap();

    let seen = embed.seen_lengths();
    assert_eq!(seen, vec![MAX_RECALL_EMBED_QUERY_CHARS]);
}

#[tokio::test]
async fn recall_unified_retries_smaller_query_after_provider_context_error() {
    let embed = LengthCheckingEmbed::new(RETRY_RECALL_EMBED_QUERY_CHARS);
    let embed_dyn: Arc<dyn EmbeddingClient> = embed.clone();
    let recall = MemoryRecall::new(Some(embed_dyn), Arc::new(RecallConfig::default()));

    let long_query = "memory hygiene ".repeat(400);
    let _ = recall
        .recall_unified("agent", &long_query, None, &[], 5)
        .await
        .unwrap();

    let seen = embed.seen_lengths();
    assert_eq!(
        seen,
        vec![MAX_RECALL_EMBED_QUERY_CHARS, RETRY_RECALL_EMBED_QUERY_CHARS]
    );
}

#[tokio::test]
async fn recall_unified_skips_fuzzy_results_when_embedding_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let store = make_memory_store_with_embedder(&tmp, Arc::new(DirectionalEmbed)).await;
    store
        .save_fact(
            "agent",
            "domain",
            "memory.hygiene",
            "memory hygiene guard should still be found by lexical recall",
            0.95,
            None,
            None,
        )
        .await
        .unwrap();

    let mut recall = MemoryRecall::new(Some(Arc::new(FailingEmbed)), relaxed_recall_config());
    recall.set_memory_store(store);

    let out = recall
        .recall_unified("agent", "memory hygiene guard", None, &[], 10)
        .await
        .unwrap();

    assert!(
        out.iter().all(|item| item.id != "fact:memory.hygiene"
            && !item.content.contains("memory hygiene guard")),
        "fuzzy lexical results must not survive embedding failure: {out:?}"
    );
}

/// In-memory belief store that returns a fixed list from
/// `search_beliefs` and records the call count. Other methods are
/// no-ops or empty — only what `recall_unified` calls matters.
struct StubBeliefStore {
    canned: Vec<ScoredBelief>,
    search_calls: std::sync::atomic::AtomicUsize,
}

impl StubBeliefStore {
    fn new(canned: Vec<ScoredBelief>) -> Self {
        Self {
            canned,
            search_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    fn search_calls(&self) -> usize {
        self.search_calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

use async_trait::async_trait;

#[async_trait]
impl BeliefStore for StubBeliefStore {
    async fn get_belief(
        &self,
        _: &str,
        _: &str,
        _: Option<chrono::DateTime<chrono::Utc>>,
    ) -> StoreResult<Option<Belief>> {
        Ok(None)
    }
    async fn list_beliefs(&self, _: &str, _: usize) -> StoreResult<Vec<Belief>> {
        Ok(vec![])
    }
    async fn upsert_belief(&self, _: &Belief) -> StoreResult<()> {
        Ok(())
    }
    async fn supersede_belief(
        &self,
        _: &str,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn mark_stale(&self, _: &str) -> StoreResult<()> {
        Ok(())
    }
    async fn retract_belief(&self, _: &str, _: chrono::DateTime<chrono::Utc>) -> StoreResult<()> {
        Ok(())
    }
    async fn beliefs_referencing_fact(&self, _: &str) -> StoreResult<Vec<String>> {
        Ok(vec![])
    }
    async fn get_belief_by_id(&self, _: &str) -> StoreResult<Option<Belief>> {
        Ok(None)
    }
    async fn list_stale(&self, _: &str, _: usize) -> StoreResult<Vec<Belief>> {
        Ok(vec![])
    }
    async fn clear_stale(&self, _: &str) -> StoreResult<()> {
        Ok(())
    }
    async fn search_beliefs(&self, _: &str, _: &[f32], _: usize) -> StoreResult<Vec<ScoredBelief>> {
        self.search_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self.canned.clone())
    }
}

fn sample_belief(id: &str, subject: &str, content: &str) -> Belief {
    let now = chrono::Utc::now();
    Belief {
        id: id.to_string(),
        partition_id: "ag".to_string(),
        subject: subject.to_string(),
        content: content.to_string(),
        confidence: 0.85,
        valid_from: Some(now),
        valid_until: None,
        source_fact_ids: vec![],
        synthesizer_version: 1,
        reasoning: None,
        created_at: now,
        updated_at: now,
        superseded_by: None,
        stale: false,
        embedding: None,
    }
}

/// When a belief_store is wired AND a query embedding is produced,
/// `recall_unified` calls `search_beliefs` exactly once and the
/// returned items carry `ItemKind::Belief`.
#[tokio::test]
async fn recall_unified_fetches_beliefs_when_store_wired() {
    let beliefs = vec![
        ScoredBelief {
            belief: sample_belief("b1", "user.location", "User lives in Mason, OH"),
            score: 0.9,
        },
        ScoredBelief {
            belief: sample_belief("b2", "user.diet", "User is vegetarian"),
            score: 0.7,
        },
    ];
    let stub: Arc<StubBeliefStore> = Arc::new(StubBeliefStore::new(beliefs));
    let stub_dyn: Arc<dyn BeliefStore> = stub.clone();

    let config = Arc::new(RecallConfig::default());
    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let mut recall = MemoryRecall::new(Some(embed), config);
    recall.set_belief_store(stub_dyn);

    let out = recall
        .recall_unified("ag", "where do I live?", None, &[], 20)
        .await
        .unwrap();

    assert_eq!(stub.search_calls(), 1, "search_beliefs called exactly once");
    let belief_count = out
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Belief))
        .count();
    assert_eq!(belief_count, 2, "both stub beliefs surface");
}

/// When no belief_store is wired, `recall_unified` produces zero
/// belief items — pre-B-4 behavior preserved byte-for-byte.
#[tokio::test]
async fn recall_unified_does_not_fetch_beliefs_when_store_absent() {
    let config = Arc::new(RecallConfig::default());
    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let recall = MemoryRecall::new(Some(embed), config);

    let out = recall
        .recall_unified("ag", "where do I live?", None, &[], 20)
        .await
        .unwrap();
    assert!(
        !out.iter().any(|i| matches!(i.kind, ItemKind::Belief)),
        "no belief_store wired ⇒ no belief items"
    );
}

/// Direct assertion on the category-weight map: belief = 1.5,
/// matching corrections and sitting below schema (1.6) — the
/// design doc Q4 decision.
#[test]
fn belief_category_weight_is_one_point_five() {
    let config = RecallConfig::default();
    assert!((config.category_weight("belief") - 1.5).abs() < f64::EPSILON);
}

/// Schemas (1.6) outrank beliefs (1.5) when both carry the same
/// raw score — proves the weight ordering survives rescore.
#[test]
fn schemas_outrank_beliefs_at_equal_raw_score() {
    let config = RecallConfig::default();
    let raw = 0.5_f64;
    let schema_weighted = raw * config.category_weight("schema");
    let belief_weighted = raw * config.category_weight("belief");
    assert!(
        schema_weighted > belief_weighted,
        "schema ({schema_weighted}) must beat belief ({belief_weighted}) at equal raw score"
    );
}

// ========================================================================
// MMR — Maximal Marginal Relevance integration tests
//
// These tests exercise the full recall_unified pipeline with MMR enabled
// vs disabled, end-to-end through a real SQLite-backed memory store.
// ========================================================================

use crate::MmrConfig;

/// 384-dim is what the SQLite vec0 table expects. We synthesize
/// orthogonal directions by setting exactly one of the first three
/// components to 1.0. Cosine similarity is then `1.0` between
/// same-direction items and `0.0` between different directions.
const EMBED_DIM: usize = 384;

/// Directional embedder used by MMR integration tests. Routes input
/// text to one of three orthogonal directions based on keywords —
/// lets tests set up controlled near-duplicate vs. diverse scenarios.
struct DirectionalEmbed;

#[async_trait]
impl EmbeddingClient for DirectionalEmbed {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|t| direction_for_text(t)).collect())
    }
    fn dimensions(&self) -> usize {
        EMBED_DIM
    }
    fn model_name(&self) -> String {
        "directional".to_string()
    }
}

fn direction_for_text(text: &str) -> Vec<f32> {
    let t = text.to_lowercase();
    let mut v = vec![0.0_f32; EMBED_DIM];
    // Order matters: "apple/banana/fruit" wins over "color/red" if
    // both appear, but our test contents don't overlap.
    let idx = if t.contains("apple") || t.contains("banana") || t.contains("fruit") {
        0
    } else if t.contains("color") || t.contains("red") || t.contains("blue") {
        1
    } else {
        2
    };
    v[idx] = 1.0;
    // A tilt toward the fruit axis for color texts: keeps the color
    // direction dominant (color queries match color facts first) while
    // putting fruit-direction queries at mid-relevance cosine
    // (0.5/sqrt(1.25) ≈ 0.45) — above the admission guard so the
    // diverse candidate enters the fused pool, which is what MMR
    // diversifies. A pure one-hot made the cross-direction cosine 0
    // and guard-dropped, leaving MMR nothing to diversify with.
    if idx == 1 {
        v[0] = 0.5;
    }
    v
}

use knowledge_graph::types::{Entity, EntityType};

/// `save_fact` generates embeddings that we can later look up via
/// `get_fact_embedding`.
async fn make_memory_store_with_embedder(
    tmp: &tempfile::TempDir,
    embed: Arc<dyn EmbeddingClient>,
) -> Arc<dyn zbot_stores_traits::MemoryFactStore> {
    // Production wiring: the engram adapter store, with the test
    // embedder's identity propagated so identity-aware hybrid paths
    // are exercised for real (no sqlite fallback wrapper needed).
    use zbot_engram_adapter::{AdapterConfig, EngramMemoryFactStore, EngramProvider};
    let root = tmp.path().join("engram-recall-fixture");
    std::fs::create_dir_all(&root).expect("engram fixture root");
    let mut config = AdapterConfig::engram_for_data_root(&root, "engram.db");
    config.embedding_provider.provider_type = embed.provider_type();
    config.embedding_provider.model = embed.model_name();
    config.embedding_provider.dimensions = embed.dimensions() as u32;
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    Arc::new(
        EngramMemoryFactStore::from_provider_with_embedding_client(config, &provider, Some(embed))
            .expect("fixture fact store opens"),
    )
}

/// A real graph ANN fixture used to prove taxonomy configuration is
/// additive. It uses the same bounded embedding dimension as facts so the
/// `kg_name_index` path is exercised rather than mocked.
async fn make_kg_store_with_apple_entity(
    tmp: &tempfile::TempDir,
) -> Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore> {
    // Production wiring: the engram KG store, with the fixture
    // embedder's identity dimensions so the kg_name_index path is
    // exercised for real.
    use zbot_engram_adapter::{AdapterConfig, EngramKnowledgeGraphStore, EngramProvider};
    let root = tmp.path().join("engram-kg-recall-fixture");
    std::fs::create_dir_all(&root).expect("engram fixture root");
    let mut config = AdapterConfig::engram_for_data_root(&root, "engram.db");
    // Adopt the query embedder's identity so the identity-aware ANN
    // lane accepts the stored vectors (same discipline as the fact
    // fixture above).
    let embedder = DirectionalEmbed;
    config.embedding_provider.provider_type = embedder.provider_type();
    config.embedding_provider.model = embedder.model_name();
    config.embedding_provider.dimensions = embedder.dimensions() as u32;
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    let store: Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore> = Arc::new(
        EngramKnowledgeGraphStore::from_provider(config, &provider).expect("kg fixture opens"),
    );
    let mut entity = Entity::new(
        "agent-a".to_string(),
        EntityType::Concept,
        "Apple Knowledge Graph".to_string(),
    );
    entity.id = "taxonomy-disabled-graph".to_string();
    entity.name_embedding = Some(direction_for_text("apple"));
    store
        .upsert_entity("agent-a", entity)
        .await
        .expect("graph entity");
    store
}

/// RecallConfig with min_score relaxed to 0 — the hybrid scorer
/// produces values around 0.01–0.05 for synthetic test corpora,
/// well below the default 0.3 threshold. Tests that exercise the
/// real store need this so any facts survive the filter.
fn relaxed_recall_config() -> Arc<RecallConfig> {
    Arc::new(RecallConfig {
        min_score: 0.0,
        ..RecallConfig::default()
    })
}

#[tokio::test]
async fn location_dependent_recall_reserves_canonical_location_fact() {
    let tmp = tempfile::tempdir().unwrap();
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;

    let now = chrono::Utc::now().to_rfc3339();
    store
        .upsert_typed_fact(
            serde_json::from_value(serde_json::json!({
                    "id": "agent-scoped-home-base",
                    "session_id": null,
                    "agent_id": "agent",
                    "scope": "agent",
                    "category": "user",
                    "key": "user.location.home_base",
                    "content": "canonical home location",
                    "confidence": 0.95,
                    "mention_count": 1,
                    "source_summary": null,
                    "ward_id": "__global__",
                    "contradicted_by": null,
                    "created_at": now,
                    "updated_at": now,
                    "expires_at": null,
                    "valid_from": null,
                    "valid_until": null,
                    "superseded_by": null,
                    "pinned": false,
                    "epistemic_class": "current",
                    "source_episode_id": null,
                    "source_ref": null }))
            .unwrap(),
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "user",
            "user.location.unknown",
            "unknown location",
            0.99,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "user",
            "user.name",
            "canonical profile name",
            0.95,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "domain",
            "domain.weather",
            "weather reference material",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "domain",
            "domain.apple",
            "apple is a fruit",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();

    let mut recall = MemoryRecall::new(Some(embed), relaxed_recall_config());
    recall.set_memory_store(store);

    let local = recall
        .recall_unified("agent", "what is the weather near me", None, &[], 1)
        .await
        .unwrap();
    assert_eq!(local.len(), 1);
    assert!(
        local[0].content.contains("user.location.home_base"),
        "local-context recall must select the canonical location first: {:?}",
        local.iter().map(|item| &item.content).collect::<Vec<_>>()
    );

    let identity = recall
        .recall_unified("agent", "what is my name", None, &[], 1)
        .await
        .unwrap();
    assert_eq!(identity.len(), 1);
    assert!(
        identity[0].content.contains("user.name"),
        "identity recall must select the canonical name first: {:?}",
        identity
            .iter()
            .map(|item| &item.content)
            .collect::<Vec<_>>()
    );

    let unrelated = recall
        .recall_unified("agent", "apple fruit", None, &[], 1)
        .await
        .unwrap();
    assert_eq!(
        unrelated.len(),
        1,
        "unrelated recall must preserve its generic result budget"
    );
    assert!(
        unrelated[0].content.contains("domain.apple"),
        "unrelated recall must not reserve profile context: {:?}",
        unrelated
            .iter()
            .map(|item| &item.content)
            .collect::<Vec<_>>()
    );
}

#[test]
fn profile_query_cues_select_only_requested_slots() {
    assert_eq!(
        profile_fact_key_sets_for_query("what is my name?"),
        vec![IDENTITY_PROFILE_KEYS]
    );
    assert_eq!(
        profile_fact_key_sets_for_query("what is the weather near me?"),
        vec![LOCATION_PROFILE_KEYS]
    );
    assert_eq!(
        profile_fact_key_sets_for_query("where am I?"),
        vec![LOCATION_PROFILE_KEYS]
    );
    assert!(profile_fact_key_sets_for_query("tell me about apples").is_empty());
}

/// MMR disabled → recall_unified output identical to no-MMR.
/// Sanity check that the gating is the whole seam.
#[tokio::test]
async fn mmr_disabled_pipeline_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;

    store
        .save_fact(
            "agent",
            "domain",
            "f.apple",
            "apple is a fruit",
            0.8,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "domain",
            "f.banana",
            "banana is a fruit",
            0.8,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "domain",
            "f.color",
            "red is a color",
            0.8,
            None,
            None,
        )
        .await
        .unwrap();

    let config = relaxed_recall_config();
    let mut recall = MemoryRecall::new(Some(embed), config);
    recall.set_memory_store(store.clone());

    // Baseline (no MMR wired).
    let baseline = recall
        .recall_unified("agent", "fruit", None, &[], 10)
        .await
        .unwrap();

    // Explicit disabled.
    recall.set_mmr_config(MmrConfig {
        enabled: false,
        lambda: 0.5,
        candidate_pool: 30,
    });
    let disabled = recall
        .recall_unified("agent", "fruit", None, &[], 10)
        .await
        .unwrap();

    assert_eq!(
        baseline.len(),
        disabled.len(),
        "disabled MMR must not change result count"
    );
    let baseline_ids: Vec<&str> = baseline.iter().map(|i| i.id.as_str()).collect();
    let disabled_ids: Vec<&str> = disabled.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        baseline_ids, disabled_ids,
        "disabled MMR must not change result order"
    );
}

/// MMR enabled with two duplicate-direction facts → only one survives
/// in the top-2 output; the diverse third fact takes the other slot.
#[tokio::test]
async fn mmr_drops_near_duplicate_in_top_k() {
    let tmp = tempfile::tempdir().unwrap();
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;

    // Two facts in the "fruit" direction (near-duplicate embeddings).
    store
        .save_fact(
            "agent",
            "domain",
            "f.apple",
            "apple is a fruit",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .save_fact(
            "agent",
            "domain",
            "f.banana",
            "banana is a fruit",
            0.85,
            None,
            None,
        )
        .await
        .unwrap();
    // One fact in a different direction (orthogonal).
    store
        .save_fact(
            "agent",
            "domain",
            "f.color",
            "red is a color",
            0.6,
            None,
            None,
        )
        .await
        .unwrap();

    let config = relaxed_recall_config();
    let mut recall = MemoryRecall::new(Some(embed), config);
    recall.set_memory_store(store.clone());
    recall.set_mmr_config(MmrConfig {
        enabled: true,
        lambda: 0.5,
        candidate_pool: 30,
    });

    let out = recall
        .recall_unified("agent", "fruit", None, &[], 2)
        .await
        .unwrap();

    assert_eq!(out.len(), 2, "budget=2 → 2 items");
    // The color fact must be present — diversity bonus pushes it over
    // the second fruit fact.
    let color_present = out.iter().any(|i| i.content.contains("f.color"));
    let fruit_count = out
        .iter()
        .filter(|i| i.content.contains("f.apple") || i.content.contains("f.banana"))
        .count();
    assert!(
        color_present,
        "MMR should surface the orthogonal color fact over a second near-duplicate fruit"
    );
    assert_eq!(
        fruit_count, 1,
        "exactly one fruit fact should survive the diversity penalty"
    );
}

/// Belief + source fact in the same direction — MMR keeps just one.
/// This is the canonical B-4 case the design doc Q3 deferred to MMR.
#[tokio::test]
async fn mmr_dedups_belief_and_source_fact() {
    let tmp = tempfile::tempdir().unwrap();
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;

    // Source fact in the "fruit" direction.
    store
        .save_fact(
            "agent",
            "user",
            "f.fruit_preference",
            "user likes apple",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    // An orthogonal fact (color direction) to provide the diverse pick.
    store
        .save_fact(
            "agent",
            "domain",
            "f.color",
            "red is a color the user mentioned",
            0.4,
            None,
            None,
        )
        .await
        .unwrap();

    // Belief overlaps semantically with the fruit fact (the embedder
    // routes "apple" content into the same direction).
    let belief = sample_belief(
        "b-fruit",
        "user.preferences",
        "user prefers apple-flavored things",
    );
    let belief_emb_bytes: Vec<u8> = direction_for_text(&belief.content)
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let mut belief_with_emb = belief.clone();
    belief_with_emb.embedding = Some(belief_emb_bytes);
    let stub: Arc<dyn BeliefStore> = Arc::new(StubBeliefStore::new(vec![ScoredBelief {
        belief: belief_with_emb,
        score: 0.95,
    }]));

    let config = relaxed_recall_config();
    let mut recall = MemoryRecall::new(Some(embed), config);
    recall.set_memory_store(store.clone());
    recall.set_belief_store(stub);
    recall.set_mmr_config(MmrConfig {
        enabled: true,
        lambda: 0.5,
        candidate_pool: 30,
    });

    let out = recall
        .recall_unified("agent", "apple preferences", None, &[], 2)
        .await
        .unwrap();

    assert_eq!(out.len(), 2);
    // We must NOT see both the belief and the source fact at top-2 —
    // they're in the same direction. MMR keeps one, swaps the other
    // for the orthogonal color fact.
    let belief_count = out
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Belief))
        .count();
    let apple_fact_count = out
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Fact) && i.content.contains("apple"))
        .count();
    assert!(
        belief_count + apple_fact_count <= 1,
        "MMR should keep only one of (belief, apple fact) at top-2, got {} belief + {} apple fact",
        belief_count,
        apple_fact_count
    );
    let color_present = out.iter().any(|i| i.content.contains("f.color"));
    assert!(
        color_present,
        "the diverse orthogonal item should be selected"
    );
}

/// `candidate_pool` controls the over-fetch from RRF. With a wide
/// pool, MMR sees the orthogonal item even when many high-relevance
/// near-duplicates exist. Proves the knob is honored on the recall
/// path.
#[tokio::test]
async fn mmr_candidate_pool_is_respected() {
    let tmp = tempfile::tempdir().unwrap();
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;

    // 6 fruit facts (all in the same direction).
    for i in 0..6 {
        store
            .save_fact(
                "agent",
                "domain",
                &format!("f.fruit_{i}"),
                &format!("fruit fact {i} apple"),
                0.9 - (i as f64) * 0.01,
                None,
                None,
            )
            .await
            .unwrap();
    }
    // One orthogonal color fact at mid-relevance.
    store
        .save_fact(
            "agent",
            "domain",
            "f.color",
            "red is a color",
            0.5,
            None,
            None,
        )
        .await
        .unwrap();

    let config = relaxed_recall_config();
    let mut recall = MemoryRecall::new(Some(embed.clone()), config.clone());
    recall.set_memory_store(store.clone());
    recall.set_mmr_config(MmrConfig {
        enabled: true,
        lambda: 0.5,
        candidate_pool: 30,
    });
    let wide = recall
        .recall_unified("agent", "fruit", None, &[], 3)
        .await
        .unwrap();
    assert!(
        wide.iter().any(|i| i.content.contains("f.color")),
        "with candidate_pool=30, the diverse color fact must appear at top-3 — got {:?}",
        wide.iter().map(|i| &i.content).collect::<Vec<_>>()
    );
}

/// Empty candidate list → empty result, no panic.
#[tokio::test]
async fn mmr_empty_candidates_yields_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;

    let config = relaxed_recall_config();
    let mut recall = MemoryRecall::new(Some(embed), config);
    recall.set_memory_store(store);
    recall.set_mmr_config(MmrConfig {
        enabled: true,
        lambda: 0.5,
        candidate_pool: 30,
    });

    let out = recall
        .recall_unified("agent", "anything", None, &[], 10)
        .await
        .unwrap();
    assert!(out.is_empty());
}

// -----------------------------------------------------------------
// H-4: hierarchical-memory LCA recall integration
// -----------------------------------------------------------------
//
// The KnowledgeGraphStore trait has 25+ required methods so a
// crate-local stub becomes 200+ lines of `Ok(Default::default())`.
// We rely instead on the focused tests below + the unit tests
// sitting next to the SQLite impl (zbot-stores-sqlite::kg::storage
// `lca_*` family) for end-to-end coverage. The recall pipeline's
// H-4 branch is also exercised indirectly by the existing
// recall_unified_* tests above (which all pass after H-4 lands).

/// When no `kg_store` is wired, the LCA branch must short-circuit
/// — recall behaviour is byte-for-byte unchanged from pre-H-4.
#[tokio::test]
async fn recall_unified_does_not_query_lca_when_kg_store_absent() {
    let config = Arc::new(RecallConfig::default());
    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let recall = MemoryRecall::new(Some(embed), config);
    let out = recall
        .recall_unified("ag", "anything", None, &[], 20)
        .await
        .unwrap();
    assert!(
        !out.iter().any(|i| matches!(i.kind, ItemKind::HierEntity)),
        "no kg_store wired ⇒ no HierEntity items"
    );
}

#[tokio::test]
async fn recall_unified_uses_taxonomy_expanded_query_for_retrieval() {
    struct RecordingExpandedQueryStore {
        saw_query: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl zbot_stores_traits::MemoryFactStore for RecordingExpandedQueryStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!({"success": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!([]))
        }

        async fn search_memory_facts_hybrid_with_identity(
            &self,
            _agent_id: Option<&str>,
            query: &str,
            _mode: &str,
            _limit: usize,
            _ward_id: Option<&str>,
            _query_embedding: Option<&[f32]>,
            _query_identity: Option<&zbot_stores_traits::EmbeddingQueryIdentity>,
            _as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Vec<serde_json::Value>> {
            *self.saw_query.lock().unwrap() = Some(query.to_string());
            Ok(Vec::new())
        }
    }

    struct StaticTaxonomyExpander;

    #[async_trait]
    impl zbot_stores_traits::RecallTaxonomyExpander for StaticTaxonomyExpander {
        async fn expand_recall_query(
            &self,
            request: zbot_stores_traits::RecallTaxonomyExpansionRequest,
        ) -> StoreResult<zbot_stores_traits::RecallTaxonomyExpansion> {
            assert_eq!(request.max_depth, 1);
            assert_eq!(request.max_fan_out, 8);
            assert_eq!(request.max_candidates, 16);
            Ok(zbot_stores_traits::RecallTaxonomyExpansion {
                expanded_query: format!("{} Knowledge Graph", request.query),
                candidates: vec![zbot_stores_traits::RecallTaxonomyExpansionCandidate {
                    scheme_id: "zbot.general:v1".to_string(),
                    concept_id: "zbot.general:v1:concept:knowledge_graph".to_string(),
                    label: "Knowledge Graph".to_string(),
                    matched_label: "kg".to_string(),
                    relation: Some("alt_label".to_string()),
                    depth: 0,
                }],
            })
        }
    }

    let saw_query = Arc::new(Mutex::new(None));
    let store: Arc<dyn zbot_stores_traits::MemoryFactStore> =
        Arc::new(RecordingExpandedQueryStore {
            saw_query: saw_query.clone(),
        });
    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let mut recall = MemoryRecall::new(Some(embed), Arc::new(RecallConfig::default()));
    recall.set_memory_store(store);
    recall.set_taxonomy_expander(Arc::new(StaticTaxonomyExpander));

    let _ = recall
        .recall_unified("agent-a", "kg recall", None, &[], 5)
        .await
        .expect("unified recall");

    assert_eq!(
        saw_query.lock().unwrap().as_deref(),
        Some("kg recall Knowledge Graph")
    );

    let outcome = recall
        .recall_unified_outcome("agent-a", "kg recall", None, &[], 5)
        .await
        .expect("unified recall outcome");
    let taxonomy = outcome.taxonomy_expansion.expect("taxonomy trace");
    assert_eq!(taxonomy.retrieval_query, "kg recall Knowledge Graph");
    assert_eq!(taxonomy.candidates.len(), 1);
    assert_eq!(taxonomy.candidates[0].label, "Knowledge Graph");
    assert_eq!(
        taxonomy.candidates[0].relation,
        UnifiedRecallTaxonomyRelation::AltLabel
    );
    assert_eq!(
        outcome.source_summary.taxonomy.state,
        UnifiedRecallSourceState::Used
    );
    assert_eq!(outcome.source_summary.taxonomy.count, 1);
}
#[tokio::test]
async fn unconfigured_taxonomy_is_not_configured_and_leaves_fact_retrieval_intact() {
    let tmp = tempfile::tempdir().expect("temporary memory store");
    let embed: Arc<dyn EmbeddingClient> = Arc::new(DirectionalEmbed);
    let store = make_memory_store_with_embedder(&tmp, embed.clone()).await;
    let kg_store = make_kg_store_with_apple_entity(&tmp).await;
    store
        .save_fact(
            "agent-a",
            "domain",
            "taxonomy.disabled.fact",
            "apple taxonomy retrieval fact",
            0.9,
            None,
            None,
        )
        .await
        .expect("fact");

    let mut recall = MemoryRecall::new(Some(embed), relaxed_recall_config());
    recall.set_memory_store(store);
    recall.set_kg_store(kg_store);
    let outcome = recall
        .recall_unified_outcome("agent-a", "apple", None, &[], 8)
        .await
        .expect("unified recall");

    assert!(
        outcome.items.iter().any(|item| {
            item.kind == ItemKind::Fact
                && item.content.contains("taxonomy.disabled.fact")
                && item.content.contains("apple")
        }),
        "fact retrieval changed when taxonomy is unconfigured: {outcome:#?}"
    );
    assert!(
        outcome.items.iter().any(|item| {
            item.kind == ItemKind::GraphNode && item.content.contains("Apple Knowledge Graph")
        }),
        "graph retrieval changed when taxonomy is unconfigured: {outcome:#?}"
    );
    assert_eq!(
        outcome.source_summary.facts.state,
        UnifiedRecallSourceState::Used
    );
    assert_eq!(
        outcome.source_summary.graph.state,
        UnifiedRecallSourceState::Used
    );
    assert_eq!(
        outcome.source_summary.taxonomy,
        UnifiedRecallSourceStatus::not_configured()
    );
    assert!(outcome.taxonomy_expansion.is_none());
}

#[tokio::test]
async fn recall_unified_outcome_exposes_only_finite_source_failure_diagnostics() {
    struct FailingTaxonomyExpander;

    #[async_trait]
    impl zbot_stores_traits::RecallTaxonomyExpander for FailingTaxonomyExpander {
        async fn expand_recall_query(
            &self,
            _request: zbot_stores_traits::RecallTaxonomyExpansionRequest,
        ) -> StoreResult<zbot_stores_traits::RecallTaxonomyExpansion> {
            Err(zbot_stores_traits::StoreError::Backend(
                "taxonomy expansion failed (redacted fixture)".into(),
            ))
        }
    }

    let mut recall = MemoryRecall::new(None, Arc::new(RecallConfig::default()));
    recall.set_taxonomy_expander(Arc::new(FailingTaxonomyExpander));

    let outcome = recall
        .recall_unified_outcome("agent-a", "taxonomy failure", None, &[], 5)
        .await
        .expect("source failure degrades instead of failing recall");
    assert!(outcome.items.is_empty());
    assert!(outcome.taxonomy_expansion.is_none());
    assert_eq!(
        outcome.source_summary.taxonomy,
        UnifiedRecallSourceStatus {
            state: UnifiedRecallSourceState::Unavailable,
            count: 0,
            reason_code: Some(UnifiedRecallReasonCode::SourceUnavailable)
        }
    );
    assert!(format!("{:?}", outcome.source_summary).contains("SourceUnavailable"));
    assert!(!format!("{:?}", outcome.source_summary).contains("postgres://"));
    assert!(!format!("{:?}", outcome.source_summary).contains("/mnt/private"));
}

#[tokio::test]
async fn recall_unified_outcome_marks_a_failed_fact_source_unavailable() {
    struct FailingFactStore;

    #[async_trait]
    impl zbot_stores_traits::MemoryFactStore for FailingFactStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!({"success": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!([]))
        }

        async fn search_memory_facts_hybrid_with_identity(
            &self,
            _agent_id: Option<&str>,
            _query: &str,
            _mode: &str,
            _limit: usize,
            _ward_id: Option<&str>,
            _query_embedding: Option<&[f32]>,
            _query_identity: Option<&zbot_stores_traits::EmbeddingQueryIdentity>,
            _as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Vec<serde_json::Value>> {
            Err(zbot_stores_traits::StoreError::Backend(
                "store unavailable".into(),
            ))
        }
    }

    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let mut recall = MemoryRecall::new(Some(embed), Arc::new(RecallConfig::default()));
    recall.set_memory_store(Arc::new(FailingFactStore));

    let outcome = recall
        .recall_unified_outcome("agent-a", "fact failure", None, &[], 5)
        .await
        .expect("source failure degrades instead of failing recall");
    assert_eq!(
        outcome.source_summary.facts,
        UnifiedRecallSourceStatus {
            state: UnifiedRecallSourceState::Unavailable,
            count: 0,
            reason_code: Some(UnifiedRecallReasonCode::SourceUnavailable)
        }
    );
    assert!(!format!("{:?}", outcome.source_summary).contains("sqlite:///"));
}

#[test]
fn taxonomy_direct_match_defaults_to_pref_label_in_outcome() {
    let candidate = RecallTaxonomyExpansionCandidate {
        scheme_id: "zbot.general:v1".to_string(),
        concept_id: "zbot.general:v1:concept:memory".to_string(),
        label: "Memory".to_string(),
        matched_label: "Memory".to_string(),
        relation: None,
        depth: 0,
    };
    assert_eq!(
        normalized_taxonomy_relation(&candidate),
        UnifiedRecallTaxonomyRelation::PrefLabel
    );
}

#[test]
fn partial_graph_or_hierarchy_failure_is_degraded_not_unavailable() {
    for status in [
        source_status(true, true, true, 2, true),
        source_status(true, true, true, 1, true),
    ] {
        assert_eq!(status.state, UnifiedRecallSourceState::Degraded);
        assert_eq!(
            status.reason_code,
            Some(UnifiedRecallReasonCode::SourceUnavailable)
        );
        assert!(status.count > 0);
    }
}

#[tokio::test]
async fn recall_unified_keeps_rrf_scaled_engram_fact_with_default_min_score() {
    struct RrfScaleStore;

    #[async_trait]
    impl zbot_stores_traits::MemoryFactStore for RrfScaleStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!({"success": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!([]))
        }

        async fn search_memory_facts_hybrid(
            &self,
            _agent_id: Option<&str>,
            _query: &str,
            _mode: &str,
            _limit: usize,
            _ward_id: Option<&str>,
            _query_embedding: Option<&[f32]>,
            _as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Vec<serde_json::Value>> {
            let fact = make_scored_fact(Some("current"), None, 1.0);
            let mut value = serde_json::to_value(fact).expect("fact json");
            let object = value.as_object_mut().expect("fact object");
            object.insert("id".to_string(), serde_json::json!("fact-arxiv"));
            object.insert(
                "key".to_string(),
                serde_json::json!("arxiv.2602.03315.paper_under_review"),
            );
            object.insert(
                "content".to_string(),
                serde_json::json!("arXiv 2602.03315 academic paper critical review"),
            );
            object.insert("score".to_string(), serde_json::json!(1.0 / 61.0));
            object.insert("match_source".to_string(), serde_json::json!("hybrid"));
            Ok(vec![value])
        }

        async fn search_memory_facts_hybrid_with_identity(
            &self,
            agent_id: Option<&str>,
            query: &str,
            mode: &str,
            limit: usize,
            ward_id: Option<&str>,
            query_embedding: Option<&[f32]>,
            _query_identity: Option<&zbot_stores_traits::EmbeddingQueryIdentity>,
            as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Vec<serde_json::Value>> {
            self.search_memory_facts_hybrid(
                agent_id,
                query,
                mode,
                limit,
                ward_id,
                query_embedding,
                as_of,
            )
            .await
        }
    }

    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let mut recall = MemoryRecall::new(Some(embed), Arc::new(RecallConfig::default()));
    recall.set_memory_store(Arc::new(RrfScaleStore));

    let out = recall
        .recall_unified(
            "agent",
            "arXiv 2602.03315 academic paper critical review",
            None,
            &[],
            5,
        )
        .await
        .expect("unified recall");

    assert!(
            out.iter().any(|item| item.id == "fact-arxiv"),
            "RRF-scale Engram fact score must be normalized before default min_score filtering: {out:?}"
        );
}

#[tokio::test]
async fn recall_keeps_rrf_scaled_engram_fact_with_default_min_score() {
    struct RrfScaleStore;

    #[async_trait]
    impl zbot_stores_traits::MemoryFactStore for RrfScaleStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!({"success": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> StoreResult<serde_json::Value> {
            Ok(serde_json::json!([]))
        }

        async fn search_memory_facts_hybrid(
            &self,
            _agent_id: Option<&str>,
            _query: &str,
            _mode: &str,
            _limit: usize,
            _ward_id: Option<&str>,
            _query_embedding: Option<&[f32]>,
            _as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Vec<serde_json::Value>> {
            let fact = make_scored_fact(Some("current"), None, 1.0);
            let mut value = serde_json::to_value(fact).expect("fact json");
            let object = value.as_object_mut().expect("fact object");
            object.insert("id".to_string(), serde_json::json!("fact-arxiv"));
            object.insert(
                "key".to_string(),
                serde_json::json!("arxiv.2602.03315.paper_under_review"),
            );
            object.insert(
                "content".to_string(),
                serde_json::json!("arXiv 2602.03315 academic paper critical review"),
            );
            object.insert("score".to_string(), serde_json::json!(1.0 / 61.0));
            object.insert("match_source".to_string(), serde_json::json!("hybrid"));
            Ok(vec![value])
        }

        async fn search_memory_facts_hybrid_with_identity(
            &self,
            agent_id: Option<&str>,
            query: &str,
            mode: &str,
            limit: usize,
            ward_id: Option<&str>,
            query_embedding: Option<&[f32]>,
            _query_identity: Option<&zbot_stores_traits::EmbeddingQueryIdentity>,
            as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Vec<serde_json::Value>> {
            self.search_memory_facts_hybrid(
                agent_id,
                query,
                mode,
                limit,
                ward_id,
                query_embedding,
                as_of,
            )
            .await
        }
    }

    let embed: Arc<dyn EmbeddingClient> = Arc::new(TestEmbed);
    let mut recall = MemoryRecall::new(Some(embed), Arc::new(RecallConfig::default()));
    recall.set_memory_store(Arc::new(RrfScaleStore));

    let out = recall
        .recall_unified(
            "agent",
            "arXiv 2602.03315 academic paper critical review",
            None,
            &[],
            5,
        )
        .await
        .expect("recall_unified");

    assert!(
        out.iter().any(|item| item.id == "fact-arxiv"),
        "unified recall must admit RRF-scale Engram fact scores above min_score: {out:?}"
    );
}

#[tokio::test]
async fn unified_recall_keeps_graph_evidence_above_memory_store() {
    let item = ScoredItem {
        kind: ItemKind::GraphNode,
        id: "graph:arxiv".to_string(),
        content: "arxiv 2602.03315".to_string(),
        score: 0.91,
        provenance: Provenance {
            source: "knowledge_graph".to_string(),
            source_id: "entity:arxiv".to_string(),
            session_id: Some("sess-1".to_string()),
            ward_id: Some("academic-research".to_string()),
        },
        route_hint: None,
    };

    let atom = crate::recall::scored_item_to_context_atom(&item);
    assert_eq!(atom.kind, "graph_node");
    assert_eq!(atom.source, "knowledge_graph");
    assert!(
        atom.provenance
            .iter()
            .any(|handle| handle == "knowledge_graph:entity:arxiv"),
        "graph provenance must stay graph-labeled rather than pretending to be memory_facts"
    );
}

#[tokio::test]
async fn recall_trace_redacts_embedding_and_db_internals() {
    let item = ScoredItem {
        kind: ItemKind::Fact,
        id: "fact:safe".to_string(),
        content: "safe recall content".to_string(),
        score: 0.77,
        provenance: Provenance {
            source: "memory_facts".to_string(),
            source_id: "fact-safe".to_string(),
            session_id: Some("sess-1".to_string()),
            ward_id: Some("ward-1".to_string()),
        },
        route_hint: None,
    };
    let atom = crate::recall::scored_item_to_context_atom(&item);
    let serialized = serde_json::to_string(&atom).expect("serialize atom");
    assert!(!serialized.contains("embedding_json"));
    assert!(!serialized.contains("/home/"));
    assert!(!serialized.contains("SELECT "));

    let trace = gateway_events::GatewayEvent::RecallTrace {
        agent_id: "agent".to_string(),
        conversation_id: Some("conv".to_string()),
        seed_entity_ids: vec!["entity:arxiv".to_string()],
        seed_aggregate_ids: vec!["cluster:papers".to_string()],
        lca_aggregate_id: Some("cluster:papers".to_string()),
        surfaced_item_count: 1,
        match_sources: vec!["memory_facts".to_string()],
        ranking_reasons: vec!["reciprocal_rank_fusion".to_string()],
        degraded_reasons: vec!["embedding_identity_mismatch".to_string()],
        embedding_provider_identity: Some(serde_json::json!({
            "providerType": "fastembed",
            "model": "test",
            "dimensions": 3,
            "promptProfile": "query",
            "normalization": null
        })),
        taxonomy_expansion: Vec::new(),
    };
    let trace_json = serde_json::to_string(&trace).expect("serialize recall trace");
    assert!(trace_json.contains("match_sources"));
    assert!(trace_json.contains("ranking_reasons"));
    assert!(trace_json.contains("degraded_reasons"));
    assert!(trace_json.contains("embedding_provider_identity"));
    assert!(!trace_json.contains("embedding_json"));
    assert!(!trace_json.contains("safe recall content"));
    assert!(!trace_json.contains("/home/"));
    assert!(!trace_json.contains("SELECT "));
}
