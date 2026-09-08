//! Narrow invoker traits — stub recording tests.
//!
//! Run with: cargo test -p gateway-execution --test session_invoker_tests --features test-stubs

#![cfg(feature = "test-stubs")]

use gateway_execution::config::ExecutionConfig;
use gateway_execution::runner::StubSessionInvoker;
use std::path::PathBuf;

#[tokio::test]
async fn stub_records_each_call() {
    let stub = StubSessionInvoker::new();
    let cfg = ExecutionConfig::new(
        "root".to_string(),
        "conv-1".to_string(),
        PathBuf::from("/tmp"),
    );

    stub.spawn_session(cfg.clone(), "hi".to_string())
        .await
        .expect("stub must succeed");

    let calls = stub.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.agent_id, "root");
    assert_eq!(calls[0].1, "hi");
}

#[tokio::test]
async fn stub_records_multiple_calls_in_order() {
    let stub = StubSessionInvoker::new();
    let cfg = ExecutionConfig::new("root".to_string(), "c".to_string(), PathBuf::from("/tmp"));
    stub.spawn_session(cfg.clone(), "a".into()).await.unwrap();
    stub.spawn_session(cfg.clone(), "b".into()).await.unwrap();
    stub.spawn_session(cfg, "c".into()).await.unwrap();

    let calls = stub.calls.lock().unwrap();
    let messages: Vec<&str> = calls.iter().map(|(_, m)| m.as_str()).collect();
    assert_eq!(messages, vec!["a", "b", "c"]);
}
