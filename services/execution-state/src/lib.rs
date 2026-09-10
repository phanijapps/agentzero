//! # Execution State
//!
//! Session state tracking, execution management, and checkpointing.
//!
//! This crate provides:
//! - Session lifecycle management (running, paused, completed, crashed)
//! - Agent execution tracking (root and delegated subagents)
//! - Token consumption tracking
//! - Checkpointing for crash recovery
//! - HTTP handlers for REST API
//!
//! ## Usage
//!
//! ```ignore
//! use execution_state::{StateService, StateDbProvider, routes};
//!
//! impl StateDbProvider for DatabaseManager { ... }
//!
//! let state_service = Arc::new(StateService::new(db.clone()));
//! router.nest("/api/state", routes(state_service.clone()));
//!
//! // During execution
//! let (session, execution) = state_service.create_session(agent_id)?;
//! state_service.start_execution(&execution.id)?;
//! state_service.update_execution_tokens(&execution.id, tokens_in, tokens_out)?;
//! state_service.complete_execution(&execution.id)?;
//! state_service.complete_session(&session.id)?;
//! ```

pub mod handlers;
mod repository;
mod service;
mod types;
mod work;

#[cfg(test)]
pub mod test_utils;

// Re-export public types
pub use repository::StateDbProvider;
pub use service::StateService;
pub use types::*;
pub use work::*;

use axum::{routing::delete, routing::get, routing::post, Router};
use std::sync::Arc;

/// Create the session state API router.
///
/// # Endpoints
///
/// ## Sessions (V2 API)
/// - `GET /v2/sessions` - List sessions (basic)
/// - `GET /v2/sessions/full` - List sessions with executions (dashboard)
/// - `GET /v2/mission-control/sessions` - List bounded Mission Control summaries
/// - `GET /v2/mission-control/sessions/:id/tokens` - Selected-session token slices
/// - `GET /v2/sessions/:id` - Get session detail
/// - `GET /v2/sessions/:id/full` - Get session with all executions
/// - `GET /v2/sessions/:id/messages` - Get session messages with scope filtering
/// - `DELETE /v2/sessions/:id` - Delete a session
///
/// ## Executions (root paths - nested under /api/executions in gateway)
/// - `GET /` - List executions with filtering
/// - `GET /:id` - Get execution detail
/// - `GET /:id/children` - Get child executions
/// - `GET /:id/messages` - Get messages for execution
///
/// ## Control
/// - `POST /sessions/:id/pause` - Pause a running session
/// - `POST /sessions/:id/resume` - Resume a paused session
/// - `POST /sessions/:id/cancel` - Cancel a session
///
/// ## Stats
/// - `GET /stats` - Get dashboard stats (session + execution counts)
/// - `GET /stats/counts` - Get stats as key-value map
pub fn routes<D>(service: Arc<StateService<D>>) -> Router
where
    D: StateDbProvider + 'static,
{
    Router::new()
        // Sessions (V2 API - use /v2/sessions/full for dashboard)
        .route("/v2/sessions", get(handlers::list_sessions::<D>))
        .route("/v2/sessions/full", get(handlers::list_sessions_full::<D>))
        .route(
            "/v2/mission-control/sessions",
            get(handlers::list_mission_control_sessions::<D>),
        )
        .route(
            "/v2/mission-control/sessions/:id/tokens",
            get(handlers::get_mission_control_session_tokens::<D>),
        )
        .route(
            "/v2/sessions/:id",
            get(handlers::get_session::<D>).delete(handlers::delete_session::<D>),
        )
        .route(
            "/v2/sessions/:id/full",
            get(handlers::get_session_full::<D>),
        )
        .route(
            "/v2/sessions/:id/messages",
            get(handlers::get_session_messages::<D>),
        )
        // Session control (works with both old and new IDs)
        .route("/sessions/:id/pause", post(handlers::pause_session::<D>))
        .route("/sessions/:id/resume", post(handlers::resume_session::<D>))
        .route("/sessions/:id/cancel", post(handlers::cancel_session::<D>))
        // Executions (no /executions prefix since already nested under /api/executions)
        .route("/", get(handlers::list_executions::<D>))
        .route("/:id", get(handlers::get_execution::<D>))
        .route("/:id/children", get(handlers::get_child_executions::<D>))
        .route("/:id/messages", get(handlers::get_execution_messages::<D>))
        // Stats
        .route("/stats", get(handlers::get_dashboard_stats::<D>))
        .route("/stats/counts", get(handlers::get_stats_counts::<D>))
        // Cleanup
        .route("/cleanup", delete(handlers::cleanup_old_sessions::<D>))
        .with_state(service)
}
