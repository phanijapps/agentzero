// ============================================================================
// PROVIDERS HTTP ENDPOINTS
// REST API for LLM provider management
// ============================================================================

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::services::providers::{ModelConfig, Provider};
use crate::state::AppState;
use gateway_services::models::ModelRegistry;

// ============================================================================
// Routes
// ============================================================================

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_providers).post(create_provider))
        .route(
            "/:id",
            get(get_provider)
                .put(update_provider)
                .delete(delete_provider),
        )
        .route("/:id/test", post(test_provider))
        .route("/:id/default", post(set_default_provider))
        .route("/test", post(test_provider_inline))
}

// ============================================================================
// Helpers
// ============================================================================

fn commissioning_mutation_pending(state: &AppState) -> bool {
    gateway_services::providers::ollama_cloud_commissioning_pending(state.paths().as_ref())
}

fn provider_mutation_guard() -> Result<tokio::sync::MutexGuard<'static, ()>, StatusCode> {
    gateway_services::providers::provider_mutation_lock()
        .try_lock()
        .map_err(|_| StatusCode::CONFLICT)
}

/// Enrich a provider's model list with fallback capabilities.
/// Only populates model_configs if it's None (doesn't overwrite user data).
fn enrich_provider(provider: &mut Provider, registry: &ModelRegistry) {
    // Inject default rate limits so UI always sees them
    if provider.rate_limits.is_none() {
        provider.rate_limits = Some(provider.effective_rate_limits());
    }

    if provider.model_configs.is_some() {
        return; // Already enriched or user-configured
    }

    let mut configs = std::collections::HashMap::new();
    for model_id in &provider.models {
        let profile = registry.get(model_id);
        configs.insert(
            model_id.clone(),
            ModelConfig {
                capabilities: profile.capabilities.clone(),
                max_input: Some(profile.context.input),
                max_output: profile.context.output,
                source: "registry".to_string(),
            },
        );
    }

    if !configs.is_empty() {
        provider.model_configs = Some(configs);
    }
}

fn public_provider(provider: Provider) -> serde_json::Value {
    let has_api_key = !provider.api_key.trim().is_empty();
    let mut value = serde_json::to_value(provider).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = value.as_object_mut() {
        object.remove("apiKey");
        object.insert(
            "hasApiKey".to_string(),
            serde_json::Value::Bool(has_api_key),
        );
    }
    value
}

// ============================================================================
// Handlers
// ============================================================================

/// List all providers
async fn list_providers(State(state): State<AppState>) -> impl IntoResponse {
    match state.provider_service().list() {
        Ok(mut providers) => {
            for p in &mut providers {
                enrich_provider(p, &state.model_registry());
            }
            Json(
                providers
                    .into_iter()
                    .map(public_provider)
                    .collect::<Vec<_>>(),
            )
            .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Get a single provider
async fn get_provider(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.provider_service().get(&id) {
        Ok(mut provider) => {
            enrich_provider(&mut provider, &state.model_registry());
            Json(public_provider(provider)).into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Create a new provider
async fn create_provider(
    State(state): State<AppState>,
    Json(provider): Json<Provider>,
) -> impl IntoResponse {
    let _guard = match provider_mutation_guard() {
        Ok(guard) => guard,
        Err(status) => return (status, "Provider mutation is already in progress").into_response(),
    };
    if commissioning_mutation_pending(&state) {
        return (StatusCode::CONFLICT, "Commissioning recovery is pending").into_response();
    }
    match state.provider_service().create(provider) {
        Ok(created) => (StatusCode::CREATED, Json(public_provider(created))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Update an existing provider
async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut provider): Json<Provider>,
) -> impl IntoResponse {
    let _guard = match provider_mutation_guard() {
        Ok(guard) => guard,
        Err(status) => return (status, "Provider mutation is already in progress").into_response(),
    };
    if commissioning_mutation_pending(&state) {
        return (StatusCode::CONFLICT, "Commissioning recovery is pending").into_response();
    }
    if provider.api_key.trim().is_empty() {
        if let Ok(existing) = state.provider_service().get(&id) {
            provider.api_key = existing.api_key;
        }
    }
    match state.provider_service().update(&id, provider) {
        Ok(updated) => Json(public_provider(updated)).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Delete a provider
async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let _guard = match provider_mutation_guard() {
        Ok(guard) => guard,
        Err(status) => return (status, "Provider mutation is already in progress").into_response(),
    };
    if commissioning_mutation_pending(&state) {
        return (StatusCode::CONFLICT, "Commissioning recovery is pending").into_response();
    }
    match state.provider_service().delete(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Set a provider as the default
async fn set_default_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let _guard = match provider_mutation_guard() {
        Ok(guard) => guard,
        Err(status) => return (status, "Provider mutation is already in progress").into_response(),
    };
    if commissioning_mutation_pending(&state) {
        return (StatusCode::CONFLICT, "Commissioning recovery is pending").into_response();
    }
    match state.provider_service().set_default(&id) {
        Ok(provider) => Json(public_provider(provider)).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Test a provider connection (by ID)
async fn test_provider(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let _guard = match provider_mutation_guard() {
        Ok(guard) => guard,
        Err(status) => return (status, "Provider mutation is already in progress").into_response(),
    };
    if commissioning_mutation_pending(&state) {
        return (StatusCode::CONFLICT, "Commissioning recovery is pending").into_response();
    }
    match state.provider_service().get(&id) {
        Ok(mut provider) => {
            let result = state.provider_service().test(&provider).await;

            // Persist verified status + merge discovered models back to providers.json
            if result.success {
                provider.verified = Some(true);
                // Only populate models if the provider had none (empty list).
                // If the provider already has a curated model list (from preset),
                // keep it — don't pollute with every model the API discovers.
                if let Some(ref discovered) = result.models {
                    if !discovered.is_empty() && provider.models.is_empty() {
                        provider.models = discovered.clone();
                    }
                }
                // Enrich with registry capabilities
                enrich_provider(&mut provider, &state.model_registry());
                let _ = state.provider_service().update(&id, provider);
            }

            Json(result).into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Test a provider connection (inline, without saving)
async fn test_provider_inline(
    State(state): State<AppState>,
    Json(provider): Json<Provider>,
) -> impl IntoResponse {
    let result = state.provider_service().test(&provider).await;
    Json(result).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_provider_never_serializes_the_api_key() {
        let provider: Provider = serde_json::from_value(serde_json::json!({
            "id": "provider-ollama-cloud",
            "name": "Ollama Cloud",
            "description": "Ollama Cloud API",
            "apiKey": "sentinel-secret",
            "baseUrl": "https://ollama.com/v1",
            "models": ["glm-5.2:cloud"],
            "isDefault": false
        }))
        .unwrap();

        let response = public_provider(provider);
        assert_eq!(response.get("hasApiKey"), Some(&serde_json::json!(true)));
        assert!(response.get("apiKey").is_none());
        assert!(!response.to_string().contains("sentinel-secret"));
    }
}
