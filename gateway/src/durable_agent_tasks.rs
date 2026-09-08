//! Durable, versioned root-agent tasks produced by the local gateway.

use gateway_bus::{
    DurableWorkQueue, ValidatedWorkCommand, WorkHandler, WorkHandlerAuthorizationError,
    WorkHandlerContext, WorkHandlerOutcome, WorkHandlerPayloadError, WorkTransport,
};
use gateway_services::validate_configured_agent_id;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

pub const AGENT_TASK_KIND: &str = "agent.task.v1";
pub const AGENT_TASK_TARGET: &str = "zbot.local";
pub const AGENT_TASK_SOURCE: &str = "gateway.research";
pub const MAX_CONVERSATION_ID_BYTES: usize = 128;
pub const MAX_MESSAGE_BYTES: usize = 60_000;
pub const MAX_AGENT_TASK_PAYLOAD_BYTES: usize = execution_state::MAX_PAYLOAD_BYTES;
const READY_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HANDLER_OWNERSHIP: Duration = Duration::from_secs(55 * 60);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTaskMode {
    Research,
}

impl AgentTaskMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Research => "research",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskV1 {
    pub agent_id: String,
    pub conversation_id: String,
    pub message: String,
    pub mode: AgentTaskMode,
    pub session_id: String,
    pub execution_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskValidationError {
    InvalidAgent,
    InvalidConversation,
    InvalidMessage,
    InvalidSessionId,
    InvalidExecutionId,
    InvalidMessageId,
    PayloadTooLarge,
}

impl AgentTaskV1 {
    pub fn validate(&self) -> Result<(), AgentTaskValidationError> {
        validate_invocation_agent_id(&self.agent_id)?;
        validate_bounded_identifier(&self.conversation_id, MAX_CONVERSATION_ID_BYTES)
            .map_err(|_| AgentTaskValidationError::InvalidConversation)?;
        if self.message.is_empty() || self.message.len() > MAX_MESSAGE_BYTES {
            return Err(AgentTaskValidationError::InvalidMessage);
        }
        validate_prefixed_uuid(&self.session_id, "sess-")
            .map_err(|_| AgentTaskValidationError::InvalidSessionId)?;
        validate_prefixed_uuid(&self.execution_id, "exec-")
            .map_err(|_| AgentTaskValidationError::InvalidExecutionId)?;
        validate_prefixed_uuid(&self.message_id, "msg-")
            .map_err(|_| AgentTaskValidationError::InvalidMessageId)?;
        let encoded =
            serde_json::to_vec(self).map_err(|_| AgentTaskValidationError::PayloadTooLarge)?;
        if encoded.len() > MAX_AGENT_TASK_PAYLOAD_BYTES {
            return Err(AgentTaskValidationError::PayloadTooLarge);
        }
        Ok(())
    }
}

pub fn validate_invocation_agent_id(value: &str) -> Result<(), AgentTaskValidationError> {
    if value == "root" {
        return Ok(());
    }
    validate_configured_agent_id(value).map_err(|_| AgentTaskValidationError::InvalidAgent)
}

pub fn canonical_message_id(candidate: Option<&str>) -> String {
    candidate
        .filter(|value| validate_prefixed_uuid(value, "msg-").is_ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("msg-{}", uuid::Uuid::new_v4()))
}

pub fn reserved_session_id(message_id: &str) -> String {
    format!(
        "sess-{}",
        derived_task_uuid("zbot.agent-task.session.v1", message_id)
    )
}

pub fn reserved_execution_id(message_id: &str) -> String {
    format!(
        "exec-{}",
        derived_task_uuid("zbot.agent-task.execution.v1", message_id)
    )
}

fn derived_task_uuid(domain: &str, message_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(message_id.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // Mark the opaque hash as an RFC 4122 variant/version UUID. Separate
    // domains make session and execution identities independent, while the
    // one-way mapping prevents a client UUID from naming an existing row.
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).hyphenated().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchEnqueueReceipt {
    pub work_id: String,
    pub actor_id: String,
    pub session_id: String,
    pub execution_id: String,
    pub message_id: String,
    pub inserted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskEnqueueError {
    Invalid,
    Unauthorized,
    Unavailable,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskCancelOutcome {
    Canceled,
    AlreadyCanceled,
    NotFound,
    NotCancelable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskCancelError {
    Unauthorized,
    Unavailable,
}

struct ResearchTaskPolicy {
    task: AgentTaskV1,
    node_id: String,
    actor_id: String,
}

impl execution_state::WorkPolicy for ResearchTaskPolicy {
    fn authorize(
        &self,
        draft: &execution_state::WorkDraft,
    ) -> Result<execution_state::WorkAuthorization, execution_state::WorkPolicyError> {
        if draft.kind() != AGENT_TASK_KIND {
            return Err(execution_state::WorkPolicyError::KindNotAllowed);
        }
        if draft.target() != AGENT_TASK_TARGET {
            return Err(execution_state::WorkPolicyError::TargetNotAllowed);
        }
        let expected = serde_json::to_value(&self.task)
            .map_err(|_| execution_state::WorkPolicyError::PayloadInvalid)?;
        if draft.payload() != &expected {
            return Err(execution_state::WorkPolicyError::PayloadInvalid);
        }
        Ok(execution_state::WorkAuthorization::new(
            AGENT_TASK_SOURCE,
            &self.node_id,
            &self.actor_id,
            &self.task.session_id,
            &self.task.execution_id,
        ))
    }
}

pub struct DurableAgentTaskService {
    store: Arc<dyn execution_state::WorkStore>,
    transport: Arc<dyn WorkTransport>,
    state: Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
    agents: Arc<gateway_services::AgentService>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    node_id: String,
}

impl DurableAgentTaskService {
    pub fn new(
        store: Arc<dyn execution_state::WorkStore>,
        transport: Arc<dyn WorkTransport>,
        state: Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
        agents: Arc<gateway_services::AgentService>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        node_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            transport,
            state,
            agents,
            messages,
            node_id: node_id.into(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue_research(
        &self,
        actor_id: &str,
        agent_id: &str,
        conversation_id: &str,
        message: &str,
        existing_session_id: Option<&str>,
        client_message_id: Option<&str>,
    ) -> Result<ResearchEnqueueReceipt, AgentTaskEnqueueError> {
        validate_bounded_identifier(actor_id, execution_state::MAX_ROUTING_BYTES)
            .map_err(|_| AgentTaskEnqueueError::Unauthorized)?;
        validate_invocation_agent_id(agent_id).map_err(|_| AgentTaskEnqueueError::Invalid)?;
        if agent_id != "root" {
            self.agents
                .get(agent_id)
                .await
                .map_err(|_| AgentTaskEnqueueError::Invalid)?;
        }

        let message_id = canonical_message_id(client_message_id);
        let (session_id, execution_id, require_existing_message) = if let Some(session_id) =
            existing_session_id
        {
            let session = self
                .state
                .get_session(session_id)
                .map_err(|_| AgentTaskEnqueueError::Unavailable)?
                .ok_or(AgentTaskEnqueueError::Unauthorized)?;
            if session.root_agent_id != agent_id {
                return Err(AgentTaskEnqueueError::Unauthorized);
            }
            let execution = self
                .state
                .get_root_execution(session_id)
                .map_err(|_| AgentTaskEnqueueError::Unavailable)?
                .ok_or(AgentTaskEnqueueError::Unauthorized)?;
            if execution.agent_id != agent_id {
                return Err(AgentTaskEnqueueError::Unauthorized);
            }
            (session_id.to_owned(), execution.id, false)
        } else {
            let session_id = reserved_session_id(&message_id);
            let execution_id = reserved_execution_id(&message_id);
            let existing_session = self
                .state
                .get_session(&session_id)
                .map_err(|_| AgentTaskEnqueueError::Unavailable)?;
            let existing_execution = self
                .state
                .get_execution(&execution_id)
                .map_err(|_| AgentTaskEnqueueError::Unavailable)?;
            let collision = match (existing_session.as_ref(), existing_execution.as_ref()) {
                (None, None) => false,
                (Some(session), Some(execution))
                    if session.root_agent_id == agent_id
                        && execution.id == execution_id
                        && execution.session_id == session_id
                        && execution.agent_id == agent_id
                        && execution.parent_execution_id.is_none()
                        && execution.delegation_type == execution_state::DelegationType::Root =>
                {
                    true
                }
                _ => return Err(AgentTaskEnqueueError::Conflict),
            };
            (session_id, execution_id, collision)
        };
        let task = AgentTaskV1 {
            agent_id: agent_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            message: message.to_owned(),
            mode: AgentTaskMode::Research,
            session_id: session_id.clone(),
            execution_id: execution_id.clone(),
            message_id: message_id.clone(),
        };
        task.validate()
            .map_err(|_| AgentTaskEnqueueError::Invalid)?;
        match self
            .messages
            .get(&message_id)
            .map_err(|_| AgentTaskEnqueueError::Unavailable)?
        {
            Some(existing)
                if existing.session_id != session_id
                    || existing.execution_id.as_deref() != Some(execution_id.as_str())
                    || existing.role != "user"
                    || existing.content != message =>
            {
                return Err(AgentTaskEnqueueError::Conflict)
            }
            Some(_) => {}
            None if require_existing_message => return Err(AgentTaskEnqueueError::Conflict),
            None => {}
        }
        let payload = serde_json::to_value(&task).map_err(|_| AgentTaskEnqueueError::Invalid)?;
        let draft = execution_state::WorkDraft::new(AGENT_TASK_KIND, AGENT_TASK_TARGET, payload)
            .with_correlation_id(conversation_id)
            .with_dedupe_key(&message_id);
        let policy = Arc::new(ResearchTaskPolicy {
            task: task.clone(),
            node_id: self.node_id.clone(),
            actor_id: actor_id.to_owned(),
        });
        let queue = DurableWorkQueue::new(self.store.clone(), policy, self.transport.clone());
        let receipt = queue
            .enqueue(draft, chrono::Utc::now())
            .await
            .map_err(|_| AgentTaskEnqueueError::Unavailable)?;

        let stored = self
            .store
            .get(receipt.work_id())
            .map_err(|_| AgentTaskEnqueueError::Unavailable)?
            .ok_or(AgentTaskEnqueueError::Unavailable)?;
        let envelope = stored.envelope();
        if envelope.kind() != AGENT_TASK_KIND
            || envelope.target() != AGENT_TASK_TARGET
            || envelope.source() != AGENT_TASK_SOURCE
            || envelope.correlation_id() != Some(conversation_id)
            || envelope.dedupe_key() != Some(message_id.as_str())
            || envelope.provenance().node_id() != self.node_id
            || envelope.provenance().actor_id() != actor_id
            || envelope.provenance().session_id() != session_id
            || envelope.provenance().execution_id() != execution_id
            || envelope.payload() != &serde_json::to_value(&task).unwrap_or_default()
        {
            return Err(AgentTaskEnqueueError::Conflict);
        }

        tracing::info!(
            work_id = receipt.work_id(),
            session_id,
            execution_id,
            message_id,
            inserted = receipt.inserted(),
            event = if receipt.inserted() {
                "agent_task_enqueued"
            } else {
                "agent_task_deduplicated"
            },
            "Durable agent task accepted"
        );

        Ok(ResearchEnqueueReceipt {
            work_id: receipt.work_id().to_owned(),
            actor_id: actor_id.to_owned(),
            session_id,
            execution_id,
            message_id,
            inserted: receipt.inserted(),
        })
    }

    pub fn work_store(&self) -> Arc<dyn execution_state::WorkStore> {
        self.store.clone()
    }

    /// Cancel the durable Research request owned by this WebSocket actor.
    ///
    /// The read-before-transition check binds both the request's conversation
    /// and reserved session identity, so a reused conversation cannot cancel a
    /// different request from the same client.
    pub fn cancel_research(
        &self,
        actor_id: &str,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<AgentTaskCancelOutcome, AgentTaskCancelError> {
        validate_bounded_identifier(actor_id, execution_state::MAX_ROUTING_BYTES)
            .map_err(|_| AgentTaskCancelError::Unauthorized)?;
        validate_bounded_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES)
            .map_err(|_| AgentTaskCancelError::Unauthorized)?;
        validate_prefixed_uuid(session_id, "sess-")
            .map_err(|_| AgentTaskCancelError::Unauthorized)?;
        let scope = execution_state::WorkScope::new(
            AGENT_TASK_SOURCE,
            AGENT_TASK_KIND,
            actor_id.to_owned(),
        )
        .map_err(|_| AgentTaskCancelError::Unauthorized)?;
        let Some(work) = self
            .store
            .find_scoped(&scope, conversation_id)
            .map_err(|_| AgentTaskCancelError::Unavailable)?
        else {
            return Ok(AgentTaskCancelOutcome::NotFound);
        };
        let envelope = work.envelope();
        if envelope.target() != AGENT_TASK_TARGET
            || envelope.provenance().session_id() != session_id
            || envelope.correlation_id() != Some(conversation_id)
        {
            return Err(AgentTaskCancelError::Unauthorized);
        }
        match self
            .store
            .cancel_scoped(&scope, conversation_id, chrono::Utc::now())
            .map_err(|_| AgentTaskCancelError::Unavailable)?
        {
            execution_state::WorkCancelOutcome::Canceled(_) => Ok(AgentTaskCancelOutcome::Canceled),
            execution_state::WorkCancelOutcome::AlreadyCanceled(_) => {
                Ok(AgentTaskCancelOutcome::AlreadyCanceled)
            }
            execution_state::WorkCancelOutcome::NotCancelable(_) => {
                Ok(AgentTaskCancelOutcome::NotCancelable)
            }
            execution_state::WorkCancelOutcome::NotFound => Ok(AgentTaskCancelOutcome::NotFound),
        }
    }

    pub async fn wait_until_ready(
        &self,
        receipt: &ResearchEnqueueReceipt,
    ) -> Result<(), AgentTaskEnqueueError> {
        let stored = self
            .store
            .get(&receipt.work_id)
            .map_err(|_| AgentTaskEnqueueError::Unavailable)?
            .ok_or(AgentTaskEnqueueError::Unavailable)?;
        let envelope = stored.envelope();
        let task: AgentTaskV1 = serde_json::from_value(envelope.payload().clone())
            .map_err(|_| AgentTaskEnqueueError::Conflict)?;
        task.validate()
            .map_err(|_| AgentTaskEnqueueError::Conflict)?;
        if task.session_id != receipt.session_id
            || task.execution_id != receipt.execution_id
            || task.message_id != receipt.message_id
            || envelope.kind() != AGENT_TASK_KIND
            || envelope.target() != AGENT_TASK_TARGET
            || envelope.source() != AGENT_TASK_SOURCE
            || envelope.correlation_id() != Some(task.conversation_id.as_str())
            || envelope.dedupe_key() != Some(task.message_id.as_str())
            || envelope.provenance().node_id() != self.node_id
            || envelope.provenance().actor_id() != receipt.actor_id
            || envelope.provenance().session_id() != task.session_id
            || envelope.provenance().execution_id() != task.execution_id
        {
            return Err(AgentTaskEnqueueError::Conflict);
        }
        let deadline = tokio::time::Instant::now() + READY_WAIT_TIMEOUT;
        loop {
            let work = self
                .store
                .get(&receipt.work_id)
                .map_err(|_| AgentTaskEnqueueError::Unavailable)?
                .ok_or(AgentTaskEnqueueError::Unavailable)?;
            if work.status() == execution_state::WorkStatus::DeadLetter {
                return Err(AgentTaskEnqueueError::Unavailable);
            }
            match self.messages.get(&receipt.message_id) {
                Ok(Some(message)) => {
                    let session = self
                        .state
                        .get_session(&receipt.session_id)
                        .map_err(|_| AgentTaskEnqueueError::Unavailable)?
                        .ok_or(AgentTaskEnqueueError::Conflict)?;
                    let execution = self
                        .state
                        .get_root_execution(&receipt.session_id)
                        .map_err(|_| AgentTaskEnqueueError::Unavailable)?
                        .ok_or(AgentTaskEnqueueError::Conflict)?;
                    if message.session_id != receipt.session_id
                        || message.execution_id.as_deref() != Some(receipt.execution_id.as_str())
                        || message.role != "user"
                        || message.content != task.message
                        || execution.id != receipt.execution_id
                        || execution.agent_id != task.agent_id
                        || session.root_agent_id != task.agent_id
                    {
                        return Err(AgentTaskEnqueueError::Conflict);
                    }
                    return Ok(());
                }
                Ok(None) => {}
                Err(_) => return Err(AgentTaskEnqueueError::Unavailable),
            }
            if work.status() == execution_state::WorkStatus::Completed {
                return Err(AgentTaskEnqueueError::Unavailable);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AgentTaskEnqueueError::Unavailable);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn validate_bounded_identifier(value: &str, max_bytes: usize) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.chars().any(|character| character.is_ascii_control())
    {
        return Err(());
    }
    Ok(())
}

fn validate_prefixed_uuid(value: &str, prefix: &str) -> Result<(), ()> {
    let suffix = value.strip_prefix(prefix).ok_or(())?;
    let parsed = uuid::Uuid::parse_str(suffix).map_err(|_| ())?;
    if parsed.hyphenated().to_string() != suffix {
        return Err(());
    }
    Ok(())
}

impl fmt::Debug for AgentTaskV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentTaskV1")
            .field("agent_id", &self.agent_id)
            .field("conversation_id", &self.conversation_id)
            .field("message", &"[REDACTED]")
            .field("mode", &self.mode.as_str())
            .field("session_id", &self.session_id)
            .field("execution_id", &self.execution_id)
            .field("message_id", &self.message_id)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskRunState {
    Initial,
    Running,
    Resume,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskRuntimeError {
    Retryable,
    Integrity,
}

#[async_trait::async_trait]
pub trait AgentTaskRuntime: Send + Sync {
    async fn inspect(&self, task: &AgentTaskV1)
        -> Result<AgentTaskRunState, AgentTaskRuntimeError>;
    async fn start(&self, task: &AgentTaskV1, actor_id: &str) -> Result<(), AgentTaskRuntimeError>;
    async fn resume(&self, task: &AgentTaskV1, actor_id: &str)
        -> Result<(), AgentTaskRuntimeError>;
}

pub struct GatewayAgentTaskRuntime {
    runtime: Arc<crate::services::RuntimeService>,
    state: Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    agents: Arc<gateway_services::AgentService>,
}

impl GatewayAgentTaskRuntime {
    pub fn new(
        runtime: Arc<crate::services::RuntimeService>,
        state: Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        agents: Arc<gateway_services::AgentService>,
    ) -> Self {
        Self {
            runtime,
            state,
            messages,
            agents,
        }
    }

    async fn validate_agent(&self, agent_id: &str) -> Result<(), AgentTaskRuntimeError> {
        validate_invocation_agent_id(agent_id)?;
        if agent_id != "root" {
            self.agents
                .get(agent_id)
                .await
                .map_err(|_| AgentTaskRuntimeError::Integrity)?;
        }
        Ok(())
    }

    fn ensure_initial_state(&self, task: &AgentTaskV1) -> Result<(), AgentTaskRuntimeError> {
        match self.state.get_session(&task.session_id) {
            Ok(Some(session)) => {
                if session.root_agent_id != task.agent_id {
                    return Err(AgentTaskRuntimeError::Integrity);
                }
            }
            Ok(None) => {
                let session = execution_state::Session::new_with_id(
                    &task.session_id,
                    &task.agent_id,
                    execution_state::TriggerSource::Web,
                )
                .map_err(|_| AgentTaskRuntimeError::Integrity)?;
                self.state
                    .create_session_from(&session)
                    .map_err(|_| AgentTaskRuntimeError::Retryable)?;
            }
            Err(_) => return Err(AgentTaskRuntimeError::Retryable),
        }
        match self.state.get_root_execution(&task.session_id) {
            Ok(Some(execution)) => {
                if execution.id != task.execution_id || execution.agent_id != task.agent_id {
                    return Err(AgentTaskRuntimeError::Integrity);
                }
            }
            Ok(None) => {
                if self
                    .state
                    .get_execution(&task.execution_id)
                    .map_err(|_| AgentTaskRuntimeError::Retryable)?
                    .is_some()
                {
                    return Err(AgentTaskRuntimeError::Integrity);
                }
                let execution = execution_state::AgentExecution::new_root_with_id(
                    &task.execution_id,
                    &task.session_id,
                    &task.agent_id,
                )
                .map_err(|_| AgentTaskRuntimeError::Integrity)?;
                self.state
                    .create_execution(&execution)
                    .map_err(|_| AgentTaskRuntimeError::Retryable)?;
            }
            Err(_) => return Err(AgentTaskRuntimeError::Retryable),
        }
        Ok(())
    }

    fn exact_message(&self, task: &AgentTaskV1) -> Result<bool, AgentTaskRuntimeError> {
        let message = self
            .messages
            .get(&task.message_id)
            .map_err(|_| AgentTaskRuntimeError::Retryable)?;
        let Some(message) = message else {
            return Ok(false);
        };
        if message.session_id != task.session_id
            || message.execution_id.as_deref() != Some(task.execution_id.as_str())
            || message.role != "user"
            || message.content != task.message
        {
            return Err(AgentTaskRuntimeError::Integrity);
        }
        Ok(true)
    }
}

#[async_trait::async_trait]
impl AgentTaskRuntime for GatewayAgentTaskRuntime {
    async fn inspect(
        &self,
        task: &AgentTaskV1,
    ) -> Result<AgentTaskRunState, AgentTaskRuntimeError> {
        self.validate_agent(&task.agent_id).await?;
        let session = self
            .state
            .get_session(&task.session_id)
            .map_err(|_| AgentTaskRuntimeError::Retryable)?;
        let Some(session) = session else {
            return if self.exact_message(task)? {
                Err(AgentTaskRuntimeError::Integrity)
            } else {
                Ok(AgentTaskRunState::Initial)
            };
        };
        if session.root_agent_id != task.agent_id {
            return Err(AgentTaskRuntimeError::Integrity);
        }
        let execution = self
            .state
            .get_root_execution(&task.session_id)
            .map_err(|_| AgentTaskRuntimeError::Retryable)?;
        let Some(execution) = execution else {
            return if self.exact_message(task)? {
                Err(AgentTaskRuntimeError::Integrity)
            } else {
                Ok(AgentTaskRunState::Initial)
            };
        };
        if execution.id != task.execution_id || execution.agent_id != task.agent_id {
            return Err(AgentTaskRuntimeError::Integrity);
        }
        if !self.exact_message(task)? {
            return Ok(AgentTaskRunState::Initial);
        }
        if execution.status == execution_state::ExecutionStatus::Cancelled {
            return Ok(AgentTaskRunState::Cancelled);
        }
        if session.status == execution_state::SessionStatus::Completed
            && execution.status == execution_state::ExecutionStatus::Completed
        {
            return Ok(AgentTaskRunState::Completed);
        }
        if session.status == execution_state::SessionStatus::Running
            && execution.status == execution_state::ExecutionStatus::Running
        {
            return Ok(AgentTaskRunState::Running);
        }
        Ok(AgentTaskRunState::Resume)
    }

    async fn start(&self, task: &AgentTaskV1, actor_id: &str) -> Result<(), AgentTaskRuntimeError> {
        self.ensure_initial_state(task)?;
        self.runtime
            .invoke_durable_with_hook_and_callback(
                &task.agent_id,
                &task.conversation_id,
                &task.message,
                crate::hooks::HookContext::web(actor_id),
                Some(task.session_id.clone()),
                None,
                Some(task.mode.as_str().to_owned()),
                Some(task.message_id.clone()),
            )
            .await
            .map(|_| ())
            .map_err(|_| AgentTaskRuntimeError::Retryable)
    }

    async fn resume(
        &self,
        task: &AgentTaskV1,
        actor_id: &str,
    ) -> Result<(), AgentTaskRuntimeError> {
        self.runtime
            .invoke_persisted_with_hook_and_callback(
                &task.agent_id,
                &task.conversation_id,
                &task.message,
                crate::hooks::HookContext::web(actor_id),
                task.session_id.clone(),
                task.execution_id.clone(),
                task.message_id.clone(),
                None,
                Some(task.mode.as_str().to_owned()),
            )
            .await
            .map(|_| ())
            .map_err(|error| {
                if matches!(
                    error.as_str(),
                    "durable_resume_session_missing"
                        | "durable_resume_execution_missing"
                        | "durable_resume_message_missing"
                        | "durable_resume_identity_mismatch"
                        | "durable_resume_message_mismatch"
                ) {
                    AgentTaskRuntimeError::Integrity
                } else {
                    AgentTaskRuntimeError::Retryable
                }
            })
    }
}

impl From<AgentTaskValidationError> for AgentTaskRuntimeError {
    fn from(_: AgentTaskValidationError) -> Self {
        Self::Integrity
    }
}

pub struct AgentTaskHandler {
    runtime: Arc<dyn AgentTaskRuntime>,
    node_id: String,
    poll_interval: Duration,
    max_ownership: Duration,
}

impl AgentTaskHandler {
    pub fn new(runtime: Arc<dyn AgentTaskRuntime>, node_id: impl Into<String>) -> Self {
        Self {
            runtime,
            node_id: node_id.into(),
            poll_interval: Duration::from_millis(250),
            max_ownership: MAX_HANDLER_OWNERSHIP,
        }
    }

    #[cfg(test)]
    fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    #[cfg(test)]
    fn with_max_ownership(mut self, max_ownership: Duration) -> Self {
        self.max_ownership = max_ownership;
        self
    }

    fn runtime_outcome(error: AgentTaskRuntimeError) -> WorkHandlerOutcome {
        match error {
            AgentTaskRuntimeError::Retryable => {
                WorkHandlerOutcome::Retry(execution_state::WorkFailureCode::Internal)
            }
            AgentTaskRuntimeError::Integrity => {
                WorkHandlerOutcome::Permanent(execution_state::WorkFailureCode::IntegrityViolation)
            }
        }
    }

    async fn handle_task(&self, task: AgentTaskV1, actor_id: String) -> WorkHandlerOutcome {
        let deadline = tokio::time::Instant::now() + self.max_ownership;
        let state = match self.runtime.inspect(&task).await {
            Ok(state) => state,
            Err(error) => return Self::runtime_outcome(error),
        };
        let launch = match state {
            AgentTaskRunState::Initial => self.runtime.start(&task, &actor_id).await,
            AgentTaskRunState::Resume => self.runtime.resume(&task, &actor_id).await,
            AgentTaskRunState::Running => Ok(()),
            AgentTaskRunState::Completed | AgentTaskRunState::Cancelled => {
                return WorkHandlerOutcome::Complete
            }
        };
        if let Err(error) = launch {
            return Self::runtime_outcome(error);
        }

        loop {
            if tokio::time::Instant::now() >= deadline {
                return WorkHandlerOutcome::Retry(execution_state::WorkFailureCode::Internal);
            }
            tokio::time::sleep(self.poll_interval).await;
            match self.runtime.inspect(&task).await {
                Ok(AgentTaskRunState::Completed | AgentTaskRunState::Cancelled) => {
                    return WorkHandlerOutcome::Complete
                }
                Ok(AgentTaskRunState::Running) => {}
                Ok(AgentTaskRunState::Initial | AgentTaskRunState::Resume) => {
                    return WorkHandlerOutcome::Retry(execution_state::WorkFailureCode::Internal)
                }
                Err(error) => return Self::runtime_outcome(error),
            }
        }
    }

    async fn settle_task(
        &self,
        work_id: &str,
        task: AgentTaskV1,
        actor_id: String,
    ) -> WorkHandlerOutcome {
        let session_id = task.session_id.clone();
        let execution_id = task.execution_id.clone();
        tracing::info!(
            work_id,
            session_id,
            execution_id,
            event = "agent_task_dispatch",
            "Dispatching durable agent task"
        );
        let outcome = self.handle_task(task, actor_id).await;
        let event = match outcome {
            WorkHandlerOutcome::Complete => "agent_task_completed",
            WorkHandlerOutcome::Retry(_) => "agent_task_retry",
            WorkHandlerOutcome::Permanent(_) => "agent_task_rejected",
        };
        tracing::info!(
            work_id,
            session_id,
            execution_id,
            event,
            "Durable agent task handler settled"
        );
        outcome
    }
}

#[async_trait::async_trait]
impl WorkHandler for AgentTaskHandler {
    fn target(&self) -> &'static str {
        AGENT_TASK_TARGET
    }

    fn kind(&self) -> &'static str {
        AGENT_TASK_KIND
    }

    fn validate_payload(
        &self,
        payload: &serde_json::Value,
    ) -> Result<ValidatedWorkCommand, WorkHandlerPayloadError> {
        let task: AgentTaskV1 = serde_json::from_value(payload.clone())
            .map_err(|_| WorkHandlerPayloadError::Invalid)?;
        task.validate()
            .map_err(|_| WorkHandlerPayloadError::Invalid)?;
        Ok(ValidatedWorkCommand::new(task))
    }

    fn authorize(
        &self,
        context: &WorkHandlerContext,
        command: &ValidatedWorkCommand,
    ) -> Result<(), WorkHandlerAuthorizationError> {
        let task = command
            .downcast_ref::<AgentTaskV1>()
            .ok_or(WorkHandlerAuthorizationError::Rejected)?;
        let provenance = context.provenance();
        if context.source() != AGENT_TASK_SOURCE
            || context.correlation_id() != Some(task.conversation_id.as_str())
            || provenance.node_id() != self.node_id
            || validate_bounded_identifier(
                provenance.actor_id(),
                execution_state::MAX_ROUTING_BYTES,
            )
            .is_err()
            || provenance.session_id() != task.session_id
            || provenance.execution_id() != task.execution_id
        {
            return Err(WorkHandlerAuthorizationError::Rejected);
        }
        Ok(())
    }

    async fn handle(
        &self,
        context: WorkHandlerContext,
        command: ValidatedWorkCommand,
    ) -> WorkHandlerOutcome {
        let task = match command.downcast::<AgentTaskV1>() {
            Ok(task) => task,
            Err(_) => {
                return WorkHandlerOutcome::Permanent(
                    execution_state::WorkFailureCode::IntegrityViolation,
                )
            }
        };
        self.settle_task(
            context.work_id(),
            task,
            context.provenance().actor_id().to_owned(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    struct CapturedLogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedLogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedLogs {
        type Writer = CapturedLogWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedLogWriter(self.0.clone())
        }
    }

    fn valid_task() -> AgentTaskV1 {
        AgentTaskV1 {
            agent_id: "root".to_owned(),
            conversation_id: "research-1".to_owned(),
            message: "Investigate durable queues".to_owned(),
            mode: AgentTaskMode::Research,
            session_id: "sess-550e8400-e29b-41d4-a716-446655440000".to_owned(),
            execution_id: "exec-550e8400-e29b-41d4-a716-446655440001".to_owned(),
            message_id: "msg-550e8400-e29b-41d4-a716-446655440002".to_owned(),
        }
    }

    #[test]
    fn agent_task_contract_accepts_exact_valid_shape() {
        let task = valid_task();
        assert_eq!(task.validate(), Ok(()));
        let encoded = serde_json::to_value(&task).unwrap();
        assert_eq!(encoded["mode"], "research");
    }

    #[test]
    fn agent_task_contract_rejects_unknown_fields_and_modes() {
        let mut encoded = serde_json::to_value(valid_task()).unwrap();
        encoded["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentTaskV1>(encoded).is_err());

        let mut encoded = serde_json::to_value(valid_task()).unwrap();
        encoded["mode"] = serde_json::json!("chat");
        assert!(serde_json::from_value::<AgentTaskV1>(encoded).is_err());
    }

    #[test]
    fn agent_task_contract_enforces_field_boundaries() {
        let mut task = valid_task();
        task.conversation_id = "a".repeat(MAX_CONVERSATION_ID_BYTES);
        assert_eq!(task.validate(), Ok(()));
        task.conversation_id.push('a');
        assert_eq!(
            task.validate(),
            Err(AgentTaskValidationError::InvalidConversation)
        );
        task = valid_task();
        task.conversation_id = "bad\nconversation".to_owned();
        assert_eq!(
            task.validate(),
            Err(AgentTaskValidationError::InvalidConversation)
        );
        task = valid_task();
        task.message = "m".repeat(MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            task.validate(),
            Err(AgentTaskValidationError::InvalidMessage)
        );
    }

    #[test]
    fn agent_task_contract_rejects_malformed_or_noncanonical_ids() {
        let mut task = valid_task();
        task.session_id = "sess-not-a-uuid".to_owned();
        assert_eq!(
            task.validate(),
            Err(AgentTaskValidationError::InvalidSessionId)
        );
        task = valid_task();
        task.execution_id = task.execution_id.to_uppercase();
        assert_eq!(
            task.validate(),
            Err(AgentTaskValidationError::InvalidExecutionId)
        );
    }

    #[test]
    fn invocation_agent_id_reuses_configured_agent_rules() {
        assert_eq!(validate_invocation_agent_id("root"), Ok(()));
        assert_eq!(validate_invocation_agent_id("research-agent"), Ok(()));
        for invalid in ["orchestrator", "ward:foo", "../x", "-agent", "agent-"] {
            assert_eq!(
                validate_invocation_agent_id(invalid),
                Err(AgentTaskValidationError::InvalidAgent)
            );
        }
    }

    #[test]
    fn canonical_message_id_preserves_valid_client_id_and_replaces_invalid() {
        let valid = "msg-550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(canonical_message_id(Some(valid)), valid);
        let minted = canonical_message_id(Some("not-valid"));
        assert!(validate_prefixed_uuid(&minted, "msg-").is_ok());
        assert_ne!(minted, "not-valid");
    }

    #[test]
    fn agent_task_debug_redacts_prompt() {
        let rendered = format!("{:?}", valid_task());
        assert!(!rendered.contains("Investigate durable queues"));
        assert!(rendered.contains("[REDACTED]"));
    }

    struct FakeRuntime {
        states: Mutex<VecDeque<Result<AgentTaskRunState, AgentTaskRuntimeError>>>,
        starts: AtomicUsize,
        resumes: AtomicUsize,
    }

    struct TestPolicy;

    impl execution_state::WorkPolicy for TestPolicy {
        fn authorize(
            &self,
            _draft: &execution_state::WorkDraft,
        ) -> Result<execution_state::WorkAuthorization, execution_state::WorkPolicyError> {
            Ok(execution_state::WorkAuthorization::new(
                AGENT_TASK_SOURCE,
                "node-local",
                "connection-1",
                "sess-550e8400-e29b-41d4-a716-446655440000",
                "exec-550e8400-e29b-41d4-a716-446655440001",
            ))
        }
    }

    fn authorized_envelope(
        payload: serde_json::Value,
        correlation_id: &str,
    ) -> execution_state::WorkEnvelope {
        execution_state::WorkEnvelope::authorize(
            execution_state::WorkDraft::new(AGENT_TASK_KIND, AGENT_TASK_TARGET, payload)
                .with_correlation_id(correlation_id)
                .with_dedupe_key("msg-550e8400-e29b-41d4-a716-446655440002"),
            &TestPolicy,
            chrono::Utc::now(),
        )
        .unwrap()
    }

    #[test]
    fn handler_registry_denies_invalid_shape_or_correlation_before_dispatch() {
        let runtime = Arc::new(FakeRuntime::new([]));
        let handler: Arc<dyn WorkHandler> = Arc::new(AgentTaskHandler::new(runtime, "node-local"));
        let registry = gateway_bus::WorkHandlerRegistry::from_handlers(vec![handler]).unwrap();
        let payload = serde_json::to_value(valid_task()).unwrap();
        assert!(registry
            .authorized_handler(&authorized_envelope(payload.clone(), "research-1"))
            .is_ok());
        assert!(matches!(
            registry.authorized_handler(&authorized_envelope(payload, "other-conversation")),
            Err(gateway_bus::WorkDispatchRejection::HandlerRejected)
        ));

        let mut invalid = serde_json::to_value(valid_task()).unwrap();
        invalid["unexpected"] = serde_json::json!(true);
        assert!(matches!(
            registry.authorized_handler(&authorized_envelope(invalid, "research-1")),
            Err(gateway_bus::WorkDispatchRejection::InvalidPayload)
        ));
    }

    impl FakeRuntime {
        fn new(states: impl IntoIterator<Item = AgentTaskRunState>) -> Self {
            Self {
                states: Mutex::new(states.into_iter().map(Ok).collect()),
                starts: AtomicUsize::new(0),
                resumes: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl AgentTaskRuntime for FakeRuntime {
        async fn inspect(
            &self,
            _task: &AgentTaskV1,
        ) -> Result<AgentTaskRunState, AgentTaskRuntimeError> {
            self.states
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(AgentTaskRunState::Running))
        }

        async fn start(
            &self,
            _task: &AgentTaskV1,
            _actor_id: &str,
        ) -> Result<(), AgentTaskRuntimeError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn resume(
            &self,
            _task: &AgentTaskV1,
            _actor_id: &str,
        ) -> Result<(), AgentTaskRuntimeError> {
            self.resumes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn handler_starts_once_and_waits_for_terminal_completion() {
        let runtime = Arc::new(FakeRuntime::new([
            AgentTaskRunState::Initial,
            AgentTaskRunState::Running,
            AgentTaskRunState::Completed,
        ]));
        let handler = AgentTaskHandler::new(runtime.clone(), "node-local")
            .with_poll_interval(Duration::from_millis(1));

        assert_eq!(
            handler
                .handle_task(valid_task(), "connection-1".to_owned())
                .await,
            WorkHandlerOutcome::Complete
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.resumes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn handler_resumes_persisted_initial_bootstrap_once() {
        let runtime = Arc::new(FakeRuntime::new([
            AgentTaskRunState::Resume,
            AgentTaskRunState::Running,
            AgentTaskRunState::Completed,
        ]));
        let handler = AgentTaskHandler::new(runtime.clone(), "node-local")
            .with_poll_interval(Duration::from_millis(1));

        assert_eq!(
            handler
                .handle_task(valid_task(), "connection-1".to_owned())
                .await,
            WorkHandlerOutcome::Complete
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.resumes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn handler_monitors_live_execution_without_relaunching() {
        let runtime = Arc::new(FakeRuntime::new([
            AgentTaskRunState::Running,
            AgentTaskRunState::Completed,
        ]));
        let handler = AgentTaskHandler::new(runtime.clone(), "node-local")
            .with_poll_interval(Duration::from_millis(1));

        assert_eq!(
            handler
                .handle_task(valid_task(), "connection-1".to_owned())
                .await,
            WorkHandlerOutcome::Complete
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.resumes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn handler_relinquishes_ownership_before_outer_worker_timeout() {
        let runtime = Arc::new(FakeRuntime::new([AgentTaskRunState::Running]));
        let handler = AgentTaskHandler::new(runtime.clone(), "node-local")
            .with_poll_interval(Duration::from_millis(1))
            .with_max_ownership(Duration::ZERO);

        assert_eq!(
            handler
                .handle_task(valid_task(), "connection-1".to_owned())
                .await,
            WorkHandlerOutcome::Retry(execution_state::WorkFailureCode::Internal)
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.resumes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn operational_diagnostics_keep_prompt_and_provider_details_out_of_logs_and_failures() {
        const PROMPT_SENTINEL: &str = "PROMPT-SECRET-SENTINEL";
        const PROVIDER_SENTINEL: &str = "PROVIDER-RAW-DIAGNOSTIC-SENTINEL";
        let captured = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(CapturedLogs(captured.clone()))
            .finish();
        let runtime = Arc::new(FakeRuntime {
            states: Mutex::new(VecDeque::from([Err(AgentTaskRuntimeError::Retryable)])),
            starts: AtomicUsize::new(0),
            resumes: AtomicUsize::new(0),
        });
        let handler = AgentTaskHandler::new(runtime, "node-local");
        let mut task = valid_task();
        task.message = PROMPT_SENTINEL.to_owned();
        let outcome = tracing::subscriber::with_default(subscriber, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap()
                .block_on(handler.settle_task("work-safe-id", task, "connection-1".to_owned()))
        });

        let WorkHandlerOutcome::Retry(failure_code) = outcome else {
            panic!("retryable runtime failure must remain retryable");
        };
        let persisted_failure = serde_json::to_string(&failure_code).unwrap();
        let logs = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert_eq!(persisted_failure, "\"internal\"");
        assert!(logs.contains("agent_task_dispatch"));
        assert!(logs.contains("agent_task_retry"));
        for secret in [PROMPT_SENTINEL, PROVIDER_SENTINEL] {
            assert!(!logs.contains(secret));
            assert!(!persisted_failure.contains(secret));
        }
    }

    #[tokio::test]
    async fn session_stop_durable_cancellation_contract() {
        let runtime = Arc::new(FakeRuntime::new([AgentTaskRunState::Cancelled]));
        let handler = AgentTaskHandler::new(runtime.clone(), "node-local")
            .with_poll_interval(Duration::from_millis(1));

        assert_eq!(
            handler
                .handle_task(valid_task(), "connection-1".to_owned())
                .await,
            WorkHandlerOutcome::Complete
        );

        assert_eq!(
            runtime.starts.load(Ordering::SeqCst),
            0,
            "queued cancellation must prevent runtime launch"
        );
    }
}
