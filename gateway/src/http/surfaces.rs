//! Gateway-owned handlers for catalog surface actions.
//!
//! The request carries only a stable action id and target. It never resolves a
//! tool, URL, shell command, or client-provided handler.

use agent_surfaces::{SurfaceActionId, SurfaceActionRequest};
use axum::{extract::State, http::StatusCode, Json};
use zbot_conversation::AutonomyState;

use crate::state::AppState;

use super::autonomy::{AutonomyDetailResponse, ErrorResponse};

pub async fn invoke_action(
    State(state): State<AppState>,
    Json(request): Json<SurfaceActionRequest>,
) -> Result<Json<AutonomyDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let next_state = match request.action_id {
        SurfaceActionId::LedgerApprove => AutonomyState::Approved,
        SurfaceActionId::LedgerBlock => AutonomyState::Blocked,
        SurfaceActionId::LedgerComplete => AutonomyState::Complete,
        // Inspect belongs to the existing read endpoint. Revision has no
        // registered domain operation yet, so fail closed instead of exposing
        // tool authority through the surface.
        SurfaceActionId::Inspect | SurfaceActionId::PlanRequestRevision => {
            return Err(rejected("surface action is not registered for mutation"));
        }
    };

    let current = state
        .autonomy
        .get(&request.target)
        .map_err(internal_error)?
        .ok_or_else(|| rejected("surface target does not exist"))?;
    if request.expected_state.as_deref() != Some(current.state.as_str()) {
        return Err(rejected("surface action state is stale or replayed"));
    }
    if !current.state.can_transition_to(next_state) {
        return Err(rejected(
            "surface action is not permitted from the target state",
        ));
    }

    let item = state
        .autonomy
        .transition(&request.target, next_state, None)
        .map_err(internal_error)?;
    tracing::info!(action_id = ?request.action_id, target = %request.target, resulting_state = %item.state, "surface action audited by autonomy ledger");
    let evidence = state
        .autonomy
        .evidence(&request.target)
        .map_err(internal_error)?;
    Ok(Json(AutonomyDetailResponse { item, evidence }))
}

fn rejected(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: message.to_owned(),
        }),
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "surface action rejected by gateway");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "surface action failed".to_owned(),
        }),
    )
}
