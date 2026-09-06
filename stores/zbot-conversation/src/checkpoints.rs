//! `CheckpointStore` — versioned agent-state snapshots (mutable, latest-wins).
//!
//! Promotes the legacy inline `agent_executions.checkpoint` blob into a
//! first-class versioned table. `latest` returns the most recent snapshot for an
//! execution by `(llm_turn DESC, created_at DESC)` — not by UUIDv7 string order
//! (which is only millisecond-monotonic). `session_state.rs` reads from here
//! (O(1) snapshot) instead of replaying `execution_logs` + `messages`.

use crate::domain::Checkpoint;
use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub trait CheckpointStore: Send + Sync {
    fn write(&self, cp: &Checkpoint) -> Result<()>;
    fn latest(&self, execution_id: &str) -> Result<Option<Checkpoint>>;
}

pub struct SqliteCheckpointStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteCheckpointStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }
}

impl CheckpointStore for SqliteCheckpointStore {
    fn write(&self, cp: &Checkpoint) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO checkpoints
                (id, execution_id, session_id, llm_turn, last_message_id,
                 pending_tool_calls, context_state, child_executions,
                 schema_version, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                cp.id,
                cp.execution_id,
                cp.session_id,
                cp.llm_turn,
                cp.last_message_id,
                cp.pending_tool_calls,
                cp.context_state,
                cp.child_executions,
                cp.schema_version,
                cp.created_at,
            ],
        )?;
        Ok(())
    }

    fn latest(&self, execution_id: &str) -> Result<Option<Checkpoint>> {
        let conn = self.pool.get()?;
        let cp = conn
            .query_row(
                "SELECT id, execution_id, session_id, llm_turn, last_message_id,
                        pending_tool_calls, context_state, child_executions,
                        schema_version, created_at
                 FROM checkpoints WHERE execution_id = ?
                 ORDER BY created_at DESC, llm_turn DESC LIMIT 1",
                [execution_id],
                |r| {
                    Ok(Checkpoint {
                        id: r.get(0)?,
                        execution_id: r.get(1)?,
                        session_id: r.get(2)?,
                        llm_turn: r.get(3)?,
                        last_message_id: r.get(4)?,
                        pending_tool_calls: r.get(5)?,
                        context_state: r.get(6)?,
                        child_executions: r.get(7)?,
                        schema_version: r.get(8)?,
                        created_at: r.get(9)?,
                    })
                },
            )
            .ok();
        Ok(cp)
    }
}
