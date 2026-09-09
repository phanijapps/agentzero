//! HTTP lifecycle surface for durable decision threads.

use super::ErrorResponse;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zbot_conversation::{AutonomyApprovalPolicy, AutonomyEvidence, AutonomyItem, AutonomyState};

use crate::state::AppState;

const TRANSITION_OUTCOME_MAX_BYTES: usize = 512;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAutonomyItemRequest {
    pub title: String,
    pub objective: String,
    pub next_action: String,
    pub source_session_id: Option<String>,
    pub dedupe_key: String,
    #[serde(default)]
    pub approval_policy: AutonomyApprovalPolicy,
    #[serde(default)]
    pub evidence: Vec<EvidenceRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequest {
    pub kind: String,
    pub reference_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionRequest {
    pub state: AutonomyState,
    pub outcome: Option<String>,
}

/// Intentionally empty: the selected URL id is the entire explicit-resume
/// request. Denying unknown fields prevents a caller from smuggling a packet,
/// source agent, or generic execution context through this boundary.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeRequest {}

#[derive(Debug, Serialize)]
pub struct AutonomyDetailResponse {
    #[serde(flatten)]
    pub item: AutonomyItem,
    pub evidence: Vec<AutonomyEvidence>,
}

/// A resume result deliberately contains no packet or copied source content.
#[derive(Debug, Serialize)]
pub struct AutonomyResumeResponse {
    pub item_id: String,
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EligibilityQuery {
    pub trigger: String,
}

/// Read-only eligibility for a deliberately unimplemented future trigger.
#[derive(Debug, Serialize)]
pub struct AutonomyEligibilityResponse {
    pub item_id: String,
    pub state: AutonomyState,
    pub approval_policy: AutonomyApprovalPolicy,
    pub trigger: String,
    pub eligible: bool,
    pub reason: &'static str,
    pub read_only: bool,
    pub scheduler_configured: bool,
    pub execution_started: bool,
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
    if request.title.len() > 200
        || request.objective.len() > 2_048
        || request.next_action.len() > 1_024
        || request.dedupe_key.len() > 300
        || request
            .source_session_id
            .as_ref()
            .is_some_and(|session_id| session_id.trim().is_empty() || session_id.len() > 128)
        || request.evidence.len() > 32
    {
        return Err(bad_request(
            "autonomy item exceeds its bounded input limits",
        ));
    }
    if request.evidence.iter().any(|entry| {
        entry.kind.trim().is_empty()
            || entry.reference_id.trim().is_empty()
            || entry.kind.len() > 64
            || entry.reference_id.len() > 512
            || entry.label.as_ref().is_some_and(|label| label.len() > 256)
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
        approval_policy: request.approval_policy,
        source_session_id: request
            .source_session_id
            .map(|session_id| session_id.trim().to_string()),
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

/// Explicitly resume one approved decision thread in a fresh session.
///
/// This is the only HTTP path that can attach a ledger packet to execution.
/// The client supplies only the URL id by clicking the corresponding Mission
/// Control item; packet construction, source-agent resolution, audit, and the
/// execution prompt are all server-owned.
pub async fn resume_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_request): Json<ResumeRequest>,
) -> Result<(StatusCode, Json<AutonomyResumeResponse>), (StatusCode, Json<ErrorResponse>)> {
    if id.is_empty() || id.len() > 128 {
        return Err(bad_request("invalid autonomy item id"));
    }
    let item = state
        .autonomy
        .get(&id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found(&id))?;
    if item.state != AutonomyState::Approved {
        return Err(conflict(
            "decision thread must be approved before it can resume",
        ));
    }
    let source_session_id = item
        .source_session_id
        .filter(|session_id| !session_id.trim().is_empty())
        .ok_or_else(|| conflict("decision thread has no source session"))?;
    let source_session = state
        .state_service
        .get_session(&source_session_id)
        .map_err(internal_error)?
        .ok_or_else(|| conflict("decision thread source session is unavailable"))?;
    if source_session.root_agent_id.trim().is_empty() {
        return Err(conflict("decision thread source agent is unavailable"));
    }

    // This performs a second approved-state check in one SQLite transaction,
    // validates all packet bounds, and commits `resume_requested` before any
    // runner call. Every failure above and here leaves execution untouched.
    let packet = state.autonomy.prepare_resume(&id).map_err(|error| {
        let message = error.to_string();
        if message.contains("not found") {
            not_found(&id)
        } else if message.contains("must be approved") {
            conflict("decision thread must be approved before it can resume")
        } else {
            internal_error(error)
        }
    })?;
    let conversation_id = format!("autonomy-{}", Uuid::now_v7());
    let (_handle, session_id) = state
        .runtime
        .invoke_ledger_resume(&source_session.root_agent_id, &conversation_id, packet)
        .await
        .map_err(internal_error)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(AutonomyResumeResponse {
            item_id: id,
            session_id,
        }),
    ))
}

/// Project timer eligibility without scheduling or executing anything.
pub async fn eligibility(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EligibilityQuery>,
) -> Result<Json<AutonomyEligibilityResponse>, (StatusCode, Json<ErrorResponse>)> {
    if query.trigger != "timer" {
        return Err(bad_request("only timer eligibility is supported"));
    }
    let item = state
        .autonomy
        .get(&id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found(&id))?;
    let (eligible, reason) = match (item.state, item.approval_policy) {
        (AutonomyState::Approved, AutonomyApprovalPolicy::AskOnce) => {
            (true, "approved_for_future_nonwriting_trigger")
        }
        (AutonomyState::Approved, AutonomyApprovalPolicy::AutoReadonly) => {
            (true, "approved_for_future_nonwriting_trigger")
        }
        (AutonomyState::Approved, AutonomyApprovalPolicy::Manual) => {
            (false, "manual_resume_required")
        }
        _ => (false, "decision_thread_not_approved"),
    };
    Ok(Json(AutonomyEligibilityResponse {
        item_id: item.id,
        state: item.state,
        approval_policy: item.approval_policy,
        trigger: query.trigger,
        eligible,
        reason,
        read_only: true,
        scheduler_configured: false,
        execution_started: false,
    }))
}

pub async fn transition_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<TransitionRequest>,
) -> Result<Json<AutonomyDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let outcome = request
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|outcome| !outcome.is_empty());
    if request.outcome.is_some() && outcome.is_none() {
        return Err(bad_request("transition outcome must not be blank"));
    }
    if outcome.is_some_and(|entry| entry.len() > TRANSITION_OUTCOME_MAX_BYTES) {
        return Err(bad_request(
            "transition outcome exceeds its bounded input limit",
        ));
    }
    let item = state
        .autonomy
        .transition(&id, request.state, outcome)
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
        Json(ErrorResponse::new(message.to_string())),
    )
}

fn not_found(id: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new(format!("autonomy item not found: {id}"))),
    )
}

fn conflict(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse::new(message.to_string())),
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "autonomy ledger request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(
            "internal autonomy ledger error".to_string(),
        )),
    )
}
