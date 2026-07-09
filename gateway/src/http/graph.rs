//! # Knowledge Graph Endpoints
//!
//! HTTP API for querying the knowledge graph.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use knowledge_graph::{Direction, Entity, GraphStats, Relationship, Subgraph};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use zbot_engram_adapter::GovernanceCapabilityHealth;
use zbot_stores::{Direction as StoreDirection, KnowledgeGraphStore};
use zbot_stores_domain::{DistillationStats, UndistilledSession};

// ============================================================================
// REQUEST/RESPONSE TYPES
// ============================================================================

/// Query parameters for listing entities.
#[derive(Debug, Deserialize)]
pub struct EntityListQuery {
    /// Filter by entity type (e.g., "person", "tool", "project")
    pub entity_type: Option<String>,
    /// Maximum number of results
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset for pagination
    #[serde(default)]
    pub offset: usize,
}

/// Query parameters for listing relationships.
#[derive(Debug, Deserialize)]
pub struct RelationshipListQuery {
    /// Filter by relationship type (e.g., "uses", "created", "part_of")
    pub relationship_type: Option<String>,
    /// Maximum number of results
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset for pagination
    #[serde(default)]
    pub offset: usize,
}

/// Query parameters for neighbor queries.
#[derive(Debug, Deserialize)]
pub struct NeighborQuery {
    /// Direction of relationships to follow
    #[serde(default)]
    pub direction: Option<String>,
    /// Maximum number of neighbors
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// Query parameters for subgraph queries.
#[derive(Debug, Deserialize)]
pub struct SubgraphQuery {
    /// Maximum number of hops from center entity
    #[serde(default = "default_hops")]
    pub max_hops: usize,
}

fn default_limit() -> usize {
    50
}

fn default_hops() -> usize {
    2
}

// ============================================================================
// RESPONSE TYPES
// ============================================================================

/// Entity response for API.
#[derive(Debug, Serialize)]
pub struct EntityResponse {
    pub id: String,
    pub agent_id: String,
    pub entity_type: String,
    pub name: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub mention_count: i64,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

impl From<Entity> for EntityResponse {
    fn from(entity: Entity) -> Self {
        Self {
            id: entity.id,
            agent_id: entity.agent_id,
            entity_type: entity.entity_type.as_str().to_string(),
            name: entity.name,
            properties: entity.properties,
            mention_count: entity.mention_count,
            first_seen_at: entity.first_seen_at.to_rfc3339(),
            last_seen_at: entity.last_seen_at.to_rfc3339(),
        }
    }
}

/// Relationship response for API.
#[derive(Debug, Serialize)]
pub struct RelationshipResponse {
    pub id: String,
    pub agent_id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub relationship_type: String,
    pub mention_count: i64,
}

impl From<Relationship> for RelationshipResponse {
    fn from(rel: Relationship) -> Self {
        Self {
            id: rel.id,
            agent_id: rel.agent_id,
            source_entity_id: rel.source_entity_id,
            target_entity_id: rel.target_entity_id,
            relationship_type: rel.relationship_type.as_str().to_string(),
            mention_count: rel.mention_count,
        }
    }
}

/// Graph statistics response.
#[derive(Debug, Serialize)]
pub struct GraphStatsResponse {
    pub entity_count: usize,
    pub relationship_count: usize,
    pub entity_types: HashMap<String, usize>,
    pub relationship_types: HashMap<String, usize>,
    pub most_connected_entities: Vec<(String, usize)>,
}

impl From<GraphStats> for GraphStatsResponse {
    fn from(stats: GraphStats) -> Self {
        Self {
            entity_count: stats.entity_count,
            relationship_count: stats.relationship_count,
            entity_types: stats.entity_types,
            relationship_types: stats.relationship_types,
            most_connected_entities: stats.most_connected_entities,
        }
    }
}

/// Entity list response.
#[derive(Debug, Serialize)]
pub struct EntityListResponse {
    pub entities: Vec<EntityResponse>,
    pub total: usize,
}

/// Relationship list response.
#[derive(Debug, Serialize)]
pub struct RelationshipListResponse {
    pub relationships: Vec<RelationshipResponse>,
    pub total: usize,
}

/// Neighbor response.
#[derive(Debug, Serialize)]
pub struct NeighborResponse {
    pub entity_id: String,
    pub neighbors: Vec<NeighborEntry>,
}

/// Single neighbor entry.
#[derive(Debug, Serialize)]
pub struct NeighborEntry {
    pub entity: EntityResponse,
    pub relationship: RelationshipResponse,
    pub direction: String,
}

/// Subgraph response.
#[derive(Debug, Serialize)]
pub struct SubgraphResponse {
    pub entities: Vec<EntityResponse>,
    pub relationships: Vec<RelationshipResponse>,
    pub center: String,
    pub max_hops: usize,
}

impl From<Subgraph> for SubgraphResponse {
    fn from(subgraph: Subgraph) -> Self {
        Self {
            entities: subgraph
                .entities
                .into_iter()
                .map(EntityResponse::from)
                .collect(),
            relationships: subgraph
                .relationships
                .into_iter()
                .map(RelationshipResponse::from)
                .collect(),
            center: subgraph.center,
            max_hops: subgraph.max_hops,
        }
    }
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// Strip the `agent:` prefix from UI-supplied agent ids so the graph service
/// (which stores the bare name) finds rows. Both forms are accepted.
fn normalize_agent_id(id: &str) -> &str {
    id.strip_prefix("agent:").unwrap_or(id)
}

/// GET /api/graph/:agent_id/stats
/// Get graph statistics for an agent.
pub async fn get_graph_stats(
    Path(agent_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<GraphStatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    kg_store
        .graph_stats(normalize_agent_id(&agent_id))
        .await
        .map(|stats| Json(GraphStatsResponse::from(stats)))
        .map_err(store_err_to_http)
}

/// GET /api/graph/:agent_id/entities
/// List entities for an agent.
pub async fn list_entities(
    Path(agent_id): Path<String>,
    Query(query): Query<EntityListQuery>,
    State(state): State<AppState>,
) -> Result<Json<EntityListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    let entities = kg_store
        .list_entities(
            normalize_agent_id(&agent_id),
            query.entity_type.as_deref(),
            query.limit,
            query.offset,
        )
        .await
        .map_err(store_err_to_http)?;
    let total = entities.len();
    Ok(Json(EntityListResponse {
        entities: entities.into_iter().map(EntityResponse::from).collect(),
        total,
    }))
}

/// GET /api/graph/:agent_id/relationships
/// List relationships for an agent.
pub async fn list_relationships(
    Path(agent_id): Path<String>,
    Query(query): Query<RelationshipListQuery>,
    State(state): State<AppState>,
) -> Result<Json<RelationshipListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    let relationships = kg_store
        .list_relationships(
            normalize_agent_id(&agent_id),
            query.relationship_type.as_deref(),
            query.limit,
            query.offset,
        )
        .await
        .map_err(store_err_to_http)?;
    let total = relationships.len();
    Ok(Json(RelationshipListResponse {
        relationships: relationships
            .into_iter()
            .map(RelationshipResponse::from)
            .collect(),
        total,
    }))
}

/// GET /api/graph/:agent_id/entities/:entity_id/neighbors
/// Get neighbors of an entity.
pub async fn get_entity_neighbors(
    Path((agent_id, entity_id)): Path<(String, String)>,
    Query(query): Query<NeighborQuery>,
    State(state): State<AppState>,
) -> Result<Json<NeighborResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    let direction = parse_direction(query.direction.as_deref());
    let neighbors = kg_store
        .get_neighbors_full(
            normalize_agent_id(&agent_id),
            &entity_id,
            direction,
            query.limit,
        )
        .await
        .map_err(store_err_to_http)?;
    let neighbor_entries: Vec<NeighborEntry> = neighbors
        .into_iter()
        .map(|n| NeighborEntry {
            entity: EntityResponse::from(n.entity),
            relationship: RelationshipResponse::from(n.relationship),
            direction: match n.direction {
                Direction::Outgoing => "outgoing".to_string(),
                Direction::Incoming => "incoming".to_string(),
                Direction::Both => "both".to_string(),
            },
        })
        .collect();
    Ok(Json(NeighborResponse {
        entity_id,
        neighbors: neighbor_entries,
    }))
}

/// Parse the optional `?direction=` query string into the trait-side
/// `StoreDirection`. Default = `Both`. Unknown values fall back to
/// `Both` to match the historical handler behavior.
fn parse_direction(s: Option<&str>) -> StoreDirection {
    match s {
        Some("outgoing") => StoreDirection::Outgoing,
        Some("incoming") => StoreDirection::Incoming,
        _ => StoreDirection::Both,
    }
}

/// GET /api/graph/:agent_id/entities/:entity_id/subgraph
/// Get subgraph around an entity.
pub async fn get_entity_subgraph(
    Path((agent_id, entity_id)): Path<(String, String)>,
    Query(query): Query<SubgraphQuery>,
    State(state): State<AppState>,
) -> Result<Json<SubgraphResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    kg_store
        .get_subgraph(normalize_agent_id(&agent_id), &entity_id, query.max_hops)
        .await
        .map(|subgraph| Json(SubgraphResponse::from(subgraph)))
        .map_err(store_err_to_http)
}

/// GET /api/graph/:agent_id/search
/// Search entities by name.
///
/// Backed by `kg_store` (KnowledgeGraphStore trait). Response shape
/// is identical to the historical handler.
pub async fn search_entities(
    Path(agent_id): Path<String>,
    Query(query): Query<SearchQuery>,
    State(state): State<AppState>,
) -> Result<Json<EntityListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    let entities = kg_store
        .search_entities_by_name(
            normalize_agent_id(&agent_id),
            &query.q,
            query.limit.unwrap_or(20),
        )
        .await
        .map_err(store_err_to_http)?;
    let total = entities.len();
    Ok(Json(EntityListResponse {
        entities: entities.into_iter().map(EntityResponse::from).collect(),
        total,
    }))
}

/// Resolve `state.kg_store` or short-circuit with 503. Centralised so
/// every graph handler emits the same payload when the trait-erased
/// store hasn't been wired (smoke tests, partial init).
fn require_kg_store(
    state: &AppState,
) -> Result<Arc<dyn KnowledgeGraphStore>, (StatusCode, Json<ErrorResponse>)> {
    state.kg_store.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Knowledge graph store unavailable".to_string(),
            }),
        )
    })
}

/// Map a [`zbot_stores::StoreError`] to the HTTP error pair used by graph
/// handlers: `(StatusCode, Json<ErrorResponse>)`.
fn store_err_to_http(err: zbot_stores::StoreError) -> (StatusCode, Json<ErrorResponse>) {
    use zbot_stores::StoreError;
    match err {
        StoreError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Entity not found".to_string(),
            }),
        ),
        StoreError::Conflict(msg) => (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: format!("Conflict: {}", msg),
            }),
        ),
        StoreError::Invalid(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid request: {}", msg),
            }),
        ),
        StoreError::Unavailable { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Knowledge graph store temporarily unavailable".to_string(),
            }),
        ),
        StoreError::Schema(msg) | StoreError::Backend(msg) | StoreError::Config(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Knowledge graph error: {}", msg),
            }),
        ),
    }
}

/// Query parameters for entity search.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Search query string
    pub q: String,
    /// Maximum number of results
    pub limit: Option<usize>,
}

/// Query parameters for cross-agent entity listing.
#[derive(Debug, Deserialize)]
pub struct AllEntitiesQuery {
    /// Filter by ward/agent ID
    pub ward_id: Option<String>,
    /// Filter by entity type
    pub entity_type: Option<String>,
    /// Maximum number of results
    #[serde(default = "default_all_entities_limit")]
    pub limit: usize,
}

fn default_all_entities_limit() -> usize {
    200
}

/// Aggregate graph statistics for the Observatory health bar.
#[derive(Debug, Serialize)]
pub struct AggregateGraphStats {
    pub entities: usize,
    pub relationships: usize,
    pub facts: usize,
    pub episodes: i64,
    pub distillation: Option<DistillationStats>,
    pub governance: Option<GovernanceCapabilityHealth>,
}

// ============================================================================
// DISTILLATION STATUS
// ============================================================================

/// GET /api/distillation/status
/// Get aggregate distillation statistics.
///
/// Returns zeroed stats when `distillation_repo` is unavailable
/// (stripped-down test fixtures) so the Observatory health bar
/// renders cleanly.
pub async fn distillation_status(
    State(state): State<AppState>,
) -> Result<Json<DistillationStats>, (StatusCode, Json<ErrorResponse>)> {
    let repo = match &state.distillation_repo {
        Some(repo) => repo,
        None => return Ok(Json(DistillationStats::default())),
    };

    match repo.get_stats() {
        Ok(stats) => Ok(Json(stats)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get distillation stats: {}", e),
            }),
        )),
    }
}

/// GET /api/distillation/undistilled
/// Returns undistilled sessions (session_id + agent_id pairs).
///
/// Returns an empty list when `distillation_repo` is unavailable.
pub async fn undistilled_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<UndistilledSession>>, (StatusCode, Json<ErrorResponse>)> {
    let repo = match &state.distillation_repo {
        Some(repo) => repo,
        None => return Ok(Json(Vec::new())),
    };

    match repo.get_undistilled_sessions() {
        Ok(sessions) => Ok(Json(sessions)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get undistilled sessions: {}", e),
            }),
        )),
    }
}

/// Response for the trigger distillation endpoint.
#[derive(Debug, Serialize)]
pub struct TriggerDistillationResponse {
    pub session_id: String,
    pub status: String,
    pub facts_upserted: usize,
    pub error: Option<String>,
}

/// POST /api/distillation/trigger/:session_id
/// Trigger distillation for a specific session.
pub async fn trigger_distillation(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<TriggerDistillationResponse>, (StatusCode, Json<ErrorResponse>)> {
    let distiller = match &state.distiller {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Distillation service not available".to_string(),
                }),
            ));
        }
    };

    // Look up the root_agent_id for this session from the database
    let agent_id = match state.session_meta.session_agent_id(&session_id) {
        Ok(Some(aid)) => aid,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Session '{}' not found", session_id),
                }),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to look up session: {}", e),
                }),
            ));
        }
    };

    match distiller.distill(&session_id, &agent_id).await {
        Ok(facts_upserted) => Ok(Json(TriggerDistillationResponse {
            session_id,
            status: "success".to_string(),
            facts_upserted,
            error: None,
        })),
        Err(e) => Ok(Json(TriggerDistillationResponse {
            session_id,
            status: "failed".to_string(),
            facts_upserted: 0,
            error: Some(e),
        })),
    }
}

// ============================================================================
// OBSERVATORY ENDPOINTS
// ============================================================================

/// GET /api/graph/stats
/// Aggregate graph statistics for the Observatory health bar.
///
/// Counts come from trait-erased stores where possible: `kg_store`
/// for entity/relationship counts and `memory_store` for fact count.
/// `episode_repo` and `distillation_repo` remain on their concrete
/// repos — neither has been migrated to a `zbot-stores` trait yet.
pub async fn graph_stats(
    State(state): State<AppState>,
) -> Result<Json<AggregateGraphStats>, (StatusCode, Json<ErrorResponse>)> {
    // Entity + relationship counts from kg_store.
    let (entities, relationships) = match &state.kg_store {
        Some(store) => {
            let e = store.count_all_entities().await.unwrap_or(0);
            let r = store.count_all_relationships().await.unwrap_or(0);
            (e, r)
        }
        None => (0, 0),
    };

    // Fact count from memory_store.
    let facts = match &state.memory_store {
        Some(store) => store
            .count_all_facts(None)
            .await
            .ok()
            .map(|n| n as usize)
            .unwrap_or(0),
        None => 0,
    };

    // Episode count: prefer the trait surface; fall back to the SQLite
    // repo when only that is available (legacy / minimal AppStates).
    let episodes = match (&state.episode_store, &state.episode_repo) {
        (Some(store), _) => store.episode_stats().await.map(|s| s.total).unwrap_or(0),
        (None, Some(repo)) => repo.count().unwrap_or(0),
        (None, None) => 0,
    };

    // Distillation stats
    let distillation = match &state.distillation_repo {
        Some(repo) => repo.get_stats().ok(),
        None => None,
    };

    Ok(Json(AggregateGraphStats {
        entities,
        relationships,
        facts,
        episodes,
        distillation,
        governance: state.governance_health.clone(),
    }))
}

/// GET /api/graph/all/relationships
/// Cross-agent relationship listing for the Observatory "All Agents" mode.
pub async fn all_relationships(
    Query(query): Query<AllEntitiesQuery>,
    State(state): State<AppState>,
) -> Result<Json<RelationshipListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    let relationships = kg_store
        .list_all_relationships(query.limit)
        .await
        .map_err(store_err_to_http)?;
    let total = relationships.len();
    Ok(Json(RelationshipListResponse {
        relationships: relationships
            .into_iter()
            .map(RelationshipResponse::from)
            .collect(),
        total,
    }))
}

/// GET /api/graph/all/entities
/// Cross-agent entity listing for the Observatory "All Agents" mode.
pub async fn all_entities(
    Query(query): Query<AllEntitiesQuery>,
    State(state): State<AppState>,
) -> Result<Json<EntityListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let kg_store = require_kg_store(&state)?;
    let entities = kg_store
        .list_all_entities(
            query.ward_id.as_deref(),
            query.entity_type.as_deref(),
            query.limit,
        )
        .await
        .map_err(store_err_to_http)?;
    let total = entities.len();
    Ok(Json(EntityListResponse {
        entities: entities.into_iter().map(EntityResponse::from).collect(),
        total,
    }))
}

/// Response body for the reindex endpoint.
#[derive(Debug, Serialize)]
pub struct ReindexResponse {
    pub wards_processed: usize,
    pub entities_created: usize,
}

/// POST /api/graph/reindex — force re-indexing of every ward on disk.
/// Idempotent: relationships upsert via UNIQUE(source, target, type).
///
/// Trait-routed via `state.kg_episode_store` + `state.kg_store`.
/// Returns 503 only when neither trait is wired (defensive —
/// production always has both).
pub async fn reindex_all_wards(
    State(state): State<AppState>,
) -> Result<Json<ReindexResponse>, StatusCode> {
    use gateway_execution::ward_artifact_indexer::{index_ward_with_options, IndexOptions};

    let episode_store = state
        .kg_episode_store
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let kg_store = state
        .kg_store
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let wards_dir = state.paths.wards_dir();
    let Ok(read) = std::fs::read_dir(&wards_dir) else {
        return Ok(Json(ReindexResponse {
            wards_processed: 0,
            entities_created: 0,
        }));
    };

    let mut total_entities = 0_usize;
    let mut wards_processed = 0_usize;
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let n = index_ward_with_options(
            &path,
            "admin-reindex",
            "root",
            &episode_store,
            &kg_store,
            IndexOptions {
                force_reindex: true,
            },
        )
        .await;
        total_entities += n;
        wards_processed += 1;
    }

    Ok(Json(ReindexResponse {
        wards_processed,
        entities_created: total_entities,
    }))
}
