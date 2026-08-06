//! A2A 1.0 HTTP+JSON routes for explicitly paired zBots.

use crate::a2a_tasks::{A2aPeerIdentity, A2aTaskError, A2aTaskService};
use crate::config::GatewayConfig;
use axum::body::{to_bytes, Body};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use execution_state::{WorkCursor, WorkStore};
use gateway_a2a::peers::PeerStore;
use gateway_a2a::{
    agent_card, error_response, list_tasks_response, parse_send_message_request,
    send_message_response, validate_headers, AgentCardConfig, AgentSkillConfig, ProtocolError,
    APPLICATION_A2A_JSON, MAX_SERIALIZED_INBOUND_REQUEST_BYTES, MAX_TASK_ID_BYTES,
};
use gateway_bus::WorkTransport;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct A2aHttpState {
    peers: PeerStore,
    tasks: A2aTaskService,
    allowed_origins: Arc<Vec<String>>,
    card: a2a::AgentCard,
    public_skill_instructions: Arc<String>,
    runtime: Arc<crate::services::RuntimeService>,
}

impl A2aHttpState {
    pub(crate) fn new(
        config: &GatewayConfig,
        data_dir: &std::path::Path,
        store: Arc<dyn WorkStore>,
        transport: Arc<dyn WorkTransport>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        runtime: Arc<crate::services::RuntimeService>,
    ) -> Result<Self, ProtocolError> {
        let base_url = config
            .a2a_public_base_url
            .clone()
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", config.http_port));
        let card = agent_card(AgentCardConfig {
            name: "zBot".to_string(),
            description: "Explicitly paired, bounded A2A task execution".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            base_url,
            skills: vec![AgentSkillConfig {
                id: "generic-task".to_string(),
                name: "Generic task".to_string(),
                description: "Answer a bounded text request using the paired public skill"
                    .to_string(),
                tags: vec!["assistant".to_string()],
                examples: Vec::new(),
            }],
        })?;
        Ok(Self {
            peers: PeerStore::new(data_dir),
            tasks: A2aTaskService::new(store, transport, messages),
            allowed_origins: Arc::new(config.a2a_allowed_origins.clone()),
            card,
            public_skill_instructions: Arc::new(config.a2a_public_skill_instructions.clone()),
            runtime,
        })
    }
}

pub(crate) fn routes(state: A2aHttpState) -> Router {
    Router::new()
        .route("/.well-known/agent-card.json", get(agent_card_handler))
        .route("/a2a/*path", any(a2a_operation))
        .with_state(state)
}

async fn a2a_operation(
    State(state): State<A2aHttpState>,
    Path(path): Path<String>,
    request: Request<Body>,
) -> Response {
    match (request.method(), path.as_str()) {
        (&Method::POST, "message:send") => send_message(&state, request).await,
        (&Method::GET, "tasks") => {
            let query = match Query::<ListTaskQuery>::try_from_uri(request.uri()) {
                Ok(Query(query)) => query,
                Err(_) => return protocol_error(ProtocolError::InvalidParams),
            };
            list_tasks(&state, request.headers(), query).await
        }
        (&Method::GET, path) if path.starts_with("tasks/") && !path.ends_with(":cancel") => {
            let task_id = path.trim_start_matches("tasks/");
            let query = match Query::<GetTaskQuery>::try_from_uri(request.uri()) {
                Ok(Query(query)) => query,
                Err(_) => return protocol_error(ProtocolError::InvalidParams),
            };
            get_task(&state, task_id, request.headers(), query)
        }
        (&Method::POST, path) if path.starts_with("tasks/") && path.ends_with(":cancel") => {
            cancel_task(&state, path.trim_start_matches("tasks/"), request.headers()).await
        }
        _ => unsupported_operation(&state, request.headers()),
    }
}

async fn agent_card_handler(State(state): State<A2aHttpState>) -> Response {
    let mut response = Json(state.card).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_static("\"zbot-a2a-1\""));
    response
}

async fn send_message(state: &A2aHttpState, request: Request<Body>) -> Response {
    let peer = match authenticate(state, request.headers(), true) {
        Ok(peer) => peer,
        Err(error) => return protocol_error(error),
    };
    let body = match to_bytes(
        request.into_body(),
        MAX_SERIALIZED_INBOUND_REQUEST_BYTES + 1,
    )
    .await
    {
        Ok(body) => body,
        Err(_) => return protocol_error(ProtocolError::PayloadTooLarge),
    };
    let (_, accepted) = match parse_send_message_request(&body) {
        Ok(request) => request,
        Err(error) => return protocol_error(error),
    };
    match state
        .tasks
        .submit(
            &peer,
            &accepted.message_id,
            &accepted.text,
            &state.public_skill_instructions,
        )
        .await
    {
        Ok(task) => a2a_json(StatusCode::OK, send_message_response(task)),
        Err(error) => task_error(error),
    }
}

fn get_task(
    state: &A2aHttpState,
    task_id: &str,
    headers: &HeaderMap,
    query: GetTaskQuery,
) -> Response {
    let peer = match authenticate(state, headers, false) {
        Ok(peer) => peer,
        Err(error) => return protocol_error(error),
    };
    if !valid_task_id(task_id) {
        return protocol_error(ProtocolError::TaskNotFound);
    }
    match state
        .tasks
        .get(&peer.peer_id, task_id, query.include_artifacts)
    {
        Ok(task) => a2a_json(StatusCode::OK, task),
        Err(error) => task_error(error),
    }
}

async fn list_tasks(state: &A2aHttpState, headers: &HeaderMap, query: ListTaskQuery) -> Response {
    let peer = match authenticate(state, headers, false) {
        Ok(peer) => peer,
        Err(error) => return protocol_error(error),
    };
    let limit = query.page_size.unwrap_or(50);
    if limit == 0 || limit > 100 {
        return protocol_error(ProtocolError::InvalidParams);
    }
    let cursor = match query.page_token.as_deref() {
        Some(token) => match decode_cursor(token, &peer.peer_id) {
            Ok(cursor) => Some(cursor),
            Err(error) => return protocol_error(error),
        },
        None => None,
    };
    let (page, tasks) = match state.tasks.list(
        &peer.peer_id,
        cursor.as_ref(),
        limit,
        query.include_artifacts,
    ) {
        Ok(page) => page,
        Err(error) => return task_error(error),
    };
    let next = page
        .next_cursor()
        .map(|cursor| encode_cursor(cursor, &peer.peer_id));
    let total = i32::try_from(page.total_size()).unwrap_or(i32::MAX);
    match list_tasks_response(tasks, next, i32::from(limit), total) {
        Ok(response) => a2a_json(StatusCode::OK, response),
        Err(error) => protocol_error(error),
    }
}

async fn cancel_task(state: &A2aHttpState, path_id: &str, headers: &HeaderMap) -> Response {
    let peer = match authenticate(state, headers, false) {
        Ok(peer) => peer,
        Err(error) => return protocol_error(error),
    };
    let Some(task_id) = path_id.strip_suffix(":cancel") else {
        return protocol_error(ProtocolError::UnsupportedOperation);
    };
    if !valid_task_id(task_id) {
        return protocol_error(ProtocolError::TaskNotFound);
    }
    match state.tasks.cancel(&peer.peer_id, task_id) {
        Ok(result) => {
            if result.newly_canceled {
                let _ = state
                    .runtime
                    .cancel_exact(&result.session_id, &result.conversation_id)
                    .await;
            }
            a2a_json(StatusCode::OK, result.task)
        }
        Err(error) => task_error(error),
    }
}

fn unsupported_operation(state: &A2aHttpState, headers: &HeaderMap) -> Response {
    if let Err(error) = authenticate(state, headers, false) {
        return protocol_error(error);
    }
    protocol_error(ProtocolError::UnsupportedOperation)
}

fn authenticate(
    state: &A2aHttpState,
    headers: &HeaderMap,
    require_content_type: bool,
) -> Result<A2aPeerIdentity, ProtocolError> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let allowed = origin.to_str().ok().is_some_and(|origin| {
            state
                .allowed_origins
                .iter()
                .any(|allowed| allowed == origin)
        });
        if !allowed {
            return Err(ProtocolError::Unauthorized);
        }
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
        .ok_or(ProtocolError::Unauthorized)?;
    let snapshot = state
        .peers
        .load_snapshot()
        .map_err(|_| ProtocolError::InternalError)?;
    let peer = snapshot
        .authenticate_inbound_token(token)
        .ok_or(ProtocolError::Unauthorized)?;
    let version = headers
        .get("A2A-Version")
        .and_then(|value| value.to_str().ok());
    if require_content_type {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        validate_headers(version, content_type)?;
    } else if version != Some(a2a::VERSION) {
        return Err(ProtocolError::VersionNotSupported);
    }
    Ok(A2aPeerIdentity {
        peer_id: peer.node_id.clone(),
        target_agent_id: peer.target_agent_id.clone(),
    })
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() && !token.contains(' '))
        .then_some(token)
}

fn valid_task_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TASK_ID_BYTES
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTaskQuery {
    #[serde(default)]
    include_artifacts: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListTaskQuery {
    page_size: Option<u16>,
    page_token: Option<String>,
    #[serde(default)]
    include_artifacts: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorToken {
    peer_id: String,
    updated_at: DateTime<Utc>,
    id: String,
}

fn encode_cursor(cursor: &WorkCursor, peer_id: &str) -> String {
    let token = CursorToken {
        peer_id: peer_id.to_string(),
        updated_at: cursor.updated_at(),
        id: cursor.id().to_string(),
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&token).expect("cursor serialization is infallible"))
}

fn decode_cursor(value: &str, peer_id: &str) -> Result<WorkCursor, ProtocolError> {
    if value.len() > 512 {
        return Err(ProtocolError::InvalidParams);
    }
    let raw = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProtocolError::InvalidParams)?;
    let token: CursorToken =
        serde_json::from_slice(&raw).map_err(|_| ProtocolError::InvalidParams)?;
    if token.peer_id != peer_id {
        return Err(ProtocolError::InvalidParams);
    }
    WorkCursor::new(token.updated_at, token.id).map_err(|_| ProtocolError::InvalidParams)
}

fn task_error(error: A2aTaskError) -> Response {
    protocol_error(match error {
        A2aTaskError::InvalidRequest | A2aTaskError::MessageConflict => {
            ProtocolError::InvalidParams
        }
        A2aTaskError::NotFound => ProtocolError::TaskNotFound,
        A2aTaskError::NotCancelable => ProtocolError::TaskNotCancelable,
        A2aTaskError::StorageUnavailable => ProtocolError::InternalError,
    })
}

fn protocol_error(error: ProtocolError) -> Response {
    let response = error_response(error);
    let status =
        StatusCode::from_u16(response.error.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = a2a_json(status, response);
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

fn a2a_json<T: Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_A2A_JSON),
    );
    response
        .headers_mut()
        .insert("A2A-Version", HeaderValue::from_static(a2a::VERSION));
    response
}
