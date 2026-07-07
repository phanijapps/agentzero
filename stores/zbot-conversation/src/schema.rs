//! Schema v1 for the conversation store (tables owned by `zbot-conversation`).
//!
//! `messages` and `checkpoints` live in `conversations.db`; the legacy
//! `tool_results` column is dropped. `thread_summaries` is deferred (see
//! `docs/backlog.md` → conversation-store-revamp-summary-store).

use anyhow::Result;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

pub const SCHEMA_SQL: &str = r#"
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
    seq           INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_session_seq ON messages(session_id, seq);

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
CREATE INDEX IF NOT EXISTS idx_checkpoints_exec_turn ON checkpoints(execution_id, llm_turn DESC, created_at DESC);
"#;

/// Apply the conversation-store schema to a connection (idempotent).
pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}
