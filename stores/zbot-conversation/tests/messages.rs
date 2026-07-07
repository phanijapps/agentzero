use std::sync::Arc;
use tempfile::NamedTempFile;
use zbot_conversation::{open_conversation_pool, Message, MessageStore, SqliteMessageStore};

fn store() -> Arc<SqliteMessageStore> {
    let f = NamedTempFile::new().unwrap();
    Arc::new(SqliteMessageStore::new(open_conversation_pool(f.path()).unwrap()))
}

fn msg(id: &str, session: &str, role: &str, content: &str, tool_calls: Option<&str>) -> Message {
    Message {
        id: id.to_string(),
        execution_id: None,
        session_id: session.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        created_at: "2026-07-07T00:00:00Z".to_string(),
        token_count: 1,
        tool_calls: tool_calls.map(String::from),
        tool_call_id: None,
        seq: 0, // ignored — assigned server-side
    }
}

#[test]
fn append_then_replay_roundtrip() {
    let store = store();
    let session = "s1";
    store.append(&msg("msg-1", session, "user", "hi", None)).unwrap();
    store
        .append(&msg("msg-2", session, "assistant", "ok", Some(r#"[{"tool_id":"t1","tool_name":"read_file","args":{},"result":"","error":null}]"#)))
        .unwrap();
    store.append(&msg("msg-3", session, "tool", "<result>", None)).unwrap();

    let replayed = store.replay(session, None, 100).unwrap();
    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0].content, "hi");
    assert_eq!(replayed[2].content, "<result>");
    // seq assigned server-side, monotonically.
    assert_eq!(replayed[0].seq, 1);
    assert_eq!(replayed[1].seq, 2);
    assert_eq!(replayed[2].seq, 3);
}

#[test]
fn replay_respects_after_seq_cursor() {
    let store = store();
    let session = "s1";
    for i in 0..5 {
        store.append(&msg(&format!("msg-{i}"), session, "user", &format!("m{i}"), None)).unwrap();
    }
    let tail = store.replay(session, Some(3), 100).unwrap();
    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0].seq, 4);
    assert_eq!(tail[1].seq, 5);
}

#[test]
fn tool_sequence_returns_tool_names_in_order() {
    let store = store();
    let session = "s1";
    store
        .append(&msg("msg-1", session, "assistant", "a", Some(r#"[{"tool_name":"read_file"},{"tool_name":"grep"}]"#)))
        .unwrap();
    store
        .append(&msg("msg-2", session, "assistant", "b", Some(r#"[{"tool_name":"write_file"}]"#)))
        .unwrap();
    store.append(&msg("msg-3", session, "user", "c", None)).unwrap(); // non-assistant ignored

    let seq = store.tool_sequence_for_session(session).unwrap();
    assert_eq!(seq, vec!["read_file", "grep", "write_file"]);
}

#[test]
fn concurrent_appends_yield_distinct_ordered_seqs() {
    // AC: seq assigned atomically (no TOCTOU). 2 threads x 100 appends -> 200
    // distinct, contiguous seqs 1..=200.
    let store = store();
    let session = "s1";
    let mut handles = Vec::new();
    for t in 0..2 {
        let store = Arc::clone(&store);
        let session = session.to_string();
        handles.push(std::thread::spawn(move || {
            for i in 0..100 {
                let id = format!("msg-{}-{}", t, i);
                let offset = t * 1000; // unique PKs across threads
                store
                    .append(&msg(&id, &session, "user", "x", None))
                    .map_err(|e| {
                        let _ = offset; // silence unused
                        e
                    })
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let all = store.replay(session, None, 10_000).unwrap();
    assert_eq!(all.len(), 200, "expected 200 messages");
    let seqs: Vec<i64> = all.iter().map(|m| m.seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort();
    let expected: Vec<i64> = (1..=200).collect();
    assert_eq!(sorted, expected, "seqs must be exactly 1..=200");
    let uniq: std::collections::HashSet<i64> = seqs.iter().copied().collect();
    assert_eq!(uniq.len(), 200, "seqs must be distinct");
}
