//! Narrow session metadata reads for runtime consumers.
//!
//! This replaces the metadata-only methods on the legacy
//! `ConversationRepository` without growing a broad conversation facade.

use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;

pub trait SessionMetaStore: Send + Sync {
    /// Return the session's active ward id, if one is recorded.
    fn session_ward_id(&self, session_id: &str) -> Result<Option<String>>;

    /// Return the root agent id recorded on the session.
    fn session_agent_id(&self, session_id: &str) -> Result<Option<String>>;
}

pub struct SqliteSessionMetaStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteSessionMetaStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }
}

impl SessionMetaStore for SqliteSessionMetaStore {
    fn session_ward_id(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.pool.get()?;
        let ward_id = conn
            .query_row(
                "SELECT ward_id FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(ward_id)
    }

    fn session_agent_id(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.pool.get()?;
        let agent_id = conn
            .query_row(
                "SELECT root_agent_id FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(agent_id)
    }
}
