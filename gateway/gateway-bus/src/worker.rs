//! Durable-work handler contracts and bounded worker configuration.

use crate::WorkWake;
use async_trait::async_trait;
use chrono::Utc;
use execution_state::{
    WorkClaimCancellation, WorkEnvelope, WorkError, WorkFailureCode, WorkItem, WorkProvenance,
    WorkStore, MAX_LEASE_SECONDS, MAX_ROUTING_BYTES, MIN_LEASE_SECONDS,
};
use serde_json::Value;
use std::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Once;
use std::task::{Context, Poll};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::task::{JoinError, JoinSet};

const MIN_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(60);
const MIN_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(100);
const MIN_HANDLER_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_HANDLER_TIMEOUT: Duration = Duration::from_secs(3_600);
const MIN_SHUTDOWN_DRAIN: Duration = Duration::from_secs(1);
const MAX_SHUTDOWN_DRAIN: Duration = Duration::from_secs(300);
const MAX_CONCURRENCY: usize = 32;

thread_local! {
    static REDACT_WORKER_PANIC: Cell<bool> = const { Cell::new(false) };
}

static INSTALL_WORKER_PANIC_HOOK: Once = Once::new();

fn install_worker_panic_hook() {
    INSTALL_WORKER_PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            if REDACT_WORKER_PANIC.with(Cell::get) {
                return;
            }
            previous(panic_info);
        }));
    });
}

fn with_worker_panic_redaction<T>(operation: impl FnOnce() -> T) -> T {
    REDACT_WORKER_PANIC.with(|redact| {
        let previous = redact.replace(true);
        struct Restore<'a> {
            redact: &'a Cell<bool>,
            previous: bool,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.redact.set(self.previous);
            }
        }
        let _restore = Restore { redact, previous };
        operation()
    })
}

struct RedactedPanicFuture<F> {
    inner: Pin<Box<F>>,
}

struct AbortOnDropTask<T> {
    join: tokio::task::JoinHandle<T>,
}

impl<T> AbortOnDropTask<T> {
    fn new(join: tokio::task::JoinHandle<T>) -> Self {
        Self { join }
    }

    async fn abort_and_wait(&mut self) {
        self.join.abort();
        let _ = (&mut self.join).await;
    }
}

impl<T> Drop for AbortOnDropTask<T> {
    fn drop(&mut self) {
        self.join.abort();
    }
}

impl<F> RedactedPanicFuture<F> {
    fn new(inner: F) -> Self {
        Self {
            inner: Box::pin(inner),
        }
    }
}

impl<F: Future> Future for RedactedPanicFuture<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        with_worker_panic_redaction(|| self.inner.as_mut().poll(context))
    }
}

/// Invalid worker or registry configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkWorkerConfigError {
    #[error("invalid_handler_key")]
    InvalidHandlerKey,
    #[error("duplicate_handler")]
    DuplicateHandler,
    #[error("invalid_target")]
    InvalidTarget,
    #[error("invalid_owner")]
    InvalidOwner,
    #[error("invalid_concurrency")]
    InvalidConcurrency,
    #[error("invalid_poll_interval")]
    InvalidPollInterval,
    #[error("invalid_lease_duration")]
    InvalidLeaseDuration,
    #[error("invalid_heartbeat_interval")]
    InvalidHeartbeatInterval,
    #[error("invalid_handler_timeout")]
    InvalidHandlerTimeout,
    #[error("invalid_shutdown_drain")]
    InvalidShutdownDrain,
}

/// A handler rejected the shape of a kind-specific payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkHandlerPayloadError {
    #[error("invalid_payload")]
    Invalid,
}

/// A handler rejected the envelope's authenticated provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkHandlerAuthorizationError {
    #[error("handler_rejected")]
    Rejected,
}

/// The terminal intent returned by a successful handler invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkHandlerOutcome {
    Complete,
    Retry(WorkFailureCode),
    Permanent(WorkFailureCode),
}

/// A safe dispatch rejection that can be persisted without input details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkDispatchRejection {
    #[error("handler_unavailable")]
    HandlerUnavailable,
    #[error("invalid_payload")]
    InvalidPayload,
    #[error("handler_rejected")]
    HandlerRejected,
    #[error("handler_panicked")]
    HandlerPanicked,
}

/// Handler-visible identity context with routing and raw payload deliberately omitted.
#[derive(Clone)]
pub struct WorkHandlerContext {
    work_id: String,
    source: String,
    correlation_id: Option<String>,
    provenance: WorkProvenance,
}

impl WorkHandlerContext {
    fn from_envelope(envelope: &WorkEnvelope) -> Self {
        Self {
            work_id: envelope.id().to_owned(),
            source: envelope.source().to_owned(),
            correlation_id: envelope.correlation_id().map(ToOwned::to_owned),
            provenance: envelope.provenance().clone(),
        }
    }

    pub fn work_id(&self) -> &str {
        &self.work_id
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    pub fn provenance(&self) -> &WorkProvenance {
        &self.provenance
    }
}

impl fmt::Debug for WorkHandlerContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkHandlerContext")
            .field("work_id", &self.work_id)
            .field("source", &"[REDACTED]")
            .field("correlation_id", &"[REDACTED]")
            .field("provenance", &"[REDACTED]")
            .finish()
    }
}

/// Type-erased command produced only after a handler validates raw payload data.
pub struct ValidatedWorkCommand(Box<dyn Any + Send>);

impl ValidatedWorkCommand {
    pub fn new<T: Any + Send>(command: T) -> Self {
        Self(Box::new(command))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }

    pub fn downcast<T: Any>(self) -> Result<T, Self> {
        match self.0.downcast::<T>() {
            Ok(command) => Ok(*command),
            Err(command) => Err(Self(command)),
        }
    }
}

impl fmt::Debug for ValidatedWorkCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidatedWorkCommand([REDACTED])")
    }
}

impl WorkDispatchRejection {
    pub fn failure_code(self) -> WorkFailureCode {
        match self {
            Self::HandlerUnavailable => WorkFailureCode::HandlerUnavailable,
            Self::InvalidPayload => WorkFailureCode::InvalidPayload,
            Self::HandlerRejected => WorkFailureCode::HandlerRejected,
            Self::HandlerPanicked => WorkFailureCode::Internal,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(self, Self::HandlerPanicked)
    }
}

/// One exact `(target, kind)` durable-work handler.
#[async_trait]
pub trait WorkHandler: Send + Sync {
    fn target(&self) -> &'static str;
    fn kind(&self) -> &'static str;
    fn validate_payload(
        &self,
        payload: &Value,
    ) -> Result<ValidatedWorkCommand, WorkHandlerPayloadError>;
    fn authorize(
        &self,
        context: &WorkHandlerContext,
        command: &ValidatedWorkCommand,
    ) -> Result<(), WorkHandlerAuthorizationError>;
    async fn handle(
        &self,
        context: WorkHandlerContext,
        command: ValidatedWorkCommand,
    ) -> WorkHandlerOutcome;
}

/// A handler plus the typed command and constrained identity context it approved.
pub struct AuthorizedWork {
    handler: Arc<dyn WorkHandler>,
    context: WorkHandlerContext,
    command: ValidatedWorkCommand,
}

impl AuthorizedWork {
    fn into_parts(
        self,
    ) -> (
        Arc<dyn WorkHandler>,
        WorkHandlerContext,
        ValidatedWorkCommand,
    ) {
        (self.handler, self.context, self.command)
    }
}

/// Immutable, exact-match handler registry. Unknown work is denied by default.
#[derive(Clone)]
pub struct WorkHandlerRegistry {
    handlers: HashMap<(String, String), Arc<dyn WorkHandler>>,
}

impl WorkHandlerRegistry {
    pub fn empty() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn from_handlers(
        handlers: Vec<Arc<dyn WorkHandler>>,
    ) -> Result<Self, WorkWorkerConfigError> {
        let mut registry = HashMap::with_capacity(handlers.len());
        for handler in handlers {
            let target = handler.target();
            let kind = handler.kind();
            if !valid_routing_key(target) || !valid_routing_key(kind) {
                return Err(WorkWorkerConfigError::InvalidHandlerKey);
            }
            if registry
                .insert((target.to_owned(), kind.to_owned()), handler)
                .is_some()
            {
                return Err(WorkWorkerConfigError::DuplicateHandler);
            }
        }
        Ok(Self { handlers: registry })
    }

    pub fn authorized_handler(
        &self,
        envelope: &WorkEnvelope,
    ) -> Result<AuthorizedWork, WorkDispatchRejection> {
        install_worker_panic_hook();
        let key = (envelope.target().to_owned(), envelope.kind().to_owned());
        let handler = self
            .handlers
            .get(&key)
            .ok_or(WorkDispatchRejection::HandlerUnavailable)?;
        let context = WorkHandlerContext::from_envelope(envelope);
        let gates = with_worker_panic_redaction(|| {
            catch_unwind(AssertUnwindSafe(|| {
                let command = handler
                    .validate_payload(envelope.payload())
                    .map_err(|_| WorkDispatchRejection::InvalidPayload)?;
                handler
                    .authorize(&context, &command)
                    .map_err(|_| WorkDispatchRejection::HandlerRejected)?;
                Ok::<ValidatedWorkCommand, WorkDispatchRejection>(command)
            }))
        });
        let command = match gates {
            Ok(Ok(command)) => command,
            Ok(Err(rejection)) => return Err(rejection),
            Err(_) => return Err(WorkDispatchRejection::HandlerPanicked),
        };
        Ok(AuthorizedWork {
            handler: Arc::clone(handler),
            context,
            command,
        })
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for WorkHandlerRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Debug for WorkHandlerRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkHandlerRegistry")
            .field("handler_count", &self.handlers.len())
            .finish()
    }
}

/// Resource and timing bounds for one worker process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkWorkerLimits {
    concurrency: usize,
    poll_interval: Duration,
    lease_duration: Duration,
    heartbeat_interval: Duration,
    handler_timeout: Duration,
    shutdown_drain: Duration,
}

impl WorkWorkerLimits {
    pub fn new(
        concurrency: usize,
        poll_interval: Duration,
        lease_duration: Duration,
        heartbeat_interval: Duration,
        handler_timeout: Duration,
        shutdown_drain: Duration,
    ) -> Result<Self, WorkWorkerConfigError> {
        if !(1..=MAX_CONCURRENCY).contains(&concurrency) {
            return Err(WorkWorkerConfigError::InvalidConcurrency);
        }
        if !(MIN_POLL_INTERVAL..=MAX_POLL_INTERVAL).contains(&poll_interval) {
            return Err(WorkWorkerConfigError::InvalidPollInterval);
        }
        if !(Duration::from_secs(MIN_LEASE_SECONDS)..=Duration::from_secs(MAX_LEASE_SECONDS))
            .contains(&lease_duration)
        {
            return Err(WorkWorkerConfigError::InvalidLeaseDuration);
        }
        if heartbeat_interval < MIN_HEARTBEAT_INTERVAL
            || heartbeat_interval
                .checked_mul(2)
                .is_none_or(|twice| twice >= lease_duration)
        {
            return Err(WorkWorkerConfigError::InvalidHeartbeatInterval);
        }
        if !(MIN_HANDLER_TIMEOUT..=MAX_HANDLER_TIMEOUT).contains(&handler_timeout) {
            return Err(WorkWorkerConfigError::InvalidHandlerTimeout);
        }
        if !(MIN_SHUTDOWN_DRAIN..=MAX_SHUTDOWN_DRAIN).contains(&shutdown_drain) {
            return Err(WorkWorkerConfigError::InvalidShutdownDrain);
        }
        Ok(Self {
            concurrency,
            poll_interval,
            lease_duration,
            heartbeat_interval,
            handler_timeout,
            shutdown_drain,
        })
    }

    pub fn concurrency(self) -> usize {
        self.concurrency
    }
    pub fn poll_interval(self) -> Duration {
        self.poll_interval
    }
    pub fn lease_duration(self) -> Duration {
        self.lease_duration
    }
    pub fn heartbeat_interval(self) -> Duration {
        self.heartbeat_interval
    }
    pub fn handler_timeout(self) -> Duration {
        self.handler_timeout
    }
    pub fn shutdown_drain(self) -> Duration {
        self.shutdown_drain
    }
}

impl Default for WorkWorkerLimits {
    fn default() -> Self {
        Self {
            concurrency: 4,
            poll_interval: Duration::from_secs(1),
            lease_duration: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(10),
            handler_timeout: Duration::from_secs(300),
            shutdown_drain: Duration::from_secs(30),
        }
    }
}

/// Validated identity and bounds for one worker instance.
#[derive(Clone)]
pub struct WorkWorkerConfig {
    target: String,
    owner: String,
    limits: WorkWorkerLimits,
}

impl WorkWorkerConfig {
    pub fn new(
        target: impl Into<String>,
        owner: impl Into<String>,
        limits: WorkWorkerLimits,
    ) -> Result<Self, WorkWorkerConfigError> {
        let target = target.into();
        let owner = owner.into();
        if !valid_routing_key(&target) {
            return Err(WorkWorkerConfigError::InvalidTarget);
        }
        if !valid_routing_key(&owner) {
            return Err(WorkWorkerConfigError::InvalidOwner);
        }
        Ok(Self {
            target,
            owner,
            limits,
        })
    }

    pub fn target(&self) -> &str {
        &self.target
    }
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn limits(&self) -> WorkWorkerLimits {
        self.limits
    }
}

impl fmt::Debug for WorkWorkerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkWorkerConfig")
            .field("target", &self.target)
            .field("owner", &"[REDACTED]")
            .field("limits", &self.limits)
            .finish()
    }
}

fn valid_routing_key(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_ROUTING_BYTES && !value.chars().any(char::is_whitespace)
}

/// Supervised durable queue consumer for one exact target.
pub struct DurableWorkWorker {
    store: Arc<dyn WorkStore>,
    wake: Arc<dyn WorkWake>,
    registry: WorkHandlerRegistry,
    config: WorkWorkerConfig,
}

impl DurableWorkWorker {
    pub fn new(
        store: Arc<dyn WorkStore>,
        wake: Arc<dyn WorkWake>,
        registry: WorkHandlerRegistry,
        config: WorkWorkerConfig,
    ) -> Self {
        Self {
            store,
            wake,
            registry,
            config,
        }
    }

    /// Start the worker on the current Tokio runtime.
    #[must_use]
    pub fn start(self) -> WorkWorkerHandle {
        install_worker_panic_hook();
        let shutdown_drain = self.config.limits().shutdown_drain();
        let claim_cancellation = WorkClaimCancellation::new();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(self.run(shutdown_rx, claim_cancellation.clone()));
        WorkWorkerHandle {
            shutdown_tx: Some(shutdown_tx),
            join,
            shutdown_drain,
            claim_cancellation,
        }
    }

    async fn run(
        self,
        mut shutdown_rx: oneshot::Receiver<()>,
        claim_cancellation: WorkClaimCancellation,
    ) {
        let limits = self.config.limits();
        let mut active = JoinSet::new();
        let mut needs_recovery = true;

        tracing::info!(
            target = self.config.target(),
            concurrency = limits.concurrency(),
            transition = "worker_started",
            "durable work transition"
        );

        'worker: loop {
            if active.len() >= limits.concurrency() {
                tokio::select! {
                _ = &mut shutdown_rx => break 'worker,
                joined = active.join_next() => log_item_join(joined) }
                continue;
            }

            if needs_recovery {
                let store = Arc::clone(&self.store);
                let target = self.config.target().to_owned();
                let recovery =
                    blocking_store_call(move || store.recover_expired(&target, Utc::now()));
                tokio::pin!(recovery);
                let recovered = tokio::select! {
                _ = &mut shutdown_rx => {
                    let _ = recovery.await;
                    break 'worker;
                }
                recovered = &mut recovery => recovered };
                match recovered {
                    Ok(outcome) => {
                        if outcome.requeued != 0 || outcome.dead_lettered != 0 {
                            tracing::info!(
                                target = self.config.target(),
                                requeued = outcome.requeued,
                                dead_lettered = outcome.dead_lettered,
                                transition = "leases_recovered",
                                "durable work transition"
                            );
                        }
                        needs_recovery = false;
                    }
                    Err(reason) => {
                        log_store_failure(self.config.target(), "recover_failed", reason);
                        if wait_backoff_or_shutdown(limits.poll_interval(), &mut shutdown_rx).await
                        {
                            break 'worker;
                        }
                        continue;
                    }
                }
            }

            let store = Arc::clone(&self.store);
            let target = self.config.target().to_owned();
            let owner = self.config.owner().to_owned();
            let cancellation = claim_cancellation.clone();
            let claim = blocking_store_call(move || {
                store.claim_next_cancellable(
                    &target,
                    &owner,
                    Utc::now(),
                    limits.lease_duration(),
                    &cancellation,
                )
            });
            tokio::pin!(claim);
            let claimed = tokio::select! {
            _ = &mut shutdown_rx => {
                match claim.await {
                    Ok(Some(item)) => tracing::info!(
                        work_id = item.envelope().id(),
                        target = self.config.target(),
                        kind = item.envelope().kind(),
                        attempt = item.attempts(),
                        transition = "claim_settled_after_shutdown",
                        "durable work transition"
                    ),
                    Ok(None) => {}
                    Err(reason) => log_store_failure(
                        self.config.target(),
                        "shutdown_claim_settlement_failed",
                        reason,
                    ) }
                break 'worker;
            }
            claimed = &mut claim => claimed };

            match claimed {
                Ok(Some(_)) if claim_cancellation.is_cancelled() => break 'worker,
                Ok(Some(item)) => {
                    let store = Arc::clone(&self.store);
                    let registry = self.registry.clone();
                    let config = self.config.clone();
                    active.spawn(async move {
                        process_item(store, registry, config, item).await;
                    });
                }
                Ok(None) => {
                    needs_recovery = true;
                    tokio::select! {
                    _ = &mut shutdown_rx => break 'worker,
                    _ = self.wake.wait() => {},
                    _ = tokio::time::sleep(limits.poll_interval()) => {},
                    joined = active.join_next(), if !active.is_empty() => log_item_join(joined) }
                }
                Err(reason) => {
                    log_store_failure(self.config.target(), "claim_failed", reason);
                    needs_recovery = true;
                    if wait_backoff_or_shutdown(limits.poll_interval(), &mut shutdown_rx).await {
                        break 'worker;
                    }
                }
            }
        }

        let drain = async {
            while let Some(joined) = active.join_next().await {
                log_item_join(Some(joined));
            }
        };
        if tokio::time::timeout(limits.shutdown_drain(), drain)
            .await
            .is_err()
        {
            active.abort_all();
            while active.join_next().await.is_some() {}
            tracing::warn!(
                target = self.config.target(),
                transition = "shutdown_drain_expired",
                "durable work transition"
            );
        }
        tracing::info!(
            target = self.config.target(),
            transition = "worker_stopped",
            "durable work transition"
        );
    }
}

/// Lifecycle handle owned by the worker's host service.
pub struct WorkWorkerHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
    shutdown_drain: Duration,
    claim_cancellation: WorkClaimCancellation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkWorkerShutdownError {
    #[error("durable work claim settlement timed out")]
    ClaimSettlementTimedOut,
    #[error("durable work claim settlement task failed")]
    ClaimSettlementFailed,
}

impl WorkWorkerShutdownError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ClaimSettlementTimedOut => "claim_settlement_timed_out",
            Self::ClaimSettlementFailed => "claim_settlement_failed",
        }
    }
}

impl WorkWorkerHandle {
    pub async fn shutdown(mut self) -> Result<(), WorkWorkerShutdownError> {
        let shutdown_deadline = tokio::time::Instant::now() + self.shutdown_drain;
        self.claim_cancellation.signal();
        let cancellation = self.claim_cancellation.clone();
        let cancellation_task = tokio::task::spawn_blocking(move || cancellation.settle());
        match tokio::time::timeout_at(shutdown_deadline, cancellation_task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                log_join_failure("claim_cancellation_failed", error);
                self.join.abort();
                let _ = self.join.await;
                return Err(WorkWorkerShutdownError::ClaimSettlementFailed);
            }
            Err(_) => {
                self.join.abort();
                let _ = self.join.await;
                tracing::warn!(
                    reason_code = "shutdown_deadline_exceeded",
                    transition = "worker_abort",
                    "durable work transition"
                );
                return Err(WorkWorkerShutdownError::ClaimSettlementTimedOut);
            }
        }
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        match tokio::time::timeout_at(shutdown_deadline, &mut self.join).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log_join_failure("worker_join_failed", error),
            Err(_) => {
                self.join.abort();
                let _ = self.join.await;
                tracing::warn!(
                    reason_code = "shutdown_deadline_exceeded",
                    transition = "worker_abort",
                    "durable work transition"
                );
            }
        }
        Ok(())
    }

    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }
}

#[derive(Debug, Clone)]
enum StoreCallFailure {
    Store(WorkError),
    BlockingTask,
}

async fn blocking_store_call<T, F>(operation: F) -> Result<T, StoreCallFailure>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, WorkError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || with_worker_panic_redaction(operation))
        .await
        .map_err(|_| StoreCallFailure::BlockingTask)?
        .map_err(StoreCallFailure::Store)
}

async fn wait_backoff_or_shutdown(
    duration: Duration,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> bool {
    tokio::select! {
    _ = shutdown_rx => true,
    _ = tokio::time::sleep(duration) => false }
}

async fn process_item(
    store: Arc<dyn WorkStore>,
    registry: WorkHandlerRegistry,
    config: WorkWorkerConfig,
    item: WorkItem,
) {
    let envelope = item.envelope().clone();
    let work_id = envelope.id().to_owned();
    let kind = envelope.kind().to_owned();
    let attempt = item.attempts();
    let Some(token) = item.lease_token().map(str::to_owned) else {
        tracing::warn!(
            work_id,
            kind,
            attempt,
            reason_code = "missing_lease_token",
            transition = "work_abandoned",
            "durable work transition"
        );
        return;
    };

    tracing::info!(
        work_id,
        target = config.target(),
        kind,
        attempt,
        transition = "work_claimed",
        "durable work transition"
    );

    let authorized = match registry.authorized_handler(&envelope) {
        Ok(authorized) => authorized,
        Err(rejection) => {
            match rejection {
                WorkDispatchRejection::HandlerPanicked => tracing::warn!(
                    work_id,
                    target = config.target(),
                    kind,
                    attempt,
                    reason_code = "task_panicked",
                    transition = "handler_gate_panicked",
                    "durable work transition"
                ),
                WorkDispatchRejection::HandlerRejected => tracing::warn!(
                    work_id,
                    target = config.target(),
                    kind,
                    attempt,
                    reason_code = WorkFailureCode::HandlerRejected.as_str(),
                    transition = "authorization_rejected",
                    "durable work transition"
                ),
                WorkDispatchRejection::HandlerUnavailable
                | WorkDispatchRejection::InvalidPayload => {}
            }
            write_failure(
                store,
                &config,
                &work_id,
                &kind,
                attempt,
                &token,
                rejection.failure_code(),
                rejection.retryable(),
            )
            .await;
            return;
        }
    };
    let (handler, context, command) = authorized.into_parts();

    let mut handler_task =
        AbortOnDropTask::new(tokio::spawn(RedactedPanicFuture::new(async move {
            handler.handle(context, command).await
        })));
    let deadline = tokio::time::sleep(config.limits().handler_timeout());
    tokio::pin!(deadline);
    let first_heartbeat = tokio::time::Instant::now() + config.limits().heartbeat_interval();
    let mut heartbeat =
        tokio::time::interval_at(first_heartbeat, config.limits().heartbeat_interval());

    let outcome = loop {
        tokio::select! {
            result = &mut handler_task.join => {
                break match result {
                    Ok(outcome) => Some(outcome),
                    Err(error) => {
                        log_handler_join_failure(
                            &work_id,
                            config.target(),
                            &kind,
                            attempt,
                            "handler_panicked",
                            error,
                        );
                        Some(WorkHandlerOutcome::Retry(WorkFailureCode::Internal))
                    }
                };
            }
            _ = &mut deadline => {
                handler_task.abort_and_wait().await;
                tracing::warn!(
                    work_id,
                    target = config.target(),
                    kind,
                    attempt,
                    reason_code = WorkFailureCode::Timeout.as_str(),
                    transition = "handler_timed_out",
                    "durable work transition"
                );
                break Some(WorkHandlerOutcome::Retry(WorkFailureCode::Timeout));
            }
            _ = heartbeat.tick() => {
                let renewal_store = Arc::clone(&store);
                let renewal_id = work_id.clone();
                let renewal_owner = config.owner().to_owned();
                let renewal_token = token.clone();
                let lease_duration = config.limits().lease_duration();
                let renewal = blocking_store_call(move || {
                    renewal_store.renew_lease(
                        &renewal_id,
                        &renewal_owner,
                        &renewal_token,
                        Utc::now(),
                        lease_duration,
                    )
                });
                let renewal_budget = tokio::time::sleep(config.limits().heartbeat_interval());
                tokio::pin!(renewal_budget);
                let renewal = tokio::select! {
                    renewal = renewal => Some(renewal),
                    _ = &mut deadline => {
                        tracing::warn!(
                            work_id,
                            target = config.target(),
                            kind,
                            attempt,
                            reason_code = WorkFailureCode::Timeout.as_str(),
                            transition = "handler_timed_out_during_renewal",
                            "durable work transition"
                        );
                        None
                    }
                    _ = &mut renewal_budget => {
                        tracing::warn!(
                            work_id,
                            target = config.target(),
                            kind,
                            attempt,
                            reason_code = "renewal_budget_exceeded",
                            transition = "lease_renewal_timed_out",
                            "durable work transition"
                        );
                        None
                    }
                };
                let Some(renewal) = renewal else {
                    handler_task.abort_and_wait().await;
                    break None;
                };
                if let Err(reason) = renewal {
                    handler_task.abort_and_wait().await;
                    log_item_store_failure(
                        &work_id,
                        config.target(),
                        &kind,
                        attempt,
                        "lease_renewal_failed",
                        reason,
                    );
                    break None;
                }
                tracing::debug!(
                    work_id,
                    target = config.target(),
                    kind,
                    attempt,
                    transition = "lease_renewed",
                    "durable work transition"
                );
            }
        }
    };

    match outcome {
        Some(WorkHandlerOutcome::Complete) => {
            let complete_store = Arc::clone(&store);
            let complete_id = work_id.clone();
            let complete_owner = config.owner().to_owned();
            let complete_token = token.clone();
            match blocking_store_call(move || {
                complete_store.complete(&complete_id, &complete_owner, &complete_token, Utc::now())
            })
            .await
            {
                Ok(()) => tracing::info!(
                    work_id,
                    target = config.target(),
                    kind,
                    attempt,
                    transition = "work_completed",
                    "durable work transition"
                ),
                Err(reason) => log_item_store_failure(
                    &work_id,
                    config.target(),
                    &kind,
                    attempt,
                    "completion_failed",
                    reason,
                ),
            }
        }
        Some(WorkHandlerOutcome::Retry(code)) => {
            write_failure(store, &config, &work_id, &kind, attempt, &token, code, true).await;
        }
        Some(WorkHandlerOutcome::Permanent(code)) => {
            write_failure(
                store, &config, &work_id, &kind, attempt, &token, code, false,
            )
            .await;
        }
        None => {}
    }
}

#[allow(clippy::too_many_arguments)]
async fn write_failure(
    store: Arc<dyn WorkStore>,
    config: &WorkWorkerConfig,
    work_id: &str,
    kind: &str,
    attempt: u8,
    token: &str,
    code: WorkFailureCode,
    retryable: bool,
) {
    let failure_store = Arc::clone(&store);
    let failure_id = work_id.to_owned();
    let failure_owner = config.owner().to_owned();
    let failure_token = token.to_owned();
    match blocking_store_call(move || {
        failure_store.fail(
            &failure_id,
            &failure_owner,
            &failure_token,
            Utc::now(),
            code,
            retryable,
        )
    })
    .await
    {
        Ok(status) => tracing::info!(
            work_id,
            target = config.target(),
            kind,
            attempt,
            reason_code = code.as_str(),
            retryable,
            status = status.as_str(),
            transition = "work_failed",
            "durable work transition"
        ),
        Err(reason) => log_item_store_failure(
            work_id,
            config.target(),
            kind,
            attempt,
            "failure_write_failed",
            reason,
        ),
    }
}

fn log_store_failure(scope: &str, transition: &'static str, reason: StoreCallFailure) {
    let reason_code = store_failure_reason(&reason);
    tracing::warn!(scope, reason_code, transition, "durable work transition");
}

fn log_item_join(joined: Option<Result<(), JoinError>>) {
    if let Some(Err(error)) = joined {
        log_join_failure("item_task_failed", error);
    }
}

fn log_item_store_failure(
    work_id: &str,
    target: &str,
    kind: &str,
    attempt: u8,
    transition: &'static str,
    reason: StoreCallFailure,
) {
    let reason_code = store_failure_reason(&reason);
    tracing::warn!(
        work_id,
        target,
        kind,
        attempt,
        reason_code,
        transition,
        "durable work transition"
    );
}

fn store_failure_reason(reason: &StoreCallFailure) -> &'static str {
    match reason {
        StoreCallFailure::Store(WorkError::InvalidEnvelope(_)) => "invalid_envelope",
        StoreCallFailure::Store(WorkError::Policy(_)) => "policy_rejected",
        StoreCallFailure::Store(WorkError::StorageUnavailable) => "storage_unavailable",
        StoreCallFailure::Store(WorkError::StoredDataInvalid) => "stored_data_invalid",
        StoreCallFailure::Store(WorkError::NotFound) => "work_not_found",
        StoreCallFailure::Store(WorkError::StaleLease) => "stale_lease",
        StoreCallFailure::BlockingTask => "blocking_task_failed",
    }
}

fn log_handler_join_failure(
    work_id: &str,
    target: &str,
    kind: &str,
    attempt: u8,
    transition: &'static str,
    error: JoinError,
) {
    let reason_code = if error.is_cancelled() {
        "task_cancelled"
    } else {
        "task_panicked"
    };
    tracing::warn!(
        work_id,
        target,
        kind,
        attempt,
        reason_code,
        transition,
        "durable work transition"
    );
}

fn log_join_failure(transition: &'static str, error: JoinError) {
    let reason_code = if error.is_cancelled() {
        "task_cancelled"
    } else {
        "task_panicked"
    };
    tracing::warn!(reason_code, transition, "durable work transition");
}
