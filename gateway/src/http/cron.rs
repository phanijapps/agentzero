//! # Cron HTTP Endpoints
//!
//! REST API for managing cron jobs.

use crate::cron::{CreateCronJobRequest, UpdateCronJobRequest};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use tracing::{error, info};

/// Error response for cron operations.
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

impl ErrorResponse {
    fn not_found(id: &str) -> Self {
        Self {
            error: format!("Cron job not found: {}", id),
            code: "JOB_NOT_FOUND".to_string(),
        }
    }

    fn already_exists(id: &str) -> Self {
        Self {
            error: format!("Cron job already exists: {}", id),
            code: "JOB_EXISTS".to_string(),
        }
    }

    fn invalid_id(msg: &str) -> Self {
        Self {
            error: msg.to_string(),
            code: "INVALID_ID".to_string(),
        }
    }

    fn invalid_schedule(msg: &str) -> Self {
        Self {
            error: msg.to_string(),
            code: "INVALID_SCHEDULE".to_string(),
        }
    }

    fn scheduler_not_available() -> Self {
        Self {
            error: "Cron scheduler not available".to_string(),
            code: "SCHEDULER_UNAVAILABLE".to_string(),
        }
    }

    fn internal(msg: &str) -> Self {
        Self {
            error: msg.to_string(),
            code: "INTERNAL_ERROR".to_string(),
        }
    }
}

/// GET /api/cron - List all cron jobs.
pub async fn list_cron_jobs(State(state): State<AppState>) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    match scheduler.list_jobs().await {
        Ok(jobs) => Json(jobs).into_response(),
        Err(e) => {
            error!(error = %e, "Failed to list cron jobs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::internal(&e.to_string())),
            )
                .into_response()
        }
    }
}

/// POST /api/cron - Create a new cron job.
pub async fn create_cron_job(
    State(state): State<AppState>,
    Json(request): Json<CreateCronJobRequest>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    info!(job_id = %request.id, "Creating cron job");

    match scheduler.create_job(request).await {
        Ok(job) => (StatusCode::CREATED, Json(job)).into_response(),
        Err(e) => {
            use crate::cron::CronServiceError;
            match &e {
                CronServiceError::AlreadyExists(id) => (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse::already_exists(id)),
                )
                    .into_response(),
                CronServiceError::InvalidId(msg) => (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::invalid_id(msg)),
                )
                    .into_response(),
                CronServiceError::InvalidSchedule(msg) => (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::invalid_schedule(msg)),
                )
                    .into_response(),
                _ => {
                    error!(error = %e, "Failed to create cron job");
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

/// GET /api/cron/:id - Get a cron job by ID.
pub async fn get_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    match scheduler.get_job(&id).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            use crate::cron::CronServiceError;
            match &e {
                CronServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to get cron job");
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

/// PUT /api/cron/:id - Update a cron job.
pub async fn update_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateCronJobRequest>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    info!(job_id = %id, "Updating cron job");

    match scheduler.update_job(&id, request).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            use crate::cron::CronServiceError;
            match &e {
                CronServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                CronServiceError::InvalidSchedule(msg) => (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::invalid_schedule(msg)),
                )
                    .into_response(),
                _ => {
                    error!(error = %e, "Failed to update cron job");
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

/// DELETE /api/cron/:id - Delete a cron job.
pub async fn delete_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    info!(job_id = %id, "Deleting cron job");

    match scheduler.delete_job(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            use crate::cron::CronServiceError;
            match &e {
                CronServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to delete cron job");
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

/// POST /api/cron/:id/trigger - Manually trigger a cron job.
pub async fn trigger_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    info!(job_id = %id, "Manually triggering cron job");

    match scheduler.trigger(&id).await {
        Ok(result) => {
            let status = if result.success {
                StatusCode::OK
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(result)).into_response()
        }
        Err(e) => {
            use crate::cron::CronSchedulerError;
            match &e {
                CronSchedulerError::Service(crate::cron::CronServiceError::NotFound(id)) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to trigger cron job");
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

/// POST /api/cron/:id/enable - Enable a cron job.
pub async fn enable_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    info!(job_id = %id, "Enabling cron job");

    match scheduler.enable_job(&id).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            use crate::cron::CronServiceError;
            match &e {
                CronServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to enable cron job");
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

/// POST /api/cron/:id/disable - Disable a cron job.
pub async fn disable_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let scheduler = match &state.cron_scheduler {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::scheduler_not_available()),
            )
                .into_response();
        }
    };

    info!(job_id = %id, "Disabling cron job");

    match scheduler.disable_job(&id).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            use crate::cron::CronServiceError;
            match &e {
                CronServiceError::NotFound(id) => {
                    (StatusCode::NOT_FOUND, Json(ErrorResponse::not_found(id))).into_response()
                }
                _ => {
                    error!(error = %e, "Failed to disable cron job");
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
