use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_runtime::llm::EmbeddingClient;
use async_trait::async_trait;
use knowledge_graph::types::{
    Direction as KgDirection, Entity, EntityType, GraphStats, NeighborInfo, Relationship, Subgraph,
};

use crate::kg::storage::{ArchivableEntityRow, GraphStorage};
use zbot_stores::error::StoreError;
use zbot_stores::extracted::ExtractedKnowledge;
use zbot_stores::types::{
    ArchivableEntity, Direction, EntityId, KgStats, Neighbor, ReindexReport, RelationshipId,
    ResolveOutcome, StoreOutcome, TraversalHit, VecIndexHealth,
};
use zbot_stores::KnowledgeGraphStore;
use zbot_stores::{KgNodesForEpisodes, StoreResult};

use crate::blocking::{block, map_graph_err};
use crate::reindex;

/// Embed each entity's `name` and assign it to `entity.name_embedding`.
///
/// `store_entity` (in `kg::storage`) writes a `kg_name_index` row only
/// when `name_embedding` is `Some`. That index is what the hierarchy
/// builder clusters on (`list_entities_with_embeddings_at_layer` JOINs
/// `kg_name_index`). Without this step every entity lands with
/// `name_embedding = None`, `kg_name_index` stays empty, and the
/// hierarchy builder bails `PoolTooSmall` every cycle — never building a
/// layer past 0.
///
/// Best-effort: on embed failure or a count mismatch the entities keep
/// `name_embedding = None` and the caller proceeds — degraded exactly to
/// pre-fix behavior, never an error.
async fn embed_entity_names(client: &dyn EmbeddingClient, entities: &mut [Entity]) {
    if entities.is_empty() {
        return;
    }
    let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
    match client.embed(&names).await {
        Ok(vecs) if vecs.len() == entities.len() => {
            for (entity, vec) in entities.iter_mut().zip(vecs) {
                entity.name_embedding = Some(vec);
            }
        }
        Ok(vecs) => {
            tracing::warn!(
                expected = entities.len(),
                got = vecs.len(),
                "kg: entity-name embedding count mismatch — entities stored without name index"
            );
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                count = entities.len(),
                "kg: entity-name embedding failed — entities stored without name index"
            );
        }
    }
}

/// SQLite implementation of `KnowledgeGraphStore`. Wraps the existing
/// `crate::kg::storage::GraphStorage` and bridges its synchronous
/// rusqlite API into the async trait via `spawn_blocking`.
#[derive(Clone)]
pub struct SqliteKgStore {
    storage: Arc<GraphStorage>,
    /// Active embedding client used by `reindex_embeddings`. Optional so
    /// integration tests that don't exercise the reindex path can construct
    /// the store without wiring an embedding backend. Production wiring
    /// goes through [`SqliteKgStore::with_embedding_client`].
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
}

impl SqliteKgStore {
    /// Construct a store without an embedding client. Calls to
    /// `reindex_embeddings` on a store built this way return
    /// `StoreError::Backend("no embedding client configured ...")`.
    pub fn new(storage: Arc<GraphStorage>) -> Self {
        Self {
            storage,
            embedding_client: None,
        }
    }

    /// Construct a store that supports `reindex_embeddings`.
    ///
    /// The supplied client must produce vectors of the dimension passed to
    /// [`KnowledgeGraphStore::reindex_embeddings`]; per-row mismatches are
    /// logged and skipped (the index for that row stays empty until the next
    /// reindex).
    pub fn with_embedding_client(
        storage: Arc<GraphStorage>,
        embedding_client: Arc<dyn EmbeddingClient>,
    ) -> Self {
        Self {
            storage,
            embedding_client: Some(embedding_client),
        }
    }
}

#[async_trait]
impl KnowledgeGraphStore for SqliteKgStore {
    // Methods filled in by Tasks 4-9.

    async fn upsert_entity(&self, agent_id: &str, entity: Entity) -> StoreResult<EntityId> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            storage
                .upsert_entity(&agent_id, entity)
                .map(EntityId::from)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn get_entity(&self, id: &EntityId) -> StoreResult<Option<Entity>> {
        let storage = self.storage.clone();
        let id = id.0.clone();
        block(move || storage.get_entity_by_id(&id).map_err(map_graph_err)).await
    }

    async fn delete_entity(&self, id: &EntityId) -> StoreResult<()> {
        let storage = self.storage.clone();
        let id = id.0.clone();
        block(move || storage.delete_entity_by_id(&id).map_err(map_graph_err)).await
    }

    async fn bump_entity_mention(&self, id: &EntityId) -> StoreResult<()> {
        let storage = self.storage.clone();
        let id = id.0.clone();
        block(move || storage.bump_entity_mention(&id).map_err(map_graph_err)).await
    }

    async fn add_alias(&self, entity_id: &EntityId, surface: &str) -> StoreResult<()> {
        let storage = self.storage.clone();
        let entity_id = entity_id.0.clone();
        let surface = surface.to_string();
        block(move || {
            storage
                .add_alias(&entity_id, &surface)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn resolve_entity(
        &self,
        agent_id: &str,
        entity_type: &EntityType,
        name: &str,
        embedding: Option<&[f32]>,
    ) -> StoreResult<ResolveOutcome> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let entity_type = entity_type.clone();
        let name = name.to_string();
        let embedding = embedding.map(|e| e.to_vec());
        block(move || {
            let result = storage
                .resolve_entity(&agent_id, &entity_type, &name, embedding.as_deref())
                .map_err(map_graph_err)?;
            Ok(match result {
                Some(matched_id) => ResolveOutcome::Match(EntityId::from(matched_id)),
                None => ResolveOutcome::NoMatch,
            })
        })
        .await
    }

    async fn upsert_relationship(
        &self,
        agent_id: &str,
        rel: Relationship,
    ) -> StoreResult<RelationshipId> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            storage
                .upsert_relationship(&agent_id, rel)
                .map(RelationshipId::from)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn delete_relationship(&self, id: &RelationshipId) -> StoreResult<()> {
        let storage = self.storage.clone();
        let id = id.0.clone();
        block(move || storage.delete_relationship(&id).map_err(map_graph_err)).await
    }

    async fn store_knowledge(
        &self,
        agent_id: &str,
        mut knowledge: ExtractedKnowledge,
    ) -> StoreResult<StoreOutcome> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        // Capture counts before consuming the value via `.into()`.
        // GraphStorage::store_knowledge returns `()` with no outcome breakdown,
        // so we derive counts from the input. merged vs inserted is unknown at
        // this layer for Phase 1, so all entities count as inserted.
        let entity_count = knowledge.entities.len() as u64;
        let rel_count = knowledge.relationships.len() as u64;

        // Embed entity names so `store_entity` populates `kg_name_index` —
        // the index the hierarchy builder clusters on. Async, so it runs
        // before the synchronous `block`. Best-effort (see
        // `embed_entity_names`): a missing client or embed failure leaves
        // `name_embedding = None` and the store proceeds unchanged.
        if let Some(client) = self.embedding_client.clone() {
            embed_entity_names(client.as_ref(), &mut knowledge.entities).await;
        }

        block(move || {
            storage
                .store_knowledge(&agent_id, knowledge.into())
                .map_err(map_graph_err)?;
            Ok(StoreOutcome {
                entities_inserted: entity_count,
                entities_merged: 0,
                relationships_inserted: rel_count,
            })
        })
        .await
    }

    async fn get_neighbors(
        &self,
        id: &EntityId,
        direction: Direction,
        limit: usize,
    ) -> StoreResult<Vec<Neighbor>> {
        let storage = self.storage.clone();
        let id = id.0.clone();
        block(move || {
            storage
                .get_neighbors("", &id, direction.into(), limit)
                .map(|rows| rows.into_iter().map(Into::into).collect())
                .map_err(map_graph_err)
        })
        .await
    }

    async fn traverse(
        &self,
        seed: &EntityId,
        max_hops: usize,
        limit: usize,
    ) -> StoreResult<Vec<TraversalHit>> {
        let storage = self.storage.clone();
        let seed = seed.0.clone();
        block(move || {
            storage
                .traverse_sync(&seed, max_hops.min(255) as u8, limit)
                .map(|rows| rows.into_iter().map(Into::into).collect())
                .map_err(map_graph_err)
        })
        .await
    }

    async fn traverse_weighted(
        &self,
        seed: &EntityId,
        agent_id: &str,
        max_hops: usize,
        min_edge_confidence: f64,
        limit: usize,
    ) -> StoreResult<Vec<zbot_stores::WeightedTraversalHit>> {
        let storage = self.storage.clone();
        let seed = seed.0.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            // Pull more rows than the cap so the shortest-hop / max-product
            // dedup has room to work — without slack the SQL `LIMIT` could
            // cut off a shorter-hop path in favor of a longer one with
            // higher edge product (since ORDER BY hop ASC kicks in first
            // anyway). 4× cap is plenty: BFS to 2 hops on a sparse graph
            // doesn't yield many alternate paths per node.
            let raw_limit = limit.saturating_mul(4).max(limit);
            let rows = storage
                .traverse_weighted_sync(
                    &seed,
                    &agent_id,
                    max_hops.min(255) as u8,
                    min_edge_confidence,
                    raw_limit,
                )
                .map_err(map_graph_err)?;

            // Dedup by entity_id: keep shortest hop; tie-break by highest
            // edge_confidence_product. Rows arrive already sorted by
            // (hop ASC, edge_conf_product DESC) so the first occurrence
            // of each entity_id is the winner.
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut deduped: Vec<zbot_stores::WeightedTraversalHit> = Vec::new();
            for r in rows {
                if seen.insert(r.entity_id.clone()) {
                    deduped.push(zbot_stores::WeightedTraversalHit {
                        entity_id: EntityId(r.entity_id),
                        name: r.name,
                        entity_type: r.entity_type,
                        hop: r.hop,
                        path: r.path,
                        entity_confidence: r.entity_confidence,
                        edge_confidence_product: r.edge_confidence_product,
                    });
                    if deduped.len() >= limit {
                        break;
                    }
                }
            }
            Ok(deduped)
        })
        .await
    }

    async fn search_entities_by_name(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let query = query.to_string();
        block(move || {
            storage
                .search_by_name(&agent_id, &query, limit)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn get_entity_by_name(&self, agent_id: &str, name: &str) -> StoreResult<Option<Entity>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let name = name.to_string();
        block(move || {
            storage
                .get_entity_by_name(&agent_id, &name)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn search_entities_view(
        &self,
        agent_id: &str,
        query: &str,
        view: zbot_stores::GraphView,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        let service = crate::kg::service::GraphService::new(self.storage.clone());
        service
            .search_entities_view(agent_id, query, view, limit)
            .await
            .map_err(map_graph_err)
    }

    async fn search_entities_by_name_embedding(
        &self,
        agent_id: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> StoreResult<Vec<zbot_stores::EntityNameEmbeddingHit>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let query_embedding = query_embedding.to_vec();
        block(move || {
            storage
                .search_entities_by_name_embedding(&query_embedding, top_k, &agent_id)
                .map(|rows| {
                    rows.into_iter()
                        .map(|(id, name, entity_type, distance, confidence)| {
                            zbot_stores::EntityNameEmbeddingHit {
                                id,
                                name,
                                entity_type,
                                distance,
                                confidence,
                            }
                        })
                        .collect()
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn reindex_embeddings(&self, new_dim: usize) -> StoreResult<ReindexReport> {
        // The trait contract is "rebuild embedding indexes for new dim and
        // return a final report" — there is intentionally no progress
        // callback in the trait surface (a SurrealDB impl rebuilds in one
        // shot; UX progress is impl-specific). Callers that want per-table
        // progress events drive `crate::reindex::reindex_all` directly via
        // the gateway-side wrapper at
        // `gateway-execution/src/sleep/embedding_reindex.rs`.
        let client = self.embedding_client.clone().ok_or_else(|| {
            StoreError::Backend(
                "no embedding client configured — use SqliteKgStore::with_embedding_client".into(),
            )
        })?;

        let db = self.storage.knowledge_db().clone();
        let summaries = reindex::reindex_all(&db, client, new_dim, &|_, _, _| {})
            .await
            .map_err(StoreError::Backend)?;

        let tables_rebuilt: Vec<String> = summaries
            .iter()
            .map(|(target, _)| target.table.to_string())
            .collect();
        let rows_indexed: u64 = summaries
            .iter()
            .map(|(_, summary)| summary.indexed as u64)
            .sum();

        Ok(ReindexReport {
            tables_rebuilt,
            rows_indexed,
        })
    }

    async fn stats(&self) -> StoreResult<KgStats> {
        let storage = self.storage.clone();
        block(move || {
            let (entity_count, relationship_count, alias_count) =
                storage.stats().map_err(map_graph_err)?;
            Ok(KgStats {
                entity_count,
                relationship_count,
                alias_count,
            })
        })
        .await
    }

    async fn list_archivable_orphans(
        &self,
        min_age_hours: u32,
        limit: usize,
    ) -> StoreResult<Vec<ArchivableEntity>> {
        let storage = self.storage.clone();
        block(move || {
            storage
                .find_archivable_orphans(min_age_hours, limit)
                .map(|rows| {
                    rows.into_iter()
                        .map(|r: ArchivableEntityRow| ArchivableEntity {
                            entity_id: EntityId::from(r.id),
                            agent_id: r.agent_id,
                            entity_type: r.entity_type,
                            name: r.name,
                        })
                        .collect()
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn mark_entity_archival(&self, id: &EntityId, reason: &str) -> StoreResult<()> {
        let storage = self.storage.clone();
        let id = id.0.clone();
        let reason = reason.to_string();
        block(move || {
            storage
                .mark_entity_archival(&id, &reason)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn decay_entity_confidence(
        &self,
        agent_id: &str,
        half_life_days: f64,
        min_confidence: f64,
        skip_recent_hours: i64,
    ) -> StoreResult<u64> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            decay_kg_table(
                &storage,
                "kg_entities",
                &agent_id,
                half_life_days,
                min_confidence,
                skip_recent_hours,
            )
        })
        .await
    }

    async fn decay_relationship_confidence(
        &self,
        agent_id: &str,
        half_life_days: f64,
        min_confidence: f64,
        skip_recent_hours: i64,
    ) -> StoreResult<u64> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            decay_kg_table(
                &storage,
                "kg_relationships",
                &agent_id,
                half_life_days,
                min_confidence,
                skip_recent_hours,
            )
        })
        .await
    }

    // -------- HTTP read paths -----------------------------------------------

    async fn graph_stats(&self, agent_id: &str) -> StoreResult<GraphStats> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || compute_graph_stats(&storage, &agent_id)).await
    }

    async fn list_entities(
        &self,
        agent_id: &str,
        entity_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<Entity>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let entity_type = entity_type.map(|s| s.to_string());
        block(move || {
            storage
                .list_entities(&agent_id, entity_type.as_deref(), limit, offset)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn list_relationships(
        &self,
        agent_id: &str,
        relationship_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<Relationship>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let relationship_type = relationship_type.map(|s| s.to_string());
        block(move || {
            storage
                .list_relationships(&agent_id, relationship_type.as_deref(), limit, offset)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn get_neighbors_full(
        &self,
        agent_id: &str,
        entity_id: &str,
        direction: Direction,
        limit: usize,
    ) -> StoreResult<Vec<NeighborInfo>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let entity_id = entity_id.to_string();
        block(move || {
            storage
                .get_neighbors(&agent_id, &entity_id, direction.into(), limit)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn get_subgraph(
        &self,
        agent_id: &str,
        center_entity_id: &str,
        max_hops: usize,
    ) -> StoreResult<Subgraph> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let center = center_entity_id.to_string();
        block(move || compute_subgraph(&storage, &agent_id, &center, max_hops)).await
    }

    async fn count_all_entities(&self) -> StoreResult<usize> {
        let storage = self.storage.clone();
        block(move || storage.count_all_entities().map_err(map_graph_err)).await
    }

    async fn count_all_relationships(&self) -> StoreResult<usize> {
        let storage = self.storage.clone();
        block(move || storage.count_all_relationships().map_err(map_graph_err)).await
    }

    async fn list_all_entities(
        &self,
        ward_id: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        let storage = self.storage.clone();
        let ward_id = ward_id.map(|s| s.to_string());
        let entity_type = entity_type.map(|s| s.to_string());
        block(move || {
            storage
                .list_all_entities(ward_id.as_deref(), entity_type.as_deref(), limit)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn list_all_relationships(&self, limit: usize) -> StoreResult<Vec<Relationship>> {
        let storage = self.storage.clone();
        block(move || storage.list_all_relationships(limit).map_err(map_graph_err)).await
    }

    async fn connectivity_strength(
        &self,
        agent_id: &str,
        cluster_a: &[zbot_stores::types::EntityId],
        cluster_b: &[zbot_stores::types::EntityId],
    ) -> StoreResult<usize> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let cluster_a: Vec<String> = cluster_a.iter().map(|e| e.0.clone()).collect();
        let cluster_b: Vec<String> = cluster_b.iter().map(|e| e.0.clone()).collect();
        block(move || {
            storage
                .connectivity_strength(&agent_id, &cluster_a, &cluster_b)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn promote_cluster_to_aggregate(
        &self,
        agent_id: &str,
        layer: i64,
        members: &[zbot_stores::types::EntityId],
        name: &str,
        description: &str,
        embedding: Option<Vec<f32>>,
    ) -> StoreResult<zbot_stores::types::EntityId> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let members: Vec<String> = members.iter().map(|e| e.0.clone()).collect();
        let name = name.to_string();
        let description = description.to_string();
        block(move || {
            storage
                .promote_cluster_to_aggregate(
                    &agent_id,
                    layer,
                    &members,
                    &name,
                    &description,
                    embedding,
                )
                .map(zbot_stores::types::EntityId)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn write_inter_cluster_relation(
        &self,
        agent_id: &str,
        layer: i64,
        source_aggregate: &zbot_stores::types::EntityId,
        target_aggregate: &zbot_stores::types::EntityId,
        relationship_type: &str,
    ) -> StoreResult<zbot_stores::types::RelationshipId> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let src = source_aggregate.0.clone();
        let tgt = target_aggregate.0.clone();
        let rel_type = relationship_type.to_string();
        block(move || {
            storage
                .write_inter_cluster_relation(&agent_id, layer, &src, &tgt, &rel_type)
                .map(zbot_stores::types::RelationshipId)
                .map_err(map_graph_err)
        })
        .await
    }

    async fn list_entities_with_embeddings_at_layer(
        &self,
        agent_id: &str,
        layer: i64,
        limit: usize,
    ) -> StoreResult<Vec<zbot_stores::EntityWithEmbedding>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            storage
                .list_entities_with_embeddings_at_layer(&agent_id, layer, limit)
                .map(|rows| {
                    rows.into_iter()
                        .map(|(id, embedding)| zbot_stores::EntityWithEmbedding {
                            id: zbot_stores::types::EntityId(id),
                            embedding,
                        })
                        .collect()
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn hierarchy_summary(
        &self,
        agent_id: &str,
        top_n: usize,
    ) -> StoreResult<zbot_stores::HierarchySummary> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            storage
                .hierarchy_summary(&agent_id, top_n)
                .map(|(layer_counts, inter_cluster_relations, aggs)| {
                    zbot_stores::HierarchySummary {
                        layer_counts,
                        inter_cluster_relations,
                        top_aggregates: aggs
                            .into_iter()
                            .map(|(id, name, layer, member_count, description)| {
                                zbot_stores::AggregateSummary {
                                    id,
                                    name,
                                    layer,
                                    member_count,
                                    description,
                                }
                            })
                            .collect(),
                    }
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn list_inter_cluster_relations(
        &self,
        agent_id: &str,
        entity_ids: &[zbot_stores::types::EntityId],
    ) -> StoreResult<Vec<zbot_stores::InterClusterRelationHit>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let ids: Vec<String> = entity_ids.iter().map(|e| e.0.clone()).collect();
        block(move || {
            storage
                .list_inter_cluster_relations(&agent_id, &ids)
                .map(|rows| {
                    rows.into_iter()
                        .map(|(id, src, tgt, rel_type, layer)| {
                            zbot_stores::InterClusterRelationHit {
                                id,
                                source_entity_id: src,
                                target_entity_id: tgt,
                                relationship_type: rel_type,
                                layer,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn compute_lca_path(
        &self,
        agent_id: &str,
        seed_entity_ids: &[zbot_stores::types::EntityId],
    ) -> StoreResult<zbot_stores::LcaPath> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let seeds: Vec<String> = seed_entity_ids.iter().map(|e| e.0.clone()).collect();
        block(move || {
            storage
                .compute_lca_path(&agent_id, &seeds)
                .map(|(lca, path, max_layer)| zbot_stores::LcaPath {
                    lca: lca.map(zbot_stores::types::EntityId),
                    path_entities: path.into_iter().map(zbot_stores::types::EntityId).collect(),
                    max_layer,
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn vec_index_health(&self) -> StoreResult<VecIndexHealth> {
        // SQLite-vec maintains an aux `<table>_rowids` table per index;
        // counting its rows is the faithful "indexed row count" used by
        // the embeddings health endpoint. This SQL is intentionally
        // SQLite-specific and stays inside the impl crate — the trait
        // surface stays portable.
        let db = self.storage.knowledge_db().clone();
        block(move || {
            const ROWID_TABLES: &[&str] = &[
                "memory_facts_index_rowids",
                "kg_name_index_rowids",
                "session_episodes_index_rowids",
            ];
            let (tables_present, tables_missing) = db
                .with_connection(|conn| Ok(crate::list_vec_table_presence(conn)))
                .map_err(|e| StoreError::Backend(format!("vec_index_health: {e}")))?;
            let indexed_rows = db
                .with_connection(|conn| {
                    let mut total = 0usize;
                    for tbl in ROWID_TABLES {
                        let n: i64 = conn
                            .query_row(&format!("SELECT count(*) FROM {tbl}"), [], |r| r.get(0))
                            .unwrap_or(0);
                        total = total.saturating_add(n.max(0) as usize);
                    }
                    Ok(total)
                })
                .unwrap_or(0);
            Ok(VecIndexHealth {
                tables_present,
                tables_missing,
                indexed_rows,
            })
        })
        .await
    }

    // ---- Sleep-time maintenance (Phase D2) -------------------------------

    async fn find_duplicate_candidates(
        &self,
        agent_id: &str,
        entity_type: &knowledge_graph::EntityType,
        threshold: f32,
        limit: usize,
    ) -> StoreResult<Vec<zbot_stores::DuplicateCandidate>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        let type_str = entity_type.as_str().to_string();
        block(move || {
            storage
                .find_duplicate_candidates(&agent_id, &type_str, threshold, limit)
                .map(|pairs| {
                    pairs
                        .into_iter()
                        .map(
                            |(loser_id, winner_id, score)| zbot_stores::DuplicateCandidate {
                                loser_entity_id: loser_id,
                                winner_entity_id: winner_id,
                                cosine_similarity: score,
                            },
                        )
                        .collect()
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn merge_entity_into(&self, loser: &EntityId, winner: &EntityId) -> StoreResult<()> {
        let storage = self.storage.clone();
        let loser_id = loser.0.clone();
        let winner_id = winner.0.clone();
        block(move || {
            storage
                .merge_entity_into(&loser_id, &winner_id)
                .map(|_| ())
                .map_err(map_graph_err)
        })
        .await
    }

    async fn list_orphan_old_candidates(
        &self,
        agent_id: &str,
        min_age_days: i64,
        limit: usize,
    ) -> StoreResult<Vec<zbot_stores::DecayCandidate>> {
        let storage = self.storage.clone();
        let agent_id = agent_id.to_string();
        block(move || {
            storage
                .list_orphan_old_candidates(&agent_id, min_age_days, limit)
                .map(|rows| {
                    rows.into_iter()
                        .map(|c| zbot_stores::DecayCandidate {
                            id: c.id,
                            name: c.name,
                            entity_type: c.entity_type,
                            mention_count: c.mention_count,
                        })
                        .collect()
                })
                .map_err(map_graph_err)
        })
        .await
    }

    async fn mark_entity_pruned(&self, id: &EntityId) -> StoreResult<()> {
        let storage = self.storage.clone();
        let entity_id = id.0.clone();
        block(move || storage.mark_pruned(&entity_id).map_err(map_graph_err)).await
    }

    // ---- Sleep-time synthesis (Phase D4) -------------------------------

    async fn list_strategy_candidates(
        &self,
        min_sessions: i64,
        lookback_days: i64,
        limit: usize,
    ) -> StoreResult<Vec<zbot_stores::StrategyCandidate>> {
        let db = self.storage.knowledge_db().clone();
        let limit_i64 = limit as i64;
        block(move || {
            db.with_connection(|conn| {
                // SQLite `datetime()` doesn't take bound days directly, so format
                // the modifier into the SQL literal. Both `min_sessions` and
                // `limit` stay parameterised.
                let date_modifier = format!("-{lookback_days} days");
                let sql = format!(
                    "SELECT e.id, e.agent_id, e.name, e.entity_type, COUNT(DISTINCT ep.session_id) AS n_sessions
                     FROM kg_entities e
                     JOIN kg_relationships r ON r.source_entity_id = e.id OR r.target_entity_id = e.id
                     JOIN kg_episodes ep    ON instr(COALESCE(r.source_episode_ids, ''), ep.id) > 0
                     WHERE e.epistemic_class != 'archival'
                       AND e.compressed_into IS NULL
                       AND ep.created_at > datetime('now', '{date_modifier}')
                       AND ep.session_id IS NOT NULL
                     GROUP BY e.id
                     HAVING n_sessions >= ?1
                     ORDER BY n_sessions DESC, e.mention_count DESC
                     LIMIT ?2"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![min_sessions, limit_i64], |row| {
                        Ok(zbot_stores::StrategyCandidate {
                            entity_id: row.get(0)?,
                            agent_id: row.get(1)?,
                            name: row.get(2)?,
                            entity_type: row.get(3)?,
                            n_sessions: row.get::<_, i64>(4)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .map_err(StoreError::Backend)
        })
        .await
    }

    async fn relationship_context_for_entity(
        &self,
        entity_id: &str,
        lookback_days: i64,
        edge_limit: usize,
    ) -> StoreResult<zbot_stores::RelationshipContext> {
        let db = self.storage.knowledge_db().clone();
        let entity_id = entity_id.to_string();
        let edge_limit_i64 = edge_limit as i64;
        block(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT r.relationship_type, r.source_entity_id, r.target_entity_id,
                            r.source_episode_ids
                     FROM kg_relationships r
                     WHERE r.source_entity_id = ?1 OR r.target_entity_id = ?1
                     LIMIT ?2",
                )?;
                let rows: Vec<(String, String, String, Option<String>)> = stmt
                    .query_map(rusqlite::params![entity_id, edge_limit_i64], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                let summaries: Vec<String> = rows
                    .iter()
                    .map(|(ty, src, tgt, _)| format!("{src} --[{ty}]--> {tgt}"))
                    .collect();

                let episode_id_blob: String = rows
                    .iter()
                    .filter_map(|(_, _, _, eids)| eids.clone())
                    .collect::<Vec<_>>()
                    .join(",");

                let mut session_ids: Vec<String> = Vec::new();
                if !episode_id_blob.is_empty() {
                    let date_modifier = format!("-{lookback_days} days");
                    let sql = format!(
                        "SELECT DISTINCT session_id FROM kg_episodes
                         WHERE session_id IS NOT NULL
                           AND instr(?1, id) > 0
                           AND created_at > datetime('now', '{date_modifier}')"
                    );
                    let mut q = conn.prepare(&sql)?;
                    session_ids = q
                        .query_map(rusqlite::params![episode_id_blob], |row| {
                            row.get::<_, String>(0)
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                }
                Ok(zbot_stores::RelationshipContext {
                    summaries,
                    session_ids,
                })
            })
            .map_err(StoreError::Backend)
        })
        .await
    }

    async fn episode_ids_for_entity(
        &self,
        entity_id: &str,
        lookback_days: i64,
    ) -> StoreResult<Vec<String>> {
        let db = self.storage.knowledge_db().clone();
        let entity_id = entity_id.to_string();
        block(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT r.source_episode_ids FROM kg_relationships r
                     WHERE (r.source_entity_id = ?1 OR r.target_entity_id = ?1)
                       AND r.source_episode_ids IS NOT NULL",
                )?;
                let blobs = stmt
                    .query_map(rusqlite::params![entity_id], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut ids: Vec<String> = blobs
                    .iter()
                    .flat_map(|b| b.split(',').map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect();
                ids.sort();
                ids.dedup();
                if ids.is_empty() {
                    return Ok(ids);
                }
                let placeholders = vec!["?"; ids.len()].join(",");
                let date_modifier = format!("-{lookback_days} days");
                let sql = format!(
                    "SELECT id FROM kg_episodes
                     WHERE id IN ({placeholders})
                       AND session_id IS NOT NULL
                       AND created_at > datetime('now', '{date_modifier}')"
                );
                let mut q = conn.prepare(&sql)?;
                let params_vec: Vec<&dyn rusqlite::types::ToSql> = ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::types::ToSql)
                    .collect();
                let filtered: Vec<String> = q
                    .query_map(params_vec.as_slice(), |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(filtered)
            })
            .map_err(StoreError::Backend)
        })
        .await
    }

    // ---- Contradiction propagation (MEM-001 Part A) ----------------------

    async fn find_kg_nodes_by_episode_ids(
        &self,
        agent_id: &str,
        episode_ids: &[String],
    ) -> StoreResult<KgNodesForEpisodes> {
        if episode_ids.is_empty() {
            return Ok(KgNodesForEpisodes::default());
        }
        let db = self.storage.knowledge_db().clone();
        let agent_id = agent_id.to_string();
        // Comma-flanked tokens so we don't match a longer episode id by
        // substring: WHERE instr(',' || source_episode_ids || ',', ',<id>,') > 0
        let needles: Vec<String> = episode_ids.iter().map(|id| format!(",{id},")).collect();

        block(move || {
            db.with_connection(|conn| {
                let or_clauses: String = (0..needles.len())
                    .map(|i| format!("instr(',' || source_episode_ids || ',', ?{}) > 0", i + 2))
                    .collect::<Vec<_>>()
                    .join(" OR ");

                let entity_sql = format!(
                    "SELECT id FROM kg_entities
                     WHERE agent_id = ?1
                       AND source_episode_ids IS NOT NULL
                       AND epistemic_class != 'archival'
                       AND ({or_clauses})"
                );
                let rel_sql = format!(
                    "SELECT id FROM kg_relationships
                     WHERE agent_id = ?1
                       AND source_episode_ids IS NOT NULL
                       AND epistemic_class != 'archival'
                       AND ({or_clauses})"
                );

                let mut params: Vec<&dyn rusqlite::types::ToSql> =
                    Vec::with_capacity(1 + needles.len());
                params.push(&agent_id);
                for n in &needles {
                    params.push(n);
                }

                let mut entity_stmt = conn.prepare(&entity_sql)?;
                let entity_ids: Vec<EntityId> = entity_stmt
                    .query_map(params.as_slice(), |row| row.get::<_, String>(0))?
                    .map(|r| r.map(EntityId))
                    .collect::<Result<Vec<_>, _>>()?;
                drop(entity_stmt);

                let mut rel_stmt = conn.prepare(&rel_sql)?;
                let relationship_ids: Vec<RelationshipId> = rel_stmt
                    .query_map(params.as_slice(), |row| row.get::<_, String>(0))?
                    .map(|r| r.map(RelationshipId))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(KgNodesForEpisodes {
                    entity_ids,
                    relationship_ids,
                })
            })
            .map_err(StoreError::Backend)
        })
        .await
    }

    async fn apply_entity_confidence_multiplier(
        &self,
        agent_id: &str,
        entity_ids: &[EntityId],
        factor: f64,
        min_floor: f64,
    ) -> StoreResult<u64> {
        if entity_ids.is_empty() {
            return Ok(0);
        }
        let db = self.storage.knowledge_db().clone();
        let agent_id = agent_id.to_string();
        let id_strs: Vec<String> = entity_ids.iter().map(|e| e.0.clone()).collect();
        block(move || {
            db.with_connection(|conn| {
                let placeholders = vec!["?"; id_strs.len()].join(",");
                let sql = format!(
                    "UPDATE kg_entities
                     SET confidence = MAX(?1, confidence * ?2)
                     WHERE agent_id = ?3 AND id IN ({placeholders})"
                );
                let mut params: Vec<&dyn rusqlite::types::ToSql> =
                    Vec::with_capacity(3 + id_strs.len());
                params.push(&min_floor);
                params.push(&factor);
                params.push(&agent_id);
                for id in &id_strs {
                    params.push(id);
                }
                let n = conn.execute(&sql, params.as_slice())?;
                Ok(n as u64)
            })
            .map_err(StoreError::Backend)
        })
        .await
    }

    async fn apply_relationship_confidence_multiplier(
        &self,
        agent_id: &str,
        relationship_ids: &[RelationshipId],
        factor: f64,
        min_floor: f64,
    ) -> StoreResult<u64> {
        if relationship_ids.is_empty() {
            return Ok(0);
        }
        let db = self.storage.knowledge_db().clone();
        let agent_id = agent_id.to_string();
        let id_strs: Vec<String> = relationship_ids.iter().map(|r| r.0.clone()).collect();
        block(move || {
            db.with_connection(|conn| {
                let placeholders = vec!["?"; id_strs.len()].join(",");
                let sql = format!(
                    "UPDATE kg_relationships
                     SET confidence = MAX(?1, confidence * ?2)
                     WHERE agent_id = ?3 AND id IN ({placeholders})"
                );
                let mut params: Vec<&dyn rusqlite::types::ToSql> =
                    Vec::with_capacity(3 + id_strs.len());
                params.push(&min_floor);
                params.push(&factor);
                params.push(&agent_id);
                for id in &id_strs {
                    params.push(id);
                }
                let n = conn.execute(&sql, params.as_slice())?;
                Ok(n as u64)
            })
            .map_err(StoreError::Backend)
        })
        .await
    }
}

// ============================================================================
// Helpers — extracted from the trait body so each impl method stays small.
// ============================================================================

/// Compute the rich stats view used by `GET /api/graph/:agent_id/stats`.
/// Mirrors the historical `GraphService::get_stats` — port lives here so
/// the handler can call through the trait without depending on
/// `GraphService`. Synchronous; runs inside a `spawn_blocking`.
fn compute_graph_stats(storage: &GraphStorage, agent_id: &str) -> StoreResult<GraphStats> {
    let entity_count = storage.count_entities(agent_id).map_err(map_graph_err)?;
    let relationship_count = storage
        .count_relationships(agent_id)
        .map_err(map_graph_err)?;

    let entities = storage
        .list_entities(agent_id, None, 10_000, 0)
        .map_err(map_graph_err)?;
    let mut entity_types: HashMap<String, usize> = HashMap::new();
    for entity in &entities {
        *entity_types
            .entry(entity.entity_type.as_str().to_string())
            .or_default() += 1;
    }

    let relationships = storage
        .list_relationships(agent_id, None, 10_000, 0)
        .map_err(map_graph_err)?;
    let mut relationship_types: HashMap<String, usize> = HashMap::new();
    let mut entity_connections: HashMap<String, usize> = HashMap::new();
    for rel in &relationships {
        *relationship_types
            .entry(rel.relationship_type.as_str().to_string())
            .or_default() += 1;
        *entity_connections
            .entry(rel.source_entity_id.clone())
            .or_default() += 1;
        *entity_connections
            .entry(rel.target_entity_id.clone())
            .or_default() += 1;
    }

    let entity_id_to_name: HashMap<&str, &str> = entities
        .iter()
        .map(|e| (e.id.as_str(), e.name.as_str()))
        .collect();

    let mut connection_vec: Vec<(String, usize)> = entity_connections
        .into_iter()
        .filter_map(|(id, count)| {
            entity_id_to_name
                .get(id.as_str())
                .map(|name| (name.to_string(), count))
        })
        .collect();
    connection_vec.sort_by(|a, b| b.1.cmp(&a.1));
    connection_vec.truncate(10);

    Ok(GraphStats {
        entity_count,
        relationship_count,
        entity_types,
        relationship_types,
        most_connected_entities: connection_vec,
    })
}

/// BFS subgraph centered on `center_entity_id` out to `max_hops`.
/// Mirrors the historical `GraphService::get_subgraph`.
fn compute_subgraph(
    storage: &GraphStorage,
    agent_id: &str,
    center_entity_id: &str,
    max_hops: usize,
) -> StoreResult<Subgraph> {
    let mut visited_entities: HashSet<String> = HashSet::new();
    let mut visited_relationships: HashSet<String> = HashSet::new();
    let mut entities: Vec<Entity> = Vec::new();
    let mut relationships: Vec<Relationship> = Vec::new();

    let mut current_hop: Vec<String> = vec![center_entity_id.to_string()];
    visited_entities.insert(center_entity_id.to_string());

    for _hop in 0..max_hops {
        if current_hop.is_empty() {
            break;
        }
        let mut next_hop: Vec<String> = Vec::new();
        for entity_id in &current_hop {
            let neighbors = storage
                .get_neighbors(agent_id, entity_id, KgDirection::Both, 1_000)
                .map_err(map_graph_err)?;
            collect_subgraph_neighbors(
                neighbors,
                &mut visited_entities,
                &mut visited_relationships,
                &mut entities,
                &mut relationships,
                &mut next_hop,
            );
        }
        current_hop = next_hop;
    }

    // Insert the center entity at the front, matching the historical shape.
    let all_entities = storage
        .list_entities(agent_id, None, 10_000, 0)
        .map_err(map_graph_err)?;
    if let Some(center) = all_entities.into_iter().find(|e| e.id == center_entity_id) {
        entities.insert(0, center);
    }

    Ok(Subgraph {
        entities,
        relationships,
        center: center_entity_id.to_string(),
        max_hops,
    })
}

/// Batch-apply temporal confidence decay to a KG table.
///
/// Both `kg_entities` and `kg_relationships` share the same schema for
/// `id`, `agent_id`, `confidence`, `last_seen_at`, and `epistemic_class`,
/// so one helper covers both. We compute the per-row multiplier in Rust
/// (`new = max(min_confidence, old * 0.5^(days / half_life))`) and apply
/// via individual UPDATEs inside a single connection — this avoids
/// depending on SQLite's optional `exp()` math extension and keeps the
/// math portable.
fn decay_kg_table(
    storage: &Arc<crate::kg::storage::GraphStorage>,
    table: &str,
    agent_id: &str,
    half_life_days: f64,
    min_confidence: f64,
    skip_recent_hours: i64,
) -> StoreResult<u64> {
    if half_life_days <= 0.0 {
        return Err(StoreError::Invalid(
            "half_life_days must be > 0".to_string(),
        ));
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(skip_recent_hours);
    let cutoff_rfc = cutoff.to_rfc3339();
    let decay_constant = std::f64::consts::LN_2 / half_life_days;
    let table = table.to_string();
    let agent_id = agent_id.to_string();

    // MEM-004: single bulk UPDATE replaces the historical O(N) per-row
    // SELECT+compute+UPDATE loop. SQLite's `exp()` + `julianday()` do the
    // math server-side. rusqlite's `bundled` feature ships SQLite ≥3.35
    // with `SQLITE_ENABLE_MATH_FUNCTIONS` so `exp()` is always available.
    //
    // Behaviour change from the old loop: rows whose new confidence equals
    // their old confidence (e.g. already at `min_confidence` floor and
    // exp() pushes them below) are counted by the bulk UPDATE but were
    // skipped by the per-row loop. The cycle-summary semantics are
    // "decay was applied to N rows in this pass" either way; callers
    // don't rely on the exact pre-MEM-004 count meaning.
    storage
        .knowledge_db()
        .with_connection(move |conn| {
            let sql = format!(
                "UPDATE {table} \
                 SET confidence = MAX(?1, confidence * exp(-?2 * \
                                                          (julianday('now') - julianday(last_seen_at)))) \
                 WHERE agent_id = ?3 \
                   AND epistemic_class != 'archival' \
                   AND last_seen_at < ?4"
            );
            let n = conn.execute(
                &sql,
                rusqlite::params![min_confidence, decay_constant, agent_id, cutoff_rfc],
            )?;
            Ok(n as u64)
        })
        .map_err(StoreError::Backend)
}

/// Process one hop's worth of neighbors into the BFS accumulators.
fn collect_subgraph_neighbors(
    neighbors: Vec<NeighborInfo>,
    visited_entities: &mut HashSet<String>,
    visited_relationships: &mut HashSet<String>,
    entities: &mut Vec<Entity>,
    relationships: &mut Vec<Relationship>,
    next_hop: &mut Vec<String>,
) {
    for neighbor in neighbors {
        if !visited_relationships.contains(&neighbor.relationship.id) {
            visited_relationships.insert(neighbor.relationship.id.clone());
            relationships.push(neighbor.relationship);
        }
        let neighbor_id = neighbor.entity.id.clone();
        if !visited_entities.contains(&neighbor_id) {
            visited_entities.insert(neighbor_id.clone());
            entities.push(neighbor.entity);
            next_hop.push(neighbor_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_db::KnowledgeDatabase;
    use gateway_services::VaultPaths;

    #[tokio::test]
    async fn decay_entity_confidence_reduces_old_entities() {
        use chrono::Duration as ChronoDuration;
        let tmp = tempfile::tempdir().unwrap();
        let paths = std::sync::Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = std::sync::Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        let graph = std::sync::Arc::new(crate::kg::storage::GraphStorage::new(db).expect("graph"));
        let store = SqliteKgStore::new(graph.clone());

        let agent_id = "agent-decay-test";
        let stale_time = chrono::Utc::now() - ChronoDuration::days(180); // 2 half-lives at 90d
        let fresh_time = chrono::Utc::now();

        // Insert one stale + one fresh entity directly via SQL for full control.
        graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('stale', ?1, 'Concept', 'Stale', 'stale', 'h1',
                             'current', 0.8, 1, 0, ?2, ?2)",
                    rusqlite::params![agent_id, stale_time.to_rfc3339()],
                )?;
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('fresh', ?1, 'Concept', 'Fresh', 'fresh', 'h2',
                             'current', 0.8, 1, 0, ?2, ?2)",
                    rusqlite::params![agent_id, fresh_time.to_rfc3339()],
                )?;
                Ok(())
            })
            .unwrap();

        let updated = store
            .decay_entity_confidence(agent_id, 90.0, 0.01, 24)
            .await
            .expect("decay");
        assert_eq!(updated, 1, "exactly the stale entity should be decayed");

        // Verify stale confidence approximately halved twice
        // (2 half-lives → 0.25× of 0.8 = 0.2).
        let new_stale_conf: f64 = graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT confidence FROM kg_entities WHERE id = 'stale'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert!(
            (new_stale_conf - 0.2).abs() < 0.02,
            "stale conf {new_stale_conf} should be ~0.2"
        );

        // Fresh entity unchanged.
        let fresh_conf: f64 = graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT confidence FROM kg_entities WHERE id = 'fresh'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert!(
            (fresh_conf - 0.8).abs() < 1e-6,
            "fresh conf should be unchanged"
        );
    }

    // ----- MEM-004: bulk decay UPDATE correctness ---------------------

    /// Verify the bulk UPDATE (which uses the SQLite-side `exp()` UDF)
    /// produces the same values as the canonical formula
    /// `confidence × exp(-ln(2)/half_life × days)`. Three durations:
    /// 1 day (small decay), 90 days (one half-life ≈ 0.5×), 180 days
    /// (two half-lives ≈ 0.25×).
    #[tokio::test]
    async fn bulk_decay_matches_canonical_formula_at_1_90_and_180_days() {
        use chrono::Duration as ChronoDuration;
        let tmp = tempfile::tempdir().unwrap();
        let paths = std::sync::Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = std::sync::Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        let graph = std::sync::Arc::new(crate::kg::storage::GraphStorage::new(db).expect("graph"));
        let store = SqliteKgStore::new(graph.clone());

        let agent_id = "agent-bulk-decay";
        let half_life_days = 90.0_f64;
        let cases: [(&str, i64); 3] = [("d1", 1), ("d90", 90), ("d180", 180)];

        for (id, days_ago) in cases {
            let ts = (chrono::Utc::now() - ChronoDuration::days(days_ago)).to_rfc3339();
            graph
                .knowledge_db()
                .with_connection(|conn| {
                    conn.execute(
                        "INSERT INTO kg_entities
                            (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                             epistemic_class, confidence, mention_count, access_count,
                             first_seen_at, last_seen_at)
                         VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, 'current', 1.0, 1, 0, ?3, ?3)",
                        rusqlite::params![id, agent_id, ts],
                    )?;
                    Ok(())
                })
                .unwrap();
        }

        let n = store
            .decay_entity_confidence(agent_id, half_life_days, 0.01, 0)
            .await
            .expect("decay ok");
        assert_eq!(n, 3, "all three test rows should be touched");

        let decay_constant = std::f64::consts::LN_2 / half_life_days;
        for (id, days_ago) in cases {
            let actual: f64 = graph
                .knowledge_db()
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT confidence FROM kg_entities WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                })
                .unwrap();
            let expected = (1.0 * (-decay_constant * days_ago as f64).exp()).max(0.01);
            assert!(
                (actual - expected).abs() < 1e-3,
                "{id}: actual {actual:.6} should match canonical {expected:.6} (Δ < 1e-3)"
            );
        }
    }

    // ----- MEM-001 Part A: contradiction propagation helpers ----------

    fn insert_entity(
        graph: &crate::kg::storage::GraphStorage,
        id: &str,
        agent_id: &str,
        confidence: f64,
        source_episode_ids: Option<&str>,
        epistemic_class: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at, source_episode_ids)
                     VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, ?3, ?4, 1, 0, ?5, ?5, ?6)",
                    rusqlite::params![
                        id,
                        agent_id,
                        epistemic_class,
                        confidence,
                        now,
                        source_episode_ids
                    ],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)] // 9 args in a test helper is fine — keeps callers explicit.
    fn insert_relationship(
        graph: &crate::kg::storage::GraphStorage,
        id: &str,
        agent_id: &str,
        src: &str,
        tgt: &str,
        rel_type: &str,
        confidence: f64,
        source_episode_ids: Option<&str>,
        epistemic_class: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_relationships
                        (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at, source_episode_ids)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 0, ?8, ?8, ?9)",
                    rusqlite::params![
                        id,
                        agent_id,
                        src,
                        tgt,
                        rel_type,
                        epistemic_class,
                        confidence,
                        now,
                        source_episode_ids
                    ],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn setup_store() -> (
        tempfile::TempDir,
        std::sync::Arc<crate::kg::storage::GraphStorage>,
        SqliteKgStore,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = std::sync::Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = std::sync::Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        let graph = std::sync::Arc::new(crate::kg::storage::GraphStorage::new(db).expect("graph"));
        let store = SqliteKgStore::new(graph.clone());
        (tmp, graph, store)
    }

    #[tokio::test]
    async fn find_kg_nodes_by_episode_ids_empty_input_returns_empty() {
        let (_tmp, _graph, store) = setup_store();
        let out = store
            .find_kg_nodes_by_episode_ids("a", &[])
            .await
            .expect("ok");
        assert!(out.entity_ids.is_empty());
        assert!(out.relationship_ids.is_empty());
    }

    #[tokio::test]
    async fn find_kg_nodes_by_episode_ids_matches_token_not_substring() {
        let (_tmp, graph, store) = setup_store();
        let agent = "a";
        // e1 has episode "ep-1"; e2 has "ep-10" (longer episode id that
        // would substring-match "ep-1" under a naive instr). Only e1
        // should come back when we search for "ep-1".
        insert_entity(&graph, "e1", agent, 0.8, Some("ep-1"), "current");
        insert_entity(&graph, "e2", agent, 0.8, Some("ep-10"), "current");
        insert_entity(&graph, "e3", agent, 0.8, Some("ep-2,ep-1"), "current"); // multi-token, should match
        insert_entity(&graph, "e4", agent, 0.8, Some("ep-99"), "current"); // no match

        let out = store
            .find_kg_nodes_by_episode_ids(agent, &["ep-1".to_string()])
            .await
            .expect("ok");
        let mut ids: Vec<String> = out.entity_ids.into_iter().map(|e| e.0).collect();
        ids.sort();
        assert_eq!(ids, vec!["e1".to_string(), "e3".to_string()]);
    }

    #[tokio::test]
    async fn find_kg_nodes_by_episode_ids_scopes_by_agent_and_skips_archival() {
        let (_tmp, graph, store) = setup_store();
        insert_entity(&graph, "e-mine", "a", 0.8, Some("ep-1"), "current");
        insert_entity(&graph, "e-other", "b", 0.8, Some("ep-1"), "current"); // wrong agent
        insert_entity(&graph, "e-archival", "a", 0.8, Some("ep-1"), "archival"); // archived

        let out = store
            .find_kg_nodes_by_episode_ids("a", &["ep-1".to_string()])
            .await
            .expect("ok");
        let ids: Vec<String> = out.entity_ids.into_iter().map(|e| e.0).collect();
        assert_eq!(ids, vec!["e-mine".to_string()]);
    }

    #[tokio::test]
    async fn find_kg_nodes_by_episode_ids_returns_relationships_too() {
        let (_tmp, graph, store) = setup_store();
        insert_entity(&graph, "e-src", "a", 0.8, None, "current");
        insert_entity(&graph, "e-tgt", "a", 0.8, None, "current");
        insert_relationship(
            &graph,
            "r1",
            "a",
            "e-src",
            "e-tgt",
            "RelatedTo",
            0.7,
            Some("ep-1"),
            "current",
        );
        insert_relationship(
            &graph,
            "r2",
            "a",
            "e-src",
            "e-tgt",
            "WorksFor",
            0.7,
            Some("ep-99"),
            "current",
        );

        let out = store
            .find_kg_nodes_by_episode_ids("a", &["ep-1".to_string()])
            .await
            .expect("ok");
        let ids: Vec<String> = out.relationship_ids.into_iter().map(|r| r.0).collect();
        assert_eq!(ids, vec!["r1".to_string()]);
    }

    #[tokio::test]
    async fn apply_entity_confidence_multiplier_decays_and_floors() {
        let (_tmp, graph, store) = setup_store();
        insert_entity(&graph, "e1", "a", 0.8, None, "current");
        insert_entity(&graph, "e2", "a", 0.04, None, "current");
        insert_entity(&graph, "e-other", "b", 0.8, None, "current"); // wrong agent

        let n = store
            .apply_entity_confidence_multiplier(
                "a",
                &[EntityId("e1".into()), EntityId("e2".into())],
                0.9,
                0.05,
            )
            .await
            .expect("ok");
        assert_eq!(n, 2);

        let read = |id: &str| -> f64 {
            graph
                .knowledge_db()
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT confidence FROM kg_entities WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                })
                .unwrap()
        };
        assert!((read("e1") - 0.72).abs() < 1e-6, "e1: 0.8 * 0.9 = 0.72");
        assert!((read("e2") - 0.05).abs() < 1e-6, "e2: floor at 0.05");
        assert!(
            (read("e-other") - 0.8).abs() < 1e-6,
            "other agent untouched"
        );
    }

    #[tokio::test]
    async fn apply_entity_confidence_multiplier_empty_id_list_is_noop() {
        let (_tmp, _graph, store) = setup_store();
        let n = store
            .apply_entity_confidence_multiplier("a", &[], 0.9, 0.05)
            .await
            .expect("ok");
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn apply_relationship_confidence_multiplier_decays_and_floors() {
        let (_tmp, graph, store) = setup_store();
        insert_entity(&graph, "e-src", "a", 0.8, None, "current");
        insert_entity(&graph, "e-tgt", "a", 0.8, None, "current");
        insert_relationship(
            &graph,
            "r1",
            "a",
            "e-src",
            "e-tgt",
            "RelatedTo",
            0.6,
            None,
            "current",
        );

        let n = store
            .apply_relationship_confidence_multiplier(
                "a",
                &[RelationshipId("r1".into())],
                0.5,
                0.05,
            )
            .await
            .expect("ok");
        assert_eq!(n, 1);

        let new_conf: f64 = graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT confidence FROM kg_relationships WHERE id = 'r1'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert!((new_conf - 0.3).abs() < 1e-6, "0.6 * 0.5 = 0.3");
    }

    // ============================================================================
    // MEM-001 Part B-2 — traverse_weighted trait wrapper (dedup + cap)
    // ============================================================================

    #[tokio::test]
    async fn traverse_weighted_trait_dedupes_by_entity_id_shortest_hop_wins() {
        let (_tmp, graph, store) = setup_store();
        let agent = "agent-b2-wrap";

        // Build a tiny graph where `target` is reachable in two ways:
        //   seed --(direct, conf 0.5)--> target            [hop 1, prod 0.5]
        //   seed --(conf 0.9)--> mid --(conf 0.9)--> target [hop 2, prod 0.81]
        // Despite the 2-hop path having a higher edge_confidence_product,
        // the trait wrapper must pick the hop-1 entry (shortest-hop wins).
        let now = chrono::Utc::now().to_rfc3339();
        graph
            .knowledge_db()
            .with_connection(|conn| {
                for id in ["seed", "mid", "target"] {
                    conn.execute(
                        "INSERT INTO kg_entities
                            (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                             epistemic_class, confidence, mention_count, access_count,
                             first_seen_at, last_seen_at)
                         VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, 'current', 0.8, 1, 0, ?3, ?3)",
                        rusqlite::params![id, agent, now],
                    )?;
                }
                conn.execute(
                    "INSERT INTO kg_relationships
                        (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('r-direct', ?1, 'seed', 'target', 'A', 'current', 0.5, 1, 0, ?2, ?2)",
                    rusqlite::params![agent, now],
                )?;
                conn.execute(
                    "INSERT INTO kg_relationships
                        (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('r-via-mid-a', ?1, 'seed', 'mid', 'B', 'current', 0.9, 1, 0, ?2, ?2)",
                    rusqlite::params![agent, now],
                )?;
                conn.execute(
                    "INSERT INTO kg_relationships
                        (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('r-via-mid-b', ?1, 'mid', 'target', 'C', 'current', 0.9, 1, 0, ?2, ?2)",
                    rusqlite::params![agent, now],
                )?;
                Ok(())
            })
            .unwrap();

        let hits = store
            .traverse_weighted(&EntityId("seed".into()), agent, 2, 0.0, 10)
            .await
            .expect("ok");

        let target = hits
            .iter()
            .find(|h| h.entity_id.0 == "target")
            .expect("target should be reachable");
        assert_eq!(target.hop, 1, "shortest hop wins (direct edge)");
        assert!(
            (target.edge_confidence_product - 0.5).abs() < 1e-6,
            "edge product should be 0.5 (direct), got {}",
            target.edge_confidence_product
        );
    }

    #[tokio::test]
    async fn traverse_weighted_trait_respects_limit_after_dedup() {
        let (_tmp, graph, store) = setup_store();
        let agent = "agent-b2-wrap";
        let now = chrono::Utc::now().to_rfc3339();
        graph
            .knowledge_db()
            .with_connection(|conn| {
                for id in ["seed", "n1", "n2", "n3"] {
                    conn.execute(
                        "INSERT INTO kg_entities
                            (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                             epistemic_class, confidence, mention_count, access_count,
                             first_seen_at, last_seen_at)
                         VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, 'current', 0.8, 1, 0, ?3, ?3)",
                        rusqlite::params![id, agent, now],
                    )?;
                }
                for (rid, tgt) in [("r1", "n1"), ("r2", "n2"), ("r3", "n3")] {
                    conn.execute(
                        "INSERT INTO kg_relationships
                            (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                             epistemic_class, confidence, mention_count, access_count,
                             first_seen_at, last_seen_at)
                         VALUES (?1, ?2, 'seed', ?3, 'A', 'current', 0.9, 1, 0, ?4, ?4)",
                        rusqlite::params![rid, agent, tgt, now],
                    )?;
                }
                Ok(())
            })
            .unwrap();

        let hits = store
            .traverse_weighted(&EntityId("seed".into()), agent, 1, 0.0, 2)
            .await
            .expect("ok");
        assert_eq!(hits.len(), 2, "limit must cap deduped output to 2");
    }

    // =======================================================================
    // Write-time entity-name embedding — kg_name_index population
    //
    // Regression: kg_name_index was never populated, so the hierarchy
    // builder's pool fetch (which JOINs kg_name_index) always returned 0
    // entities and bailed PoolTooSmall — the hierarchy never built past
    // layer 0. Root cause: store_knowledge never set Entity.name_embedding.
    // =======================================================================

    /// Deterministic 384-dim embedder. `fail = true` simulates an
    /// embedding-backend outage.
    struct MockEmbedder {
        fail: bool,
    }

    #[async_trait]
    impl EmbeddingClient for MockEmbedder {
        async fn embed(
            &self,
            texts: &[&str],
        ) -> Result<Vec<Vec<f32>>, agent_runtime::llm::embedding::EmbeddingError> {
            if self.fail {
                return Err(agent_runtime::llm::embedding::EmbeddingError::ModelError(
                    "mock embedding failure".into(),
                ));
            }
            Ok(texts.iter().map(|_| vec![0.05_f32; 384]).collect())
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_name(&self) -> String {
            "mock-384".to_string()
        }
    }

    fn fresh_store() -> (tempfile::TempDir, Arc<KnowledgeDatabase>, GraphStorage) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        let graph = GraphStorage::new(db.clone()).expect("graph");
        (tmp, db, graph)
    }

    #[tokio::test]
    async fn embed_entity_names_sets_embedding_on_each_entity() {
        let mut entities = vec![
            Entity::new("root".into(), EntityType::Concept, "Apple".into()),
            Entity::new("root".into(), EntityType::Concept, "Banana".into()),
        ];
        embed_entity_names(&MockEmbedder { fail: false }, &mut entities).await;
        assert!(entities[0].name_embedding.is_some());
        assert!(entities[1].name_embedding.is_some());
        assert_eq!(entities[0].name_embedding.as_ref().unwrap().len(), 384);
    }

    #[tokio::test]
    async fn embed_entity_names_empty_slice_is_noop() {
        let mut entities: Vec<Entity> = vec![];
        // Must not panic / must not call the client.
        embed_entity_names(&MockEmbedder { fail: false }, &mut entities).await;
    }

    #[tokio::test]
    async fn embed_entity_names_failure_leaves_embeddings_none() {
        let mut entities = vec![Entity::new("root".into(), EntityType::Concept, "X".into())];
        embed_entity_names(&MockEmbedder { fail: true }, &mut entities).await;
        assert!(
            entities[0].name_embedding.is_none(),
            "embed failure must leave name_embedding None — graceful, no regression"
        );
    }

    /// The load-bearing test: store_knowledge with an embedding client
    /// wired MUST land a kg_name_index row. This is the exact chain the
    /// hierarchy builder depends on.
    #[tokio::test]
    async fn store_knowledge_populates_kg_name_index() {
        let (_tmp, db, graph) = fresh_store();
        let store = SqliteKgStore::with_embedding_client(
            Arc::new(graph),
            Arc::new(MockEmbedder { fail: false }),
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![Entity::new(
                "root".into(),
                EntityType::Concept,
                "Apple Inc".into(),
            )],
            relationships: vec![],
        };
        store
            .store_knowledge("root", knowledge)
            .await
            .expect("store_knowledge");

        let n: i64 = db
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM kg_name_index", [], |r| r.get(0))
            })
            .unwrap();
        assert_eq!(
            n, 1,
            "store_knowledge with an embedding client must populate kg_name_index"
        );
    }

    #[tokio::test]
    async fn store_knowledge_without_embedding_client_leaves_index_empty() {
        // No embedding client → graceful degradation: entity is still
        // stored, kg_name_index stays empty, no error. Pre-fix behavior.
        let (_tmp, db, graph) = fresh_store();
        let store = SqliteKgStore::new(Arc::new(graph));

        let knowledge = ExtractedKnowledge {
            entities: vec![Entity::new(
                "root".into(),
                EntityType::Concept,
                "Apple Inc".into(),
            )],
            relationships: vec![],
        };
        store
            .store_knowledge("root", knowledge)
            .await
            .expect("store_knowledge must still succeed without an embedding client");

        let n: i64 = db
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM kg_name_index", [], |r| r.get(0))
            })
            .unwrap();
        assert_eq!(n, 0, "no embedding client → no kg_name_index row, no error");
    }
}
