//! # Embedding Endpoints
//!
//! Phase 1 HTTP surface for the embedding backend selection feature.
//!
//! Endpoints:
//! - `GET  /api/embeddings/health` — current backend + dim + status
//! - `GET  /api/embeddings/models` — curated Ollama dropdown entries
//! - `GET  /api/embeddings/ollama-models?url=<base>` — models the user's
//!   Ollama instance actually has pulled, filtered to likely embedding
//!   models. Soft-fails to an empty list when Ollama is unreachable so
//!   the UI can gracefully fall back to curated suggestions.
//! - `POST /api/embeddings/configure` — apply a new [`EmbeddingConfig`];
//!   responds with an SSE stream of [`Health`] events terminating in
//!   `ready` or `error`.

use std::convert::Infallible;

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream::Stream;
use gateway_services::{CuratedModel, EmbeddingConfig, Health, OllamaClient, CURATED_MODELS};
use serde::{Deserialize, Serialize};

// ============================================================================
// GET /api/embeddings/health
// ============================================================================

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub backend: String,
    pub model: Option<String>,
    pub dim: usize,
    pub status: String,
    pub indexed_count: usize,
    pub needs_reindex: bool,
    /// Backend-reported embedding index lanes that are currently present.
    pub tables_present: Vec<String>,
    /// Backend-reported embedding index lanes that are missing.
    pub tables_missing: Vec<String>,
}

pub async fn get_health(State(state): State<AppState>) -> Json<HealthResponse> {
    let svc = &state.embedding_service;
    let client = svc.client();
    let snapshot = svc.config_snapshot();
    let status_str = match svc.health() {
        Health::Ready => "ready".to_string(),
        Health::Reindexing { .. } => "reindexing".to_string(),
        Health::Pulling { .. } => "pulling".to_string(),
        Health::OllamaUnreachable => "ollama_unreachable".to_string(),
        Health::ModelMissing => "model_missing".to_string(),
        Health::Misconfigured(_) => "misconfigured".to_string(),
    };
    let vec_health = vec_health_snapshot(&state).await;
    Json(HealthResponse {
        backend: snapshot.backend.as_str().to_string(),
        model: Some(client.model_name().to_string()),
        dim: svc.dimensions(),
        status: status_str,
        indexed_count: vec_health.indexed_rows,
        needs_reindex: svc.needs_reindex(),
        tables_present: vec_health.tables_present,
        tables_missing: vec_health.tables_missing,
    })
}

/// Pull the vector-index health snapshot from `state.kg_store`. When
/// the trait-erased store is unavailable (smoke tests, partial init),
/// fall back to "all five tables missing, zero indexed" so the
/// endpoint keeps responding — same degraded-but-honest behavior the
/// historical handler exhibited on DB errors.
async fn vec_health_snapshot(state: &AppState) -> knowledge_graph::kg_trait::VecIndexHealth {
    if let Some(kg_store) = state.kg_store.as_ref() {
        if let Ok(h) = kg_store.vec_index_health().await {
            return h;
        }
    }
    knowledge_graph::kg_trait::VecIndexHealth {
        tables_present: Vec::new(),
        tables_missing: Vec::new(),
        indexed_rows: 0,
    }
}

// ============================================================================
// GET /api/embeddings/models
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ModelEntry {
    pub tag: &'static str,
    pub dim: usize,
    pub size_mb: u32,
    pub mteb: u32,
}

impl From<&CuratedModel> for ModelEntry {
    fn from(m: &CuratedModel) -> Self {
        Self {
            tag: m.tag,
            dim: m.dim,
            size_mb: m.size_mb,
            mteb: m.mteb,
        }
    }
}

pub async fn list_models() -> Json<Vec<ModelEntry>> {
    Json(CURATED_MODELS.iter().map(ModelEntry::from).collect())
}

// ============================================================================
// GET /api/embeddings/ollama-models?url=...
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct OllamaModelsQuery {
    /// Base URL of the user's Ollama instance, e.g. `http://localhost:11434`.
    /// Omitted → default localhost URL.
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OllamaModelsResponse {
    /// All model tags returned by `/api/tags` on the user's instance.
    pub all: Vec<String>,
    /// Subset of `all` whose name heuristically looks like an embedding
    /// model. The UI shows this first in the typeahead, but does not
    /// prevent the user from picking something in `all` — the dim probe
    /// on reconfigure catches wrong-dim picks regardless.
    pub likely_embedding: Vec<String>,
    /// `true` when the request reached the instance. `false` means we
    /// couldn't connect (common when Ollama isn't running or the URL is
    /// wrong); the UI falls back to curated suggestions in that case.
    pub reachable: bool,
}

/// Heuristic: model tags that look like embedding models. Ollama has no
/// explicit "is-embedding" flag, so we filter by substring match against
/// names known to be embedding-family. This is advisory — the dim probe
/// is the source of truth, so missing a match here just means the user's
/// model is still selectable, it's just not bubbled to the top.
const EMBEDDING_NAME_HINTS: &[&str] = &[
    "embed", "bge", "nomic", "arctic", "mxbai", "e5", "gte-", "minilm",
];

fn looks_like_embedding(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    EMBEDDING_NAME_HINTS.iter().any(|h| lower.contains(h))
}

pub async fn list_ollama_models(Query(q): Query<OllamaModelsQuery>) -> Json<OllamaModelsResponse> {
    let base_url = q
        .url
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let client = OllamaClient::new(base_url);
    match client.list_models().await {
        Ok(all) => {
            let likely: Vec<String> = all
                .iter()
                .filter(|n| looks_like_embedding(n))
                .cloned()
                .collect();
            Json(OllamaModelsResponse {
                likely_embedding: likely,
                all,
                reachable: true,
            })
        }
        Err(err) => {
            tracing::debug!(error = %err, "ollama-models: instance unreachable");
            Json(OllamaModelsResponse {
                all: Vec::new(),
                likely_embedding: Vec::new(),
                reachable: false,
            })
        }
    }
}

// ============================================================================
// POST /api/embeddings/configure — SSE stream
// ============================================================================

pub async fn configure(
    State(state): State<AppState>,
    Json(new): Json<EmbeddingConfig>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let svc = state.embedding_service.clone();
    // Persist the intent first so a daemon restart will honor the selection.
    svc.persist_settings(&new)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Collect progress events into a channel.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Health>();
    let tx_cb = tx.clone();
    let cb = move |h: Health| {
        let _ = tx_cb.send(h);
    };
    let svc_clone = svc.clone();
    let new_clone = new.clone();
    tokio::spawn(async move {
        // Phase 1: reconfigure (ping + pull + client swap).
        if let Err(e) = svc_clone.reconfigure(new_clone.clone(), cb).await {
            let _ = tx.send(Health::Misconfigured(e));
            return;
        }

        // Phase 2: reindex if required (dim or model changed).
        if svc_clone.needs_reindex() {
            let current_dim = svc_clone.dimensions();
            if let Err(e) = svc_clone.mark_indexed(current_dim) {
                let _ = tx.send(Health::Misconfigured(format!(
                    "embedding config updated but marker write failed: {e}"
                )));
                return;
            }
        }

        let _ = tx.send(Health::Ready);
    });

    let s = async_stream::stream! {
        while let Some(h) = rx.recv().await {
            let (ev_name, terminal) = match &h {
                Health::Ready => ("ready", true),
                Health::Misconfigured(_) => ("error", true),
                Health::Reindexing { .. } => ("reindexing", false),
                Health::Pulling { .. } => ("pulling", false),
                Health::OllamaUnreachable => ("error", true),
                Health::ModelMissing => ("error", true) };
            let payload = serde_json::to_string(&h).unwrap_or_else(|_| "{}".into());
            let ev = Event::default().event(ev_name).data(payload);
            yield Ok::<_, Infallible>(ev);
            if terminal {
                break;
            }
        }
    };
    Ok(Sse::new(s).keep_alive(KeepAlive::default()))
}

// ============================================================================
// POST /api/embeddings/reindex
// ============================================================================

/// Ask the active knowledge graph store to rebuild embedding indexes.
///
/// Engram-backed stores own the actual rebuild strategy. This route remains
/// as the backend-agnostic manual trigger used by Settings/Observatory.
pub async fn reindex(
    State(state): State<AppState>,
) -> Result<Json<knowledge_graph::kg_trait::ReindexReport>, (StatusCode, String)> {
    let kg_store = state.kg_store.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "knowledge graph store not available".to_string(),
    ))?;
    let dim = state.embedding_service.dimensions();
    kg_store
        .reindex_embeddings(dim)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_embedding_recognizes_common_families() {
        assert!(looks_like_embedding("nomic-embed-text"));
        assert!(looks_like_embedding("bge-large:latest"));
        assert!(looks_like_embedding("snowflake-arctic-embed"));
        assert!(looks_like_embedding("mxbai-embed-large"));
        assert!(looks_like_embedding("all-MiniLM-L6-v2"));
    }

    #[test]
    fn looks_like_embedding_rejects_chat_models() {
        assert!(!looks_like_embedding("llama3"));
        assert!(!looks_like_embedding("qwen2.5"));
        assert!(!looks_like_embedding("gpt-oss"));
        assert!(!looks_like_embedding("deepseek-r1"));
    }
}
