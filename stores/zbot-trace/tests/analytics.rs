use std::path::Path;
use tempfile::TempDir;
use zbot_trace::{TraceAnalytics, TraceEvent, TraceWriter};

fn ev(session: &str, category: &str, tool: &str, level: &str) -> TraceEvent {
    TraceEvent {
        trace_id: "tr".to_string(),
        span_id: format!("{session}-{tool}"),
        session_id: session.to_string(),
        execution_id: "e1".to_string(),
        agent_id: "root".to_string(),
        parent_session_id: None,
        timestamp: "2026-07-07T00:00:00Z".to_string(),
        level: level.to_string(),
        category: category.to_string(),
        message: format!("{tool} {level}"),
        duration_ms: None,
        tool_name: Some(tool.to_string()),
        payload: None,
        usage: None,
        model: None,
    }
}

fn write_trace(dir: &Path, session: &str, events: &[TraceEvent]) {
    let mut w = TraceWriter::open_confined(dir, session).unwrap();
    for e in events {
        w.append(e).unwrap();
    }
    w.close().unwrap();
}

#[test]
fn sessions_with_failed_tool_filters() {
    let dir = TempDir::new().unwrap();
    // s1: read_file errored.
    write_trace(dir.path(), "s1", &[ev("s1", "tool_result", "read_file", "error")]);
    // s2: read_file succeeded (no error).
    write_trace(dir.path(), "s2", &[ev("s2", "tool_result", "read_file", "info")]);
    // s3: a different tool errored (must not match read_file).
    write_trace(dir.path(), "s3", &[ev("s3", "tool_result", "grep", "error")]);

    let ta = TraceAnalytics::open(dir.path()).unwrap();
    let sessions = ta.sessions_with_failed_tool("read_file").unwrap();
    assert!(sessions.contains(&"s1".to_string()), "s1 hit a read_file error");
    assert!(!sessions.contains(&"s2".to_string()), "s2 did not error");
    assert!(!sessions.contains(&"s3".to_string()), "s3 errored on a different tool");
}

#[test]
fn empty_traces_dir_returns_empty() {
    let dir = TempDir::new().unwrap();
    let ta = TraceAnalytics::open(dir.path()).unwrap();
    assert!(ta.sessions_with_failed_tool("read_file").unwrap().is_empty());
}
