//! # Artifact Endpoints
//!
//! HTTP API for listing and serving file artifacts produced by agent executions.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Serialize;

// ============================================================================
// RESPONSE TYPES
// ============================================================================

/// JSON representation of an artifact for API responses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactResponse {
    pub id: String,
    pub session_id: String,
    pub ward_id: Option<String>,
    pub execution_id: Option<String>,
    pub agent_id: Option<String>,
    pub file_path: String,
    pub file_name: String,
    pub file_type: Option<String>,
    pub file_size: Option<i64>,
    pub label: Option<String>,
    pub created_at: String,
}

impl From<execution_state::Artifact> for ArtifactResponse {
    fn from(a: execution_state::Artifact) -> Self {
        Self {
            id: a.id,
            session_id: a.session_id,
            ward_id: a.ward_id,
            execution_id: a.execution_id,
            agent_id: a.agent_id,
            file_path: a.file_path,
            file_name: a.file_name,
            file_type: a.file_type,
            file_size: a.file_size,
            label: a.label,
            created_at: a.created_at,
        }
    }
}

// ============================================================================
// ENDPOINTS
// ============================================================================

/// GET /api/sessions/:session_id/artifacts
///
/// List all artifacts produced during a session.
pub async fn list_session_artifacts(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ArtifactResponse>>, (StatusCode, String)> {
    let artifacts = state
        .state_service
        .list_artifacts_by_session(&session_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(
        artifacts.into_iter().map(ArtifactResponse::from).collect(),
    ))
}

/// GET /api/artifacts/:artifact_id/content
///
/// Serve the raw file content of an artifact with appropriate content-type.
pub async fn serve_artifact_content(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let artifact = state
        .state_service
        .get_artifact(&artifact_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Artifact not found".to_string()))?;

    let content = std::fs::read(&artifact.file_path)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("File not found: {}", e)))?;

    let mime = match artifact.file_type.as_deref() {
        Some("md") => "text/markdown",
        Some("html") | Some("htm") => "text/html",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("txt") => "text/plain",
        Some("py") | Some("rs") | Some("js") | Some("ts") => "text/plain",
        _ => "application/octet-stream",
    };

    Ok(([(header::CONTENT_TYPE, mime)], content))
}
