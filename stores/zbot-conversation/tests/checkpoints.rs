use tempfile::NamedTempFile;
use zbot_conversation::{
    open_conversation_pool, Checkpoint, CheckpointStore, SqliteCheckpointStore,
};

fn store() -> SqliteCheckpointStore {
    let f = NamedTempFile::new().unwrap();
    SqliteCheckpointStore::new(open_conversation_pool(f.path()).unwrap())
}

fn cp(exec: &str, turn: u32, context: Option<&str>) -> Checkpoint {
    Checkpoint {
        id: format!("cp-{exec}-{turn}"),
        execution_id: exec.to_string(),
        session_id: "s1".to_string(),
        llm_turn: turn,
        last_message_id: format!("msg-{turn}"),
        pending_tool_calls: None,
        context_state: context.map(String::from),
        child_executions: None,
        schema_version: 1,
        created_at: format!("2026-07-07T00:00:0{}Z", turn),
    }
}

#[test]
fn latest_returns_highest_turn() {
    let store = store();
    let exec = "e1";
    store.write(&cp(exec, 1, None)).unwrap();
    store.write(&cp(exec, 2, None)).unwrap();
    store
        .write(&cp(exec, 3, Some(r#"{"ward":"ward-a"}"#)))
        .unwrap();

    let latest = store.latest(exec).unwrap().expect("a checkpoint");
    assert_eq!(latest.llm_turn, 3);
    assert_eq!(
        latest.context_state.as_deref(),
        Some(r#"{"ward":"ward-a"}"#)
    );
}

#[test]
fn latest_returns_none_for_unknown_execution() {
    let store = store();
    assert!(store.latest("nope").unwrap().is_none());
}

#[test]
fn latest_tiebreaks_on_created_at() {
    // Same llm_turn across two rows — created_at decides.
    let store = store();
    let exec = "e1";
    let mut a = cp(exec, 5, None);
    a.id = "cp-a".into();
    a.created_at = "2026-07-07T00:00:00Z".into();
    let mut b = cp(exec, 5, None);
    b.id = "cp-b".into();
    b.created_at = "2026-07-07T00:00:09Z".into();
    store.write(&a).unwrap();
    store.write(&b).unwrap();

    let latest = store.latest(exec).unwrap().expect("a checkpoint");
    assert_eq!(latest.id, "cp-b", "later created_at wins the tie");
}
