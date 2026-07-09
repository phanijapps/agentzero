//! ContinuationWatcher — unit tests for the event-loop struct.
//!
//! These tests exercise the watcher's event-handling logic in isolation
//! using `StubSessionInvoker` (no executor pipeline needed).
//!
//! Run with:
//!   cargo test -p gateway-execution --test continuation_watcher_tests --features test-stubs

#![cfg(feature = "test-stubs")]

use execution_state::StateService;
use gateway_events::{EventBus, GatewayEvent};
use gateway_execution::runner::{ContinuationSpawner, ContinuationWatcher, StubSessionInvoker};
use gateway_services::VaultPaths;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use zbot_stores_sqlite::DatabaseManager;

// ============================================================================
// Helpers
// ============================================================================

struct Fixture {
    bus: Arc<EventBus>,
    invoker: Arc<StubSessionInvoker>,
    state: Arc<StateService<DatabaseManager>>,
}

fn setup() -> Fixture {
    #[allow(deprecated)]
    let dir = tempfile::tempdir().expect("tempdir");
    #[allow(deprecated)]
    let path = dir.into_path();
    let paths = Arc::new(VaultPaths::new(path));
    let db = Arc::new(DatabaseManager::new(paths).expect("DB init"));
    Fixture {
        bus: Arc::new(EventBus::new()),
        invoker: Arc::new(StubSessionInvoker::new()),
        state: Arc::new(StateService::new(db)),
    }
}

// ============================================================================
// Tests
// ============================================================================

/// The watcher task shuts down cleanly when the bus is dropped (channel closed).
#[tokio::test]
async fn watcher_spawns_and_shuts_down_cleanly_when_bus_closes() {
    let f = setup();
    let bus = f.bus.clone();

    let watcher = ContinuationWatcher {
        event_bus: bus.clone(),
        invoker: f.invoker.clone(),
    };
    let handle = watcher.spawn();

    // Drop the bus to close the broadcast channel.
    drop(bus);
    drop(f.bus);

    // The watcher task should exit cleanly.
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "watcher task must exit within 2s after bus closes"
    );

    // No continuation calls should have been made.
    let calls = f.invoker.continuation_calls.lock().unwrap();
    assert!(
        calls.is_empty(),
        "no continuation calls expected on clean shutdown"
    );
}

/// Publishing a `SessionContinuationReady` event causes the watcher to call
/// `spawn_continuation` on the invoker with the correct args.
#[tokio::test]
async fn watcher_invokes_session_on_continuation_ready_event() {
    let f = setup();
    let (session, _root) = f.state.create_session("test-agent").unwrap();

    let watcher = ContinuationWatcher {
        event_bus: f.bus.clone(),
        invoker: f.invoker.clone(),
    };
    let _handle = watcher.spawn();

    // Give the watcher time to start listening.
    sleep(Duration::from_millis(50)).await;

    f.bus
        .publish(GatewayEvent::SessionContinuationReady {
            session_id: session.id.clone(),
            root_agent_id: "test-agent".to_string(),
            root_execution_id: "exec-1".to_string(),
        })
        .await;

    // Allow time for the watcher to process the event.
    sleep(Duration::from_millis(200)).await;

    let calls = f.invoker.continuation_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "watcher must spawn one continuation");
    assert_eq!(calls[0].0, session.id);
    assert_eq!(calls[0].1, "test-agent");
}

/// When `spawn_continuation` fails the watcher logs a warning and continues
/// processing subsequent events (it does not crash or stop).
#[tokio::test]
async fn watcher_continues_after_invoker_error() {
    // A failing invoker that counts calls — implements only ContinuationSpawner.
    struct FailingInvoker(Arc<AtomicU32>);

    #[async_trait::async_trait]
    impl ContinuationSpawner for FailingInvoker {
        async fn spawn_continuation(&self, _: String, _: String) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err("simulated".into())
        }
    }

    let counter = Arc::new(AtomicU32::new(0));
    let failing: Arc<dyn ContinuationSpawner> = Arc::new(FailingInvoker(counter.clone()));

    let bus = Arc::new(EventBus::new());
    let watcher = ContinuationWatcher {
        event_bus: bus.clone(),
        invoker: failing,
    };
    let _handle = watcher.spawn();

    sleep(Duration::from_millis(50)).await;

    // Publish two events — both should be handled despite the first failing.
    for i in 0..2u32 {
        bus.publish(GatewayEvent::SessionContinuationReady {
            session_id: format!("session-{i}"),
            root_agent_id: "test-agent".to_string(),
            root_execution_id: format!("exec-{i}"),
        })
        .await;
    }

    sleep(Duration::from_millis(200)).await;

    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "watcher must attempt spawn_continuation for every event, even after prior failures"
    );
}
