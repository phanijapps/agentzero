//! GET /api/network/info — exposes the current LAN discoverability state
//! to the Settings UI.

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use discovery::NetworkInfo;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct NetworkInfoResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<NetworkInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn get_network_info(
    State(state): State<AppState>,
) -> Result<Json<NetworkInfoResponse>, (StatusCode, Json<NetworkInfoResponse>)> {
    let settings = state.settings.load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(NetworkInfoResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )
    })?;
    let network_cfg = settings.network;

    let mdns_active = state
        .advertise_handle
        .lock()
        .ok()
        .map(|guard| guard.is_some())
        .unwrap_or(false);

    let alias_claimed = if mdns_active {
        // Pull from handle if available; otherwise optimistic true.
        state
            .advertise_handle
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.alias_claimed))
            .unwrap_or(true)
    } else {
        false
    };

    let instance_name = network_cfg
        .discovery
        .instance_name
        .clone()
        .unwrap_or_else(crate::server::default_instance_name);
    let instance_id = network_cfg
        .discovery
        .instance_id
        .clone()
        .unwrap_or_default();

    let bind_host = format!("{}", crate::config::resolve_bind_host(&network_cfg));

    let enumerator = discovery::RealEnumerator;
    let info = discovery::collect_network_info(
        &network_cfg,
        &bind_host,
        network_cfg.advanced.http_port,
        &enumerator,
        mdns_active,
        alias_claimed,
        &instance_name,
        &instance_id,
    );

    Ok(Json(NetworkInfoResponse {
        success: true,
        data: Some(info),
        error: None,
    }))
}
