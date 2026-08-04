//! Durable executable-work domain and persistence boundary.
//!
//! The store is authoritative. Transport adapters may announce an envelope,
//! but cannot claim, complete, retry, or authorize it.

use crate::StateDbProvider;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

pub const WORK_ENVELOPE_VERSION: u16 = 1;
pub const MAX_ROUTING_BYTES: usize = 128;
pub const MAX_PROVENANCE_ID_BYTES: usize = 128;
pub const MAX_PAYLOAD_BYTES: usize = 65_536;
pub const MAX_CORRELATION_BYTES: usize = 256;
pub const MAX_DEDUPE_BYTES: usize = 256;
pub const MIN_ATTEMPTS: u8 = 1;
pub const MAX_ATTEMPTS: u8 = 20;
pub const MIN_LEASE_SECONDS: u64 = 1;
pub const MAX_LEASE_SECONDS: u64 = 300;
pub const MAX_FAILURE_CODE_BYTES: usize = 64;

/// Safe, normalized validation failures. No input text is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum WorkValidationError {
    #[error("unsupported_version")]
    UnsupportedVersion,
    #[error("invalid_kind")]
    InvalidKind,
    #[error("invalid_source")]
    InvalidSource,
    #[error("invalid_target")]
    InvalidTarget,
    #[error("invalid_provenance")]
    InvalidProvenance,
    #[error("payload_too_large")]
    PayloadTooLarge,
    #[error("invalid_correlation")]
    InvalidCorrelation,
    #[error("invalid_dedupe_key")]
    InvalidDedupeKey,
    #[error("invalid_attempt_limit")]
    InvalidAttemptLimit,
    #[error("invalid_lease_duration")]
    InvalidLeaseDuration,
    #[error("invalid_timestamp")]
    InvalidTimestamp,
    #[error("invalid_lease_owner")]
    InvalidLeaseOwner,
}

/// Closed policy rejection reasons. Routing labels never grant authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum WorkPolicyError {
    #[error("kind_not_allowed")]
    KindNotAllowed,
    #[error("target_not_allowed")]
    TargetNotAllowed,
    #[error("payload_invalid")]
    PayloadInvalid,
    #[error("provenance_rejected")]
    ProvenanceRejected,
}

/// Queue errors intentionally expose only normalized codes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkError {
    #[error("invalid_envelope:{0}")]
    InvalidEnvelope(WorkValidationError),
    #[error("policy_rejected:{0}")]
    Policy(WorkPolicyError),
    #[error("storage_unavailable")]
    StorageUnavailable,
    #[error("stored_data_invalid")]
    StoredDataInvalid,
    #[error("work_not_found")]
    NotFound,
    #[error("stale_lease")]
    StaleLease,
}

/// Host-derived caller identity carried for audit and future re-authorization.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkProvenance {
    node_id: String,
    actor_id: String,
    session_id: String,
    execution_id: String,
}

impl WorkProvenance {
    fn new(
        node_id: impl Into<String>,
        actor_id: impl Into<String>,
        session_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            actor_id: actor_id.into(),
            session_id: session_id.into(),
            execution_id: execution_id.into(),
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
}

/// Host-attached identity context returned by a trusted [`WorkPolicy`].
///
/// Producers cannot place source or provenance on [`WorkDraft`]; the queue's
/// configured policy derives this value from authenticated host context.
#[derive(Clone)]
pub struct WorkAuthorization {
    source: String,
    provenance: WorkProvenance,
}

impl WorkAuthorization {
    pub fn new(
        source: impl Into<String>,
        node_id: impl Into<String>,
        actor_id: impl Into<String>,
        session_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            provenance: WorkProvenance::new(node_id, actor_id, session_id, execution_id),
        }
    }
}

/// Host-side request that cannot be persisted until a [`WorkPolicy`] approves it.
#[derive(Clone)]
pub struct WorkDraft {
    version: u16,
    kind: String,
    target: String,
    payload: Value,
    correlation_id: Option<String>,
    dedupe_key: Option<String>,
    priority: i16,
    max_attempts: u8,
}

impl WorkDraft {
    pub fn new(kind: impl Into<String>, target: impl Into<String>, payload: Value) -> Self {
        Self {
            version: WORK_ENVELOPE_VERSION,
            kind: kind.into(),
            target: target.into(),
            payload,
            correlation_id: None,
            dedupe_key: None,
            priority: 0,
            max_attempts: 5,
        }
    }

    pub fn with_version(mut self, version: u16) -> Self {
        self.version = version;
        self
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn with_dedupe_key(mut self, dedupe_key: impl Into<String>) -> Self {
        self.dedupe_key = Some(dedupe_key.into());
        self
    }

    pub fn with_priority(mut self, priority: i16) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_max_attempts(mut self, max_attempts: u8) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }
}

impl fmt::Debug for WorkDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkDraft")
            .field("version", &self.version)
            .field("kind", &self.kind)
            .field("target", &self.target)
            .field("priority", &self.priority)
            .field("max_attempts", &self.max_attempts)
            .finish_non_exhaustive()
    }
}

/// Host policy validates work and attaches trusted caller provenance.
pub trait WorkPolicy: Send + Sync {
    fn authorize(&self, draft: &WorkDraft) -> Result<WorkAuthorization, WorkPolicyError>;
}

/// Validated, versioned transport envelope. It has no public unchecked constructor.
#[derive(Clone, PartialEq)]
pub struct WorkEnvelope {
    id: String,
    version: u16,
    kind: String,
    source: String,
    target: String,
    payload: Value,
    provenance: WorkProvenance,
    correlation_id: Option<String>,
    dedupe_key: Option<String>,
    priority: i16,
    max_attempts: u8,
    created_at: DateTime<Utc>,
}

impl WorkEnvelope {
    pub fn authorize(
        draft: WorkDraft,
        policy: &dyn WorkPolicy,
        now: DateTime<Utc>,
    ) -> Result<Self, WorkError> {
        validate_draft(&draft)?;
        let authorization = policy.authorize(&draft).map_err(WorkError::Policy)?;
        validate_authorization(&authorization)?;

        Ok(Self {
            id: format!("work-{}", uuid::Uuid::new_v4()),
            version: draft.version,
            kind: draft.kind,
            source: authorization.source,
            target: draft.target,
            payload: draft.payload,
            provenance: authorization.provenance,
            correlation_id: draft.correlation_id,
            dedupe_key: draft.dedupe_key,
            priority: draft.priority,
            max_attempts: draft.max_attempts,
            created_at: now,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn provenance(&self) -> &WorkProvenance {
        &self.provenance
    }

    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    pub fn dedupe_key(&self) -> Option<&str> {
        self.dedupe_key.as_deref()
    }

    pub fn priority(&self) -> i16 {
        self.priority
    }

    pub fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

impl fmt::Debug for WorkEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkEnvelope")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("kind", &self.kind)
            .field("source", &self.source)
            .field("target", &self.target)
            .field("priority", &self.priority)
            .field("max_attempts", &self.max_attempts)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

fn validate_draft(draft: &WorkDraft) -> Result<(), WorkError> {
    if draft.version != WORK_ENVELOPE_VERSION {
        return Err(WorkError::InvalidEnvelope(
            WorkValidationError::UnsupportedVersion,
        ));
    }
    validate_required(&draft.kind, WorkValidationError::InvalidKind)?;
    validate_required(&draft.target, WorkValidationError::InvalidTarget)?;

    let payload_len = serde_json::to_vec(&draft.payload)
        .map_err(|_| WorkError::InvalidEnvelope(WorkValidationError::PayloadTooLarge))?
        .len();
    if payload_len > MAX_PAYLOAD_BYTES {
        return Err(WorkError::InvalidEnvelope(
            WorkValidationError::PayloadTooLarge,
        ));
    }
    validate_optional(
        draft.correlation_id.as_deref(),
        MAX_CORRELATION_BYTES,
        WorkValidationError::InvalidCorrelation,
    )?;
    validate_optional(
        draft.dedupe_key.as_deref(),
        MAX_DEDUPE_BYTES,
        WorkValidationError::InvalidDedupeKey,
    )?;
    if !(MIN_ATTEMPTS..=MAX_ATTEMPTS).contains(&draft.max_attempts) {
        return Err(WorkError::InvalidEnvelope(
            WorkValidationError::InvalidAttemptLimit,
        ));
    }
    Ok(())
}

fn validate_authorization(authorization: &WorkAuthorization) -> Result<(), WorkError> {
    validate_required(&authorization.source, WorkValidationError::InvalidSource)?;
    for value in [
        authorization.provenance.node_id(),
        authorization.provenance.actor_id(),
        authorization.provenance.session_id(),
        authorization.provenance.execution_id(),
    ] {
        if value.is_empty() || value.len() > MAX_PROVENANCE_ID_BYTES {
            return Err(WorkError::InvalidEnvelope(
                WorkValidationError::InvalidProvenance,
            ));
        }
    }
    Ok(())
}

fn validate_required(value: &str, error: WorkValidationError) -> Result<(), WorkError> {
    if value.is_empty() || value.len() > MAX_ROUTING_BYTES {
        return Err(WorkError::InvalidEnvelope(error));
    }
    Ok(())
}

fn validate_optional(
    value: Option<&str>,
    max: usize,
    error: WorkValidationError,
) -> Result<(), WorkError> {
    if value.is_some_and(|value| value.is_empty() || value.len() > max) {
        return Err(WorkError::InvalidEnvelope(error));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Pending,
    Leased,
    Completed,
    DeadLetter,
}

impl WorkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Completed => "completed",
            Self::DeadLetter => "dead_letter",
        }
    }

    fn parse(value: &str) -> Result<Self, WorkError> {
        match value {
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
            "completed" => Ok(Self::Completed),
            "dead_letter" => Ok(Self::DeadLetter),
            _ => Err(WorkError::StoredDataInvalid),
        }
    }
}

/// Closed failure vocabulary; callers cannot persist raw error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkFailureCode {
    HandlerUnavailable,
    HandlerRejected,
    Timeout,
    InvalidPayload,
    AttemptsExhausted,
    IntegrityViolation,
    Internal,
}

impl WorkFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HandlerUnavailable => "handler_unavailable",
            Self::HandlerRejected => "handler_rejected",
            Self::Timeout => "timeout",
            Self::InvalidPayload => "invalid_payload",
            Self::AttemptsExhausted => "attempts_exhausted",
            Self::IntegrityViolation => "integrity_violation",
            Self::Internal => "internal",
        }
    }

    fn parse(value: &str) -> Result<Self, WorkError> {
        match value {
            "handler_unavailable" => Ok(Self::HandlerUnavailable),
            "handler_rejected" => Ok(Self::HandlerRejected),
            "timeout" => Ok(Self::Timeout),
            "invalid_payload" => Ok(Self::InvalidPayload),
            "attempts_exhausted" => Ok(Self::AttemptsExhausted),
            "integrity_violation" => Ok(Self::IntegrityViolation),
            "internal" => Ok(Self::Internal),
            _ => Err(WorkError::StoredDataInvalid),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct WorkItem {
    envelope: WorkEnvelope,
    status: WorkStatus,
    attempts: u8,
    available_at: DateTime<Utc>,
    lease_owner: Option<String>,
    lease_token: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    last_failure_code: Option<WorkFailureCode>,
    updated_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

impl WorkItem {
    pub fn envelope(&self) -> &WorkEnvelope {
        &self.envelope
    }

    pub fn status(&self) -> WorkStatus {
        self.status
    }

    pub fn attempts(&self) -> u8 {
        self.attempts
    }

    pub fn available_at(&self) -> DateTime<Utc> {
        self.available_at
    }

    pub fn lease_owner(&self) -> Option<&str> {
        self.lease_owner.as_deref()
    }

    pub fn lease_token(&self) -> Option<&str> {
        self.lease_token.as_deref()
    }

    pub fn lease_expires_at(&self) -> Option<DateTime<Utc>> {
        self.lease_expires_at
    }

    pub fn last_failure_code(&self) -> Option<WorkFailureCode> {
        self.last_failure_code
    }

    pub fn completed_at(&self) -> Option<DateTime<Utc>> {
        self.completed_at
    }
}

impl fmt::Debug for WorkItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkItem")
            .field("envelope", &self.envelope)
            .field("status", &self.status)
            .field("attempts", &self.attempts)
            .field("available_at", &self.available_at)
            .field("lease_owner", &self.lease_owner)
            .field("lease_expires_at", &self.lease_expires_at)
            .field("last_failure_code", &self.last_failure_code)
            .field("updated_at", &self.updated_at)
            .field("completed_at", &self.completed_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnqueueOutcome {
    item: WorkItem,
    inserted: bool,
}

impl EnqueueOutcome {
    pub fn item(&self) -> &WorkItem {
        &self.item
    }

    pub fn inserted(&self) -> bool {
        self.inserted
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryOutcome {
    pub requeued: usize,
    pub dead_lettered: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryDisposition {
    Requeued,
    DeadLetter(WorkFailureCode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkTransitionDiagnostic {
    work_id: String,
    kind: String,
    attempt: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RecoveryTransition {
    outcome: RecoveryOutcome,
    diagnostics: Vec<(WorkTransitionDiagnostic, RecoveryDisposition)>,
}

pub trait WorkStore: Send + Sync {
    fn enqueue(&self, envelope: &WorkEnvelope) -> Result<EnqueueOutcome, WorkError>;
    fn get(&self, id: &str) -> Result<Option<WorkItem>, WorkError>;
    fn claim_next(
        &self,
        target: &str,
        owner: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
    ) -> Result<Option<WorkItem>, WorkError>;
    fn renew_lease(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
    ) -> Result<DateTime<Utc>, WorkError>;
    fn complete(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<(), WorkError>;
    fn fail(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
        code: WorkFailureCode,
        retryable: bool,
    ) -> Result<WorkStatus, WorkError>;
    fn recover_expired(
        &self,
        target: &str,
        now: DateTime<Utc>,
    ) -> Result<RecoveryOutcome, WorkError>;
}

pub struct SqliteWorkStore<D: StateDbProvider> {
    db: Arc<D>,
}

impl<D: StateDbProvider> SqliteWorkStore<D> {
    pub fn new(db: Arc<D>) -> Self {
        Self { db }
    }
}

impl<D: StateDbProvider> WorkStore for SqliteWorkStore<D> {
    fn enqueue(&self, envelope: &WorkEnvelope) -> Result<EnqueueOutcome, WorkError> {
        let payload_json = serde_json::to_string(envelope.payload())
            .map_err(|_| WorkError::InvalidEnvelope(WorkValidationError::PayloadTooLarge))?;
        let timestamp = format_timestamp(envelope.created_at());
        let stored = self
            .db
            .with_connection(|conn| {
                let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO durable_work_items (
                        id, envelope_version, kind, source, target, payload_json,
                        provenance_node_id, provenance_actor_id,
                        provenance_session_id, provenance_execution_id,
                        correlation_id, dedupe_key, priority, status, attempts,
                        max_attempts, available_at, created_at, updated_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                        ?13, 'pending', 0, ?14, ?15, ?15, ?15
                    )",
                    params![
                        envelope.id(),
                        i64::from(envelope.version()),
                        envelope.kind(),
                        envelope.source(),
                        envelope.target(),
                        payload_json,
                        envelope.provenance().node_id(),
                        envelope.provenance().actor_id(),
                        envelope.provenance().session_id(),
                        envelope.provenance().execution_id(),
                        envelope.correlation_id(),
                        envelope.dedupe_key(),
                        i64::from(envelope.priority()),
                        i64::from(envelope.max_attempts()),
                        timestamp,
                    ],
                )? == 1;

                let row = if inserted {
                    query_stored_by_id(&tx, envelope.id())?
                } else if let Some(dedupe_key) = envelope.dedupe_key() {
                    query_stored_by_dedupe(&tx, envelope.source(), dedupe_key)?
                } else {
                    None
                };
                tx.commit()?;
                Ok((row, inserted))
            })
            .map_err(|_| WorkError::StorageUnavailable)?;

        let (row, inserted) = stored;
        let item = row.ok_or(WorkError::StorageUnavailable)?.try_into_item()?;
        tracing::info!(
            work_id = %item.envelope().id(),
            kind = %item.envelope().kind(),
            target = %item.envelope().target(),
            inserted,
            transition = "enqueue",
            "durable work transition"
        );
        Ok(EnqueueOutcome { item, inserted })
    }

    fn get(&self, id: &str) -> Result<Option<WorkItem>, WorkError> {
        let row = self
            .db
            .with_connection(|conn| query_stored_by_id(conn, id))
            .map_err(|_| WorkError::StorageUnavailable)?;
        row.map(StoredWorkRow::try_into_item).transpose()
    }

    fn claim_next(
        &self,
        target: &str,
        owner: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
    ) -> Result<Option<WorkItem>, WorkError> {
        validate_routing_value(target, WorkValidationError::InvalidTarget)?;
        let lease_expires_at = validate_lease(owner, now, lease_duration)?;
        let now_text = format_timestamp(now);
        let expiry_text = format_timestamp(lease_expires_at);
        let token = format!("lease-{}", uuid::Uuid::new_v4());

        let (row, recovery, invalid_candidate) = self
            .db
            .with_connection(|conn| {
                let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                let recovery = recover_expired_in_tx(&tx, target, now)?;
                let id: Option<String> = tx
                    .query_row(
                        "SELECT id FROM durable_work_items
                         WHERE target = ?1 AND status = 'pending'
                           AND available_at <= ?2 AND attempts < max_attempts
                         ORDER BY priority DESC, available_at ASC, created_at ASC, id ASC
                         LIMIT 1",
                        params![target, now_text],
                        |row| row.get(0),
                    )
                    .optional()?;

                let (row, invalid_candidate) = if let Some(id) = id {
                    let candidate = query_stored_by_id(&tx, &id)?;
                    let diagnostic = candidate.as_ref().map(StoredWorkRow::transition_diagnostic);
                    let candidate_is_valid =
                        candidate.is_some_and(|candidate| candidate.try_into_item().is_ok());
                    if candidate_is_valid {
                        let changed = tx.execute(
                            "UPDATE durable_work_items
                             SET status = 'leased', attempts = attempts + 1,
                                 lease_owner = ?2, lease_token = ?3,
                                 lease_expires_at = ?4, updated_at = ?5
                             WHERE id = ?1 AND status = 'pending' AND available_at <= ?5",
                            params![id, owner, token, expiry_text, now_text],
                        )?;
                        let row = if changed == 1 {
                            query_stored_by_id(&tx, &id)?
                        } else {
                            None
                        };
                        (row, None)
                    } else {
                        tx.execute(
                            "UPDATE durable_work_items
                             SET status = 'dead_letter', lease_owner = NULL,
                                 lease_token = NULL, lease_expires_at = NULL,
                                 last_failure_code = 'integrity_violation',
                                 updated_at = ?2, completed_at = ?2
                             WHERE id = ?1 AND status = 'pending'",
                            params![id, now_text],
                        )?;
                        (None, diagnostic)
                    }
                } else {
                    (None, None)
                };
                tx.commit()?;
                Ok((row, recovery, invalid_candidate))
            })
            .map_err(|_| WorkError::StorageUnavailable)?;

        trace_recovery(target, &recovery);
        if let Some(diagnostic) = invalid_candidate {
            tracing::warn!(
                work_id = %diagnostic.work_id,
                kind = %diagnostic.kind,
                target = %target,
                attempt = diagnostic.attempt,
                transition = "dead_letter",
                reason_code = WorkFailureCode::IntegrityViolation.as_str(),
                "durable work transition"
            );
            return Err(WorkError::StoredDataInvalid);
        }
        let item = row.map(StoredWorkRow::try_into_item).transpose()?;
        if let Some(item) = &item {
            tracing::info!(
                work_id = %item.envelope().id(),
                kind = %item.envelope().kind(),
                target = %item.envelope().target(),
                attempt = item.attempts(),
                transition = "claim",
                "durable work transition"
            );
        }
        Ok(item)
    }

    fn renew_lease(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
    ) -> Result<DateTime<Utc>, WorkError> {
        let expires_at = validate_lease(owner, now, lease_duration)?;
        let now_text = format_timestamp(now);
        let expiry_text = format_timestamp(expires_at);
        let transition = self
            .db
            .with_connection(|conn| {
                conn.query_row(
                    "UPDATE durable_work_items
                     SET lease_expires_at = ?4, updated_at = ?5
                     WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2
                       AND lease_token = ?3 AND lease_expires_at > ?5
                     RETURNING kind, target, attempts",
                    params![id, owner, token, expiry_text, now_text],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()
            })
            .map_err(|_| WorkError::StorageUnavailable)?;
        let Some((kind, target, attempt)) = transition else {
            return Err(WorkError::StaleLease);
        };
        tracing::info!(
            work_id = %id,
            kind = %kind,
            target = %target,
            attempt,
            transition = "renew",
            "durable work transition"
        );
        Ok(expires_at)
    }

    fn complete(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<(), WorkError> {
        let now_text = format_timestamp(now);
        let transition = self
            .db
            .with_connection(|conn| {
                conn.query_row(
                    "UPDATE durable_work_items
                     SET status = 'completed', lease_owner = NULL,
                         lease_token = NULL, lease_expires_at = NULL,
                         updated_at = ?4, completed_at = ?4
                     WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2
                       AND lease_token = ?3 AND lease_expires_at > ?4
                     RETURNING kind, target, attempts",
                    params![id, owner, token, now_text],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()
            })
            .map_err(|_| WorkError::StorageUnavailable)?;
        let Some((kind, target, attempt)) = transition else {
            return Err(WorkError::StaleLease);
        };
        tracing::info!(
            work_id = %id,
            kind = %kind,
            target = %target,
            attempt,
            transition = "complete",
            "durable work transition"
        );
        Ok(())
    }

    fn fail(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
        code: WorkFailureCode,
        retryable: bool,
    ) -> Result<WorkStatus, WorkError> {
        let now_text = format_timestamp(now);
        let result = self
            .db
            .with_connection(|conn| {
                let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                let budget: Option<(i64, i64, String, String)> = tx
                    .query_row(
                        "SELECT attempts, max_attempts, kind, target FROM durable_work_items
                         WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2
                           AND lease_token = ?3 AND lease_expires_at > ?4",
                        params![id, owner, token, now_text],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?;
                let Some((attempts, max_attempts, kind, target)) = budget else {
                    tx.commit()?;
                    return Ok(None);
                };

                let should_retry = retryable && attempts < max_attempts;
                let status = if should_retry {
                    WorkStatus::Pending
                } else {
                    WorkStatus::DeadLetter
                };
                let available_at = if should_retry {
                    let delay = retry_delay(attempts as u8);
                    format_timestamp(now + TimeDelta::seconds(delay as i64))
                } else {
                    now_text.clone()
                };
                let completed_at = (!should_retry).then_some(now_text.clone());
                let changed = tx.execute(
                    "UPDATE durable_work_items
                     SET status = ?5, available_at = ?6,
                         lease_owner = NULL, lease_token = NULL,
                         lease_expires_at = NULL, last_failure_code = ?7,
                         updated_at = ?4, completed_at = ?8
                     WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2
                       AND lease_token = ?3 AND lease_expires_at > ?4",
                    params![
                        id,
                        owner,
                        token,
                        now_text,
                        status.as_str(),
                        available_at,
                        code.as_str(),
                        completed_at,
                    ],
                )?;
                tx.commit()?;
                Ok((changed == 1).then_some((status, kind, target, attempts)))
            })
            .map_err(|_| WorkError::StorageUnavailable)?;

        let (status, kind, target, attempt) = result.ok_or(WorkError::StaleLease)?;
        tracing::info!(
            work_id = %id,
            kind = %kind,
            target = %target,
            attempt,
            transition = if status == WorkStatus::Pending { "retry" } else { "dead_letter" },
            reason_code = code.as_str(),
            "durable work transition"
        );
        Ok(status)
    }

    fn recover_expired(
        &self,
        target: &str,
        now: DateTime<Utc>,
    ) -> Result<RecoveryOutcome, WorkError> {
        validate_routing_value(target, WorkValidationError::InvalidTarget)?;
        let recovery = self
            .db
            .with_connection(|conn| {
                let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                let recovery = recover_expired_in_tx(&tx, target, now)?;
                tx.commit()?;
                Ok(recovery)
            })
            .map_err(|_| WorkError::StorageUnavailable)?;
        trace_recovery(target, &recovery);
        Ok(recovery.outcome)
    }
}

fn validate_routing_value(value: &str, error: WorkValidationError) -> Result<(), WorkError> {
    if value.is_empty() || value.len() > MAX_ROUTING_BYTES {
        return Err(WorkError::InvalidEnvelope(error));
    }
    Ok(())
}

fn validate_lease(
    owner: &str,
    now: DateTime<Utc>,
    duration: Duration,
) -> Result<DateTime<Utc>, WorkError> {
    if owner.is_empty() || owner.len() > MAX_ROUTING_BYTES {
        return Err(WorkError::InvalidEnvelope(
            WorkValidationError::InvalidLeaseOwner,
        ));
    }
    if duration.subsec_nanos() != 0
        || !(MIN_LEASE_SECONDS..=MAX_LEASE_SECONDS).contains(&duration.as_secs())
    {
        return Err(WorkError::InvalidEnvelope(
            WorkValidationError::InvalidLeaseDuration,
        ));
    }
    let delta = TimeDelta::from_std(duration)
        .map_err(|_| WorkError::InvalidEnvelope(WorkValidationError::InvalidLeaseDuration))?;
    now.checked_add_signed(delta)
        .ok_or(WorkError::InvalidEnvelope(
            WorkValidationError::InvalidTimestamp,
        ))
}

fn retry_delay(attempts: u8) -> u64 {
    let exponent = u32::from(attempts.saturating_sub(1)).min(20);
    1_u64.checked_shl(exponent).unwrap_or(u64::MAX).min(300)
}

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, WorkError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| WorkError::StoredDataInvalid)
}

fn recover_expired_in_tx(
    tx: &Transaction<'_>,
    target: &str,
    now: DateTime<Utc>,
) -> rusqlite::Result<RecoveryTransition> {
    let now_text = format_timestamp(now);
    let leased_ids = {
        let mut statement = tx.prepare(
            "SELECT id FROM durable_work_items
             WHERE target = ?1 AND status = 'leased'",
        )?;
        let ids = statement
            .query_map([target], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids
    };

    let mut transition = RecoveryTransition::default();
    for id in leased_ids {
        let Some(stored) = query_stored_by_id(tx, &id)? else {
            continue;
        };
        let diagnostic = stored.transition_diagnostic();
        let Ok(item) = stored.try_into_item() else {
            let changed = tx.execute(
                "UPDATE durable_work_items
                 SET status = 'dead_letter', lease_owner = NULL, lease_token = NULL,
                     lease_expires_at = NULL, last_failure_code = 'integrity_violation',
                     updated_at = ?2, completed_at = ?2
                 WHERE id = ?1 AND status = 'leased'",
                params![id, now_text],
            )?;
            transition.outcome.dead_lettered += changed;
            if changed == 1 {
                transition.diagnostics.push((
                    diagnostic,
                    RecoveryDisposition::DeadLetter(WorkFailureCode::IntegrityViolation),
                ));
            }
            continue;
        };
        if item.lease_expires_at().is_some_and(|expiry| expiry > now) {
            continue;
        }

        if item.attempts() >= item.envelope().max_attempts() {
            let changed = tx.execute(
                "UPDATE durable_work_items
                 SET status = 'dead_letter', lease_owner = NULL, lease_token = NULL,
                     lease_expires_at = NULL, last_failure_code = 'attempts_exhausted',
                     updated_at = ?2, completed_at = ?2
                 WHERE id = ?1 AND status = 'leased'",
                params![id, now_text],
            )?;
            transition.outcome.dead_lettered += changed;
            if changed == 1 {
                transition.diagnostics.push((
                    diagnostic,
                    RecoveryDisposition::DeadLetter(WorkFailureCode::AttemptsExhausted),
                ));
            }
        } else {
            let changed = tx.execute(
                "UPDATE durable_work_items
                 SET status = 'pending', lease_owner = NULL, lease_token = NULL,
                     lease_expires_at = NULL, available_at = ?2, updated_at = ?2
                 WHERE id = ?1 AND status = 'leased'",
                params![id, now_text],
            )?;
            transition.outcome.requeued += changed;
            if changed == 1 {
                transition
                    .diagnostics
                    .push((diagnostic, RecoveryDisposition::Requeued));
            }
        }
    }
    Ok(transition)
}

fn trace_recovery(target: &str, recovery: &RecoveryTransition) {
    for (diagnostic, disposition) in &recovery.diagnostics {
        match disposition {
            RecoveryDisposition::Requeued => tracing::info!(
                work_id = %diagnostic.work_id,
                kind = %diagnostic.kind,
                target = %target,
                attempt = diagnostic.attempt,
                transition = "lease_recovery",
                reason_code = "lease_expired",
                "durable work transition"
            ),
            RecoveryDisposition::DeadLetter(reason) => tracing::warn!(
                work_id = %diagnostic.work_id,
                kind = %diagnostic.kind,
                target = %target,
                attempt = diagnostic.attempt,
                transition = "dead_letter",
                reason_code = reason.as_str(),
                "durable work transition"
            ),
        }
    }
}

const WORK_SELECT: &str = "SELECT
    id, envelope_version, kind, source, target, payload_json,
    provenance_node_id, provenance_actor_id, provenance_session_id,
    provenance_execution_id, correlation_id, dedupe_key, priority, status,
    attempts, max_attempts, available_at, lease_owner, lease_token,
    lease_expires_at, last_failure_code, created_at, updated_at, completed_at
    FROM durable_work_items";

fn query_stored_by_id(
    conn: &rusqlite::Connection,
    id: &str,
) -> rusqlite::Result<Option<StoredWorkRow>> {
    conn.query_row(
        &format!("{WORK_SELECT} WHERE id = ?1"),
        [id],
        StoredWorkRow::from_row,
    )
    .optional()
}

fn query_stored_by_dedupe(
    conn: &rusqlite::Connection,
    source: &str,
    dedupe_key: &str,
) -> rusqlite::Result<Option<StoredWorkRow>> {
    conn.query_row(
        &format!("{WORK_SELECT} WHERE source = ?1 AND dedupe_key = ?2"),
        params![source, dedupe_key],
        StoredWorkRow::from_row,
    )
    .optional()
}

struct StoredWorkRow {
    id: String,
    envelope_version: i64,
    kind: String,
    source: String,
    target: String,
    payload_json: String,
    provenance_node_id: String,
    provenance_actor_id: String,
    provenance_session_id: String,
    provenance_execution_id: String,
    correlation_id: Option<String>,
    dedupe_key: Option<String>,
    priority: i64,
    status: String,
    attempts: i64,
    max_attempts: i64,
    available_at: String,
    lease_owner: Option<String>,
    lease_token: Option<String>,
    lease_expires_at: Option<String>,
    last_failure_code: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl StoredWorkRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            envelope_version: row.get(1)?,
            kind: row.get(2)?,
            source: row.get(3)?,
            target: row.get(4)?,
            payload_json: row.get(5)?,
            provenance_node_id: row.get(6)?,
            provenance_actor_id: row.get(7)?,
            provenance_session_id: row.get(8)?,
            provenance_execution_id: row.get(9)?,
            correlation_id: row.get(10)?,
            dedupe_key: row.get(11)?,
            priority: row.get(12)?,
            status: row.get(13)?,
            attempts: row.get(14)?,
            max_attempts: row.get(15)?,
            available_at: row.get(16)?,
            lease_owner: row.get(17)?,
            lease_token: row.get(18)?,
            lease_expires_at: row.get(19)?,
            last_failure_code: row.get(20)?,
            created_at: row.get(21)?,
            updated_at: row.get(22)?,
            completed_at: row.get(23)?,
        })
    }

    fn transition_diagnostic(&self) -> WorkTransitionDiagnostic {
        let work_id = self
            .id
            .strip_prefix("work-")
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .map(|id| format!("work-{id}"))
            .unwrap_or_else(|| "invalid-work-id".to_string());
        let kind = if self.kind.is_empty() || self.kind.len() > MAX_ROUTING_BYTES {
            "invalid-kind".to_string()
        } else {
            self.kind.clone()
        };
        WorkTransitionDiagnostic {
            work_id,
            kind,
            attempt: u8::try_from(self.attempts).unwrap_or(0),
        }
    }

    fn try_into_item(self) -> Result<WorkItem, WorkError> {
        let version =
            u16::try_from(self.envelope_version).map_err(|_| WorkError::StoredDataInvalid)?;
        let priority = i16::try_from(self.priority).map_err(|_| WorkError::StoredDataInvalid)?;
        let attempts = u8::try_from(self.attempts).map_err(|_| WorkError::StoredDataInvalid)?;
        let max_attempts =
            u8::try_from(self.max_attempts).map_err(|_| WorkError::StoredDataInvalid)?;
        if self.payload_json.len() > MAX_PAYLOAD_BYTES {
            return Err(WorkError::StoredDataInvalid);
        }
        let payload =
            serde_json::from_str(&self.payload_json).map_err(|_| WorkError::StoredDataInvalid)?;
        let created_at = parse_timestamp(&self.created_at)?;
        let available_at = parse_timestamp(&self.available_at)?;
        let lease_expires_at = self
            .lease_expires_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?;
        let completed_at = self
            .completed_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?;
        let status = WorkStatus::parse(&self.status)?;

        let authorization = WorkAuthorization {
            source: self.source,
            provenance: WorkProvenance {
                node_id: self.provenance_node_id,
                actor_id: self.provenance_actor_id,
                session_id: self.provenance_session_id,
                execution_id: self.provenance_execution_id,
            },
        };
        let draft = WorkDraft {
            version,
            kind: self.kind,
            target: self.target,
            payload,
            correlation_id: self.correlation_id,
            dedupe_key: self.dedupe_key,
            priority,
            max_attempts,
        };
        validate_draft(&draft).map_err(|_| WorkError::StoredDataInvalid)?;
        validate_authorization(&authorization).map_err(|_| WorkError::StoredDataInvalid)?;
        if attempts > max_attempts
            || !stored_state_is_consistent(
                status,
                self.lease_owner.as_deref(),
                self.lease_token.as_deref(),
                lease_expires_at,
                completed_at,
            )
        {
            return Err(WorkError::StoredDataInvalid);
        }

        Ok(WorkItem {
            envelope: WorkEnvelope {
                id: self.id,
                version: draft.version,
                kind: draft.kind,
                source: authorization.source,
                target: draft.target,
                payload: draft.payload,
                provenance: authorization.provenance,
                correlation_id: draft.correlation_id,
                dedupe_key: draft.dedupe_key,
                priority: draft.priority,
                max_attempts: draft.max_attempts,
                created_at,
            },
            status,
            attempts,
            available_at,
            lease_owner: self.lease_owner,
            lease_token: self.lease_token,
            lease_expires_at,
            last_failure_code: self
                .last_failure_code
                .as_deref()
                .map(WorkFailureCode::parse)
                .transpose()?,
            updated_at: parse_timestamp(&self.updated_at)?,
            completed_at,
        })
    }
}

fn stored_state_is_consistent(
    status: WorkStatus,
    lease_owner: Option<&str>,
    lease_token: Option<&str>,
    lease_expires_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
) -> bool {
    let owner_valid =
        lease_owner.is_some_and(|owner| !owner.is_empty() && owner.len() <= MAX_ROUTING_BYTES);
    let token_valid =
        lease_token.is_some_and(|token| !token.is_empty() && token.len() <= MAX_ROUTING_BYTES);
    match status {
        WorkStatus::Pending => {
            lease_owner.is_none()
                && lease_token.is_none()
                && lease_expires_at.is_none()
                && completed_at.is_none()
        }
        WorkStatus::Leased => {
            owner_valid && token_valid && lease_expires_at.is_some() && completed_at.is_none()
        }
        WorkStatus::Completed | WorkStatus::DeadLetter => {
            lease_owner.is_none()
                && lease_token.is_none()
                && lease_expires_at.is_none()
                && completed_at.is_some()
        }
    }
}
