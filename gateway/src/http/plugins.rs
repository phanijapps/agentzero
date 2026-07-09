//! # Plugin HTTP API
//!
//! REST API endpoints for plugin management.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use gateway_bridge::PluginSummary;
use gateway_services::PluginService;
use serde::Deserialize;

/// Response for list plugins endpoint.
#[derive(Debug, serde::Serialize)]
pub struct PluginListResponse {
    /// List of plugins.
    pub plugins: Vec<PluginSummary>,
    /// Total count.
    pub total: usize,
}

/// Response for plugin action endpoints.
#[derive(Debug, serde::Serialize)]
pub struct PluginActionResponse {
    /// Success status.
    pub success: bool,
    /// Message describing the result.
    pub message: String,
    /// Updated plugin summary (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginSummary>,
}

/// Response for config endpoint (secrets are masked).
#[derive(Debug, serde::Serialize)]
pub struct PluginConfigResponse {
    /// Plugin ID.
    pub plugin_id: String,
    /// Whether plugin is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// User-defined settings.
    #[serde(default)]
    pub settings: std::collections::HashMap<String, serde_json::Value>,
    /// Secret keys (values are masked).
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// Request body for updating config.
#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    /// Whether plugin is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// User-defined settings.
    #[serde(default)]
    pub settings: Option<std::collections::HashMap<String, serde_json::Value>>,
}

/// Request body for setting a secret.
#[derive(Debug, Deserialize)]
pub struct SetSecretRequest {
    /// Secret value.
    pub value: String,
}

/// Response for secrets list.
#[derive(Debug, serde::Serialize)]
pub struct SecretsListResponse {
    /// Plugin ID.
    pub plugin_id: String,
    /// Secret keys (values not included).
    pub secrets: Vec<String>,
}

/// List all plugins.
///
/// GET /api/plugins
pub async fn list_plugins(
    State(state): State<AppState>,
) -> Result<Json<PluginListResponse>, StatusCode> {
    let plugins = state.plugin_manager.list().await;
    let total = plugins.len();

    Ok(Json(PluginListResponse { plugins, total }))
}

/// Get a specific plugin by ID.
///
/// GET /api/plugins/:id
pub async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<PluginSummary>, StatusCode> {
    match state.plugin_manager.get(&plugin_id).await {
        Some(plugin) => Ok(Json(plugin)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// Start a plugin.
///
/// POST /api/plugins/:id/start
pub async fn start_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<PluginActionResponse>, StatusCode> {
    match state.plugin_manager.start(&plugin_id).await {
        Ok(()) => {
            let plugin = state.plugin_manager.get(&plugin_id).await;
            Ok(Json(PluginActionResponse {
                success: true,
                message: format!("Plugin '{}' started", plugin_id),
                plugin,
            }))
        }
        Err(e) => {
            let status = match e {
                gateway_bridge::PluginError::NotFound(_) => StatusCode::NOT_FOUND,
                gateway_bridge::PluginError::AlreadyRunning(_) => StatusCode::CONFLICT,
                gateway_bridge::PluginError::Disabled(_) => StatusCode::FORBIDDEN,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(status)
        }
    }
}

/// Stop a plugin.
///
/// POST /api/plugins/:id/stop
pub async fn stop_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<PluginActionResponse>, StatusCode> {
    match state.plugin_manager.stop(&plugin_id).await {
        Ok(()) => {
            let plugin = state.plugin_manager.get(&plugin_id).await;
            Ok(Json(PluginActionResponse {
                success: true,
                message: format!("Plugin '{}' stopped", plugin_id),
                plugin,
            }))
        }
        Err(e) => {
            let status = match e {
                gateway_bridge::PluginError::NotFound(_) => StatusCode::NOT_FOUND,
                gateway_bridge::PluginError::NotRunning(_) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(status)
        }
    }
}

/// Restart a plugin.
///
/// POST /api/plugins/:id/restart
pub async fn restart_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<PluginActionResponse>, StatusCode> {
    match state.plugin_manager.restart(&plugin_id).await {
        Ok(()) => {
            let plugin = state.plugin_manager.get(&plugin_id).await;
            Ok(Json(PluginActionResponse {
                success: true,
                message: format!("Plugin '{}' restarted", plugin_id),
                plugin,
            }))
        }
        Err(e) => {
            let status = match e {
                gateway_bridge::PluginError::NotFound(_) => StatusCode::NOT_FOUND,
                gateway_bridge::PluginError::Disabled(_) => StatusCode::FORBIDDEN,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(status)
        }
    }
}

/// Re-scan plugins directory for new plugins.
///
/// POST /api/plugins/discover
pub async fn discover_plugins(
    State(state): State<AppState>,
) -> Result<Json<PluginActionResponse>, StatusCode> {
    match state.plugin_manager.discover().await {
        Ok(discovered) => {
            let message = if discovered.is_empty() {
                "No new plugins discovered".to_string()
            } else {
                format!(
                    "Discovered {} new plugin(s): {:?}",
                    discovered.len(),
                    discovered
                )
            };

            Ok(Json(PluginActionResponse {
                success: true,
                message,
                plugin: None,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to discover plugins: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ============================================================================
// Config Endpoints
// ============================================================================

/// Get plugin configuration.
///
/// GET /api/plugins/:id/config
pub async fn get_plugin_config(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<PluginConfigResponse>, StatusCode> {
    // Check if plugin exists
    if !state.plugin_manager.exists(&plugin_id).await {
        return Err(StatusCode::NOT_FOUND);
    }

    let service = PluginService::new(state.paths.plugins_dir());
    let config = service.load_config(&plugin_id);

    let secrets = config.secret_keys();
    let enabled = config.enabled;
    let settings = config.settings.clone();

    Ok(Json(PluginConfigResponse {
        plugin_id,
        enabled,
        settings,
        secrets,
    }))
}

/// Update plugin configuration.
///
/// PUT /api/plugins/:id/config
pub async fn update_plugin_config(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(body): Json<UpdateConfigRequest>,
) -> Result<Json<PluginConfigResponse>, StatusCode> {
    // Check if plugin exists
    if !state.plugin_manager.exists(&plugin_id).await {
        return Err(StatusCode::NOT_FOUND);
    }

    let service = PluginService::new(state.paths.plugins_dir());
    let mut config = service.load_config(&plugin_id);

    // Update enabled if provided
    if let Some(enabled) = body.enabled {
        config.enabled = Some(enabled);
    }

    // Update settings if provided (replace entirely)
    if let Some(settings) = body.settings {
        config.settings = settings.into_iter().collect();
    }

    // Save updated config
    if let Err(e) = service.save_config(&plugin_id, &config) {
        tracing::error!("Failed to save plugin config: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let secrets = config.secret_keys();
    let enabled = config.enabled;
    let settings = config.settings.clone();

    Ok(Json(PluginConfigResponse {
        plugin_id,
        enabled,
        settings,
        secrets,
    }))
}

/// List secret keys for a plugin.
///
/// GET /api/plugins/:id/secrets
pub async fn list_plugin_secrets(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<SecretsListResponse>, StatusCode> {
    // Check if plugin exists
    if !state.plugin_manager.exists(&plugin_id).await {
        return Err(StatusCode::NOT_FOUND);
    }

    let service = PluginService::new(state.paths.plugins_dir());
    let secrets = service.list_secret_keys(&plugin_id);

    Ok(Json(SecretsListResponse {
        plugin_id: plugin_id.clone(),
        secrets,
    }))
}

/// Set a secret value.
///
/// PUT /api/plugins/:id/secrets/:key
pub async fn set_plugin_secret(
    State(state): State<AppState>,
    Path((plugin_id, key)): Path<(String, String)>,
    Json(body): Json<SetSecretRequest>,
) -> Result<Json<SecretsListResponse>, StatusCode> {
    // Check if plugin exists
    if !state.plugin_manager.exists(&plugin_id).await {
        return Err(StatusCode::NOT_FOUND);
    }

    let service = PluginService::new(state.paths.plugins_dir());

    if let Err(e) = service.set_secret(&plugin_id, key, body.value) {
        tracing::error!("Failed to set plugin secret: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let secrets = service.list_secret_keys(&plugin_id);

    Ok(Json(SecretsListResponse {
        plugin_id: plugin_id.clone(),
        secrets,
    }))
}

/// Delete a secret.
///
/// DELETE /api/plugins/:id/secrets/:key
pub async fn delete_plugin_secret(
    State(state): State<AppState>,
    Path((plugin_id, key)): Path<(String, String)>,
) -> Result<Json<SecretsListResponse>, StatusCode> {
    // Check if plugin exists
    if !state.plugin_manager.exists(&plugin_id).await {
        return Err(StatusCode::NOT_FOUND);
    }

    let service = PluginService::new(state.paths.plugins_dir());

    match service.delete_secret(&plugin_id, &key) {
        Ok(_) => {
            let secrets = service.list_secret_keys(&plugin_id);
            Ok(Json(SecretsListResponse {
                plugin_id: plugin_id.clone(),
                secrets,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to delete plugin secret: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
