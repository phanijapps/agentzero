//! Vector similarity index backed by sqlite-vec (`vec0`) virtual tables.
//!
//! Replaces hand-rolled cosine scans across the codebase. Every similarity
//! query in the memory layer routes through this trait.
//!
//! Embeddings MUST be L2-normalized before `upsert`. Distance returned by
//! `query_nearest` is L2 squared; for normalized vectors cosine similarity
//! equals `1.0 - dist / 2.0`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::KnowledgeDatabase;

pub trait VectorIndex: Send + Sync {
    /// Insert or replace the embedding for `id`. Embeddings must be L2-normalized.
    fn upsert(&self, id: &str, embedding: &[f32]) -> Result<(), String>;

    /// Remove the embedding for `id`. No-op if the id is absent.
    fn delete(&self, id: &str) -> Result<(), String>;

    /// Return up to `limit` nearest neighbors as `(id, L2_squared_distance)`.
    /// Callers convert to cosine similarity when needed.
    fn query_nearest(&self, embedding: &[f32], limit: usize) -> Result<Vec<(String, f32)>, String>;

    /// The embedding dimension this index expects. Implementations that don't
    /// track a dimension return `0`; callers treating `0` as "unknown" should
    /// fall back to a configured constant.
    fn dim(&self) -> usize {
        0
    }
}

/// sqlite-vec-backed VectorIndex over a single `vec0` virtual table.
///
/// `dim` is cached from the table's DDL at construction, but the reindex
/// pipeline may drop + recreate the underlying vec0 table at a different
/// dimension after we've been built. To tolerate that, we re-read the DDL
/// lazily whenever an incoming embedding's length disagrees with the
/// cached value; if the fresh DDL still disagrees, that's a real caller
/// bug and we report the mismatch.
pub struct SqliteVecIndex {
    db: Arc<KnowledgeDatabase>,
    table: &'static str,
    id_column: &'static str,
    dim: AtomicUsize,
}

impl SqliteVecIndex {
    /// Construct a wrapper over a vec0 virtual table. Reads the table's DDL
    /// via `sqlite_master` to discover the embedding dimension — the schema
    /// is the single source of truth. No call site ever passes `dim`.
    ///
    /// Returns an error if the table doesn't exist, isn't a vec0 table, or
    /// the DDL doesn't contain a parseable `FLOAT[N]` column.
    pub fn new(
        db: Arc<KnowledgeDatabase>,
        table: &'static str,
        id_column: &'static str,
    ) -> Result<Self, String> {
        let dim = read_table_dim(&db, table)?;
        Ok(Self {
            db,
            table,
            id_column,
            dim: AtomicUsize::new(dim),
        })
    }

    /// The embedding dimension this index expects (cached from the DDL).
    pub fn dim(&self) -> usize {
        self.dim.load(Ordering::Relaxed)
    }

    /// Verify `len` matches the expected dim. If it doesn't, re-read the DDL
    /// (the reindex pipeline may have swapped the table under us) and update
    /// the cached dim before deciding. Returns `Ok(())` if the call can
    /// proceed, or an `Err` message describing the live mismatch.
    fn check_dim(&self, len: usize) -> Result<(), String> {
        let cached = self.dim.load(Ordering::Relaxed);
        if len == cached {
            return Ok(());
        }
        // Cached value is stale? Re-read and self-heal.
        let fresh = read_table_dim(&self.db, self.table)?;
        if fresh != cached {
            self.dim.store(fresh, Ordering::Relaxed);
            tracing::info!(
                "SqliteVecIndex({}): dim refreshed from DDL {} → {}",
                self.table,
                cached,
                fresh
            );
        }
        if len != fresh {
            return Err(format!(
                "embedding dim mismatch: got {}, expected {}",
                len, fresh
            ));
        }
        Ok(())
    }
}

/// Look up `table`'s DDL in `sqlite_master` and parse the `FLOAT[N]` width.
fn read_table_dim(db: &Arc<KnowledgeDatabase>, table: &str) -> Result<usize, String> {
    let sql: String = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                rusqlite::params![table],
                |r| r.get::<_, String>(0),
            )
        })
        .map_err(|e| format!("table {table} not found in sqlite_master: {e}"))?;
    extract_dim_from_ddl(&sql)
        .ok_or_else(|| format!("could not parse FLOAT[N] dim from {table}'s DDL: {sql}"))
}

/// Extract `N` from the first occurrence of `FLOAT[N]` in `ddl`.
/// Tolerant of case (`float[1024]` works) and whitespace inside brackets.
pub(crate) fn extract_dim_from_ddl(ddl: &str) -> Option<usize> {
    let upper = ddl.to_ascii_uppercase();
    let start = upper.find("FLOAT[")?;
    let after = &ddl[start + "FLOAT[".len()..];
    let end = after.find(']')?;
    after[..end].trim().parse::<usize>().ok()
}

impl VectorIndex for SqliteVecIndex {
    fn dim(&self) -> usize {
        SqliteVecIndex::dim(self)
    }

    fn upsert(&self, id: &str, embedding: &[f32]) -> Result<(), String> {
        self.check_dim(embedding.len())?;
        let embedding_json =
            serde_json::to_string(embedding).map_err(|e| format!("serialize embedding: {e}"))?;
        let embedding_col = embedding_column_name(self.table);
        let sql_delete = format!("DELETE FROM {} WHERE {} = ?1", self.table, self.id_column);
        let sql_insert = format!(
            "INSERT INTO {} ({}, {}) VALUES (?1, ?2)",
            self.table, self.id_column, embedding_col
        );
        self.db.with_connection(|conn| {
            conn.execute(&sql_delete, rusqlite::params![id])?;
            conn.execute(&sql_insert, rusqlite::params![id, &embedding_json])?;
            Ok(())
        })
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        let sql = format!("DELETE FROM {} WHERE {} = ?1", self.table, self.id_column);
        self.db.with_connection(|conn| {
            conn.execute(&sql, rusqlite::params![id])?;
            Ok(())
        })
    }

    fn query_nearest(&self, embedding: &[f32], limit: usize) -> Result<Vec<(String, f32)>, String> {
        self.check_dim(embedding.len())?;
        let embedding_json =
            serde_json::to_string(embedding).map_err(|e| format!("serialize embedding: {e}"))?;
        let embedding_col = embedding_column_name(self.table);
        let sql = format!(
            "SELECT {}, distance FROM {} WHERE {} MATCH ?1 ORDER BY distance LIMIT ?2",
            self.id_column, self.table, embedding_col
        );
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![embedding_json, limit as i64], |r| {
                let id: String = r.get(0)?;
                let dist: f32 = r.get(1)?;
                Ok((id, dist))
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }
}

/// Map a vec0 table name to its embedding column name (stable per v22 schema).
fn embedding_column_name(table: &str) -> &'static str {
    match table {
        "kg_name_index" => "name_embedding",
        "memory_facts_index"
        | "wiki_articles_index"
        | "procedures_index"
        | "session_episodes_index" => "embedding",
        _ => "embedding",
    }
}

#[cfg(test)]
mod tests {
    use super::extract_dim_from_ddl;

    #[test]
    fn parses_basic_float_dim() {
        assert_eq!(
            extract_dim_from_ddl(
                "CREATE VIRTUAL TABLE memory_facts_index USING vec0(fact_id TEXT PRIMARY KEY, embedding FLOAT[1024])"
            ),
            Some(1024)
        );
    }

    #[test]
    fn parses_dim_384() {
        assert_eq!(
            extract_dim_from_ddl(
                "CREATE VIRTUAL TABLE x USING vec0(id TEXT, embedding FLOAT[384])"
            ),
            Some(384)
        );
    }

    #[test]
    fn parses_lowercase_float() {
        assert_eq!(
            extract_dim_from_ddl(
                "CREATE VIRTUAL TABLE x USING vec0(id TEXT, embedding float[768])"
            ),
            Some(768)
        );
    }

    #[test]
    fn returns_none_when_no_float_column() {
        assert_eq!(
            extract_dim_from_ddl("CREATE TABLE x (id INTEGER, name TEXT)"),
            None
        );
    }

    #[test]
    fn returns_none_for_malformed_dim() {
        assert_eq!(
            extract_dim_from_ddl("CREATE VIRTUAL TABLE x USING vec0(embedding FLOAT[abc])"),
            None
        );
    }

    #[test]
    fn tolerates_whitespace_in_brackets() {
        assert_eq!(
            extract_dim_from_ddl("CREATE VIRTUAL TABLE x USING vec0(embedding FLOAT[ 512 ])"),
            Some(512)
        );
    }
}
