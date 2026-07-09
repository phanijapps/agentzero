//! # Setup Endpoints
//!
//! Lightweight endpoints for the first-time setup wizard.

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

/// GET /api/setup/status — lightweight check for setup redirect logic.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub setup_complete: bool,
    pub has_providers: bool,
}

pub async fn get_setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatus>, StatusCode> {
    let setup_complete = state
        .settings
        .get_execution_settings()
        .map(|s| s.setup_complete)
        .unwrap_or(false);

    let has_providers = state
        .provider_service
        .list()
        .map(|providers| !providers.is_empty())
        .unwrap_or(false);

    Ok(Json(SetupStatus {
        setup_complete,
        has_providers,
    }))
}

/// GET /api/setup/mcp-defaults — sanitized MCP template for the wizard.
pub async fn get_mcp_defaults() -> Json<serde_json::Value> {
    let template = gateway_templates::Templates::get("default_mcps.json")
        .map(|f| serde_json::from_slice(&f.data).unwrap_or_else(|_| serde_json::json!([])))
        .unwrap_or_else(|| serde_json::json!([]));

    Json(template)
}
