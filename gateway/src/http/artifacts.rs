//! # Artifact Endpoints
//!
//! HTTP API for listing and serving file artifacts produced by agent executions.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path as FsPath};

const MAX_GOAL_ARTIFACTS_PER_SESSION: u32 = 24;

// ============================================================================
// RESPONSE TYPES
// ============================================================================

/// JSON representation of an artifact for API responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactResponse {
    pub id: String,
    pub session_id: String,
    pub ward_id: Option<String>,
    pub execution_id: Option<String>,
    pub agent_id: Option<String>,
    pub file_name: String,
    pub file_type: Option<String>,
    pub file_size: Option<i64>,
    pub label: Option<String>,
    pub is_goal_artifact: bool,
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
            file_name: a.file_name,
            file_type: a.file_type,
            file_size: a.file_size,
            label: a.label,
            is_goal_artifact: a.is_goal_artifact,
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ArtifactListQuery {
    #[serde(default)]
    pub goal_artifacts_only: bool,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ArtifactContentQuery {
    pub session_id: Option<String>,
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
    Query(query): Query<ArtifactListQuery>,
) -> Result<Json<Vec<ArtifactResponse>>, (StatusCode, String)> {
    if let Some(limit) = query.limit {
        if limit == 0 || limit > MAX_GOAL_ARTIFACTS_PER_SESSION {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("limit must be between 1 and {MAX_GOAL_ARTIFACTS_PER_SESSION}"),
            ));
        }
    }

    let artifacts = if query.goal_artifacts_only {
        state.state_service.list_goal_artifacts_by_session(
            &session_id,
            query.limit.unwrap_or(MAX_GOAL_ARTIFACTS_PER_SESSION),
        )
    } else {
        state.state_service.list_artifacts_by_session(&session_id)
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let artifacts = if query.goal_artifacts_only {
        artifacts
    } else if let Some(limit) = query.limit {
        artifacts.into_iter().take(limit as usize).collect()
    } else {
        artifacts
    };

    Ok(Json(
        artifacts.into_iter().map(ArtifactResponse::from).collect(),
    ))
}

/// GET /api/artifacts/:artifact_id/content?session_id=:session_id
///
/// Serve the raw file content of an artifact with appropriate content-type.
pub async fn serve_artifact_content(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    Query(query): Query<ArtifactContentQuery>,
) -> Result<Response, (StatusCode, String)> {
    let session_id = query
        .session_id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "session_id is required to fetch artifact content".to_string(),
            )
        })?;

    let artifact = state
        .state_service
        .get_artifact(&artifact_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Artifact not found".to_string()))?;

    if artifact.session_id != session_id {
        return Err((StatusCode::NOT_FOUND, "Artifact not found".to_string()));
    }

    let ward_id = artifact
        .ward_id
        .as_deref()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Artifact not found".to_string()))?;
    let validated = gateway_execution::artifacts::open_persisted_artifact(
        &state.vault_dir,
        ward_id,
        FsPath::new(&artifact.file_path),
    )
    .map_err(|error| {
        let status = if error.kind() == std::io::ErrorKind::InvalidData {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::NOT_FOUND
        };
        (status, "Artifact is no longer available".to_string())
    })?;

    let mut content = Vec::with_capacity(validated.metadata.len() as usize);
    let file = validated.file;
    file.take(gateway_execution::artifacts::MAX_ARTIFACT_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                "Artifact is no longer available".to_string(),
            )
        })?;
    if content.len() as u64 > gateway_execution::artifacts::MAX_ARTIFACT_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Artifact is too large to serve safely".to_string(),
        ));
    }

    Ok(artifact_content_response(&artifact, content))
}

fn artifact_content_response(artifact: &execution_state::Artifact, content: Vec<u8>) -> Response {
    let mime = match artifact.file_type.as_deref() {
        Some("md") => "text/markdown",
        // Direct active content must never execute. The slide-out previews
        // fetched HTML/SVG text in a script-disabled iframe instead.
        Some("html") | Some("htm") | Some("svg") => "text/plain; charset=utf-8",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("txt") => "text/plain",
        Some("py") | Some("rs") | Some("js") | Some("ts") => "text/plain",
        _ => "application/octet-stream",
    };

    let mut response = content.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    if matches!(
        artifact.file_type.as_deref(),
        Some("html") | Some("htm") | Some("svg")
    ) {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_state() -> (TempDir, AppState) {
        let dir = TempDir::new().expect("temp dir");
        let state = AppState::minimal(dir.path().to_path_buf());
        (dir, state)
    }

    fn create_session(state: &AppState) -> execution_state::Session {
        state
            .state_service
            .create_session("root")
            .expect("create session")
            .0
    }

    fn store_artifact(
        state: &AppState,
        session_id: &str,
        file_path: &std::path::Path,
        ward_id: &str,
        goal: bool,
    ) -> execution_state::Artifact {
        let file_name = file_path
            .file_name()
            .expect("artifact name")
            .to_string_lossy()
            .to_string();
        let mut artifact = execution_state::Artifact::new(
            session_id,
            file_path.to_string_lossy().to_string(),
            file_name,
        );
        artifact.ward_id = Some(ward_id.to_string());
        artifact.file_type = Some("txt".to_string());
        artifact.is_goal_artifact = goal;
        state
            .state_service
            .create_artifact(&artifact)
            .expect("store artifact");
        artifact
    }

    #[test]
    fn artifact_response_exposes_goal_bit_but_not_server_path() {
        let mut artifact =
            execution_state::Artifact::new("sess-1", "/private/path/report.md", "report.md");
        artifact.is_goal_artifact = true;

        let value = serde_json::to_value(ArtifactResponse::from(artifact)).expect("serialize");

        assert_eq!(value["isGoalArtifact"], true);
        assert!(value.get("filePath").is_none());
    }

    #[test]
    fn active_content_is_forced_to_non_executable_attachment_type() {
        let mut artifact =
            execution_state::Artifact::new("sess-1", "/private/path/report.html", "report.html");
        artifact.file_type = Some("html".to_string());

        for file_type in ["html", "svg"] {
            artifact.file_type = Some(file_type.to_string());
            let response = artifact_content_response(&artifact, b"<script>bad()</script>".to_vec());
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/plain; charset=utf-8"
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(
                response.headers()[header::CONTENT_DISPOSITION],
                "attachment"
            );
        }
    }

    #[tokio::test]
    async fn goal_only_manifest_is_bounded_and_redacts_the_server_path() {
        let (dir, state) = make_state();
        let session = create_session(&state);
        let ward = dir.path().join("wards").join("work");
        std::fs::create_dir_all(&ward).expect("create ward");
        store_artifact(
            &state,
            &session.id,
            &ward.join("deliverable.txt"),
            "work",
            true,
        );
        store_artifact(
            &state,
            &session.id,
            &ward.join("working-notes.txt"),
            "work",
            false,
        );

        let response = list_session_artifacts(
            State(state.clone()),
            Path(session.id.clone()),
            Query(ArtifactListQuery {
                goal_artifacts_only: true,
                limit: Some(1),
            }),
        )
        .await
        .expect("list deliverables")
        .0;
        assert_eq!(response.len(), 1);
        assert!(response[0].is_goal_artifact);
        let serialized = serde_json::to_value(&response).expect("serialize manifest");
        assert!(serialized[0].get("filePath").is_none());

        let error = list_session_artifacts(
            State(state),
            Path(session.id),
            Query(ArtifactListQuery {
                goal_artifacts_only: true,
                limit: Some(25),
            }),
        )
        .await
        .expect_err("limit above 24 must fail");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn artifact_content_requires_the_matching_session() {
        let (dir, state) = make_state();
        let owner = create_session(&state);
        let other = create_session(&state);
        let ward = dir.path().join("wards").join("work");
        std::fs::create_dir_all(&ward).expect("create ward");
        let artifact = store_artifact(&state, &owner.id, &ward.join("report.txt"), "work", true);

        let missing = serve_artifact_content(
            State(state.clone()),
            Path(artifact.id.clone()),
            Query(ArtifactContentQuery { session_id: None }),
        )
        .await
        .expect_err("missing session must fail");
        assert_eq!(missing.0, StatusCode::BAD_REQUEST);

        let mismatched = serve_artifact_content(
            State(state),
            Path(artifact.id),
            Query(ArtifactContentQuery {
                session_id: Some(other.id),
            }),
        )
        .await
        .expect_err("cross-session request must fail");
        assert_eq!(mismatched.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn oversized_persisted_artifact_returns_413() {
        let (dir, state) = make_state();
        let session = create_session(&state);
        let ward = dir.path().join("wards").join("work");
        std::fs::create_dir_all(&ward).expect("create ward");
        let path = ward.join("large.txt");
        std::fs::File::create(&path)
            .expect("create file")
            .set_len(gateway_execution::artifacts::MAX_ARTIFACT_BYTES + 1)
            .expect("grow file");
        let artifact = store_artifact(&state, &session.id, &path, "work", true);

        let error = serve_artifact_content(
            State(state),
            Path(artifact.id),
            Query(ArtifactContentQuery {
                session_id: Some(session.id),
            }),
        )
        .await
        .expect_err("oversized artifact must fail");
        assert_eq!(error.0, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn checked_in_openapi_shape_matches_the_manifest_response() {
        let contract: serde_json::Value = serde_yaml::from_str(include_str!(
            "../../../contracts/openapi/goal-artifacts.yaml"
        ))
        .expect("parse checked-in OpenAPI contract");
        let schema = &contract["components"]["schemas"]["Artifact"];
        let content_response =
            &contract["paths"]["/api/artifacts/{artifactId}/content"]["get"]["responses"]["200"];
        let mut artifact = execution_state::Artifact::new("sess-1", "/server/path.txt", "path.txt");
        artifact.is_goal_artifact = true;
        let manifest = serde_json::to_value(ArtifactResponse::from(artifact)).expect("serialize");

        assert!(matches_artifact_contract(&manifest, schema));
        let mut missing_goal_bit = manifest.clone();
        missing_goal_bit
            .as_object_mut()
            .expect("manifest object")
            .remove("isGoalArtifact");
        assert!(!matches_artifact_contract(&missing_goal_bit, schema));
        assert_eq!(
            content_response["headers"]["X-Content-Type-Options"]["schema"]["enum"][0],
            "nosniff"
        );
        for media_type in [
            "text/*",
            "application/json",
            "application/pdf",
            "image/*",
            "audio/*",
            "video/*",
            "application/octet-stream",
        ] {
            assert!(
                content_response["content"].get(media_type).is_some(),
                "missing {media_type}"
            );
        }
    }

    fn matches_artifact_contract(value: &serde_json::Value, schema: &serde_json::Value) -> bool {
        let Some(object) = value.as_object() else {
            return false;
        };
        let Some(properties) = schema["properties"].as_object() else {
            return false;
        };
        let required = schema["required"]
            .as_array()
            .expect("contract required array");
        required.iter().all(|field| {
            field
                .as_str()
                .is_some_and(|field| object.contains_key(field))
        }) && object.keys().all(|field| properties.contains_key(field))
    }
}
