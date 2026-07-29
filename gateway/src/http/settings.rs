//! # Settings Endpoints
//!
//! HTTP endpoints for managing application settings.

use crate::state::AppState;
use agent_tools::ToolSettings;
use axum::{extract::State, http::StatusCode, Json};
use gateway_services::{ExecutionSettings, LogSettings, PresentationSettings};
use serde::{Deserialize, Serialize};

/// Response for settings endpoints.
#[derive(Debug, Serialize)]
pub struct SettingsResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// GET /api/settings/tools - Get tool settings.
pub async fn get_tool_settings(
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse<ToolSettings>>, (StatusCode, Json<SettingsResponse<()>>)> {
    match state.settings.get_tool_settings() {
        Ok(settings) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(settings),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

/// Request for updating live tool settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateToolSettingsRequest {
    #[serde(default)]
    pub file_tools: bool,
    #[serde(default)]
    pub offload_large_results: bool,
    #[serde(default = "default_offload_threshold")]
    pub offload_threshold_tokens: usize,
}

fn default_offload_threshold() -> usize {
    5000
}

impl From<UpdateToolSettingsRequest> for ToolSettings {
    fn from(req: UpdateToolSettingsRequest) -> Self {
        ToolSettings {
            file_tools: req.file_tools,
            offload_large_results: req.offload_large_results,
            offload_threshold_tokens: req.offload_threshold_tokens,
        }
    }
}

/// PUT /api/settings/tools - Update tool settings.
pub async fn update_tool_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateToolSettingsRequest>,
) -> Result<Json<SettingsResponse<ToolSettings>>, (StatusCode, Json<SettingsResponse<()>>)> {
    let settings: ToolSettings = request.into();

    match state.settings.update_tool_settings(settings.clone()) {
        Ok(()) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(settings),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSettingsResponse {
    #[serde(flatten)]
    pub settings: PresentationSettings,
    pub restart_required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdatePresentationSettingsRequest {
    pub persist_surfaces: bool,
}

pub async fn get_presentation_settings(
    State(state): State<AppState>,
    _origin: super::SameOrigin,
) -> Result<
    Json<SettingsResponse<PresentationSettingsResponse>>,
    (StatusCode, Json<SettingsResponse<()>>),
> {
    state
        .settings
        .get_presentation_settings()
        .map(|settings| {
            Json(SettingsResponse {
                success: true,
                data: Some(PresentationSettingsResponse {
                    settings,
                    restart_required: false,
                }),
                error: None,
            })
        })
        .map_err(internal_settings_error)
}

pub async fn update_presentation_settings(
    State(state): State<AppState>,
    _origin: super::SameOrigin,
    Json(request): Json<UpdatePresentationSettingsRequest>,
) -> Result<
    Json<SettingsResponse<PresentationSettingsResponse>>,
    (StatusCode, Json<SettingsResponse<()>>),
> {
    let settings = PresentationSettings {
        persist_surfaces: request.persist_surfaces,
    };
    state
        .settings
        .update_presentation_settings(settings)
        .map_err(internal_settings_error)?;
    state
        .state_service
        .set_surface_persistence_enabled(settings.persist_surfaces);
    Ok(Json(SettingsResponse {
        success: true,
        data: Some(PresentationSettingsResponse {
            settings,
            restart_required: false,
        }),
        error: None,
    }))
}

fn internal_settings_error(error: String) -> (StatusCode, Json<SettingsResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(SettingsResponse {
            success: false,
            data: None,
            error: Some(error),
        }),
    )
}

// ============================================================================
// LOG SETTINGS ENDPOINTS
// ============================================================================

/// Response wrapper for log settings with restart warning.
#[derive(Debug, Serialize)]
pub struct LogSettingsResponse {
    /// The current log settings
    #[serde(flatten)]
    pub settings: LogSettings,
    /// Warning message about restart requirement
    pub restart_required: bool,
}

/// GET /api/settings/logs - Get log settings.
///
/// Returns the current logging configuration including file logging,
/// rotation, and retention settings.
pub async fn get_log_settings(
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse<LogSettingsResponse>>, (StatusCode, Json<SettingsResponse<()>>)> {
    match state.settings.get_log_settings() {
        Ok(settings) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(LogSettingsResponse {
                settings,
                restart_required: true, // Always true - log changes require restart
            }),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

/// Request for updating log settings.
///
/// All fields are optional - only provided fields will be updated.
/// Changes require a daemon restart to take effect.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLogSettingsRequest {
    /// Enable file logging
    #[serde(default)]
    pub enabled: bool,
    /// Custom log directory (optional, defaults to {data_dir}/logs)
    #[serde(default)]
    pub directory: Option<std::path::PathBuf>,
    /// Log level: trace, debug, info, warn, error
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Rotation strategy: daily, hourly, minutely, never
    #[serde(default = "default_rotation")]
    pub rotation: String,
    /// Maximum log files to keep (0 = unlimited)
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    /// Suppress stdout output (only log to file)
    #[serde(default)]
    pub suppress_stdout: bool,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_rotation() -> String {
    "daily".to_string()
}

fn default_max_files() -> usize {
    7
}

impl From<UpdateLogSettingsRequest> for LogSettings {
    fn from(req: UpdateLogSettingsRequest) -> Self {
        LogSettings {
            enabled: req.enabled,
            directory: req.directory,
            level: req.level,
            rotation: req.rotation,
            max_files: req.max_files,
            suppress_stdout: req.suppress_stdout,
        }
    }
}

/// PUT /api/settings/logs - Update log settings.
///
/// Updates the logging configuration. Changes require a daemon restart
/// to take effect. The response includes the updated settings and
/// a reminder about the restart requirement.
pub async fn update_log_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateLogSettingsRequest>,
) -> Result<Json<SettingsResponse<LogSettingsResponse>>, (StatusCode, Json<SettingsResponse<()>>)> {
    let settings: LogSettings = request.into();

    match state.settings.update_log_settings(settings.clone()) {
        Ok(()) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(LogSettingsResponse {
                settings,
                restart_required: true,
            }),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

// ============================================================================
// EXECUTION SETTINGS ENDPOINTS
// ============================================================================

/// Response wrapper for execution settings with restart warning.
#[derive(Debug, Serialize)]
pub struct ExecutionSettingsResponse {
    /// The current execution settings
    #[serde(flatten)]
    pub settings: ExecutionSettings,
    /// Changes require daemon restart to take effect
    pub restart_required: bool,
}

/// GET /api/settings/execution - Get execution settings.
pub async fn get_execution_settings(
    State(state): State<AppState>,
) -> Result<
    Json<SettingsResponse<ExecutionSettingsResponse>>,
    (StatusCode, Json<SettingsResponse<()>>),
> {
    match state.settings.get_execution_settings() {
        Ok(settings) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(ExecutionSettingsResponse {
                settings,
                restart_required: false,
            }),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

/// Request for updating execution settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateExecutionSettingsRequest {
    /// Maximum parallel subagents (default: 2)
    #[serde(default = "default_max_parallel")]
    pub max_parallel_agents: u32,
    /// Whether the first-time setup wizard has been completed (default: false)
    #[serde(default)]
    pub setup_complete: bool,
    /// The user-chosen name for the root agent
    #[serde(default)]
    pub agent_name: Option<String>,
    /// Disable streaming for subagents (default: true)
    #[serde(default = "default_non_streaming")]
    pub subagent_non_streaming: bool,
    /// Orchestrator (root agent) configuration
    #[serde(default)]
    pub orchestrator: Option<gateway_services::OrchestratorConfig>,
    /// Distillation (memory extraction) model configuration
    #[serde(default)]
    pub distillation: Option<gateway_services::DistillationConfig>,
    /// Ward-curator model configuration (Phase C consolidation LLM call)
    #[serde(default)]
    pub curator: Option<gateway_services::CuratorConfig>,
    /// Intent-analysis model configuration (every root prompt)
    #[serde(default)]
    pub intent_analysis: Option<gateway_services::IntentAnalysisConfig>,
    /// Default multimodal (vision) model configuration
    #[serde(default)]
    pub multimodal: Option<gateway_services::MultimodalConfig>,
    /// Background memory worker configuration.
    #[serde(default)]
    pub memory: Option<gateway_services::MemorySettings>,
    /// Experimental UI feature flags (free-form bag).
    #[serde(default)]
    pub feature_flags: std::collections::HashMap<String, bool>,
}

fn default_max_parallel() -> u32 {
    2
}
fn default_non_streaming() -> bool {
    true
}

impl UpdateExecutionSettingsRequest {
    /// Apply this request to existing execution settings, producing the new
    /// settings to persist.
    ///
    /// The request represents the user-configurable subset of
    /// [`ExecutionSettings`]. Fields present on [`ExecutionSettings`] but
    /// absent from the request are **runtime state** and must be preserved
    /// across updates. Currently this is:
    ///
    /// - [`ExecutionSettings::chat`] — session/conversation IDs owned by
    ///   `/api/chat/init` for the persistent chat session. Resetting these
    ///   causes orphan `sess-chat-*` records (the chat UI will create a
    ///   fresh session on next load, so the old one never receives any
    ///   messages and gets marked crashed on daemon restart).
    ///
    /// When adding a new field to [`ExecutionSettings`], decide whether it is
    /// user-configurable (add to the request, accept from client) or runtime
    /// state (preserve here). Never lose runtime state on config updates.
    pub fn merge_into(self, existing: &ExecutionSettings) -> ExecutionSettings {
        ExecutionSettings {
            // User-configurable fields (taken from request)
            max_parallel_agents: self.max_parallel_agents,
            setup_complete: self.setup_complete,
            agent_name: self.agent_name,
            subagent_non_streaming: self.subagent_non_streaming,
            orchestrator: self.orchestrator.unwrap_or_default(),
            distillation: self.distillation.unwrap_or_default(),
            curator: self.curator.unwrap_or_default(),
            intent_analysis: self.intent_analysis.unwrap_or_default(),
            multimodal: self.multimodal.unwrap_or_default(),
            feature_flags: self.feature_flags,

            // User-configurable with fallback to existing
            memory: self.memory.unwrap_or_else(|| existing.memory.clone()),

            // Runtime state (preserved from existing settings)
            chat: existing.chat.clone(),
            wiki: existing.wiki.clone(),
        }
    }
}

/// PUT /api/settings/execution - Update execution settings.
///
/// Changes to max_parallel_agents require a daemon restart.
/// When agent_name is set, also updates SOUL.md with the new identity.
pub async fn update_execution_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateExecutionSettingsRequest>,
) -> Result<
    Json<SettingsResponse<ExecutionSettingsResponse>>,
    (StatusCode, Json<SettingsResponse<()>>),
> {
    // Read existing settings first so the request merge can preserve runtime
    // state (e.g. the persistent chat session IDs). If no settings exist yet,
    // merge into defaults — the fresh chat config will remain empty and
    // `/api/chat/init` will lazily populate it.
    let existing = state.settings.get_execution_settings().unwrap_or_default();
    let settings: ExecutionSettings = request.merge_into(&existing);

    // Update SOUL.md if agent_name is provided
    if let Some(ref name) = settings.agent_name {
        let soul_path = state.paths.soul();
        let current = std::fs::read_to_string(&soul_path).unwrap_or_default();
        // Replace the first line "You are **OldName**" with the new name
        let updated = if let Some(rest) = current.strip_prefix("You are **") {
            if let Some(after_name) = rest.find("**") {
                format!("You are **{}**{}", name, &rest[after_name + 2..])
            } else {
                format!("You are **{}**, an autonomous agent.\n\n{}", name, current)
            }
        } else {
            format!("You are **{}**, an autonomous agent.\n\n{}", name, current)
        };
        if let Err(e) = std::fs::write(&soul_path, &updated) {
            tracing::warn!("Failed to update SOUL.md: {}", e);
        }
    }

    match state.settings.update_execution_settings(settings.clone()) {
        Ok(()) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(ExecutionSettingsResponse {
                settings,
                restart_required: true,
            }),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

// ============================================================================
// NETWORK SETTINGS ENDPOINTS
// ============================================================================

/// GET /api/settings/network — current network/discovery configuration.
pub async fn get_network_settings(
    State(state): State<AppState>,
) -> Result<
    Json<SettingsResponse<discovery::DiscoveryConfig>>,
    (StatusCode, Json<SettingsResponse<()>>),
> {
    match state.settings.load() {
        Ok(settings) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(settings.network),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

/// PUT /api/settings/network — replace the network/discovery block.
///
/// Note: changes apply on next daemon restart; the UI shows a banner
/// reminding the user.
pub async fn update_network_settings(
    State(state): State<AppState>,
    Json(new_cfg): Json<discovery::DiscoveryConfig>,
) -> Result<
    Json<SettingsResponse<discovery::DiscoveryConfig>>,
    (StatusCode, Json<SettingsResponse<()>>),
> {
    let mut current = match state.settings.load() {
        Ok(s) => s,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SettingsResponse {
                    success: false,
                    data: None,
                    error: Some(e),
                }),
            ));
        }
    };
    current.network = new_cfg.clone();
    match state.settings.save(&current) {
        Ok(()) => Ok(Json(SettingsResponse {
            success: true,
            data: Some(new_cfg),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SettingsResponse {
                success: false,
                data: None,
                error: Some(e),
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::{ChatConfig, ExecutionSettings};

    fn sample_request() -> UpdateExecutionSettingsRequest {
        UpdateExecutionSettingsRequest {
            max_parallel_agents: 4,
            setup_complete: true,
            agent_name: Some("Arthur".to_string()),
            subagent_non_streaming: false,
            orchestrator: None,
            distillation: None,
            curator: None,
            intent_analysis: None,
            multimodal: None,
            memory: None,
            feature_flags: std::collections::HashMap::new(),
        }
    }

    /// Regression test: updating execution settings must preserve the chat
    /// config, which is runtime state owned by `/api/chat/init`, not a
    /// user-configurable field.
    ///
    /// If this invariant breaks, the chat UI will create orphan
    /// `sess-chat-*` sessions on every subsequent page load because its
    /// idempotency check (`settings.chat.session_id.is_some()`) will fail.
    #[test]
    fn merge_preserves_chat_config() {
        let existing = ExecutionSettings {
            chat: ChatConfig {
                session_id: Some("sess-chat-preserved".to_string()),
                conversation_id: Some("chat-preserved".to_string()),
            },
            ..Default::default()
        };

        let merged = sample_request().merge_into(&existing);

        assert_eq!(
            merged.chat.session_id.as_deref(),
            Some("sess-chat-preserved"),
            "session_id must be preserved across settings updates",
        );
        assert_eq!(
            merged.chat.conversation_id.as_deref(),
            Some("chat-preserved"),
            "conversation_id must be preserved across settings updates",
        );
    }

    /// When the request carries new user-configurable values, they must
    /// replace the existing values (merge semantics only apply to runtime
    /// state, not user config).
    #[test]
    fn merge_overwrites_user_configurable_fields() {
        let existing = ExecutionSettings {
            max_parallel_agents: 1,
            setup_complete: false,
            agent_name: Some("OldName".to_string()),
            ..Default::default()
        };

        let merged = sample_request().merge_into(&existing);

        assert_eq!(merged.max_parallel_agents, 4);
        assert!(merged.setup_complete);
        assert_eq!(merged.agent_name.as_deref(), Some("Arthur"));
    }

    /// When no existing settings exist (fresh install), the merge falls back
    /// to default chat config. `/api/chat/init` then lazily populates it.
    #[test]
    fn merge_with_default_existing_leaves_chat_empty() {
        let merged = sample_request().merge_into(&ExecutionSettings::default());

        assert!(merged.chat.session_id.is_none());
        assert!(merged.chat.conversation_id.is_none());
    }
}
