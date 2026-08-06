use async_trait::async_trait;
use chrono::{DateTime, Utc};
use execution_state::{
    RecoveryOutcome, SqliteWorkStore, StateDbProvider, WorkAuthorization, WorkCancelOutcome,
    WorkClaimCancellation, WorkCursor, WorkDraft, WorkEnvelope, WorkError, WorkFailureCode,
    WorkItem, WorkPage, WorkPolicy, WorkPolicyError, WorkScope, WorkStatus, WorkStore,
};
use gateway_bus::{
    DurableWorkWorker, LocalWorkTransport, ValidatedWorkCommand, WorkDispatchRejection,
    WorkHandler, WorkHandlerAuthorizationError, WorkHandlerContext, WorkHandlerOutcome,
    WorkHandlerPayloadError, WorkHandlerRegistry, WorkTransport, WorkWorkerConfig,
    WorkWorkerConfigError, WorkWorkerLimits,
};
use gateway_services::VaultPaths;
use serde_json::json;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use zbot_runtime_sqlite::DatabaseManager;

struct TestPolicy {
    source: &'static str,
    node_id: &'static str,
}

impl WorkPolicy for TestPolicy {
    fn authorize(&self, _draft: &WorkDraft) -> Result<WorkAuthorization, WorkPolicyError> {
        Ok(WorkAuthorization::new(
            self.source,
            self.node_id,
            "root",
            "sess-1",
            "exec-1",
        ))
    }
}

fn envelope(kind: &str, source: &'static str, payload: serde_json::Value) -> WorkEnvelope {
    envelope_with_node(kind, source, "node-local", payload)
}

fn envelope_with_node(
    kind: &str,
    source: &'static str,
    node_id: &'static str,
    payload: serde_json::Value,
) -> WorkEnvelope {
    WorkEnvelope::authorize(
        WorkDraft::new(kind, "worker.local", payload),
        &TestPolicy { source, node_id },
        DateTime::parse_from_rfc3339("2026-08-04T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    )
    .unwrap()
}

fn real_store() -> (TempDir, Arc<SqliteWorkStore<DatabaseManager>>) {
    let temp = tempfile::tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    let db = Arc::new(DatabaseManager::new(paths).unwrap());
    (temp, Arc::new(SqliteWorkStore::new(db)))
}

struct SlowClaimDb {
    inner: Arc<DatabaseManager>,
    calls: AtomicUsize,
}

impl StateDbProvider for SlowClaimDb {
    fn with_connection<F, R>(&self, operation: F) -> Result<R, String>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<R, rusqlite::Error>,
    {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 1 {
            std::thread::sleep(Duration::from_secs(2));
        }
        self.inner.with_connection(operation)
    }
}

fn worker_limits(
    concurrency: usize,
    poll_interval: Duration,
    handler_timeout: Duration,
    shutdown_drain: Duration,
) -> WorkWorkerLimits {
    WorkWorkerLimits::new(
        concurrency,
        poll_interval,
        Duration::from_secs(1),
        Duration::from_millis(200),
        handler_timeout,
        shutdown_drain,
    )
    .unwrap()
}

async fn wait_for_status(
    store: &dyn WorkStore,
    id: &str,
    expected: WorkStatus,
    timeout: Duration,
) -> execution_state::WorkItem {
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(item) = store.get(id).unwrap() {
                if item.status() == expected {
                    return item;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

async fn wait_for_counter(counter: &AtomicUsize, expected: usize, timeout: Duration) {
    tokio::time::timeout(timeout, async {
        loop {
            if counter.load(Ordering::SeqCst) == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

struct CountingHandler {
    validations: AtomicUsize,
    authorizations: AtomicUsize,
    handles: AtomicUsize,
}

impl CountingHandler {
    fn new() -> Self {
        Self {
            validations: AtomicUsize::new(0),
            authorizations: AtomicUsize::new(0),
            handles: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl WorkHandler for CountingHandler {
    fn target(&self) -> &'static str {
        "worker.local"
    }

    fn kind(&self) -> &'static str {
        "test.work"
    }

    fn validate_payload(
        &self,
        payload: &serde_json::Value,
    ) -> Result<ValidatedWorkCommand, WorkHandlerPayloadError> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        if let Some(value) = payload.get("value").and_then(serde_json::Value::as_str) {
            Ok(ValidatedWorkCommand::new(value.to_owned()))
        } else {
            Err(WorkHandlerPayloadError::Invalid)
        }
    }

    fn authorize(
        &self,
        context: &WorkHandlerContext,
        command: &ValidatedWorkCommand,
    ) -> Result<(), WorkHandlerAuthorizationError> {
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        if context.source() == "source.allowed"
            && context.provenance().node_id() == "node-local"
            && context.provenance().actor_id() == "root"
            && command.downcast_ref::<String>().is_some()
        {
            Ok(())
        } else {
            Err(WorkHandlerAuthorizationError::Rejected)
        }
    }

    async fn handle(
        &self,
        _context: WorkHandlerContext,
        _command: ValidatedWorkCommand,
    ) -> WorkHandlerOutcome {
        self.handles.fetch_add(1, Ordering::SeqCst);
        WorkHandlerOutcome::Complete
    }
}

// STUB: AC-handler-boundary
#[tokio::test]
async fn ac_handler_registry_is_exact_typed_and_default_deny() {
    let handler = Arc::new(CountingHandler::new());
    let registry = WorkHandlerRegistry::from_handlers(vec![handler.clone()]).unwrap();

    assert_eq!(
        WorkHandlerRegistry::from_handlers(vec![handler.clone(), handler.clone()]).unwrap_err(),
        WorkWorkerConfigError::DuplicateHandler
    );
    assert!(matches!(
        registry.authorized_handler(&envelope(
            "unknown.work",
            "source.allowed",
            json!({"value": "safe"}),
        )),
        Err(WorkDispatchRejection::HandlerUnavailable)
    ));
    assert_eq!(handler.validations.load(Ordering::SeqCst), 0);
    assert_eq!(handler.authorizations.load(Ordering::SeqCst), 0);

    assert!(matches!(
        registry.authorized_handler(&envelope(
            "test.work",
            "source.allowed",
            json!({"wrong": true}),
        )),
        Err(WorkDispatchRejection::InvalidPayload)
    ));
    assert_eq!(handler.validations.load(Ordering::SeqCst), 1);
    assert_eq!(handler.authorizations.load(Ordering::SeqCst), 0);
    assert_eq!(handler.handles.load(Ordering::SeqCst), 0);

    assert!(matches!(
        registry.authorized_handler(&envelope(
            "test.work",
            "source.denied",
            json!({"value": "safe"}),
        )),
        Err(WorkDispatchRejection::HandlerRejected)
    ));
    assert_eq!(handler.validations.load(Ordering::SeqCst), 2);
    assert_eq!(handler.authorizations.load(Ordering::SeqCst), 1);
    assert_eq!(handler.handles.load(Ordering::SeqCst), 0);

    assert!(matches!(
        registry.authorized_handler(&envelope_with_node(
            "test.work",
            "source.allowed",
            "node-hostile",
            json!({"value": "safe"}),
        )),
        Err(WorkDispatchRejection::HandlerRejected)
    ));
    assert_eq!(handler.validations.load(Ordering::SeqCst), 3);
    assert_eq!(handler.authorizations.load(Ordering::SeqCst), 2);
    assert_eq!(handler.handles.load(Ordering::SeqCst), 0);

    registry
        .authorized_handler(&envelope(
            "test.work",
            "source.allowed",
            json!({"value": "safe"}),
        ))
        .unwrap();
    assert_eq!(handler.validations.load(Ordering::SeqCst), 4);
    assert_eq!(handler.authorizations.load(Ordering::SeqCst), 3);
}

// STUB: AC-bounded-execution
#[test]
fn ac_worker_limits_are_bounded() {
    let min = WorkWorkerLimits::new(
        1,
        Duration::from_millis(100),
        Duration::from_secs(1),
        Duration::from_millis(100),
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    assert!(min.is_ok());

    let max = WorkWorkerLimits::new(
        32,
        Duration::from_secs(60),
        Duration::from_secs(300),
        Duration::from_secs(149),
        Duration::from_secs(3_600),
        Duration::from_secs(300),
    );
    assert!(max.is_ok());

    let invalid = [
        WorkWorkerLimits::new(
            0,
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            33,
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(99),
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_secs(61),
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_millis(999),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(301),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_millis(99),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_millis(999),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_secs(3_601),
            Duration::from_secs(1),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_millis(999),
        ),
        WorkWorkerLimits::new(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(301),
        ),
    ];
    assert!(invalid.into_iter().all(|result| result.is_err()));

    let limits = min.unwrap();
    assert_eq!(limits.concurrency(), 1);
    assert_eq!(limits.poll_interval(), Duration::from_millis(100));
    assert_eq!(limits.lease_duration(), Duration::from_secs(1));
    assert_eq!(limits.heartbeat_interval(), Duration::from_millis(100));
    assert_eq!(limits.handler_timeout(), Duration::from_secs(1));
    assert_eq!(limits.shutdown_drain(), Duration::from_secs(1));
    assert_eq!(WorkFailureCode::InvalidPayload.as_str(), "invalid_payload");
}

struct TimedHandler {
    delay: Duration,
    active: AtomicUsize,
    max_active: AtomicUsize,
    outcome: WorkHandlerOutcome,
}

struct ActiveHandlerGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveHandlerGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl TimedHandler {
    fn new(delay: Duration, outcome: WorkHandlerOutcome) -> Self {
        Self {
            delay,
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            outcome,
        }
    }
}

#[async_trait]
impl WorkHandler for TimedHandler {
    fn target(&self) -> &'static str {
        "worker.local"
    }

    fn kind(&self) -> &'static str {
        "test.work"
    }

    fn validate_payload(
        &self,
        payload: &serde_json::Value,
    ) -> Result<ValidatedWorkCommand, WorkHandlerPayloadError> {
        payload
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(|value| ValidatedWorkCommand::new(value.to_owned()))
            .ok_or(WorkHandlerPayloadError::Invalid)
    }

    fn authorize(
        &self,
        context: &WorkHandlerContext,
        command: &ValidatedWorkCommand,
    ) -> Result<(), WorkHandlerAuthorizationError> {
        if context.source() == "source.allowed"
            && context.provenance().node_id() == "node-local"
            && command.downcast_ref::<String>().is_some()
        {
            Ok(())
        } else {
            Err(WorkHandlerAuthorizationError::Rejected)
        }
    }

    async fn handle(
        &self,
        _context: WorkHandlerContext,
        _command: ValidatedWorkCommand,
    ) -> WorkHandlerOutcome {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _active_guard = ActiveHandlerGuard(&self.active);
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.outcome
    }
}

fn start_worker(
    store: Arc<dyn WorkStore>,
    transport: Arc<LocalWorkTransport>,
    handler: Arc<dyn WorkHandler>,
    limits: WorkWorkerLimits,
) -> gateway_bus::WorkWorkerHandle {
    let registry = WorkHandlerRegistry::from_handlers(vec![handler]).unwrap();
    let config = WorkWorkerConfig::new("worker.local", "worker-test", limits).unwrap();
    DurableWorkWorker::new(store, transport, registry, config).start()
}

fn start_empty_worker(
    store: Arc<dyn WorkStore>,
    transport: Arc<LocalWorkTransport>,
    limits: WorkWorkerLimits,
) -> gateway_bus::WorkWorkerHandle {
    let config = WorkWorkerConfig::new("worker.local", "worker-test", limits).unwrap();
    DurableWorkWorker::new(store, transport, WorkHandlerRegistry::empty(), config).start()
}

// STUB: AC-worker-loop, AC-bounded-execution
#[tokio::test]
async fn ac_worker_wakes_polls_recovers_and_bounds_concurrency() {
    let (_temp, concrete_store) = real_store();
    let store: Arc<dyn WorkStore> = concrete_store.clone();
    let transport = Arc::new(LocalWorkTransport::new());
    let handler = Arc::new(TimedHandler::new(
        Duration::from_millis(150),
        WorkHandlerOutcome::Complete,
    ));
    let worker = start_worker(
        store,
        transport.clone(),
        handler.clone(),
        worker_limits(
            2,
            Duration::from_secs(5),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut ids = Vec::new();
    for value in 0..6 {
        let envelope = envelope(
            "test.work",
            "source.allowed",
            json!({"value": value.to_string()}),
        );
        ids.push(envelope.id().to_owned());
        concrete_store.enqueue(&envelope).unwrap();
        transport.publish(&envelope).await.unwrap();
    }

    for id in ids {
        wait_for_status(
            concrete_store.as_ref(),
            &id,
            WorkStatus::Completed,
            Duration::from_secs(3),
        )
        .await;
    }
    assert_eq!(handler.max_active.load(Ordering::SeqCst), 2);
    worker.shutdown().await.unwrap();

    let expired = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "expired-before-restart"}),
    );
    concrete_store.enqueue(&expired).unwrap();
    concrete_store
        .claim_next(
            "worker.local",
            "crashed-worker",
            Utc::now(),
            Duration::from_secs(1),
        )
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1_050)).await;

    let missed_wake = envelope("test.work", "source.allowed", json!({"value": "poll-only"}));
    concrete_store.enqueue(&missed_wake).unwrap();
    let polling_worker = start_worker(
        concrete_store.clone(),
        Arc::new(LocalWorkTransport::new()),
        handler,
        worker_limits(
            2,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    for id in [expired.id(), missed_wake.id()] {
        wait_for_status(
            concrete_store.as_ref(),
            id,
            WorkStatus::Completed,
            Duration::from_secs(2),
        )
        .await;
    }
    polling_worker.shutdown().await.unwrap();
}

// STUB: AC-lease-safety, AC-outcomes
#[tokio::test]
async fn ac_worker_renews_lease_and_maps_timeout() {
    let (_temp, concrete_store) = real_store();
    let transport = Arc::new(LocalWorkTransport::new());
    let renewing = Arc::new(TimedHandler::new(
        Duration::from_millis(1_200),
        WorkHandlerOutcome::Complete,
    ));
    let worker = start_worker(
        concrete_store.clone(),
        transport.clone(),
        renewing,
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    let first = envelope("test.work", "source.allowed", json!({"value": "renewed"}));
    concrete_store.enqueue(&first).unwrap();
    transport.publish(&first).await.unwrap();
    wait_for_status(
        concrete_store.as_ref(),
        first.id(),
        WorkStatus::Completed,
        Duration::from_secs(3),
    )
    .await;
    worker.shutdown().await.unwrap();

    let timeout_transport = Arc::new(LocalWorkTransport::new());
    let timeout_handler = Arc::new(TimedHandler::new(
        Duration::from_secs(5),
        WorkHandlerOutcome::Complete,
    ));
    let timeout_worker = start_worker(
        concrete_store.clone(),
        timeout_transport.clone(),
        timeout_handler,
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_millis(1_100),
            Duration::from_secs(1),
        ),
    );
    let timed_out = WorkEnvelope::authorize(
        WorkDraft::new("test.work", "worker.local", json!({"value": "timeout"}))
            .with_max_attempts(1),
        &TestPolicy {
            source: "source.allowed",
            node_id: "node-local",
        },
        DateTime::parse_from_rfc3339("2026-08-04T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    )
    .unwrap();
    concrete_store.enqueue(&timed_out).unwrap();
    timeout_transport.publish(&timed_out).await.unwrap();
    let failed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let item = concrete_store.get(timed_out.id()).unwrap().unwrap();
            if item.last_failure_code() == Some(WorkFailureCode::Timeout) {
                return item;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(failed.status(), WorkStatus::DeadLetter);
    timeout_worker.shutdown().await.unwrap();

    let (_slow_temp, slow_concrete_store) = real_store();
    let slow_base: Arc<dyn WorkStore> = slow_concrete_store.clone();
    let slow_store = Arc::new(FaultStore::new(slow_base, FAULT_RENEW_SLOW));
    let slow_transport = Arc::new(LocalWorkTransport::new());
    let slow_handler = Arc::new(TimedHandler::new(
        Duration::from_secs(5),
        WorkHandlerOutcome::Complete,
    ));
    let slow_worker = start_worker(
        slow_store,
        slow_transport.clone(),
        slow_handler.clone(),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    let slow_renewal = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "slow-renewal"}),
    );
    slow_concrete_store.enqueue(&slow_renewal).unwrap();
    slow_transport.publish(&slow_renewal).await.unwrap();
    wait_for_status(
        slow_concrete_store.as_ref(),
        slow_renewal.id(),
        WorkStatus::Leased,
        Duration::from_secs(1),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(slow_handler.active.load(Ordering::SeqCst), 0);
    let slow_item = slow_concrete_store.get(slow_renewal.id()).unwrap().unwrap();
    assert_eq!(slow_item.status(), WorkStatus::Leased);
    assert_eq!(slow_item.last_failure_code(), None);
    slow_worker.shutdown().await.unwrap();
}

// STUB: AC-outcomes
#[tokio::test]
async fn ac_worker_default_deny_is_a_permanent_failure() {
    let (_temp, concrete_store) = real_store();
    let transport = Arc::new(LocalWorkTransport::new());
    let worker = start_empty_worker(
        concrete_store.clone(),
        transport.clone(),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    let rejected = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "not-registered"}),
    );
    concrete_store.enqueue(&rejected).unwrap();
    transport.publish(&rejected).await.unwrap();

    let failed = wait_for_status(
        concrete_store.as_ref(),
        rejected.id(),
        WorkStatus::DeadLetter,
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        failed.last_failure_code(),
        Some(WorkFailureCode::HandlerUnavailable)
    );
    worker.shutdown().await.unwrap();
}

struct PayloadOutcomeHandler;

#[async_trait]
impl WorkHandler for PayloadOutcomeHandler {
    fn target(&self) -> &'static str {
        "worker.local"
    }

    fn kind(&self) -> &'static str {
        "test.work"
    }

    fn validate_payload(
        &self,
        payload: &serde_json::Value,
    ) -> Result<ValidatedWorkCommand, WorkHandlerPayloadError> {
        if payload.get("value").and_then(|value| value.as_str()) == Some("validate_panic") {
            std::panic::panic_any("VALIDATE_PANIC_SECRET");
        }
        payload
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(|value| ValidatedWorkCommand::new(value.to_owned()))
            .ok_or(WorkHandlerPayloadError::Invalid)
    }

    fn authorize(
        &self,
        context: &WorkHandlerContext,
        command: &ValidatedWorkCommand,
    ) -> Result<(), WorkHandlerAuthorizationError> {
        if command.downcast_ref::<String>().map(String::as_str) == Some("authorize_panic") {
            std::panic::panic_any("AUTHORIZE_PANIC_SECRET");
        }
        if context.source() == "source.allowed" && context.provenance().node_id() == "node-local" {
            Ok(())
        } else {
            Err(WorkHandlerAuthorizationError::Rejected)
        }
    }

    async fn handle(
        &self,
        _context: WorkHandlerContext,
        command: ValidatedWorkCommand,
    ) -> WorkHandlerOutcome {
        match command.downcast::<String>().ok().as_deref() {
            Some("retry") => WorkHandlerOutcome::Retry(WorkFailureCode::Internal),
            Some("permanent") => WorkHandlerOutcome::Permanent(WorkFailureCode::HandlerRejected),
            Some("panic") => std::panic::panic_any("HANDLER_PANIC_SECRET"),
            None => WorkHandlerOutcome::Retry(WorkFailureCode::Internal),
            _ => WorkHandlerOutcome::Complete,
        }
    }
}

// STUB: AC-outcomes
#[tokio::test]
async fn ac_worker_maps_retry_permanent_and_panic_without_stopping() {
    let (_temp, concrete_store) = real_store();
    let transport = Arc::new(LocalWorkTransport::new());
    let worker = start_worker(
        concrete_store.clone(),
        transport.clone(),
        Arc::new(PayloadOutcomeHandler),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );

    let retry = envelope("test.work", "source.allowed", json!({"value": "retry"}));
    concrete_store.enqueue(&retry).unwrap();
    transport.publish(&retry).await.unwrap();
    let retry_item = wait_for_failure(&concrete_store, retry.id(), WorkFailureCode::Internal).await;
    assert_eq!(retry_item.status(), WorkStatus::Pending);

    let permanent = envelope("test.work", "source.allowed", json!({"value": "permanent"}));
    concrete_store.enqueue(&permanent).unwrap();
    transport.publish(&permanent).await.unwrap();
    let permanent_item = wait_for_status(
        concrete_store.as_ref(),
        permanent.id(),
        WorkStatus::DeadLetter,
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        permanent_item.last_failure_code(),
        Some(WorkFailureCode::HandlerRejected)
    );

    let panicking = envelope("test.work", "source.allowed", json!({"value": "panic"}));
    concrete_store.enqueue(&panicking).unwrap();
    transport.publish(&panicking).await.unwrap();
    wait_for_failure(&concrete_store, panicking.id(), WorkFailureCode::Internal).await;

    let survivor = envelope("test.work", "source.allowed", json!({"value": "survivor"}));
    concrete_store.enqueue(&survivor).unwrap();
    transport.publish(&survivor).await.unwrap();
    wait_for_status(
        concrete_store.as_ref(),
        survivor.id(),
        WorkStatus::Completed,
        Duration::from_secs(2),
    )
    .await;
    worker.shutdown().await.unwrap();
}

async fn wait_for_failure(
    store: &Arc<SqliteWorkStore<DatabaseManager>>,
    id: &str,
    code: WorkFailureCode,
) -> execution_state::WorkItem {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let item = store.get(id).unwrap().unwrap();
            if item.last_failure_code() == Some(code) {
                return item;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

// STUB: AC-shutdown
#[tokio::test]
async fn ac_worker_shutdown_leaves_aborted_lease_recoverable() {
    let (_temp, concrete_store) = real_store();
    let transport = Arc::new(LocalWorkTransport::new());
    let handler = Arc::new(TimedHandler::new(
        Duration::from_secs(30),
        WorkHandlerOutcome::Complete,
    ));
    let worker = start_worker(
        concrete_store.clone(),
        transport.clone(),
        handler.clone(),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(60),
            Duration::from_secs(1),
        ),
    );
    let recoverable = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "recoverable"}),
    );
    concrete_store.enqueue(&recoverable).unwrap();
    transport.publish(&recoverable).await.unwrap();
    wait_for_status(
        concrete_store.as_ref(),
        recoverable.id(),
        WorkStatus::Leased,
        Duration::from_secs(2),
    )
    .await;
    wait_for_counter(&handler.active, 1, Duration::from_secs(2)).await;

    let started = tokio::time::Instant::now();
    worker.shutdown().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(handler.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        concrete_store
            .get(recoverable.id())
            .unwrap()
            .unwrap()
            .status(),
        WorkStatus::Leased
    );

    tokio::time::sleep(Duration::from_millis(1_050)).await;
    let recovered = concrete_store
        .recover_expired("worker.local", Utc::now())
        .unwrap();
    assert_eq!(recovered.requeued, 1);
    assert_eq!(
        concrete_store
            .get(recoverable.id())
            .unwrap()
            .unwrap()
            .status(),
        WorkStatus::Pending
    );
}

#[test]
fn ac_worker_shutdown_signals_before_delayed_cancellation_settlement() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (temp, concrete_store) = real_store();
        let pending = envelope(
            "test.work",
            "source.allowed",
            json!({"value": "pending-during-shutdown"}),
        );
        concrete_store.enqueue(&pending).unwrap();

        let slow_claim_paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
        let slow_claim_manager = Arc::new(DatabaseManager::new(slow_claim_paths).unwrap());
        let slow_claim_db = Arc::new(SlowClaimDb {
            inner: slow_claim_manager,
            calls: AtomicUsize::new(0),
        });
        let slow_claim_store = Arc::new(SqliteWorkStore::new(slow_claim_db.clone()));
        let worker = start_empty_worker(
            slow_claim_store,
            Arc::new(LocalWorkTransport::new()),
            worker_limits(
                1,
                Duration::from_millis(100),
                Duration::from_secs(2),
                Duration::from_secs(1),
            ),
        );
        wait_for_calls(&slow_claim_db.calls, 2).await;

        let shutdown_started = Instant::now();
        assert_eq!(
            worker.shutdown().await,
            Err(gateway_bus::WorkWorkerShutdownError::ClaimSettlementTimedOut)
        );
        assert!(shutdown_started.elapsed() < Duration::from_millis(1_200));
        assert_eq!(
            concrete_store.get(pending.id()).unwrap().unwrap().status(),
            WorkStatus::Pending
        );
        tokio::time::sleep(Duration::from_millis(1_300)).await;
        assert_eq!(
            concrete_store.get(pending.id()).unwrap().unwrap().status(),
            WorkStatus::Pending
        );
    });
}

const FAULT_RECOVER: usize = 1;
const FAULT_CLAIM: usize = 2;
const FAULT_RENEW: usize = 3;
const FAULT_COMPLETE: usize = 4;
const FAULT_FAIL: usize = 5;
const FAULT_CLAIM_PANIC: usize = 6;
const FAULT_RENEW_SLOW: usize = 7;

struct FaultStore {
    inner: Arc<dyn WorkStore>,
    fault: AtomicUsize,
    claim_calls: AtomicUsize,
    recover_calls: AtomicUsize,
    renew_calls: AtomicUsize,
    complete_calls: AtomicUsize,
    fail_calls: AtomicUsize,
    claim_times: Mutex<Vec<Instant>>,
    recover_times: Mutex<Vec<Instant>>,
}

impl FaultStore {
    fn new(inner: Arc<dyn WorkStore>, fault: usize) -> Self {
        Self {
            inner,
            fault: AtomicUsize::new(fault),
            claim_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
            renew_calls: AtomicUsize::new(0),
            complete_calls: AtomicUsize::new(0),
            fail_calls: AtomicUsize::new(0),
            claim_times: Mutex::new(Vec::new()),
            recover_times: Mutex::new(Vec::new()),
        }
    }

    fn fails(&self, point: usize) -> bool {
        self.fault.load(Ordering::SeqCst) == point
    }
}

impl WorkStore for FaultStore {
    fn enqueue(
        &self,
        envelope: &WorkEnvelope,
    ) -> Result<execution_state::EnqueueOutcome, WorkError> {
        self.inner.enqueue(envelope)
    }

    fn get(&self, id: &str) -> Result<Option<WorkItem>, WorkError> {
        self.inner.get(id)
    }

    fn find_deduped(&self, source: &str, dedupe_key: &str) -> Result<Option<WorkItem>, WorkError> {
        self.inner.find_deduped(source, dedupe_key)
    }

    fn count_scoped_nonterminal(&self, scope: &WorkScope) -> Result<u64, WorkError> {
        self.inner.count_scoped_nonterminal(scope)
    }

    fn find_scoped(
        &self,
        scope: &WorkScope,
        correlation_id: &str,
    ) -> Result<Option<WorkItem>, WorkError> {
        self.inner.find_scoped(scope, correlation_id)
    }

    fn list_scoped(
        &self,
        scope: &WorkScope,
        cursor: Option<&WorkCursor>,
        limit: u16,
    ) -> Result<WorkPage, WorkError> {
        self.inner.list_scoped(scope, cursor, limit)
    }

    fn cancel_scoped(
        &self,
        scope: &WorkScope,
        correlation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<WorkCancelOutcome, WorkError> {
        self.inner.cancel_scoped(scope, correlation_id, now)
    }

    fn claim_next(
        &self,
        target: &str,
        owner: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
    ) -> Result<Option<WorkItem>, WorkError> {
        self.claim_calls.fetch_add(1, Ordering::SeqCst);
        self.claim_times.lock().unwrap().push(Instant::now());
        if self.fails(FAULT_CLAIM_PANIC) {
            std::panic::panic_any("BLOCKING_STORE_PANIC_SECRET");
        }
        if self.fails(FAULT_CLAIM) {
            return Err(WorkError::StorageUnavailable);
        }
        self.inner.claim_next(target, owner, now, lease_duration)
    }

    fn claim_next_cancellable(
        &self,
        target: &str,
        owner: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
        cancellation: &WorkClaimCancellation,
    ) -> Result<Option<WorkItem>, WorkError> {
        self.claim_calls.fetch_add(1, Ordering::SeqCst);
        self.claim_times.lock().unwrap().push(Instant::now());
        if self.fails(FAULT_CLAIM_PANIC) {
            std::panic::panic_any("BLOCKING_STORE_PANIC_SECRET");
        }
        if self.fails(FAULT_CLAIM) {
            return Err(WorkError::StorageUnavailable);
        }
        self.inner
            .claim_next_cancellable(target, owner, now, lease_duration, cancellation)
    }

    fn renew_lease(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
    ) -> Result<DateTime<Utc>, WorkError> {
        self.renew_calls.fetch_add(1, Ordering::SeqCst);
        if self.fails(FAULT_RENEW) {
            return Err(WorkError::StaleLease);
        }
        if self.fails(FAULT_RENEW_SLOW) {
            std::thread::sleep(Duration::from_millis(600));
        }
        self.inner
            .renew_lease(id, owner, token, now, lease_duration)
    }

    fn complete(
        &self,
        id: &str,
        owner: &str,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<(), WorkError> {
        self.complete_calls.fetch_add(1, Ordering::SeqCst);
        if self.fails(FAULT_COMPLETE) {
            return Err(WorkError::StorageUnavailable);
        }
        self.inner.complete(id, owner, token, now)
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
        self.fail_calls.fetch_add(1, Ordering::SeqCst);
        if self.fails(FAULT_FAIL) {
            return Err(WorkError::StorageUnavailable);
        }
        self.inner.fail(id, owner, token, now, code, retryable)
    }

    fn recover_expired(
        &self,
        target: &str,
        now: DateTime<Utc>,
    ) -> Result<RecoveryOutcome, WorkError> {
        self.recover_calls.fetch_add(1, Ordering::SeqCst);
        self.recover_times.lock().unwrap().push(Instant::now());
        if self.fails(FAULT_RECOVER) {
            return Err(WorkError::StorageUnavailable);
        }
        self.inner.recover_expired(target, now)
    }
}

async fn wait_for_calls(counter: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while counter.load(Ordering::SeqCst) < expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

fn assert_bounded_backoff(call_times: &[Instant]) {
    assert!(call_times.len() >= 3);
    assert!(call_times
        .windows(2)
        .all(|window| window[1].duration_since(window[0]) >= Duration::from_millis(90)));
}

// STUB: AC-store-fail-closed, AC-lease-safety
#[tokio::test]
async fn ac_worker_store_errors_back_off_and_abandon_stale_authority() {
    let (_temp, concrete_store) = real_store();
    let base: Arc<dyn WorkStore> = concrete_store.clone();

    for fault in [FAULT_RECOVER, FAULT_CLAIM, FAULT_CLAIM_PANIC] {
        let faulty = Arc::new(FaultStore::new(base.clone(), fault));
        let worker = start_empty_worker(
            faulty.clone(),
            Arc::new(LocalWorkTransport::new()),
            worker_limits(
                1,
                Duration::from_millis(100),
                Duration::from_secs(2),
                Duration::from_secs(1),
            ),
        );
        if fault == FAULT_RECOVER {
            wait_for_calls(&faulty.recover_calls, 3).await;
            assert_bounded_backoff(&faulty.recover_times.lock().unwrap());
        } else {
            wait_for_calls(&faulty.claim_calls, 3).await;
            assert_bounded_backoff(&faulty.claim_times.lock().unwrap());
        }
        worker.shutdown().await.unwrap();
    }

    let renewal_store = Arc::new(FaultStore::new(base.clone(), FAULT_RENEW));
    let renewal_transport = Arc::new(LocalWorkTransport::new());
    let renewal_handler = Arc::new(TimedHandler::new(
        Duration::from_secs(5),
        WorkHandlerOutcome::Complete,
    ));
    let renewal_worker = start_worker(
        renewal_store.clone(),
        renewal_transport.clone(),
        renewal_handler.clone(),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(6),
            Duration::from_secs(1),
        ),
    );
    let stale = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "stale-renewal"}),
    );
    concrete_store.enqueue(&stale).unwrap();
    renewal_transport.publish(&stale).await.unwrap();
    wait_for_status(
        concrete_store.as_ref(),
        stale.id(),
        WorkStatus::Leased,
        Duration::from_secs(1),
    )
    .await;
    wait_for_counter(&renewal_handler.active, 1, Duration::from_secs(1)).await;
    wait_for_calls(&renewal_store.renew_calls, 1).await;
    wait_for_counter(&renewal_handler.active, 0, Duration::from_secs(1)).await;
    let stale_item = concrete_store.get(stale.id()).unwrap().unwrap();
    assert_eq!(stale_item.status(), WorkStatus::Leased);
    assert_eq!(stale_item.last_failure_code(), None);
    renewal_worker.shutdown().await.unwrap();

    for fault in [FAULT_COMPLETE, FAULT_FAIL] {
        let faulty = Arc::new(FaultStore::new(base.clone(), fault));
        let transport = Arc::new(LocalWorkTransport::new());
        let handler: Arc<dyn WorkHandler> = if fault == FAULT_COMPLETE {
            Arc::new(TimedHandler::new(
                Duration::from_millis(1),
                WorkHandlerOutcome::Complete,
            ))
        } else {
            Arc::new(PayloadOutcomeHandler)
        };
        let worker = start_worker(
            faulty.clone(),
            transport.clone(),
            handler,
            worker_limits(
                1,
                Duration::from_millis(100),
                Duration::from_secs(2),
                Duration::from_secs(1),
            ),
        );
        let value = if fault == FAULT_COMPLETE {
            "complete-write"
        } else {
            "permanent"
        };
        let item = envelope("test.work", "source.allowed", json!({"value": value}));
        concrete_store.enqueue(&item).unwrap();
        transport.publish(&item).await.unwrap();
        wait_for_status(
            concrete_store.as_ref(),
            item.id(),
            WorkStatus::Leased,
            Duration::from_secs(1),
        )
        .await;
        if fault == FAULT_COMPLETE {
            wait_for_calls(&faulty.complete_calls, 1).await;
        } else {
            wait_for_calls(&faulty.fail_calls, 1).await;
        }
        let stored = concrete_store.get(item.id()).unwrap().unwrap();
        assert_eq!(stored.status(), WorkStatus::Leased);
        assert_eq!(stored.last_failure_code(), None);
        worker.shutdown().await.unwrap();
    }
}

#[derive(Clone)]
struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl Write for BufferWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// STUB: AC-observability
#[tokio::test]
async fn ac_worker_diagnostics_are_bounded_and_payload_free() {
    const CHILD_ENV: &str = "ZBOT_WORKER_PANIC_DIAGNOSTICS_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ac_worker_diagnostics_are_bounded_and_payload_free",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "diagnostics child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("HANDLER_PANIC_SECRET"));
        assert!(!stderr.contains("VALIDATE_PANIC_SECRET"));
        assert!(!stderr.contains("AUTHORIZE_PANIC_SECRET"));
        assert!(!stderr.contains("BLOCKING_STORE_PANIC_SECRET"));
        assert!(!stderr.contains("PAYLOAD_SECRET"));
        return;
    }

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(BufferWriter(bytes.clone()))
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let (_temp, concrete_store) = real_store();
    let fault_transport = Arc::new(LocalWorkTransport::new());
    let fault_worker = start_worker(
        Arc::new(FaultStore::new(concrete_store.clone(), FAULT_CLAIM_PANIC)),
        fault_transport,
        Arc::new(PayloadOutcomeHandler),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    fault_worker.shutdown().await.unwrap();

    let mut item_ids = Vec::new();
    for value in ["validate_panic", "authorize_panic", "panic"] {
        let (_item_temp, item_store) = real_store();
        let transport = Arc::new(LocalWorkTransport::new());
        let worker = start_worker(
            item_store.clone(),
            transport.clone(),
            Arc::new(PayloadOutcomeHandler),
            worker_limits(
                1,
                Duration::from_millis(100),
                Duration::from_secs(2),
                Duration::from_secs(1),
            ),
        );
        let item = envelope(
            "test.work",
            "source.allowed",
            json!({"value": value, "secret": "PAYLOAD_SECRET"}),
        );
        item_store.enqueue(&item).unwrap();
        transport.publish(&item).await.unwrap();
        wait_for_failure(&item_store, item.id(), WorkFailureCode::Internal).await;
        worker.shutdown().await.unwrap();
        item_ids.push(item.id().to_owned());
    }

    let (_auth_temp, auth_store) = real_store();
    let auth_transport = Arc::new(LocalWorkTransport::new());
    let auth_worker = start_worker(
        auth_store.clone(),
        auth_transport.clone(),
        Arc::new(PayloadOutcomeHandler),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    let rejected = envelope_with_node(
        "test.work",
        "source.allowed",
        "node-hostile",
        json!({"value": "safe", "secret": "PAYLOAD_SECRET"}),
    );
    auth_store.enqueue(&rejected).unwrap();
    auth_transport.publish(&rejected).await.unwrap();
    wait_for_failure(&auth_store, rejected.id(), WorkFailureCode::HandlerRejected).await;
    auth_worker.shutdown().await.unwrap();
    item_ids.push(rejected.id().to_owned());

    for value in ["complete", "retry", "permanent"] {
        let (_outcome_temp, outcome_store) = real_store();
        let outcome_transport = Arc::new(LocalWorkTransport::new());
        let outcome_worker = start_worker(
            outcome_store.clone(),
            outcome_transport.clone(),
            Arc::new(PayloadOutcomeHandler),
            worker_limits(
                1,
                Duration::from_millis(100),
                Duration::from_secs(2),
                Duration::from_secs(1),
            ),
        );
        let item = envelope(
            "test.work",
            "source.allowed",
            json!({"value": value, "secret": "PAYLOAD_SECRET"}),
        );
        outcome_store.enqueue(&item).unwrap();
        outcome_transport.publish(&item).await.unwrap();
        match value {
            "complete" => {
                wait_for_status(
                    outcome_store.as_ref(),
                    item.id(),
                    WorkStatus::Completed,
                    Duration::from_secs(2),
                )
                .await;
            }
            "retry" => {
                wait_for_failure(&outcome_store, item.id(), WorkFailureCode::Internal).await;
            }
            "permanent" => {
                wait_for_status(
                    outcome_store.as_ref(),
                    item.id(),
                    WorkStatus::DeadLetter,
                    Duration::from_secs(2),
                )
                .await;
            }
            _ => unreachable!(),
        }
        outcome_worker.shutdown().await.unwrap();
        item_ids.push(item.id().to_owned());
    }

    let (_renewal_temp, renewal_store) = real_store();
    let renewal_base: Arc<dyn WorkStore> = renewal_store.clone();
    let renewal_transport = Arc::new(LocalWorkTransport::new());
    let renewal_worker = start_worker(
        Arc::new(FaultStore::new(renewal_base, FAULT_RENEW)),
        renewal_transport.clone(),
        Arc::new(TimedHandler::new(
            Duration::from_secs(5),
            WorkHandlerOutcome::Complete,
        )),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ),
    );
    let renewal = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "renewal", "secret": "PAYLOAD_SECRET"}),
    );
    renewal_store.enqueue(&renewal).unwrap();
    renewal_transport.publish(&renewal).await.unwrap();
    wait_for_status(
        renewal_store.as_ref(),
        renewal.id(),
        WorkStatus::Leased,
        Duration::from_secs(1),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    renewal_worker.shutdown().await.unwrap();
    item_ids.push(renewal.id().to_owned());

    let (_timeout_temp, timeout_store) = real_store();
    let timeout_transport = Arc::new(LocalWorkTransport::new());
    let timeout_worker = start_worker(
        timeout_store.clone(),
        timeout_transport.clone(),
        Arc::new(TimedHandler::new(
            Duration::from_secs(5),
            WorkHandlerOutcome::Complete,
        )),
        worker_limits(
            1,
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
    );
    let timed_out = envelope(
        "test.work",
        "source.allowed",
        json!({"value": "timeout", "secret": "PAYLOAD_SECRET"}),
    );
    timeout_store.enqueue(&timed_out).unwrap();
    timeout_transport.publish(&timed_out).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let logs = String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned();
            if logs.contains("handler_timed_out") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    timeout_worker.shutdown().await.unwrap();
    item_ids.push(timed_out.id().to_owned());

    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(item_ids.iter().all(|item_id| output.contains(item_id)));
    assert!(output.contains("target=worker.local"));
    assert!(output.contains("kind=test.work"));
    assert!(output.contains("attempt=1"));
    assert!(output.contains("handler_panicked"));
    assert!(output.contains("handler_gate_panicked"));
    assert!(output.contains("blocking_task_failed"));
    for transition in [
        "worker_started",
        "worker_stopped",
        "work_claimed",
        "authorization_rejected",
        "lease_renewal_failed",
        "handler_timed_out",
        "work_completed",
        "work_failed",
    ] {
        assert!(
            output.contains(transition),
            "missing transition {transition}"
        );
    }
    assert!(output.contains("pending"));
    assert!(output.contains("dead_letter"));
    assert!(output.contains("stale_lease"));
    assert!(!output.contains("HANDLER_PANIC_SECRET"));
    assert!(!output.contains("VALIDATE_PANIC_SECRET"));
    assert!(!output.contains("AUTHORIZE_PANIC_SECRET"));
    assert!(!output.contains("BLOCKING_STORE_PANIC_SECRET"));
    assert!(!output.contains("PAYLOAD_SECRET"));
    assert!(!output.contains("source.allowed"));
}
