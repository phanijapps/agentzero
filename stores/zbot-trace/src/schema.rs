//! Schema for the trace store. `execution_logs` columns are **unchanged** from
//! the legacy table — the slimming is writer-discipline (T11), not a schema
//! change, so the live `/api/logs` UI keeps working with the same SQL.

use anyhow::Result;
use rusqlite::Connection;

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS execution_logs (
    id                TEXT PRIMARY KEY,
    session_id        TEXT NOT NULL,
    conversation_id   TEXT,
    agent_id          TEXT NOT NULL,
    parent_session_id TEXT,
    timestamp         TEXT NOT NULL,
    level             TEXT NOT NULL,
    category          TEXT NOT NULL,
    message           TEXT NOT NULL,
    metadata          TEXT,
    duration_ms       INTEGER
);
CREATE INDEX IF NOT EXISTS idx_logs_session   ON execution_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON execution_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_logs_agent     ON execution_logs(agent_id);
"#;

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}
