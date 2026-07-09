//! Gateway Bus HTTP endpoints.
//!
//! Provides HTTP endpoints for external connectors to submit sessions
//! to the gateway.
//!
//! # Endpoints
//!
//! - `POST /api/gateway/submit` - Submit a new session or continue an existing one
//! - `GET /api/gateway/status/:session_id` - Get session status
//! - `POST /api/gateway/cancel/:session_id` - Cancel a session
//! - `POST /api/gateway/pause/:session_id` - Pause a session
//! - `POST /api/gateway/resume/:session_id` - Resume a session
//!
//! # Example (Python)
//!
//! ```python
//! import requests
//!
//! # Submit a new session
//! response = requests.post("http://localhost:18791/api/gateway/submit", json={
//!     "agent_id": "root",
//!     "message": "Hello from Python!",
//!     "source": "plugin",
//!     "external_ref": "python-script-123"
//! })
//! handle = response.json()
//! print(f"Session: {handle['session_id']}, Execution: {handle['execution_id']}")
//!
//! # Check status
//! status = requests.get(f"http://localhost:18791/api/gateway/status/{handle['session_id']}")
//! print(f"Status: {status.json()}")
//!
//! # Cancel if needed
//! requests.post(f"http://localhost:18791/api/gateway/cancel/{handle['session_id']}")
//! ```

use crate::bus::{BusError, GatewayBus, HttpGatewayBus, SessionHandle, SessionRequest};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

/// Error response type for the gateway bus endpoints.
type ApiError = (StatusCode, Json<ErrorResponse>);

/// Response for session status.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub session_id: String,
    pub status: String,
}

/// Response for error cases.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

/// Convert a BusError into an API error response.
fn bus_error_to_response(err: BusError) -> ApiError {
    let (status, code) = match &err {
        BusError::SessionNotFound(_) => (StatusCode::NOT_FOUND, "SESSION_NOT_FOUND"),
        BusError::ExecutionNotFound(_) => (StatusCode::NOT_FOUND, "EXECUTION_NOT_FOUND"),
        BusError::AgentError(_) => (StatusCode::BAD_REQUEST, "AGENT_ERROR"),
        BusError::ProviderError(_) => (StatusCode::BAD_REQUEST, "PROVIDER_ERROR"),
        BusError::InvalidState { .. } => (StatusCode::CONFLICT, "INVALID_STATE"),
        BusError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
            code: code.to_string(),
        }),
    )
}

/// Create the gateway bus router.
///
/// Note: This returns `Router<AppState>` to be nested in the main router.
/// The state is provided by the parent router via `.with_state()`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/submit", post(submit_session))
        .route("/status/:session_id", get(get_status))
        .route("/cancel/:session_id", post(cancel_session))
        .route("/pause/:session_id", post(pause_session))
        .route("/resume/:session_id", post(resume_session))
}

/// Submit a new session or continue an existing one.
///
/// # Request Body
///
/// ```json
/// {
///   "agent_id": "root",
///   "message": "Hello, agent!",
///   "source": "plugin",
///   "session_id": null,
///   "priority": null,
///   "external_ref": "my-script-123",
///   "metadata": { "custom": "data" }
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "session_id": "sess-abc123",
///   "execution_id": "exec-def456",
///   "conversation_id": "web-ghi789"
/// }
/// ```
pub async fn submit_session(
    State(state): State<AppState>,
    Json(request): Json<SessionRequest>,
) -> Result<Json<SessionHandle>, ApiError> {
    // Get the runner from runtime service
    let runner = state.runtime.runner().ok_or_else(|| {
        bus_error_to_response(BusError::Internal(
            "Execution runner not initialized".to_string(),
        ))
    })?;

    // Create the gateway bus
    let bus = HttpGatewayBus::new(
        runner.clone(),
        state.state_service.clone(),
        state.config_dir.clone(),
    );

    // Submit the session
    let handle = bus.submit(request).await.map_err(bus_error_to_response)?;

    Ok(Json(handle))
}

/// Get the status of a session.
///
/// # Response
///
/// ```json
/// {
///   "session_id": "sess-abc123",
///   "status": "running"
/// }
/// ```
pub async fn get_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<StatusResponse>, ApiError> {
    let runner = state.runtime.runner().ok_or_else(|| {
        bus_error_to_response(BusError::Internal(
            "Execution runner not initialized".to_string(),
        ))
    })?;

    let bus = HttpGatewayBus::new(
        runner.clone(),
        state.state_service.clone(),
        state.config_dir.clone(),
    );

    let status = bus
        .status(&session_id)
        .await
        .map_err(bus_error_to_response)?;

    Ok(Json(StatusResponse {
        session_id,
        status: format!("{:?}", status).to_lowercase(),
    }))
}

/// Cancel a running session.
pub async fn cancel_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let runner = state.runtime.runner().ok_or_else(|| {
        bus_error_to_response(BusError::Internal(
            "Execution runner not initialized".to_string(),
        ))
    })?;

    let bus = HttpGatewayBus::new(
        runner.clone(),
        state.state_service.clone(),
        state.config_dir.clone(),
    );

    bus.cancel(&session_id)
        .await
        .map_err(bus_error_to_response)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Pause a running session.
pub async fn pause_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let runner = state.runtime.runner().ok_or_else(|| {
        bus_error_to_response(BusError::Internal(
            "Execution runner not initialized".to_string(),
        ))
    })?;

    let bus = HttpGatewayBus::new(
        runner.clone(),
        state.state_service.clone(),
        state.config_dir.clone(),
    );

    bus.pause(&session_id)
        .await
        .map_err(bus_error_to_response)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Resume a paused session.
pub async fn resume_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let runner = state.runtime.runner().ok_or_else(|| {
        bus_error_to_response(BusError::Internal(
            "Execution runner not initialized".to_string(),
        ))
    })?;

    let bus = HttpGatewayBus::new(
        runner.clone(),
        state.state_service.clone(),
        state.config_dir.clone(),
    );

    bus.resume(&session_id)
        .await
        .map_err(bus_error_to_response)?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use execution_state::TriggerSource;

    #[test]
    fn test_session_request_deserialization() {
        let json = r#"{
            "agent_id": "root",
            "message": "Hello!",
            "source": "plugin",
            "external_ref": "test-123"
        }"#;

        let request: SessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.agent_id, "root");
        assert_eq!(request.message, "Hello!");
        assert_eq!(request.source, TriggerSource::Connector);
        assert_eq!(request.external_ref, Some("test-123".to_string()));
    }

    #[test]
    fn test_minimal_session_request() {
        let json = r#"{
            "agent_id": "root",
            "message": "Hello!"
        }"#;

        let request: SessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.agent_id, "root");
        assert_eq!(request.message, "Hello!");
        assert_eq!(request.source, TriggerSource::Web); // default
        assert!(request.session_id.is_none());
    }
}
