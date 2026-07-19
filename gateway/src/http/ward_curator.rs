//! Ward-curator HTTP endpoints. Spec:
//! `docs/architecture/future-state/2026-05-23-ward-curator-spec.md`.
//!
//! - `POST /api/curator/cleanup` — Phase B Layer-1 transitions (archive,
//!   stale, reactivate). Backup + audit log written on a live run.
//! - `POST /api/curator/restore` — un-tars a named backup over `wards/`.
//! - `POST /api/curator/consolidate` — Phase C LLM consolidation pass:
//!   builds candidates from the sidecar, asks the LLM for a YAML plan
//!   (merge / absorb / archive), re-keys procedures, then applies the plan
//!   via `WardCurator::apply_consolidation`. `dry_run` defaults to TRUE.

use std::sync::Arc;

use agent_runtime::llm::{openai::OpenAiClient, LlmClient, LlmConfig};
use axum::{body::Bytes, extract::State, http::StatusCode, response::IntoResponse, Json};
use gateway_execution::curator::consolidate_wards;
use gateway_services::{
    CleanupReport, CleanupRequest, ConsolidateRequest, ConsolidationReport, RestoreReport,
    RestoreRequest, WardCurator,
};

use crate::state::AppState;

fn make_curator(state: &AppState) -> WardCurator {
    WardCurator::new(state.paths.wards_dir(), state.paths.data_dir())
}

/// `POST /api/curator/cleanup` — body is an optional `CleanupRequest`. An
/// empty body or `{}` runs with defaults (stale=30d, archive=90d, dry_run=false).
pub async fn cleanup(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<CleanupReport>, (StatusCode, String)> {
    let req: CleanupRequest = if body.is_empty() {
        CleanupRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad request body: {e}")))?
    };
    make_curator(&state)
        .cleanup(&req)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

/// `POST /api/curator/restore` — body `{ "backup": "<utc-iso>" }`.
pub async fn restore(
    State(state): State<AppState>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<RestoreReport>, impl IntoResponse> {
    make_curator(&state)
        .restore(&req.backup)
        .map(Json)
        .map_err(|e| {
            // 404 if the named backup doesn't exist; anything else is 500.
            let code = if e.contains("backup not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (code, e)
        })
}

/// Build an LLM client for the ward curator. Three-tier resolution mirrors
/// the distillation pattern:
///   `settings.curator.{provider_id,model}` → `settings.orchestrator.{…}` → provider default
/// Temperature inherits the orchestrator; max output uses the curator override
/// when configured and otherwise inherits the orchestrator.
fn make_curator_llm(state: &AppState) -> Result<Arc<dyn LlmClient>, String> {
    let exec = state.settings.get_execution_settings().unwrap_or_default();
    let curator = &exec.curator;
    let orch = &exec.orchestrator;

    let providers = state
        .provider_service
        .list()
        .map_err(|e| format!("list providers: {e}"))?;

    let provider_id_override = curator
        .provider_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| orch.provider_id.as_deref().filter(|s| !s.is_empty()));

    let provider = match provider_id_override {
        Some(id) => state
            .provider_service
            .get(id)
            .map_err(|e| format!("provider {id}: {e}"))?,
        None => providers
            .iter()
            .find(|p| p.is_default)
            .or_else(|| providers.first())
            .cloned()
            .ok_or_else(|| "no providers configured".to_string())?,
    };

    let model = curator
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| orch.model.clone().filter(|m| !m.is_empty()))
        .unwrap_or_else(|| provider.default_model().to_string());
    let max_tokens = curator.max_tokens.unwrap_or(orch.max_tokens);

    let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());
    let llm_config = LlmConfig::new(provider.base_url, provider.api_key, model, provider_id)
        .with_temperature(orch.temperature)
        .with_max_tokens(max_tokens);
    let client = OpenAiClient::new(llm_config).map_err(|e| format!("build llm client: {e}"))?;
    Ok(Arc::new(client) as Arc<dyn LlmClient>)
}

/// `POST /api/curator/consolidate` — Phase C LLM consolidation. Body is an
/// optional `ConsolidateRequest`; empty body / `{}` runs with defaults
/// (`dry_run: true`, `max_consolidations: 5`).
///
/// When the caller supplies `plan` in the body, the LLM is skipped entirely
/// and the supplied plan is fed straight to the apply step — useful for
/// dry-run-then-commit workflows and tests.
pub async fn consolidate(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<ConsolidationReport>, (StatusCode, String)> {
    let req: ConsolidateRequest = if body.is_empty() {
        ConsolidateRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad request body: {e}")))?
    };

    let curator = make_curator(&state);
    let llm = make_curator_llm(&state).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    consolidate_wards(&curator, llm.as_ref(), state.procedure_store.as_ref(), &req)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}
