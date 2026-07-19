use rusqlite::Connection;
use zbot_trace::schema;

#[test]
fn schema_initializes_execution_logs_and_indexes() {
    let conn = Connection::open_in_memory().unwrap();
    schema::initialize(&conn).unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='execution_logs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let idx: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN
             ('idx_logs_session','idx_logs_timestamp','idx_logs_agent')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 3, "expected the 3 execution_logs indexes");
}
