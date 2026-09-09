use crate::error::GraphStoreResult;
use crate::extracted::ExtractedKnowledge;
use crate::memory_facts::EmbeddingQueryIdentity;
use crate::types::*;
use async_trait::async_trait;
use knowledge_graph::types::{
    Entity, EntityType, GraphStats, NeighborInfo, Relationship, Subgraph,
};
// Port request/response shapes live in `zbot-stores-domain`; re-export
// at this crate's surface so existing imports of
// `zbot_stores::{DuplicateCandidate, DecayCandidate, StrategyCandidate,
// RelationshipContext, GraphView}` keep compiling.
pub use zbot_stores_domain::{
    AggregateSummary, DecayCandidate, DuplicateCandidate, EntityNameEmbeddingHit, GraphView,
    HierarchySummary, InterClusterRelationHit, RelationshipContext, StrategyCandidate,
};

/// Backend-agnostic persistence for the knowledge graph subsystem.
#[async_trait]
pub trait KnowledgeGraphStore: Send + Sync {
    // ---- Entities ---------------------------------------------------------
    async fn upsert_entity(&self, agent_id: &str, entity: Entity) -> GraphStoreResult<EntityId>;
    async fn get_entity(&self, id: &EntityId) -> GraphStoreResult<Option<Entity>>;
    async fn delete_entity(&self, id: &EntityId) -> GraphStoreResult<()>;
    async fn bump_entity_mention(&self, id: &EntityId) -> GraphStoreResult<()>;

    // ---- Aliases & resolution --------------------------------------------
    async fn add_alias(&self, entity_id: &EntityId, surface: &str) -> GraphStoreResult<()>;
    async fn resolve_entity(
        &self,
        agent_id: &str,
        entity_type: &EntityType,
        name: &str,
        embedding: Option<&[f32]>,
    ) -> GraphStoreResult<ResolveOutcome>;

    // ---- Relationships ---------------------------------------------------
    async fn upsert_relationship(
        &self,
        agent_id: &str,
        rel: Relationship,
    ) -> GraphStoreResult<RelationshipId>;
    async fn delete_relationship(&self, id: &RelationshipId) -> GraphStoreResult<()>;

    // ---- Bulk ingest -----------------------------------------------------
    async fn store_knowledge(
        &self,
        agent_id: &str,
        knowledge: ExtractedKnowledge,
    ) -> GraphStoreResult<StoreOutcome>;

    // ---- Read paths ------------------------------------------------------
    async fn get_neighbors(
        &self,
        id: &EntityId,
        direction: Direction,
        limit: usize,
    ) -> GraphStoreResult<Vec<Neighbor>>;

    async fn traverse(
        &self,
        seed: &EntityId,
        max_hops: usize,
        limit: usize,
    ) -> GraphStoreResult<Vec<TraversalHit>>;

    /// MEM-001 Part B-2 — confidence-aware BFS traversal for recall.
    ///
    /// Walks `kg_relationships` outward from `seed` up to `max_hops`,
    /// skipping edges whose `confidence` is below `min_edge_confidence`
    /// (so contradicted / decayed edges don't pollute recall). Returns
    /// the data recall needs to compute the final score itself —
    /// keeping the SQL focused on filtering + projection, not weighting.
    ///
    /// `min_edge_confidence` is applied to each edge along the path,
    /// not just the final hop. `edge_confidence_product` is the product
    /// of every edge's confidence along the shortest path from the
    /// seed; recall multiplies it by `hop_decay^hop` and the target's
    /// `entity_confidence`.
    ///
    /// Default: empty so backends that haven't implemented yet skip
    /// the traversal step silently — the recall pipeline already
    /// surfaces graph-ANN hits in step 4.
    async fn traverse_weighted(
        &self,
        _seed: &EntityId,
        _agent_id: &str,
        _max_hops: usize,
        _min_edge_confidence: f64,
        _limit: usize,
    ) -> GraphStoreResult<Vec<WeightedTraversalHit>> {
        Ok(Vec::new())
    }

    async fn search_entities_by_name(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> GraphStoreResult<Vec<Entity>>;

    /// Exact-name lookup for an entity. Used by tools that resolve a
    /// human-typed name to an entity id before traversing neighbours.
    /// Default falls back to filtering `search_entities_by_name` so
    /// backends without an indexed lookup still work.
    async fn get_entity_by_name(
        &self,
        agent_id: &str,
        name: &str,
    ) -> GraphStoreResult<Option<Entity>> {
        let matches = self.search_entities_by_name(agent_id, name, 16).await?;
        Ok(matches.into_iter().find(|e| e.name == name))
    }

    /// Exact entity lookup after callers normalize a human or model-produced
    /// name. Backends with case-insensitive indexes should override this; the
    /// default preserves the legacy exact-name behavior without turning an
    /// identity lookup into an unbounded ranked search.
    async fn get_entity_by_normalized_name(
        &self,
        agent_id: &str,
        normalized_name: &str,
    ) -> GraphStoreResult<Option<Entity>> {
        self.get_entity_by_name(agent_id, normalized_name).await
    }

    /// Search entities through a specific [`GraphView`] lens. Backends
    /// implement at least `Semantic` (mention_count DESC); other views
    /// may degrade to `Semantic` with a tracing warn. Default routes
    /// to `search_entities_by_name` so backends without view support
    /// still return ranked results.
    async fn search_entities_view(
        &self,
        agent_id: &str,
        query: &str,
        _view: GraphView,
        limit: usize,
    ) -> GraphStoreResult<Vec<Entity>> {
        self.search_entities_by_name(agent_id, query, limit).await
    }

    /// ANN search over the name-embedding index. Returns hits ordered by
    /// ascending L2-squared distance (callers convert to cosine via
    /// `1 - distance / 2`). The query embedding MUST be L2-normalized.
    /// Used by the unified-recall graph pool. Default returns empty so
    /// backends without a vector index over entity names degrade
    /// gracefully (the recall path just contributes no graph items).
    async fn search_entities_by_name_embedding(
        &self,
        _agent_id: &str,
        _query_embedding: &[f32],
        _top_k: usize,
    ) -> GraphStoreResult<Vec<EntityNameEmbeddingHit>> {
        Ok(Vec::new())
    }

    /// Identity-aware variant of [`KnowledgeGraphStore::search_entities_by_name_embedding`].
    /// Backends with persisted embedding identities should override this and
    /// fail closed when `query_identity` is missing or mismatched.
    async fn search_entities_by_name_embedding_with_identity(
        &self,
        agent_id: &str,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        top_k: usize,
    ) -> GraphStoreResult<Vec<EntityNameEmbeddingHit>> {
        let _ = query_identity;
        self.search_entities_by_name_embedding(agent_id, query_embedding, top_k)
            .await
    }

    // ---- Maintenance -----------------------------------------------------
    async fn reindex_embeddings(&self, new_dim: usize) -> GraphStoreResult<ReindexReport>;
    async fn stats(&self) -> GraphStoreResult<KgStats>;

    /// Find entities that satisfy the orphan-archival heuristic:
    /// `mention_count = 1`, `confidence < 0.5`, `first_seen_at` older
    /// than `min_age_hours`, zero incoming AND zero outgoing
    /// relationships, and not already archived (`epistemic_class !=
    /// 'archival'`). Used by the sleep-time orphan archiver. Hard-cap
    /// the result at `limit` rows for runaway protection.
    async fn list_archivable_orphans(
        &self,
        min_age_hours: u32,
        limit: usize,
    ) -> GraphStoreResult<Vec<ArchivableEntity>>;

    /// Soft-delete an entity by marking it archival. Sets the entity's
    /// `epistemic_class` to `'archival'`, records `reason` in
    /// `compressed_into`, and removes the entity's name-index row (so
    /// future searches don't surface it). Used by the sleep-time orphan
    /// archiver. Atomically applies all writes via a single transaction.
    async fn mark_entity_archival(&self, id: &EntityId, reason: &str) -> GraphStoreResult<()>;

    /// Apply temporal confidence decay to non-archival entities for an agent.
    ///
    /// For each entity whose `last_seen_at` is older than `skip_recent_hours`,
    /// compute `new_confidence = max(min_confidence, old * 0.5^(days/half_life))`
    /// where `days = now - last_seen_at`. Returns the number of rows updated.
    async fn decay_entity_confidence(
        &self,
        _agent_id: &str,
        _half_life_days: f64,
        _min_confidence: f64,
        _skip_recent_hours: i64,
    ) -> GraphStoreResult<u64> {
        Ok(0)
    }

    /// Same as [`KnowledgeGraphStore::decay_entity_confidence`] but for `kg_relationships`.
    async fn decay_relationship_confidence(
        &self,
        _agent_id: &str,
        _half_life_days: f64,
        _min_confidence: f64,
        _skip_recent_hours: i64,
    ) -> GraphStoreResult<u64> {
        Ok(0)
    }

    // ---- HTTP read paths (graph.rs handlers) ------------------------------

    /// Aggregate stats view used by `GET /api/graph/:agent_id/stats`.
    /// Computes entity/relationship counts, type breakdowns, and the
    /// top-10 most-connected entities for a given agent. The richer
    /// shape (vs. [`KnowledgeGraphStore::stats`]) is intentional —
    /// `stats` returns global counts only and is used by maintenance
    /// jobs; this is the per-agent UI view.
    async fn graph_stats(&self, agent_id: &str) -> GraphStoreResult<GraphStats>;

    /// List entities for an agent with optional `entity_type` filter and
    /// LIMIT/OFFSET pagination. Order is `mention_count DESC`.
    async fn list_entities(
        &self,
        agent_id: &str,
        entity_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GraphStoreResult<Vec<Entity>>;

    /// List relationships for an agent with optional `relationship_type`
    /// filter and LIMIT/OFFSET pagination. Order is `mention_count DESC`.
    async fn list_relationships(
        &self,
        agent_id: &str,
        relationship_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GraphStoreResult<Vec<Relationship>>;

    /// Get full neighbor info (entity + relationship + direction) for an
    /// entity. Unlike [`KnowledgeGraphStore::get_neighbors`] which
    /// returns the lightweight [`Neighbor`] type, this preserves the
    /// hydrated entity and relationship payloads for HTTP responses.
    async fn get_neighbors_full(
        &self,
        agent_id: &str,
        entity_id: &str,
        direction: Direction,
        limit: usize,
    ) -> GraphStoreResult<Vec<NeighborInfo>>;

    /// BFS subgraph centered on `center_entity_id` out to `max_hops`.
    /// Used by `GET /api/graph/:agent_id/entities/:entity_id/subgraph`.
    async fn get_subgraph(
        &self,
        agent_id: &str,
        center_entity_id: &str,
        max_hops: usize,
    ) -> GraphStoreResult<Subgraph>;

    /// Count all entities across all agents. Used by the Observatory
    /// aggregate stats endpoint.
    async fn count_all_entities(&self) -> GraphStoreResult<usize>;

    /// Count all relationships across all agents.
    async fn count_all_relationships(&self) -> GraphStoreResult<usize>;

    /// List entities across all agents with optional ward/type filters.
    /// Used by `GET /api/graph/all/entities`.
    async fn list_all_entities(
        &self,
        ward_id: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
    ) -> GraphStoreResult<Vec<Entity>>;

    /// List all relationships across all agents.
    /// Used by `GET /api/graph/all/relationships`.
    async fn list_all_relationships(&self, limit: usize) -> GraphStoreResult<Vec<Relationship>>;

    /// Vec0-index health snapshot: which of the expected vector tables
    /// exist in the backing store and how many rows are indexed in
    /// total. Backend-specific in implementation (SQLite-vec aux tables,
    /// SurrealDB index counts) but the trait surface stays the same.
    /// Used by `GET /api/embeddings/health`.
    async fn vec_index_health(&self) -> GraphStoreResult<VecIndexHealth>;

    // ---- Sleep-time maintenance (Phase D2) -------------------------------
    //
    // Operation-oriented surface for the maintenance worker. Each method
    // describes WHAT the consumer needs (find dupes, merge, decay-prune)
    // — backends implement HOW with whatever native primitive fits. Each
    // has a default returning empty/no-op so backends that haven't
    // implemented yet degrade gracefully (the sleep cycle just does
    // less work, doesn't crash).

    /// Find pairs of entities of the same `entity_type` whose name
    /// embeddings have cosine similarity >= `threshold`. Used by the
    /// Compactor to surface merge candidates. Returns up to `limit`
    /// pairs. Default: no candidates.
    async fn find_duplicate_candidates(
        &self,
        _agent_id: &str,
        _entity_type: &knowledge_graph::EntityType,
        _threshold: f32,
        _limit: usize,
    ) -> GraphStoreResult<Vec<DuplicateCandidate>> {
        Ok(Vec::new())
    }

    /// Atomically merge `loser` into `winner`: re-target every
    /// relationship that pointed to `loser` so it points to `winner`,
    /// then mark `loser` as merged. Backend chooses the atomicity
    /// primitive (SQLite transaction, SurrealDB BEGIN/COMMIT block).
    /// Default: no-op error so misuse is loud.
    async fn merge_entity_into(
        &self,
        _loser: &EntityId,
        _winner: &EntityId,
    ) -> GraphStoreResult<()> {
        Err(crate::GraphStoreError::Backend(
            "merge_entity_into not implemented for this store".to_string(),
        ))
    }

    /// Find entities that are orphans (zero in/out edges) AND old
    /// (`last_seen_at` older than `min_age_days`). Used by the
    /// DecayEngine to surface prune candidates. Excludes already-
    /// archived entities. Default: no candidates.
    async fn list_orphan_old_candidates(
        &self,
        _agent_id: &str,
        _min_age_days: i64,
        _limit: usize,
    ) -> GraphStoreResult<Vec<DecayCandidate>> {
        Ok(Vec::new())
    }

    /// Soft-delete an entity by marking it pruned. Distinct sentinel
    /// from `mark_entity_archival` so operators can tell decay-driven
    /// prunes apart from orphan archival. Used by the Pruner.
    /// Default: no-op error.
    async fn mark_entity_pruned(&self, _id: &EntityId) -> GraphStoreResult<()> {
        Err(crate::GraphStoreError::Backend(
            "mark_entity_pruned not implemented for this store".to_string(),
        ))
    }

    // ---- Sleep-time synthesis (Phase D4) ---------------------------------
    //
    // Reads needed by `Synthesizer` to surface cross-session strategy
    // candidates and load their context. Each method captures one
    // semantic operation; backends implement with whatever primitive
    // fits (SQLite uses JOINs over kg_entities × kg_relationships ×
    // kg_episodes, Surreal uses graph traversals over the `relationship`
    // edge table). Default: empty results so the synthesis cycle is a
    // no-op on backends that haven't implemented yet.

    /// Entities seen in at least `min_sessions` distinct sessions over
    /// the last `lookback_days`. Excludes archival/compressed rows.
    /// Used by the Synthesizer to find cross-session strategy
    /// candidates. Default: no candidates.
    async fn list_strategy_candidates(
        &self,
        _min_sessions: i64,
        _lookback_days: i64,
        _limit: usize,
    ) -> GraphStoreResult<Vec<StrategyCandidate>> {
        Ok(Vec::new())
    }

    /// Relationship summaries (`src --[type]--> tgt` strings) for an
    /// entity, plus the distinct `session_id`s that referenced any of
    /// those relationships within the last `lookback_days`. Used by the
    /// Synthesizer to build per-candidate LLM context. Default: empty.
    async fn relationship_context_for_entity(
        &self,
        _entity_id: &str,
        _lookback_days: i64,
        _edge_limit: usize,
    ) -> GraphStoreResult<RelationshipContext> {
        Ok(RelationshipContext::default())
    }

    /// Distinct, deduped episode ids that touched the entity within
    /// the last `lookback_days`. Used by the Synthesizer to attribute
    /// a synthesized fact to its contributing episodes. Default: empty.
    async fn episode_ids_for_entity(
        &self,
        _entity_id: &str,
        _lookback_days: i64,
    ) -> GraphStoreResult<Vec<String>> {
        Ok(Vec::new())
    }

    // ---- Contradiction propagation (MEM-001 Part A) ----------------------
    //
    // When a `memory_facts.contradicted_by` is set, the KG entities and
    // relationships that were created from the same episode become
    // suspect. This method locates those KG rows so the caller can decay
    // their confidence multiplicatively.

    /// Locate kg_entities + kg_relationships whose `source_episode_ids`
    /// overlaps the supplied episode-id set. Comma-delimited token match
    /// (mirrors the storage format used by `episode_ids_for_entity` and
    /// `relationship_context_for_entity`).
    ///
    /// `agent_id` scopes the query so cross-agent rows don't leak. Only
    /// non-archival rows are returned — archived rows are already out of
    /// recall and don't need further decay.
    ///
    /// Empty `episode_ids` yields an empty result without issuing the
    /// query. Default: empty so backends that haven't implemented yet
    /// disable propagation without breaking the sleep cycle.
    async fn find_kg_nodes_by_episode_ids(
        &self,
        _agent_id: &str,
        _episode_ids: &[String],
    ) -> GraphStoreResult<KgNodesForEpisodes> {
        Ok(KgNodesForEpisodes::default())
    }

    /// Multiplicatively decay `confidence` for the given kg_entities
    /// (and floor at `min_floor`). Returns the number of rows updated.
    /// Default: 0 so backends that haven't implemented yet are no-ops.
    async fn apply_entity_confidence_multiplier(
        &self,
        _agent_id: &str,
        _entity_ids: &[EntityId],
        _factor: f64,
        _min_floor: f64,
    ) -> GraphStoreResult<u64> {
        Ok(0)
    }

    /// Multiplicatively decay `confidence` for the given kg_relationships
    /// (and floor at `min_floor`). Returns the number of rows updated.
    async fn apply_relationship_confidence_multiplier(
        &self,
        _agent_id: &str,
        _relationship_ids: &[RelationshipId],
        _factor: f64,
        _min_floor: f64,
    ) -> GraphStoreResult<u64> {
        Ok(0)
    }

    // ---- Hierarchical memory (Phase H-3) ---------------------------------
    //
    // Reads needed by the `HierarchyBuilder` sleep worker to decide
    // whether two clusters of entities are "connected enough" to
    // synthesise an aggregate inter-cluster relation at the next
    // hierarchy layer (LeanRAG's λ > τ gate).

    /// Count the relationships that bridge two clusters of entities at
    /// the *current* (non-archival) layer of the graph.
    ///
    /// Both directions count: an edge from `cluster_a` to `cluster_b`
    /// and an edge from `cluster_b` to `cluster_a` each contribute 1.
    /// The two clusters are assumed disjoint by the caller (K-means
    /// produces disjoint partitions); when they overlap the SQL still
    /// runs, it just counts edges within the overlap twice (once per
    /// direction) — same as it does for non-overlapping clusters.
    ///
    /// `agent_id` scopes the query so cross-agent edges don't leak.
    /// Only `epistemic_class = 'current'` relationships are counted —
    /// archived/superseded edges don't contribute to "current
    /// connectivity strength" at any layer.
    ///
    /// Empty clusters trivially yield `0`. Default: `0` so backends
    /// that haven't implemented yet just disable inter-cluster
    /// relation synthesis (the hierarchy still builds without it).
    async fn connectivity_strength(
        &self,
        _agent_id: &str,
        _cluster_a: &[EntityId],
        _cluster_b: &[EntityId],
    ) -> GraphStoreResult<usize> {
        Ok(0)
    }

    /// Promote a cluster of layer-N entities into a single aggregate
    /// entity at layer N+1 (Phase H-3d).
    ///
    /// Three things happen atomically:
    ///   1. INSERT a new entity into `kg_entities` with `layer = layer`
    ///      and `parent_cluster_id = NULL` (it sits at the top of the
    ///      hierarchy until the next layer is built on top of it).
    ///   2. UPDATE each member's `parent_cluster_id` to point to the
    ///      new aggregate.
    ///   3. INSERT the aggregate's embedding into the name-index table
    ///      when one is provided (so the aggregate participates in
    ///      semantic recall and in further clustering at layer+1).
    ///
    /// The caller is responsible for synthesising `name` + `description`
    /// (typically via an LLM, or short-circuit-copying from the single
    /// member of a singleton cluster). This method does no LLM work.
    ///
    /// Returns the newly-allocated entity id. Backends that haven't
    /// implemented yet return a `Backend` error so the orchestrator
    /// fails loudly rather than silently dropping aggregates.
    async fn promote_cluster_to_aggregate(
        &self,
        _agent_id: &str,
        _layer: i64,
        _members: &[EntityId],
        _name: &str,
        _description: &str,
        _embedding: Option<Vec<f32>>,
    ) -> GraphStoreResult<EntityId> {
        Err(crate::GraphStoreError::Backend(
            "promote_cluster_to_aggregate not implemented for this store".to_string(),
        ))
    }

    /// Write an inter-cluster relation between two aggregate entities
    /// at the same hierarchy layer (Phase H-3d).
    ///
    /// `source_aggregate` and `target_aggregate` must both live at the
    /// same `layer` (this is the LeanRAG load-bearing edge — the one
    /// that links abstract concepts at the same level of abstraction).
    /// Sets `is_inter_cluster = 1` so recall can distinguish synthesised
    /// hierarchy edges from base-extracted ones.
    ///
    /// The caller decides `relationship_type` (the LLM either picks a
    /// specific verb or the caller falls back to a generic placeholder
    /// like `"related-via"` when the LLM budget is exhausted).
    ///
    /// Returns the newly-allocated relationship id. Backends that
    /// haven't implemented yet return a `Backend` error.
    async fn write_inter_cluster_relation(
        &self,
        _agent_id: &str,
        _layer: i64,
        _source_aggregate: &EntityId,
        _target_aggregate: &EntityId,
        _relationship_type: &str,
    ) -> GraphStoreResult<RelationshipId> {
        Err(crate::GraphStoreError::Backend(
            "write_inter_cluster_relation not implemented for this store".to_string(),
        ))
    }

    /// List current-class entities at a specific hierarchy layer
    /// together with their name embeddings (Phase H-3e).
    ///
    /// Used by the `HierarchyBuilder` orchestrator to fetch the
    /// candidate pool for K-means at each layer. Entities without an
    /// embedding row in the name-index are skipped — they're invisible
    /// to clustering, but the orchestrator logs a count so an operator
    /// can spot embedding-pipeline gaps.
    ///
    /// Returns `(EntityId, embedding)` pairs. The order is unspecified.
    /// `limit = 0` is treated as "no limit" by impls — pass a real
    /// cap unless you mean it.
    ///
    /// Default: empty. Backends that haven't implemented yet just
    /// can't build the hierarchy on top of them, which is the right
    /// degradation.
    async fn list_entities_with_embeddings_at_layer(
        &self,
        _agent_id: &str,
        _layer: i64,
        _limit: usize,
    ) -> GraphStoreResult<Vec<EntityWithEmbedding>> {
        Ok(Vec::new())
    }

    /// Compute the Lowest Common Ancestor of a set of seed entities
    /// in the hierarchical KG (Phase H-4 / LeanRAG).
    ///
    /// For each seed, walks `parent_cluster_id` upward until either
    /// the path terminates (NULL parent at the top of the hierarchy)
    /// or a fixed safety cap is reached. The LCA is the deepest entity
    /// (highest `layer`) that appears in EVERY seed's ancestry chain.
    ///
    /// Returns:
    /// - `lca`: the LCA entity id, or `None` if no common ancestor.
    /// - `path_entities`: deduplicated set of all entities on every
    ///   seed's path from itself up to (and including) the LCA. The
    ///   seeds themselves are excluded — the recall pipeline already
    ///   has them from the upstream graph-ANN step and we don't want
    ///   to double-count.
    /// - `max_layer`: layer of the LCA, or `0` when there's no LCA.
    ///
    /// Empty seed input yields an empty result. Single-seed input
    /// yields `lca = Some(seed)` with an empty path (the seed is its
    /// own LCA, and we exclude it). Layer-0 seeds with NULL parents
    /// (hierarchy never built) yield `lca = None`.
    ///
    /// Default: empty. Backends that haven't implemented yet leave
    /// recall byte-for-byte identical to pre-H-4 behaviour.
    async fn compute_lca_path(
        &self,
        _agent_id: &str,
        _seed_entity_ids: &[EntityId],
    ) -> GraphStoreResult<LcaPath> {
        Ok(LcaPath::default())
    }

    /// List inter-cluster relations whose BOTH endpoints sit in a
    /// given set of entity ids (Phase H-4 follow-up).
    ///
    /// Used by recall step 5c: after `compute_lca_path` returns the
    /// `path_entities`, this lookup surfaces the synthesised edges
    /// between aggregates ALONG that path so the agent sees not just
    /// the abstraction chain but also how its siblings relate.
    ///
    /// Filters: `is_inter_cluster = 1` AND `epistemic_class = 'current'`
    /// AND agent_id matches AND both endpoints in `entity_ids`. An
    /// empty input list short-circuits to an empty result without
    /// running SQL.
    ///
    /// Default: empty. Backends without hierarchy data degrade
    /// gracefully — recall just won't surface the edges.
    async fn list_inter_cluster_relations(
        &self,
        _agent_id: &str,
        _entity_ids: &[EntityId],
    ) -> GraphStoreResult<Vec<InterClusterRelationHit>> {
        Ok(Vec::new())
    }

    /// Hierarchical-memory health snapshot used by the Observatory
    /// pill + slideover (`GET /api/hierarchy/stats`).
    ///
    /// Returns layer-by-layer entity counts, total inter-cluster edge
    /// count, and the top-N aggregates by member count (with their
    /// LLM-synthesised names + descriptions) — enough to render
    /// "Hierarchy: 2 layers / 693 / 30 entities" and a drill-in panel
    /// without a second round-trip.
    ///
    /// `top_n = 0` is treated as "no aggregates" by impls (counts
    /// alone). Default: empty summary so backends without hierarchy
    /// data report cleanly as "no hierarchy built yet".
    async fn hierarchy_summary(
        &self,
        _agent_id: &str,
        _top_n: usize,
    ) -> GraphStoreResult<HierarchySummary> {
        Ok(HierarchySummary::default())
    }
}

/// Result of an LCA computation over a seed set (Phase H-4).
///
/// See [`KnowledgeGraphStore::compute_lca_path`].
#[derive(Debug, Clone, Default)]
pub struct LcaPath {
    /// LCA entity id, or `None` when the seeds share no common ancestor.
    pub lca: Option<EntityId>,
    /// Union of every seed's ancestry chain up to (and including) the
    /// LCA. Seeds themselves are excluded. Empty when `lca` is `None`.
    /// Order is unspecified.
    pub path_entities: Vec<EntityId>,
    /// `layer` of the LCA, or `0` when there's no LCA.
    pub max_layer: i64,
}

/// `(EntityId, name_embedding)` carrier for the layer-fetch API used
/// by the HierarchyBuilder. Kept separate from `Entity` because the
/// orchestrator only needs the id + the vector for K-means; pulling
/// the full Entity row would mean joining `properties`, alias counts,
/// and other columns that the clustering step doesn't read.
#[derive(Debug, Clone)]
pub struct EntityWithEmbedding {
    pub id: EntityId,
    pub embedding: Vec<f32>,
}

/// Carrier for `find_kg_nodes_by_episode_ids` — the KG entities and
/// relationships whose `source_episode_ids` overlap an episode-id set.
/// Used by the DecayEngine to apply multiplicative confidence decay
/// when a contributing fact has been contradicted (MEM-001 Part A).
#[derive(Debug, Clone, Default)]
pub struct KgNodesForEpisodes {
    pub entity_ids: Vec<EntityId>,
    pub relationship_ids: Vec<RelationshipId>,
}

/// Carrier for `traverse_weighted` (MEM-001 Part B-2). One per
/// reachable entity at the shortest hop-distance from the seed.
///
/// The store does the filtering + projection (min_edge_confidence,
/// cycle detection, shortest-hop dedup) and returns enough data for
/// recall to compute the final score without baking decay math into
/// SQL: `score = seed_score * hop_decay^hop * edge_confidence_product *
/// entity_confidence`. Keeping the math out of SQL means recall can
/// tune `hop_decay` per-call without invalidating prepared statements.
#[derive(Debug, Clone)]
pub struct WeightedTraversalHit {
    pub entity_id: EntityId,
    pub name: String,
    pub entity_type: String,
    /// Shortest hop distance from the seed (1 = direct neighbour).
    pub hop: usize,
    /// Comma-separated relationship-type chain from seed to this hit.
    /// Same shape as `TraversalHit.path`.
    pub path: String,
    /// `kg_entities.confidence` at this hit.
    pub entity_confidence: f64,
    /// Product of `kg_relationships.confidence` along the shortest
    /// path from the seed (between 0 and 1 for well-behaved data).
    /// Empty path (hop = 0) would be 1.0 but the seed itself is
    /// excluded by the trait method.
    pub edge_confidence_product: f64,
}
