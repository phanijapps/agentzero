//! MemoryServices — single-call factory that constructs every memory
//! subsystem component the gateway needs to wire on startup.
//!
//! Before Phase E, the gateway constructed each component imperatively
//! across ~100 lines of state/mod.rs. This factory takes all the
//! dependencies and returns a struct with `sleep_time_worker` ready to
//! store on `AppState`.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::llm::embedding::EmbeddingClient;
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_traits::{
    BeliefContradictionStore, BeliefStore, CompactionStore, EpisodeStore, MemoryFactStore,
    ProcedureStore,
};

use crate::sleep::{
    BeliefContradictionConfig, BeliefContradictionDetector, BeliefPropagator, BeliefSynthesizer,
    Compactor, ConflictResolver, ContradictionPropagationConfig, CorrectionsAbstractor,
    DecayConfig, DecayEngine, LlmBeliefSynthesizer, LlmConflictJudge, LlmContradictionJudge,
    LlmCorrectionsAbstractor, LlmPairwiseVerifier, LlmPatternExtractor, LlmSynthesizer,
    OrphanArchiver, PairwiseVerifier, PatternExtractor, Pruner, RecentBeliefNetworkActivity,
    SleepOps, SleepTimeWorker, Synthesizer,
};
use crate::{KgDecayConfig, MemoryLlmFactory};

/// Inputs needed to construct memory services.
///
/// The gateway populates this once during startup and hands it to
/// [`MemoryServices::new`]. Optional fields (e.g. `embedding_client`) are
/// passed through to the components that can take advantage of them; the
/// factory itself has no policy on when to enable a feature beyond what
/// the caller already decides.
pub struct MemoryServicesConfig {
    pub agent_id: String,
    pub interval: Duration,
    pub llm_factory: Arc<dyn MemoryLlmFactory>,
    pub kg_store: Arc<dyn KnowledgeGraphStore>,
    pub episode_store: Arc<dyn EpisodeStore>,
    pub memory_store: Arc<dyn MemoryFactStore>,
    pub compaction_store: Arc<dyn CompactionStore>,
    pub procedure_store: Arc<dyn ProcedureStore>,
    pub message_store: Arc<dyn zbot_conversation::MessageStore>,
    pub embedding_client: Option<Arc<dyn EmbeddingClient>>,
    pub kg_decay_config: KgDecayConfig,
    pub corrections_abstractor_interval: Duration,
    pub conflict_resolver_interval: Duration,
    pub decay_config: DecayConfig,
    /// MEM-001 Part A — configuration for contradiction propagation
    /// from `memory_facts.contradicted_by` to KG entities/relationships
    /// sharing the same source episodes. Defaults are safe — see
    /// [`ContradictionPropagationConfig::default`].
    pub contradiction_propagation_config: ContradictionPropagationConfig,
    /// Optional belief store. When `Some`, the Belief Network worker is
    /// constructed alongside the rest of the sleep ops; when `None` (or
    /// when `belief_network_enabled = false`), the worker is omitted.
    pub belief_store: Option<Arc<dyn BeliefStore>>,
    pub belief_network_enabled: bool,
    pub belief_network_interval: Duration,
    /// Phase B-2: optional contradiction store. When `Some` AND
    /// `belief_network_enabled = true` AND a `belief_store` is wired,
    /// the `BeliefContradictionDetector` joins the sleep cycle right
    /// after the synthesizer. Detector reuses the same enable flag and
    /// interval as B-1 so operators flip one switch.
    pub belief_contradiction_store: Option<Arc<dyn BeliefContradictionStore>>,
    /// Phase B-2: how many subject-prefix components define a
    /// neighborhood. Mirrors `BeliefNetworkConfig.neighborhood_prefix_depth`.
    pub belief_contradiction_neighborhood_prefix_depth: usize,
    /// Phase B-2: LLM call cap per detection cycle.
    pub belief_contradiction_budget_per_cycle: usize,
    /// Phase B-3: threshold below which a fact-confidence drop fires
    /// the belief propagator. Mirrors
    /// `BeliefNetworkConfig.fact_confidence_drop_threshold`.
    pub belief_fact_confidence_drop_threshold: f64,

    // ---- Phase H-3 (Hierarchical Memory) --------------------------------
    /// Master switch. When `false`, `SleepOps.hierarchy_builder` is set
    /// to `None` and the cycle is byte-for-byte unchanged.
    pub hierarchy_enabled: bool,
    /// Minimum time between hierarchy cycles. Maps from
    /// `HierarchySettings.interval_hours`.
    pub hierarchy_interval: Duration,
    /// Hard cap on layers per cycle. Maps from `HierarchySettings.max_layers`.
    pub hierarchy_max_layers: u32,
    /// Target K-means cluster size. Maps from
    /// `HierarchySettings.cluster_target_size`.
    pub hierarchy_cluster_target_size: usize,
    /// λ > τ gate for inter-cluster relation synthesis. Maps from
    /// `HierarchySettings.inter_cluster_relation_threshold`.
    pub hierarchy_inter_cluster_relation_threshold: usize,
    /// Per-cycle LLM call cap. Maps from
    /// `HierarchySettings.llm_budget_per_cycle`.
    pub hierarchy_llm_budget_per_cycle: u32,
}

/// Bundle of ready-to-use memory subsystem handles.
///
/// Currently exposes only `sleep_time_worker` (the only thing the gateway
/// stores on `AppState` from this construction path). Other handles can
/// be added here as the memory crate grows.
pub struct MemoryServices {
    pub sleep_time_worker: Arc<SleepTimeWorker>,
    /// Shared recorder of recent Belief Network worker cycles (Phase B-6).
    /// The gateway clones this Arc onto `AppState` so the Observatory HTTP
    /// handlers can read the most recent stats. Always constructed —
    /// records will simply remain empty when the network is disabled.
    pub belief_network_activity: Arc<RecentBeliefNetworkActivity>,
}

impl MemoryServices {
    /// Construct every memory component, assemble [`SleepOps`], start the
    /// worker. One call replaces ~100 lines of imperative construction in
    /// the gateway.
    pub fn new(config: MemoryServicesConfig) -> Self {
        let MemoryServicesConfig {
            agent_id,
            interval,
            llm_factory,
            kg_store,
            episode_store,
            memory_store,
            compaction_store,
            procedure_store,
            message_store,
            embedding_client,
            kg_decay_config,
            corrections_abstractor_interval,
            conflict_resolver_interval,
            decay_config,
            belief_store,
            belief_network_enabled,
            belief_network_interval,
            belief_contradiction_store,
            belief_contradiction_neighborhood_prefix_depth,
            belief_contradiction_budget_per_cycle,
            belief_fact_confidence_drop_threshold,
            hierarchy_enabled,
            hierarchy_interval,
            hierarchy_max_layers,
            hierarchy_cluster_target_size,
            hierarchy_inter_cluster_relation_threshold,
            hierarchy_llm_budget_per_cycle,
            contradiction_propagation_config,
        } = config;

        let verifier: Option<Arc<dyn PairwiseVerifier>> =
            Some(Arc::new(LlmPairwiseVerifier::new(llm_factory.clone())));
        let compactor = Arc::new(Compactor::new(
            kg_store.clone(),
            compaction_store.clone(),
            verifier,
        ));

        // B-3: optional propagator gating on the same `enabled` flag
        // as the rest of the Belief Network. Construct it once and
        // inject into both ConflictResolver and DecayEngine — the same
        // instance is shared so propagation stats merge naturally if
        // we ever want to aggregate them.
        let belief_propagator: Option<Arc<BeliefPropagator>> =
            match (belief_network_enabled, belief_store.as_ref()) {
                (true, Some(bs)) => Some(Arc::new(BeliefPropagator::new(bs.clone(), true))),
                _ => None,
            };

        let decay = Arc::new(
            DecayEngine::new(kg_store.clone(), decay_config)
                .with_belief_propagator(
                    belief_propagator.clone(),
                    belief_fact_confidence_drop_threshold,
                )
                .with_contradiction_propagation(
                    Some(memory_store.clone()),
                    contradiction_propagation_config,
                ),
        );
        let pruner = Arc::new(Pruner::new(kg_store.clone(), compaction_store.clone()));

        let synth_llm = Arc::new(LlmSynthesizer::new(llm_factory.clone()));
        let synthesizer = Arc::new(Synthesizer::new(
            kg_store.clone(),
            episode_store.clone(),
            memory_store.clone(),
            compaction_store.clone(),
            synth_llm,
            embedding_client.clone(),
        ));

        let pattern_llm = Arc::new(LlmPatternExtractor::new(llm_factory.clone()));
        let pattern_extractor = Arc::new(PatternExtractor::new(
            episode_store.clone(),
            message_store.clone(),
            procedure_store.clone(),
            compaction_store.clone(),
            pattern_llm,
            embedding_client.clone(),
            default_pattern_tool_whitelist(),
        ));

        let orphan_archiver = Arc::new(OrphanArchiver::new(
            kg_store.clone(),
            compaction_store.clone(),
        ));

        let corrections_llm = Arc::new(LlmCorrectionsAbstractor::new(llm_factory.clone()));
        let corrections_abstractor = Arc::new(CorrectionsAbstractor::new(
            memory_store.clone(),
            compaction_store.clone(),
            corrections_llm,
            corrections_abstractor_interval,
        ));

        let conflict_llm = Arc::new(LlmConflictJudge::new(llm_factory.clone()));
        let conflict_resolver = Arc::new(
            ConflictResolver::new(
                memory_store.clone(),
                compaction_store.clone(),
                conflict_llm,
                conflict_resolver_interval,
            )
            .with_belief_propagator(belief_propagator.clone()),
        );

        // Belief Network — opt-in, requires a wired BeliefStore. Even
        // when the flag is on, missing the store falls back to None so
        // mis-configuration degrades gracefully rather than panicking.
        let (belief_synthesizer, belief_contradiction_detector) =
            match (belief_network_enabled, belief_store) {
                (true, Some(bs)) => {
                    let belief_llm = Arc::new(LlmBeliefSynthesizer::new(llm_factory.clone()));
                    // B-4: thread the embedding client so synthesized
                    // beliefs carry a semantic vector for recall.
                    let synth = Some(Arc::new(
                        BeliefSynthesizer::new(
                            memory_store.clone(),
                            bs.clone(),
                            belief_llm,
                            belief_network_interval,
                            true,
                        )
                        .with_embedding_client(embedding_client.clone()),
                    ));
                    // Detector requires both stores. If the contradiction
                    // store is missing we degrade to "synthesis only" rather
                    // than panicking.
                    let detector = belief_contradiction_store.map(|cs| {
                        let judge = Arc::new(LlmContradictionJudge::new(llm_factory.clone()));
                        Arc::new(BeliefContradictionDetector::new(
                            bs,
                            cs,
                            judge,
                            BeliefContradictionConfig {
                                enabled: true,
                                neighborhood_prefix_depth:
                                    belief_contradiction_neighborhood_prefix_depth,
                                budget_per_cycle: belief_contradiction_budget_per_cycle,
                            },
                            belief_network_interval,
                        ))
                    });
                    (synth, detector)
                }
                _ => (None, None),
            };

        let belief_network_activity = Arc::new(RecentBeliefNetworkActivity::new());

        // Phase H-3: HierarchyBuilder is constructed when the operator
        // flips `execution.memory.hierarchy.enabled = true`. When off,
        // we leave it as None and the sleep cycle is byte-for-byte
        // unchanged. Same opt-in shape as the Belief Network.
        let hierarchy_builder = if hierarchy_enabled {
            let agg_llm = Arc::new(crate::sleep::llm_aggregate_entity::LlmAggregateEntity::new(
                llm_factory.clone(),
            ));
            let hierarchy_cfg = crate::sleep::hierarchy_builder::HierarchyConfig {
                cluster_target_size: hierarchy_cluster_target_size,
                max_layers: hierarchy_max_layers,
                inter_cluster_relation_threshold: hierarchy_inter_cluster_relation_threshold,
                llm_budget_per_cycle: hierarchy_llm_budget_per_cycle,
                ..crate::sleep::hierarchy_builder::HierarchyConfig::default()
            };
            Some(Arc::new(
                crate::sleep::hierarchy_builder::HierarchyBuilder::new(kg_store.clone(), agg_llm)
                    .with_embedding_client(embedding_client.clone())
                    .with_config(hierarchy_cfg)
                    .with_interval(hierarchy_interval),
            ))
        } else {
            None
        };

        let ops = SleepOps {
            synthesizer: Some(synthesizer),
            pattern_extractor: Some(pattern_extractor),
            orphan_archiver: Some(orphan_archiver),
            corrections_abstractor: Some(corrections_abstractor),
            conflict_resolver: Some(conflict_resolver),
            belief_synthesizer,
            belief_contradiction_detector,
            belief_network_activity: Some(belief_network_activity.clone()),
            hierarchy_builder,
        };

        let sleep_time_worker = Arc::new(SleepTimeWorker::start_with_ops(
            compactor,
            decay,
            pruner,
            ops,
            kg_decay_config,
            interval,
            agent_id,
        ));

        Self {
            sleep_time_worker,
            belief_network_activity,
        }
    }
}

/// Tool names the LLM pattern extractor may emit as procedure step
/// `action` fields. Sourced statically from the executor's tool
/// registry surface (`runtime/agent-tools/src/tools/*`) — kept in this
/// crate so the memory composition site doesn't take an
/// `agent-tools` dependency. When the registry becomes reachable from
/// here (Task 10), this static list will be replaced by a live
/// `tool_names()` query.
fn default_pattern_tool_whitelist() -> Vec<String> {
    [
        "shell",
        "read",
        "write",
        "edit",
        "write_file",
        "edit_file",
        "grep",
        "glob",
        "web_fetch",
        "memory",
        "graph_query",
        "execution_graph",
        "update_plan",
        "goal",
        "ward",
        "list_agents",
        "create_agent",
        "load_skill",
        "ingest",
        "multimodal_analyze",
        "query_resource",
        "request_input",
        "show_content",
        "python",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
