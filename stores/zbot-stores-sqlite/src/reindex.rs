//! # Embedding Reindex (SQLite-backed)
//!
//! Rebuilds the sqlite-vec virtual tables when the active embedding dimension
//! (or backend/model) changes. Each target table follows the same routine:
//!
//! 1. `DROP TABLE IF EXISTS {table}__new` — crash-safe cleanup of any
//!    previous half-swap (kept for historical compatibility).
//! 2. `DROP TABLE IF EXISTS {table}` then
//!    `CREATE VIRTUAL TABLE {table} USING vec0(...FLOAT[new_dim])`
//!    — sqlite-vec does not support `ALTER TABLE ... RENAME` on vec0
//!    virtual tables, so we drop-and-recreate in place.
//! 3. Stream source rows in batches of 100, embed via the active client, and
//!    insert into `{table}`.
//!
//! Per-row embedding failures are logged + skipped rather than aborting the
//! whole reindex.
//!
//! This module previously lived at
//! `gateway/gateway-execution/src/sleep/embedding_reindex.rs`. It moved here
//! so that `KnowledgeGraphStore::reindex_embeddings` (the trait surface the
//! sleep job calls into) can own the SQLite-specific schema rebuild without
//! the gateway crate having to know which physical layout the store uses.
//! See `memory-bank/tech-debt.md` TD-012 (Phase 3d).
//!
//! The orchestration is exposed as `pub` so the gateway-side wrapper at
//! `gateway/gateway-execution/src/sleep/embedding_reindex.rs` can keep its
//! progress-callback API for the UI's `EmbeddingProgressModal` — the trait
//! method itself is "fire and report" with no progress callback.

use std::sync::Arc;

use crate::KnowledgeDatabase;
use agent_runtime::llm::EmbeddingClient;

const BATCH_SIZE: usize = 100;

/// Conservative character cap per input sent to an embedding provider.
///
/// The smallest context window across providers we target is 512 tokens
/// (Snowflake Arctic Embed family, BGE-small). At ~4 chars per token that
/// maps to ~2000 characters. Truncating here prevents HTTP 400
/// "input length exceeds the context length" from crashing a batch.
/// Callers that need full-document fidelity should chunk upstream.
const MAX_CHARS_PER_EMBED_INPUT: usize = 2000;

fn truncate_for_embedding(text: &str) -> &str {
    match text.char_indices().nth(MAX_CHARS_PER_EMBED_INPUT) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

/// Target of a single reindex pass.
#[derive(Debug, Clone, Copy)]
pub struct ReindexTarget {
    pub table: &'static str,
    pub id_column: &'static str,
    pub embedding_column: &'static str,
    pub source_table: &'static str,
    pub source_id_column: &'static str,
    pub source_text_column: &'static str,
    /// Extra `WHERE` fragment (without the leading `AND`) or empty string.
    pub extra_filter: &'static str,
}

/// The five reindex targets touched by backend swaps.
pub const REINDEX_TARGETS: &[ReindexTarget] = &[
    ReindexTarget {
        table: "memory_facts_index",
        id_column: "fact_id",
        embedding_column: "embedding",
        source_table: "memory_facts",
        source_id_column: "id",
        source_text_column: "content",
        extra_filter: "",
    },
    ReindexTarget {
        table: "kg_name_index",
        id_column: "entity_id",
        embedding_column: "name_embedding",
        source_table: "kg_entities",
        source_id_column: "id",
        source_text_column: "name",
        extra_filter: "",
    },
    ReindexTarget {
        table: "session_episodes_index",
        id_column: "episode_id",
        embedding_column: "embedding",
        source_table: "session_episodes",
        source_id_column: "id",
        source_text_column: "task_summary",
        extra_filter: "task_summary IS NOT NULL",
    },
    ReindexTarget {
        table: "wiki_articles_index",
        id_column: "article_id",
        embedding_column: "embedding",
        source_table: "ward_wiki_articles",
        source_id_column: "id",
        source_text_column: "content",
        extra_filter: "content IS NOT NULL AND content != ''",
    },
    ReindexTarget {
        table: "procedures_index",
        id_column: "procedure_id",
        embedding_column: "embedding",
        source_table: "procedures",
        source_id_column: "id",
        source_text_column: "description",
        extra_filter: "description IS NOT NULL AND description != ''",
    },
];

/// Summary returned by a single-table reindex.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReindexSummary {
    pub indexed: usize,
    pub skipped: usize,
    pub total: usize,
}

/// Progress callback: `(table, current, total)`.
pub type ProgressFn<'a> = &'a (dyn Fn(&'static str, usize, usize) + Send + Sync);

// ============================================================================
// Per-table reindex
// ============================================================================

fn count_source_rows(db: &KnowledgeDatabase, target: &ReindexTarget) -> Result<usize, String> {
    let where_clause = if target.extra_filter.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", target.extra_filter)
    };
    let sql = format!(
        "SELECT COUNT(*) FROM {}{}",
        target.source_table, where_clause
    );
    db.with_connection(|conn| {
        let n: i64 = conn.query_row(&sql, [], |r| r.get(0))?;
        Ok(n as usize)
    })
}

fn fetch_batch(
    db: &KnowledgeDatabase,
    target: &ReindexTarget,
    last_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let where_extra = if target.extra_filter.is_empty() {
        String::new()
    } else {
        format!(" AND {}", target.extra_filter)
    };
    let sql = format!(
        "SELECT {id}, {text} FROM {tbl} WHERE {id} > ?1{extra} ORDER BY {id} LIMIT {lim}",
        id = target.source_id_column,
        text = target.source_text_column,
        tbl = target.source_table,
        extra = where_extra,
        lim = BATCH_SIZE,
    );
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![last_id], |r| {
            let id: String = r.get(0)?;
            let text: String = r.get(1)?;
            Ok((id, text))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

fn drop_and_recreate_table(
    db: &KnowledgeDatabase,
    target: &ReindexTarget,
    new_dim: usize,
) -> Result<(), String> {
    // Drop any orphan `__new` plus the live table; recreate the live table
    // at the target dimension. sqlite-vec's vec0 virtual tables do not
    // support `ALTER TABLE ... RENAME`, so we can't do the classic
    // create-new-and-swap pattern. Atomicity is instead preserved by the
    // `.embedding-state` marker file: until the marker is updated, callers
    // treat embeddings as stale.
    let sql = format!(
        "DROP TABLE IF EXISTS {table}__new;\n\
         DROP TABLE IF EXISTS {table};\n\
         CREATE VIRTUAL TABLE {table} USING vec0(\
            {id} TEXT PRIMARY KEY, \
            {emb} FLOAT[{dim}]\
         );",
        table = target.table,
        id = target.id_column,
        emb = target.embedding_column,
        dim = new_dim,
    );
    db.with_connection(|conn| {
        conn.execute_batch(&sql)?;
        Ok(())
    })
}

fn insert_batch(
    db: &KnowledgeDatabase,
    target: &ReindexTarget,
    rows: &[(String, Vec<f32>)],
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "INSERT INTO {table} ({id}, {emb}) VALUES (?1, ?2)",
        table = target.table,
        id = target.id_column,
        emb = target.embedding_column,
    );
    db.with_connection(|conn| {
        let tx_sql = "BEGIN";
        conn.execute_batch(tx_sql)?;
        {
            let mut stmt = conn.prepare(&sql)?;
            for (id, emb) in rows {
                let json = serde_json::to_string(emb).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
                        format!("serialize embedding: {e}"),
                    )))
                })?;
                stmt.execute(rusqlite::params![id, json])?;
            }
        }
        conn.execute_batch("COMMIT")?;
        Ok(())
    })
}

/// Per-row fallback for when a batched embed call fails. Embeds each row on
/// its own so a single oversized input skips only itself, not the whole batch.
///
/// Returns `(indexed, skipped)`.
async fn reindex_rows_individually(
    db: &KnowledgeDatabase,
    client: &Arc<dyn EmbeddingClient>,
    target: &ReindexTarget,
    batch: &[(String, String)],
) -> (usize, usize) {
    let mut indexed = 0usize;
    let mut skipped = 0usize;
    for (id, text) in batch {
        let truncated = truncate_for_embedding(text);
        let emb = match client.embed(&[truncated]).await {
            Ok(mut v) if v.len() == 1 => v.remove(0),
            _ => {
                skipped += 1;
                continue;
            }
        };
        match insert_batch(db, target, &[(id.clone(), emb)]) {
            Ok(()) => indexed += 1,
            Err(e) => {
                tracing::warn!(
                    target = target.table,
                    error = %e,
                    "reindex per-row insert failed; row skipped"
                );
                skipped += 1;
            }
        }
    }
    (indexed, skipped)
}

/// Reindex a single target table. Returns `(indexed, skipped)`.
///
/// The caller supplies the active embedding `client`, the target dimension,
/// and a progress callback invoked once per batch.
///
/// # Errors
///
/// Returns on fatal errors (table create/drop/rename). Per-row embed failures
/// are logged + counted in `skipped`.
pub async fn reindex_table<F>(
    db: &KnowledgeDatabase,
    client: Arc<dyn EmbeddingClient>,
    target: &ReindexTarget,
    new_dim: usize,
    mut on_progress: F,
) -> Result<ReindexSummary, String>
where
    F: FnMut(&'static str, usize, usize) + Send,
{
    let total = count_source_rows(db, target)?;
    drop_and_recreate_table(db, target, new_dim)?;

    let mut summary = ReindexSummary {
        indexed: 0,
        skipped: 0,
        total,
    };
    on_progress(target.table, 0, total);

    // Seed `last_id` with empty string (lexicographic minimum for TEXT).
    let mut last_id = String::new();

    loop {
        let batch = fetch_batch(db, target, &last_id)?;
        if batch.is_empty() {
            break;
        }
        let next_last_id = batch
            .last()
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| last_id.clone());

        // Embed this batch. Tolerate failure of the batch call by skipping all
        // rows in that batch. Truncate each input to a provider-safe length so
        // a single oversized row (e.g. a full wiki article) doesn't 400 the
        // whole batch.
        let texts: Vec<&str> = batch
            .iter()
            .map(|(_, t)| truncate_for_embedding(t.as_str()))
            .collect();
        let embeddings_res = client.embed(&texts).await;

        match embeddings_res {
            Ok(embeddings) if embeddings.len() == batch.len() => {
                let pairs: Vec<(String, Vec<f32>)> = batch
                    .iter()
                    .map(|(id, _)| id.clone())
                    .zip(embeddings.into_iter())
                    .collect();

                if let Err(e) = insert_batch(db, target, &pairs) {
                    tracing::warn!(
                        target = target.table,
                        error = %e,
                        count = pairs.len(),
                        "reindex insert batch failed; rows skipped"
                    );
                    summary.skipped += pairs.len();
                } else {
                    summary.indexed += pairs.len();
                }
            }
            Ok(mismatched) => {
                tracing::warn!(
                    target = target.table,
                    returned = mismatched.len(),
                    expected = batch.len(),
                    "embedding client returned wrong count; retrying rows individually"
                );
                let (indexed, skipped) =
                    reindex_rows_individually(db, &client, target, &batch).await;
                summary.indexed += indexed;
                summary.skipped += skipped;
            }
            Err(e) => {
                tracing::warn!(
                    target = target.table,
                    error = %e,
                    count = batch.len(),
                    "embedding batch failed; retrying rows individually"
                );
                let (indexed, skipped) =
                    reindex_rows_individually(db, &client, target, &batch).await;
                summary.indexed += indexed;
                summary.skipped += skipped;
            }
        }

        on_progress(target.table, summary.indexed + summary.skipped, total);

        last_id = next_last_id;
        if batch.len() < BATCH_SIZE {
            break;
        }
    }

    Ok(summary)
}

/// Reindex all targets in [`REINDEX_TARGETS`].
///
/// # Errors
///
/// Returns the first fatal error; partial progress may have been persisted
/// (orphan `*__new` tables are cleaned up on next boot).
pub async fn reindex_all(
    db: &KnowledgeDatabase,
    client: Arc<dyn EmbeddingClient>,
    new_dim: usize,
    on_progress: ProgressFn<'_>,
) -> Result<Vec<(ReindexTarget, ReindexSummary)>, String> {
    let mut out = Vec::with_capacity(REINDEX_TARGETS.len());
    for target in REINDEX_TARGETS {
        let summary = reindex_table(db, client.clone(), target, new_dim, |t, c, n| {
            on_progress(t, c, n);
        })
        .await?;
        out.push((*target, summary));
    }
    Ok(out)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::llm::embedding::{EmbeddingClient as Trait, EmbeddingError};
    use async_trait::async_trait;
    use gateway_services::VaultPaths;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    /// Deterministic mock embedder: returns `[k, k, ...]` for text of length k,
    /// normalized to unit length. Always produces `dim` floats.
    struct MockClient {
        dim: usize,
        calls: Arc<AtomicUsize>,
        fail_mode: FailMode,
    }

    #[derive(Clone, Copy)]
    enum FailMode {
        Never,
        Always,
        OnLengthEq(usize),
    }

    impl MockClient {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                calls: Arc::new(AtomicUsize::new(0)),
                fail_mode: FailMode::Never,
            }
        }
    }

    #[async_trait]
    impl Trait for MockClient {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.fail_mode {
                FailMode::Always => {
                    return Err(EmbeddingError::ModelError("mock fail".into()));
                }
                FailMode::OnLengthEq(n) if texts.len() == n => {
                    return Err(EmbeddingError::ModelError("mock fail".into()));
                }
                _ => {}
            }
            Ok(texts
                .iter()
                .map(|t| {
                    let seed = (t.len() as f32).max(1.0);
                    let v = vec![seed; self.dim];
                    let norm = (seed * seed * (self.dim as f32)).sqrt();
                    v.into_iter().map(|x| x / norm).collect()
                })
                .collect())
        }

        fn dimensions(&self) -> usize {
            self.dim
        }

        fn model_name(&self) -> String {
            "mock".to_string()
        }
    }

    fn fresh_db() -> (TempDir, Arc<KnowledgeDatabase>) {
        let tmp = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        (tmp, db)
    }

    fn seed_memory_facts(db: &KnowledgeDatabase, n: usize) {
        db.with_connection(|conn| {
            for i in 0..n {
                conn.execute(
                    "INSERT INTO memory_facts (id, agent_id, scope, category, key, content, created_at, updated_at)
                     VALUES (?1, 'root', 's', 'c', ?2, ?3, datetime('now'), datetime('now'))",
                    rusqlite::params![
                        format!("fact-{:04}", i),
                        format!("k-{i}"),
                        format!("content number {i}")
                    ],
                )?;
            }
            Ok(())
        })
        .unwrap();
    }

    fn table_dim(db: &KnowledgeDatabase, table: &str) -> Option<String> {
        db.with_connection(|conn| {
            conn.query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                rusqlite::params![table],
                |r| r.get::<_, Option<String>>(0),
            )
        })
        .ok()
        .flatten()
    }

    fn row_count(db: &KnowledgeDatabase, table: &str) -> i64 {
        db.with_connection(|conn| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |r| r.get(0))
        })
        .unwrap_or(-1)
    }

    #[tokio::test]
    async fn reindex_creates_new_table_with_specified_dim() {
        let (_tmp, db) = fresh_db();
        seed_memory_facts(&db, 3);
        let client: Arc<dyn Trait> = Arc::new(MockClient::new(1024));
        let target = &REINDEX_TARGETS[0];
        let summary = reindex_table(&db, client, target, 1024, |_, _, _| {})
            .await
            .unwrap();
        assert_eq!(summary.indexed, 3);
        assert_eq!(summary.skipped, 0);
        let sql = table_dim(&db, "memory_facts_index").unwrap();
        assert!(sql.contains("FLOAT[1024]"), "got: {sql}");
    }

    #[tokio::test]
    async fn reindex_drops_old_renames_new_atomically() {
        let (_tmp, db) = fresh_db();
        seed_memory_facts(&db, 2);
        let client: Arc<dyn Trait> = Arc::new(MockClient::new(768));
        reindex_table(&db, client, &REINDEX_TARGETS[0], 768, |_, _, _| {})
            .await
            .unwrap();
        // `__new` gone, main exists with new dim and correct row count.
        assert!(table_dim(&db, "memory_facts_index__new").is_none());
        assert_eq!(row_count(&db, "memory_facts_index"), 2);
    }

    #[tokio::test]
    async fn reindex_skips_failed_embeddings_continues() {
        let (_tmp, db) = fresh_db();
        seed_memory_facts(&db, 3);
        let mock = MockClient {
            dim: 384,
            calls: Arc::new(AtomicUsize::new(0)),
            fail_mode: FailMode::Always,
        };
        let client: Arc<dyn Trait> = Arc::new(mock);
        let summary = reindex_table(&db, client, &REINDEX_TARGETS[0], 384, |_, _, _| {})
            .await
            .unwrap();
        assert_eq!(summary.indexed, 0);
        assert_eq!(summary.skipped, 3);
        assert_eq!(summary.total, 3);
        // Table exists but is empty.
        assert_eq!(row_count(&db, "memory_facts_index"), 0);
    }

    #[tokio::test]
    async fn reindex_falls_back_to_per_row_when_batch_fails() {
        let (_tmp, db) = fresh_db();
        seed_memory_facts(&db, 3);
        // Fails the batched call (3 texts) but succeeds per-row (1 text each):
        // one oversized row must not drop the whole batch.
        let mock = MockClient {
            dim: 384,
            calls: Arc::new(AtomicUsize::new(0)),
            fail_mode: FailMode::OnLengthEq(3),
        };
        let client: Arc<dyn Trait> = Arc::new(mock);
        let summary = reindex_table(&db, client, &REINDEX_TARGETS[0], 384, |_, _, _| {})
            .await
            .unwrap();
        assert_eq!(summary.indexed, 3);
        assert_eq!(summary.skipped, 0);
        assert_eq!(row_count(&db, "memory_facts_index"), 3);
    }

    #[tokio::test]
    async fn reindex_progress_callback_fires_per_batch() {
        let (_tmp, db) = fresh_db();
        seed_memory_facts(&db, 5);
        let client: Arc<dyn Trait> = Arc::new(MockClient::new(384));
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        reindex_table(&db, client, &REINDEX_TARGETS[0], 384, move |_, _, _| {
            calls_c.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();
        // At least two callbacks: initial (0/total) and one after the single batch.
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn reindex_all_processes_every_target() {
        let (_tmp, db) = fresh_db();
        seed_memory_facts(&db, 2);
        let client: Arc<dyn Trait> = Arc::new(MockClient::new(384));
        let calls = AtomicUsize::new(0);
        let calls_ref = &calls;
        let cb = move |_: &'static str, _: usize, _: usize| {
            calls_ref.fetch_add(1, Ordering::SeqCst);
        };
        let out = reindex_all(&db, client, 384, &cb).await.unwrap();
        assert_eq!(out.len(), REINDEX_TARGETS.len());
    }

    #[tokio::test]
    async fn boot_cleanup_drops_orphan_new_tables_idempotent() {
        let (_tmp, db) = fresh_db();
        // Create an orphan `__new` table directly.
        db.with_connection(|conn| {
            conn.execute_batch(
                "CREATE VIRTUAL TABLE memory_facts_index__new USING vec0(\
                    fact_id TEXT PRIMARY KEY, embedding FLOAT[1024])",
            )?;
            Ok(())
        })
        .unwrap();
        // Running reindex should DROP-then-create it cleanly (no error).
        let client: Arc<dyn Trait> = Arc::new(MockClient::new(384));
        let _ = reindex_table(&db, client, &REINDEX_TARGETS[0], 384, |_, _, _| {})
            .await
            .unwrap();
        // Also run the vec-table helper explicitly — must not error.
        db.with_connection(crate::knowledge_schema::cleanup_orphan_reindex_tables)
            .unwrap();
    }

    #[tokio::test]
    async fn reindex_empty_source_table_succeeds() {
        let (_tmp, db) = fresh_db();
        let client: Arc<dyn Trait> = Arc::new(MockClient::new(1024));
        let summary = reindex_table(&db, client, &REINDEX_TARGETS[0], 1024, |_, _, _| {})
            .await
            .unwrap();
        assert_eq!(summary.indexed, 0);
        assert_eq!(summary.total, 0);
        // Table exists at new dim.
        let sql = table_dim(&db, "memory_facts_index").unwrap();
        assert!(sql.contains("FLOAT[1024]"));
    }

    #[test]
    fn truncate_for_embedding_is_noop_for_short_input() {
        let input = "short text";
        assert_eq!(truncate_for_embedding(input), input);
    }

    #[test]
    fn truncate_for_embedding_caps_long_ascii() {
        let input = "a".repeat(MAX_CHARS_PER_EMBED_INPUT + 500);
        let out = truncate_for_embedding(&input);
        assert_eq!(out.chars().count(), MAX_CHARS_PER_EMBED_INPUT);
    }

    #[test]
    fn truncate_for_embedding_preserves_utf8_boundary() {
        // Repeat a 3-byte char past the cap; the slice must not split
        // a multi-byte codepoint.
        let ch = "✓"; // 3 bytes
        let input = ch.repeat(MAX_CHARS_PER_EMBED_INPUT + 10);
        let out = truncate_for_embedding(&input);
        assert_eq!(out.chars().count(), MAX_CHARS_PER_EMBED_INPUT);
        // If truncation sliced mid-codepoint the next line would panic.
        let _round_trip = out.to_string();
    }

    #[test]
    fn reindex_targets_covers_all_vec0_tables() {
        // Each entry must map 1:1 with a vec0 virtual table created by
        // `initialize_vec_tables_with_dim`. Update
        // both when adding a new embedded source.
        let expected = [
            "memory_facts_index",
            "kg_name_index",
            "session_episodes_index",
            "wiki_articles_index",
            "procedures_index",
        ];
        assert_eq!(REINDEX_TARGETS.len(), expected.len());
        for table in expected {
            assert!(
                REINDEX_TARGETS.iter().any(|t| t.table == table),
                "missing reindex target for vec0 table: {table}"
            );
        }
    }
}
