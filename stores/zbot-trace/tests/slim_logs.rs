use tempfile::NamedTempFile;
use zbot_trace::{open_trace_pool, SlimLog, SlimLogStore, SqliteSlimLogStore};

fn store() -> SqliteSlimLogStore {
    let f = NamedTempFile::new().unwrap();
    SqliteSlimLogStore::new(open_trace_pool(f.path()).unwrap())
}

fn log(id: &str, session: &str, category: &str, metadata: Option<&str>) -> SlimLog {
    SlimLog {
        id: id.to_string(),
        session_id: session.to_string(),
        conversation_id: None,
        agent_id: "root".to_string(),
        parent_session_id: None,
        timestamp: format!("2026-07-07T00:00:0{}Z", id.as_bytes().last().unwrap() - b'0'),
        level: "info".to_string(),
        category: category.to_string(),
        message: format!("{category} event"),
        // Display scalars ONLY — no args/result payloads (writer-discipline, T11).
        metadata: metadata.map(String::from),
        duration_ms: None,
    }
}

#[test]
fn append_then_query_roundtrip() {
    let store = store();
    let session = "s1";
    store
        .append(&log("1", session, "tool_call", Some(r#"{"tool_name":"read_file","tool_id":"t1"}"#)))
        .unwrap();
    store.append(&log("2", session, "tool_result", Some(r#"{"tool_name":"read_file","tool_id":"t1","error":false}"#))).unwrap();
    store.append(&log("3", session, "response", None)).unwrap();

    let rows = store.query(session, 100).unwrap();
    assert_eq!(rows.len(), 3);
    // Ordered chronologically by timestamp.
    assert_eq!(rows[0].category, "tool_call");
    assert_eq!(rows[2].category, "response");
}

#[test]
fn metadata_carries_only_display_scalars() {
    // AC: execution_logs.metadata holds only display scalars — no args/result
    // payloads. The store is agnostic; this asserts the contract a T11 writer
    // must honour: tool_call metadata = {tool_name, tool_id}, no args.
    let store = store();
    let session = "s1";
    let scalar_meta = r#"{"tool_name":"read_file","tool_id":"t1"}"#;
    store.append(&log("1", session, "tool_call", Some(scalar_meta))).unwrap();

    let rows = store.query(session, 100).unwrap();
    let meta: serde_json::Value = serde_json::from_str(rows[0].metadata.as_deref().unwrap()).unwrap();
    assert!(meta.get("tool_name").is_some(), "tool_name present");
    assert!(meta.get("args").is_none(), "args must NOT be in slim metadata");
    assert!(meta.get("result").is_none(), "result must NOT be in slim metadata");
}

#[test]
fn query_is_scoped_to_session() {
    let store = store();
    store.append(&log("1", "s1", "response", None)).unwrap();
    store.append(&log("2", "s2", "response", None)).unwrap();
    assert_eq!(store.query("s1", 100).unwrap().len(), 1);
    assert_eq!(store.query("s2", 100).unwrap().len(), 1);
}
