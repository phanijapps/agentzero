use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use zbot_conversation::{open_conversation_pool, SessionMetaStore, SqliteSessionMetaStore};

fn store() -> (Pool<SqliteConnectionManager>, SqliteSessionMetaStore) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let pool = open_conversation_pool(file.path()).unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                root_agent_id TEXT NOT NULL,
                ward_id TEXT
            );
            "#,
        )
        .unwrap();
    }
    let store = SqliteSessionMetaStore::new(pool.clone());
    (pool, store)
}

#[test]
fn reads_session_ward_and_agent() {
    let (pool, store) = store();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, root_agent_id, ward_id) VALUES (?1, ?2, ?3)",
            ("sess-1", "root-agent", "ward-alpha"),
        )
        .unwrap();
    }

    assert_eq!(
        store.session_ward_id("sess-1").unwrap().as_deref(),
        Some("ward-alpha")
    );
    assert_eq!(
        store.session_agent_id("sess-1").unwrap().as_deref(),
        Some("root-agent")
    );
}

#[test]
fn missing_session_returns_none() {
    let (_pool, store) = store();
    assert!(store.session_ward_id("missing").unwrap().is_none());
    assert!(store.session_agent_id("missing").unwrap().is_none());
}
