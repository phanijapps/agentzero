use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct TraceQueryRequest {
    pub preset: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct TraceQueryResponse {
    pub rows: Vec<TraceQueryRow>,
}

#[derive(Debug, Serialize)]
pub struct TraceQueryRow {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
struct TraceQueryError {
    error: String,
}

pub async fn query_traces(
    State(state): State<AppState>,
    Json(request): Json<TraceQueryRequest>,
) -> Response {
    match request.preset.as_str() {
        "sessions_with_failed_tool" => {
            let Some(tool) = request.params.get("tool").and_then(|v| v.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(TraceQueryError {
                        error: "params.tool is required".to_string(),
                    }),
                )
                    .into_response();
            };

            match state.trace_analytics().sessions_with_failed_tool(tool) {
                Ok(sessions) => Json(TraceQueryResponse {
                    rows: sessions
                        .into_iter()
                        .map(|session_id| TraceQueryRow { session_id })
                        .collect(),
                })
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(TraceQueryError {
                        error: e.to_string(),
                    }),
                )
                    .into_response(),
            }
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(TraceQueryError {
                error: format!("unknown trace query preset: {}", request.preset),
            }),
        )
            .into_response(),
    }
}
