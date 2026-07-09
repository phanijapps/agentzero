//! `KnowledgeDatabase` — r2d2 pool for `knowledge.db` with sqlite-vec
//! extension auto-loaded on every connection.

use gateway_services::SharedVaultPaths;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::time::Duration;

use crate::knowledge_schema::{
    cleanup_orphan_reindex_tables, drop_and_recreate_vec_tables_at_dim,
    initialize_knowledge_database, initialize_vec_tables_with_dim,
};
use crate::sqlite_vec_loader::load_sqlite_vec;
use crate::system_profile;

/// Connection pool for `knowledge.db`.
///
/// Every connection has sqlite-vec loaded and WAL-mode pragmas applied.
/// Schema v22 is initialized on first construction.
pub struct KnowledgeDatabase {
    pool: Pool<SqliteConnectionManager>,
}

/// Customizer: applies WAL pragmas and loads sqlite-vec on every acquired connection.
///
/// `cache_size` and `mmap_size` are sourced from [`system_profile`] so
/// the knowledge DB scales with the host (Pi → laptop → CI runner).
#[derive(Debug)]
struct KnowledgeConnectionCustomizer {
    cache_kib: u32,
    mmap_bytes: u64,
}

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for KnowledgeConnectionCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -{cache_kib};
             PRAGMA mmap_size = {mmap_bytes};
             PRAGMA busy_timeout = 5000;
             PRAGMA wal_autocheckpoint = 1000;
             PRAGMA temp_store = MEMORY;
             PRAGMA foreign_keys = ON;",
            cache_kib = self.cache_kib,
            mmap_bytes = self.mmap_bytes,
        ))?;
        load_sqlite_vec(conn)?;
        register_math_udfs(conn)?;
        Ok(())
    }
}

/// MEM-004: register the math UDFs the DecayEngine needs for bulk
/// UPDATE statements. rusqlite's `bundled` feature ships SQLite
/// without `SQLITE_ENABLE_MATH_FUNCTIONS`, so `exp()` isn't natively
/// available. We register a tiny Rust-side `exp(x)` so the bulk
/// confidence-decay UPDATE can do the math server-side without
/// pulling every row into Rust first. Pure, deterministic, no I/O —
/// safe to call from any query.
fn register_math_udfs(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.create_scalar_function(
        "exp",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC
            | rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        |ctx| {
            let x: f64 = ctx.get(0)?;
            Ok(x.exp())
        },
    )?;
    Ok(())
}

impl KnowledgeDatabase {
    /// Create a new knowledge database manager.
    ///
    /// Creates the data directory if it doesn't exist, builds an r2d2 pool
    /// of up to 8 connections (each with sqlite-vec loaded), and initializes
    /// the v22 schema + vec0 tables. Idempotent — safe to call on an
    /// already-initialized DB.
    pub fn new(paths: SharedVaultPaths) -> Result<Self, String> {
        // Default constructor reads the active embedding dim from the marker
        // file if present, falling back to 384. Keeps the widely-used call
        // sites unchanged while honoring fresh-install user choices.
        let dim = read_indexed_dim_or_default(&paths, 384);
        Self::new_with_dim(paths, dim)
    }

    /// Construct with an explicit sqlite-vec dimension. Used by callers that
    /// know the active `EmbeddingService` dim (e.g. daemon boot with an
    /// Ollama-backed 1024d config).
    pub fn new_with_dim(paths: SharedVaultPaths, dim: usize) -> Result<Self, String> {
        let db_path = paths.knowledge_db();

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create data dir: {e}"))?;
        }

        let pool_max = system_profile::pool_max_size();
        let cache_kib = system_profile::cache_size_kib();
        let mmap_bytes = system_profile::mmap_size_bytes();

        let manager = SqliteConnectionManager::file(&db_path);
        let pool = Pool::builder()
            .max_size(pool_max)
            .min_idle(Some(2))
            .connection_timeout(Duration::from_secs(15))
            .connection_customizer(Box::new(KnowledgeConnectionCustomizer {
                cache_kib,
                mmap_bytes,
            }))
            .build(manager)
            .map_err(|e| format!("Failed to create knowledge pool: {e}"))?;

        // Initialize schema + vec tables on a single connection from the pool.
        {
            let conn = pool
                .get()
                .map_err(|e| format!("Failed to get init connection: {e}"))?;
            initialize_knowledge_database(&conn)
                .map_err(|e| format!("Failed to init knowledge schema: {e}"))?;
            initialize_vec_tables_with_dim(&conn, dim)
                .map_err(|e| format!("Failed to init vec tables: {e}"))?;
            // Boot-time crash recovery: drop any orphan `*__new` reindex
            // tables left behind by a previous mid-reindex crash.
            cleanup_orphan_reindex_tables(&conn)
                .map_err(|e| format!("Failed to cleanup orphan reindex tables: {e}"))?;
        }

        tracing::info!(
            target: "zbot_sqlite",
            "knowledge.db pool: path={:?} pool_max={} cache_kib={} mmap_mib={}",
            db_path,
            pool_max,
            cache_kib,
            mmap_bytes / (1024 * 1024),
        );

        Ok(Self { pool })
    }

    /// Borrow a connection from the pool and run `f`.
    pub fn with_connection<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce(&Connection) -> Result<R, rusqlite::Error>,
    {
        let conn = self
            .pool
            .get()
            .map_err(|e| format!("Failed to get knowledge connection: {e}"))?;
        f(&conn).map_err(|e| format!("Knowledge DB operation failed: {e}"))
    }

    /// Drop-and-recreate every vec0 index table at `new_dim`.
    ///
    /// Used by the boot-time dim reconciler in `AppState::new` when the
    /// configured `EmbeddingService` dimensions disagree with the
    /// `.embedding-state` marker. Data loss is intentional — source rows
    /// get re-embedded by the reindex pipeline at the next sleep cycle.
    /// Recall returns empty results in the interim instead of blowing up
    /// with `no such table` / `embedding dim mismatch`.
    ///
    /// # Errors
    ///
    /// Propagates any DDL error from the connection.
    pub fn reconcile_vec_tables_dim(&self, new_dim: usize) -> Result<(), String> {
        self.with_connection(|conn| drop_and_recreate_vec_tables_at_dim(conn, new_dim))
    }
}

/// Read `dim=<usize>` from `data/.embedding-state` without depending on
/// `EmbeddingService`. Returns `default` on any IO/parse failure.
fn read_indexed_dim_or_default(paths: &SharedVaultPaths, default: usize) -> usize {
    let marker = paths.data_dir().join(".embedding-state");
    let Ok(text) = std::fs::read_to_string(marker) else {
        return default;
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("dim=") {
            if let Ok(n) = rest.trim().parse::<usize>() {
                return n;
            }
        }
    }
    default
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::VaultPaths;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn new_defaults_to_384_when_no_marker() {
        let tmp = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let db = KnowledgeDatabase::new(paths).unwrap();
        let sql: String = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT sql FROM sqlite_master WHERE name='memory_facts_index'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert!(sql.contains("FLOAT[384]"), "got: {sql}");
    }

    #[test]
    fn new_honors_marker_dim() {
        let tmp = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        std::fs::write(
            paths.data_dir().join(".embedding-state"),
            "backend=ollama\nmodel=mxbai-embed-large\ndim=1024\nindexed_at=x\n",
        )
        .unwrap();
        let db = KnowledgeDatabase::new(paths).unwrap();
        let sql: String = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT sql FROM sqlite_master WHERE name='memory_facts_index'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert!(sql.contains("FLOAT[1024]"), "got: {sql}");
    }

    #[test]
    fn new_with_dim_uses_explicit_dim() {
        let tmp = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let db = KnowledgeDatabase::new_with_dim(paths, 768).unwrap();
        let sql: String = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT sql FROM sqlite_master WHERE name='kg_name_index'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert!(sql.contains("FLOAT[768]"), "got: {sql}");
    }

    /// Fix 2: boot-time dim reconcile drops + recreates tables at the new
    /// dim even when the marker pinned a different dim.
    #[test]
    fn reconcile_vec_tables_dim_drops_and_recreates_at_new_dim() {
        let tmp = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();

        // Seed marker at 384 so the DB comes up at 384.
        std::fs::write(
            paths.data_dir().join(".embedding-state"),
            "backend=internal\nmodel=fastembed/all-MiniLM-L6-v2\ndim=384\nindexed_at=x\n",
        )
        .unwrap();

        let db = KnowledgeDatabase::new(paths).unwrap();

        // Sanity: schema currently at 384.
        let sql_384: String = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT sql FROM sqlite_master WHERE name='memory_facts_index'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert!(sql_384.contains("FLOAT[384]"), "got: {sql_384}");

        // Act: reconcile to 1024 (what happens when the user picks an
        // Ollama 1024-d backend).
        db.reconcile_vec_tables_dim(1024).unwrap();

        // All 5 vec0 tables must now be at 1024.
        for name in crate::knowledge_schema::REQUIRED_VEC_TABLES {
            let sql: String = db
                .with_connection(|c| {
                    c.query_row(
                        "SELECT sql FROM sqlite_master WHERE name=?1",
                        rusqlite::params![name],
                        |r| r.get(0),
                    )
                })
                .unwrap();
            assert!(sql.contains("FLOAT[1024]"), "{name} not at 1024: {sql}");
        }
    }
}
