//! POST /api/graph/ingest — enqueue chunks for extraction.
//! GET  /api/graph/ingest/:source_id/progress — poll status.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::state::AppState;
use gateway_execution::ingest::chunker::{chunk_text, ChunkOptions};

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub source_id: String,
    #[serde(default = "default_source_type")]
    pub source_type: String,
    pub text: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub chunk_opts: Option<IngestChunkOpts>,
}

fn default_source_type() -> String {
    "document".to_string()
}

#[derive(Debug, Deserialize)]
pub struct IngestChunkOpts {
    pub target_tokens: Option<usize>,
    pub overlap_tokens: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub source_id: String,
    pub episode_count: usize,
}

pub async fn ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<(StatusCode, Json<IngestResponse>), (StatusCode, String)> {
    let queue = state.ingestion_queue.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "ingestion queue not initialized".into(),
    ))?;
    // Phase B: prefer the trait-routed kg_episode_store (wired in both
    // backends). Falls back to the concrete kg_episode_repo only when
    // only that is available (legacy / minimal AppStates).
    let episode_store = state.kg_episode_store.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "kg episode store missing".into(),
    ))?;
    let backpressure = state.ingestion_backpressure.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "backpressure not initialized".into(),
    ))?;

    backpressure
        .check(&req.source_id)
        .await
        .map_err(|e| (StatusCode::TOO_MANY_REQUESTS, e))?;

    let opts = ChunkOptions {
        target_tokens: req
            .chunk_opts
            .as_ref()
            .and_then(|o| o.target_tokens)
            .unwrap_or(1000),
        overlap_tokens: req
            .chunk_opts
            .as_ref()
            .and_then(|o| o.overlap_tokens)
            .unwrap_or(100),
    };
    let chunks = chunk_text(&req.text, opts);
    let agent_id = req.agent_id.unwrap_or_else(|| "root".to_string());

    let mut enqueued = 0usize;
    for chunk in &chunks {
        let source_ref = format!("{}#chunk-{}", req.source_id, chunk.index);
        let content_hash = {
            let mut h = Sha256::new();
            h.update(chunk.text.as_bytes());
            format!("{:x}", h.finalize())
        };
        let episode_id = episode_store
            .upsert_pending(
                &req.source_type,
                &source_ref,
                &content_hash,
                req.session_id.as_deref(),
                &agent_id,
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        episode_store
            .set_payload(&episode_id, &chunk.text)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        enqueued += 1;
    }
    queue.notify();

    Ok((
        StatusCode::ACCEPTED,
        Json(IngestResponse {
            source_id: req.source_id,
            episode_count: enqueued,
        }),
    ))
}

#[derive(Debug, Serialize)]
pub struct ProgressResponse {
    pub source_id: String,
    pub pending: u64,
    pub running: u64,
    pub done: u64,
    pub failed: u64,
}

pub async fn progress(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
) -> Result<Json<ProgressResponse>, (StatusCode, String)> {
    // Phase B: trait-routed (works on both backends).
    let store = state.kg_episode_store.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "kg episode store missing".into(),
    ))?;
    let counts = store
        .status_counts_for_source(&source_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(ProgressResponse {
        source_id,
        pending: counts.pending,
        running: counts.running,
        done: counts.done,
        failed: counts.failed,
    }))
}
