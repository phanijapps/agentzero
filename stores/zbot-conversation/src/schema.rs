//! Schema v1 for the conversation store (tables owned by `zbot-conversation`).
//!
//! `messages` and `checkpoints` live in `conversations.db`; the legacy
//! `tool_results` column is dropped. `thread_summaries` is deferred (see
//! `docs/backlog.md` → conversation-store-revamp-summary-store).
//!
//! **Cutover compatibility:** during the cutover the legacy `messages` table
//! (created by `zbot-stores-sqlite`'s `DatabaseManager`, which lacks `seq`) may
//! already exist. `initialize` adds `seq` if absent before creating the index,
//! so the two schemas don't collide. This shim is removed when T16 drops the
//! old DDL.

use anyhow::Result;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

const TABLES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id            TEXT PRIMARY KEY,
    execution_id  TEXT,
    session_id    TEXT NOT NULL,
    role          TEXT NOT NULL,
    content       TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    token_count   INTEGER NOT NULL DEFAULT 0,
    tool_calls    TEXT,
    tool_call_id  TEXT,
    seq           INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS checkpoints (
    id                  TEXT PRIMARY KEY,
    execution_id        TEXT NOT NULL,
    session_id          TEXT NOT NULL,
    llm_turn            INTEGER NOT NULL,
    last_message_id     TEXT NOT NULL,
    pending_tool_calls  TEXT,
    context_state       TEXT,
    child_executions    TEXT,
    schema_version      INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT NOT NULL
);
"#;

const INDEXES_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_messages_session_seq ON messages(session_id, seq);
CREATE INDEX IF NOT EXISTS idx_checkpoints_exec_turn ON checkpoints(execution_id, llm_turn DESC, created_at DESC);
"#;

/// Apply the conversation-store schema to a connection (idempotent, and
/// tolerant of the legacy pre-cutover `messages` table).
pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(TABLES_SQL)?;
    ensure_messages_seq_column(conn)?;
    conn.execute_batch(INDEXES_SQL)?;
    Ok(())
}

/// If the legacy `messages` table exists without `seq` (created by
/// `DatabaseManager`), add it so the `(session_id, seq)` index can be created.
fn ensure_messages_seq_column(conn: &Connection) -> Result<()> {
    let has_seq = conn
        .prepare("PRAGMA table_info(messages)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|col| col == "seq");
    if !has_seq {
        conn.execute(
            "ALTER TABLE messages ADD COLUMN seq INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}
