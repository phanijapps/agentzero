//! Connection pool for the trace store (`execution_logs`). Same WAL pragmas as
//! `zbot_conversation::pool`; independent of `DatabaseManager`.

use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::schema;

pub fn open_trace_pool(path: &std::path::Path) -> Result<Pool<SqliteConnectionManager>> {
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
