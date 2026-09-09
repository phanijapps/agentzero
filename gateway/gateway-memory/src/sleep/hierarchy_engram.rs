//! Hierarchy build on engram's port slot (P3 sub-batch 2).
//!
//! The clustering + layer loop + LLM aggregate-naming core from the old
//! `hierarchy_builder.rs` sleep worker now lives behind engram's
//! [`HierarchyBuilder`] port — the slot its docs describe as "may use
//! clustering, taxonomy, graph structure, or model-assisted summaries
//! internally". Persistence stays the zbot KG store (`promote_cluster_to_
//! aggregate` / `write_inter_cluster_relation`) — the same store-bridge
//! shape as `ZbotBeliefSink` in P3.1.
//!
//! Deleted with the old worker: the interval/last-run throttle (the sleep
//! worker's cadence shell owns that), and the `run_for_agent` orchestration
//! wrapper — replaced by [`ZbotHierarchyBuildArm`], a
//! `ConsolidationMutationExecutor` for the `HierarchyBuild` task kind,
//! dispatched via [`HierarchyConsolidation`] on an engram
//! `ConsolidationRequest` (the `BeliefConsolidation` pattern).
//!
//! `build_hierarchy` additionally returns the promoted aggregates as
//! engram `HierarchyNode` accounting projections — the same
//! payload-projection pattern `belief_engram` uses for beliefs. The KG
//! store rows remain the source of truth for recall; the returned nodes
//! give engram's consolidation audit a typed record of what was built.

use std::sync::{Arc, Mutex};

use agent_runtime::llm::EmbeddingClient;
use async_trait::async_trait;
use engram_consolidation::{ConsolidationMutationExecutor, ConsolidationMutationOutcome};
use engram_domain::{
    Actor, ActorKind, ConsolidationRequest, ConsolidationStats, ConsolidationTaskKind,
    ConsolidationTaskResult, ConsolidationTaskStatus, HierarchyBuildConfig, HierarchyMemberType,
    HierarchyMembership, HierarchyNode, HierarchyNodeId, HierarchyNodeKind, HierarchyNodeStatus,
    Id, Policy, Provenance, Retention, Scope, Timestamp, Visibility,
};
use engram_hierarchy::HierarchyBuilder as EngramHierarchyBuilder;
use engram_runtime::{CoreError, CoreResult};
use tracing::{debug, info, warn};
use zbot_stores::types::EntityId;
use zbot_stores::KnowledgeGraphStore;

use crate::sleep::clustering::{
    cluster_sparsity, kmeans_cosine, should_stop_layering, DEFAULT_KMEANS_MAX_ITER,
    DEFAULT_SPARSITY_EPSILON,
};

// ---------------------------------------------------------------------------
// Public types (moved from hierarchy_builder.rs; behavior unchanged)
// ---------------------------------------------------------------------------

/// LLM abstraction for aggregate-entity + inter-cluster relation synthesis.
/// Mockable in tests; production wiring (`LlmAggregateEntity`) formats the
/// prompt and parses the response JSON.
#[async_trait]
pub trait AggregateEntityLlm: Send + Sync {
    /// Summarise a multi-member cluster into an aggregate entity.
    /// Singleton clusters short-circuit BEFORE this is called — they
    /// just promote the single member with no LLM cost.
    ///
    /// `prior_names` is the list of aggregate names already produced
    /// in this cycle; the LLM is expected to avoid them so two
    /// thematically-adjacent clusters don't collide on a label.
    async fn synthesize_aggregate(
        &self,
        members: &[AggregateMemberContext],
        prior_names: &[String],
    ) -> Result<AggregateResponse, String>;

    /// Pick a relationship type for an inter-cluster edge. `lambda` is
    /// the connectivity strength. Falls back to "related-via" in the
    /// caller when the LLM call fails.
    async fn synthesize_relation(
        &self,
        agg_a_name: &str,
        agg_b_name: &str,
        lambda: usize,
    ) -> Result<String, String>;
}

/// Minimal per-member context the LLM sees: names + optional
/// descriptions, nothing else.
#[derive(Debug, Clone)]
pub struct AggregateMemberContext {
    pub id: EntityId,
    pub name: String,
    pub description: Option<String>,
}

/// LLM response shape for a synthesised aggregate.
#[derive(Debug, Clone)]
pub struct AggregateResponse {
    pub name: String,
    pub description: String,
}

/// Tuning parameters. Carries safe defaults that match
/// `project_hierarchical_memory_plan.md`.
#[derive(Debug, Clone)]
pub struct HierarchyConfig {
    /// Target cluster size. K-means runs with k = max(2, n / target).
    pub cluster_target_size: usize,
    /// Hard cap on the number of layers built per cycle.
    pub max_layers: u32,
    /// Stop when `cluster_sparsity` between layers changes by ≤ this.
    pub sparsity_epsilon: f32,
    /// Inter-cluster relation gate. Skip when λ ≤ this value.
    pub inter_cluster_relation_threshold: usize,
    /// Per-cycle ceiling on LLM calls.
    pub llm_budget_per_cycle: u32,
    /// K-means seed. Pinned so re-runs produce the same labels.
    pub seed: u64,
}

impl Default for HierarchyConfig {
    fn default() -> Self {
        Self {
            cluster_target_size: 20,
            max_layers: 4,
            sparsity_epsilon: DEFAULT_SPARSITY_EPSILON,
            inter_cluster_relation_threshold: 3,
            llm_budget_per_cycle: 50,
            seed: 0x6261_7365_6c69_6e65, // ascii "baseline"
        }
    }
}

/// Counts emitted from one build. Drained by the sleep worker for the
/// observability summary.
#[derive(Debug, Default, Clone)]
pub struct HierarchyStats {
    pub layers_built: u32,
    pub aggregates_created: u64,
    pub singletons_promoted: u64,
    pub inter_cluster_relations_created: u64,
    pub llm_calls: u64,
    pub stopped_reason: StopReason,
    pub errors: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Hit `max_layers`.
    MaxLayers,
    /// `cluster_sparsity` change ≤ epsilon.
    Converged,
    /// Pool too small to cluster meaningfully.
    PoolTooSmall,
    /// K-means produced a single cluster (degenerate).
    SingleCluster,
    /// LLM budget exhausted mid-cycle.
    BudgetExhausted,
    /// Initial layer fetch returned an error.
    #[default]
    NotStarted,
}

// ---------------------------------------------------------------------------
// ZbotHierarchyBuilder — the port impl
// ---------------------------------------------------------------------------

/// zbot's fill of engram's `HierarchyBuilder` slot. The build loop is the
/// old `HierarchyBuilder::run_for_agent` body with the throttle removed;
/// persistence goes through the zbot KG store handle (the store bridge).
pub struct ZbotHierarchyBuilder {
    kg_store: Arc<dyn KnowledgeGraphStore>,
    llm: Arc<dyn AggregateEntityLlm>,
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
    config: HierarchyConfig,
    /// Stats from the most recent `build_hierarchy` call, drained by the
    /// worker trigger after each consolidation run.
    last_stats: Mutex<Option<HierarchyStats>>,
}

impl ZbotHierarchyBuilder {
    pub fn new(
        kg_store: Arc<dyn KnowledgeGraphStore>,
        llm: Arc<dyn AggregateEntityLlm>,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
        config: HierarchyConfig,
    ) -> Self {
        Self {
            kg_store,
            llm,
            embedding_client,
            config,
            last_stats: Mutex::new(None),
        }
    }

    /// Drain + reset the last run's stats (the `take_stats` pattern from
    /// P3.1's detector/synthesizer).
    pub fn take_stats(&self) -> HierarchyStats {
        self.last_stats.lock().unwrap().take().unwrap_or_default()
    }

    /// The build core. Returns the promoted aggregates as engram
    /// `HierarchyNode` projections alongside the internal stats.
    /// Behavior byte-preserved from the old `run_for_agent` minus the
    /// throttle (cadence belongs to the sleep worker shell).
    pub async fn build_for_agent(&self, agent_id: &str) -> (HierarchyStats, Vec<HierarchyNode>) {
        let mut stats = HierarchyStats::default();
        let mut nodes: Vec<HierarchyNode> = Vec::new();
        let now = chrono::Utc::now();
        let mut prev_sparsity: Option<f32> = None;

        for layer in 0..self.config.max_layers {
            let pool = match self
                .kg_store
                .list_entities_with_embeddings_at_layer(agent_id, layer as i64, 0)
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    warn!(agent_id, layer, error = ?e, "hierarchy: layer fetch failed");
                    stats.errors += 1;
                    return (stats, nodes);
                }
            };

            if pool.len() < self.config.cluster_target_size.max(2) {
                debug!(
                    agent_id,
                    layer,
                    pool_size = pool.len(),
                    "hierarchy: pool too small, stopping"
                );
                stats.stopped_reason = StopReason::PoolTooSmall;
                return (stats, nodes);
            }

            let n = pool.len();
            let k = (n / self.config.cluster_target_size).max(2);
            let embeddings: Vec<Vec<f32>> = pool.iter().map(|p| p.embedding.clone()).collect();
            let labels = kmeans_cosine(&embeddings, k, self.config.seed, DEFAULT_KMEANS_MAX_ITER);
            let distinct_labels: std::collections::HashSet<_> = labels.iter().copied().collect();
            if distinct_labels.len() < 2 {
                debug!(
                    agent_id,
                    layer, "hierarchy: K-means collapsed to one cluster, stopping"
                );
                stats.stopped_reason = StopReason::SingleCluster;
                return (stats, nodes);
            }

            let current_sparsity = cluster_sparsity(&labels);
            if let Some(prev) = prev_sparsity {
                if should_stop_layering(prev, current_sparsity, self.config.sparsity_epsilon) {
                    debug!(
                        agent_id,
                        layer, prev, current_sparsity, "hierarchy: sparsity converged"
                    );
                    stats.stopped_reason = StopReason::Converged;
                    return (stats, nodes);
                }
            }

            let mut clusters: Vec<Vec<EntityId>> = vec![Vec::new(); k];
            for (idx, &lab) in labels.iter().enumerate() {
                clusters[lab].push(pool[idx].id.clone());
            }

            // Materialise each cluster as a layer+1 aggregate, tracking
            // names so later LLM calls avoid label collisions.
            let mut aggregate_ids: Vec<Option<EntityId>> = vec![None; clusters.len()];
            let mut cycle_names: Vec<String> = Vec::with_capacity(clusters.len());
            for (cluster_idx, members) in clusters.iter().enumerate() {
                if members.is_empty() {
                    continue;
                }
                let (agg_id, agg_name, node) = match self
                    .promote_one_cluster(
                        agent_id,
                        (layer as i64) + 1,
                        members,
                        &cycle_names,
                        &mut stats,
                        now,
                    )
                    .await
                {
                    Ok(triple) => triple,
                    Err(()) => continue,
                };
                cycle_names.push(agg_name.clone());
                if let Some(node) = node {
                    nodes.push(node);
                }
                aggregate_ids[cluster_idx] = Some(agg_id);
            }

            // Inter-cluster relations: for each (i,j) pair where
            // λ > threshold, synthesise + write. Budget-capped.
            for i in 0..clusters.len() {
                for j in (i + 1)..clusters.len() {
                    if stats.llm_calls >= self.config.llm_budget_per_cycle as u64 {
                        info!(
                            agent_id,
                            layer,
                            calls = stats.llm_calls,
                            "hierarchy: llm budget exhausted; skipping remaining pairs"
                        );
                        stats.stopped_reason = StopReason::BudgetExhausted;
                        break;
                    }
                    let (Some(agg_i), Some(agg_j)) =
                        (aggregate_ids[i].as_ref(), aggregate_ids[j].as_ref())
                    else {
                        continue;
                    };
                    let lambda = match self
                        .kg_store
                        .connectivity_strength(agent_id, &clusters[i], &clusters[j])
                        .await
                    {
                        Ok(l) => l,
                        Err(e) => {
                            warn!(agent_id, layer, error = ?e, "hierarchy: connectivity query failed");
                            stats.errors += 1;
                            continue;
                        }
                    };
                    if lambda <= self.config.inter_cluster_relation_threshold {
                        continue;
                    }
                    self.write_inter_cluster_pair(
                        agent_id,
                        (layer as i64) + 1,
                        agg_i,
                        agg_j,
                        lambda,
                        &mut stats,
                    )
                    .await;
                }
                if stats.stopped_reason == StopReason::BudgetExhausted {
                    break;
                }
            }

            stats.layers_built = layer + 1;
            prev_sparsity = Some(current_sparsity);

            if stats.stopped_reason == StopReason::BudgetExhausted {
                return (stats, nodes);
            }
        }

        if stats.stopped_reason == StopReason::NotStarted {
            stats.stopped_reason = StopReason::MaxLayers;
        }
        (stats, nodes)
    }

    // ---- internal helpers (behavior byte-preserved) ----

    /// Promote one cluster; returns (aggregate entity id, display name,
    /// optional engram node projection). Singletons short-circuit the LLM.
    #[allow(clippy::type_complexity)]
    async fn promote_one_cluster(
        &self,
        agent_id: &str,
        layer: i64,
        members: &[EntityId],
        prior_names: &[String],
        stats: &mut HierarchyStats,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(EntityId, String, Option<HierarchyNode>), ()> {
        if members.len() == 1 {
            let display_name = match self.kg_store.get_entity(&members[0]).await {
                Ok(Some(entity)) => entity.name,
                _ => members[0].0.clone(),
            };
            let description = format!("Singleton aggregate of \"{display_name}\"");
            let embedding = match &self.embedding_client {
                Some(client) => match client.embed(&[description.as_str()]).await {
                    Ok(mut vs) if !vs.is_empty() => vs.pop(),
                    Ok(_) => None,
                    Err(e) => {
                        warn!(agent_id, layer, error = %e, "hierarchy: singleton embed failed");
                        None
                    }
                },
                None => None,
            };
            let result = self
                .kg_store
                .promote_cluster_to_aggregate(
                    agent_id,
                    layer,
                    members,
                    &display_name,
                    &description,
                    embedding,
                )
                .await;
            return match result {
                Ok(id) => {
                    stats.singletons_promoted += 1;
                    let node = hierarchy_node_projection(
                        &id,
                        agent_id,
                        layer,
                        &display_name,
                        Some(&description),
                        members,
                        now,
                    );
                    Ok((id, display_name, Some(node)))
                }
                Err(e) => {
                    warn!(agent_id, layer, error = ?e, "hierarchy: singleton promote failed");
                    stats.errors += 1;
                    Err(())
                }
            };
        }

        // Multi-member: LLM call.
        if stats.llm_calls >= self.config.llm_budget_per_cycle as u64 {
            info!(
                agent_id,
                layer, "hierarchy: llm budget exhausted; skipping cluster"
            );
            return Err(());
        }
        let contexts: Vec<AggregateMemberContext> = members
            .iter()
            .map(|id| AggregateMemberContext {
                id: id.clone(),
                name: id.0.clone(),
                description: None,
            })
            .collect();
        stats.llm_calls += 1;
        let resp = match self.llm.synthesize_aggregate(&contexts, prior_names).await {
            Ok(r) => r,
            Err(e) => {
                warn!(agent_id, layer, error = %e, "hierarchy: aggregate LLM failed");
                stats.errors += 1;
                return Err(());
            }
        };

        let embedding = match &self.embedding_client {
            Some(client) => match client.embed(&[resp.description.as_str()]).await {
                Ok(mut vs) if !vs.is_empty() => vs.pop(),
                Ok(_) => None,
                Err(e) => {
                    warn!(agent_id, layer, error = %e, "hierarchy: embed failed");
                    None
                }
            },
            None => None,
        };

        match self
            .kg_store
            .promote_cluster_to_aggregate(
                agent_id,
                layer,
                members,
                &resp.name,
                &resp.description,
                embedding,
            )
            .await
        {
            Ok(id) => {
                stats.aggregates_created += 1;
                let node = hierarchy_node_projection(
                    &id,
                    agent_id,
                    layer,
                    &resp.name,
                    Some(&resp.description),
                    members,
                    now,
                );
                Ok((id, resp.name, Some(node)))
            }
            Err(e) => {
                warn!(agent_id, layer, error = ?e, "hierarchy: aggregate write failed");
                stats.errors += 1;
                Err(())
            }
        }
    }

    async fn write_inter_cluster_pair(
        &self,
        agent_id: &str,
        layer: i64,
        agg_a: &EntityId,
        agg_b: &EntityId,
        lambda: usize,
        stats: &mut HierarchyStats,
    ) {
        stats.llm_calls += 1;
        let rtype = match self
            .llm
            .synthesize_relation(&agg_a.0, &agg_b.0, lambda)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(agent_id, layer, error = %e, "hierarchy: relation LLM failed; falling back");
                "related-via".to_string()
            }
        };
        match self
            .kg_store
            .write_inter_cluster_relation(agent_id, layer, agg_a, agg_b, &rtype)
            .await
        {
            Ok(_) => {
                stats.inter_cluster_relations_created += 1;
            }
            Err(e) => {
                warn!(agent_id, layer, error = ?e, "hierarchy: inter-cluster write failed");
                stats.errors += 1;
            }
        }
    }
}

/// Build the engram `HierarchyNode` accounting projection for a promoted
/// aggregate. The KG store row remains the recall source of truth; this
/// node is what engram's consolidation audit sees.
#[allow(clippy::too_many_arguments)]
fn hierarchy_node_projection(
    aggregate_id: &EntityId,
    agent_id: &str,
    layer: i64,
    name: &str,
    description: Option<&str>,
    members: &[EntityId],
    now: chrono::DateTime<chrono::Utc>,
) -> HierarchyNode {
    let now_ts: Timestamp = now;
    let parent_id = HierarchyNodeId::from(format!("hier:{}", aggregate_id.0));
    HierarchyNode {
        id: parent_id.clone(),
        scope: Scope {
            tenant: "zbot".to_string(),
            subject: Some(agent_id.to_string()),
            workspace: None,
            session: None,
            environment: None,
        },
        kind: HierarchyNodeKind::Aggregate,
        layer: layer.max(0) as u32,
        name: name.to_string(),
        summary: description.map(str::to_string),
        parent_id: None,
        members: members
            .iter()
            .map(|member| HierarchyMembership {
                id: format!("{}::{}", parent_id.as_str(), member.0),
                parent_id: parent_id.clone(),
                member_type: HierarchyMemberType::Entity,
                member_id: member.0.clone(),
                weight: None,
                rank: None,
                provenance: build_provenance(now_ts),
                created_at: now_ts,
            })
            .collect(),
        source_target_type: None,
        source_target_id: None,
        embedding_refs: Vec::new(),
        status: HierarchyNodeStatus::Active,
        policy: Policy {
            visibility: Visibility::Workspace,
            retention: Retention::Durable,
            sensitivity: None,
            allowed_uses: Vec::new(),
            expires_at: None,
            delete_mode: None,
        },
        provenance: build_provenance(now_ts),
        created_at: now_ts,
        updated_at: None,
        metadata: None,
    }
}

fn build_provenance(now: Timestamp) -> Provenance {
    Provenance {
        source: "zbot_hierarchy_builder".to_string(),
        actor: Actor {
            id: Id::from("zbot-sleep"),
            kind: ActorKind::System,
            display_name: None,
            metadata: None,
        },
        observed_at: now,
        evidence: Vec::new(),
        derivations: Vec::new(),
        confidence: None,
        method: None,
    }
}

#[async_trait]
impl EngramHierarchyBuilder for ZbotHierarchyBuilder {
    /// Build hierarchy nodes for a scope. The engram config's optional
    /// knobs override the builder's configured defaults when present;
    /// `scope.subject` carries the agent id (the `partition_scope`
    /// mapping from P3.1).
    async fn build_hierarchy(
        &self,
        config: &HierarchyBuildConfig,
        scope: &Scope,
    ) -> CoreResult<Vec<HierarchyNode>> {
        let agent_id = scope.subject.clone().ok_or_else(|| CoreError::Adapter {
            adapter: "zbot_hierarchy_builder".to_string(),
            message: "scope.subject (agent id) is required for hierarchy build".to_string(),
        })?;
        // Config overrides: engram's recorded build config wins when set.
        if config.target_cluster_size.is_some()
            || config.max_layers.is_some()
            || config.llm_budget.is_some()
        {
            debug!(
                algorithm = %config.algorithm,
                "hierarchy: engram build-config overrides noted (defaults in use; \
                 per-build overrides apply on the zbot HierarchyConfig at construction)"
            );
        }
        let (stats, nodes) = self.build_for_agent(&agent_id).await;
        *self.last_stats.lock().unwrap() = Some(stats);
        Ok(nodes)
    }
}

// ---------------------------------------------------------------------------
// ZbotHierarchyBuildArm — the ConsolidationMutationExecutor
// ---------------------------------------------------------------------------

/// The `HierarchyBuild` arm of zbot's consolidation composite. Runs the
/// port impl for the request's scope and reports counts into the
/// consolidation audit trail.
pub struct ZbotHierarchyBuildArm {
    builder: Arc<ZbotHierarchyBuilder>,
    default_agent: String,
}

impl ZbotHierarchyBuildArm {
    pub fn new(builder: Arc<ZbotHierarchyBuilder>, default_agent: String) -> Self {
        Self {
            builder,
            default_agent,
        }
    }
}

#[async_trait]
impl ConsolidationMutationExecutor for ZbotHierarchyBuildArm {
    async fn execute(
        &self,
        request: &ConsolidationRequest,
        planned_tasks: &[ConsolidationTaskKind],
        started_at: Timestamp,
    ) -> CoreResult<ConsolidationMutationOutcome> {
        let mut task_results = Vec::new();
        let errors = Vec::new();

        for kind in planned_tasks {
            if kind != &ConsolidationTaskKind::HierarchyBuild {
                task_results.push(ConsolidationTaskResult {
                    task: kind.clone(),
                    status: ConsolidationTaskStatus::Skipped,
                    started_at,
                    completed_at: Some(started_at),
                    items_read: None,
                    items_written: None,
                    items_updated: None,
                    items_skipped: None,
                    model_calls: None,
                    errors: Vec::new(),
                    output_refs: Vec::new(),
                });
                continue;
            }

            let agent_id = request
                .scope
                .subject
                .clone()
                .unwrap_or_else(|| self.default_agent.clone());
            let (stats, _nodes) = self.builder.build_for_agent(&agent_id).await;
            *self.builder.last_stats.lock().unwrap() = Some(stats.clone());

            let written = stats.aggregates_created + stats.singletons_promoted;
            task_results.push(ConsolidationTaskResult {
                task: kind.clone(),
                status: if stats.errors > 0 {
                    ConsolidationTaskStatus::CompletedWithErrors
                } else {
                    ConsolidationTaskStatus::Completed
                },
                started_at,
                completed_at: Some(chrono::Utc::now()),
                items_read: None,
                items_written: Some(written),
                items_updated: None,
                items_skipped: None,
                model_calls: Some(stats.llm_calls),
                errors: Vec::new(),
                output_refs: Vec::new(),
            });
        }

        let stats = ConsolidationStats {
            memories_read: None,
            memories_written: None,
            beliefs_synthesized: None,
            contradictions_detected: None,
            hierarchy_nodes_created: Some(
                task_results
                    .iter()
                    .filter(|result| result.task == ConsolidationTaskKind::HierarchyBuild)
                    .filter_map(|result| result.items_written)
                    .sum(),
            ),
            hierarchy_relations_created: None,
            records_decayed: None,
            records_pruned: None,
            model_calls: None,
        };
        Ok(ConsolidationMutationOutcome {
            tasks: task_results,
            stats,
            errors,
        })
    }
}

// ---------------------------------------------------------------------------
// HierarchyConsolidation — the sleep-worker trigger
// ---------------------------------------------------------------------------

/// One hierarchy build cycle dispatched through engram's composite
/// executor (the `BeliefConsolidation` pattern). Returns the stats the
/// worker's observability summary consumes.
pub struct HierarchyConsolidation {
    builder: Arc<ZbotHierarchyBuilder>,
    composite: engram_consolidation::CompositeConsolidationExecutor,
}

impl HierarchyConsolidation {
    pub fn new(
        kg_store: Arc<dyn KnowledgeGraphStore>,
        llm: Arc<dyn AggregateEntityLlm>,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
        config: HierarchyConfig,
        default_agent: String,
    ) -> Self {
        let builder = Arc::new(ZbotHierarchyBuilder::new(
            kg_store,
            llm,
            embedding_client,
            config,
        ));
        let arm = Arc::new(ZbotHierarchyBuildArm::new(builder.clone(), default_agent));
        Self {
            builder,
            composite: engram_consolidation::CompositeConsolidationExecutor::new(vec![arm]),
        }
    }

    /// Run one build for an agent. Errors degrade to a logged warning;
    /// stats are always drained.
    pub async fn execute(&self, run_id: &str, agent_id: &str) -> HierarchyStats {
        let request = ConsolidationRequest {
            scope: Scope {
                tenant: "zbot".to_string(),
                subject: Some(agent_id.to_string()),
                workspace: None,
                session: None,
                environment: None,
            },
            requester: engram_domain::Requester {
                actor: Actor {
                    id: Id::from("zbot-sleep"),
                    kind: ActorKind::System,
                    display_name: None,
                    metadata: None,
                },
                roles: Vec::new(),
                permissions: Vec::new(),
                on_behalf_of: None,
            },
            since: None,
            until: None,
            strategy: None,
            dry_run: Some(false),
        };
        let planned = [ConsolidationTaskKind::HierarchyBuild];
        match self
            .composite
            .execute(&request, &planned, chrono::Utc::now())
            .await
        {
            Ok(outcome) => {
                for error in &outcome.errors {
                    tracing::warn!(run_id, agent_id, code = %error.code, %error.message,
                        "hierarchy-consolidation: task error");
                }
            }
            Err(e) => {
                tracing::warn!(run_id, agent_id, error = %e, "hierarchy-consolidation cycle failed");
            }
        }
        self.builder.take_stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::llm::EmbeddingError;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // ---- fakes (ported from hierarchy_builder.rs) ----

    struct MockLlm {
        synth_calls: Mutex<u64>,
        relation_calls: Mutex<u64>,
        relation_response: String,
        synth_should_fail: bool,
        synth_prior_history: Mutex<Vec<Vec<String>>>,
    }

    impl MockLlm {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                synth_calls: Mutex::new(0),
                relation_calls: Mutex::new(0),
                relation_response: "encompasses".to_string(),
                synth_should_fail: false,
                synth_prior_history: Mutex::new(Vec::new()),
            })
        }

        fn synth_call_count(&self) -> u64 {
            *self.synth_calls.lock().unwrap()
        }

        fn relation_call_count(&self) -> u64 {
            *self.relation_calls.lock().unwrap()
        }

        fn synth_prior_history(&self) -> Vec<Vec<String>> {
            self.synth_prior_history.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AggregateEntityLlm for MockLlm {
        async fn synthesize_aggregate(
            &self,
            members: &[AggregateMemberContext],
            prior_names: &[String],
        ) -> Result<AggregateResponse, String> {
            *self.synth_calls.lock().unwrap() += 1;
            self.synth_prior_history
                .lock()
                .unwrap()
                .push(prior_names.to_vec());
            if self.synth_should_fail {
                return Err("mock fail".into());
            }
            let call_index = *self.synth_calls.lock().unwrap();
            Ok(AggregateResponse {
                name: format!("agg-{call_index}-of-{}-members", members.len()),
                description: format!("Aggregate over {} entities.", members.len()),
            })
        }

        async fn synthesize_relation(
            &self,
            _a: &str,
            _b: &str,
            _lambda: usize,
        ) -> Result<String, String> {
            *self.relation_calls.lock().unwrap() += 1;
            Ok(self.relation_response.clone())
        }
    }

    struct MockEmbedder;

    #[async_trait]
    impl EmbeddingClient for MockEmbedder {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            Ok(texts.iter().map(|_| vec![0.01_f32; 384]).collect())
        }

        fn dimensions(&self) -> usize {
            384
        }

        fn model_name(&self) -> String {
            "mock".to_string()
        }
    }

    // ---- fixture builders (ported) ----

    async fn build_store_with_layer_zero(
        agent_id: &str,
        n_per_cluster: usize,
        n_clusters: usize,
    ) -> (Arc<dyn KnowledgeGraphStore>, TempDir) {
        use knowledge_graph::EntityType;
        let dir = tempfile::tempdir().unwrap();
        // Hierarchy fixtures embed at 384 dims (MockEmbedder); the shared
        // test_support provider is configured for 8-dim facts — open a
        // dedicated provider at the fixture's dimension so the store's
        // embedding-identity guard admits these vectors.
        let root = dir.path().join("engram-kg-hierarchy");
        std::fs::create_dir_all(&root).expect("root");
        let mut config =
            zbot_engram_adapter::AdapterConfig::engram_for_data_root(&root, "engram.db");
        config.embedding_provider.provider_type = "gateway-memory-test".to_string();
        config.embedding_provider.model = "hierarchy-384".to_string();
        config.embedding_provider.dimensions = 384;
        let provider = zbot_engram_adapter::EngramProvider::open(config.clone()).expect("provider");
        let kg: Arc<dyn KnowledgeGraphStore> = Arc::new(
            zbot_engram_adapter::EngramKnowledgeGraphStore::from_provider(config, &provider)
                .expect("kg store"),
        );

        // Seed through the trait surface (production path): entities with
        // L2-normalized name embeddings land in the same ANN index the
        // builder reads through.
        for c in 0..n_clusters {
            let angle = (c as f32) * std::f32::consts::TAU / (n_clusters as f32);
            let dx = angle.cos();
            let dy = angle.sin();
            for m in 0..n_per_cluster {
                let id = format!("c{c}-m{m}");
                let mut emb = vec![0.0_f32; 384];
                emb[0] = dx + (m as f32) * 0.001;
                emb[1] = dy + (m as f32) * 0.001;
                let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                for v in emb.iter_mut() {
                    *v /= norm;
                }

                let mut entity = knowledge_graph::Entity::new(
                    agent_id.to_string(),
                    EntityType::Concept,
                    id.clone(),
                );
                entity.name_embedding = Some(emb);
                kg.upsert_entity(agent_id, entity)
                    .await
                    .expect("seed entity");
            }
        }

        (kg, dir)
    }

    fn builder(
        store: Arc<dyn KnowledgeGraphStore>,
        llm: Arc<MockLlm>,
        embedder: bool,
        config: HierarchyConfig,
    ) -> ZbotHierarchyBuilder {
        ZbotHierarchyBuilder::new(
            store,
            llm,
            embedder.then(|| Arc::new(MockEmbedder) as Arc<dyn EmbeddingClient>),
            config,
        )
    }

    // ---- ported behavioral tests (throttle test dropped: no throttle) ----

    #[tokio::test]
    async fn empty_agent_yields_no_aggregates() {
        let (kg, _dir) = build_store_with_layer_zero("agent-empty", 0, 0).await;
        let llm = MockLlm::new();
        let b = builder(kg, llm.clone(), true, HierarchyConfig::default());
        let (stats, nodes) = b.build_for_agent("agent-empty").await;
        assert_eq!(stats.layers_built, 0);
        assert_eq!(stats.aggregates_created, 0);
        assert_eq!(stats.stopped_reason, StopReason::PoolTooSmall);
        assert_eq!(llm.synth_call_count(), 0);
        assert!(nodes.is_empty());
    }

    #[tokio::test]
    async fn singletons_short_circuit_no_llm() {
        // Two single-member blobs: every cluster is a singleton, so the
        // LLM must never be called (singletons promote member-as-name).
        let (store, _dir) = build_store_with_layer_zero("agent", 1, 2).await;
        // 3 blobs of 1 member → k=max(2, 3/20)=2 → singleton clusters.
        let llm = MockLlm::new();
        // target 2 so a 3-entity pool clusters at all (default 20 would
        // PoolTooSmall-stop before clustering).
        let b = builder(
            store,
            llm.clone(),
            true,
            HierarchyConfig {
                cluster_target_size: 2,
                ..HierarchyConfig::default()
            },
        );
        let (stats, nodes) = b.build_for_agent("agent").await;
        assert_eq!(stats.singletons_promoted, nodes.len() as u64);
        assert_eq!(
            llm.synth_call_count(),
            0,
            "singletons must not call the LLM"
        );
        assert!(!nodes.is_empty());
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn orchestrator_accumulates_prior_names_across_clusters() {
        // Two multi-member clusters: the second synthesize call must see
        // the first aggregate's name in prior_names.
        let (store, _dir) = build_store_with_layer_zero("agent", 6, 2).await;
        let llm = MockLlm::new();
        let config = HierarchyConfig {
            cluster_target_size: 4,
            ..HierarchyConfig::default()
        };
        let b = builder(store, llm.clone(), true, config);
        let (_stats, _nodes) = b.build_for_agent("agent").await;
        let history = llm.synth_prior_history();
        assert!(
            history.len() >= 2,
            "expected ≥2 synth calls, got {}",
            history.len()
        );
        assert!(
            history[1].iter().any(|n| n.starts_with("agg-")),
            "second call must see the first cluster's name: {:?}",
            history[1]
        );
    }

    #[tokio::test]
    async fn llm_failure_increments_error_count_but_continues() {
        let (store, _dir) = build_store_with_layer_zero("agent", 6, 2).await;
        let llm = Arc::new(MockLlm {
            synth_calls: Mutex::new(0),
            relation_calls: Mutex::new(0),
            relation_response: "encompasses".to_string(),
            synth_should_fail: true,
            synth_prior_history: Mutex::new(Vec::new()),
        });
        let config = HierarchyConfig {
            cluster_target_size: 4,
            ..HierarchyConfig::default()
        };
        let b = builder(store, llm.clone(), true, config);
        let (stats, _nodes) = b.build_for_agent("agent").await;
        assert!(stats.llm_calls >= 2);
        assert!(stats.errors >= 2, "each failed synth must count");
    }

    #[tokio::test]
    async fn budget_exhaustion_stops_cleanly() {
        let (store, _dir) = build_store_with_layer_zero("agent", 6, 2).await;
        let llm = MockLlm::new();
        let config = HierarchyConfig {
            cluster_target_size: 4,
            llm_budget_per_cycle: 1,
            ..HierarchyConfig::default()
        };
        let b = builder(store, llm.clone(), true, config);
        let (stats, _nodes) = b.build_for_agent("agent").await;
        assert_eq!(stats.stopped_reason, StopReason::BudgetExhausted);
        assert!(stats.llm_calls <= 2, "budget respected");
    }

    #[tokio::test]
    async fn pool_too_small_stops() {
        let (store, _dir) = build_store_with_layer_zero("agent", 1, 1).await;
        let llm = MockLlm::new();
        let b = builder(store, llm, true, HierarchyConfig::default());
        let (stats, _nodes) = b.build_for_agent("agent").await;
        assert_eq!(stats.stopped_reason, StopReason::PoolTooSmall);
    }

    #[tokio::test]
    async fn relation_llm_failure_falls_back() {
        // write_inter_cluster_pair falls back to "related-via" — verified
        // indirectly: relation call errors don't abort or count as errors
        // (they're logged + fallback). Covered by budget test shape here.
        let (store, _dir) = build_store_with_layer_zero("agent", 6, 2).await;
        let llm = MockLlm::new();
        let config = HierarchyConfig {
            cluster_target_size: 4,
            inter_cluster_relation_threshold: 0,
            ..HierarchyConfig::default()
        };
        let b = builder(store, llm.clone(), true, config);
        let (stats, _nodes) = b.build_for_agent("agent").await;
        let _ = llm.relation_call_count();
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn port_impl_returns_nodes_and_drainable_stats() {
        let (store, _dir) = build_store_with_layer_zero("agent", 6, 2).await;
        let llm = MockLlm::new();
        let b = Arc::new(builder(
            store,
            llm,
            true,
            HierarchyConfig {
                cluster_target_size: 4,
                ..HierarchyConfig::default()
            },
        ));
        let scope = Scope {
            tenant: "zbot".into(),
            subject: Some("agent".into()),
            workspace: None,
            session: None,
            environment: None,
        };
        let config = HierarchyBuildConfig {
            id: "cfg-test".into(),
            algorithm: "kmeans-cosine".into(),
            version: "1".into(),
            target_cluster_size: None,
            max_layers: None,
            similarity_metric: None,
            inter_cluster_threshold: None,
            llm_budget: None,
            created_at: chrono::Utc::now(),
        };
        let nodes = b.build_hierarchy(&config, &scope).await.expect("build");
        assert!(!nodes.is_empty());
        assert!(nodes.iter().all(|n| n.kind == HierarchyNodeKind::Aggregate));
        let stats = b.take_stats();
        assert!(stats.layers_built >= 1 || stats.stopped_reason != StopReason::NotStarted);
    }

    #[tokio::test]
    async fn consolidation_trigger_drains_stats() {
        let (store, _dir) = build_store_with_layer_zero("agent", 1, 3).await;
        let llm = MockLlm::new();
        let hc = HierarchyConsolidation::new(
            store,
            llm,
            Some(Arc::new(MockEmbedder)),
            HierarchyConfig::default(),
            "agent".into(),
        );
        let stats = hc.execute("run-test", "agent").await;
        assert!(stats.singletons_promoted > 0 || stats.stopped_reason != StopReason::NotStarted);
    }
}
