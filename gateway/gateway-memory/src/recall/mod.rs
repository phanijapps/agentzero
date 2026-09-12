// ============================================================================
// SMART RECALL
// Automatically retrieve relevant facts at session start
// ============================================================================

//! At the start of each session, `MemoryRecall` retrieves relevant facts
//! from the memory system and formats them for injection into the agent's
//! context. This gives the agent automatic access to prior knowledge without
//! needing to explicitly search memory.
//!
//! ## Recall Strategy
//!
//! 1. Embed the user's first message
//! 2. Run hybrid search (vector + FTS5) against memory_facts
//! 3. Also fetch all high-confidence facts (>= 0.9) — always relevant
//! 4. Merge, dedup by key, take top-K
//! 5. (Optional) Enrich with knowledge graph context
//! 6. Format as a "Recalled Memory" system message

pub mod adapters;
pub mod context_atoms;
pub mod previous_episodes;
pub mod rerank;
pub mod scored_item;
pub use context_atoms::{
    dropped_candidate_for_superseded_fact, scored_fact_to_context_atom,
    scored_item_to_context_atom, scored_items_to_context_atoms,
};
pub use scored_item::{intent_boost, GoalLite, ItemKind, Provenance, ScoredItem};

use std::sync::Arc;

use crate::{MmrConfig, RecallConfig};
use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
use zbot_stores_domain::{MemoryFact, Procedure};
use zbot_stores_traits::{
    EmbeddingQueryIdentity, RecallTaxonomyExpander, RecallTaxonomyExpansionCandidate,
    RecallTaxonomyExpansionRequest,
};

const MAX_RECALL_EMBED_QUERY_CHARS: usize = 500;
const RETRY_RECALL_EMBED_QUERY_CHARS: usize = 384;
const MAX_OUTCOME_SOURCE_COUNT: usize = 20;
const MAX_OUTCOME_TAXONOMY_FIELD_CHARS: usize = 256;
const GLOBAL_SESSION_SCOPE: &str = "__global__";
const GLOBAL_MEMORY_WARD: &str = "__global__";
const PROFILE_MEMORY_SCOPES: &[&str] = &["agent", "global"];
const IDENTITY_PROFILE_KEYS: &[&str] = &["user.name", "user.identity"];
const LOCATION_PROFILE_KEYS: &[&str] = &[
    "user.location.home_base",
    "user.location.home",
    "user.location",
];

/// Finite public state for one logical source in a unified recall outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedRecallSourceState {
    Used,
    Empty,
    NotConfigured,
    Unavailable,
    Degraded,
}

/// Stable, non-sensitive explanation for a degraded recall source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedRecallReasonCode {
    NotConfigured,
    EmbeddingUnavailable,
    SourceUnavailable,
}

/// Count and finite diagnostic for one logical recall source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedRecallSourceStatus {
    pub state: UnifiedRecallSourceState,
    pub count: usize,
    pub reason_code: Option<UnifiedRecallReasonCode>,
}

impl UnifiedRecallSourceStatus {
    fn used_or_empty(count: usize) -> Self {
        Self {
            state: if count == 0 {
                UnifiedRecallSourceState::Empty
            } else {
                UnifiedRecallSourceState::Used
            },
            count: count.min(MAX_OUTCOME_SOURCE_COUNT),
            reason_code: None,
        }
    }

    fn not_configured() -> Self {
        Self {
            state: UnifiedRecallSourceState::NotConfigured,
            count: 0,
            reason_code: Some(UnifiedRecallReasonCode::NotConfigured),
        }
    }

    fn embedding_unavailable() -> Self {
        Self {
            state: UnifiedRecallSourceState::Degraded,
            count: 0,
            reason_code: Some(UnifiedRecallReasonCode::EmbeddingUnavailable),
        }
    }

    fn unavailable(count: usize) -> Self {
        Self {
            state: if count == 0 {
                UnifiedRecallSourceState::Unavailable
            } else {
                UnifiedRecallSourceState::Degraded
            },
            count: count.min(MAX_OUTCOME_SOURCE_COUNT),
            reason_code: Some(UnifiedRecallReasonCode::SourceUnavailable),
        }
    }
}

/// Fixed source-status map for a unified recall invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedRecallSourceSummary {
    pub facts: UnifiedRecallSourceStatus,
    pub graph: UnifiedRecallSourceStatus,
    pub wiki: UnifiedRecallSourceStatus,
    pub procedures: UnifiedRecallSourceStatus,
    pub episodes: UnifiedRecallSourceStatus,
    pub beliefs: UnifiedRecallSourceStatus,
    pub hierarchy: UnifiedRecallSourceStatus,
    pub goals: UnifiedRecallSourceStatus,
    pub taxonomy: UnifiedRecallSourceStatus,
}

/// Finite relation label exposed for a taxonomy recall cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedRecallTaxonomyRelation {
    PrefLabel,
    AltLabel,
    Broader,
    Narrower,
    Related,
}

/// Normalized, bounded taxonomy cue used to expand one unified-recall query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedRecallTaxonomyCandidate {
    pub scheme_id: String,
    pub concept_id: String,
    pub label: String,
    pub relation: UnifiedRecallTaxonomyRelation,
}

/// Bounded taxonomy cues used to expand one unified-recall query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedRecallTaxonomyTrace {
    pub retrieval_query: String,
    pub candidates: Vec<UnifiedRecallTaxonomyCandidate>,
}

/// Gateway-only result model for one unified-recall invocation.
#[derive(Debug, Clone)]
pub struct UnifiedRecallOutcome {
    pub items: Vec<ScoredItem>,
    pub source_summary: UnifiedRecallSourceSummary,
    pub taxonomy_expansion: Option<UnifiedRecallTaxonomyTrace>,
}

/// Trusted scope controls for a gateway-authorized unified-recall invocation.
///
/// Model input must never construct this value. The gateway derives the
/// visibility predicate from authenticated execution state and only enables
/// taxonomy expansion after it has proved the corresponding configuration
/// scope.
pub struct UnifiedRecallScope<'a> {
    candidate_visible: &'a (dyn Fn(&ScoredItem) -> bool + Send + Sync),
    taxonomy_scope_proven: bool,
    taxonomy_session_id: Option<&'a str>,
}

/// Immutable identity of the semantic provider wired into one `MemoryRecall`
/// instance.
///
/// The gateway composition root creates this from the provider configuration
/// that opened the stores. Executor code may only resolve the configured ward
/// workspace; it may not infer provider ownership from a model request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallProviderScope {
    pub tenant_id: String,
    workspace_from_ward: bool,
    taxonomy_scope_proven: bool,
}

impl RecallProviderScope {
    #[must_use]
    pub fn new(tenant_id: String, workspace_from_ward: bool, taxonomy_scope_proven: bool) -> Self {
        Self {
            tenant_id,
            workspace_from_ward,
            taxonomy_scope_proven,
        }
    }

    #[must_use]
    pub fn workspace_for_ward(&self, ward_id: Option<&str>) -> Option<String> {
        self.workspace_from_ward
            .then(|| ward_id.map(str::to_owned))
            .flatten()
    }

    #[must_use]
    pub const fn taxonomy_scope_proven(&self) -> bool {
        self.taxonomy_scope_proven
    }
}

impl<'a> UnifiedRecallScope<'a> {
    pub fn new(
        candidate_visible: &'a (dyn Fn(&ScoredItem) -> bool + Send + Sync),
        taxonomy_scope_proven: bool,
    ) -> Self {
        Self {
            candidate_visible,
            taxonomy_scope_proven,
            taxonomy_session_id: None,
        }
    }

    /// Attach the trusted session identity used for governance selection.
    #[must_use]
    pub fn with_taxonomy_session_id(mut self, session_id: Option<&'a str>) -> Self {
        self.taxonomy_session_id = session_id;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallSkosExpansionLimits {
    pub max_depth: u8,
    pub max_fan_out: u16,
    pub max_candidates: u16,
}

impl Default for RecallSkosExpansionLimits {
    fn default() -> Self {
        Self {
            max_depth: 1,
            max_fan_out: 8,
            max_candidates: 16,
        }
    }
}

/// Retrieves relevant memory facts for injection at session start.
///
/// Phase E6c: fully trait-routed. Every store dependency is an
/// `Arc<dyn ...>`; the composition root (`gateway/src/state/mod.rs`)
/// picks the concrete adapter (SQLite today, any future backend
/// tomorrow) and wires it via setters. No SQLite types appear in
/// this struct's signatures.
pub struct MemoryRecall {
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
    memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>>,
    kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>>,
    episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>>,
    wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>>,
    procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    /// Belief store for Phase B-4 recall integration. Wired only when
    /// the Belief Network is enabled — when `None`, `recall_unified`
    /// stays byte-for-byte identical to pre-B-4 behavior (no extra
    /// fetch, no extra log).
    belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,
    /// Self-RAG retrieval gate. When `None`, recall behaves identically to
    /// pre-gate behavior (raw user message → hybrid search).
    /// MMR diversity reranking. When `None` or `enabled = false`,
    /// `recall_unified` is byte-for-byte identical to pre-MMR behavior.
    mmr_config: Option<MmrConfig>,
    /// Cross-encoder rerank stage. `None` (or `enabled = false`) keeps
    /// the fused order — recall is byte-for-byte identical without it.
    rerank: Option<Arc<rerank::RerankStage>>,
    taxonomy_expander: Option<Arc<dyn RecallTaxonomyExpander>>,
    taxonomy_limits: RecallSkosExpansionLimits,
    provider_scope: Option<RecallProviderScope>,
    /// Observatory v2 (Phase 3) — optional EventBus for emitting
    /// `RecallTrace` telemetry. When `None` (tests, headless invocations),
    /// recall stays silent. Production wiring attaches this in
    /// `MemoryServices::new()`.
    event_bus: Option<Arc<gateway_events::EventBus>>,
    config: Arc<RecallConfig>,
}

#[derive(Debug, Default)]
struct UnifiedRecallSourceFailures {
    facts: bool,
    graph: bool,
    wiki: bool,
    procedures: bool,
    episodes: bool,
    beliefs: bool,
    hierarchy: bool,
}

impl MemoryRecall {
    /// Create a new memory recall service. All store dependencies are
    /// wired via setters; the embedding client is optional (recall
    /// degrades to FTS-only when absent).
    pub fn new(
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
        config: Arc<RecallConfig>,
    ) -> Self {
        Self {
            embedding_client,
            memory_store: None,
            kg_store: None,
            episode_store: None,
            wiki_store: None,
            procedure_store: None,
            belief_store: None,
            mmr_config: None,
            rerank: None,
            taxonomy_expander: None,
            taxonomy_limits: RecallSkosExpansionLimits::default(),
            provider_scope: None,
            event_bus: None,
            config,
        }
    }

    /// Wire the EventBus so `recall_unified` can emit `RecallTrace`
    /// telemetry for the Observatory v2 canvas. When unset, recall is
    /// byte-for-byte identical to its pre-Phase-3 behavior.
    pub fn set_event_bus(&mut self, bus: Arc<gateway_events::EventBus>) {
        self.event_bus = Some(bus);
    }

    /// Wire the belief store so `recall_unified` surfaces beliefs
    /// alongside facts (Phase B-4). Caller is expected to attach this
    /// only when `beliefNetwork.enabled = true`; when unset, beliefs
    /// stay out of the recall pool entirely.
    pub fn set_belief_store(&mut self, store: Arc<dyn zbot_stores_traits::BeliefStore>) {
        self.belief_store = Some(store);
    }

    /// Wire MMR diversity reranking. When set with `enabled = true`,
    /// `recall_unified` over-fetches `candidate_pool` items from RRF, then
    /// reranks via MMR before truncating to the caller's budget. When
    /// unset (or `enabled = false`), recall is byte-for-byte identical to
    /// pre-MMR behavior.
    pub fn set_mmr_config(&mut self, cfg: MmrConfig) {
        self.mmr_config = Some(cfg);
    }

    /// Wire the cross-encoder rerank stage. When set and enabled, the
    /// top `pool` fused candidates are query-scored and reordered before
    /// MMR/truncation; failures keep fused order (fail-open).
    pub fn set_rerank_stage(&mut self, stage: rerank::RerankStage) {
        self.rerank = Some(Arc::new(stage));
    }

    pub fn set_taxonomy_expander(&mut self, expander: Arc<dyn RecallTaxonomyExpander>) {
        self.taxonomy_expander = Some(expander);
    }

    pub fn set_taxonomy_expansion_limits(&mut self, limits: RecallSkosExpansionLimits) {
        self.taxonomy_limits = limits;
    }

    /// Attach the immutable provider identity selected by the composition
    /// root. A model-visible recall invocation fails closed when this is not
    /// present.
    pub fn set_provider_scope(&mut self, scope: RecallProviderScope) {
        self.provider_scope = Some(scope);
    }

    #[must_use]
    pub fn provider_scope(&self) -> Option<RecallProviderScope> {
        self.provider_scope.clone()
    }

    /// Access the recall configuration.
    pub fn config(&self) -> &RecallConfig {
        &self.config
    }

    /// Wire the memory-fact store (hybrid FTS + vector recall path).
    pub fn set_memory_store(&mut self, store: Arc<dyn zbot_stores_traits::MemoryFactStore>) {
        self.memory_store = Some(store);
    }

    /// Wire the KG store (graph ANN recall path).
    pub fn set_kg_store(&mut self, store: Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>) {
        self.kg_store = Some(store);
    }

    /// Wire the episode store (previous-episode chain recall path).
    pub fn set_episode_store(&mut self, store: Arc<dyn zbot_stores_traits::EpisodeStore>) {
        self.episode_store = Some(store);
    }

    /// Wire the wiki store (ward-scoped wiki recall path).
    pub fn set_wiki_store(&mut self, store: Arc<dyn zbot_stores_traits::WikiStore>) {
        self.wiki_store = Some(store);
    }

    /// Wire the procedure store (procedure recall path).
    pub fn set_procedure_store(&mut self, store: Arc<dyn zbot_stores_traits::ProcedureStore>) {
        self.procedure_store = Some(store);
    }

    /// Search for proven procedures similar to a query.
    ///
    /// Returns matching procedures with their similarity scores, filtered to
    /// the given agent and optional ward scope.
    pub async fn recall_procedures(
        &self,
        query: &str,
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(Procedure, f64)>, String> {
        let embedding = match self.embed_query(query).await {
            Some(emb) => emb,
            None => return Ok(Vec::new()),
        };
        let query_identity = self.embedding_query_identity();

        let store = match self.procedure_store.as_ref() {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        store
            .search_procedures_by_similarity_typed_with_identity(
                &embedding,
                query_identity.as_ref(),
                agent_id,
                ward_id,
                limit,
            )
            .await
            .map_err(|e| e.to_string())
    }

    /// Recall relevant facts for a given agent and user message.
    ///
    /// Returns scored facts sorted by relevance (highest first), with
    /// category weights and optional ward affinity boost applied.
    /// Unified scored-pool recall: query every configured source (facts, wiki,
    /// procedures, graph ANN, active goals), adapt each into [`ScoredItem`],
    /// apply [`intent_boost`] against `active_goals`, then fuse via
    /// Reciprocal Rank Fusion capped to `budget`.
    ///
    /// Missing subsystems (no embedding client, no wiki repo, etc.) are
    /// silently skipped — the caller gets whatever sources are wired.
    pub async fn recall_unified(
        &self,
        agent_id: &str,
        query: &str,
        ward_id: Option<&str>,
        active_goals: &[GoalLite],
        budget: usize,
    ) -> Result<Vec<ScoredItem>, String> {
        self.recall_unified_outcome(agent_id, query, ward_id, active_goals, budget)
            .await
            .map(|outcome| outcome.items)
    }

    /// Same retrieval pipeline as [`Self::recall_unified`], with bounded source
    /// diagnostics and taxonomy-expansion evidence for model-facing adapters.
    ///
    /// Detailed backend failures remain private: this method emits only the
    /// finite [`UnifiedRecallReasonCode`] values in its source summary.
    pub async fn recall_unified_outcome(
        &self,
        agent_id: &str,
        query: &str,
        ward_id: Option<&str>,
        active_goals: &[GoalLite],
        budget: usize,
    ) -> Result<UnifiedRecallOutcome, String> {
        self.recall_unified_outcome_with_visibility(
            agent_id,
            query,
            ward_id,
            active_goals,
            budget,
            None,
        )
        .await
    }

    /// Scoped variant of unified recall for gateway-authorized callers.
    ///
    /// Candidate visibility is applied after every configured source has
    /// projected explicit provenance but before intent boost, RRF, and MMR.
    /// Callers must pass a fail-closed predicate derived from trusted execution
    /// state; the compatibility method above intentionally remains unscoped.
    pub async fn recall_unified_outcome_scoped(
        &self,
        agent_id: &str,
        query: &str,
        ward_id: Option<&str>,
        active_goals: &[GoalLite],
        budget: usize,
        scope: UnifiedRecallScope<'_>,
    ) -> Result<UnifiedRecallOutcome, String> {
        self.recall_unified_outcome_with_visibility(
            agent_id,
            query,
            ward_id,
            active_goals,
            budget,
            Some(scope),
        )
        .await
    }

    async fn recall_unified_outcome_with_visibility(
        &self,
        agent_id: &str,
        query: &str,
        ward_id: Option<&str>,
        active_goals: &[GoalLite],
        budget: usize,
        scope: Option<UnifiedRecallScope<'_>>,
    ) -> Result<UnifiedRecallOutcome, String> {
        let taxonomy_scope_proven = scope
            .as_ref()
            .is_none_or(|scope| scope.taxonomy_scope_proven);
        let taxonomy_session_id = scope.as_ref().and_then(|scope| scope.taxonomy_session_id);
        let taxonomy_configured = taxonomy_scope_proven
            && self
                .taxonomy_expander
                .as_ref()
                .is_some_and(|expander| expander.is_configured_for(ward_id, taxonomy_session_id))
            && self.taxonomy_limits.max_candidates > 0;
        let taxonomy_result = if taxonomy_scope_proven {
            self.expand_query_with_taxonomy(query, ward_id, taxonomy_session_id)
                .await
        } else {
            Ok(None)
        };
        let (taxonomy_expansion, taxonomy_status) = match taxonomy_result {
            Ok(Some(expansion)) => {
                let status = UnifiedRecallSourceStatus::used_or_empty(expansion.candidates.len());
                (Some(expansion), status)
            }
            Ok(None) if taxonomy_configured => (None, UnifiedRecallSourceStatus::used_or_empty(0)),
            Ok(None) if taxonomy_scope_proven => {
                (None, UnifiedRecallSourceStatus::not_configured())
            }
            Ok(None) => (None, UnifiedRecallSourceStatus::unavailable(0)),
            Err(()) => (None, UnifiedRecallSourceStatus::unavailable(0)),
        };
        let retrieval_query = taxonomy_expansion
            .as_ref()
            .map(|expansion| expansion.expanded_query.as_str())
            .unwrap_or(query);
        let taxonomy_candidates = taxonomy_expansion
            .as_ref()
            .map(|expansion| expansion.candidates.clone())
            .unwrap_or_default();
        let taxonomy_outcome_trace = taxonomy_expansion.as_ref().map(taxonomy_outcome_trace);
        let query_emb = self.embed_query(retrieval_query).await;
        let query_identity = query_emb
            .as_ref()
            .and_then(|_| self.embedding_query_identity());
        let mut source_failures = UnifiedRecallSourceFailures::default();
        let mut profile_items = self.profile_context_items(agent_id, query, budget).await;

        // 1. Facts via hybrid search. Phase E8: prefer the trait
        // `memory_store` (wired by AppState), fall back to the
        // SQLite repo. On Surreal, scores aren't yet preserved by the
        // trait surface — we synthesize 0.5 so facts still rank into
        // the fused pool but don't dominate it.
        let (mut fact_items, facts_unavailable): (Vec<ScoredItem>, bool) = if let (
            Some(store),
            Some(query_emb),
        ) =
            (self.memory_store.as_ref(), query_emb.as_ref())
        {
            match store
                .search_memory_facts_hybrid_with_identity(
                    Some(agent_id),
                    retrieval_query,
                    "hybrid",
                    10,
                    ward_id,
                    Some(query_emb.as_slice()),
                    query_identity.as_ref(),
                    None, // as_of — default "now" recall
                )
                .await
            {
                Ok(values) => (
                    values
                        .into_iter()
                        .filter_map(|v| {
                            let score = normalized_trait_fact_score(&v);
                            serde_json::from_value::<MemoryFact>(v)
                                .ok()
                                .filter(|fact| fact.superseded_by.is_none())
                                .map(|fact| {
                                    let durable_scope = fact.scope != "session";
                                    let mut item = adapters::fact_to_item(&fact, score);
                                    if durable_scope {
                                        // The query already bound this fact to
                                        // the authenticated agent and ward.
                                        // Its source session is provenance, not
                                        // an access boundary for durable facts.
                                        item.provenance.session_id =
                                            Some(GLOBAL_SESSION_SCOPE.to_string());
                                    }
                                    item
                                })
                        })
                        .filter(|item| item.score >= self.config.min_score)
                        .collect(),
                    false,
                ),
                Err(_) => (Vec::new(), true),
            }
        } else {
            (Vec::new(), false)
        };
        source_failures.facts = facts_unavailable;

        // 2. Wiki articles (ward-scoped).
        let (mut wiki_items, wiki_unavailable): (Vec<ScoredItem>, bool) =
            match (self.wiki_store.as_ref(), query_emb.as_ref(), ward_id) {
                (Some(store), Some(emb), Some(wid)) => match store
                    .search_wiki_by_similarity_typed_with_identity(
                        wid,
                        emb,
                        query_identity.as_ref(),
                        5,
                    )
                    .await
                {
                    Ok(values) => (
                        values
                            .into_iter()
                            .filter(|(article, _score)| article.agent_id == agent_id)
                            .map(|(a, s)| adapters::wiki_to_item(&a, s))
                            .map(mark_durable_global_session)
                            .collect(),
                        false,
                    ),
                    Err(_) => (Vec::new(), true),
                },
                _ => (Vec::new(), false),
            };
        source_failures.wiki = wiki_unavailable;

        // 3. Procedures.
        let (mut procedure_items, procedures_unavailable): (Vec<ScoredItem>, bool) =
            match (self.procedure_store.as_ref(), query_emb.as_ref()) {
                (Some(store), Some(emb)) => match store
                    .search_procedures_by_similarity_typed_with_identity(
                        emb,
                        query_identity.as_ref(),
                        agent_id,
                        ward_id,
                        5,
                    )
                    .await
                {
                    Ok(values) => (
                        values
                            .into_iter()
                            .map(|(p, s)| adapters::procedure_to_item(&p, s))
                            .map(mark_durable_global_session)
                            .collect(),
                        false,
                    ),
                    Err(_) => (Vec::new(), true),
                },
                _ => (Vec::new(), false),
            };
        source_failures.procedures = procedures_unavailable;

        // 4. Graph ANN over the entity name embedding index. We share
        // the raw hit set with step 5c (hierarchy LCA) so both surfaces
        // use the same seed entities without a second ANN query.
        //
        // MEM-001 Part B-1: hits below `min_kg_confidence` are dropped
        // before scoring (low-confidence noise that would clutter
        // recall — including entities decayed by Part A contradiction
        // propagation). The score of surviving hits is multiplied by
        // entity confidence so a 0.9-confidence hit at cosine 0.8 wins
        // over a 0.4-confidence hit at the same cosine. Filter applies
        // to seed_ids too, so the hierarchy LCA surface (step 5c) is
        // consistent with what recall actually shows.
        let min_kg_conf = self.config.graph_traversal.min_kg_confidence;
        let mut graph_unavailable = false;
        let (mut graph_items, mut graph_seed_ids): (
            Vec<ScoredItem>,
            Vec<knowledge_graph::kg_trait::kg_types::EntityId>,
        ) = match (self.kg_store.as_ref(), query_emb.as_ref()) {
            (Some(store), Some(emb)) => match store
                .search_entities_by_name_embedding_with_identity(
                    agent_id,
                    emb,
                    query_identity.as_ref(),
                    10,
                )
                .await
            {
                Ok(raw_hits) => {
                    let hits: Vec<_> = raw_hits
                        .into_iter()
                        .filter(|h| h.confidence >= min_kg_conf)
                        .collect();
                    let seed_ids: Vec<knowledge_graph::kg_trait::kg_types::EntityId> = hits
                        .iter()
                        .filter(|h| !h.id.is_empty())
                        .map(|h| knowledge_graph::kg_trait::kg_types::EntityId(h.id.clone()))
                        .collect();
                    let items: Vec<ScoredItem> = hits
                        .into_iter()
                        .enumerate()
                        .map(|(idx, hit)| {
                            // Mirror graph_ann_to_items scoring exactly,
                            // plus the B-1 entity-confidence multiplier.
                            let cosine = 1.0 - (hit.distance as f64) / 2.0;
                            let rank_one = (idx as f64) + 1.0;
                            let score = (1.0 / rank_one) * cosine * hit.confidence;
                            ScoredItem {
                                kind: ItemKind::GraphNode,
                                id: format!("graph:{}", hit.name),
                                content: format!(
                                    "Entity: {} [{}] (cosine ~ {cosine:.2}, conf ~ {:.2})",
                                    hit.name, hit.entity_type, hit.confidence
                                ),
                                score,
                                provenance: Provenance {
                                    source: "kg_name_index".to_string(),
                                    source_id: hit.id,
                                    // The graph query is authenticated by
                                    // agent at the store seam. Graph data is
                                    // deliberately agent-wide, so mark both
                                    // dimensions as explicit global scope.
                                    session_id: Some(GLOBAL_SESSION_SCOPE.to_string()),
                                    ward_id: Some(GLOBAL_SESSION_SCOPE.to_string()),
                                },
                                route_hint: None,
                            }
                        })
                        .collect();
                    (items, seed_ids)
                }
                Err(_) => {
                    graph_unavailable = true;
                    (Vec::new(), Vec::new())
                }
            },
            _ => (Vec::new(), Vec::new()),
        };

        // Scope graph hits before they can become traversal/LCA seeds or
        // telemetry. The graph projection above classifies its agent-bound
        // records as explicit global scope; the gateway still has to grant the
        // matching source-specific capability before they can proceed.
        if let Some(scope) = scope.as_ref() {
            apply_scoped_candidate_visibility(&mut graph_items, scope.candidate_visible);
            graph_seed_ids = graph_items
                .iter()
                .filter(|item| item.kind == ItemKind::GraphNode)
                .map(|item| {
                    knowledge_graph::kg_trait::kg_types::EntityId(item.provenance.source_id.clone())
                })
                .collect();
        }

        // 4b. Confidence-weighted graph traversal from top seeds
        // (MEM-001 Part B-2). For each top-3 graph-ANN seed, walk
        // `kg_relationships` outward up to `max_hops`, skipping edges
        // below `min_kg_confidence`. Score each hit as
        //   hop_decay^hop * edge_confidence_product * entity_confidence
        // Dedupe by entity_id (best score wins), exclude entities
        // already surfaced as seeds, cap at `max_graph_facts`. Disabled
        // when `graph_traversal.enabled = false` or `max_hops = 0`.
        let traversal_max_hops = self.config.graph_traversal.max_hops as usize;
        let traversal_hop_decay = self.config.graph_traversal.hop_decay;
        let traversal_cap = self.config.graph_traversal.max_graph_facts;
        let traversal_enabled =
            self.config.graph_traversal.enabled && traversal_max_hops > 0 && traversal_cap > 0;

        let mut traversal_items: Vec<ScoredItem> = if traversal_enabled {
            match self.kg_store.as_ref() {
                Some(store) if !graph_seed_ids.is_empty() => {
                    let already_surfaced: std::collections::HashSet<String> =
                        graph_seed_ids.iter().map(|e| e.0.clone()).collect();
                    // Bound traversal cost: walk from the top-3 seeds
                    // only. Beyond that the marginal value drops fast.
                    let mut best_by_id: std::collections::HashMap<String, ScoredItem> =
                        std::collections::HashMap::new();
                    for seed in graph_seed_ids.iter().take(3) {
                        let hits = match store
                            .traverse_weighted(
                                seed,
                                agent_id,
                                traversal_max_hops,
                                min_kg_conf,
                                traversal_cap.saturating_mul(2),
                            )
                            .await
                        {
                            Ok(hits) => hits,
                            Err(_) => {
                                graph_unavailable = true;
                                Vec::new()
                            }
                        };
                        for h in hits {
                            if already_surfaced.contains(&h.entity_id.0) {
                                continue;
                            }
                            let score = traversal_hop_decay.powi(h.hop as i32)
                                * h.edge_confidence_product
                                * h.entity_confidence;
                            let item = mark_agent_global_scope(ScoredItem {
                                kind: ItemKind::GraphNode,
                                id: format!("graph:{}", h.name),
                                content: format!(
                                    "Entity: {} [{}] (hop {}, edge ~ {:.2}, conf ~ {:.2})",
                                    h.name,
                                    h.entity_type,
                                    h.hop,
                                    h.edge_confidence_product,
                                    h.entity_confidence
                                ),
                                score,
                                provenance: Provenance {
                                    source: "kg_traversal".to_string(),
                                    source_id: h.name,
                                    session_id: Some(GLOBAL_SESSION_SCOPE.to_string()),
                                    ward_id: Some(GLOBAL_SESSION_SCOPE.to_string()),
                                },
                                route_hint: None,
                            });
                            best_by_id
                                .entry(h.entity_id.0)
                                .and_modify(|prev| {
                                    if item.score > prev.score {
                                        *prev = item.clone();
                                    }
                                })
                                .or_insert(item);
                        }
                    }
                    let mut out: Vec<ScoredItem> = best_by_id.into_values().collect();
                    out.sort_by(|a, b| {
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    out.truncate(traversal_cap);
                    out
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        source_failures.graph = graph_unavailable;

        // 5a. Previous episodes in this ward (chain continuity).
        let (mut episode_items, episodes_unavailable): (Vec<ScoredItem>, bool) =
            match (self.episode_store.as_ref(), ward_id) {
                (Some(store), Some(wid)) => {
                    match previous_episodes::PreviousEpisodesAdapter::new(store.clone())
                        .fetch(agent_id, wid)
                        .await
                    {
                        Ok(items) => (items, false),
                        Err(_) => (Vec::new(), true),
                    }
                }
                _ => (Vec::new(), false),
            };
        source_failures.episodes = episodes_unavailable;

        // 5. Active goals as retrievable items.
        let mut goal_items: Vec<ScoredItem> = active_goals
            .iter()
            .map(|g| ScoredItem {
                kind: ItemKind::Goal,
                id: g.id.clone(),
                content: format!("Active goal: {}", g.title),
                score: 1.0,
                provenance: Provenance {
                    source: "kg_goals".to_string(),
                    source_id: g.id.clone(),
                    // GoalAccess has already bound this record to the
                    // executing agent and the gateway checked ward ownership
                    // before converting it to GoalLite.
                    session_id: Some(GLOBAL_SESSION_SCOPE.to_string()),
                    ward_id: ward_id.map(String::from),
                },
                route_hint: ward_id.map(|ward| {
                    zbot_stores_domain::RouteHint::new(
                        ward.to_string(),
                        zbot_stores_domain::RouteSourceKind::Goal,
                    )
                    .with_memory_id(g.id.clone())
                }),
            })
            .collect();

        // 5b. Beliefs (Phase B-4). Only when both a belief_store is
        // wired AND a query embedding was produced — beliefs are
        // semantic-only (no FTS fallback). When the Belief Network is
        // disabled or the embedder is unavailable, this list stays
        // empty and `recall_unified` behaves byte-for-byte identically
        // to pre-B-4. The partition_id used here is `agent_id`,
        // mirroring how the synthesizer writes beliefs (one belief
        // partition per agent).
        let (mut belief_items, beliefs_unavailable): (Vec<ScoredItem>, bool) =
            match (self.belief_store.as_ref(), query_emb.as_ref()) {
                (Some(store), Some(emb)) => match store
                    .search_beliefs_with_identity(agent_id, emb, query_identity.as_ref(), 10)
                    .await
                {
                    Ok(values) => (
                        values
                            .into_iter()
                            .map(|sb| {
                                let weight = self.config.category_weight("belief");
                                let mut item = adapters::belief_to_item(&sb.belief, sb.score);
                                item.score *= weight;
                                item
                            })
                            .map(mark_agent_global_scope)
                            .collect(),
                        false,
                    ),
                    Err(_) => (Vec::new(), true),
                },
                _ => (Vec::new(), false),
            };
        source_failures.beliefs = beliefs_unavailable;

        // 5c. Hierarchical-memory LCA path (Phase H-4 / LeanRAG).
        // Opportunistic: when the hierarchy hasn't been built yet,
        // `compute_lca_path` returns an empty result and this list
        // stays empty — no extra cost beyond the parent-pointer walk.
        // Seeds reuse the graph-ANN top-K from step 4 so we don't
        // double-query the kg_name_index. Category weight uses the
        // `pattern` slot (0.9) per `project_hierarchical_memory_plan.md`
        // — conservative until empirical validation justifies a bump.
        //
        // 5d (follow-up): once we have the LCA path entities, also fetch
        // any inter-cluster relations whose both endpoints sit on the
        // path. That's the "lean" part of LeanRAG — the edges between
        // sibling abstractions that explain how they relate.
        let mut hierarchy_unavailable = false;
        let (mut hier_items, mut hier_relation_items): (Vec<ScoredItem>, Vec<ScoredItem>) =
            match (self.kg_store.as_ref(), graph_seed_ids.is_empty()) {
                (Some(store), false) => {
                    match store.compute_lca_path(agent_id, &graph_seed_ids).await {
                        Ok(lca) => {
                            let weight = self.config.category_weight("pattern");
                            let entity_items: Vec<ScoredItem> = lca
                                .path_entities
                                .iter()
                                .enumerate()
                                .map(|(idx, id)| {
                                    // Higher-layer entities (closer to the LCA)
                                    // rank slightly higher in the per-source
                                    // order; downstream RRF takes over for
                                    // cross-source fusion.
                                    let raw = 1.0 / (1.0 + idx as f64);
                                    let mut item =
                                        adapters::hier_entity_to_item(id, lca.max_layer, raw);
                                    item.score *= weight;
                                    mark_agent_global_scope(item)
                                })
                                .collect();

                            // Fetch the inter-cluster edges between path
                            // entities. Empty path ⇒ skip the query (the trait
                            // method short-circuits on empty input anyway, but
                            // skipping at the call site saves an async hop).
                            let relation_items: Vec<ScoredItem> = if lca.path_entities.is_empty() {
                                Vec::new()
                            } else {
                                match store
                                    .list_inter_cluster_relations(agent_id, &lca.path_entities)
                                    .await
                                {
                                    Ok(edges) => edges
                                        .into_iter()
                                        .enumerate()
                                        .map(|(idx, hit)| {
                                            let raw = 1.0 / (1.0 + idx as f64);
                                            let mut item = adapters::hier_relation_to_item(
                                                &hit.id,
                                                &hit.source_entity_id,
                                                &hit.target_entity_id,
                                                &hit.relationship_type,
                                                hit.layer,
                                                raw,
                                            );
                                            item.score *= weight;
                                            mark_agent_global_scope(item)
                                        })
                                        .collect(),
                                    Err(_) => {
                                        hierarchy_unavailable = true;
                                        Vec::new()
                                    }
                                }
                            };
                            (entity_items, relation_items)
                        }
                        Err(_) => {
                            hierarchy_unavailable = true;
                            (Vec::new(), Vec::new())
                        }
                    }
                }
                _ => (Vec::new(), Vec::new()),
            };
        source_failures.hierarchy = hierarchy_unavailable;

        if let Some(scope) = scope.as_ref() {
            let visible = scope.candidate_visible;
            for items in [
                &mut profile_items,
                &mut fact_items,
                &mut wiki_items,
                &mut procedure_items,
                &mut graph_items,
                &mut traversal_items,
                &mut episode_items,
                &mut goal_items,
                &mut belief_items,
                &mut hier_items,
                &mut hier_relation_items,
            ] {
                apply_scoped_candidate_visibility(items, visible);
            }
        }

        // Intent-boost precondition (telemetry + fusion share it): boost
        // fires only when a goal actually names an unfilled slot.
        let intent_boosted = !active_goals.is_empty()
            && active_goals
                .iter()
                .any(|goal| !goal.unfilled_slot_names.is_empty());

        // Observatory v2 Phase 3 — broadcast a RecallTrace telemetry
        // event so the dashboard can light up the consulted clusters in
        // real time. Best-effort: bus may not be wired (tests, headless
        // bootstrap), and `publish_sync` is non-blocking.
        if let Some(bus) = self.event_bus.as_ref() {
            let seed_entity_ids: Vec<String> = graph_seed_ids.iter().map(|e| e.0.clone()).collect();
            // The LCA path's `path_entities` is the union of every
            // seed's ancestry chain up to (and including) the LCA.
            // For the visualisation we treat them all as
            // "aggregates the agent touched" — close enough to give
            // the live walk a target without an extra trait round-trip.
            let (seed_aggregate_ids, lca_aggregate_id): (Vec<String>, Option<String>) =
                match (self.kg_store.as_ref(), graph_seed_ids.is_empty()) {
                    (Some(store), false) => {
                        match store.compute_lca_path(agent_id, &graph_seed_ids).await {
                            Ok(lca) => (
                                lca.path_entities.iter().map(|e| e.0.clone()).collect(),
                                lca.lca.map(|e| e.0),
                            ),
                            Err(_) => (Vec::new(), None),
                        }
                    }
                    _ => (Vec::new(), None),
                };
            let surfaced_item_count = (profile_items.len()
                + fact_items.len()
                + wiki_items.len()
                + procedure_items.len()
                + graph_items.len()
                + traversal_items.len()
                + belief_items.len()
                + hier_items.len()
                + hier_relation_items.len()) as u32;
            let mut match_sources = Vec::new();
            for (source, count) in [
                ("memory_facts", profile_items.len() + fact_items.len()),
                ("wiki", wiki_items.len()),
                ("procedures", procedure_items.len()),
                ("graph", graph_items.len() + traversal_items.len()),
                ("beliefs", belief_items.len()),
                ("hierarchy", hier_items.len() + hier_relation_items.len()),
                ("taxonomy", taxonomy_candidates.len()),
            ] {
                if count > 0 {
                    match_sources.push(source.to_string());
                }
            }
            // Honest reasons: report only what actually runs. Weighted
            // RRF always fuses; intent boost only when goals carry
            // unfilled slots AND the boost re-sorted its lists; MMR only
            // when enabled.
            let mut ranking_reasons =
                vec!["source_relevance".to_string(), "weighted_rrf".to_string()];
            if intent_boosted {
                ranking_reasons.push("intent_boost".to_string());
            }
            if self.mmr_config.as_ref().is_some_and(|cfg| cfg.enabled) {
                ranking_reasons.push("mmr_diversity".to_string());
            }
            if self
                .rerank
                .as_ref()
                .is_some_and(|stage| stage.config.enabled)
            {
                ranking_reasons.push("cross_encoder_rerank".to_string());
            }
            if !taxonomy_candidates.is_empty() {
                ranking_reasons.push("skos_taxonomy_expansion".to_string());
            }
            let degraded_reasons = if query_emb.is_none() {
                vec!["query_embedding_unavailable".to_string()]
            } else {
                Vec::new()
            };
            let embedding_provider_identity = query_identity.as_ref().map(|identity| {
                serde_json::json!({
                    "providerType": identity.provider_type.clone(),
                    "model": identity.model.clone(),
                    "dimensions": identity.dimensions,
                    "promptProfile": identity.prompt_profile.clone(),
                    "normalization": identity.normalization.clone() })
            });
            bus.publish_sync(gateway_events::GatewayEvent::RecallTrace {
                agent_id: agent_id.to_string(),
                conversation_id: None,
                seed_entity_ids,
                seed_aggregate_ids,
                lca_aggregate_id,
                surfaced_item_count,
                match_sources,
                ranking_reasons,
                degraded_reasons,
                embedding_provider_identity,
                taxonomy_expansion: taxonomy_trace(&taxonomy_candidates),
            });
        }

        // Intent boost on non-goal lists.
        let mut all_lists = vec![
            fact_items,
            wiki_items,
            procedure_items,
            graph_items,
            traversal_items,
            episode_items,
            belief_items,
            hier_items,
            hier_relation_items,
        ];
        for list in &mut all_lists {
            intent_boost(list, active_goals);
            // The boost changes scores, and the fuser consumes per-list
            // ORDER — re-sort so an intent-boosted item actually rises.
            list.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        all_lists.push(goal_items);

        // Decide fusion budget: when MMR is enabled, over-fetch from RRF so
        // MMR has a wider candidate pool to diversify over. When disabled,
        // pass `budget` straight through.
        let generic_budget = budget.saturating_sub(profile_items.len());
        let (fusion_budget, run_mmr) = match self.mmr_config.as_ref() {
            Some(cfg) if cfg.enabled => (cfg.candidate_pool.max(generic_budget), true),
            _ => (generic_budget, false),
        };

        let mut fused = fuse_source_lists(all_lists, fusion_budget, intent_boosted);

        // Cross-encoder precision stage: query-score the top pool and
        // reorder, fail-open to fused order on any failure/timeout.
        if let Some(stage) = self.rerank.as_ref() {
            if stage.will_run(fused.len()) {
                fused = stage.apply(retrieval_query, fused).await;
            }
        }

        let generic_items = if run_mmr {
            let lambda = self.mmr_config.as_ref().map(|c| c.lambda).unwrap_or(0.6);
            self.mmr_rerank(fused, lambda, generic_budget).await
        } else {
            fused
        };
        let mut items = profile_items;
        for item in generic_items {
            if items.iter().all(|existing| existing.id != item.id) {
                items.push(item);
            }
            if items.len() >= budget {
                break;
            }
        }
        // Access reinforcement (ACT-R): facts that made the final packet
        // get mention_count + last_accessed bumps so their recency decay
        // slows. Fire-and-forget — a failed touch must not fail recall.
        if let Some(store) = self.memory_store.as_ref() {
            let touched: Vec<String> = items
                .iter()
                .filter(|item| item.kind == ItemKind::Fact)
                .map(|item| item.id.clone())
                .collect();
            if !touched.is_empty() {
                if let Err(error) = store.touch_facts(&touched).await {
                    tracing::debug!(count = touched.len(), %error, "fact touch skipped");
                }
            }
        }

        let source_summary = self.unified_source_summary(
            &items,
            query_emb.is_some(),
            taxonomy_status,
            &source_failures,
        );
        Ok(UnifiedRecallOutcome {
            items,
            source_summary,
            taxonomy_expansion: taxonomy_outcome_trace,
        })
    }

    /// Return the durable profile facts that a request explicitly needs.
    ///
    /// These lookups deliberately use canonical fact keys instead of a broad
    /// semantic query: identity and local context are correctness-sensitive,
    /// and a generic vector query can be crowded out before unified reranking.
    /// The resulting items still pass the caller's scoped-visibility filter.
    async fn profile_context_items(
        &self,
        agent_id: &str,
        query: &str,
        budget: usize,
    ) -> Vec<ScoredItem> {
        if budget == 0 {
            return Vec::new();
        }

        let Some(store) = self.memory_store.as_ref() else {
            return Vec::new();
        };

        let mut items = Vec::new();
        for keys in profile_fact_key_sets_for_query(query) {
            let mut fact = None;
            for scope in PROFILE_MEMORY_SCOPES {
                for key in keys {
                    if let Ok(Some(candidate)) = store
                        .get_fact_by_key(agent_id, scope, GLOBAL_MEMORY_WARD, key)
                        .await
                    {
                        if candidate.superseded_by.is_none() {
                            fact = Some(candidate);
                            break;
                        }
                    }
                }
                if fact.is_some() {
                    break;
                }
            }
            if let Some(fact) = fact {
                let mut item = adapters::fact_to_item(&fact, 1.0);
                item.provenance.session_id = Some(GLOBAL_SESSION_SCOPE.to_string());
                items.push(item);
            }
            if items.len() >= budget {
                break;
            }
        }
        items
    }

    fn unified_source_summary(
        &self,
        items: &[ScoredItem],
        embedding_available: bool,
        taxonomy: UnifiedRecallSourceStatus,
        failures: &UnifiedRecallSourceFailures,
    ) -> UnifiedRecallSourceSummary {
        let count = |kind: ItemKind| items.iter().filter(|item| item.kind == kind).count();
        UnifiedRecallSourceSummary {
            facts: source_status(
                self.memory_store.is_some(),
                true,
                embedding_available,
                count(ItemKind::Fact),
                failures.facts,
            ),
            graph: source_status(
                self.kg_store.is_some(),
                true,
                embedding_available,
                count(ItemKind::GraphNode),
                failures.graph,
            ),
            wiki: source_status(
                self.wiki_store.is_some(),
                true,
                embedding_available,
                count(ItemKind::Wiki),
                failures.wiki,
            ),
            procedures: source_status(
                self.procedure_store.is_some(),
                true,
                embedding_available,
                count(ItemKind::Procedure),
                failures.procedures,
            ),
            episodes: source_status(
                self.episode_store.is_some(),
                false,
                embedding_available,
                count(ItemKind::Episode),
                failures.episodes,
            ),
            beliefs: source_status(
                self.belief_store.is_some(),
                true,
                embedding_available,
                count(ItemKind::Belief),
                failures.beliefs,
            ),
            hierarchy: source_status(
                self.kg_store.is_some(),
                true,
                embedding_available,
                count(ItemKind::HierEntity) + count(ItemKind::HierRelation),
                failures.hierarchy,
            ),
            goals: UnifiedRecallSourceStatus::used_or_empty(count(ItemKind::Goal)),
            taxonomy,
        }
    }

    async fn expand_query_with_taxonomy(
        &self,
        query: &str,
        ward_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Option<zbot_stores_traits::RecallTaxonomyExpansion>, ()> {
        let Some(expander) = self.taxonomy_expander.as_ref() else {
            return Ok(None);
        };
        let limits = self.taxonomy_limits;
        if limits.max_candidates == 0 {
            return Ok(None);
        }
        expander
            .expand_recall_query(RecallTaxonomyExpansionRequest {
                query: query.to_string(),
                ward_id: ward_id.map(ToOwned::to_owned),
                session_id: session_id.map(ToOwned::to_owned),
                max_depth: limits.max_depth,
                max_fan_out: limits.max_fan_out,
                max_candidates: limits.max_candidates,
            })
            .await
            .map_err(|_| ())
            .map(|mut expansion| {
                expansion
                    .candidates
                    .truncate(usize::from(limits.max_candidates));
                expansion.expanded_query = bounded_recall_embedding_query(
                    &expansion.expanded_query,
                    MAX_RECALL_EMBED_QUERY_CHARS,
                );
                (!expansion.candidates.is_empty()).then_some(expansion)
            })
    }

    /// Emit typed context atoms from the existing unified recall path.
    ///
    /// This does not change ranking or prompt rendering; it gives later packet
    /// assembly a structured read model over the same scored recall output.
    pub async fn recall_context_atoms(
        &self,
        agent_id: &str,
        query: &str,
        ward_id: Option<&str>,
        active_goals: &[GoalLite],
        budget: usize,
    ) -> Result<Vec<agent_runtime::ContextAtom>, String> {
        let items = self
            .recall_unified(agent_id, query, ward_id, active_goals, budget)
            .await?;
        Ok(context_atoms::scored_items_to_context_atoms(&items))
    }

    /// Resolve a per-item embedding for MMR. Returns `None` when the
    /// candidate's kind has no embedding source or the lookup fails.
    ///
    /// - `Fact` → `memory_store.get_fact_embedding(id)` (single SQLite hop).
    /// - `Belief` / `Wiki` / `Procedure` / `GraphNode` → embed the rendered
    ///   `content` via the embedding client. The belief's stored embedding
    ///   bytes are not carried through `ScoredItem`, and re-embedding the
    ///   rendered content tracks the same semantic neighborhood closely
    ///   enough for diversity scoring at this scale.
    /// - `Goal` / `Episode` → `None` (diversity penalty contributes 0).
    async fn fetch_item_embedding(&self, item: &ScoredItem) -> Option<Vec<f32>> {
        match item.kind {
            ItemKind::Fact => {
                let store = self.memory_store.as_ref()?;
                store.get_fact_embedding(&item.id).await.ok().flatten()
            }
            ItemKind::Belief
            | ItemKind::Wiki
            | ItemKind::Procedure
            | ItemKind::GraphNode
            | ItemKind::HierEntity
            | ItemKind::HierRelation => {
                let client = self.embedding_client.as_ref()?;
                match client.embed(&[item.content.as_str()]).await {
                    Ok(mut v) if !v.is_empty() => Some(v.remove(0)),
                    _ => None,
                }
            }
            ItemKind::Goal | ItemKind::Episode => None,
        }
    }

    /// Apply MMR reranking to `candidates`, returning at most `budget`
    /// items in selection order. Embeddings are fetched per item via
    /// [`Self::fetch_item_embedding`]; candidates without embeddings keep
    /// their relevance score but contribute zero diversity penalty.
    async fn mmr_rerank(
        &self,
        candidates: Vec<ScoredItem>,
        lambda: f64,
        budget: usize,
    ) -> Vec<ScoredItem> {
        if candidates.is_empty() || budget == 0 {
            return Vec::new();
        }

        // Diversity rerank via engram's MmrReranker. Embeddings prefer the
        // STORED item vectors (fact-id keyed, exact same vector that matched
        // the query); the content embedder is only the fallback. The bridge
        // embedder serves those precomputed vectors by content key.
        use engram_domain::{
            Actor, ActorKind, AllowedUse, DeleteMode, FusionStrategy, Id, Policy, Provenance,
            Retention, RetrievalMode, RetrievalRequest, RetrievalResult, RetrievalScore,
            RetrievalTargetType, Scope, Sensitivity, Visibility,
        };
        use engram_rerank_mmr::{MmrEmbedder, MmrReranker};
        use engram_retrieval::RetrievalReranker as _;
        use std::collections::HashMap;

        struct StoredItemEmbedder {
            by_content: HashMap<String, Vec<f32>>,
        }
        impl MmrEmbedder for StoredItemEmbedder {
            fn embed(&self, text: &str) -> engram_runtime::CoreResult<Vec<f32>> {
                self.by_content.get(text).cloned().ok_or_else(|| {
                    engram_runtime::CoreError::InvalidRequest {
                        reason: "content not pre-embedded".to_owned(),
                    }
                })
            }
        }

        let mut by_content: HashMap<String, Vec<f32>> = HashMap::new();
        for item in &candidates {
            if let Some(embedding) = self.fetch_item_embedding(item).await {
                by_content.insert(item.content.clone(), embedding);
            }
        }

        let mut items_by_id: HashMap<String, ScoredItem> = candidates
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        let rerank_candidates: Vec<RetrievalResult> = items_by_id
            .values()
            .map(|item| {
                let id = item.id.clone();
                let score = item.score;
                RetrievalResult {
                    id: format!("mmr:{id}"),
                    target_type: RetrievalTargetType::Memory,
                    target_id: id,
                    content: item.content.clone(),
                    score: RetrievalScore {
                        total: score as f32,
                        relevance: Some(score as f32),
                        recency: None,
                        confidence: None,
                        cue_match: None,
                        hierarchical_fit: None,
                        policy_fit: None,
                    },
                    provenance: Provenance {
                        source: "zbot_unified_recall".to_string(),
                        actor: Actor {
                            id: Id::from("gateway-memory"),
                            kind: ActorKind::System,
                            display_name: None,
                            metadata: None,
                        },
                        observed_at: chrono::Utc::now(),
                        evidence: Vec::new(),
                        derivations: Vec::new(),
                        confidence: None,
                        method: None,
                    },
                    policy: Policy {
                        visibility: Visibility::Workspace,
                        retention: Retention::Durable,
                        sensitivity: Some(Sensitivity::Low),
                        allowed_uses: vec![AllowedUse::Retrieval],
                        expires_at: None,
                        delete_mode: Some(DeleteMode::Tombstone),
                    },
                    explanation: None,
                    fusion_trace: Some(engram_domain::FusionTrace {
                        query_id: None,
                        vector_index: None,
                        embedding_time_ms: None,
                        search_time_ms: None,
                        source: "unified_pool".to_string(),
                        source_rank: None,
                        source_score: Some(score as f32),
                        score: None,
                        rank: None,
                        fusion_strategy: Some(FusionStrategy::None),
                        fusion_score: None,
                        rerank_strategy: None,
                        rerank_score: None,
                        discard_reason: None,
                        deduplicated_with: Vec::new(),
                    }),
                    metadata: None,
                }
            })
            .collect();

        let embedder = if by_content.is_empty() {
            None
        } else {
            Some(Arc::new(StoredItemEmbedder { by_content }) as Arc<dyn MmrEmbedder>)
        };
        let reranker = MmrReranker::new(embedder, lambda as f32);
        let request = RetrievalRequest {
            limit: Some(budget as u32),
            query: String::new(),
            scope: Scope {
                tenant: "zbot".to_string(),
                subject: None,
                workspace: None,
                session: None,
                environment: None,
            },
            requester: engram_domain::Requester {
                actor: Actor {
                    id: Id::from("gateway-memory"),
                    kind: ActorKind::System,
                    display_name: None,
                    metadata: None,
                },
                roles: Vec::new(),
                permissions: Vec::new(),
                on_behalf_of: None,
            },
            modes: vec![RetrievalMode::Semantic],
            filters: None,
            cues: Vec::new(),
            budget: None,
            include_explanations: None,
        };
        let reranked = reranker
            .rerank(&request, rerank_candidates)
            .unwrap_or_default();
        reranked
            .into_iter()
            .filter_map(|result| items_by_id.remove(&result.target_id))
            .take(budget)
            .collect()
    }

    /// Embed a query string for vector search.
    async fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        let client = self.embedding_client.as_ref()?;
        let attempts = recall_embedding_queries(text);
        if attempts
            .first()
            .is_some_and(|embedding_text| embedding_text.chars().count() < text.chars().count())
        {
            tracing::warn!(
                recall_embed_input_too_long = true,
                original_chars = text.chars().count(),
                bounded_chars = attempts[0].chars().count(),
                max_chars = MAX_RECALL_EMBED_QUERY_CHARS,
                "Recall embedding input exceeded hygiene cap; embedding bounded query"
            );
        }

        for (attempt_idx, embedding_text) in attempts.iter().enumerate() {
            match client.embed(&[embedding_text.as_str()]).await {
                Ok(mut embeddings) if !embeddings.is_empty() => return Some(embeddings.remove(0)),
                Ok(_) => return None,
                Err(e)
                    if attempt_idx == 0
                        && attempts.len() > 1
                        && is_embedding_context_length_error(&e) =>
                {
                    tracing::warn!(
                        recall_embed_retry_context_too_long = true,
                        error = %e,
                        retry_chars = attempts[1].chars().count(),
                        "Recall embedding provider rejected bounded query; retrying smaller query"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        recall_embed_failed = true,
                        error = %e,
                        "Failed to embed query for recall; skipping fuzzy hybrid recall"
                    );
                    return None;
                }
            }
        }

        None
    }

    fn embedding_query_identity(&self) -> Option<EmbeddingQueryIdentity> {
        let client = self.embedding_client.as_ref()?;
        Some(EmbeddingQueryIdentity {
            provider_type: client.provider_type(),
            model: client.model_name(),
            dimensions: client.dimensions() as u32,
            prompt_profile: client.prompt_profile(),
            normalization: client.normalization(),
        })
    }
}

mod support;
use support::*;

#[cfg(test)]
mod ontology_retrieval_evaluation;

#[cfg(test)]
mod tests;
