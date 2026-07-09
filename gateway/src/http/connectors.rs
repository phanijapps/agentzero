//! # Connector HTTP Endpoints
//!
//! REST API for managing external connectors.

use crate::bus::{GatewayBus, HttpGatewayBus, SessionRequest};
use crate::connectors::{
    ConnectorServiceError, CreateConnectorRequest, InboundLogEntry, InboundPayload, InboundResult,
    UpdateConnectorRequest,
};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use execution_state::TriggerSource;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

/// Error response for connector operations.
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

impl ErrorResponse {
    fn not_found(id: &str) -> Self {
        Self {
            error: format!("Connector not found: {}", id),
            code: "CONNECTOR_NOT_FOUND".to_string(),
        }
    }

    fn already_exists(id: &str) -> Self {
        Self {
            error: format!("Connector already exists: {}", id),
            code: "CONNECTOR_EXISTS".to_string(),
        }
    }

    fn invalid_id(msg: &str) -> Self {
        Self {
            error: msg.to_string(),
            code: "INVALID_ID".to_string(),
        }
    }

    fn internal(msg: &str) -> Self {
        Self {
            error: msg.to_string(),
            code: "INTERNAL_ERROR".to_string(),
        }
    }
}

/// GET /api/connectors - List all connectors.
pub async fn list_connectors(State(state): State<AppState>) -> impl IntoResponse {
    match state.connector_registry.list().await {
        Ok(connectors) => Json(connectors).into_response(),
        Err(e) => {
            error!(error = %e, "Failed to list connectors");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::internal(&e.to_string())),
            )
                .into_response()
        }
    }
}

/// POST /api/connectors - Create a new connector.
pub async fn create_connector(
    State(state): State<AppState>,
    Json(request): Json<CreateConnectorRequest>,
) -> impl IntoResponse {
    info!(connector_id = %request.id, "Creating connector");

    match state.connector_registry.create(request).await {
        Ok(connector) => (StatusCode::CREATED, Json(connector)).into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::AlreadyExists(id) => (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse::already_exists(id)),
                )
                    .into_response(),
                ConnectorServiceError::InvalidId(msg) => (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::invalid_id(msg)),
                )
                    .into_response(),
                _ => {
                    error!(error = %e, "Failed to create connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// GET /api/connectors/:id - Get a connector by ID.
pub async fn get_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.connector_registry.get(&id).await {
        Ok(connector) => Json(connector).into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to get connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// PUT /api/connectors/:id - Update a connector.
pub async fn update_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateConnectorRequest>,
) -> impl IntoResponse {
    info!(connector_id = %id, "Updating connector");

    match state.connector_registry.update(&id, request).await {
        Ok(connector) => Json(connector).into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to update connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// DELETE /api/connectors/:id - Delete a connector.
pub async fn delete_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(connector_id = %id, "Deleting connector");

    match state.connector_registry.delete(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to delete connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// GET /api/connectors/:id/metadata - Get connector capabilities and metadata.
pub async fn get_connector_metadata(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.connector_registry.get(&id).await {
        Ok(connector) => Json(connector.metadata).into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to get connector metadata");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// POST /api/connectors/:id/test - Test connector connectivity.
pub async fn test_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(connector_id = %id, "Testing connector connectivity");

    match state.connector_registry.test(&id).await {
        Ok(result) => {
            let status = if result.success {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            (status, Json(result)).into_response()
        }
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to test connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// POST /api/connectors/:id/enable - Enable a connector.
pub async fn enable_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(connector_id = %id, "Enabling connector");

    match state
        .connector_registry
        .update(
            &id,
            UpdateConnectorRequest {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
    {
        Ok(connector) => Json(connector).into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to enable connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// POST /api/connectors/:id/disable - Disable a connector.
pub async fn disable_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(connector_id = %id, "Disabling connector");

    match state
        .connector_registry
        .update(
            &id,
            UpdateConnectorRequest {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
    {
        Ok(connector) => Json(connector).into_response(),
        Err(e) => {
            use crate::connectors::ConnectorServiceError;
            match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to disable connector");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::internal(&e.to_string())),
                    )
                        .into_response()
                }
            }
        }
    }
}

/// POST /api/connectors/:id/inbound - Receive an inbound message from a connector.
pub async fn inbound(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<InboundPayload>,
) -> impl IntoResponse {
    info!(connector_id = %id, "Inbound message from connector");

    // Look up connector
    let connector = match state.connector_registry.get(&id).await {
        Ok(c) => c,
        Err(e) => {
            return match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::internal(&e.to_string())),
                )
                    .into_response(),
            };
        }
    };

    // Validate connector is enabled and inbound is allowed
    if !connector.enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: format!("Connector '{}' is disabled", id),
                code: "CONNECTOR_DISABLED".to_string(),
            }),
        )
            .into_response();
    }

    if !connector.inbound_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: format!("Inbound not enabled for connector '{}'", id),
                code: "INBOUND_DISABLED".to_string(),
            }),
        )
            .into_response();
    }

    // Get the runner
    let runner = match state.runtime.runner() {
        Some(r) => r,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::internal("Execution runner not initialized")),
            )
                .into_response();
        }
    };

    // Save thread_id for logging before it's moved into the request
    let log_thread_id = payload.thread_id.clone();

    // Build SessionRequest
    let agent_id = payload.agent_id.as_deref().unwrap_or("root");
    let respond_to = payload.respond_to.unwrap_or_else(|| vec![id.clone()]);

    let mut request = SessionRequest::new(agent_id, &payload.message)
        .with_source(TriggerSource::Connector)
        .with_connector_id(&id)
        .with_respond_to(respond_to);

    if let Some(thread_id) = payload.thread_id {
        request = request.with_thread_id(thread_id);
    }

    if let Some(metadata) = payload.metadata {
        // Merge sender info into metadata if present
        let mut meta = metadata;
        if let Some(sender) = &payload.sender {
            if let serde_json::Value::Object(ref mut map) = meta {
                map.insert(
                    "sender".to_string(),
                    serde_json::json!({
                        "id": sender.id,
                        "name": sender.name,
                    }),
                );
            }
        }
        request = request.with_metadata(meta);
    } else if let Some(sender) = &payload.sender {
        request = request.with_metadata(serde_json::json!({
            "sender": {
                "id": sender.id,
                "name": sender.name,
            }
        }));
    }

    // Submit via bus
    let bus = HttpGatewayBus::new(
        runner.clone(),
        state.state_service.clone(),
        state.config_dir.clone(),
    );

    match bus.submit(request).await {
        Ok(handle) => {
            info!(
                connector_id = %id,
                session_id = %handle.session_id,
                "Inbound session created"
            );

            // Log the inbound message
            state
                .connector_registry
                .log_inbound(InboundLogEntry {
                    connector_id: id.clone(),
                    message: payload.message,
                    sender: payload.sender,
                    thread_id: log_thread_id,
                    session_id: handle.session_id.clone(),
                    received_at: chrono::Utc::now(),
                })
                .await;

            (
                StatusCode::ACCEPTED,
                Json(InboundResult {
                    session_id: handle.session_id,
                    accepted: true,
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(connector_id = %id, error = %e, "Failed to submit inbound session");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::internal(&e.to_string())),
            )
                .into_response()
        }
    }
}

/// Query parameters for inbound log.
#[derive(Debug, Deserialize)]
pub struct InboundLogQuery {
    #[serde(default = "default_log_limit")]
    pub limit: usize,
}

fn default_log_limit() -> usize {
    50
}

/// GET /api/connectors/:id/inbound-log - Get recent inbound messages.
pub async fn get_inbound_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<InboundLogQuery>,
) -> impl IntoResponse {
    // Verify connector exists
    match state.connector_registry.get(&id).await {
        Ok(_) => {}
        Err(e) => {
            return match &e {
                ConnectorServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::internal(&e.to_string())),
                )
                    .into_response(),
            };
        }
    }

    let limit = query.limit.min(500);
    let entries = state.connector_registry.get_inbound_log(&id, limit).await;
    Json(entries).into_response()
}
