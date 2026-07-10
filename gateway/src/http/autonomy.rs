//! HTTP lifecycle surface for durable decision threads.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zbot_conversation::{AutonomyApprovalPolicy, AutonomyEvidence, AutonomyItem, AutonomyState};

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateAutonomyItemRequest {
    pub title: String,
    pub objective: String,
    pub next_action: String,
    pub source_session_id: Option<String>,
    pub dedupe_key: String,
    #[serde(default)]
    pub evidence: Vec<EvidenceRequest>,
}

#[derive(Debug, Deserialize)]
pub struct EvidenceRequest {
    pub kind: String,
    pub reference_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    pub state: AutonomyState,
    pub outcome: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AutonomyDetailResponse {
    #[serde(flatten)]
    pub item: AutonomyItem,
    pub evidence: Vec<AutonomyEvidence>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn list_items(
    State(state): State<AppState>,
) -> Result<Json<Vec<AutonomyItem>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .autonomy
        .list_open(100)
        .map(Json)
        .map_err(internal_error)
}

pub async fn get_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AutonomyDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let item = state
        .autonomy
        .get(&id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found(&id))?;
    let evidence = state.autonomy.evidence(&id).map_err(internal_error)?;
    Ok(Json(AutonomyDetailResponse { item, evidence }))
}

pub async fn create_item(
    State(state): State<AppState>,
    Json(request): Json<CreateAutonomyItemRequest>,
) -> Result<(StatusCode, Json<AutonomyDetailResponse>), (StatusCode, Json<ErrorResponse>)> {
    if request.title.trim().is_empty()
        || request.objective.trim().is_empty()
        || request.next_action.trim().is_empty()
        || request.dedupe_key.trim().is_empty()
    {
        return Err(bad_request(
            "title, objective, next_action, and dedupe_key are required",
        ));
    }
    if request.title.chars().count() > 200
        || request.objective.chars().count() > 2_000
        || request.next_action.chars().count() > 1_000
        || request.dedupe_key.chars().count() > 300
        || request.evidence.len() > 32
    {
        return Err(bad_request(
            "autonomy item exceeds its bounded input limits",
        ));
    }
    if request.evidence.iter().any(|entry| {
        entry.kind.trim().is_empty()
            || entry.reference_id.trim().is_empty()
            || entry.kind.chars().count() > 64
            || entry.reference_id.chars().count() > 512
            || entry
                .label
                .as_ref()
                .is_some_and(|label| label.chars().count() > 256)
    }) {
        return Err(bad_request(
            "evidence must contain bounded kind and reference_id values",
        ));
    }
    let now = Utc::now().to_rfc3339();
    let id = format!("aut-{}", Uuid::now_v7());
    let item = AutonomyItem {
        id: id.clone(),
        title: request.title.trim().to_string(),
        objective: request.objective.trim().to_string(),
        next_action: request.next_action.trim().to_string(),
        state: AutonomyState::Proposed,
        approval_policy: AutonomyApprovalPolicy::Manual,
        source_session_id: request.source_session_id,
        dedupe_key: request.dedupe_key.trim().to_string(),
        created_at: now.clone(),
        updated_at: now.clone(),
        completed_at: None,
    };
    let evidence = request
        .evidence
        .into_iter()
        .map(|entry| AutonomyEvidence {
            id: format!("ae-{}", Uuid::now_v7()),
            item_id: id.clone(),
            kind: entry.kind,
            reference_id: entry.reference_id,
            label: entry.label,
            created_at: now.clone(),
        })
        .collect::<Vec<_>>();
    state.autonomy.create(&item, &evidence).map_err(|error| {
        if error.to_string().contains("UNIQUE constraint failed") {
            bad_request("an item already has this dedupe key")
        } else {
            internal_error(error)
        }
    })?;
    Ok((
        StatusCode::CREATED,
        Json(AutonomyDetailResponse { item, evidence }),
    ))
}

pub async fn transition_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<TransitionRequest>,
) -> Result<Json<AutonomyDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let item = state
        .autonomy
        .transition(&id, request.state, request.outcome.as_deref())
        .map_err(|error| {
            if error.to_string().contains("not found") {
                not_found(&id)
            } else if error.to_string().contains("invalid autonomy transition") {
                bad_request(&error.to_string())
            } else {
                internal_error(error)
            }
        })?;
    let evidence = state.autonomy.evidence(&id).map_err(internal_error)?;
    Ok(Json(AutonomyDetailResponse { item, evidence }))
}

fn bad_request(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

fn not_found(id: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("autonomy item not found: {id}"),
        }),
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "autonomy ledger request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "internal autonomy ledger error".to_string(),
        }),
    )
}
