//! # Tool Endpoints
//!
//! Endpoints for listing available tools.

use crate::state::AppState;
use agent_runtime::{ContextCapability, ContextCapabilityCatalog};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use gateway_execution::invoke::RuntimeActorKind;
use serde::{Deserialize, Serialize};

/// Error response for tool catalog endpoints.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Query parameters for actor-filtered tool catalog snapshots.
#[derive(Debug, Default, Deserialize)]
pub struct ToolCatalogQuery {
    /// Runtime actor kind. Defaults to root.
    #[serde(default, alias = "actorKind", alias = "actor_kind")]
    pub actor: Option<String>,
    /// Optional session id to echo into the catalog snapshot.
    #[serde(default, rename = "sessionId", alias = "session_id")]
    pub session_id: Option<String>,
    /// Optional agent id to echo into the catalog snapshot.
    #[serde(default, rename = "agentId", alias = "agent_id")]
    pub agent_id: Option<String>,
}

/// GET /api/tools - List all available tools.
pub async fn list_tools(
    State(state): State<AppState>,
    Query(query): Query<ToolCatalogQuery>,
) -> Result<Json<ContextCapabilityCatalog>, (StatusCode, Json<ErrorResponse>)> {
    let actor_kind = parse_actor_kind(query.actor.as_deref())?;
    Ok(Json(
        state
            .context_capability_catalog_with_resources(actor_kind, query.session_id, query.agent_id)
            .await,
    ))
}

/// GET /api/tools/:name - Get a tool by name.
pub async fn get_tool(
    State(state): State<AppState>,
    Query(query): Query<ToolCatalogQuery>,
    Path(name): Path<String>,
) -> Result<Json<ContextCapability>, (StatusCode, Json<ErrorResponse>)> {
    let actor_kind = parse_actor_kind(query.actor.as_deref())?;
    let catalog = state
        .context_capability_catalog_with_resources(actor_kind, query.session_id, query.agent_id)
        .await;

    catalog
        .capabilities
        .into_iter()
        .find(|capability| capability.id == name)
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("tool not found: {name}"),
                }),
            )
        })
}

fn parse_actor_kind(
    actor: Option<&str>,
) -> Result<RuntimeActorKind, (StatusCode, Json<ErrorResponse>)> {
    match actor.unwrap_or("root") {
        "root" => Ok(RuntimeActorKind::Root),
        "delegated_executor" | "delegated-executor" | "executor" => {
            Ok(RuntimeActorKind::DelegatedExecutor)
        }
        "delegated_reviewer" | "delegated-reviewer" | "reviewer" => {
            Ok(RuntimeActorKind::DelegatedReviewer)
        }
        "ward_agent" | "ward-agent" | "ward" => Ok(RuntimeActorKind::WardAgent),
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("unsupported actor kind: {other}"),
            }),
        )),
    }
}
