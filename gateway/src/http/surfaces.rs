//! Gateway-owned handlers for catalog surface actions.
//!
//! The request carries only a stable action id and target. It never resolves a
//! tool, URL, shell command, or client-provided handler.

use agent_surfaces::{
    is_persistable_surface, SurfaceActionId, SurfaceActionRequest, SurfaceValidator, WorkSurface,
    ZbotWorkSurfaceCatalog, MAX_SURFACE_BYTES,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use execution_state::SessionSurfaceRecord;
use serde::{Deserialize, Serialize};
use zbot_conversation::AutonomyState;

use crate::state::AppState;

use super::autonomy::AutonomyDetailResponse;
use super::ErrorResponse;
use super::{HttpErrorResponse, SameOrigin};

/// Saved surface with the execution that produced it — the UI interleaves
/// surfaces into the chat timeline under their turn (execution id).
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct SavedSurfaceResponse {
    pub execution_id: String,
    /// Session the surface was persisted under — the ward-agent's child
    /// session for subagent-created surfaces. The UI matches subagent turns
    /// by session id (snapshot) or execution id (live), so both keys ride.
    pub session_id: String,
    /// When the surface was created. Root executions span multiple user
    /// turns with continuations, so execution ids cannot attribute a
    /// surface to its turn — the UI places surfaces by time window and
    /// falls back to the id keys for legacy rows.
    pub created_at: String,
    pub surface: WorkSurface,
}

pub async fn list_saved_session_surfaces(
    State(state): State<AppState>,
    _origin: SameOrigin,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<SavedSurfaceResponse>>, (StatusCode, Json<HttpErrorResponse>)> {
    if session_id.is_empty() || session_id.len() > 128 {
        return Err(bad_request("invalid session id"));
    }
    if !state.state_service.surface_persistence_enabled() {
        return Ok(Json(Vec::new()));
    }
    let records = state
        .state_service
        .list_session_surfaces(&session_id)
        .map_err(|_| internal_surface_error())?;
    Ok(Json(decode_saved_surfaces(records)))
}

fn decode_saved_surfaces(records: Vec<SessionSurfaceRecord>) -> Vec<SavedSurfaceResponse> {
    let mut surfaces = Vec::with_capacity(records.len());
    for record in records {
        if record.surface_json.len() > MAX_SURFACE_BYTES {
            tracing::warn!(
                event = "saved_surface_rejected",
                session_id = %record.session_id,
                surface_id = %record.surface_id,
                "omitting invalid saved work surface"
            );
            continue;
        }
        let parsed = serde_json::from_str::<WorkSurface>(&record.surface_json);
        match parsed {
            Ok(surface)
                if surface.surface_id == record.surface_id
                    && ZbotWorkSurfaceCatalog.validate(&surface).is_ok()
                    && is_persistable_surface(&surface) =>
            {
                surfaces.push(SavedSurfaceResponse {
                    execution_id: record.execution_id,
                    session_id: record.session_id,
                    created_at: record.created_at,
                    surface,
                });
            }
            _ => tracing::warn!(
                event = "saved_surface_rejected",
                session_id = %record.session_id,
                surface_id = %record.surface_id,
                "omitting invalid saved work surface"
            ),
        }
    }
    surfaces
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearSavedSurfacesRequest {
    confirmation: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearSavedSurfacesResponse {
    deleted_count: usize,
}

pub async fn clear_saved_surfaces(
    State(state): State<AppState>,
    _origin: SameOrigin,
    Json(request): Json<ClearSavedSurfacesRequest>,
) -> Result<Json<ClearSavedSurfacesResponse>, (StatusCode, Json<HttpErrorResponse>)> {
    if request.confirmation != "clear_saved_infographics" {
        return Err(bad_request("invalid confirmation"));
    }
    let deleted_count = state
        .state_service
        .clear_session_surfaces()
        .map_err(|_| internal_surface_error())?;
    Ok(Json(ClearSavedSurfacesResponse { deleted_count }))
}

fn bad_request(message: &str) -> (StatusCode, Json<HttpErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(HttpErrorResponse {
            error: message.to_owned(),
        }),
    )
}

fn internal_surface_error() -> (StatusCode, Json<HttpErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(HttpErrorResponse {
            error: "saved surface operation failed".to_owned(),
        }),
    )
}

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
        Json(ErrorResponse::new(message.to_owned())),
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "surface action rejected by gateway");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new("surface action failed".to_owned())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_surfaces::{ComponentType, SurfaceComponent, ZBOT_WORK_SURFACE_CATALOG};
    use std::collections::BTreeMap;

    fn record(surface: &WorkSurface) -> SessionSurfaceRecord {
        SessionSurfaceRecord {
            session_id: "sess-1".to_owned(),
            surface_id: surface.surface_id.clone(),
            execution_id: "exec-1".to_owned(),
            surface_json: serde_json::to_string(surface).unwrap(),
            created_at: "2026-07-28T00:00:00Z".to_owned(),
            updated_at: "2026-07-28T00:00:00Z".to_owned(),
        }
    }

    fn surface(component_type: ComponentType) -> WorkSurface {
        let props = match component_type {
            ComponentType::PlanChecklist => BTreeMap::from([
                (
                    "title".to_owned(),
                    serde_json::Value::String("Saved".to_owned()),
                ),
                (
                    "plan_path".to_owned(),
                    serde_json::Value::String("/plan".to_owned()),
                ),
            ]),
            ComponentType::ApprovalGate => BTreeMap::from([
                (
                    "title".to_owned(),
                    serde_json::Value::String("Approve".to_owned()),
                ),
                (
                    "action_id".to_owned(),
                    serde_json::Value::String("ledger_approve".to_owned()),
                ),
                (
                    "target".to_owned(),
                    serde_json::Value::String("item-1".to_owned()),
                ),
                (
                    "expected_state".to_owned(),
                    serde_json::Value::String("proposed".to_owned()),
                ),
            ]),
            _ => BTreeMap::new(),
        };
        WorkSurface {
            surface_id: "surface-1".to_owned(),
            catalog_id: ZBOT_WORK_SURFACE_CATALOG.to_owned(),
            components: vec![SurfaceComponent {
                id: "component-1".to_owned(),
                component_type,
                props,
            }],
            data: serde_json::json!({"plan": []}),
        }
    }

    #[test]
    fn saved_surface_decode_omits_corrupt_and_actionable_rows() {
        // STUB: AC5 — read validation fails closed without returning raw data.
        let display = surface(ComponentType::PlanChecklist);
        let actionable = surface(ComponentType::ApprovalGate);
        let corrupt = SessionSurfaceRecord {
            surface_json: "{secret-not-json".to_owned(),
            ..record(&display)
        };

        let oversized = SessionSurfaceRecord {
            surface_id: "oversized".to_owned(),
            surface_json: "x".repeat(MAX_SURFACE_BYTES + 1),
            ..record(&display)
        };
        let decoded = decode_saved_surfaces(vec![
            record(&display),
            record(&actionable),
            corrupt,
            oversized,
        ]);

        // Pair shape: the persisted execution id rides with the surface so
        // the UI can interleave surfaces under their producing turn.
        assert_eq!(
            decoded,
            vec![SavedSurfaceResponse {
                execution_id: "exec-1".to_owned(),
                session_id: "sess-1".to_owned(),
                created_at: "2026-07-28T00:00:00Z".to_owned(),
                surface: display,
            }]
        );
    }
}
