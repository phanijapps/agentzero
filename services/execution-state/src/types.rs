//! # Session State Types
//!
//! Core types for session tracking, agent executions, and checkpointing.

use serde::{Deserialize, Serialize};
use std::io::{self, Write};

// ============================================================================
// SESSION STATUS
// ============================================================================

/// Status of a session (top-level container).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Session created but not yet started
    Queued,
    /// At least one execution is running
    Running,
    /// User paused the session
    Paused,
    /// All executions completed successfully
    Completed,
    /// Root execution crashed
    Crashed,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Crashed => "crashed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Crashed)
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "crashed" => Ok(Self::Crashed),
            _ => Err(format!("Invalid session status: {}", s)),
        }
    }
}

// ============================================================================
// TRIGGER SOURCE
// ============================================================================

/// Source that triggered the session creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TriggerSource {
    /// Triggered from web UI
    #[default]
    Web,
    /// Triggered from CLI
    Cli,
    /// Triggered by cron/scheduler
    Cron,
    /// Triggered via HTTP API
    Api,
    /// Triggered by an external connector
    #[serde(alias = "plugin")]
    Connector,
}

impl TriggerSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Cli => "cli",
            Self::Cron => "cron",
            Self::Api => "api",
            Self::Connector => "connector",
        }
    }

    /// Returns true if sessions from this source should auto-complete after execution.
    ///
    /// Web sessions stay open for interactive use (until `/end` or `/new`).
    /// CLI, Cron, API, and Plugin sessions auto-complete after execution finishes.
    pub fn should_auto_complete_session(&self) -> bool {
        match self {
            Self::Web => false,      // Keep open for interactive use
            Self::Cli => true,       // Complete after response
            Self::Cron => true,      // Single execution, auto-complete
            Self::Api => true,       // Controlled by caller, default to complete
            Self::Connector => true, // External connector, auto-complete
        }
    }
}

impl std::fmt::Display for TriggerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for TriggerSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "web" => Ok(Self::Web),
            "cli" => Ok(Self::Cli),
            "cron" => Ok(Self::Cron),
            "api" => Ok(Self::Api),
            "connector" | "plugin" => Ok(Self::Connector),
            _ => Err(format!("Invalid trigger source: {}", s)),
        }
    }
}

// ============================================================================
// EXECUTION STATUS
// ============================================================================

/// Status of an agent execution within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    /// Created but not yet started
    Queued,
    /// Actively executing
    Running,
    /// Paused (session paused or waiting)
    Paused,
    /// Failed with error
    Crashed,
    /// Cancelled by user or parent
    Cancelled,
    /// Successfully finished
    Completed,
}

impl ExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Crashed => "crashed",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Cancelled | Self::Completed | Self::Crashed)
    }

    pub fn is_resumable(&self) -> bool {
        matches!(self, Self::Paused | Self::Crashed)
    }
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ExecutionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "crashed" => Ok(Self::Crashed),
            "cancelled" => Ok(Self::Cancelled),
            "completed" => Ok(Self::Completed),
            _ => Err(format!("Invalid execution status: {}", s)),
        }
    }
}

// ============================================================================
// DELEGATION TYPE
// ============================================================================

/// How an agent execution was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DelegationType {
    /// Root agent (started by user)
    Root,
    /// Delegated sequentially (parent waits)
    Sequential,
    /// Delegated in parallel (parent continues)
    Parallel,
}

impl DelegationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Sequential => "sequential",
            Self::Parallel => "parallel",
        }
    }
}

impl std::fmt::Display for DelegationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for DelegationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "root" => Ok(Self::Root),
            "sequential" => Ok(Self::Sequential),
            "parallel" => Ok(Self::Parallel),
            _ => Err(format!("Invalid delegation type: {}", s)),
        }
    }
}

// ============================================================================
// SESSION
// ============================================================================

/// A user's work session - top-level container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID (sess-{uuid})
    pub id: String,

    /// Current status
    pub status: SessionStatus,

    /// Trigger source (web, cli, cron, api, connector)
    pub source: TriggerSource,

    /// The root agent for this session
    pub root_agent_id: String,

    /// Optional title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// When the session was created (RFC3339)
    pub created_at: String,

    /// When first execution started (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,

    /// When session completed (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,

    /// Total input tokens across all executions
    pub total_tokens_in: u64,

    /// Total output tokens across all executions
    pub total_tokens_out: u64,

    /// JSON metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Number of pending delegations (subagents not yet completed)
    #[serde(default)]
    pub pending_delegations: u32,

    /// Whether this session needs a continuation turn after delegations complete
    #[serde(default)]
    pub continuation_needed: bool,

    /// Active ward (named project directory) for this session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ward_id: Option<String>,

    /// Parent session ID (None = root session, Some = child/subagent session)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,

    /// Thread ID for conversation threading with external connectors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,

    /// Connector ID that triggered this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,

    /// Connector IDs to route the final response to (stored as JSON in DB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<Vec<String>>,

    /// Execution mode persisted on the session (e.g. "fast").
    /// When set, overrides per-invoke mode so continuations keep the same behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Result of atomically binding an initially wardless session to a workspace.
///
/// The caller must use the contained effective ward for its own execution
/// context. `Existing` means another invocation or an earlier lifecycle event
/// already established the session workspace and must not be overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionWardClaim {
    Claimed(String),
    Existing(String),
}

fn validate_prefixed_uuid(value: &str, prefix: &str) -> Result<(), String> {
    let suffix = value
        .strip_prefix(prefix)
        .ok_or_else(|| "invalid_reserved_id".to_string())?;
    let parsed = uuid::Uuid::parse_str(suffix).map_err(|_| "invalid_reserved_id".to_string())?;
    if parsed.hyphenated().to_string() != suffix {
        return Err("invalid_reserved_id".to_string());
    }
    Ok(())
}

impl Session {
    /// Create a new session in RUNNING state with Web source (default).
    pub fn new(root_agent_id: impl Into<String>) -> Self {
        Self::new_with_source(root_agent_id, TriggerSource::Web)
    }

    /// Create a new session in RUNNING state with specified source.
    pub fn new_with_source(root_agent_id: impl Into<String>, source: TriggerSource) -> Self {
        Self {
            id: format!("sess-{}", uuid::Uuid::new_v4()),
            status: SessionStatus::Running,
            source,
            root_agent_id: root_agent_id.into(),
            title: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            total_tokens_in: 0,
            total_tokens_out: 0,
            metadata: None,
            pending_delegations: 0,
            continuation_needed: false,
            ward_id: None,
            parent_session_id: None,
            thread_id: None,
            connector_id: None,
            respond_to: None,
            mode: None,
        }
    }

    /// Create a running session with a caller-reserved canonical ID.
    pub fn new_with_id(
        id: impl Into<String>,
        root_agent_id: impl Into<String>,
        source: TriggerSource,
    ) -> Result<Self, String> {
        let id = id.into();
        validate_prefixed_uuid(&id, "sess-")?;
        let mut session = Self::new_with_source(root_agent_id, source);
        session.id = id;
        Ok(session)
    }

    /// Create a new session in QUEUED state (not yet started).
    pub fn new_queued(root_agent_id: impl Into<String>, source: TriggerSource) -> Self {
        Self {
            id: format!("sess-{}", uuid::Uuid::new_v4()),
            status: SessionStatus::Queued,
            source,
            root_agent_id: root_agent_id.into(),
            title: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            total_tokens_in: 0,
            total_tokens_out: 0,
            metadata: None,
            pending_delegations: 0,
            continuation_needed: false,
            ward_id: None,
            parent_session_id: None,
            thread_id: None,
            connector_id: None,
            respond_to: None,
            mode: None,
        }
    }

    /// Create a queued session with a caller-reserved canonical ID.
    pub fn new_queued_with_id(
        id: impl Into<String>,
        root_agent_id: impl Into<String>,
        source: TriggerSource,
    ) -> Result<Self, String> {
        let id = id.into();
        validate_prefixed_uuid(&id, "sess-")?;
        let mut session = Self::new_queued(root_agent_id, source);
        session.id = id;
        Ok(session)
    }

    /// Create a child session for a subagent (isolated conversation context).
    pub fn new_child(
        root_agent_id: impl Into<String>,
        parent_session_id: impl Into<String>,
    ) -> Self {
        Self {
            id: format!("sess-{}", uuid::Uuid::new_v4()),
            status: SessionStatus::Running,
            source: TriggerSource::Web,
            root_agent_id: root_agent_id.into(),
            title: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            total_tokens_in: 0,
            total_tokens_out: 0,
            metadata: None,
            pending_delegations: 0,
            continuation_needed: false,
            ward_id: None,
            parent_session_id: Some(parent_session_id.into()),
            thread_id: None,
            connector_id: None,
            respond_to: None,
            mode: None,
        }
    }

    /// Set routing fields (thread_id, connector_id, respond_to) for connector sessions.
    #[must_use]
    pub fn with_routing(
        mut self,
        thread_id: Option<String>,
        connector_id: Option<String>,
        respond_to: Option<Vec<String>>,
    ) -> Self {
        self.thread_id = thread_id;
        self.connector_id = connector_id;
        self.respond_to = respond_to;
        self
    }

    /// Total tokens (in + out).
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens_in + self.total_tokens_out
    }

    /// Check if this session has pending delegations.
    pub fn has_pending_delegations(&self) -> bool {
        self.pending_delegations > 0
    }

    /// Check if this session needs a continuation turn.
    ///
    /// Returns true only if continuation is needed AND no delegations are pending.
    pub fn needs_continuation(&self) -> bool {
        self.continuation_needed && !self.has_pending_delegations()
    }
}

// ============================================================================
// AGENT EXECUTION
// ============================================================================

/// An agent's participation in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExecution {
    /// Unique execution ID (exec-{uuid})
    pub id: String,

    /// Session this execution belongs to
    pub session_id: String,

    /// Agent being executed
    pub agent_id: String,

    /// Parent execution ID (for delegated subagents)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,

    /// How this execution was created
    pub delegation_type: DelegationType,

    /// Task description (for subagents)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,

    /// Current status
    pub status: ExecutionStatus,

    /// When execution started (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,

    /// When execution completed (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,

    /// Input tokens consumed
    pub tokens_in: u64,

    /// Output tokens consumed
    pub tokens_out: u64,

    /// Checkpoint for resumption
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,

    /// Error message if crashed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Relative path to log file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,

    /// Child session ID (for delegated executions only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

impl AgentExecution {
    /// Create a new root execution in QUEUED state.
    pub fn new_root(session_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            id: format!("exec-{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            parent_execution_id: None,
            delegation_type: DelegationType::Root,
            task: None,
            status: ExecutionStatus::Queued,
            started_at: None,
            completed_at: None,
            tokens_in: 0,
            tokens_out: 0,
            checkpoint: None,
            error: None,
            log_path: None,
            child_session_id: None,
        }
    }

    /// Create a queued root execution with a caller-reserved canonical ID.
    pub fn new_root_with_id(
        id: impl Into<String>,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Result<Self, String> {
        let id = id.into();
        validate_prefixed_uuid(&id, "exec-")?;
        let mut execution = Self::new_root(session_id, agent_id);
        execution.id = id;
        Ok(execution)
    }

    /// Create a new delegated execution.
    pub fn new_delegated(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        parent_execution_id: impl Into<String>,
        delegation_type: DelegationType,
        task: impl Into<String>,
    ) -> Self {
        Self {
            id: format!("exec-{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            parent_execution_id: Some(parent_execution_id.into()),
            delegation_type,
            task: Some(task.into()),
            status: ExecutionStatus::Queued,
            started_at: None,
            completed_at: None,
            tokens_in: 0,
            tokens_out: 0,
            checkpoint: None,
            error: None,
            log_path: None,
            child_session_id: None,
        }
    }

    /// Create a new delegated execution with a caller-provided ID.
    pub fn new_delegated_with_id(
        id: impl Into<String>,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        parent_execution_id: impl Into<String>,
        delegation_type: DelegationType,
        task: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            parent_execution_id: Some(parent_execution_id.into()),
            delegation_type,
            task: Some(task.into()),
            status: ExecutionStatus::Queued,
            started_at: None,
            completed_at: None,
            tokens_in: 0,
            tokens_out: 0,
            checkpoint: None,
            error: None,
            log_path: None,
            child_session_id: None,
        }
    }

    /// Calculate duration in milliseconds (if completed).
    pub fn duration_ms(&self) -> Option<i64> {
        let started = self.started_at.as_ref()?;
        let completed = self.completed_at.as_ref()?;

        let start = chrono::DateTime::parse_from_rfc3339(started).ok()?;
        let end = chrono::DateTime::parse_from_rfc3339(completed).ok()?;

        Some((end - start).num_milliseconds())
    }

    /// Total tokens (in + out).
    pub fn total_tokens(&self) -> u64 {
        self.tokens_in + self.tokens_out
    }

    /// Check if this is a root execution.
    pub fn is_root(&self) -> bool {
        self.delegation_type == DelegationType::Root
    }
}

// ============================================================================
// SESSION WITH EXECUTIONS (API response)
// ============================================================================

/// Session with all its executions - for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWithExecutions {
    /// The session
    #[serde(flatten)]
    pub session: Session,

    /// All executions in this session
    pub executions: Vec<AgentExecution>,

    /// Number of subagent executions
    pub subagent_count: u32,
}

/// Minimal per-execution token/status slice for selected Mission Control detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionControlExecutionSummary {
    pub execution_id: String,
    pub agent_id: String,
    pub delegation_type: DelegationType,
    pub tokens_in: u64,
    pub tokens_out: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

/// Bounded high-level session row used by Mission Control's initial page load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionControlSessionSummary {
    /// The underlying `sessions.id` value (`sess-*`).
    pub conversation_id: String,
    /// Root `agent_executions.id` value. Existing log-detail APIs use this id.
    pub root_execution_id: String,
    pub status: SessionStatus,
    pub source: TriggerSource,
    pub root_agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub subagent_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Per-execution token slices for one selected Mission Control session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionControlSessionTokens {
    pub conversation_id: String,
    pub root_execution_id: String,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub executions: Vec<MissionControlExecutionSummary>,
    /// Latest validated plan for this selected session. This is intentionally
    /// absent from the bounded session-list response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_plan: Option<SessionPlanSnapshot>,
}

// ============================================================================
// CURRENT SESSION PLAN
// ============================================================================

/// Maximum number of structured steps accepted from `update_plan`.
pub const MAX_SESSION_PLAN_STEPS: usize = 20;
/// Maximum UTF-8 byte length of one plan step.
pub const MAX_SESSION_PLAN_STEP_BYTES: usize = 512;
/// Maximum UTF-8 byte length of an optional plan explanation.
pub const MAX_SESSION_PLAN_EXPLANATION_BYTES: usize = 1_000;
/// Maximum serialized byte length of model-owned plan data.
pub const MAX_SESSION_PLAN_PAYLOAD_BYTES: usize = 16 * 1024;

/// The only statuses persisted for a current session plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// One normalized current-plan step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPlanStep {
    pub step: String,
    pub status: SessionPlanStepStatus,
}

/// Strict model-owned portion of an `update_plan` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPlanInput {
    pub plan: Vec<SessionPlanStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

#[derive(Serialize)]
struct BorrowedSessionPlanInput<'a> {
    plan: &'a serde_json::Value,
    explanation: Option<&'a str>,
}

struct BoundedJsonCounter {
    remaining: usize,
}

impl BoundedJsonCounter {
    const fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }
}

impl Write for BoundedJsonCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.remaining {
            return Err(io::Error::other("serialized plan exceeds byte budget"));
        }
        self.remaining -= buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A reason code for rejecting model-owned plan input. It deliberately contains
/// no user or model text so callers can log it safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPlanRejection {
    InvalidStructure,
    EmptyPlan,
    TooManySteps,
    EmptyStep,
    StepTooLong,
    ExplanationTooLong,
    PayloadTooLarge,
    ExecutionSessionMismatch,
    StaleSequence,
}

impl SessionPlanRejection {
    /// Stable, safe-to-log rejection code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidStructure => "invalid_structure",
            Self::EmptyPlan => "empty_plan",
            Self::TooManySteps => "too_many_steps",
            Self::EmptyStep => "empty_step",
            Self::StepTooLong => "step_too_long",
            Self::ExplanationTooLong => "explanation_too_long",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ExecutionSessionMismatch => "execution_session_mismatch",
            Self::StaleSequence => "stale_sequence",
        }
    }
}

impl SessionPlanInput {
    /// Check an untrusted update without cloning its JSON or deserializing it
    /// into owned plan-step strings. This guard runs at the stream boundary and
    /// again before persistence as defence in depth.
    pub fn preflight_update(
        plan: &serde_json::Value,
        explanation: Option<&str>,
    ) -> Result<(), SessionPlanRejection> {
        let steps = plan
            .as_array()
            .ok_or(SessionPlanRejection::InvalidStructure)?;
        if steps.is_empty() {
            return Err(SessionPlanRejection::EmptyPlan);
        }
        if steps.len() > MAX_SESSION_PLAN_STEPS {
            return Err(SessionPlanRejection::TooManySteps);
        }
        for raw_step in steps {
            let object = raw_step
                .as_object()
                .ok_or(SessionPlanRejection::InvalidStructure)?;
            if object.len() != 2 || !object.contains_key("step") || !object.contains_key("status") {
                return Err(SessionPlanRejection::InvalidStructure);
            }
            let step = object
                .get("step")
                .and_then(serde_json::Value::as_str)
                .ok_or(SessionPlanRejection::InvalidStructure)?;
            if step.trim().is_empty() {
                return Err(SessionPlanRejection::EmptyStep);
            }
            if step.len() > MAX_SESSION_PLAN_STEP_BYTES {
                return Err(SessionPlanRejection::StepTooLong);
            }
            match object.get("status").and_then(serde_json::Value::as_str) {
                Some("pending" | "in_progress" | "completed" | "failed") => {}
                Some(_) | None => return Err(SessionPlanRejection::InvalidStructure),
            }
        }
        if explanation.is_some_and(|value| value.len() > MAX_SESSION_PLAN_EXPLANATION_BYTES) {
            return Err(SessionPlanRejection::ExplanationTooLong);
        }

        let mut counter = BoundedJsonCounter::new(MAX_SESSION_PLAN_PAYLOAD_BYTES);
        serde_json::to_writer(
            &mut counter,
            &BorrowedSessionPlanInput { plan, explanation },
        )
        .map_err(|_| SessionPlanRejection::PayloadTooLarge)?;
        Ok(())
    }

    /// Deserialize and validate the model-owned fields before any database
    /// operation. Trusted ownership and ordering data are supplied separately.
    pub fn from_update(
        plan: serde_json::Value,
        explanation: Option<String>,
    ) -> Result<Self, SessionPlanRejection> {
        Self::preflight_update(&plan, explanation.as_deref())?;
        let input = serde_json::from_value::<Self>(serde_json::json!({
            "plan": plan,
            "explanation": explanation }))
        .map_err(|_| SessionPlanRejection::InvalidStructure)?;

        input.validate()?;
        Ok(input)
    }

    fn validate(&self) -> Result<(), SessionPlanRejection> {
        if self.plan.is_empty() {
            return Err(SessionPlanRejection::EmptyPlan);
        }
        if self.plan.len() > MAX_SESSION_PLAN_STEPS {
            return Err(SessionPlanRejection::TooManySteps);
        }
        for step in &self.plan {
            if step.step.trim().is_empty() {
                return Err(SessionPlanRejection::EmptyStep);
            }
            if step.step.len() > MAX_SESSION_PLAN_STEP_BYTES {
                return Err(SessionPlanRejection::StepTooLong);
            }
        }
        if self
            .explanation
            .as_deref()
            .is_some_and(|value| value.len() > MAX_SESSION_PLAN_EXPLANATION_BYTES)
        {
            return Err(SessionPlanRejection::ExplanationTooLong);
        }
        Ok(())
    }
}

/// One persisted, selected-session plan snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPlanSnapshot {
    pub execution_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    pub plan: Vec<SessionPlanStep>,
    pub updated_at: String,
    /// Internal ordering metadata. It is not part of the public Mission
    /// Control response.
    #[serde(skip_serializing)]
    pub source_event_timestamp: u64,
    /// Internal ordering metadata. It is not part of the public Mission
    /// Control response.
    #[serde(skip_serializing)]
    pub source_event_sequence: u64,
}

/// Opaque persisted work-surface descriptor.
///
/// The execution-state layer owns retention and session lifecycle only.
/// Gateway code owns semantic deserialization and catalog validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSurfaceRecord {
    pub session_id: String,
    pub surface_id: String,
    pub execution_id: String,
    pub surface_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Outcome of attempting to save a plan snapshot. Rejections are expected model
/// input outcomes, while database failures are returned separately as errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPlanSaveOutcome {
    Accepted(SessionPlanSnapshot),
    Rejected(SessionPlanRejection),
}

impl SessionPlanSaveOutcome {
    #[must_use]
    pub fn accepted(self) -> Option<SessionPlanSnapshot> {
        match self {
            Self::Accepted(snapshot) => Some(snapshot),
            Self::Rejected(_) => None,
        }
    }
}

// ============================================================================
// CHECKPOINT
// ============================================================================

/// Checkpoint data for resuming a paused or crashed execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// LLM turn number
    pub llm_turn: u32,

    /// ID of the last processed message
    pub last_message_id: String,

    /// Tool calls that were in progress
    #[serde(default)]
    pub pending_tool_calls: Vec<PendingToolCall>,

    /// Snapshot of context state
    #[serde(default)]
    pub context_state: serde_json::Value,

    /// IDs of child executions that were active
    #[serde(default)]
    pub child_executions: Vec<String>,
}

impl Checkpoint {
    pub fn new(llm_turn: u32, last_message_id: impl Into<String>) -> Self {
        Self {
            llm_turn,
            last_message_id: last_message_id.into(),
            pending_tool_calls: Vec::new(),
            context_state: serde_json::Value::Null,
            child_executions: Vec::new(),
        }
    }
}

/// A tool call that was pending when execution was paused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

// ============================================================================
// STATS (API response - ready to display)
// ============================================================================

/// Dashboard stats - pre-computed, ready for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardStats {
    // =========================================================================
    // SESSION COUNTS
    // =========================================================================
    /// Number of queued sessions (waiting to start)
    pub sessions_queued: u64,

    /// Number of running sessions
    pub sessions_running: u64,

    /// Number of paused sessions
    pub sessions_paused: u64,

    /// Number of completed sessions
    pub sessions_completed: u64,

    /// Number of crashed sessions
    pub sessions_crashed: u64,

    // =========================================================================
    // EXECUTION COUNTS
    // =========================================================================
    /// Number of queued executions (waiting to start)
    pub executions_queued: u64,

    /// Number of running executions
    pub executions_running: u64,

    /// Number of completed executions
    pub executions_completed: u64,

    /// Number of crashed executions
    pub executions_crashed: u64,

    /// Number of cancelled executions
    pub executions_cancelled: u64,

    // =========================================================================
    // DAILY STATS
    // =========================================================================
    /// Total sessions today
    pub today_sessions: u64,

    /// Total tokens today
    pub today_tokens: u64,

    // =========================================================================
    // BREAKDOWN BY SOURCE
    // =========================================================================
    /// Sessions count by trigger source (web, cli, cron, api, connector)
    pub sessions_by_source: std::collections::HashMap<String, u64>,
}

// ============================================================================
// FILTERS
// ============================================================================

/// Filter criteria for querying sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SessionStatus>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_agent_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_time: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_time: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Filter criteria for Mission Control summary rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissionControlFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

/// Filter criteria for querying executions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ExecutionStatus>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

// ============================================================================
// API ERROR
// ============================================================================

/// API error type for handlers.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;

        let (status, message) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ApiError::Database(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::InvalidTransition(msg) => (StatusCode::CONFLICT, msg.clone()),
        };

        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}

// ============================================================================
// BACKWARDS COMPAT (temporary aliases)
// ============================================================================

/// Alias for backwards compatibility during migration.
pub type ExecutionSession = AgentExecution;

// ============================================================================
// ARTIFACT
// ============================================================================

/// A file artifact produced by an agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub session_id: String,
    pub ward_id: Option<String>,
    pub execution_id: Option<String>,
    pub agent_id: Option<String>,
    pub file_path: String,
    pub file_name: String,
    pub file_type: Option<String>,
    pub file_size: Option<i64>,
    pub label: Option<String>,
    pub is_goal_artifact: bool,
    pub created_at: String,
}

impl Artifact {
    pub fn new(
        session_id: impl Into<String>,
        file_path: impl Into<String>,
        file_name: impl Into<String>,
    ) -> Self {
        Self {
            id: format!("art-{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            ward_id: None,
            execution_id: None,
            agent_id: None,
            file_path: file_path.into(),
            file_name: file_name.into(),
            file_type: None,
            file_size: None,
            label: None,
            is_goal_artifact: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // SessionStatus Tests
    // ========================================================================

    #[test]
    fn session_status_as_str() {
        assert_eq!(SessionStatus::Queued.as_str(), "queued");
        assert_eq!(SessionStatus::Running.as_str(), "running");
        assert_eq!(SessionStatus::Paused.as_str(), "paused");
        assert_eq!(SessionStatus::Completed.as_str(), "completed");
        assert_eq!(SessionStatus::Crashed.as_str(), "crashed");
    }

    #[test]
    fn session_status_display() {
        assert_eq!(format!("{}", SessionStatus::Queued), "queued");
        assert_eq!(format!("{}", SessionStatus::Running), "running");
        assert_eq!(format!("{}", SessionStatus::Completed), "completed");
    }

    #[test]
    fn session_status_from_str() {
        assert_eq!(
            "queued".parse::<SessionStatus>().unwrap(),
            SessionStatus::Queued
        );
        assert_eq!(
            "running".parse::<SessionStatus>().unwrap(),
            SessionStatus::Running
        );
        assert_eq!(
            "PAUSED".parse::<SessionStatus>().unwrap(),
            SessionStatus::Paused
        );
        assert_eq!(
            "Completed".parse::<SessionStatus>().unwrap(),
            SessionStatus::Completed
        );
        assert_eq!(
            "crashed".parse::<SessionStatus>().unwrap(),
            SessionStatus::Crashed
        );
    }

    #[test]
    fn session_status_from_str_invalid() {
        assert!("invalid".parse::<SessionStatus>().is_err());
        assert!("".parse::<SessionStatus>().is_err());
    }

    #[test]
    fn session_status_is_terminal() {
        assert!(!SessionStatus::Queued.is_terminal());
        assert!(!SessionStatus::Running.is_terminal());
        assert!(!SessionStatus::Paused.is_terminal());
        assert!(SessionStatus::Completed.is_terminal());
        assert!(SessionStatus::Crashed.is_terminal());
    }

    #[test]
    fn session_status_serialization() {
        let statuses = [
            (SessionStatus::Queued, "\"queued\""),
            (SessionStatus::Running, "\"running\""),
            (SessionStatus::Paused, "\"paused\""),
            (SessionStatus::Completed, "\"completed\""),
            (SessionStatus::Crashed, "\"crashed\""),
        ];

        for (status, expected) in statuses {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected, "Serialization failed for {:?}", status);

            let parsed: SessionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status, "Deserialization failed for {}", expected);
        }
    }

    // ========================================================================
    // TriggerSource Tests
    // ========================================================================

    #[test]
    fn trigger_source_as_str() {
        assert_eq!(TriggerSource::Web.as_str(), "web");
        assert_eq!(TriggerSource::Cli.as_str(), "cli");
        assert_eq!(TriggerSource::Cron.as_str(), "cron");
        assert_eq!(TriggerSource::Api.as_str(), "api");
        assert_eq!(TriggerSource::Connector.as_str(), "connector");
    }

    #[test]
    fn trigger_source_default() {
        assert_eq!(TriggerSource::default(), TriggerSource::Web);
    }

    #[test]
    fn trigger_source_display() {
        assert_eq!(format!("{}", TriggerSource::Web), "web");
        assert_eq!(format!("{}", TriggerSource::Connector), "connector");
    }

    #[test]
    fn trigger_source_from_str() {
        assert_eq!("web".parse::<TriggerSource>().unwrap(), TriggerSource::Web);
        assert_eq!("CLI".parse::<TriggerSource>().unwrap(), TriggerSource::Cli);
        assert_eq!(
            "Cron".parse::<TriggerSource>().unwrap(),
            TriggerSource::Cron
        );
        assert_eq!("api".parse::<TriggerSource>().unwrap(), TriggerSource::Api);
        assert_eq!(
            "connector".parse::<TriggerSource>().unwrap(),
            TriggerSource::Connector
        );
        assert_eq!(
            "plugin".parse::<TriggerSource>().unwrap(),
            TriggerSource::Connector
        ); // backward compat
    }

    #[test]
    fn trigger_source_serialization() {
        let sources = [
            (TriggerSource::Web, "\"web\""),
            (TriggerSource::Cli, "\"cli\""),
            (TriggerSource::Cron, "\"cron\""),
            (TriggerSource::Api, "\"api\""),
            (TriggerSource::Connector, "\"connector\""),
        ];

        for (source, expected) in sources {
            let json = serde_json::to_string(&source).unwrap();
            assert_eq!(json, expected);

            let parsed: TriggerSource = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, source);
        }
    }

    // ========================================================================
    // ExecutionStatus Tests
    // ========================================================================

    #[test]
    fn execution_status_as_str() {
        assert_eq!(ExecutionStatus::Queued.as_str(), "queued");
        assert_eq!(ExecutionStatus::Running.as_str(), "running");
        assert_eq!(ExecutionStatus::Paused.as_str(), "paused");
        assert_eq!(ExecutionStatus::Crashed.as_str(), "crashed");
        assert_eq!(ExecutionStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(ExecutionStatus::Completed.as_str(), "completed");
    }

    #[test]
    fn execution_status_is_terminal() {
        assert!(!ExecutionStatus::Queued.is_terminal());
        assert!(!ExecutionStatus::Running.is_terminal());
        assert!(!ExecutionStatus::Paused.is_terminal());
        assert!(ExecutionStatus::Crashed.is_terminal());
        assert!(ExecutionStatus::Cancelled.is_terminal());
        assert!(ExecutionStatus::Completed.is_terminal());
    }

    #[test]
    fn execution_status_is_resumable() {
        assert!(!ExecutionStatus::Queued.is_resumable());
        assert!(!ExecutionStatus::Running.is_resumable());
        assert!(ExecutionStatus::Paused.is_resumable());
        assert!(ExecutionStatus::Crashed.is_resumable());
        assert!(!ExecutionStatus::Cancelled.is_resumable());
        assert!(!ExecutionStatus::Completed.is_resumable());
    }

    #[test]
    fn execution_status_from_str() {
        assert_eq!(
            "queued".parse::<ExecutionStatus>().unwrap(),
            ExecutionStatus::Queued
        );
        assert_eq!(
            "RUNNING".parse::<ExecutionStatus>().unwrap(),
            ExecutionStatus::Running
        );
        assert_eq!(
            "completed".parse::<ExecutionStatus>().unwrap(),
            ExecutionStatus::Completed
        );
    }

    // ========================================================================
    // DelegationType Tests
    // ========================================================================

    #[test]
    fn delegation_type_as_str() {
        assert_eq!(DelegationType::Root.as_str(), "root");
        assert_eq!(DelegationType::Sequential.as_str(), "sequential");
        assert_eq!(DelegationType::Parallel.as_str(), "parallel");
    }

    #[test]
    fn delegation_type_from_str() {
        assert_eq!(
            "root".parse::<DelegationType>().unwrap(),
            DelegationType::Root
        );
        assert_eq!(
            "SEQUENTIAL".parse::<DelegationType>().unwrap(),
            DelegationType::Sequential
        );
        assert_eq!(
            "Parallel".parse::<DelegationType>().unwrap(),
            DelegationType::Parallel
        );
    }

    // ========================================================================
    // Session Tests
    // ========================================================================

    #[test]
    fn session_new_creates_running_session() {
        let session = Session::new("root-agent");

        assert!(session.id.starts_with("sess-"));
        assert_eq!(session.status, SessionStatus::Running);
        assert_eq!(session.source, TriggerSource::Web);
        assert_eq!(session.root_agent_id, "root-agent");
        assert!(session.title.is_none());
        assert!(session.started_at.is_none());
        assert!(session.completed_at.is_none());
        assert_eq!(session.total_tokens_in, 0);
        assert_eq!(session.total_tokens_out, 0);
    }

    #[test]
    fn session_new_with_source() {
        let session = Session::new_with_source("agent", TriggerSource::Cron);

        assert_eq!(session.status, SessionStatus::Running);
        assert_eq!(session.source, TriggerSource::Cron);
    }

    #[test]
    fn session_new_queued() {
        let session = Session::new_queued("agent", TriggerSource::Api);

        assert_eq!(session.status, SessionStatus::Queued);
        assert_eq!(session.source, TriggerSource::Api);
        assert!(session.started_at.is_none());
    }

    #[test]
    fn session_reserved_id_requires_canonical_uuid() {
        let id = "sess-550e8400-e29b-41d4-a716-446655440000";
        let session = Session::new_queued_with_id(id, "agent", TriggerSource::Web).unwrap();
        assert_eq!(session.id, id);
        assert!(
            Session::new_queued_with_id("sess-NOT-A-UUID", "agent", TriggerSource::Web).is_err()
        );
    }

    #[test]
    fn session_total_tokens() {
        let mut session = Session::new("agent");
        session.total_tokens_in = 1000;
        session.total_tokens_out = 500;

        assert_eq!(session.total_tokens(), 1500);
    }

    #[test]
    fn session_id_is_unique() {
        let session1 = Session::new("agent");
        let session2 = Session::new("agent");

        assert_ne!(session1.id, session2.id);
    }

    // ========================================================================
    // AgentExecution Tests
    // ========================================================================

    #[test]
    fn agent_execution_new_root() {
        let exec = AgentExecution::new_root("sess-123", "root-agent");

        assert!(exec.id.starts_with("exec-"));
        assert_eq!(exec.session_id, "sess-123");
        assert_eq!(exec.agent_id, "root-agent");
        assert!(exec.parent_execution_id.is_none());
        assert_eq!(exec.delegation_type, DelegationType::Root);
        assert!(exec.task.is_none());
        assert_eq!(exec.status, ExecutionStatus::Queued);
        assert!(exec.started_at.is_none());
        assert_eq!(exec.tokens_in, 0);
        assert_eq!(exec.tokens_out, 0);
    }

    #[test]
    fn root_execution_reserved_id_requires_canonical_uuid() {
        let id = "exec-550e8400-e29b-41d4-a716-446655440000";
        let execution = AgentExecution::new_root_with_id(id, "sess-1", "agent").unwrap();
        assert_eq!(execution.id, id);
        assert!(AgentExecution::new_root_with_id("exec-NOT-A-UUID", "sess-1", "agent").is_err());
    }

    #[test]
    fn agent_execution_new_delegated() {
        let exec = AgentExecution::new_delegated(
            "sess-123",
            "researcher",
            "exec-parent",
            DelegationType::Sequential,
            "Research AI topics",
        );

        assert!(exec.id.starts_with("exec-"));
        assert_eq!(exec.session_id, "sess-123");
        assert_eq!(exec.agent_id, "researcher");
        assert_eq!(exec.parent_execution_id, Some("exec-parent".to_string()));
        assert_eq!(exec.delegation_type, DelegationType::Sequential);
        assert_eq!(exec.task, Some("Research AI topics".to_string()));
        assert_eq!(exec.status, ExecutionStatus::Queued);
    }

    #[test]
    fn agent_execution_is_root() {
        let root = AgentExecution::new_root("sess", "agent");
        let delegated = AgentExecution::new_delegated(
            "sess",
            "sub",
            "parent",
            DelegationType::Sequential,
            "task",
        );

        assert!(root.is_root());
        assert!(!delegated.is_root());
    }

    #[test]
    fn agent_execution_total_tokens() {
        let mut exec = AgentExecution::new_root("sess", "agent");
        exec.tokens_in = 500;
        exec.tokens_out = 250;

        assert_eq!(exec.total_tokens(), 750);
    }

    #[test]
    fn agent_execution_id_is_unique() {
        let exec1 = AgentExecution::new_root("sess", "agent");
        let exec2 = AgentExecution::new_root("sess", "agent");

        assert_ne!(exec1.id, exec2.id);
    }

    // ========================================================================
    // Checkpoint Tests
    // ========================================================================

    #[test]
    fn checkpoint_new() {
        let checkpoint = Checkpoint::new(5, "msg-123");

        assert_eq!(checkpoint.llm_turn, 5);
        assert_eq!(checkpoint.last_message_id, "msg-123");
        assert!(checkpoint.pending_tool_calls.is_empty());
        assert!(checkpoint.child_executions.is_empty());
    }

    // ========================================================================
    // SessionFilter Tests
    // ========================================================================

    #[test]
    fn session_filter_default() {
        let filter = SessionFilter::default();

        assert!(filter.status.is_none());
        assert!(filter.root_agent_id.is_none());
        assert!(filter.limit.is_none());
        assert!(filter.offset.is_none());
    }

    // ========================================================================
    // ExecutionFilter Tests
    // ========================================================================

    #[test]
    fn execution_filter_default() {
        let filter = ExecutionFilter::default();

        assert!(filter.session_id.is_none());
        assert!(filter.agent_id.is_none());
        assert!(filter.status.is_none());
        assert!(filter.limit.is_none());
    }

    // ========================================================================
    // ApiError Tests
    // ========================================================================

    #[test]
    fn api_error_display() {
        let not_found = ApiError::NotFound("Session not found".to_string());
        let db_error = ApiError::Database("Connection failed".to_string());
        let bad_request = ApiError::BadRequest("Invalid input".to_string());
        let invalid_transition =
            ApiError::InvalidTransition("Cannot pause completed session".to_string());

        assert!(not_found.to_string().contains("Session not found"));
        assert!(db_error.to_string().contains("Connection failed"));
        assert!(bad_request.to_string().contains("Invalid input"));
        assert!(invalid_transition.to_string().contains("Cannot pause"));
    }

    // ========================================================================
    // Serialization Round-Trip Tests
    // ========================================================================

    #[test]
    fn session_serialization_roundtrip() {
        let session = Session::new_with_source("agent", TriggerSource::Connector);

        let json = serde_json::to_string(&session).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, session.id);
        assert_eq!(parsed.status, session.status);
        assert_eq!(parsed.source, session.source);
        assert_eq!(parsed.root_agent_id, session.root_agent_id);
    }

    #[test]
    fn agent_execution_serialization_roundtrip() {
        let exec = AgentExecution::new_delegated(
            "sess-123",
            "researcher",
            "exec-parent",
            DelegationType::Parallel,
            "Research task",
        );

        let json = serde_json::to_string(&exec).unwrap();
        let parsed: AgentExecution = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, exec.id);
        assert_eq!(parsed.session_id, exec.session_id);
        assert_eq!(parsed.agent_id, exec.agent_id);
        assert_eq!(parsed.parent_execution_id, exec.parent_execution_id);
        assert_eq!(parsed.delegation_type, exec.delegation_type);
        assert_eq!(parsed.task, exec.task);
    }

    #[test]
    fn new_delegated_with_id_uses_provided_id() {
        let exec = AgentExecution::new_delegated_with_id(
            "exec-custom-uuid",
            "session-1",
            "writer-agent",
            "exec-parent",
            DelegationType::Sequential,
            "do work",
        );
        assert_eq!(exec.id, "exec-custom-uuid");
        assert_eq!(exec.agent_id, "writer-agent");
        assert_eq!(exec.status, ExecutionStatus::Queued);
        assert_eq!(exec.parent_execution_id, Some("exec-parent".to_string()));
    }

    #[test]
    fn session_plan_input_rejects_unknown_fields_and_overlong_content() {
        let unknown_field = SessionPlanInput::from_update(
            serde_json::json!([{
                "step": "Inspect the setup",
                "status": "pending",
                "untrusted": true }]),
            None,
        );
        assert_eq!(unknown_field, Err(SessionPlanRejection::InvalidStructure));

        let long_step = "x".repeat(MAX_SESSION_PLAN_STEP_BYTES + 1);
        let overlong = SessionPlanInput::from_update(
            serde_json::json!([{"step": long_step, "status": "pending"}]),
            None,
        );
        assert_eq!(overlong, Err(SessionPlanRejection::StepTooLong));

        let too_many_steps = serde_json::Value::Array(
            (0..=MAX_SESSION_PLAN_STEPS)
                .map(|_| serde_json::json!({"step": "bounded", "status": "pending"}))
                .collect(),
        );
        assert_eq!(
            SessionPlanInput::preflight_update(&too_many_steps, None),
            Err(SessionPlanRejection::TooManySteps)
        );

        let escape_heavy_step = "\0".repeat(MAX_SESSION_PLAN_STEP_BYTES);
        let escape_heavy_plan = serde_json::Value::Array(
            (0..MAX_SESSION_PLAN_STEPS)
                .map(
                    |_| serde_json::json!({"step": escape_heavy_step.clone(), "status": "pending"}),
                )
                .collect(),
        );
        assert_eq!(
            SessionPlanInput::preflight_update(&escape_heavy_plan, None),
            Err(SessionPlanRejection::PayloadTooLarge)
        );
    }
}
