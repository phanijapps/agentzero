//! # Memory Endpoints
//!
//! CRUD and search operations for agent memory facts. All handlers
//! route through `state.memory_store` (and `state.kg_store` for the
//! entity/relationship part of `stats`) so the underlying backend is
//! abstracted from the HTTP surface.

use super::ErrorResponse;
use crate::state::AppState;
use agent_runtime::llm::embedding::EmbeddingClient;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use zbot_engram_adapter::GovernanceCapabilityHealth;
use zbot_stores_domain::MemoryFact;
use zbot_stores_traits::EmbeddingQueryIdentity;

// ============================================================================
// REQUEST/RESPONSE TYPES
// ============================================================================

/// Memory fact response for API.
#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryFactResponse {
    pub id: String,
    pub agent_id: String,
    pub scope: String,
    pub category: String,
    pub key: String,
    pub content: String,
    pub confidence: f64,
    pub mention_count: i32,
    pub source_summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Which search arm matched this fact: `"fts"`, `"vec"`, or `"hybrid"`.
    /// Only populated on search responses; `None` elsewhere and omitted from JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_source: Option<String>,
}

impl From<MemoryFact> for MemoryFactResponse {
    fn from(fact: MemoryFact) -> Self {
        Self {
            id: fact.id,
            agent_id: fact.agent_id,
            scope: fact.scope,
            category: fact.category,
            key: fact.key,
            content: fact.content,
            confidence: fact.confidence,
            mention_count: fact.mention_count,
            source_summary: fact.source_summary,
            created_at: fact.created_at,
            updated_at: fact.updated_at,
            match_source: None,
        }
    }
}

fn is_public_memory_fact(fact: &MemoryFactResponse) -> bool {
    !matches!(fact.category.as_str(), "ctx" | "instruction" | "correction")
}

fn validate_public_fact_input(
    category: &str,
    key: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if matches!(category, "ctx" | "instruction" | "correction") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(format!(
                "category '{category}' is internal-only"
            ))),
        ));
    }
    if key.is_empty()
        || key.len() > 200
        || key
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/')))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "key must contain only ASCII letters, numbers, '.', '_', '-', ':', or '/'",
            )),
        ));
    }
    Ok(())
}

fn public_internal_error(
    log_context: &str,
    public_message: &str,
    e: impl std::fmt::Display,
) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!("{}: {}", log_context, e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(public_message.to_string())),
    )
}

/// Query parameters for listing memory facts.
#[derive(Debug, Deserialize)]
pub struct MemoryListQuery {
    pub category: Option<String>,
    pub scope: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

/// Query parameters for listing ALL memory facts (across all agents).
#[derive(Debug, Deserialize)]
pub struct AllMemoryListQuery {
    pub agent_id: Option<String>,
    pub category: Option<String>,
    pub scope: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

/// Query parameters for searching memory facts.
#[derive(Debug, Deserialize)]
pub struct MemorySearchQuery {
    pub q: String,
    pub category: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// `"hybrid"` (default), `"fts"`, or `"semantic"`.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub ward_id: Option<String>,
}

fn default_search_limit() -> usize {
    20
}

/// Response for list operations with pagination info.
#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub facts: Vec<MemoryFactResponse>,
    pub total: usize,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// GET /api/memory/:agent_id - List memory facts for an agent.
pub async fn list_memory_facts(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<MemoryListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Routed through the trait surface.
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    let raw_facts = memory_store
        .list_memory_facts(
            Some(&agent_id),
            query.category.as_deref(),
            query.scope.as_deref(),
            query.limit,
            query.offset,
        )
        .await
        .map_err(|e| {
            public_internal_error(
                "Failed to list memory facts",
                "Failed to list memory facts",
                e,
            )
        })?;

    let facts: Vec<MemoryFactResponse> = raw_facts
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<MemoryFactResponse>(v) {
            Ok(f) => Some(f),
            Err(e) => {
                tracing::warn!("memory fact row decode failed: {e}");
                None
            }
        })
        .filter(is_public_memory_fact)
        .collect();
    let total = facts.len();

    Ok(Json(MemoryListResponse { facts, total }))
}

/// GET /api/memory/:agent_id/search - Search memory facts.
pub async fn search_memory_facts(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<MemorySearchQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    let mode = query.mode.as_deref().unwrap_or("hybrid");
    let ward_id = query.ward_id.as_deref();
    let scope_agent: Option<&str> = Some(agent_id.as_str());
    let embedding_client = state.embedding_service().client();

    // For semantic + hybrid we need an embedding of the query text. Fall
    // through to FTS-only on hybrid if the embedding backend is down;
    // bubble the error as 400 for explicit semantic requests.
    let qe_opt: Option<Vec<f32>> = match mode {
        "fts" => None,
        "semantic" => {
            let emb = embedding_client
                .embed(&[query.q.as_str()])
                .await
                .map_err(|e| {
                    tracing::debug!("semantic memory search embedding unavailable: {e}");
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse::new(
                            "Embedding backend unavailable".to_string(),
                        )),
                    )
                })?;
            emb.into_iter().next()
        }
        _ => match embedding_client.embed(&[query.q.as_str()]).await {
            Ok(v) => v.into_iter().next(),
            Err(e) => {
                tracing::debug!("hybrid search: embedding unavailable ({e}); FTS-only");
                None
            }
        },
    };
    let query_identity = qe_opt
        .as_ref()
        .map(|_| embedding_query_identity(embedding_client.as_ref()));

    let raw_rows = memory_store
        .search_memory_facts_hybrid_with_identity(
            scope_agent,
            &query.q,
            mode,
            query.limit,
            ward_id,
            qe_opt.as_deref(),
            query_identity.as_ref(),
            None, // as_of — HTTP search defaults to "now"; point-in-time is exposed via the agent tool
        )
        .await
        .map_err(|e| search_err("Failed to search memory facts", e))?;

    // Decode rows + apply optional category filter (the trait method doesn't
    // filter by category — we do it client-side here so the trait stays simple).
    let facts: Vec<MemoryFactResponse> = raw_rows
        .into_iter()
        .filter_map(|v| serde_json::from_value::<MemoryFactResponse>(v).ok())
        .map(normalize_public_match_source)
        .filter(is_public_memory_fact)
        .filter(|f| {
            query
                .category
                .as_ref()
                .map(|c| f.category == *c)
                .unwrap_or(true)
        })
        .collect();

    let total = facts.len();
    Ok(Json(MemoryListResponse { facts, total }))
}

fn normalize_public_match_source(mut fact: MemoryFactResponse) -> MemoryFactResponse {
    if fact.match_source.as_deref() == Some("exact_degraded") {
        fact.match_source = Some("fts".to_string());
    }
    fact
}

fn search_err(context: &str, e: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    public_internal_error(context, context, e)
}

fn embedding_query_identity(client: &dyn EmbeddingClient) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: client.provider_type(),
        model: client.model_name(),
        dimensions: client.dimensions() as u32,
        prompt_profile: client.prompt_profile(),
        normalization: client.normalization(),
    }
}

/// GET /api/memory/:agent_id/facts/:fact_id - Get a single memory fact.
pub async fn get_memory_fact(
    State(state): State<AppState>,
    Path((agent_id, fact_id)): Path<(String, String)>,
) -> Result<Json<MemoryFactResponse>, (StatusCode, Json<ErrorResponse>)> {
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    let raw = memory_store
        .get_memory_fact_by_id(&fact_id)
        .await
        .map_err(|e| {
            public_internal_error("Failed to get memory fact", "Failed to get memory fact", e)
        })?;

    let fact: Option<MemoryFactResponse> =
        raw.and_then(|v| serde_json::from_value::<MemoryFactResponse>(v).ok());

    match fact {
        Some(f) if f.agent_id == agent_id && !is_public_memory_fact(&f) => Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "Fact is not available through public memory endpoints".to_string(),
            )),
        )),
        Some(f) if f.agent_id == agent_id => Ok(Json(f)),
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "Fact does not belong to this agent".to_string(),
            )),
        )),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Memory fact not found".to_string())),
        )),
    }
}

/// DELETE /api/memory/:agent_id/facts/:fact_id - Delete a memory fact.
pub async fn delete_memory_fact(
    State(state): State<AppState>,
    Path((agent_id, fact_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    // First verify the fact belongs to this agent
    let raw = memory_store
        .get_memory_fact_by_id(&fact_id)
        .await
        .map_err(|e| {
            public_internal_error(
                "Failed to get memory fact for deletion",
                "Failed to get memory fact",
                e,
            )
        })?;
    let fact: Option<MemoryFactResponse> =
        raw.and_then(|v| serde_json::from_value::<MemoryFactResponse>(v).ok());

    match fact {
        Some(f) if f.agent_id == agent_id && !is_public_memory_fact(&f) => Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "Fact is not available through public memory endpoints".to_string(),
            )),
        )),
        Some(f) if f.agent_id == agent_id => {
            let deleted = memory_store
                .delete_memory_fact(&fact_id)
                .await
                .map_err(|e| {
                    public_internal_error(
                        "Failed to delete memory fact",
                        "Failed to delete memory fact",
                        e,
                    )
                })?;

            if deleted {
                Ok(StatusCode::NO_CONTENT)
            } else {
                Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse::new("Memory fact not found".to_string())),
                ))
            }
        }
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "Fact does not belong to this agent".to_string(),
            )),
        )),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Memory fact not found".to_string())),
        )),
    }
}

/// POST /api/memory/:agent_id — Create a new memory fact (policy, instruction, or about-me).
#[derive(Debug, Deserialize)]
pub struct CreateMemoryFactRequest {
    pub category: String,
    pub key: String,
    pub content: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub ward_id: Option<String>,
    #[serde(default = "default_true")]
    pub pinned: bool,
}

fn default_confidence() -> f64 {
    1.0
}
fn default_true() -> bool {
    true
}

pub async fn create_memory_fact(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(request): Json<CreateMemoryFactRequest>,
) -> Result<(StatusCode, Json<MemoryFactResponse>), (StatusCode, Json<ErrorResponse>)> {
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    validate_public_fact_input(&request.category, &request.key)?;

    let now = chrono::Utc::now().to_rfc3339();
    let fact = MemoryFact {
        id: format!("fact-{}", uuid::Uuid::new_v4()),
        session_id: None,
        agent_id: agent_id.clone(),
        scope: "agent".to_string(),
        category: request.category.clone(),
        key: request.key.clone(),
        content: request.content.clone(),
        confidence: request.confidence,
        mention_count: 5,
        source_summary: Some("User-created via UI".to_string()),
        embedding: None,
        ward_id: request.ward_id.unwrap_or_else(|| "__global__".to_string()),
        contradicted_by: None,
        created_at: now.clone(),
        updated_at: now,
        expires_at: None,
        valid_from: None,
        valid_until: None,
        superseded_by: None,
        pinned: request.pinned,
        epistemic_class: Some("current".to_string()),
        source_episode_id: None,
        source_ref: None,
        last_accessed: None,
    };

    if request.pinned && request.category == "user" && request.key == "user.profile" {
        let existing_profiles = memory_store
            .list_memory_facts(Some(&agent_id), Some("user"), Some("agent"), 100, 0)
            .await
            .map_err(|e| {
                public_internal_error(
                    "Failed to list existing user profile facts",
                    "Failed to create fact",
                    e,
                )
            })?;

        for existing in existing_profiles {
            let existing: MemoryFact = match serde_json::from_value(existing) {
                Ok(fact) => fact,
                Err(e) => {
                    tracing::warn!(
                        "memory fact row decode failed while replacing user profile: {e}"
                    );
                    continue;
                }
            };

            if existing.key == "user.profile" && existing.ward_id == fact.ward_id {
                memory_store
                    .delete_memory_fact(&existing.id)
                    .await
                    .map_err(|e| {
                        public_internal_error(
                            "Failed to replace existing user profile fact",
                            "Failed to create fact",
                            e,
                        )
                    })?;
            }
        }
    }

    memory_store
        .upsert_typed_fact(fact.clone(), None)
        .await
        .map_err(|e| public_internal_error("Failed to create fact", "Failed to create fact", e))?;

    Ok((StatusCode::CREATED, Json(MemoryFactResponse::from(fact))))
}

/// GET /api/memory/search — Search ALL memory facts across all agents (server-side FTS5).
#[derive(Debug, Deserialize)]
pub struct GlobalMemorySearchQuery {
    pub q: String,
    #[serde(default = "default_global_search_limit")]
    pub limit: usize,
    pub category: Option<String>,
}

fn default_global_search_limit() -> usize {
    50
}

pub async fn search_all_memory_facts(
    State(state): State<AppState>,
    Query(query): Query<GlobalMemorySearchQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Routed through the trait surface so the underlying backend is
    // honored when opted in. Defaults to the FTS arm — the historical
    // handler was FTS-only and the trait's `mode = "fts"` matches.
    // The trait method does not accept a category filter; for now we
    // post-filter on the deserialized Value rows. Migrating the
    // category filter into the trait surface is a follow-up.
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    let raw = memory_store
        .search_memory_facts_hybrid(None, &query.q, "fts", query.limit, None, None, None)
        .await
        .map_err(|e| public_internal_error("Global memory search failed", "Search failed", e))?;

    let facts: Vec<MemoryFactResponse> = raw
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<MemoryFactResponse>(v) {
            Ok(f) => Some(f),
            Err(e) => {
                tracing::warn!("memory fact row decode failed: {e}");
                None
            }
        })
        .filter(|f| match query.category.as_deref() {
            Some(cat) => f.category == cat,
            None => true,
        })
        .filter(is_public_memory_fact)
        .collect();
    let total = facts.len();

    Ok(Json(MemoryListResponse { facts, total }))
}

/// GET /api/memory - List ALL memory facts across all agents.
pub async fn list_all_memory_facts(
    Query(query): Query<AllMemoryListQuery>,
    State(state): State<AppState>,
) -> Result<Json<MemoryListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Route through the trait surface so the underlying backend is abstracted
    // when the user has opted in via Settings → Persistence. The legacy
    // concrete `state.memory_repo` is no longer the source of truth here.
    let memory_store_slot = state.memory_store();
    let memory_store = super::require(&memory_store_slot, "Memory service not available")?;

    let raw_facts = memory_store
        .list_memory_facts(
            query.agent_id.as_deref(),
            query.category.as_deref(),
            query.scope.as_deref(),
            query.limit,
            query.offset,
        )
        .await
        .map_err(|e| {
            public_internal_error(
                "Failed to list memory facts",
                "Failed to list memory facts",
                e,
            )
        })?;

    // Each row is a serde_json::Value matching the MemoryFactResponse shape.
    // Rows that fail to deserialize are skipped with a warning rather than
    // failing the whole request.
    let facts: Vec<MemoryFactResponse> = raw_facts
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<MemoryFactResponse>(v) {
            Ok(f) => Some(f),
            Err(e) => {
                tracing::warn!("memory fact row decode failed: {e}");
                None
            }
        })
        .filter(is_public_memory_fact)
        .collect();
    let total = facts.len();

    Ok(Json(MemoryListResponse { facts, total }))
}

// ============================================================================
// PHASE 4: CONSOLIDATE / STATS / HEALTH
// ============================================================================

/// Response for `POST /api/memory/consolidate`.
#[derive(Debug, Serialize)]
pub struct ConsolidateResponse {
    pub status: &'static str,
}

/// Trigger a sleep-time consolidation cycle.
///
/// Returns `503 Service Unavailable` when the worker has not been wired
/// into `AppState` yet.
pub async fn consolidate(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<ConsolidateResponse>), (StatusCode, String)> {
    let worker = state.sleep_time_worker().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sleep-time worker not initialized".to_string(),
    ))?;
    worker.trigger();
    Ok((
        StatusCode::ACCEPTED,
        Json(ConsolidateResponse {
            status: "triggered",
        }),
    ))
}

/// Response for `POST /api/procedures/dedupe`.
#[derive(Debug, Serialize)]
pub struct DedupeProceduresResponse {
    pub status: &'static str,
    pub deleted: usize,
}

/// `POST /api/procedures/dedupe` — collapse procedure duplicates by name.
///
/// For each `(agent_id, name)` pair, keeps the row with the highest
/// `success_count` (ties broken by most recent `created_at`) and deletes
/// the rest. Vec-index rows are cleaned up alongside.
///
/// Idempotent. Returns `{ status, deleted }` with the row delete count.
/// Returns `503` when the procedure store isn't wired into AppState.
pub async fn dedupe_procedures(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<DedupeProceduresResponse>), (StatusCode, String)> {
    let store = state.procedure_store().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "procedure store not initialized".to_string(),
    ))?;
    let deleted = store
        .dedupe_procedures_by_name()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        StatusCode::OK,
        Json(DedupeProceduresResponse {
            status: "ok",
            deleted,
        }),
    ))
}

/// Memory subsystem stats response.
#[derive(Debug, Serialize, Default)]
pub struct MemoryStats {
    pub entities: i64,
    pub relationships: i64,
    pub facts: i64,
    pub episodes: i64,
    pub procedures: i64,
    pub wiki_articles: i64,
    pub goals_active: i64,
    pub db_size_mb: f64,
}

/// `GET /api/memory/stats` — aggregate counts across memory subsystems.
///
/// Counts are pulled through trait-erased stores:
/// - entity / relationship counts come from `kg_store` (per-agent
///   "root" view, mirroring the historical handler's `get_entities`
///   / `get_relationships` calls).
/// - fact / episode / procedure / wiki / goal counts come from
///   `memory_store.aggregate_stats()`.
///
/// `db_size_mb` is read directly from the configured Engram storage path —
/// that's a filesystem operation, not a store concern.
pub async fn stats(State(state): State<AppState>) -> Json<MemoryStats> {
    let mut stats = MemoryStats::default();

    if let Some(kg_store) = state.kg_store().as_ref() {
        // The historical handler used `get_entities`/`get_relationships`
        // (which return all rows for the agent and `len()` them);
        // `list_entities`/`list_relationships` with a high cap mirrors
        // that without paging surprises.
        if let Ok(entities) = kg_store.list_entities("root", None, 100_000, 0).await {
            stats.entities = entities.len() as i64;
        }
        if let Ok(rels) = kg_store.list_relationships("root", None, 100_000, 0).await {
            stats.relationships = rels.len() as i64;
        }
    }

    if let Some(memory_store) = state.memory_store().as_ref() {
        if let Ok(agg) = memory_store.aggregate_stats().await {
            stats.facts = agg.facts;
            stats.episodes = agg.episodes;
            stats.procedures = agg.procedures;
            stats.wiki_articles = agg.wiki_articles;
            stats.goals_active = agg.goals_active;
        }
    }

    let provider = state
        .settings()
        .get_execution_settings()
        .map(|settings| settings.memory.provider)
        .unwrap_or_default();
    let storage_path =
        crate::state::persistence_factory::adapter_config_from_memory_provider_settings(
            state.paths().as_ref(),
            &provider,
        )
        .and_then(|config| {
            config
                .resolve_engram_path()
                .map(|resolved| resolved.path().to_path_buf())
                .map_err(|error| error.to_string())
        });
    if let Ok(storage_path) = storage_path {
        if let Ok(bytes) = storage_size_bytes(&storage_path, 0) {
            stats.db_size_mb = (bytes as f64) / (1024.0 * 1024.0);
        }
    }

    Json(stats)
}

fn storage_size_bytes(path: &std::path::Path, depth: usize) -> std::io::Result<u64> {
    if depth > 4 {
        return Ok(0);
    }
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(0);
    }
    if meta.is_file() {
        return Ok(meta.len());
    }

    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        total = total.saturating_add(storage_size_bytes(&entry.path(), depth + 1)?);
    }
    Ok(total)
}

/// Memory subsystem health response.
#[derive(Debug, Serialize, Default)]
pub struct MemoryHealth {
    pub ingestion_queue_pending: u64,
    pub ingestion_queue_running: u64,
    pub failed_episodes_recent: u64,
    pub last_compaction_run_id: Option<String>,
    pub last_compaction_merges: u64,
    pub last_compaction_prunes: u64,
    pub last_compaction_at: Option<String>,
    pub governance: Option<GovernanceCapabilityHealth>,
}

/// `GET /api/memory/health` — queue depth, recent failures, last compaction.
///
/// Pulls episode-pipeline metrics through `state.memory_store.health_metrics`
/// instead of reaching into a concrete semantic database handle.
pub async fn health(State(state): State<AppState>) -> Json<MemoryHealth> {
    let mut health = MemoryHealth::default();

    if let Some(memory_store) = state.memory_store().as_ref() {
        if let Ok(m) = memory_store.health_metrics().await {
            health.ingestion_queue_pending = m.queue_pending;
            health.ingestion_queue_running = m.queue_running;
            health.failed_episodes_recent = m.failed_recent;
        }
    }

    if let Some(compaction_store) = state.compaction_store().as_ref() {
        if let Ok(Some(summary)) = compaction_store.latest_run_summary().await {
            health.last_compaction_run_id = Some(summary.run_id);
            health.last_compaction_merges = summary.merges;
            health.last_compaction_prunes = summary.prunes;
            health.last_compaction_at = Some(summary.latest_at);
        }
    }

    health.governance = state.governance_health().clone();

    Json(health)
}
