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

        let fused = fuse_source_lists(all_lists, fusion_budget, intent_boosted);

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

fn recall_embedding_queries(text: &str) -> Vec<String> {
    let primary = bounded_recall_embedding_query(text, MAX_RECALL_EMBED_QUERY_CHARS);
    let retry = bounded_recall_embedding_query(text, RETRY_RECALL_EMBED_QUERY_CHARS);
    if primary == retry {
        vec![primary]
    } else {
        vec![primary, retry]
    }
}

fn profile_fact_key_sets_for_query(query: &str) -> Vec<&'static [&'static str]> {
    let normalized = query.to_ascii_lowercase();
    let mut key_sets = Vec::new();
    if ["my name", "who am i", "who i am", "my identity", "call me"]
        .iter()
        .any(|cue| normalized.contains(cue))
    {
        key_sets.push(IDENTITY_PROFILE_KEYS);
    }
    if [
        "weather",
        "forecast",
        "temperature",
        "air quality",
        "aqi",
        "near me",
        "nearby",
        "where am i",
        "where i live",
    ]
    .iter()
    .any(|cue| normalized.contains(cue))
    {
        key_sets.push(LOCATION_PROFILE_KEYS);
    }
    key_sets
}

fn apply_scoped_candidate_visibility(
    items: &mut Vec<ScoredItem>,
    visible: &(dyn Fn(&ScoredItem) -> bool + Send + Sync),
) {
    // Source adapters must attach an explicit, trusted scope before a
    // candidate reaches this point. Missing metadata is not a synonym for the
    // current request scope or a global record: the gateway predicate denies
    // it before telemetry, traversal, RRF, MMR, and model output.
    items.retain(|item| visible(item));
}

fn mark_durable_global_session(mut item: ScoredItem) -> ScoredItem {
    item.provenance.session_id = Some(GLOBAL_SESSION_SCOPE.to_string());
    item
}

fn mark_agent_global_scope(mut item: ScoredItem) -> ScoredItem {
    item.provenance.ward_id = Some(GLOBAL_SESSION_SCOPE.to_string());
    item.provenance.session_id = Some(GLOBAL_SESSION_SCOPE.to_string());
    item
}

fn source_status(
    configured: bool,
    requires_embedding: bool,
    embedding_available: bool,
    count: usize,
    unavailable: bool,
) -> UnifiedRecallSourceStatus {
    if !configured {
        UnifiedRecallSourceStatus::not_configured()
    } else if unavailable {
        UnifiedRecallSourceStatus::unavailable(count)
    } else if requires_embedding && !embedding_available {
        UnifiedRecallSourceStatus::embedding_unavailable()
    } else {
        UnifiedRecallSourceStatus::used_or_empty(count)
    }
}

fn taxonomy_outcome_trace(
    expansion: &zbot_stores_traits::RecallTaxonomyExpansion,
) -> UnifiedRecallTaxonomyTrace {
    UnifiedRecallTaxonomyTrace {
        retrieval_query: bounded_recall_embedding_query(
            &expansion.expanded_query,
            MAX_RECALL_EMBED_QUERY_CHARS,
        ),
        candidates: expansion
            .candidates
            .iter()
            .map(|candidate| UnifiedRecallTaxonomyCandidate {
                scheme_id: bounded_outcome_text(
                    &candidate.scheme_id,
                    MAX_OUTCOME_TAXONOMY_FIELD_CHARS,
                ),
                concept_id: bounded_outcome_text(
                    &candidate.concept_id,
                    MAX_OUTCOME_TAXONOMY_FIELD_CHARS,
                ),
                label: bounded_outcome_text(&candidate.label, MAX_OUTCOME_TAXONOMY_FIELD_CHARS),
                relation: normalized_taxonomy_relation(candidate),
            })
            .collect(),
    }
}

fn normalized_taxonomy_relation(
    candidate: &RecallTaxonomyExpansionCandidate,
) -> UnifiedRecallTaxonomyRelation {
    match candidate.relation.as_deref() {
        Some("pref_label") => UnifiedRecallTaxonomyRelation::PrefLabel,
        Some("alt_label") => UnifiedRecallTaxonomyRelation::AltLabel,
        Some("broader") => UnifiedRecallTaxonomyRelation::Broader,
        Some("narrower") => UnifiedRecallTaxonomyRelation::Narrower,
        Some("related") => UnifiedRecallTaxonomyRelation::Related,
        _ if candidate.matched_label == candidate.label => UnifiedRecallTaxonomyRelation::PrefLabel,
        _ => UnifiedRecallTaxonomyRelation::AltLabel,
    }
}

fn bounded_outcome_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn bounded_recall_embedding_query(text: &str, max_chars: usize) -> String {
    let compact = compact_recall_embedding_query(text);
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect()
}

fn compact_recall_embedding_query(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn taxonomy_trace(candidates: &[RecallTaxonomyExpansionCandidate]) -> Vec<serde_json::Value> {
    candidates
        .iter()
        .map(|candidate| {
            serde_json::json!({
                "schemeId": candidate.scheme_id,
                "conceptId": candidate.concept_id,
                "label": candidate.label,
                "matchedLabel": candidate.matched_label,
                "relation": candidate.relation,
                "depth": candidate.depth })
        })
        .collect()
}

fn is_embedding_context_length_error(error: &EmbeddingError) -> bool {
    let msg = error.to_string().to_ascii_lowercase();
    (msg.contains("input length") && msg.contains("context length"))
        || msg.contains("maximum context")
        || msg.contains("too many tokens")
        || msg.contains("context window")
}

/// Fuse the per-source candidate lists with engram's weighted Reciprocal
/// Rank Fusion (`ReciprocalRankFusion`). Replaces gateway-memory's own
/// Weighted fusion over the unified source lists: per-source weights encode source trust (goals steer
/// hardest, facts/procedures lead content, traversal/hierarchy are
/// supporting context), and engram's `FusionTrace` keeps every fused item
/// explainable. Fused scores are normalized back onto the ≈[0, 1) scale
/// the packet builders expect (multiply by `RRF_K`, then saturate).
fn fuse_source_lists(
    lists: Vec<Vec<ScoredItem>>,
    budget: usize,
    intent_boosted: bool,
) -> Vec<ScoredItem> {
    use engram_domain::{
        Actor, ActorKind, AllowedUse, DeleteMode, FusionStrategy, Id, Policy, Provenance,
        Retention, RetrievalMode, RetrievalRequest, RetrievalResult, RetrievalScore,
        RetrievalTargetType, Scope, Sensitivity, Visibility,
    };
    use engram_retrieval::{ReciprocalFusionConfig, ReciprocalRankFusion, RetrievalFusion as _};
    use std::collections::BTreeMap;

    const RRF_K: u32 = 60;
    let source_weight = |kind: &ItemKind| -> f32 {
        match kind {
            ItemKind::Goal => 1.2,
            ItemKind::Procedure => 1.1,
            ItemKind::Fact => 1.0,
            ItemKind::Wiki | ItemKind::Belief => 0.9,
            ItemKind::GraphNode | ItemKind::Episode => 0.8,
            ItemKind::HierEntity | ItemKind::HierRelation => 0.7,
        }
    };

    // Group lists by dominant kind (lists are homogeneous by construction)
    // and record per-item weights for the trace-driven weight lookup.
    let mut candidates: Vec<RetrievalResult> = Vec::new();
    let mut items_by_id: std::collections::HashMap<String, ScoredItem> =
        std::collections::HashMap::new();
    let mut source_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for list in lists {
        let Some(first) = list.first() else {
            continue;
        };
        let kind_name = match first.kind {
            ItemKind::Fact => "facts",
            ItemKind::Wiki => "wiki",
            ItemKind::Procedure => "procedures",
            ItemKind::GraphNode => "graph",
            ItemKind::Goal => "goals",
            ItemKind::Episode => "episodes",
            ItemKind::Belief => "beliefs",
            ItemKind::HierEntity | ItemKind::HierRelation => "hierarchy",
        };
        // Traversal items share ItemKind::GraphNode with ANN seeds; their
        // provenance separates them (kg_traversal vs kg_name_index).
        let kind_weight = source_weight(&first.kind);
        for (rank, item) in list.into_iter().enumerate() {
            let source = match item.provenance.source.as_str() {
                "kg_traversal" => "traversal".to_string(),
                _ => kind_name.to_string(),
            };
            let weight = if source == "traversal" {
                0.7
            } else {
                kind_weight
            };
            let id = item.id.clone();
            items_by_id.insert(id.clone(), item);
            source_names.insert(id.clone(), source.clone());
            let score = items_by_id[&id].score;
            candidates.push(RetrievalResult {
                id: format!("{source}:{id}"),
                target_type: RetrievalTargetType::Memory,
                target_id: id.clone(),
                content: String::new(),
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
                    source,
                    source_rank: Some((rank + 1) as u32),
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
            });
            let _ = weight;
        }
    }

    // Per-source weights for the fuser: lookup by trace source name.
    let source_weights: BTreeMap<String, f32> = [
        ("goals".to_string(), 1.2),
        ("procedures".to_string(), 1.1),
        ("facts".to_string(), 1.0),
        ("wiki".to_string(), 0.9),
        ("beliefs".to_string(), 0.9),
        ("graph".to_string(), 0.8),
        ("episodes".to_string(), 0.8),
        ("traversal".to_string(), 0.7),
        ("hierarchy".to_string(), 0.7),
    ]
    .into_iter()
    .collect();

    let fusion = ReciprocalRankFusion::new(
        ReciprocalFusionConfig::new(RRF_K, 1.0, source_weights)
            .unwrap_or_else(|_| ReciprocalFusionConfig::default()),
    );
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
        modes: vec![RetrievalMode::Semantic, RetrievalMode::Keyword],
        filters: None,
        cues: Vec::new(),
        budget: None,
        include_explanations: None,
    };

    let Ok(fused) = fusion.fuse(&request, candidates) else {
        return Vec::new();
    };
    let _ = intent_boosted;
    fused
        .into_iter()
        .filter_map(|result| {
            let mut item = items_by_id.remove(&result.target_id)?;
            let scaled = (result.score.total as f64) * (RRF_K as f64);
            item.score = scaled / (1.0 + scaled);
            Some(item)
        })
        .collect()
}

fn normalized_trait_fact_score(value: &serde_json::Value) -> f64 {
    let raw = value
        .get("score")
        .and_then(|score| score.as_f64())
        .unwrap_or(0.5);
    let match_source = value
        .get("match_source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if raw > 0.0 && raw < 0.3 && matches!(match_source, "hybrid" | "vec" | "fts") {
        (raw * 60.0).min(1.0)
    } else {
        raw
    }
}

#[cfg(test)]
mod ontology_retrieval_evaluation;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
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

    fn make_scored_fact(
        class: Option<&str>,
        superseded_by: Option<&str>,
        _score: f64,
    ) -> MemoryFact {
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
        async fn retract_belief(
            &self,
            _: &str,
            _: chrono::DateTime<chrono::Utc>,
        ) -> StoreResult<()> {
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
        async fn search_beliefs(
            &self,
            _: &str,
            _: &[f32],
            _: usize,
        ) -> StoreResult<Vec<ScoredBelief>> {
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
            EngramMemoryFactStore::from_provider_with_embedding_client(
                config,
                &provider,
                Some(embed),
            )
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
}
