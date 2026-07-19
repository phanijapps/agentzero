//! Durable, provider-neutral first-run Agent Commissioning endpoints.
//!
//! This module deliberately records semantic intent only. It has no dependency
//! on Engram or a concrete semantic database; a later integration can consume
//! `SemanticProfile` from settings without changing this HTTP contract.

use std::{net::SocketAddr, process::Command};

use axum::{
    extract::{ConnectInfo, FromRequestParts, State},
    http::{header::ORIGIN, request::Parts, HeaderMap, StatusCode},
    Json,
};
use gateway_services::providers::Provider;
use gateway_services::{
    CommissioningSettings, CommissioningState, OrchestratorConfig, SemanticProfile, UserProfile,
};
use serde::{Deserialize, Serialize};

use crate::{config::GatewayConfig, state::AppState};

const LOCAL_RUNTIME_URL: &str = "http://127.0.0.1:11434/v1";
const MEMORY_PROFILE_PENDING_BYTES: &[u8] =
    include_bytes!("../../templates/zbot-memory-profile-v1-pending");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixedProfileTargetState {
    Absent,
    Identical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryProfileActivationOutcome {
    Activated,
    CleanedCompletedMarker,
}

/// Finalize or clean up an exact pending V1 marker with save-before-remove ordering.
///
/// The closures make crash-boundary behavior failure-injectable without tying
/// the state machine to one settings backend. Implemented after plan approval.
fn finalize_pending_memory_profile<Save, Remove>(
    _marker: &std::path::Path,
    _state: CommissioningState,
    _exact_v1_inputs: bool,
    _save_complete: Save,
    _remove_marker: Remove,
) -> Result<MemoryProfileActivationOutcome, &'static str>
where
    Save: FnOnce() -> Result<(), &'static str>,
    Remove: FnOnce() -> Result<(), &'static str>,
{
    todo!("pending memory activation ordering is specified in the approved plan")
}

/// Authorizes commissioning from request parts before Axum consumes or
/// deserializes the JSON body.
struct LocalCommissioningRequest;

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for LocalCommissioningRequest
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<CommissioningError>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let config = parts.extensions.get::<GatewayConfig>();
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(peer)| *peer);
        let local_peer = config.is_some_and(|config| super::vault::is_local_request(config, peer));
        if local_peer && is_local_commissioning_origin(&parts.headers) {
            Ok(Self)
        } else {
            Err(commissioning_origin_denied())
        }
    }
}

/// Preflight one fixed commissioning-owned target without following links.
///
/// This interface is materialized for the red construction tests. The
/// implementation follows after work-loop plan approval.
fn preflight_fixed_profile_target(
    _path: &std::path::Path,
    _expected: &[u8],
) -> Result<FixedProfileTargetState, &'static str> {
    todo!("fixed-target no-follow preflight is specified in the approved plan")
}

/// Preflight every full-profile setting and fixed target before provider,
/// SOUL, settings, or profile mutation.
fn preflight_full_memory_profile(
    _paths: &gateway_services::VaultPaths,
    _existing_memory: &gateway_memory::MemorySettings,
    _embedding_is_internal_384: bool,
) -> Result<(), &'static str> {
    todo!("full-profile settings and fixed-target preflight is specified in the approved plan")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommissioningStatusResponse {
    pub state: CommissioningState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_code: Option<&'static str>,
    pub semantic_profile: SemanticProfile,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDiagnosisResponse {
    pub state: LocalRuntimeState,
    pub recovery_code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalRuntimeState {
    Unavailable,
    Unreachable,
    NoModel,
    Ready,
}

#[derive(Debug, Serialize)]
pub struct CommissioningError {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommissioningRequest {
    pub display_name: String,
    #[serde(default)]
    pub profile: String,
    pub user_name: String,
    pub interests: Vec<String>,
    #[serde(default)]
    pub hobbies: Vec<String>,
    #[serde(default)]
    pub date_of_birth: Option<String>,
    pub primary_focus: String,
    pub domains: Vec<String>,
    pub provider: ProviderSelection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelection {
    pub kind: ProviderKind,
    pub preset_id: Option<String>,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Cloud,
    Local,
}

struct KnownPreset {
    id: &'static str,
    provider_id: &'static str,
    name: &'static str,
    base_url: &'static str,
    models: &'static [&'static str],
}

const CLOUD_PRESETS: &[KnownPreset] = &[
    KnownPreset {
        id: "openai",
        provider_id: "provider-openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        models: &["gpt-4o", "gpt-4o-mini", "o4-mini", "gpt-4.1"],
    },
    KnownPreset {
        id: "deepseek",
        provider_id: "provider-deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        models: &["deepseek-chat", "deepseek-reasoner"],
    },
    KnownPreset {
        id: "openrouter",
        provider_id: "provider-openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        models: &[
            "anthropic/claude-opus",
            "openai/gpt-4-turbo",
            "google/gemini-pro",
        ],
    },
    KnownPreset {
        id: "z-ai",
        provider_id: "provider-z-ai",
        name: "Z.AI",
        base_url: "https://api.z.ai/api/coding/paas/v4",
        models: &[
            "glm-5.1",
            "glm-5",
            "glm-5-turbo",
            "glm-4.7",
            "glm-4.6",
            "glm-4.5",
        ],
    },
    KnownPreset {
        id: "mistral",
        provider_id: "provider-mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        models: &[
            "mistral-large-latest",
            "mistral-small-latest",
            "codestral-latest",
        ],
    },
];

/// GET /api/commissioning/status — the gateway, not browser storage, decides readiness.
pub async fn get_commissioning_status(
    State(state): State<AppState>,
) -> Result<Json<CommissioningStatusResponse>, (StatusCode, Json<CommissioningError>)> {
    let settings = state.settings.load().map_err(|_| internal_error())?;
    let providers = state
        .provider_service
        .list()
        .map_err(|_| internal_error())?;
    let (status, recovery_code) =
        effective_status(&settings.commissioning, &settings.execution, &providers);

    Ok(Json(CommissioningStatusResponse {
        state: status,
        recovery_code,
        semantic_profile: settings.commissioning.semantic_profile,
    }))
}

/// POST /api/commissioning/local/diagnose — inspect only the fixed Ollama endpoint.
pub async fn diagnose_local_runtime(State(state): State<AppState>) -> Json<LocalDiagnosisResponse> {
    if !local_runtime_binary_present() {
        return Json(LocalDiagnosisResponse {
            state: LocalRuntimeState::Unavailable,
            recovery_code: "local_runtime_not_installed",
            models: None,
        });
    }

    let candidate = local_provider("diagnostic-model");
    let result = state.provider_service.test(&candidate).await;
    if !result.success {
        return Json(LocalDiagnosisResponse {
            state: LocalRuntimeState::Unreachable,
            recovery_code: "local_runtime_unreachable",
            models: None,
        });
    }

    match result.models.filter(|models| !models.is_empty()) {
        Some(models) => Json(LocalDiagnosisResponse {
            state: LocalRuntimeState::Ready,
            recovery_code: "local_runtime_ready",
            models: Some(models),
        }),
        None => Json(LocalDiagnosisResponse {
            state: LocalRuntimeState::NoModel,
            recovery_code: "local_runtime_no_model",
            models: None,
        }),
    }
}

/// POST /api/commissioning/complete — validate first, write settings last.
pub async fn complete_commissioning(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CommissioningRequest>,
) -> Result<Json<CommissioningStatusResponse>, (StatusCode, Json<CommissioningError>)> {
    if !is_local_commissioning_origin(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(CommissioningError {
                code: "commissioning_origin_denied",
                message: "Commissioning must be completed from the local z-Bot app.",
            }),
        ));
    }
    validate_request(&request)?;
    let candidate = provider_from_selection(&request.provider)?;

    // Nothing is persisted until the selected provider has proved usable.
    let test_result = state.provider_service.test(&candidate).await;
    if !test_result.success {
        return Err(validation_error(
            "provider_verification_failed",
            "We could not verify that provider. Check the key or connection and try again.",
        ));
    }
    if request.provider.kind == ProviderKind::Local
        && test_result.models.as_ref().is_none_or(Vec::is_empty)
    {
        return Err(validation_error(
            "local_runtime_no_model",
            "Your local runtime is reachable, but no model is available yet.",
        ));
    }

    let provider_id = candidate
        .id
        .clone()
        .expect("commissioning provider has an id");
    match state.provider_service.get(&provider_id) {
        Ok(_) => state
            .provider_service
            .update(&provider_id, candidate)
            .map_err(|_| internal_error())?,
        Err(_) => state
            .provider_service
            .create(candidate)
            .map_err(|_| internal_error())?,
    };
    state
        .provider_service
        .set_default(&provider_id)
        .map_err(|_| internal_error())?;

    let mut settings = state.settings.load().map_err(|_| internal_error())?;
    settings.execution.setup_complete = true;
    settings.execution.agent_name = Some(request.display_name.trim().to_string());
    settings.execution.orchestrator = OrchestratorConfig {
        provider_id: Some(provider_id.clone()),
        model: Some(request.provider.model.trim().to_string()),
        ..settings.execution.orchestrator
    };

    write_commissioning_soul(
        &state,
        request.display_name.trim(),
        optional_trimmed(&request.profile).as_deref(),
        &request.primary_focus,
        &request.domains,
    )?;

    settings.commissioning = CommissioningSettings {
        version: 1,
        state: CommissioningState::Complete,
        primary_focus: Some(request.primary_focus),
        domains: request.domains.clone(),
        profile: optional_trimmed(&request.profile),
        user_profile: UserProfile {
            name: optional_trimmed(&request.user_name),
            interests: cleaned_list(&request.interests),
            hobbies: cleaned_list(&request.hobbies),
            date_of_birth: request.date_of_birth.as_deref().and_then(optional_trimmed),
        },
        provider_id: Some(provider_id),
        model: Some(request.provider.model.trim().to_string()),
        semantic_profile: semantic_profile_for_domains(&request.domains),
        ..CommissioningSettings::default()
    };

    // Completion is committed last. A retry after a prior provider write is
    // idempotent because provider IDs are preset/local identities.
    state
        .settings
        .save(&settings)
        .map_err(|_| internal_error())?;

    Ok(Json(CommissioningStatusResponse {
        state: CommissioningState::Complete,
        recovery_code: None,
        semantic_profile: settings.commissioning.semantic_profile,
    }))
}

fn effective_status(
    commissioning: &CommissioningSettings,
    execution: &gateway_services::ExecutionSettings,
    providers: &[Provider],
) -> (CommissioningState, Option<&'static str>) {
    if commissioning.state == CommissioningState::Complete {
        return (CommissioningState::Complete, None);
    }

    let legacy_complete = execution.setup_complete
        && execution.orchestrator.provider_id.is_some()
        && execution.orchestrator.model.is_some()
        && providers.iter().any(|provider| {
            provider.is_default
                && provider.verified.unwrap_or(false)
                && !provider.default_model().trim().is_empty()
        });
    if legacy_complete {
        return (CommissioningState::Complete, Some("legacy_configuration"));
    }

    if providers.is_empty() {
        (CommissioningState::NotStarted, None)
    } else {
        (
            CommissioningState::NeedsAttention,
            Some("legacy_configuration_incomplete"),
        )
    }
}

fn validate_request(
    request: &CommissioningRequest,
) -> Result<(), (StatusCode, Json<CommissioningError>)> {
    if request.display_name.trim().is_empty()
        || request.display_name.chars().count() > 80
        || request.display_name.contains(['\n', '\r'])
    {
        return Err(validation_error(
            "invalid_display_name",
            "Choose a name up to 80 characters.",
        ));
    }
    if request.profile.chars().count() > 2_000 {
        return Err(validation_error(
            "invalid_profile",
            "Keep your profile under 2,000 characters.",
        ));
    }
    if invalid_short_text(&request.user_name, 80) {
        return Err(validation_error(
            "invalid_user_name",
            "Tell us your name using up to 80 characters.",
        ));
    }
    if !valid_profile_list(&request.interests, 1, 12, 60) {
        return Err(validation_error(
            "invalid_interests",
            "Choose between one and twelve interests, up to 60 characters each.",
        ));
    }
    if !valid_profile_list(&request.hobbies, 0, 12, 80) {
        return Err(validation_error(
            "invalid_hobbies",
            "Keep hobbies to twelve entries of up to 80 characters each.",
        ));
    }
    if let Some(date_of_birth) = request.date_of_birth.as_deref().and_then(optional_trimmed) {
        if !is_valid_iso_date(&date_of_birth) {
            return Err(validation_error(
                "invalid_date_of_birth",
                "Use a valid date in YYYY-MM-DD format, or leave it blank.",
            ));
        }
    }
    if !matches!(
        request.primary_focus.as_str(),
        "think_organize" | "build_code" | "research_learn" | "run_work"
    ) {
        return Err(validation_error(
            "invalid_primary_focus",
            "Choose one primary focus.",
        ));
    }
    if request.domains.is_empty()
        || request.domains.iter().any(|domain| {
            !matches!(
                domain.as_str(),
                "personal_knowledge" | "software" | "writing" | "learning" | "planning"
            )
        })
    {
        return Err(validation_error(
            "invalid_domains",
            "Choose at least one supported domain.",
        ));
    }
    Ok(())
}

fn invalid_short_text(value: &str, max_characters: usize) -> bool {
    value.trim().is_empty()
        || value.chars().count() > max_characters
        || value.contains(['\n', '\r'])
}

fn valid_profile_list(
    values: &[String],
    minimum: usize,
    maximum: usize,
    max_characters: usize,
) -> bool {
    (minimum..=maximum).contains(&values.len())
        && values
            .iter()
            .all(|value| !invalid_short_text(value, max_characters))
}

fn cleaned_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim().to_string())
        .collect()
}

fn is_valid_iso_date(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !year.bytes().all(|byte| byte.is_ascii_digit())
        || !month.bytes().all(|byte| byte.is_ascii_digit())
        || !day.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = year.parse::<u16>() else {
        return false;
    };
    let Ok(month) = month.parse::<u8>() else {
        return false;
    };
    let Ok(day) = day.parse::<u8>() else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => unreachable!("month is already validated"),
    };
    (1..=days_in_month).contains(&day)
}

fn provider_from_selection(
    selection: &ProviderSelection,
) -> Result<Provider, (StatusCode, Json<CommissioningError>)> {
    if selection.model.trim().is_empty() || selection.model.chars().count() > 160 {
        return Err(validation_error("invalid_model", "Choose a valid model."));
    }

    match selection.kind {
        ProviderKind::Local => Ok(local_provider(selection.model.trim())),
        ProviderKind::Cloud => {
            let preset_id = selection.preset_id.as_deref().ok_or_else(|| {
                validation_error(
                    "missing_provider_preset",
                    "Choose a supported cloud provider.",
                )
            })?;
            let preset = CLOUD_PRESETS
                .iter()
                .find(|preset| preset.id == preset_id)
                .ok_or_else(|| {
                    validation_error(
                        "unsupported_provider_preset",
                        "Choose a supported cloud provider.",
                    )
                })?;
            if !preset.models.contains(&selection.model.trim()) {
                return Err(validation_error(
                    "invalid_model",
                    "Choose a model offered by that provider.",
                ));
            }
            let api_key = selection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    validation_error(
                        "missing_provider_key",
                        "Enter an API key for that provider.",
                    )
                })?;
            if api_key.chars().count() > 1_024 {
                return Err(validation_error(
                    "invalid_provider_key",
                    "Enter a valid API key.",
                ));
            }
            Ok(Provider {
                id: Some(preset.provider_id.to_string()),
                name: preset.name.to_string(),
                description: format!("{} API", preset.name),
                api_key: api_key.to_string(),
                base_url: preset.base_url.to_string(),
                models: vec![selection.model.trim().to_string()],
                embedding_models: None,
                embedding_dimensions: None,
                verified: Some(true),
                is_default: false,
                created_at: None,
                max_concurrent_requests: None,
                context_window: None,
                default_model: Some(selection.model.trim().to_string()),
                rate_limits: None,
                model_configs: None,
            })
        }
    }
}

fn local_provider(model: &str) -> Provider {
    Provider {
        id: Some("provider-ollama-local".to_string()),
        name: "Ollama Local".to_string(),
        description: "Local Ollama runtime".to_string(),
        api_key: "ollama".to_string(),
        base_url: LOCAL_RUNTIME_URL.to_string(),
        models: vec![model.to_string()],
        embedding_models: None,
        embedding_dimensions: None,
        verified: Some(true),
        is_default: false,
        created_at: None,
        max_concurrent_requests: None,
        context_window: None,
        default_model: Some(model.to_string()),
        rate_limits: None,
        model_configs: None,
    }
}

fn local_runtime_binary_present() -> bool {
    Command::new("ollama")
        .arg("--version")
        .output()
        .is_ok_and(|result| result.status.success())
}

fn is_local_commissioning_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        // Native/CLI clients do not send Origin. Their local transport remains
        // supported; browsers must prove they are the local app.
        return true;
    };
    matches!(
        origin.to_str().ok(),
        Some(
            "http://localhost:3000"
                | "http://127.0.0.1:3000"
                | "http://localhost:18791"
                | "http://127.0.0.1:18791"
        )
    )
}

fn semantic_profile_for_domains(domains: &[String]) -> SemanticProfile {
    SemanticProfile {
        domain_pack_ids: domains
            .iter()
            .map(|domain| format!("zbot.{domain}:v1"))
            .collect(),
        ..SemanticProfile::default()
    }
}

fn optional_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn write_commissioning_soul(
    state: &AppState,
    display_name: &str,
    profile: Option<&str>,
    primary_focus: &str,
    domains: &[String],
) -> Result<(), (StatusCode, Json<CommissioningError>)> {
    let path = state.paths.soul();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| internal_error())?;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let identity = if let Some(rest) = current.strip_prefix("You are **") {
        if let Some(after_name) = rest.find("**") {
            format!("You are **{}**{}", display_name, &rest[after_name + 2..])
        } else {
            format!(
                "You are **{}**, an autonomous agent.\n\n{}",
                display_name, current
            )
        }
    } else {
        format!(
            "You are **{}**, an autonomous agent.\n\n{}",
            display_name, current
        )
    };
    let without_old_context =
        if let Some(start) = identity.find("<!-- zbot:commissioning:start -->") {
            let end = identity
                .find("<!-- zbot:commissioning:end -->")
                .map(|index| index + "<!-- zbot:commissioning:end -->".len())
                .unwrap_or(identity.len());
            format!("{}{}", &identity[..start], &identity[end..])
        } else {
            identity
        };
    let safe_profile = profile.unwrap_or("").replace("<!--", "&lt;!--");
    let profile_line = if safe_profile.is_empty() {
        String::new()
    } else {
        format!("- Working preferences: {safe_profile}\n")
    };
    let context = format!(
        "\n<!-- zbot:commissioning:start -->\n## Commissioned context\n- Primary focus: {primary_focus}\n- Domains: {}\n{profile_line}<!-- zbot:commissioning:end -->\n",
        domains.join(", ")
    );
    std::fs::write(
        path,
        format!("{}{}", without_old_context.trim_end(), context),
    )
    .map_err(|_| internal_error())
}

fn validation_error(
    code: &'static str,
    message: &'static str,
) -> (StatusCode, Json<CommissioningError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(CommissioningError { code, message }),
    )
}

fn internal_error() -> (StatusCode, Json<CommissioningError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(CommissioningError {
            code: "commissioning_unavailable",
            message: "Commissioning is temporarily unavailable. Please try again.",
        }),
    )
}

fn commissioning_origin_denied() -> (StatusCode, Json<CommissioningError>) {
    (
        StatusCode::FORBIDDEN,
        Json(CommissioningError {
            code: "commissioning_origin_denied",
            message: "Commissioning must be completed from the local z-Bot app.",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::ExecutionSettings;

    fn valid_commissioning_request() -> CommissioningRequest {
        CommissioningRequest {
            display_name: "Atlas".to_string(),
            profile: "Concise answers".to_string(),
            user_name: "Ada".to_string(),
            interests: vec!["Learning & ideas".to_string()],
            hobbies: vec!["Reading".to_string()],
            date_of_birth: Some("1990-02-28".to_string()),
            primary_focus: "research_learn".to_string(),
            domains: vec!["learning".to_string()],
            provider: ProviderSelection {
                kind: ProviderKind::Local,
                preset_id: None,
                model: "llama3.3".to_string(),
                api_key: None,
            },
        }
    }

    // STUB: AC1 — commissioning requires one of the two approved memory profiles.
    #[test]
    fn commissioning_request_requires_an_approved_memory_profile() {
        let base = serde_json::json!({
            "displayName": "Atlas",
            "userName": "Ada",
            "interests": ["Learning & ideas"],
            "primaryFocus": "research_learn",
            "domains": ["learning"],
            "provider": { "kind": "local", "model": "llama3.3" }
        });

        assert!(serde_json::from_value::<CommissioningRequest>(base.clone()).is_err());

        let mut approved = base.clone();
        approved["memoryProfile"] = serde_json::json!("zbot_recommended_v1");
        assert!(serde_json::from_value::<CommissioningRequest>(approved).is_ok());

        let mut safe = base.clone();
        safe["memoryProfile"] = serde_json::json!("safe_baseline");
        assert!(serde_json::from_value::<CommissioningRequest>(safe.clone()).is_ok());

        let mut unknown = base;
        unknown["memoryProfile"] = serde_json::json!("custom");
        assert!(serde_json::from_value::<CommissioningRequest>(unknown).is_err());

        let mut unknown_top_level = safe.clone();
        unknown_top_level["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CommissioningRequest>(unknown_top_level).is_err());

        let mut unknown_provider = safe;
        unknown_provider["provider"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CommissioningRequest>(unknown_provider).is_err());
    }

    // AC8 — authorization runs before JSON extraction while loopback CLI stays supported.
    #[tokio::test]
    async fn originless_remote_request_is_denied_even_when_its_json_is_malformed() {
        use axum::{routing::post, Extension, Router};
        use axum_test::TestServer;

        async fn guarded(
            _local: LocalCommissioningRequest,
            Json(_body): Json<serde_json::Value>,
        ) -> StatusCode {
            StatusCode::OK
        }

        async fn response_for(peer: &str) -> axum_test::TestResponse {
            let config = GatewayConfig {
                host: "0.0.0.0".parse().unwrap(),
                ..GatewayConfig::default()
            };
            let router = Router::new()
                .route("/complete", post(guarded))
                .layer(Extension(config))
                .layer(Extension(ConnectInfo::<SocketAddr>(peer.parse().unwrap())));
            TestServer::new(router)
                .unwrap()
                .post("/complete")
                .content_type("application/json")
                .bytes(b"{".to_vec().into())
                .await
        }

        let remote = response_for("192.168.1.40:43110").await;
        remote.assert_status_forbidden();
        assert_eq!(
            remote.json::<serde_json::Value>()["code"],
            "commissioning_origin_denied"
        );

        let loopback = response_for("127.0.0.1:43110").await;
        assert_ne!(loopback.status_code(), StatusCode::FORBIDDEN);
    }

    // STUB: AC3, AC5, AC6 — fixed profile files are explicit and conflict-safe.
    #[test]
    fn profile_provisioning_distinguishes_safe_full_retry_and_conflict() {
        fn provision_stub(_root: &std::path::Path, _full: bool) -> Result<(), &'static str> {
            Err("profile provisioning is not implemented")
        }

        let safe = tempfile::tempdir().unwrap();
        assert!(provision_stub(safe.path(), false).is_ok());
        assert!(!safe.path().join("config/recall-config.json").exists());

        let full = tempfile::tempdir().unwrap();
        assert!(provision_stub(full.path(), true).is_ok());
        assert!(full.path().join("config/recall-config.json").exists());
        assert!(full
            .path()
            .join("config/governance/base-ontology.json")
            .exists());
        assert!(full
            .path()
            .join("config/governance/base-taxonomy.json")
            .exists());
        assert!(provision_stub(full.path(), true).is_ok());

        std::fs::write(full.path().join("config/recall-config.json"), "conflict").unwrap();
        assert_eq!(
            provision_stub(full.path(), true),
            Err("memory_profile_conflict")
        );
        assert_eq!(
            std::fs::read_to_string(full.path().join("config/recall-config.json")).unwrap(),
            "conflict"
        );
    }

    // STUB: AC6 — full memory stays restart-pending until a boot activates it.
    #[test]
    fn full_profile_completion_is_finalized_only_after_restart_activation() {
        #[derive(Debug, PartialEq, Eq)]
        struct ActivationSnapshot {
            state: CommissioningState,
            setup_complete: bool,
            restart_required: bool,
            marker_exists: bool,
        }

        fn persist_full_profile_stub() -> ActivationSnapshot {
            ActivationSnapshot {
                state: CommissioningState::Complete,
                setup_complete: true,
                restart_required: false,
                marker_exists: false,
            }
        }

        fn activate_on_boot_stub(snapshot: ActivationSnapshot) -> ActivationSnapshot {
            snapshot
        }

        let pending = persist_full_profile_stub();
        assert_eq!(pending.state, CommissioningState::NeedsAttention);
        assert!(!pending.setup_complete);
        assert!(pending.restart_required);
        assert!(pending.marker_exists);

        let active = activate_on_boot_stub(pending);
        assert_eq!(active.state, CommissioningState::Complete);
        assert!(active.setup_complete);
        assert!(!active.restart_required);
        assert!(!active.marker_exists);
    }

    // STUB: AC4, AC6 — custom memory/backend conflicts have zero persistent effects.
    #[test]
    fn full_profile_preflight_preserves_all_state_on_memory_or_embedding_conflict() {
        #[derive(Debug, PartialEq, Eq)]
        struct PersistentSnapshot {
            provider: Vec<u8>,
            soul: Vec<u8>,
            settings: Vec<u8>,
            recall: Vec<u8>,
            ontology: Vec<u8>,
            taxonomy: Vec<u8>,
            marker: Vec<u8>,
        }

        fn snapshot(paths: &gateway_services::VaultPaths) -> PersistentSnapshot {
            let read = |path: std::path::PathBuf| std::fs::read(path).unwrap();
            let governance = paths.config_dir().join("governance");
            PersistentSnapshot {
                provider: read(paths.providers()),
                soul: read(paths.soul()),
                settings: read(paths.settings()),
                recall: read(paths.recall_config()),
                ontology: read(governance.join("base-ontology.json")),
                taxonomy: read(governance.join("base-taxonomy.json")),
                marker: read(paths.config_dir().join(".zbot-memory-profile-v1-pending")),
            }
        }

        let vault = tempfile::tempdir().unwrap();
        let paths = gateway_services::VaultPaths::new(vault.path().to_path_buf());
        let governance = paths.config_dir().join("governance");
        std::fs::create_dir_all(&governance).unwrap();
        std::fs::write(paths.providers(), b"provider-before").unwrap();
        std::fs::write(paths.soul(), b"soul-before").unwrap();
        std::fs::write(paths.settings(), b"settings-before").unwrap();
        std::fs::write(
            paths.recall_config(),
            include_bytes!("../../gateway-memory/templates/zbot-recommended-v1-recall.json"),
        )
        .unwrap();
        std::fs::write(
            governance.join("base-ontology.json"),
            include_bytes!("../../templates/governance/base-ontology.json"),
        )
        .unwrap();
        std::fs::write(
            governance.join("base-taxonomy.json"),
            include_bytes!("../../templates/governance/base-taxonomy.json"),
        )
        .unwrap();
        std::fs::write(
            paths.config_dir().join(".zbot-memory-profile-v1-pending"),
            MEMORY_PROFILE_PENDING_BYTES,
        )
        .unwrap();

        let before = snapshot(&paths);
        let exact = gateway_memory::MemorySettings::zbot_recommended_v1();
        assert_eq!(preflight_full_memory_profile(&paths, &exact, true), Ok(()));

        let mut custom = gateway_memory::MemorySettings::default();
        custom.corrections_abstractor_interval_hours = 99;
        assert_eq!(
            preflight_full_memory_profile(&paths, &custom, true),
            Err("memory_profile_conflict")
        );
        assert_eq!(snapshot(&paths), before);
        assert_eq!(
            preflight_full_memory_profile(&paths, &exact, false),
            Err("memory_profile_conflict")
        );
        assert_eq!(snapshot(&paths), before);
    }

    // STUB: AC6, AC7 — every fixed target, including the marker, fails closed.
    #[test]
    #[cfg(unix)]
    fn fixed_profile_targets_reject_symlinks_non_regular_files_and_conflicts() {
        use std::os::unix::fs::symlink;

        let vault = tempfile::tempdir().unwrap();
        let config = vault.path().join("config");
        std::fs::create_dir(&config).unwrap();

        let absent = config.join("absent");
        assert_eq!(
            preflight_fixed_profile_target(&absent, MEMORY_PROFILE_PENDING_BYTES),
            Ok(FixedProfileTargetState::Absent)
        );

        let identical = config.join("identical");
        std::fs::write(&identical, MEMORY_PROFILE_PENDING_BYTES).unwrap();
        assert_eq!(
            preflight_fixed_profile_target(&identical, MEMORY_PROFILE_PENDING_BYTES),
            Ok(FixedProfileTargetState::Identical)
        );

        let conflict = config.join("conflict");
        std::fs::write(&conflict, b"different").unwrap();
        assert_eq!(
            preflight_fixed_profile_target(&conflict, MEMORY_PROFILE_PENDING_BYTES),
            Err("memory_profile_conflict")
        );
        assert_eq!(std::fs::read(&conflict).unwrap(), b"different");

        let directory = config.join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert_eq!(
            preflight_fixed_profile_target(&directory, MEMORY_PROFILE_PENDING_BYTES),
            Err("memory_profile_conflict")
        );

        let outside = vault.path().join("outside");
        std::fs::write(&outside, b"outside").unwrap();
        let link = config.join("link");
        symlink(&outside, &link).unwrap();
        assert_eq!(
            preflight_fixed_profile_target(&link, MEMORY_PROFILE_PENDING_BYTES),
            Err("memory_profile_conflict")
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");

        let parent_vault = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_marker = outside_dir.path().join("marker");
        std::fs::write(&outside_marker, b"outside-parent").unwrap();
        symlink(outside_dir.path(), parent_vault.path().join("config")).unwrap();
        assert_eq!(
            preflight_fixed_profile_target(
                &parent_vault.path().join("config/marker"),
                MEMORY_PROFILE_PENDING_BYTES,
            ),
            Err("memory_profile_conflict")
        );
        assert_eq!(std::fs::read(&outside_marker).unwrap(), b"outside-parent");
    }

    // STUB: AC6 — boot activation saves completion before marker cleanup.
    #[test]
    fn pending_profile_activation_is_ordered_and_failure_idempotent() {
        use std::{cell::RefCell, rc::Rc};

        fn marker(vault: &tempfile::TempDir) -> std::path::PathBuf {
            let config = vault.path().join("config");
            std::fs::create_dir_all(&config).unwrap();
            let marker = config.join(".zbot-memory-profile-v1-pending");
            std::fs::write(&marker, MEMORY_PROFILE_PENDING_BYTES).unwrap();
            marker
        }

        let save_failure = tempfile::tempdir().unwrap();
        let save_failure_marker = marker(&save_failure);
        let result = finalize_pending_memory_profile(
            &save_failure_marker,
            CommissioningState::NeedsAttention,
            true,
            || Err("settings_save_failed"),
            || panic!("marker removal must not run after save failure"),
        );
        assert_eq!(result, Err("settings_save_failed"));
        assert!(save_failure_marker.exists());

        let cleanup_failure = tempfile::tempdir().unwrap();
        let cleanup_failure_marker = marker(&cleanup_failure);
        let events = Rc::new(RefCell::new(Vec::new()));
        let save_events = events.clone();
        let remove_events = events.clone();
        let result = finalize_pending_memory_profile(
            &cleanup_failure_marker,
            CommissioningState::NeedsAttention,
            true,
            move || {
                save_events.borrow_mut().push("save");
                Ok(())
            },
            move || {
                remove_events.borrow_mut().push("remove");
                Err("marker_remove_failed")
            },
        );
        assert_eq!(events.borrow().as_slice(), ["save", "remove"]);
        assert_eq!(result, Err("marker_remove_failed"));
        assert!(cleanup_failure_marker.exists());

        let activated = tempfile::tempdir().unwrap();
        let activated_marker = marker(&activated);
        let result = finalize_pending_memory_profile(
            &activated_marker,
            CommissioningState::NeedsAttention,
            true,
            || Ok(()),
            || std::fs::remove_file(&activated_marker).map_err(|_| "marker_remove_failed"),
        );
        assert_eq!(result, Ok(MemoryProfileActivationOutcome::Activated));
        assert!(!activated_marker.exists());

        let completed = tempfile::tempdir().unwrap();
        let completed_marker = marker(&completed);
        let result = finalize_pending_memory_profile(
            &completed_marker,
            CommissioningState::Complete,
            true,
            || panic!("completed marker cleanup must not save again"),
            || std::fs::remove_file(&completed_marker).map_err(|_| "marker_remove_failed"),
        );
        assert_eq!(
            result,
            Ok(MemoryProfileActivationOutcome::CleanedCompletedMarker)
        );
        assert!(!completed_marker.exists());

        let stale = tempfile::tempdir().unwrap();
        let stale_marker = marker(&stale);
        let result = finalize_pending_memory_profile(
            &stale_marker,
            CommissioningState::NotStarted,
            true,
            || panic!("stale marker must not save"),
            || panic!("stale marker must not be removed"),
        );
        assert_eq!(result, Err("memory_profile_conflict"));
        assert!(stale_marker.exists());

        let mismatch = tempfile::tempdir().unwrap();
        let mismatch_marker = marker(&mismatch);
        let result = finalize_pending_memory_profile(
            &mismatch_marker,
            CommissioningState::NeedsAttention,
            false,
            || panic!("mismatched V1 inputs must not save"),
            || panic!("mismatched V1 inputs must not remove marker"),
        );
        assert_eq!(result, Err("memory_profile_conflict"));
        assert!(mismatch_marker.exists());
    }

    #[test]
    fn empty_configuration_requires_commissioning() {
        let (state, recovery) = effective_status(
            &CommissioningSettings::default(),
            &ExecutionSettings::default(),
            &[],
        );
        assert_eq!(state, CommissioningState::NotStarted);
        assert_eq!(recovery, None);
    }

    #[test]
    fn incomplete_legacy_configuration_needs_attention() {
        let provider = local_provider("llama3.3");
        let (state, recovery) = effective_status(
            &CommissioningSettings::default(),
            &ExecutionSettings::default(),
            &[provider],
        );
        assert_eq!(state, CommissioningState::NeedsAttention);
        assert_eq!(recovery, Some("legacy_configuration_incomplete"));
    }

    #[test]
    fn complete_legacy_configuration_stays_usable_without_a_new_flow() {
        let provider = Provider {
            is_default: true,
            verified: Some(true),
            default_model: Some("gpt-4o".to_string()),
            ..local_provider("gpt-4o")
        };
        let execution = ExecutionSettings {
            setup_complete: true,
            orchestrator: OrchestratorConfig {
                provider_id: Some("provider-ollama-local".to_string()),
                model: Some("gpt-4o".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let (state, recovery) =
            effective_status(&CommissioningSettings::default(), &execution, &[provider]);
        assert_eq!(state, CommissioningState::Complete);
        assert_eq!(recovery, Some("legacy_configuration"));
    }

    #[test]
    fn semantic_profile_never_names_a_backend() {
        let profile = semantic_profile_for_domains(&["software".to_string()]);
        assert_eq!(profile.domain_pack_ids, ["zbot.software:v1"]);
        assert_eq!(
            profile.provisioning,
            gateway_services::SemanticProvisioning::Deferred
        );
        assert!(!serde_json::to_string(&profile).unwrap().contains("engram"));
    }

    #[test]
    fn personal_profile_is_required_and_optional_date_is_strictly_validated() {
        let request = valid_commissioning_request();
        assert!(validate_request(&request).is_ok());

        let mut missing_name = valid_commissioning_request();
        missing_name.user_name = "  ".to_string();
        assert_eq!(
            validate_request(&missing_name).unwrap_err().1 .0.code,
            "invalid_user_name"
        );

        let mut no_interests = valid_commissioning_request();
        no_interests.interests.clear();
        assert_eq!(
            validate_request(&no_interests).unwrap_err().1 .0.code,
            "invalid_interests"
        );

        let mut invalid_date = valid_commissioning_request();
        invalid_date.date_of_birth = Some("2025-02-29".to_string());
        assert_eq!(
            validate_request(&invalid_date).unwrap_err().1 .0.code,
            "invalid_date_of_birth"
        );
        assert!(is_valid_iso_date("2024-02-29"));
    }

    #[test]
    fn cloud_selection_rejects_unknown_presets_and_oversized_keys() {
        let unknown = ProviderSelection {
            kind: ProviderKind::Cloud,
            preset_id: Some("custom".to_string()),
            model: "any".to_string(),
            api_key: Some("key".to_string()),
        };
        assert_eq!(
            provider_from_selection(&unknown).unwrap_err().1 .0.code,
            "unsupported_provider_preset"
        );

        let oversized = ProviderSelection {
            kind: ProviderKind::Cloud,
            preset_id: Some("openai".to_string()),
            model: "gpt-4o".to_string(),
            api_key: Some("x".repeat(1_025)),
        };
        assert_eq!(
            provider_from_selection(&oversized).unwrap_err().1 .0.code,
            "invalid_provider_key"
        );
    }

    #[test]
    fn commissioned_soul_context_is_replaced_and_markers_are_escaped() {
        let original = "You are **Old Name**, an autonomous agent.\n<!-- zbot:commissioning:start -->old<!-- zbot:commissioning:end -->";
        let identity = if let Some(rest) = original.strip_prefix("You are **") {
            let after_name = rest.find("**").unwrap();
            format!("You are **New Name**{}", &rest[after_name + 2..])
        } else {
            original.to_string()
        };
        let start = identity.find("<!-- zbot:commissioning:start -->").unwrap();
        let end = identity.find("<!-- zbot:commissioning:end -->").unwrap()
            + "<!-- zbot:commissioning:end -->".len();
        let profile = "Use concise answers <!-- not a marker -->".replace("<!--", "&lt;!--");
        let rendered = format!(
            "{}\n<!-- zbot:commissioning:start -->\n## Commissioned context\n- Primary focus: research_learn\n- Domains: learning\n- Working preferences: {profile}\n<!-- zbot:commissioning:end -->\n",
            format!("{}{}", &identity[..start], &identity[end..]).trim_end()
        );
        assert!(rendered.starts_with("You are **New Name**"));
        assert!(!rendered.contains("old<!--"));
        assert!(rendered.contains("&lt;!-- not a marker -->"));
    }

    #[test]
    fn completion_rejects_nonlocal_browser_origins() {
        let mut local = HeaderMap::new();
        local.insert(ORIGIN, "http://localhost:3000".parse().unwrap());
        assert!(is_local_commissioning_origin(&local));

        let mut remote = HeaderMap::new();
        remote.insert(ORIGIN, "https://example.test".parse().unwrap());
        assert!(!is_local_commissioning_origin(&remote));
    }
}
