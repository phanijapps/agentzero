mod common;

use common::setup;
use serde_json::{json, Value};
use zbot_trace::{TraceEvent, TraceWriter};

fn ev(session: &str, tool: &str, level: &str) -> TraceEvent {
    TraceEvent {
        trace_id: "tr".to_string(),
        span_id: format!("{session}-{tool}"),
        session_id: session.to_string(),
        execution_id: "exec-1".to_string(),
        agent_id: "root".to_string(),
        parent_session_id: None,
        timestamp: "2026-07-07T00:00:00Z".to_string(),
        level: level.to_string(),
        category: "tool_result".to_string(),
        message: format!("{tool} {level}"),
        duration_ms: None,
        tool_name: Some(tool.to_string()),
        payload: None,
        usage: None,
        model: None,
    }
}

#[tokio::test]
async fn trace_query_sessions_with_failed_tool_returns_matching_sessions() {
    let (server, _dir, state) = setup();
    let traces_dir = state.paths().traces_dir();
    std::fs::create_dir_all(&traces_dir).unwrap();
    let mut writer = TraceWriter::open_confined(&traces_dir, "sess-trace-1").unwrap();
    writer
        .append(&ev("sess-trace-1", "read_file", "error"))
        .unwrap();
    writer.close().unwrap();

    let response = server
        .post("/api/traces/query")
        .json(&json!({
            "preset": "sessions_with_failed_tool",
            "params": { "tool": "read_file" }
        }))
        .await;

    response.assert_status_ok();
    let body: Value = response.json();
    let rows = body["rows"].as_array().expect("rows array");
    assert!(rows
        .iter()
        .any(|row| row["session_id"].as_str() == Some("sess-trace-1")));
}

#[tokio::test]
async fn trace_query_rejects_unknown_preset() {
    let (server, _dir, _state) = setup();

    let response = server
        .post("/api/traces/query")
        .json(&json!({
            "preset": "raw_sql",
            "params": {}
        }))
        .await;

    response.assert_status_bad_request();
}
