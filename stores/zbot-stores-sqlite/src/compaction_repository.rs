//! CRUD over the `kg_compactions` table. Compaction rows track merge, prune,
//! and invalidate operations applied to the knowledge graph during a
//! compaction run.

use crate::KnowledgeDatabase;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

/// A compaction row from `kg_compactions`.
#[derive(Debug, Clone)]
pub struct Compaction {
    pub id: String,
    pub run_id: String,
    /// One of `'merge'`, `'prune'`, or `'invalidate'`.
    pub operation: String,
    pub entity_id: Option<String>,
    pub relationship_id: Option<String>,
    pub merged_into: Option<String>,
    pub reason: Option<String>,
    pub created_at: String,
}

/// Summary of the latest compaction run.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub run_id: String,
    pub latest_at: String,
    pub merges: u64,
    pub prunes: u64,
}

/// Repository for `kg_compactions` — records merge/prune operations and
/// provides per-run query helpers.
pub struct CompactionRepository {
    db: Arc<KnowledgeDatabase>,
}

impl CompactionRepository {
    pub fn new(db: Arc<KnowledgeDatabase>) -> Self {
        Self { db }
    }

    /// Record a merge: `loser_entity_id` is absorbed into `winner_entity_id`.
    /// Returns the generated row ID.
    pub fn record_merge(
        &self,
        run_id: &str,
        loser_entity_id: &str,
        winner_entity_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_compactions
                    (id, run_id, operation, entity_id, merged_into, reason, created_at)
                 VALUES (?1, ?2, 'merge', ?3, ?4, ?5, ?6)",
                params![id, run_id, loser_entity_id, winner_entity_id, reason, now],
            )?;
            Ok(id.clone())
        })
    }

    /// Record a synthesis: a cross-session strategy fact was extracted
    /// into `memory_facts`. `fact_id` is the newly-inserted (or updated)
    /// fact's id, stored in the `entity_id` column for cross-referencing.
    /// Returns the generated row ID.
    pub fn record_synthesis(
        &self,
        run_id: &str,
        fact_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_compactions
                    (id, run_id, operation, entity_id, reason, created_at)
                 VALUES (?1, ?2, 'synthesize', ?3, ?4, ?5)",
                params![id, run_id, fact_id, reason, now],
            )?;
            Ok(id.clone())
        })
    }

    /// Record a pattern extraction: a recurring tool-call sequence was
    /// synthesized into a `procedures` row. `procedure_id` is stored in the
    /// `entity_id` column for cross-referencing. Returns the generated row ID.
    pub fn record_pattern(
        &self,
        run_id: &str,
        procedure_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_compactions
                    (id, run_id, operation, entity_id, reason, created_at)
                 VALUES (?1, ?2, 'pattern_extract', ?3, ?4, ?5)",
                params![id, run_id, procedure_id, reason, now],
            )?;
            Ok(id.clone())
        })
    }

    /// Record a prune: `entity_id` was soft-deleted due to decay.
    /// Returns the generated row ID.
    pub fn record_prune(
        &self,
        run_id: &str,
        entity_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_compactions
                    (id, run_id, operation, entity_id, reason, created_at)
                 VALUES (?1, ?2, 'prune', ?3, ?4, ?5)",
                params![id, run_id, entity_id, reason, now],
            )?;
            Ok(id.clone())
        })
    }

    /// List all rows for a specific run, ordered by `created_at` ASC.
    pub fn list_run(&self, run_id: &str) -> Result<Vec<Compaction>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, operation, entity_id, relationship_id,
                        merged_into, reason, created_at
                 FROM kg_compactions
                 WHERE run_id = ?1
                 ORDER BY created_at ASC",
            )?;
            let rows = stmt
                .query_map(params![run_id], row_to_compaction)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Returns a summary of the latest compaction run, or `None` if the
    /// table is empty.
    pub fn latest_run_summary(&self) -> Result<Option<RunSummary>, String> {
        self.db.with_connection(|conn| {
            // Find the run_id with the most-recent created_at timestamp.
            let result: Option<(String, String, u64, u64)> = {
                let mut stmt = conn.prepare(
                    "SELECT run_id,
                            MAX(created_at) AS latest_at,
                            SUM(CASE WHEN operation = 'merge' THEN 1 ELSE 0 END) AS merges,
                            SUM(CASE WHEN operation = 'prune' THEN 1 ELSE 0 END) AS prunes
                     FROM kg_compactions
                     WHERE run_id = (
                         SELECT run_id FROM kg_compactions
                         ORDER BY created_at DESC LIMIT 1
                     )
                     GROUP BY run_id",
                )?;
                stmt.query_row([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                })
                .optional()?
            };

            Ok(
                result.map(|(run_id, latest_at, merges, prunes)| RunSummary {
                    run_id,
                    latest_at,
                    merges,
                    prunes,
                }),
            )
        })
    }
}

fn row_to_compaction(row: &rusqlite::Row) -> rusqlite::Result<Compaction> {
    Ok(Compaction {
        id: row.get(0)?,
        run_id: row.get(1)?,
        operation: row.get(2)?,
        entity_id: row.get(3)?,
        relationship_id: row.get(4)?,
        merged_into: row.get(5)?,
        reason: row.get(6)?,
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeDatabase;
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, CompactionRepository) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        (tmp, CompactionRepository::new(db))
    }

    #[test]
    fn record_merge_then_list_run() {
        let (_tmp, repo) = setup();
        let run = "run-001";

        repo.record_merge(run, "entity-loser-a", "entity-winner-x", "duplicate name")
            .unwrap();
        repo.record_merge(
            run,
            "entity-loser-b",
            "entity-winner-x",
            "same canonical id",
        )
        .unwrap();

        let rows = repo.list_run(run).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].operation, "merge");
        assert_eq!(rows[0].entity_id.as_deref(), Some("entity-loser-a"));
        assert_eq!(rows[0].merged_into.as_deref(), Some("entity-winner-x"));
        assert_eq!(rows[1].entity_id.as_deref(), Some("entity-loser-b"));
        // Order must be ASC by created_at; both rows were inserted in sequence.
        // The second insert cannot precede the first.
        assert!(rows[0].created_at <= rows[1].created_at);
    }

    #[test]
    fn record_prune_logs_operation_correctly() {
        let (_tmp, repo) = setup();
        let run = "run-002";

        repo.record_prune(run, "stale-entity-42", "decay score below threshold")
            .unwrap();

        let rows = repo.list_run(run).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.operation, "prune");
        assert_eq!(row.entity_id.as_deref(), Some("stale-entity-42"));
        assert_eq!(row.reason.as_deref(), Some("decay score below threshold"));
        assert!(row.merged_into.is_none());
    }

    #[test]
    fn latest_run_summary_aggregates_counts() {
        let (_tmp, repo) = setup();
        let run = "run-003";

        repo.record_merge(run, "e1", "e-winner", "dup").unwrap();
        repo.record_merge(run, "e2", "e-winner", "dup").unwrap();
        repo.record_prune(run, "e3", "stale").unwrap();

        let summary = repo.latest_run_summary().unwrap().expect("has summary");
        assert_eq!(summary.run_id, run);
        assert_eq!(summary.merges, 2);
        assert_eq!(summary.prunes, 1);
        assert!(!summary.latest_at.is_empty());
    }

    #[test]
    fn record_synthesis_logs_operation_correctly() {
        let (_tmp, repo) = setup();
        let run = "run-synth";

        let row_id = repo
            .record_synthesis(
                run,
                "fact-xyz",
                "strategy 'retry backoff' across 3 sessions",
            )
            .unwrap();
        assert!(!row_id.is_empty());

        let rows = repo.list_run(run).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.operation, "synthesize");
        assert_eq!(row.entity_id.as_deref(), Some("fact-xyz"));
        assert!(row.reason.as_deref().unwrap().contains("retry backoff"));
    }

    #[test]
    fn record_pattern_logs_operation_correctly() {
        let (_tmp, repo) = setup();
        let run = "run-pattern";

        let row_id = repo
            .record_pattern(
                run,
                "proc-abc",
                "pattern 'search_then_read' across 2 sessions",
            )
            .unwrap();
        assert!(!row_id.is_empty());

        let rows = repo.list_run(run).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.operation, "pattern_extract");
        assert_eq!(row.entity_id.as_deref(), Some("proc-abc"));
        assert!(row.reason.as_deref().unwrap().contains("search_then_read"));
    }

    #[test]
    fn latest_run_summary_returns_none_when_empty() {
        let (_tmp, repo) = setup();
        let result = repo.latest_run_summary().unwrap();
        assert!(result.is_none());
    }
}
