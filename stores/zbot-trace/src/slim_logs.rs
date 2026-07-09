//! `SlimLogStore` — payload-free `execution_logs` rows for the live `/api/logs`
//! UI. The store stores whatever `metadata` it is given; the *writers*
//! (gateway `event_logging.rs`, rewired in T11) are what keep `metadata` to
//! display scalars only (`{tool_name, tool_id, error, blocked_by_hook}`) — never
//! tool args/result payloads.

use crate::domain::SlimLog;
use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub trait SlimLogStore: Send + Sync {
    fn append(&self, log: &SlimLog) -> Result<()>;
    fn query(&self, session_id: &str, limit: usize) -> Result<Vec<SlimLog>>;
}

pub struct SqliteSlimLogStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteSlimLogStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }
}

impl SlimLogStore for SqliteSlimLogStore {
    fn append(&self, log: &SlimLog) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO execution_logs
                (id, session_id, conversation_id, agent_id, parent_session_id,
                 timestamp, level, category, message, metadata, duration_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                log.id,
                log.session_id,
                log.conversation_id,
                log.agent_id,
                log.parent_session_id,
                log.timestamp,
                log.level,
                log.category,
                log.message,
                log.metadata,
                log.duration_ms,
            ],
        )?;
        Ok(())
    }

    fn query(&self, session_id: &str, limit: usize) -> Result<Vec<SlimLog>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, conversation_id, agent_id, parent_session_id,
                    timestamp, level, category, message, metadata, duration_ms
             FROM execution_logs WHERE session_id = ?
             ORDER BY timestamp ASC LIMIT ?",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id, limit as i64], |r| {
            Ok(SlimLog {
                id: r.get(0)?,
                session_id: r.get(1)?,
                conversation_id: r.get(2)?,
                agent_id: r.get(3)?,
                parent_session_id: r.get(4)?,
                timestamp: r.get(5)?,
                level: r.get(6)?,
                category: r.get(7)?,
                message: r.get(8)?,
                metadata: r.get(9)?,
                duration_ms: r.get(10)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }
}
