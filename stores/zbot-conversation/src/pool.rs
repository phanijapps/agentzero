//! Connection pool for the conversation store.
//!
//! Opens an `r2d2` pool to `conversations.db` with the same WAL pragmas as
//! `DatabaseManager` (`stores/zbot-stores-sqlite/src/connection.rs:38-52`) but
//! does **not** depend on `DatabaseManager`. The new stores share one pool
//! constructed in the gateway (spec §Constraints).

use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::schema;

/// Open a pool to the conversations database and initialize its schema.
pub fn open_conversation_pool(path: &std::path::Path) -> Result<Pool<SqliteConnectionManager>> {
    let manager = SqliteConnectionManager::file(path);
    let pool = Pool::builder()
        .max_size(8)
        .connection_customizer(Box::new(Fixtures))
        .build(manager)?;
    {
        let conn = pool.get()?;
        schema::initialize(&conn)?;
    }
    Ok(pool)
}

#[derive(Debug)]
struct Fixtures;

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for Fixtures {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(())
    }
}
