//! `GET /api/curator/usage[/:ward]` + `POST /api/curator/usage/:ward/pin` —
//! Phase A.3 read endpoints over the per-ward telemetry sidecar.
//!
//! Each handler constructs a fresh `WardUsage` from `state.paths` and lets
//! the service's internal `Mutex` serialise the read-modify-write. The
//! persistent writer (delegation `bump_use`) lives in `ExecutionRunner` —
//! the only cross-instance race is HTTP-pin vs. an in-flight delegation
//! bump, which is acceptable at this scale.
//!
//! Endpoints live under `/api/curator/` rather than `/api/wards/curator`
//! to keep them distinct from `/api/wards/:ward_id/...` (a ward named
//! "curator" would otherwise be unreachable through the latter).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use gateway_services::{WardRecord, WardUsage, WardUsageMap};
use serde::Deserialize;

use crate::state::AppState;

/// `GET /api/curator/usage` — full sidecar (empty map when missing).
pub async fn list_usage(State(state): State<AppState>) -> Json<WardUsageMap> {
    Json(WardUsage::new(state.paths.wards_dir()).load())
}

/// `GET /api/curator/usage/:ward` — single ward record. `404` if missing.
pub async fn get_usage(
    State(state): State<AppState>,
    Path(ward): Path<String>,
) -> Result<Json<WardRecord>, StatusCode> {
    match WardUsage::new(state.paths.wards_dir()).get(&ward) {
        Some(rec) => Ok(Json(rec)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Debug, Deserialize)]
pub struct PinPayload {
    pub pinned: bool,
}

/// `POST /api/curator/usage/:ward/pin` — toggle the curator opt-out flag.
/// Returns `404` when the ward has no usage record yet (no implicit
/// creation through this endpoint — the user must touch the ward through
/// normal channels first).
pub async fn set_pinned(
    State(state): State<AppState>,
    Path(ward): Path<String>,
    Json(payload): Json<PinPayload>,
) -> Result<StatusCode, StatusCode> {
    let usage = WardUsage::new(state.paths.wards_dir());
    if usage.get(&ward).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    usage
        .set_pinned(&ward, payload.pinned)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}
