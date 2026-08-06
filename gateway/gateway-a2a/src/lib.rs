use a2a::{
    AgentCapabilities, AgentCard, AgentInterface, AgentSkill, Artifact, CancelTaskRequest,
    HttpAuthSecurityScheme, ListTasksResponse, Message, Part, PartContent, Role, SecurityScheme,
    SendMessageRequest, SendMessageResponse, Task, TaskState, TaskStatus,
    TRANSPORT_PROTOCOL_HTTP_JSON, VERSION,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

pub mod config;
pub mod peers;
pub mod registry;
pub mod secrets;
pub mod url_policy;

pub const APPLICATION_A2A_JSON: &str = "application/a2a+json";
pub const TEXT_PLAIN: &str = "text/plain";
pub const MAX_SERIALIZED_INBOUND_REQUEST_BYTES: usize = 65_536;
pub const MAX_TEXT_CODE_POINTS: usize = 1_000;
pub const MAX_TEXT_UTF8_BYTES: usize = 4_000;
pub const MAX_TASK_ID_BYTES: usize = 128;
pub const MAX_PAGE_SIZE: i32 = 100;

const SECURITY_SCHEME_NAME: &str = "peerBearer";
const ERROR_DOMAIN: &str = "a2a-protocol.org";
const ERROR_INFO_TYPE: &str = "type.googleapis.com/google.rpc.ErrorInfo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardConfig {
    pub name: String,
    pub description: String,
    pub version: String,
    pub base_url: String,
    pub skills: Vec<AgentSkillConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkillConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedTextMessage {
    pub message_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskProjection {
    pub id: String,
    pub context_id: String,
    pub state: TaskProjectionState,
    pub message: Option<String>,
    pub artifact_text: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskProjectionState {
    Submitted,
    Working,
    Completed,
    Failed,
    Canceled,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid request")]
    InvalidRequest,
    #[error("invalid parameters")]
    InvalidParams,
    #[error("unsupported operation")]
    UnsupportedOperation,
    #[error("content type not supported")]
    ContentTypeNotSupported,
    #[error("version not supported")]
    VersionNotSupported,
    #[error("extension support required")]
    ExtensionSupportRequired,
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("unauthorized")]
    Unauthorized,
    #[error("task not found")]
    TaskNotFound,
    #[error("task not cancelable")]
    TaskNotCancelable,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: RpcStatus,
}

impl std::fmt::Debug for ErrorResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ErrorResponse")
            .field("code", &self.error.code)
            .field("status", &self.error.status)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcStatus {
    pub code: u16,
    pub status: String,
    pub message: String,
    pub details: Vec<ErrorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    #[serde(rename = "@type")]
    pub type_url: String,
    pub reason: String,
    pub domain: String,
}

pub fn agent_card(config: AgentCardConfig) -> Result<AgentCard, ProtocolError> {
    validate_len(&config.name, 1, 80)?;
    validate_len(&config.description, 0, 500)?;
    validate_len(&config.version, 1, 32)?;
    if config.skills.is_empty() || config.skills.len() > 16 {
        return Err(ProtocolError::InvalidParams);
    }

    let mut security_schemes = HashMap::new();
    security_schemes.insert(
        SECURITY_SCHEME_NAME.to_string(),
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "Bearer".to_string(),
            description: Some("Random per-peer secret provisioned out of band".to_string()),
            bearer_format: None,
        }),
    );

    let mut security_requirement = HashMap::new();
    security_requirement.insert(SECURITY_SCHEME_NAME.to_string(), Vec::new());

    Ok(AgentCard {
        name: config.name,
        description: config.description,
        version: config.version,
        supported_interfaces: vec![AgentInterface {
            url: format!("{}/a2a", config.base_url.trim_end_matches('/')),
            protocol_binding: TRANSPORT_PROTOCOL_HTTP_JSON.to_string(),
            protocol_version: VERSION.to_string(),
            tenant: None,
        }],
        capabilities: AgentCapabilities {
            streaming: Some(false),
            push_notifications: Some(false),
            extensions: None,
            extended_agent_card: Some(false),
        },
        default_input_modes: vec![TEXT_PLAIN.to_string()],
        default_output_modes: vec![TEXT_PLAIN.to_string()],
        skills: config
            .skills
            .into_iter()
            .map(agent_skill)
            .collect::<Result<Vec<_>, _>>()?,
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: Some(security_schemes),
        security_requirements: Some(vec![security_requirement]),
        signatures: None,
    })
}

fn agent_skill(config: AgentSkillConfig) -> Result<AgentSkill, ProtocolError> {
    validate_len(&config.id, 1, 128)?;
    validate_len(&config.name, 1, 80)?;
    validate_len(&config.description, 0, 500)?;
    if config.tags.len() > 16 || config.examples.len() > 8 {
        return Err(ProtocolError::InvalidParams);
    }
    for tag in &config.tags {
        validate_len(tag, 0, 64)?;
    }
    for example in &config.examples {
        validate_len(example, 0, 500)?;
    }

    Ok(AgentSkill {
        id: config.id,
        name: config.name,
        description: config.description,
        tags: config.tags,
        examples: Some(config.examples),
        input_modes: Some(vec![TEXT_PLAIN.to_string()]),
        output_modes: Some(vec![TEXT_PLAIN.to_string()]),
        security_requirements: None,
    })
}

pub fn validate_headers(
    version: Option<&str>,
    content_type: Option<&str>,
) -> Result<(), ProtocolError> {
    match version {
        Some(VERSION) => {}
        _ => return Err(ProtocolError::VersionNotSupported),
    }

    match content_type {
        Some(APPLICATION_A2A_JSON) => Ok(()),
        _ => Err(ProtocolError::ContentTypeNotSupported),
    }
}

pub fn validate_serialized_request(body: &[u8]) -> Result<(), ProtocolError> {
    if body.len() > MAX_SERIALIZED_INBOUND_REQUEST_BYTES {
        return Err(ProtocolError::PayloadTooLarge);
    }
    Ok(())
}

pub fn parse_send_message_request(
    body: &[u8],
) -> Result<(SendMessageRequest, AcceptedTextMessage), ProtocolError> {
    validate_serialized_request(body)?;
    validate_raw_send_message_shape(body)?;
    let request: SendMessageRequest =
        serde_json::from_slice(body).map_err(|_| ProtocolError::InvalidRequest)?;
    let accepted = validate_send_message_request(&request)?;
    Ok((request, accepted))
}

pub fn validate_send_message_request(
    request: &SendMessageRequest,
) -> Result<AcceptedTextMessage, ProtocolError> {
    if request.metadata.is_some() || request.tenant.is_some() {
        return Err(ProtocolError::InvalidParams);
    }

    let configuration = request
        .configuration
        .as_ref()
        .ok_or(ProtocolError::InvalidParams)?;
    if configuration.accepted_output_modes.as_deref() != Some(&[TEXT_PLAIN.to_string()]) {
        return Err(ProtocolError::InvalidParams);
    }
    if configuration.return_immediately != Some(true)
        || configuration.task_push_notification_config.is_some()
        || configuration.history_length.is_some()
    {
        return Err(ProtocolError::InvalidParams);
    }

    validate_user_message(&request.message)
}

fn validate_user_message(message: &Message) -> Result<AcceptedTextMessage, ProtocolError> {
    if message.message_id.is_empty() || message.message_id.len() > MAX_TASK_ID_BYTES {
        return Err(ProtocolError::InvalidParams);
    }
    if message.role != Role::User
        || message.context_id.is_some()
        || message.task_id.is_some()
        || message.metadata.is_some()
        || message.reference_task_ids.is_some()
    {
        return Err(ProtocolError::InvalidParams);
    }
    if message
        .extensions
        .as_ref()
        .is_some_and(|extensions| !extensions.is_empty())
    {
        return Err(ProtocolError::ExtensionSupportRequired);
    }
    if message.parts.len() != 1 {
        return Err(ProtocolError::InvalidParams);
    }

    let part = &message.parts[0];
    if part.filename.is_some() || part.metadata.is_some() {
        return Err(ProtocolError::InvalidParams);
    }
    if part
        .media_type
        .as_deref()
        .is_some_and(|value| value != TEXT_PLAIN)
    {
        return Err(ProtocolError::InvalidParams);
    }
    let PartContent::Text(text) = &part.content else {
        return Err(ProtocolError::InvalidParams);
    };
    validate_text(text)?;

    Ok(AcceptedTextMessage {
        message_id: message.message_id.clone(),
        text: text.clone(),
    })
}

pub fn project_task(projection: TaskProjection) -> Result<Task, ProtocolError> {
    validate_task_id(&projection.id)?;
    validate_task_id(&projection.context_id)?;

    let message = projection
        .message
        .map(|text| agent_message(derived_id(&projection.id, "status"), text))
        .transpose()?;
    let artifacts = projection
        .artifact_text
        .map(|text| {
            validate_text(&text)?;
            Ok(vec![Artifact {
                artifact_id: derived_id(&projection.id, "artifact"),
                name: None,
                description: None,
                parts: vec![Part::text(text).with_media_type(TEXT_PLAIN)],
                metadata: None,
                extensions: None,
            }])
        })
        .transpose()?;

    Ok(Task {
        id: projection.id,
        context_id: projection.context_id,
        status: TaskStatus {
            state: projection.state.into(),
            message,
            timestamp: projection.updated_at,
        },
        artifacts,
        history: None,
        metadata: None,
    })
}

fn agent_message(message_id: String, text: String) -> Result<Message, ProtocolError> {
    validate_text(&text)?;
    Ok(Message {
        message_id,
        context_id: None,
        task_id: None,
        role: Role::Agent,
        parts: vec![Part::text(text).with_media_type(TEXT_PLAIN)],
        metadata: None,
        extensions: None,
        reference_task_ids: None,
    })
}

pub fn send_message_response(task: Task) -> SendMessageResponse {
    SendMessageResponse::Task(task)
}

pub fn list_tasks_response(
    tasks: Vec<Task>,
    next_page_token: Option<String>,
    page_size: i32,
    total_size: i32,
) -> Result<ListTasksResponse, ProtocolError> {
    if tasks.len() > MAX_PAGE_SIZE as usize || !(0..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(ProtocolError::InvalidParams);
    }
    if total_size < 0 {
        return Err(ProtocolError::InvalidParams);
    }

    Ok(ListTasksResponse {
        tasks,
        next_page_token: next_page_token.unwrap_or_default(),
        page_size,
        total_size,
    })
}

pub fn cancel_task_request(id: impl Into<String>) -> CancelTaskRequest {
    CancelTaskRequest {
        id: id.into(),
        metadata: None,
        tenant: None,
    }
}

pub fn error_response(error: ProtocolError) -> ErrorResponse {
    ErrorResponse {
        error: RpcStatus {
            code: error.http_status(),
            status: error.reason().to_string(),
            message: error.message().to_string(),
            details: vec![ErrorInfo {
                type_url: ERROR_INFO_TYPE.to_string(),
                reason: error.reason().to_string(),
                domain: ERROR_DOMAIN.to_string(),
            }],
        },
    }
}

pub fn supported_paths() -> BTreeSet<String> {
    [
        "/.well-known/agent-card.json",
        "/a2a/message:send",
        "/a2a/tasks",
        "/a2a/tasks/{id}",
        "/a2a/tasks/{id}:cancel",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

pub fn supported_optional_operations() -> BTreeSet<String> {
    [
        "SendStreamingMessage",
        "SubscribeToTask",
        "CreateTaskPushNotificationConfig",
        "GetTaskPushNotificationConfig",
        "ListTaskPushNotificationConfigs",
        "DeleteTaskPushNotificationConfig",
        "GetExtendedAgentCard",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl ProtocolError {
    fn http_status(self) -> u16 {
        match self {
            ProtocolError::Unauthorized => 401,
            ProtocolError::TaskNotFound => 404,
            ProtocolError::PayloadTooLarge => 413,
            ProtocolError::InvalidRequest
            | ProtocolError::InvalidParams
            | ProtocolError::UnsupportedOperation
            | ProtocolError::ContentTypeNotSupported
            | ProtocolError::VersionNotSupported
            | ProtocolError::ExtensionSupportRequired
            | ProtocolError::TaskNotCancelable => 400,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            ProtocolError::InvalidRequest => "INVALID_REQUEST",
            ProtocolError::InvalidParams => "INVALID_PARAMS",
            ProtocolError::UnsupportedOperation => "UNSUPPORTED_OPERATION",
            ProtocolError::ContentTypeNotSupported => "CONTENT_TYPE_NOT_SUPPORTED",
            ProtocolError::VersionNotSupported => "VERSION_NOT_SUPPORTED",
            ProtocolError::ExtensionSupportRequired => "EXTENSION_SUPPORT_REQUIRED",
            ProtocolError::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            ProtocolError::Unauthorized => "UNAUTHORIZED",
            ProtocolError::TaskNotFound => "TASK_NOT_FOUND",
            ProtocolError::TaskNotCancelable => "TASK_NOT_CANCELABLE",
        }
    }

    fn message(self) -> &'static str {
        match self {
            ProtocolError::InvalidRequest => "The A2A request is malformed.",
            ProtocolError::InvalidParams => "The A2A request is outside the supported subset.",
            ProtocolError::UnsupportedOperation => "The A2A operation is not supported.",
            ProtocolError::ContentTypeNotSupported => "The A2A media type is not supported.",
            ProtocolError::VersionNotSupported => "The A2A protocol version is not supported.",
            ProtocolError::ExtensionSupportRequired => {
                "A requested A2A extension is not supported."
            }
            ProtocolError::PayloadTooLarge => "The A2A request exceeds the configured limit.",
            ProtocolError::Unauthorized => "A2A peer authentication failed.",
            ProtocolError::TaskNotFound => "The A2A task was not found.",
            ProtocolError::TaskNotCancelable => "The A2A task is not cancelable.",
        }
    }
}

impl From<TaskProjectionState> for TaskState {
    fn from(state: TaskProjectionState) -> Self {
        match state {
            TaskProjectionState::Submitted => TaskState::Submitted,
            TaskProjectionState::Working => TaskState::Working,
            TaskProjectionState::Completed => TaskState::Completed,
            TaskProjectionState::Failed => TaskState::Failed,
            TaskProjectionState::Canceled => TaskState::Canceled,
            TaskProjectionState::Rejected => TaskState::Rejected,
        }
    }
}

fn validate_len(value: &str, min: usize, max: usize) -> Result<(), ProtocolError> {
    let length = value.chars().count();
    if length < min || length > max {
        return Err(ProtocolError::InvalidParams);
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty() {
        return Err(ProtocolError::InvalidParams);
    }
    if value.chars().count() > MAX_TEXT_CODE_POINTS || value.len() > MAX_TEXT_UTF8_BYTES {
        return Err(ProtocolError::PayloadTooLarge);
    }
    Ok(())
}

fn validate_task_id(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty() || value.len() > MAX_TASK_ID_BYTES {
        return Err(ProtocolError::InvalidParams);
    }
    Ok(())
}

fn validate_raw_send_message_shape(body: &[u8]) -> Result<(), ProtocolError> {
    let raw: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| ProtocolError::InvalidRequest)?;
    let root = raw.as_object().ok_or(ProtocolError::InvalidRequest)?;
    reject_unknown_fields(root, &["message", "configuration", "metadata", "tenant"])?;
    let message = root
        .get("message")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProtocolError::InvalidParams)?;
    reject_unknown_fields(
        message,
        &[
            "messageId",
            "contextId",
            "taskId",
            "role",
            "parts",
            "metadata",
            "extensions",
            "referenceTaskIds",
        ],
    )?;
    let configuration = root
        .get("configuration")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProtocolError::InvalidParams)?;
    reject_unknown_fields(
        configuration,
        &[
            "acceptedOutputModes",
            "taskPushNotificationConfig",
            "historyLength",
            "returnImmediately",
        ],
    )?;
    let parts = message
        .get("parts")
        .and_then(serde_json::Value::as_array)
        .ok_or(ProtocolError::InvalidParams)?;
    for part in parts {
        let object = part.as_object().ok_or(ProtocolError::InvalidParams)?;
        let text_fields = object.contains_key("text") as u8;
        let unsupported_fields = ["raw", "url", "file", "data"]
            .into_iter()
            .filter(|field| object.contains_key(*field))
            .count() as u8;
        if text_fields != 1 || unsupported_fields != 0 {
            return Err(ProtocolError::InvalidParams);
        }
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "text" | "mediaType"))
        {
            return Err(ProtocolError::InvalidParams);
        }
    }
    Ok(())
}

fn reject_unknown_fields(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
) -> Result<(), ProtocolError> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ProtocolError::InvalidParams);
    }
    Ok(())
}

fn derived_id(base: &str, suffix: &str) -> String {
    let digest = Sha256::digest([base.as_bytes(), b"\0", suffix.as_bytes()].concat());
    let mut id = String::with_capacity(suffix.len() + 1 + digest.len() * 2);
    id.push_str(suffix);
    id.push('-');
    for byte in digest {
        write!(&mut id, "{byte:02x}").expect("writing to a String cannot fail");
    }
    id
}
