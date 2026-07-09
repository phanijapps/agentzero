//! # KG Store Adapter (trait-routed)
//!
//! Bridges `Arc<dyn zbot_stores::KnowledgeGraphStore>` to
//! `agent_tools::GraphStorageAccess` so the `GraphQueryTool` can query
//! the knowledge graph regardless of backend (the configured backend).
//!
//! The earlier [`super::graph_adapter::GraphStorageAdapter`] only knew
//! about the concrete `GraphStorage` (SQLite).
//! `state.graph_storage` is `None`, so that adapter could not be wired
//! and subagents lost the `graph_query` tool entirely. This adapter
//! consumes the trait-routed `state.kg_store` (wired by AppState)
//! so the tool registers regardless of which DB is selected.

use std::sync::Arc;

use agent_tools::{EntityInfo, GraphStorageAccess, NeighborInfo};
use async_trait::async_trait;
use knowledge_graph::{Direction, Entity, Relationship};
use zbot_stores::{EntityId, KnowledgeGraphStore};

/// Map `knowledge_graph::Entity` to the tool-facing [`EntityInfo`]
/// without dropping fields. Mirror of the helper in
/// `graph_adapter.rs` so the tool sees identical shapes regardless
/// of which backend produced the entity.
fn entity_to_info(e: Entity) -> EntityInfo {
    let properties = serde_json::to_value(&e.properties).unwrap_or_else(|_| serde_json::json!({}));
    EntityInfo {
        id: e.id,
        name: e.name,
        entity_type: e.entity_type.as_str().to_string(),
        mention_count: e.mention_count,
        properties,
        first_seen_at: e.first_seen_at.to_rfc3339(),
        last_seen_at: e.last_seen_at.to_rfc3339(),
    }
}

fn neighbor_to_info(
    entity: Entity,
    relationship: Relationship,
    direction: Direction,
) -> NeighborInfo {
    let dir_str = match direction {
        Direction::Outgoing => "outgoing",
        Direction::Incoming => "incoming",
        Direction::Both => "both",
    };
    let rel_properties =
        serde_json::to_value(&relationship.properties).unwrap_or_else(|_| serde_json::json!({}));
    NeighborInfo {
        entity: entity_to_info(entity),
        relationship_type: relationship.relationship_type.as_str().to_string(),
        direction: dir_str.to_string(),
        rel_properties,
        rel_first_seen_at: relationship.first_seen_at.to_rfc3339(),
        rel_last_seen_at: relationship.last_seen_at.to_rfc3339(),
    }
}

/// Adapter that implements [`GraphStorageAccess`] by delegating to a
/// trait-object [`KnowledgeGraphStore`]. Both SQLite (and any future alternate backend)
/// backends are addressed uniformly.
pub struct KgStoreAdapter {
    store: Arc<dyn KnowledgeGraphStore>,
}

impl KgStoreAdapter {
    pub fn new(store: Arc<dyn KnowledgeGraphStore>) -> Self {
        Self { store }
    }
}

/// Same agent-id convention as `GraphStorageAdapter` — `__global__`
/// sees all entities. The tool was historically scoped this way; we
/// preserve it so the tool's behavior is identical across adapters.
const GLOBAL_AGENT_ID: &str = "__global__";

#[async_trait]
impl GraphStorageAccess for KgStoreAdapter {
    async fn search_entities_by_name(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityInfo>, String> {
        let entities = self
            .store
            .search_entities_by_name(GLOBAL_AGENT_ID, query, limit.saturating_mul(2))
            .await
            .map_err(|e| format!("graph search failed: {e}"))?;

        Ok(entities
            .into_iter()
            .filter(|e| {
                entity_type
                    .map(|t| e.entity_type.as_str() == t)
                    .unwrap_or(true)
            })
            .take(limit)
            .map(entity_to_info)
            .collect())
    }

    async fn get_entity_neighbors(
        &self,
        entity_name: &str,
        direction: &str,
        limit: usize,
    ) -> Result<Vec<NeighborInfo>, String> {
        // Resolve entity name -> id via search (limit=1 with case-insensitive
        // exact match for parity with the SQLite adapter's `get_entity_by_name`).
        let needle = entity_name.to_lowercase();
        let candidates = self
            .store
            .search_entities_by_name(GLOBAL_AGENT_ID, entity_name, 5)
            .await
            .map_err(|e| format!("entity lookup failed: {e}"))?;
        let Some(entity) = candidates
            .into_iter()
            .find(|e| e.name.to_lowercase() == needle)
        else {
            return Ok(vec![]);
        };

        let dir = match direction {
            "outgoing" => Direction::Outgoing,
            "incoming" => Direction::Incoming,
            _ => Direction::Both,
        };

        // Trait-side `Direction` mirrors `knowledge_graph::Direction`.
        let trait_dir = match dir {
            Direction::Outgoing => zbot_stores::types::Direction::Outgoing,
            Direction::Incoming => zbot_stores::types::Direction::Incoming,
            Direction::Both => zbot_stores::types::Direction::Both,
        };

        let neighbors = self
            .store
            .get_neighbors_full(GLOBAL_AGENT_ID, &entity.id, trait_dir, limit)
            .await
            .map_err(|e| format!("neighbor query failed: {e}"))?;

        Ok(neighbors
            .into_iter()
            .map(|n| neighbor_to_info(n.entity, n.relationship, n.direction))
            .collect())
    }

    async fn get_entity_by_name(&self, name: &str) -> Result<Option<EntityInfo>, String> {
        let needle = name.to_lowercase();
        let candidates = self
            .store
            .search_entities_by_name(GLOBAL_AGENT_ID, name, 5)
            .await
            .map_err(|e| format!("entity lookup failed: {e}"))?;
        Ok(candidates
            .into_iter()
            .find(|e| e.name.to_lowercase() == needle)
            .map(entity_to_info))
    }
}

// Suppress dead_code on `EntityId` import — kept for symmetry with the
// SQLite adapter's API surface; future callers may need it.
#[allow(dead_code)]
const _: fn(EntityId) = |_| {};
