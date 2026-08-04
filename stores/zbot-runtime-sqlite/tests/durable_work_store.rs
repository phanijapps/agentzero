use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use execution_state::{
    SqliteWorkStore, StateDbProvider, WorkAuthorization, WorkDraft, WorkEnvelope, WorkError,
    WorkFailureCode, WorkPolicy, WorkPolicyError, WorkStatus, WorkStore, MAX_PAYLOAD_BYTES,
};
use gateway_services::VaultPaths;
use rusqlite::Connection;
use serde_json::json;
use std::io::Write;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;
use zbot_runtime_sqlite::DatabaseManager;

struct AllowTestWork {
    source: String,
}

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
            &self.source,
            "node-local",
            "root",
            "sess-1",
            "exec-1",
        ))
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("2026-08-04T00:00:{seconds:02}Z"))
        .unwrap()
        .with_timezone(&Utc)
}

fn at_nanos(seconds: i64, nanos: i64) -> DateTime<Utc> {
    at(seconds) + TimeDelta::nanoseconds(nanos)
}

fn setup() -> (
    tempfile::TempDir,
    Arc<SqliteWorkStore<DatabaseManager>>,
    Arc<DatabaseManager>,
) {
    let temp = tempfile::tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    let db = Arc::new(DatabaseManager::new(paths).unwrap());
    let store = Arc::new(SqliteWorkStore::new(db.clone()));
    (temp, store, db)
}

fn envelope(
    source: &str,
    dedupe_key: Option<&str>,
    priority: i16,
    max_attempts: u8,
    created_at: DateTime<Utc>,
    value: &str,
) -> WorkEnvelope {
    let mut draft = WorkDraft::new("test.work", "worker.local", json!({"value": value}))
        .with_priority(priority)
        .with_max_attempts(max_attempts);
    if let Some(key) = dedupe_key {
        draft = draft.with_dedupe_key(key);
    }
    WorkEnvelope::authorize(
        draft,
        &AllowTestWork {
            source: source.to_string(),
        },
        created_at,
    )
    .unwrap()
}

// STUB: AC-enqueue-dedupe
#[test]
fn ac_enqueue_dedupe() {
    let (_temp, store, _db) = setup();
    let first = envelope("node-a", Some("same"), 0, 5, at(0), "first");
    let duplicate = envelope("node-a", Some("same"), 0, 5, at(1), "second");
    let other_source = envelope("node-b", Some("same"), 0, 5, at(1), "third");

    let inserted = store.enqueue(&first).unwrap();
    let deduped = store.enqueue(&duplicate).unwrap();
    let distinct = store.enqueue(&other_source).unwrap();

    assert!(inserted.inserted());
    assert!(!deduped.inserted());
    assert_eq!(deduped.item().envelope().id(), first.id());
    assert!(distinct.inserted());
}

// STUB: AC-atomic-claim
#[test]
fn ac_atomic_claim_total_order() {
    let (_temp, store, db) = setup();
    let same_time = at(0);
    let available_first = envelope("node-a", None, 9, 5, same_time, "available-first");
    let available_second = envelope("node-a", None, 9, 5, same_time, "available-second");
    let created_first = envelope("node-a", None, 9, 5, same_time, "created-first");
    let created_second = envelope("node-a", None, 9, 5, same_time, "created-second");
    let mut id_tie = [
        envelope("node-a", None, 9, 5, same_time, "high-a"),
        envelope("node-a", None, 9, 5, same_time, "high-b"),
    ];
    id_tie.sort_by(|left, right| left.id().cmp(right.id()));
    let low = envelope("node-a", None, 1, 5, at(0), "low");
    for item in [
        &available_first,
        &available_second,
        &created_first,
        &created_second,
        &id_tie[0],
        &id_tie[1],
        &low,
    ] {
        store.enqueue(item).unwrap();
    }
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items
             SET available_at = '2026-08-04T00:00:00.000000000Z',
                 created_at = '2026-08-04T00:00:05.000000000Z'
             WHERE id = ?1",
            [available_first.id()],
        )?;
        conn.execute(
            "UPDATE durable_work_items
             SET available_at = '2026-08-04T00:00:01.000000000Z',
                 created_at = '2026-08-04T00:00:00.000000000Z'
             WHERE id = ?1",
            [available_second.id()],
        )?;
        for (item, available_at, created_at) in [
            (
                &created_first,
                "2026-08-04T00:00:02.000000000Z",
                "2026-08-04T00:00:00.000000000Z",
            ),
            (
                &created_second,
                "2026-08-04T00:00:02.000000000Z",
                "2026-08-04T00:00:01.000000000Z",
            ),
            (
                &id_tie[0],
                "2026-08-04T00:00:03.000000000Z",
                "2026-08-04T00:00:02.000000000Z",
            ),
            (
                &id_tie[1],
                "2026-08-04T00:00:03.000000000Z",
                "2026-08-04T00:00:02.000000000Z",
            ),
        ] {
            conn.execute(
                "UPDATE durable_work_items
                 SET available_at = ?2, created_at = ?3
                 WHERE id = ?1",
                [item.id(), available_at, created_at],
            )?;
        }
        Ok(())
    })
    .unwrap();

    let expected = [
        available_first.id(),
        available_second.id(),
        created_first.id(),
        created_second.id(),
        id_tie[0].id(),
        id_tie[1].id(),
        low.id(),
    ];
    let claimed: Vec<String> = (0..expected.len())
        .map(|_| {
            store
                .claim_next("worker.local", "worker-a", at(4), Duration::from_secs(30))
                .unwrap()
                .unwrap()
                .envelope()
                .id()
                .to_string()
        })
        .collect();
    assert_eq!(claimed, expected);

    let concurrent = envelope("node-a", None, 0, 5, at(5), "concurrent");
    store.enqueue(&concurrent).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut joins = Vec::new();
    for owner in ["worker-b", "worker-c"] {
        let store = store.clone();
        let barrier = barrier.clone();
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .claim_next("worker.local", owner, at(6), Duration::from_secs(30))
                .unwrap()
        }));
    }
    barrier.wait();
    let claimed = joins
        .into_iter()
        .map(|join| join.join().unwrap().is_some())
        .filter(|claimed| *claimed)
        .count();
    assert_eq!(claimed, 1);
}

#[test]
fn nanosecond_timestamps_preserve_order_and_lease_fences() {
    let (_temp, store, db) = setup();
    let later = envelope("node-a", None, 0, 5, at_nanos(0, 200), "later");
    let earlier = envelope("node-a", None, 0, 5, at_nanos(0, 100), "earlier");
    store.enqueue(&later).unwrap();
    store.enqueue(&earlier).unwrap();
    let same_available = at_nanos(1, 500).to_rfc3339_opts(SecondsFormat::Nanos, true);
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items SET available_at = ?1 WHERE id IN (?2, ?3)",
            [&same_available, earlier.id(), later.id()],
        )?;
        Ok(())
    })
    .unwrap();

    let first = store
        .claim_next("worker.local", "worker-a", at(2), Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(first.envelope().id(), earlier.id());
    store
        .complete(
            earlier.id(),
            "worker-a",
            first.lease_token().unwrap(),
            at(2),
        )
        .unwrap();

    let claim_at = at_nanos(2, 123);
    let second = store
        .claim_next("worker.local", "worker-b", claim_at, Duration::from_secs(1))
        .unwrap()
        .unwrap();
    let expected_expiry = claim_at + TimeDelta::seconds(1);
    assert_eq!(second.lease_expires_at(), Some(expected_expiry));
    let renew_at = expected_expiry - TimeDelta::nanoseconds(1);
    let renewed_expiry = store
        .renew_lease(
            later.id(),
            "worker-b",
            second.lease_token().unwrap(),
            renew_at,
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(renewed_expiry, renew_at + TimeDelta::seconds(1));
    assert_eq!(
        store
            .fail(
                later.id(),
                "worker-b",
                second.lease_token().unwrap(),
                renew_at,
                WorkFailureCode::Timeout,
                true,
            )
            .unwrap(),
        WorkStatus::Pending
    );
    assert_eq!(
        store.get(later.id()).unwrap().unwrap().available_at(),
        renew_at + TimeDelta::seconds(1)
    );

    let exact = envelope("node-a", None, 0, 5, at(4), "exact-expiry");
    store.enqueue(&exact).unwrap();
    let exact_claim = store
        .claim_next(
            "worker.local",
            "worker-c",
            at_nanos(5, 321),
            Duration::from_secs(1),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        store.complete(
            exact.id(),
            "worker-c",
            exact_claim.lease_token().unwrap(),
            exact_claim.lease_expires_at().unwrap(),
        ),
        Err(WorkError::StaleLease)
    );
}

// STUB: AC-fenced-transitions
#[test]
fn ac_fenced_transitions_reject_expired_owner() {
    let (_temp, store, _db) = setup();
    let work = envelope("node-a", None, 0, 5, at(0), "fenced");
    store.enqueue(&work).unwrap();
    let first = store
        .claim_next("worker.local", "worker-a", at(1), Duration::from_secs(1))
        .unwrap()
        .unwrap();
    let old_token = first.lease_token().unwrap().to_string();
    assert!(!format!("{first:?}").contains(&old_token));

    assert_eq!(
        store.complete(work.id(), "worker-a", &old_token, at(3)),
        Err(WorkError::StaleLease)
    );
    assert_eq!(
        store.renew_lease(
            work.id(),
            "worker-a",
            &old_token,
            at(3),
            Duration::from_secs(5)
        ),
        Err(WorkError::StaleLease)
    );
    assert_eq!(
        store.fail(
            work.id(),
            "worker-a",
            &old_token,
            at(3),
            WorkFailureCode::Timeout,
            true
        ),
        Err(WorkError::StaleLease)
    );

    let reassigned = store
        .claim_next("worker.local", "worker-b", at(3), Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_ne!(reassigned.lease_token(), Some(old_token.as_str()));
    assert_eq!(
        store.complete(work.id(), "worker-a", &old_token, at(4)),
        Err(WorkError::StaleLease)
    );
    store
        .complete(
            work.id(),
            "worker-b",
            reassigned.lease_token().unwrap(),
            at(4),
        )
        .unwrap();

    let permanent = envelope("node-a", None, 0, 5, at(5), "permanent");
    store.enqueue(&permanent).unwrap();
    let permanent_lease = store
        .claim_next("worker.local", "worker-c", at(6), Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(
        store.fail(
            permanent.id(),
            "worker-c",
            "stale-token",
            at(7),
            WorkFailureCode::HandlerRejected,
            false,
        ),
        Err(WorkError::StaleLease)
    );
    assert_eq!(
        store
            .fail(
                permanent.id(),
                "worker-c",
                permanent_lease.lease_token().unwrap(),
                at(7),
                WorkFailureCode::HandlerRejected,
                false,
            )
            .unwrap(),
        WorkStatus::DeadLetter
    );
    let dead = store.get(permanent.id()).unwrap().unwrap();
    assert_eq!(dead.status(), WorkStatus::DeadLetter);
    assert_eq!(
        dead.last_failure_code(),
        Some(WorkFailureCode::HandlerRejected)
    );
    assert!(dead.lease_owner().is_none());
    assert!(dead.lease_token().is_none());
    assert!(dead.lease_expires_at().is_none());
    assert_eq!(dead.completed_at(), Some(at(7)));
}

// STUB: AC-lease-bounds
#[test]
fn ac_lease_bounds() {
    let (_temp, store, _db) = setup();
    let work = envelope("node-a", None, 0, 5, at(0), "lease");
    store.enqueue(&work).unwrap();
    for invalid in [
        Duration::ZERO,
        Duration::from_millis(1_500),
        Duration::from_secs(301),
    ] {
        assert_eq!(
            store.claim_next("worker.local", "worker-a", at(1), invalid),
            Err(WorkError::InvalidEnvelope(
                execution_state::WorkValidationError::InvalidLeaseDuration
            ))
        );
    }
    let claimed = store
        .claim_next("worker.local", "worker-a", at(1), Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.lease_expires_at(), Some(at(2)));
    assert_eq!(
        store.renew_lease(
            work.id(),
            "worker-a",
            claimed.lease_token().unwrap(),
            at(1),
            Duration::from_secs(301)
        ),
        Err(WorkError::InvalidEnvelope(
            execution_state::WorkValidationError::InvalidLeaseDuration
        ))
    );
}

// STUB: AC-recovery, AC-bounded-retry
#[test]
fn ac_recovery_and_bounded_retry() {
    let (_temp, store, _db) = setup();
    let work = envelope("node-a", None, 0, 2, at(0), "retry");
    store.enqueue(&work).unwrap();
    let first = store
        .claim_next("worker.local", "worker-a", at(1), Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .fail(
                work.id(),
                "worker-a",
                first.lease_token().unwrap(),
                at(1),
                WorkFailureCode::Timeout,
                true,
            )
            .unwrap(),
        WorkStatus::Pending
    );
    assert!(store
        .claim_next("worker.local", "worker-a", at(1), Duration::from_secs(1))
        .unwrap()
        .is_none());
    let second = store
        .claim_next("worker.local", "worker-a", at(2), Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(second.attempts(), 2);
    let recovery = store.recover_expired("worker.local", at(4)).unwrap();
    assert_eq!(recovery.dead_lettered, 1);
    let dead = store.get(work.id()).unwrap().unwrap();
    assert_eq!(dead.status(), WorkStatus::DeadLetter);
    assert_eq!(
        dead.last_failure_code(),
        Some(WorkFailureCode::AttemptsExhausted)
    );

    let capped = envelope("node-a", None, 0, 20, at(5), "cap");
    store.enqueue(&capped).unwrap();
    let mut now = at(6);
    let expected_delays = [1, 2, 4, 8, 16, 32, 64, 128, 256, 300];
    for (expected_attempt, expected_delay) in (1..=10).zip(expected_delays) {
        let lease = store
            .claim_next("worker.local", "worker-a", now, Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(lease.attempts(), expected_attempt);
        store
            .fail(
                capped.id(),
                "worker-a",
                lease.lease_token().unwrap(),
                now,
                WorkFailureCode::Timeout,
                true,
            )
            .unwrap();
        let item = store.get(capped.id()).unwrap().unwrap();
        let delay = item.available_at() - now;
        assert_eq!(delay, TimeDelta::seconds(expected_delay));
        now = item.available_at();
    }
}

#[test]
fn corrupt_expired_lease_is_dead_lettered_before_recovery() {
    let (_temp, store, db) = setup();
    let work = envelope("node-a", None, 0, 5, at(0), "corrupt-lease");
    store.enqueue(&work).unwrap();
    store
        .claim_next("worker.local", "worker-a", at(1), Duration::from_secs(1))
        .unwrap()
        .unwrap();
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items SET lease_owner = NULL WHERE id = ?1",
            [work.id()],
        )?;
        Ok(())
    })
    .unwrap();

    let recovery = store.recover_expired("worker.local", at(3)).unwrap();
    assert_eq!(recovery.requeued, 0);
    assert_eq!(recovery.dead_lettered, 1);
    let dead = store.get(work.id()).unwrap().unwrap();
    assert_eq!(dead.status(), WorkStatus::DeadLetter);
    assert_eq!(dead.attempts(), 1);
    assert_eq!(
        dead.last_failure_code(),
        Some(WorkFailureCode::IntegrityViolation)
    );
    assert!(store
        .claim_next("worker.local", "worker-b", at(4), Duration::from_secs(1),)
        .unwrap()
        .is_none());
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
#[test]
fn ac_store_transition_observability() {
    let (_temp, store, db) = setup();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(BufferWriter(bytes.clone()))
        .finish();

    tracing::subscriber::set_global_default(subscriber).unwrap();
    let (observed_work_id, recoverable_id, exhausted_id, corrupt_pending_id, corrupt_lease_id) = {
        let draft = WorkDraft::new(
            "test.work",
            "worker.local",
            json!({"value": "PAYLOAD_SECRET"}),
        )
        .with_correlation_id("CORRELATION_SECRET")
        .with_max_attempts(2);
        let work = WorkEnvelope::authorize(
            draft,
            &AllowTestWork {
                source: "node-a".to_string(),
            },
            at(0),
        )
        .unwrap();
        store.enqueue(&work).unwrap();
        let first = store
            .claim_next("worker.local", "worker-a", at(1), Duration::from_secs(1))
            .unwrap()
            .unwrap();
        store
            .fail(
                work.id(),
                "worker-a",
                first.lease_token().unwrap(),
                at(1),
                WorkFailureCode::Timeout,
                true,
            )
            .unwrap();
        let second = store
            .claim_next("worker.local", "worker-a", at(2), Duration::from_secs(1))
            .unwrap()
            .unwrap();
        store
            .complete(work.id(), "worker-a", second.lease_token().unwrap(), at(2))
            .unwrap();
        let observed_work_id = work.id().to_string();

        let recoverable = envelope("node-a", None, 0, 2, at(3), "recoverable");
        store.enqueue(&recoverable).unwrap();
        store
            .claim_next("worker.local", "worker-a", at(4), Duration::from_secs(1))
            .unwrap();
        store.recover_expired("worker.local", at(6)).unwrap();
        let recoverable_id = recoverable.id().to_string();

        let exhausted = envelope("node-a", None, 1, 1, at(7), "exhausted");
        store.enqueue(&exhausted).unwrap();
        store
            .claim_next("worker.local", "worker-a", at(8), Duration::from_secs(1))
            .unwrap();
        store.recover_expired("worker.local", at(10)).unwrap();
        let exhausted_id = exhausted.id().to_string();

        let corrupt_pending = envelope("node-a", None, 2, 2, at(11), "corrupt-pending");
        store.enqueue(&corrupt_pending).unwrap();
        db.with_connection(|conn| {
            conn.execute(
                "UPDATE durable_work_items SET lease_owner = 'stale' WHERE id = ?1",
                [corrupt_pending.id()],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            store.claim_next("worker.local", "worker-a", at(12), Duration::from_secs(1),),
            Err(WorkError::StoredDataInvalid)
        );
        let corrupt_pending_id = corrupt_pending.id().to_string();

        let corrupt_lease = envelope("node-a", None, 3, 2, at(13), "corrupt-lease");
        store.enqueue(&corrupt_lease).unwrap();
        store
            .claim_next("worker.local", "worker-a", at(14), Duration::from_secs(1))
            .unwrap();
        db.with_connection(|conn| {
            conn.execute(
                "UPDATE durable_work_items SET lease_owner = NULL WHERE id = ?1",
                [corrupt_lease.id()],
            )?;
            Ok(())
        })
        .unwrap();
        store.recover_expired("worker.local", at(16)).unwrap();
        let corrupt_lease_id = corrupt_lease.id().to_string();

        (
            observed_work_id,
            recoverable_id,
            exhausted_id,
            corrupt_pending_id,
            corrupt_lease_id,
        )
    };

    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    for transition in [
        "enqueue",
        "claim",
        "retry",
        "complete",
        "lease_recovery",
        "dead_letter",
    ] {
        assert!(
            output.contains(transition),
            "missing transition {transition}: {output}"
        );
    }
    for transition in ["retry", "complete"] {
        let line = output
            .lines()
            .find(|line| line.contains(&observed_work_id) && line.contains(transition))
            .unwrap_or_else(|| {
                panic!("missing {transition} line for {observed_work_id}: {output}")
            });
        for field in ["kind=test.work", "target=worker.local", "attempt="] {
            assert!(line.contains(field), "missing {field} in {line}");
        }
    }
    let recovery_line = output
        .lines()
        .find(|line| {
            line.contains(&recoverable_id) && line.contains("transition=\"lease_recovery\"")
        })
        .unwrap_or_else(|| panic!("missing recovery line for {recoverable_id}: {output}"));
    for field in [
        "kind=test.work",
        "target=worker.local",
        "attempt=1",
        "reason_code=\"lease_expired\"",
    ] {
        assert!(
            recovery_line.contains(field),
            "missing {field} in {recovery_line}"
        );
    }
    let dead_letter_line = output
        .lines()
        .find(|line| {
            line.contains(&exhausted_id)
                && line.contains("kind=test.work")
                && line.contains("target=worker.local")
                && line.contains("attempt=1")
                && line.contains("transition=\"dead_letter\"")
                && line.contains("reason_code=\"attempts_exhausted\"")
        })
        .unwrap();
    assert!(dead_letter_line.contains("target=worker.local"));
    for work_id in [&corrupt_pending_id, &corrupt_lease_id] {
        let integrity_line = output
            .lines()
            .find(|line| {
                line.contains(work_id)
                    && line.contains("kind=test.work")
                    && line.contains("target=worker.local")
                    && line.contains("attempt=")
                    && line.contains("transition=\"dead_letter\"")
                    && line.contains("reason_code=\"integrity_violation\"")
            })
            .unwrap_or_else(|| panic!("missing integrity line for {work_id}: {output}"));
        assert!(!integrity_line.contains("corrupt-"));
    }
    assert!(!output.contains("PAYLOAD_SECRET"));
    assert!(!output.contains("CORRELATION_SECRET"));
}

struct FailingDb;

impl StateDbProvider for FailingDb {
    fn with_connection<F, R>(&self, _f: F) -> Result<R, String>
    where
        F: FnOnce(&Connection) -> Result<R, rusqlite::Error>,
    {
        Err("SQL /home/user/secret.db token=SUPER_SECRET".to_string())
    }
}

// STUB: AC-error-redaction
#[test]
fn ac_store_errors_are_normalized() {
    let unavailable = SqliteWorkStore::new(Arc::new(FailingDb));
    let work = envelope("node-a", None, 0, 5, at(0), "PAYLOAD_SECRET");
    let error = unavailable.enqueue(&work).unwrap_err();
    assert_eq!(error, WorkError::StorageUnavailable);
    let diagnostic = format!("{error:?} {error}");
    for forbidden in [
        "SQL",
        "/home/user/secret.db",
        "SUPER_SECRET",
        "PAYLOAD_SECRET",
    ] {
        assert!(!diagnostic.contains(forbidden));
    }

    let (_temp, store, db) = setup();
    store.enqueue(&work).unwrap();
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items SET payload_json = 'not-json' WHERE id = ?1",
            [work.id()],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(store.get(work.id()), Err(WorkError::StoredDataInvalid));

    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items SET payload_json = '{}', available_at = 'not-a-time' WHERE id = ?1",
            [work.id()],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(store.get(work.id()), Err(WorkError::StoredDataInvalid));

    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items
             SET available_at = '2026-08-04T00:00:00.000000000Z', status = 'leased',
                 lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL
             WHERE id = ?1",
            [work.id()],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(store.get(work.id()), Err(WorkError::StoredDataInvalid));

    db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items
             SET status = 'pending', lease_owner = 'stale-owner',
                 lease_token = 'stale-token',
                 lease_expires_at = '2026-08-04T00:00:30.000000000Z'
             WHERE id = ?1",
            [work.id()],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        store.claim_next("worker.local", "worker-a", at(1), Duration::from_secs(30)),
        Err(WorkError::StoredDataInvalid)
    );
    let (status, failure_code, attempts): (String, String, i64) = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT status, last_failure_code, attempts
                 FROM durable_work_items WHERE id = ?1",
                [work.id()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .unwrap();
    assert_eq!(status, "dead_letter");
    assert_eq!(failure_code, "integrity_violation");
    assert_eq!(attempts, 0);
}

#[test]
fn oversized_stored_payload_is_constrained_and_quarantined_before_dispatch() {
    let (_temp, store, db) = setup();
    let work = envelope("node-a", None, 0, 5, at(0), "safe");
    store.enqueue(&work).unwrap();
    let oversized_payload = format!("{{\"value\":\"{}\"}}", "x".repeat(MAX_PAYLOAD_BYTES));

    let constrained = db.with_connection(|conn| {
        conn.execute(
            "UPDATE durable_work_items SET payload_json = ?2 WHERE id = ?1",
            rusqlite::params![work.id(), oversized_payload],
        )
    });
    assert!(
        constrained.is_err(),
        "oversized payload must fail the CHECK constraint"
    );

    db.with_connection(|conn| {
        conn.pragma_update(None, "ignore_check_constraints", true)?;
        conn.execute(
            "UPDATE durable_work_items SET payload_json = ?2 WHERE id = ?1",
            rusqlite::params![work.id(), oversized_payload],
        )?;
        conn.pragma_update(None, "ignore_check_constraints", false)?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        store.claim_next("worker.local", "worker-a", at(1), Duration::from_secs(30)),
        Err(WorkError::StoredDataInvalid)
    );
    let status: String = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT status FROM durable_work_items WHERE id = ?1",
                [work.id()],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(status, "dead_letter");
}
