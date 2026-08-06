//! Durable task mapping for the supported inbound A2A surface.

use chrono::Utc;
use execution_state::{
    WorkAuthorization, WorkCancelOutcome, WorkCursor, WorkDraft, WorkEnvelope, WorkError, WorkItem,
    WorkPage, WorkPolicy, WorkPolicyError, WorkScope, WorkStatus, WorkStore,
};
use gateway_a2a::{project_task, ProtocolError, TaskProjection, TaskProjectionState};
use gateway_bus::WorkTransport;
use gateway_bus::{
    ValidatedWorkCommand, WorkHandler, WorkHandlerAuthorizationError, WorkHandlerContext,
    WorkHandlerOutcome, WorkHandlerPayloadError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

pub const A2A_INBOUND_KIND: &str = "agent.a2a-inbound.v1";
const A2A_SOURCE: &str = "a2a";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct A2aInboundPayload {
    pub task_id: String,
    pub context_id: String,
    pub message_id: String,
    pub text: String,
    pub target_agent_id: String,
    pub public_skill_instructions: String,
    pub conversation_id: String,
    pub session_id: String,
    pub execution_id: String,
    pub root_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aPeerIdentity {
    pub peer_id: String,
    pub target_agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum A2aTaskError {
    #[error("invalid task request")]
    InvalidRequest,
    #[error("message id conflicts with an existing task")]
    MessageConflict,
    #[error("task was not found")]
    NotFound,
    #[error("task cannot be canceled")]
    NotCancelable,
    #[error("task storage is unavailable")]
    StorageUnavailable,
}

pub struct A2aCancelResult {
    pub task: a2a::Task,
    pub session_id: String,
    pub conversation_id: String,
    pub newly_canceled: bool,
}

#[derive(Clone)]
pub struct A2aTaskService {
    store: Arc<dyn WorkStore>,
    transport: Arc<dyn WorkTransport>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
}

impl A2aTaskService {
    pub fn new(
        store: Arc<dyn WorkStore>,
        transport: Arc<dyn WorkTransport>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
    ) -> Self {
        Self {
            store,
            transport,
            messages,
        }
    }

    pub async fn submit(
        &self,
        peer: &A2aPeerIdentity,
        message_id: &str,
        text: &str,
        public_skill_instructions: &str,
    ) -> Result<a2a::Task, A2aTaskError> {
        let task_uuid = uuid::Uuid::new_v4();
        let payload = A2aInboundPayload {
            task_id: format!("task-{task_uuid}"),
            context_id: format!("ctx-{}", uuid::Uuid::new_v4()),
            message_id: message_id.to_string(),
            text: text.to_string(),
            target_agent_id: peer.target_agent_id.clone(),
            public_skill_instructions: public_skill_instructions.to_string(),
            conversation_id: format!("a2a-conversation-{task_uuid}"),
            session_id: format!("sess-{}", uuid::Uuid::new_v4()),
            execution_id: format!("exec-{}", uuid::Uuid::new_v4()),
            root_message_id: format!("msg-{}", uuid::Uuid::new_v4()),
        };
        let target = crate::durable_agent_tasks::AGENT_TASK_TARGET;
        let draft = WorkDraft::new(
            A2A_INBOUND_KIND,
            target,
            serde_json::to_value(&payload).map_err(|_| A2aTaskError::InvalidRequest)?,
        )
        .with_correlation_id(&payload.task_id)
        .with_dedupe_key(dedupe_key(&peer.peer_id, message_id));
        let envelope = WorkEnvelope::authorize(
            draft,
            &InboundPolicy {
                peer,
                expected_target: target,
            },
            Utc::now(),
        )
        .map_err(map_work_error)?;
        let outcome = self.store.enqueue(&envelope).map_err(map_work_error)?;
        let stored_payload = parse_payload(outcome.item())?;
        if !same_request(&stored_payload, &payload) {
            return Err(A2aTaskError::MessageConflict);
        }
        if outcome.inserted() {
            let _ = self.transport.publish(outcome.item().envelope()).await;
        }
        self.project_item(outcome.item(), false)
    }

    pub fn get(
        &self,
        peer_id: &str,
        task_id: &str,
        include_artifacts: bool,
    ) -> Result<a2a::Task, A2aTaskError> {
        let scope = peer_scope(peer_id)?;
        let item = self
            .store
            .find_scoped(&scope, task_id)
            .map_err(map_work_error)?
            .ok_or(A2aTaskError::NotFound)?;
        self.project_item(&item, include_artifacts)
    }

    pub fn list(
        &self,
        peer_id: &str,
        cursor: Option<&WorkCursor>,
        limit: u16,
        include_artifacts: bool,
    ) -> Result<(WorkPage, Vec<a2a::Task>), A2aTaskError> {
        let page = self
            .store
            .list_scoped(&peer_scope(peer_id)?, cursor, limit)
            .map_err(map_work_error)?;
        let tasks = page
            .items()
            .iter()
            .map(|item| self.project_item(item, include_artifacts))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((page, tasks))
    }

    pub fn cancel(&self, peer_id: &str, task_id: &str) -> Result<A2aCancelResult, A2aTaskError> {
        match self
            .store
            .cancel_scoped(&peer_scope(peer_id)?, task_id, Utc::now())
            .map_err(map_work_error)?
        {
            WorkCancelOutcome::Canceled(item) => self.cancel_result(&item, true),
            WorkCancelOutcome::AlreadyCanceled(item) => self.cancel_result(&item, false),
            WorkCancelOutcome::NotCancelable(_) => Err(A2aTaskError::NotCancelable),
            WorkCancelOutcome::NotFound => Err(A2aTaskError::NotFound),
        }
    }

    fn cancel_result(
        &self,
        item: &WorkItem,
        newly_canceled: bool,
    ) -> Result<A2aCancelResult, A2aTaskError> {
        let payload = parse_payload(item)?;
        Ok(A2aCancelResult {
            task: self.project_item(item, false)?,
            session_id: payload.session_id,
            conversation_id: payload.conversation_id,
            newly_canceled,
        })
    }

    fn project_item(
        &self,
        item: &WorkItem,
        include_artifacts: bool,
    ) -> Result<a2a::Task, A2aTaskError> {
        let payload = parse_payload(item)?;
        let state = match item.status() {
            WorkStatus::Pending => TaskProjectionState::Submitted,
            WorkStatus::Leased => TaskProjectionState::Working,
            WorkStatus::Completed => TaskProjectionState::Completed,
            WorkStatus::DeadLetter => TaskProjectionState::Failed,
            WorkStatus::Canceled => TaskProjectionState::Canceled,
        };
        let artifact_text = if include_artifacts && item.status() == WorkStatus::Completed {
            self.messages
                .replay(&payload.session_id, None, 500)
                .map_err(|_| A2aTaskError::StorageUnavailable)?
                .into_iter()
                .rev()
                .find(|message| {
                    message.execution_id.as_deref() == Some(payload.execution_id.as_str())
                        && message.role == "assistant"
                        && !message.content.is_empty()
                        && message.content != "[tool calls]"
                })
                .map(|message| bounded_artifact(&message.content))
        } else {
            None
        };
        project_task(TaskProjection {
            id: payload.task_id,
            context_id: payload.context_id,
            state,
            message: (item.status() == WorkStatus::DeadLetter)
                .then(|| "The remote task failed".to_string()),
            artifact_text,
            updated_at: Some(item.updated_at()),
        })
        .map_err(map_protocol_error)
    }
}

struct InboundPolicy<'a> {
    peer: &'a A2aPeerIdentity,
    expected_target: &'a str,
}

impl WorkPolicy for InboundPolicy<'_> {
    fn authorize(&self, draft: &WorkDraft) -> Result<WorkAuthorization, WorkPolicyError> {
        if draft.kind() != A2A_INBOUND_KIND || draft.target() != self.expected_target {
            return Err(WorkPolicyError::TargetNotAllowed);
        }
        let payload: A2aInboundPayload = serde_json::from_value(draft.payload().clone())
            .map_err(|_| WorkPolicyError::PayloadInvalid)?;
        if payload.target_agent_id != self.peer.target_agent_id {
            return Err(WorkPolicyError::PayloadInvalid);
        }
        Ok(WorkAuthorization::new(
            A2A_SOURCE,
            "node-local",
            actor_id(&self.peer.peer_id),
            &payload.session_id,
            &payload.execution_id,
        ))
    }
}

fn peer_scope(peer_id: &str) -> Result<WorkScope, A2aTaskError> {
    WorkScope::new(A2A_SOURCE, A2A_INBOUND_KIND, actor_id(peer_id)).map_err(map_work_error)
}

fn actor_id(peer_id: &str) -> String {
    format!("a2a:{peer_id}")
}

fn dedupe_key(peer_id: &str, message_id: &str) -> String {
    let digest = Sha256::digest([peer_id.as_bytes(), b"\0", message_id.as_bytes()].concat());
    let mut key = String::from("a2a:");
    for byte in digest {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

fn same_request(existing: &A2aInboundPayload, candidate: &A2aInboundPayload) -> bool {
    existing.message_id == candidate.message_id
        && existing.text == candidate.text
        && existing.target_agent_id == candidate.target_agent_id
        && existing.public_skill_instructions == candidate.public_skill_instructions
}

fn parse_payload(item: &WorkItem) -> Result<A2aInboundPayload, A2aTaskError> {
    serde_json::from_value(item.envelope().payload().clone())
        .map_err(|_| A2aTaskError::StorageUnavailable)
}

fn bounded_artifact(value: &str) -> String {
    value.chars().take(1_000).collect()
}

fn map_protocol_error(_error: ProtocolError) -> A2aTaskError {
    A2aTaskError::StorageUnavailable
}

fn map_work_error(error: WorkError) -> A2aTaskError {
    match error {
        WorkError::InvalidEnvelope(_) | WorkError::Policy(_) => A2aTaskError::InvalidRequest,
        WorkError::NotFound => A2aTaskError::NotFound,
        WorkError::StorageUnavailable | WorkError::StoredDataInvalid | WorkError::StaleLease => {
            A2aTaskError::StorageUnavailable
        }
    }
}

pub struct A2aInboundHandler {
    store: Arc<dyn WorkStore>,
    runtime: Arc<crate::services::RuntimeService>,
    state: Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    agents: Arc<gateway_services::AgentService>,
}

impl A2aInboundHandler {
    pub fn new(
        store: Arc<dyn WorkStore>,
        runtime: Arc<crate::services::RuntimeService>,
        state: Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        agents: Arc<gateway_services::AgentService>,
    ) -> Self {
        Self {
            store,
            runtime,
            state,
            messages,
            agents,
        }
    }

    fn work_is_canceled(&self, work_id: &str) -> Result<bool, WorkHandlerOutcome> {
        self.store
            .get(work_id)
            .map_err(|_| retry_outcome())?
            .map(|item| item.status() == WorkStatus::Canceled)
            .ok_or_else(integrity_outcome)
    }

    async fn settle_if_canceled(
        &self,
        work_id: &str,
        task: &A2aInboundPayload,
    ) -> Result<bool, WorkHandlerOutcome> {
        if !self.work_is_canceled(work_id)? {
            return Ok(false);
        }
        if self
            .state
            .get_session(&task.session_id)
            .map_err(|_| retry_outcome())?
            .is_some()
        {
            let _ = self
                .runtime
                .cancel_exact(&task.session_id, &task.conversation_id)
                .await;
        }
        Ok(true)
    }

    async fn validate_agent(&self, agent_id: &str) -> Result<(), WorkHandlerOutcome> {
        crate::durable_agent_tasks::validate_invocation_agent_id(agent_id).map_err(|_| {
            WorkHandlerOutcome::Permanent(execution_state::WorkFailureCode::IntegrityViolation)
        })?;
        if agent_id != "root" {
            self.agents.get(agent_id).await.map_err(|_| {
                WorkHandlerOutcome::Permanent(execution_state::WorkFailureCode::IntegrityViolation)
            })?;
        }
        Ok(())
    }

    fn ensure_initial_state(&self, task: &A2aInboundPayload) -> Result<(), WorkHandlerOutcome> {
        match self.state.get_session(&task.session_id) {
            Ok(Some(session)) if session.root_agent_id == task.target_agent_id => {}
            Ok(Some(_)) => return Err(integrity_outcome()),
            Ok(None) => {
                let session = execution_state::Session::new_with_id(
                    &task.session_id,
                    &task.target_agent_id,
                    execution_state::TriggerSource::Web,
                )
                .map_err(|_| integrity_outcome())?;
                self.state
                    .create_session_from(&session)
                    .map_err(|_| retry_outcome())?;
            }
            Err(_) => return Err(retry_outcome()),
        }
        match self.state.get_root_execution(&task.session_id) {
            Ok(Some(execution))
                if execution.id == task.execution_id
                    && execution.agent_id == task.target_agent_id => {}
            Ok(Some(_)) => return Err(integrity_outcome()),
            Ok(None) => {
                if self
                    .state
                    .get_execution(&task.execution_id)
                    .map_err(|_| retry_outcome())?
                    .is_some()
                {
                    return Err(integrity_outcome());
                }
                let execution = execution_state::AgentExecution::new_root_with_id(
                    &task.execution_id,
                    &task.session_id,
                    &task.target_agent_id,
                )
                .map_err(|_| integrity_outcome())?;
                self.state
                    .create_execution(&execution)
                    .map_err(|_| retry_outcome())?;
            }
            Err(_) => return Err(retry_outcome()),
        }
        Ok(())
    }

    fn inspect(&self, task: &A2aInboundPayload) -> Result<InboundRunState, WorkHandlerOutcome> {
        let session = self
            .state
            .get_session(&task.session_id)
            .map_err(|_| retry_outcome())?;
        let execution = self
            .state
            .get_execution(&task.execution_id)
            .map_err(|_| retry_outcome())?;
        let message = self
            .messages
            .get(&task.root_message_id)
            .map_err(|_| retry_outcome())?;
        let (Some(session), Some(execution), Some(message)) = (session, execution, message) else {
            return Ok(InboundRunState::Initial);
        };
        if session.root_agent_id != task.target_agent_id
            || execution.session_id != task.session_id
            || execution.agent_id != task.target_agent_id
            || message.session_id != task.session_id
            || message.execution_id.as_deref() != Some(task.execution_id.as_str())
            || message.role != "user"
            || message.content != task.text
        {
            return Err(integrity_outcome());
        }
        if execution.status == execution_state::ExecutionStatus::Cancelled {
            return Ok(InboundRunState::Canceled);
        }
        if session.status == execution_state::SessionStatus::Completed
            && execution.status == execution_state::ExecutionStatus::Completed
        {
            return Ok(InboundRunState::Completed);
        }
        if session.status == execution_state::SessionStatus::Running
            && execution.status == execution_state::ExecutionStatus::Running
        {
            return Ok(InboundRunState::Running);
        }
        Ok(InboundRunState::Resume)
    }

    async fn launch(
        &self,
        task: &A2aInboundPayload,
        actor_id: &str,
        resume: bool,
    ) -> Result<(), WorkHandlerOutcome> {
        let prompt = gateway_execution::a2a::build_remote_peer_prompt(
            &task.public_skill_instructions,
            &task.text,
        )
        .map_err(|_| integrity_outcome())?;
        let result = if resume {
            self.runtime
                .invoke_remote_peer_persisted(
                    &task.target_agent_id,
                    &task.conversation_id,
                    &task.text,
                    actor_id,
                    task.session_id.clone(),
                    task.execution_id.clone(),
                    task.root_message_id.clone(),
                    prompt,
                )
                .await
        } else {
            self.runtime
                .invoke_remote_peer_durable(
                    &task.target_agent_id,
                    &task.conversation_id,
                    &task.text,
                    actor_id,
                    task.session_id.clone(),
                    task.root_message_id.clone(),
                    prompt,
                )
                .await
        };
        result.map(|_| ()).map_err(|_| retry_outcome())
    }

    async fn run(
        &self,
        work_id: String,
        task: A2aInboundPayload,
        actor_id: String,
    ) -> WorkHandlerOutcome {
        if let Err(outcome) = self.validate_agent(&task.target_agent_id).await {
            return outcome;
        }
        match self.settle_if_canceled(&work_id, &task).await {
            Ok(true) => return WorkHandlerOutcome::Complete,
            Ok(false) => {}
            Err(outcome) => return outcome,
        }
        let initial = match self.inspect(&task) {
            Ok(state) => state,
            Err(outcome) => return outcome,
        };
        let launch = match initial {
            InboundRunState::Initial => {
                if let Err(outcome) = self.ensure_initial_state(&task) {
                    return outcome;
                }
                match self.settle_if_canceled(&work_id, &task).await {
                    Ok(true) => return WorkHandlerOutcome::Complete,
                    Ok(false) => self.launch(&task, &actor_id, false).await,
                    Err(outcome) => return outcome,
                }
            }
            InboundRunState::Resume => self.launch(&task, &actor_id, true).await,
            InboundRunState::Running => Ok(()),
            InboundRunState::Completed | InboundRunState::Canceled => {
                return WorkHandlerOutcome::Complete;
            }
        };
        if let Err(outcome) = launch {
            return outcome;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(55 * 60);
        loop {
            match self.settle_if_canceled(&work_id, &task).await {
                Ok(true) => return WorkHandlerOutcome::Complete,
                Ok(false) => {}
                Err(outcome) => return outcome,
            }
            if tokio::time::Instant::now() >= deadline {
                return retry_outcome();
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            match self.inspect(&task) {
                Ok(InboundRunState::Completed | InboundRunState::Canceled) => {
                    return WorkHandlerOutcome::Complete;
                }
                Ok(InboundRunState::Running) => {}
                Ok(InboundRunState::Initial | InboundRunState::Resume) => {
                    return retry_outcome();
                }
                Err(outcome) => return outcome,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboundRunState {
    Initial,
    Running,
    Resume,
    Completed,
    Canceled,
}

#[async_trait::async_trait]
impl WorkHandler for A2aInboundHandler {
    fn target(&self) -> &'static str {
        crate::durable_agent_tasks::AGENT_TASK_TARGET
    }

    fn kind(&self) -> &'static str {
        A2A_INBOUND_KIND
    }

    fn validate_payload(
        &self,
        payload: &serde_json::Value,
    ) -> Result<ValidatedWorkCommand, WorkHandlerPayloadError> {
        let task: A2aInboundPayload = serde_json::from_value(payload.clone())
            .map_err(|_| WorkHandlerPayloadError::Invalid)?;
        validate_inbound_payload(&task).map_err(|_| WorkHandlerPayloadError::Invalid)?;
        Ok(ValidatedWorkCommand::new(task))
    }

    fn authorize(
        &self,
        context: &WorkHandlerContext,
        command: &ValidatedWorkCommand,
    ) -> Result<(), WorkHandlerAuthorizationError> {
        let task = command
            .downcast_ref::<A2aInboundPayload>()
            .ok_or(WorkHandlerAuthorizationError::Rejected)?;
        let provenance = context.provenance();
        if context.source() != A2A_SOURCE
            || context.correlation_id() != Some(task.task_id.as_str())
            || !provenance.actor_id().starts_with("a2a:")
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
        let task = match command.downcast::<A2aInboundPayload>() {
            Ok(task) => task,
            Err(_) => return integrity_outcome(),
        };
        self.run(
            context.work_id().to_string(),
            task,
            context.provenance().actor_id().to_string(),
        )
        .await
    }
}

fn validate_inbound_payload(task: &A2aInboundPayload) -> Result<(), ()> {
    if !valid_id(&task.task_id, "task-")
        || !valid_id(&task.context_id, "ctx-")
        || !valid_id(&task.session_id, "sess-")
        || !valid_id(&task.execution_id, "exec-")
        || !valid_id(&task.root_message_id, "msg-")
        || task.message_id.is_empty()
        || task.message_id.len() > 128
        || task.conversation_id.is_empty()
        || task.conversation_id.len() > 128
        || gateway_execution::a2a::build_remote_peer_prompt(
            &task.public_skill_instructions,
            &task.text,
        )
        .is_err()
    {
        return Err(());
    }
    Ok(())
}

fn valid_id(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|suffix| uuid::Uuid::parse_str(suffix).is_ok())
}

fn integrity_outcome() -> WorkHandlerOutcome {
    WorkHandlerOutcome::Permanent(execution_state::WorkFailureCode::IntegrityViolation)
}

fn retry_outcome() -> WorkHandlerOutcome {
    WorkHandlerOutcome::Retry(execution_state::WorkFailureCode::Internal)
}
