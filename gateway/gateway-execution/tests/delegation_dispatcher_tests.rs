//! DelegationDispatcher — per-session queue that spawns subagents
//! sequentially within a session and concurrently across sessions
//! (capped by Semaphore).
//!
//! Run with:
//!   cargo test -p gateway-execution --test delegation_dispatcher_tests --features test-stubs

#![cfg(feature = "test-stubs")]

use std::sync::Arc;
use std::time::Duration;

use gateway_execution::delegation::DelegationRequest;
use gateway_execution::runner::{DelegationDispatcher, StubSessionInvoker};
use tokio::sync::{mpsc, Semaphore};

fn make_request(session_id: &str, child_agent_id: &str, task: &str) -> DelegationRequest {
    DelegationRequest {
        parent_agent_id: "root".into(),
        session_id: session_id.into(),
        parent_execution_id: "exec-root".into(),
        parent_conversation_id: format!("{}-conv", session_id),
        child_agent_id: child_agent_id.into(),
        child_execution_id: format!("exec-{}-{}", child_agent_id, session_id),
        task: task.into(),
        mode: None,
        context: None,
        max_iterations: None,
        output_schema: None,
        skills: vec![],
        capability_assignment: None,
        planning_capability_catalog: None,
        complexity: None,
        parallel: false,
    }
}

#[tokio::test]
async fn dispatcher_terminates_when_request_channel_closes() {
    let invoker = Arc::new(StubSessionInvoker::new());
    let (tx, rx) = mpsc::unbounded_channel::<DelegationRequest>();

    let dispatcher = DelegationDispatcher {
        delegation_rx: rx,
        delegation_semaphore: Arc::new(Semaphore::new(4)),
        invoker: invoker.clone(),
    };
    let handle = dispatcher.spawn();

    drop(tx);
    tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .expect("dispatcher must terminate when tx drops")
        .expect("join handle must not panic");
}

#[tokio::test]
async fn dispatcher_dispatches_each_request() {
    // Single delegation request → invoker should be called once with
    // matching child_agent_id + task.
    let invoker = Arc::new(StubSessionInvoker::new());
    let (tx, rx) = mpsc::unbounded_channel::<DelegationRequest>();

    let dispatcher = DelegationDispatcher {
        delegation_rx: rx,
        delegation_semaphore: Arc::new(Semaphore::new(4)),
        invoker: invoker.clone(),
    };
    let _handle = dispatcher.spawn();

    tx.send(make_request("session-1", "research-agent", "research X"))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let calls = invoker.delegation_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "invoker must be called once");
    assert_eq!(calls[0].child_agent_id, "research-agent");
    assert_eq!(calls[0].task, "research X");
    assert_eq!(calls[0].session_id, "session-1");
}

#[tokio::test]
async fn dispatcher_processes_multi_session_requests() {
    // 2 sessions × 2 requests each = 4 total dispatch calls.
    let invoker = Arc::new(StubSessionInvoker::new());
    let (tx, rx) = mpsc::unbounded_channel::<DelegationRequest>();

    let dispatcher = DelegationDispatcher {
        delegation_rx: rx,
        delegation_semaphore: Arc::new(Semaphore::new(4)),
        invoker: invoker.clone(),
    };
    let _handle = dispatcher.spawn();

    for sess in ["session-a", "session-b"] {
        for i in 0..2 {
            tx.send(make_request(
                sess,
                &format!("agent-{i}"),
                &format!("task-{i}"),
            ))
            .unwrap();
        }
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    let calls = invoker.delegation_calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        4,
        "all 4 delegation requests must be dispatched"
    );
}

#[tokio::test]
async fn dispatcher_serialises_sequential_requests_within_session() {
    // Within a single session, requests are dispatched one-at-a-time. The
    // stub invoker resolves immediately, so all 3 must complete in order.
    let invoker = Arc::new(StubSessionInvoker::new());
    let (tx, rx) = mpsc::unbounded_channel::<DelegationRequest>();

    let dispatcher = DelegationDispatcher {
        delegation_rx: rx,
        delegation_semaphore: Arc::new(Semaphore::new(4)),
        invoker: invoker.clone(),
    };
    let _handle = dispatcher.spawn();

    for i in 0..3 {
        tx.send(make_request(
            "session-seq",
            &format!("agent-{i}"),
            &format!("t-{i}"),
        ))
        .unwrap();
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    let calls = invoker.delegation_calls.lock().unwrap();
    assert_eq!(calls.len(), 3, "all 3 sequential requests must complete");
}

#[tokio::test]
async fn dispatcher_bypasses_queue_for_parallel_requests() {
    // parallel=true requests skip the per-session queue and run immediately.
    let invoker = Arc::new(StubSessionInvoker::new());
    let (tx, rx) = mpsc::unbounded_channel::<DelegationRequest>();

    let dispatcher = DelegationDispatcher {
        delegation_rx: rx,
        delegation_semaphore: Arc::new(Semaphore::new(4)),
        invoker: invoker.clone(),
    };
    let _handle = dispatcher.spawn();

    let mut req = make_request("session-par", "par-agent", "parallel task");
    req.parallel = true;
    tx.send(req).unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let calls = invoker.delegation_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "parallel request must be dispatched");
    assert_eq!(calls[0].child_agent_id, "par-agent");
}
