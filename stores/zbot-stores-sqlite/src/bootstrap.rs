//! Schema bootstrap for the SQLite KG impl.
//!
//! Delegates to [`KnowledgeDatabase::new`] which runs
//! `initialize_knowledge_database` — that owns the full schema DDL plus
//! the historical inline migrations (v1-v22) and the
//! `migrations/v23, v24.sql` files.
//!
//! The function below is kept as the canonical hook point for the
//! SQLite backend's bootstrap so a future alternate backend's
//! `bootstrap.rs` follows the same shape.

pub use crate::knowledge_db::KnowledgeDatabase;

/// Idempotent schema bootstrap. Called by `SqliteKgStore` constructors
/// when running against a fresh data directory. Today this is a no-op
/// because [`KnowledgeDatabase::new`] already runs the bootstrap as a
/// side effect; kept here as the canonical hook point for the SQLite
/// backend.
pub fn bootstrap_schema(_db: &KnowledgeDatabase) -> Result<(), String> {
    // No-op: KnowledgeDatabase::new already runs the bootstrap.
    // Future SurrealDB impl will have non-trivial logic here.
    Ok(())
}
