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

// ============================================================================
// Handlers
// ============================================================================

/// List all providers
async fn list_providers(State(state): State<AppState>) -> impl IntoResponse {
    match state.provider_service.list() {
        Ok(mut providers) => {
            for p in &mut providers {
                enrich_provider(p, &state.model_registry);
            }
            Json(providers).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Get a single provider
async fn get_provider(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.provider_service.get(&id) {
        Ok(mut provider) => {
            enrich_provider(&mut provider, &state.model_registry);
            Json(provider).into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Create a new provider
async fn create_provider(
    State(state): State<AppState>,
    Json(provider): Json<Provider>,
) -> impl IntoResponse {
    match state.provider_service.create(provider) {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Update an existing provider
async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(provider): Json<Provider>,
) -> impl IntoResponse {
    match state.provider_service.update(&id, provider) {
        Ok(updated) => Json(updated).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Delete a provider
async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.provider_service.delete(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Set a provider as the default
async fn set_default_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.provider_service.set_default(&id) {
        Ok(provider) => Json(provider).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Test a provider connection (by ID)
async fn test_provider(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.provider_service.get(&id) {
        Ok(mut provider) => {
            let result = state.provider_service.test(&provider).await;

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
                enrich_provider(&mut provider, &state.model_registry);
                let _ = state.provider_service.update(&id, provider);
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
    let result = state.provider_service.test(&provider).await;
    Json(result).into_response()
}
