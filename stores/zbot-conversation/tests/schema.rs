use rusqlite::Connection;
use zbot_conversation::schema;

#[test]
fn schema_initializes_all_tables() {
    let conn = Connection::open_in_memory().unwrap();
    schema::initialize(&conn).unwrap();
    for table in ["messages", "checkpoints"] {
        let n: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "table {table} missing");
    }
}

#[test]
fn schema_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    schema::initialize(&conn).unwrap();
    schema::initialize(&conn).unwrap(); // second apply must not error
}

#[test]
fn schema_tolerates_legacy_messages_table_without_seq() {
    // Reproduces the daemon-start crash: the pre-cutover `messages` table
    // (created by DatabaseManager) has no `seq` column. `initialize` must add
    // `seq` and create the index without error.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE messages (
            id TEXT PRIMARY KEY, execution_id TEXT, session_id TEXT NOT NULL,
            role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL,
            token_count INTEGER DEFAULT 0, tool_calls TEXT, tool_results TEXT, tool_call_id TEXT
        );",
    )
    .unwrap();

    schema::initialize(&conn).unwrap(); // must not panic on the seq index

    let has_seq: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='seq'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_seq, 1, "seq column added to legacy table");

    let has_idx: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_messages_session_seq'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_idx, 1, "seq index created on legacy table");
}
