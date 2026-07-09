// ============================================================================
// MODELS API
// Read-only endpoints for the model capabilities registry.
// ============================================================================

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

/// GET /api/models — List all known models with their capabilities.
pub async fn list_models(State(state): State<AppState>) -> Json<Value> {
    let entries = state.model_registry.list();
    let map: serde_json::Map<String, Value> = entries
        .into_iter()
        .filter_map(|(id, profile)| {
            serde_json::to_value(profile)
                .ok()
                .map(|v| (id.to_string(), v))
        })
        .collect();
    Json(Value::Object(map))
}

/// GET /api/models/:id — Get a single model profile.
pub async fn get_model(
    State(state): State<AppState>,
    Path(model_id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if state.model_registry.is_known(&model_id) {
        let profile = state.model_registry.get(&model_id);
        match serde_json::to_value(profile) {
            Ok(v) => Ok(Json(json!({ "id": model_id, "profile": v }))),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
