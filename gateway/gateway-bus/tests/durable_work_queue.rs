use async_trait::async_trait;
use chrono::{DateTime, Utc};
use execution_state::{
    SqliteWorkStore, StateDbProvider, WorkAuthorization, WorkDraft, WorkError, WorkPolicy,
    WorkPolicyError, WorkStore,
};
use gateway_bus::{DurableWorkQueue, LocalWorkTransport, WorkTransport, WorkTransportError};
use gateway_services::VaultPaths;
use serde_json::json;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use zbot_runtime_sqlite::DatabaseManager;

struct AllowTestWork;

impl WorkPolicy for AllowTestWork {
    fn authorize(&self, draft: &WorkDraft) -> Result<WorkAuthorization, WorkPolicyError> {
        if draft.kind() != "test.work" {
            return Err(WorkPolicyError::KindNotAllowed);
        }
        if draft.target() != "worker.local" {
            return Err(WorkPolicyError::TargetNotAllowed);
        }
        if !draft
            .payload()
            .get("value")
            .is_some_and(|value| value.is_string())
        {
            return Err(WorkPolicyError::PayloadInvalid);
        }
        Ok(WorkAuthorization::new(
            "node-a",
            "node-local",
            "root",
            "sess-1",
            "exec-1",
        ))
    }
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-08-04T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn draft(dedupe_key: Option<&str>, payload: &str) -> WorkDraft {
    let draft = WorkDraft::new("test.work", "worker.local", json!({"value": payload}));
    match dedupe_key {
        Some(key) => draft.with_dedupe_key(key),
        None => draft,
    }
}

fn real_store() -> (TempDir, Arc<SqliteWorkStore<DatabaseManager>>) {
    let temp = tempfile::tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    let db = Arc::new(DatabaseManager::new(paths).unwrap());
    (temp, Arc::new(SqliteWorkStore::new(db)))
}

struct CountingLocalTransport {
    local: Arc<LocalWorkTransport>,
    publishes: AtomicUsize,
}

#[async_trait]
impl WorkTransport for CountingLocalTransport {
    async fn publish(
        &self,
        envelope: &execution_state::WorkEnvelope,
    ) -> Result<(), WorkTransportError> {
        self.publishes.fetch_add(1, Ordering::SeqCst);
        self.local.publish(envelope).await
    }
}

// STUB: AC-enqueue-dedupe, AC-transport-port
#[tokio::test]
async fn ac_local_transport_wakes_once() {
    let (_temp, store) = real_store();
    let local = Arc::new(LocalWorkTransport::new());
    let transport = Arc::new(CountingLocalTransport {
        local: local.clone(),
        publishes: AtomicUsize::new(0),
    });
    let queue = DurableWorkQueue::new(store, Arc::new(AllowTestWork), transport.clone());

    let first = queue
        .enqueue(draft(Some("same"), "first"), now())
        .await
        .unwrap();
    assert!(first.inserted());
    tokio::time::timeout(Duration::from_millis(100), local.notified())
        .await
        .unwrap();
    assert_eq!(transport.publishes.load(Ordering::SeqCst), 1);

    let duplicate = queue
        .enqueue(draft(Some("same"), "duplicate"), now())
        .await
        .unwrap();
    assert!(!duplicate.inserted());
    assert_eq!(duplicate.work_id(), first.work_id());
    assert_eq!(transport.publishes.load(Ordering::SeqCst), 1);
}

struct FailingTransport;

#[async_trait]
impl WorkTransport for FailingTransport {
    async fn publish(
        &self,
        _envelope: &execution_state::WorkEnvelope,
    ) -> Result<(), WorkTransportError> {
        Err(WorkTransportError::Unavailable)
    }
}

// STUB: AC-transport-port, AC-recovery
#[tokio::test]
async fn ac_transport_failure_preserves_work() {
    let (_temp, store) = real_store();
    let queue = DurableWorkQueue::new(
        store.clone(),
        Arc::new(AllowTestWork),
        Arc::new(FailingTransport),
    );

    let receipt = queue
        .enqueue(draft(None, "persisted"), now())
        .await
        .unwrap();
    assert!(receipt.inserted());
    assert_eq!(
        receipt.notification_error(),
        Some(WorkTransportError::Unavailable)
    );
    let claimed = store
        .claim_next("worker.local", "worker-a", now(), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.envelope().id(), receipt.work_id());
}

struct MaliciousDb;

impl StateDbProvider for MaliciousDb {
    fn with_connection<F, R>(&self, _f: F) -> Result<R, String>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<R, rusqlite::Error>,
    {
        Err("SQL /home/user/secret.db token=SUPER_SECRET".to_string())
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

// STUB: AC-observability, AC-error-redaction
#[tokio::test]
async fn ac_transport_errors_are_normalized() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(BufferWriter(bytes.clone()))
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let (_temp, store) = real_store();
    let transport_queue =
        DurableWorkQueue::new(store, Arc::new(AllowTestWork), Arc::new(FailingTransport));
    let receipt = transport_queue
        .enqueue(draft(None, "PAYLOAD_SECRET"), now())
        .await
        .unwrap();
    assert_eq!(
        receipt.notification_error(),
        Some(WorkTransportError::Unavailable)
    );
    assert_eq!(
        format!("{:?}", receipt.notification_error()),
        "Some(Unavailable)"
    );

    let malicious_store = Arc::new(SqliteWorkStore::new(Arc::new(MaliciousDb)));
    let storage_queue = DurableWorkQueue::new(
        malicious_store,
        Arc::new(AllowTestWork),
        Arc::new(LocalWorkTransport::new()),
    );
    assert_eq!(
        storage_queue
            .enqueue(draft(None, "ANOTHER_SECRET"), now())
            .await,
        Err(WorkError::StorageUnavailable)
    );

    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(output.contains("notification_failed"));
    assert!(output.contains("transport_unavailable"));
    assert!(output.contains("enqueue_failed"));
    assert!(output.contains("storage_unavailable"));
    for forbidden in [
        "PAYLOAD_SECRET",
        "ANOTHER_SECRET",
        "SUPER_SECRET",
        "/home/user/secret.db",
    ] {
        assert!(!output.contains(forbidden), "leaked {forbidden}: {output}");
    }
}
