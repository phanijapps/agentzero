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
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}
