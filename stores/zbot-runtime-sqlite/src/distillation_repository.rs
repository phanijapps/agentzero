// ============================================================================
// DISTILLATION REPOSITORY
// CRUD operations for the distillation_runs table
// ============================================================================

use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::DatabaseManager;

// ============================================================================
// TYPES
// ============================================================================

/// A distillation run record tracking the outcome of session distillation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DistillationRun {
    pub id: String,
    pub session_id: String,
    pub status: String, // 'success', 'failed', 'skipped', 'permanently_failed'
    pub facts_extracted: i32,
    pub entities_extracted: i32,
    pub relationships_extracted: i32,
    pub episode_created: i32, // 0 or 1
    pub error: Option<String>,
    pub retry_count: i32,
    pub duration_ms: Option<i64>,
    pub created_at: String,
}

// `UndistilledSession` and `DistillationStats` live in `zbot-stores-domain`
// so HTTP handlers and other consumers can depend on the dep-light crate.
pub use zbot_stores_domain::{DistillationStats, UndistilledSession};

// ============================================================================
// DISTILLATION REPOSITORY
// ============================================================================

/// Repository for distillation run operations.
pub struct DistillationRepository {
    db: Arc<DatabaseManager>,
}

impl DistillationRepository {
    /// Create a new distillation repository.
    pub fn new(db: Arc<DatabaseManager>) -> Self {
        Self { db }
    }

    /// Insert or refresh a distillation run record.
    ///
    /// `distillation_runs` has a `UNIQUE(session_id)` constraint — when
    /// distillation re-fires for the same session (both the initial and
    /// the continuation completions trigger it), a plain INSERT fails.
    /// On conflict we overwrite the row with the new attempt's state;
    /// the caller's subsequent `update_success` / `update_retry` will
    /// then advance that row through the lifecycle. Identity (`id`,
    /// `session_id`) is preserved across retries so downstream queries
    /// (e.g. `archiver.rs` joining on `session_id`) stay stable.
    pub fn insert(&self, run: &DistillationRun) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO distillation_runs (id, session_id, status, facts_extracted, entities_extracted, relationships_extracted, episode_created, error, retry_count, duration_ms, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(session_id) DO UPDATE SET
                    status = excluded.status,
                    facts_extracted = excluded.facts_extracted,
                    entities_extracted = excluded.entities_extracted,
                    relationships_extracted = excluded.relationships_extracted,
                    episode_created = excluded.episode_created,
                    error = excluded.error,
                    retry_count = excluded.retry_count,
                    duration_ms = excluded.duration_ms,
                    created_at = excluded.created_at",
                params![
                    run.id,
                    run.session_id,
                    run.status,
                    run.facts_extracted,
                    run.entities_extracted,
                    run.relationships_extracted,
                    run.episode_created,
                    run.error,
                    run.retry_count,
                    run.duration_ms,
                    run.created_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Get a distillation run by session_id.
    pub fn get_by_session_id(&self, session_id: &str) -> Result<Option<DistillationRun>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, status, facts_extracted, entities_extracted, relationships_extracted, episode_created, error, retry_count, duration_ms, created_at
                 FROM distillation_runs
                 WHERE session_id = ?1",
            )?;

            let result = stmt.query_row(params![session_id], row_to_distillation_run);

            match result {
                Ok(run) => Ok(Some(run)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Update a distillation run for retry (set status, increment retry_count, record error).
    pub fn update_retry(
        &self,
        session_id: &str,
        status: &str,
        retry_count: i32,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE distillation_runs SET status = ?1, retry_count = ?2, error = ?3
                 WHERE session_id = ?4",
                params![status, retry_count, error, session_id],
            )?;
            Ok(())
        })
    }

    /// Update a distillation run to success with extraction counts and duration.
    pub fn update_success(
        &self,
        session_id: &str,
        facts: i32,
        entities: i32,
        rels: i32,
        episode: bool,
        duration_ms: i64,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE distillation_runs SET status = 'success', facts_extracted = ?1, entities_extracted = ?2, relationships_extracted = ?3, episode_created = ?4, duration_ms = ?5, error = NULL
                 WHERE session_id = ?6",
                params![
                    facts,
                    entities,
                    rels,
                    episode as i32,
                    duration_ms,
                    session_id,
                ],
            )?;
            Ok(())
        })
    }

    /// Get failed distillation runs eligible for retry (retry_count < max_retries).
    pub fn get_failed_for_retry(&self, max_retries: i32) -> Result<Vec<DistillationRun>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, status, facts_extracted, entities_extracted, relationships_extracted, episode_created, error, retry_count, duration_ms, created_at
                 FROM distillation_runs
                 WHERE status = 'failed' AND retry_count < ?1
                 ORDER BY created_at ASC",
            )?;

            let rows = stmt.query_map(params![max_retries], row_to_distillation_run)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Get session IDs that have no distillation run record (undistilled sessions).
    pub fn get_undistilled_session_ids(&self) -> Result<Vec<String>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id
                 FROM sessions s
                 LEFT JOIN distillation_runs dr ON s.id = dr.session_id
                 WHERE dr.id IS NULL
                 ORDER BY (s.parent_session_id IS NULL) DESC, s.created_at ASC",
            )?;

            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Get undistilled sessions with their root agent IDs.
    pub fn get_undistilled_sessions(&self) -> Result<Vec<UndistilledSession>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.root_agent_id
                 FROM sessions s
                 LEFT JOIN distillation_runs dr ON s.id = dr.session_id
                 WHERE dr.id IS NULL
                 ORDER BY (s.parent_session_id IS NULL) DESC, s.created_at ASC",
            )?;

            let rows = stmt.query_map([], |row| {
                Ok(UndistilledSession {
                    session_id: row.get(0)?,
                    agent_id: row.get(1)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Get aggregate statistics across all distillation runs.
    pub fn get_stats(&self) -> Result<DistillationStats, String> {
        self.db.with_connection(|conn| {
            let stats = conn.query_row(
                "SELECT
                    COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'skipped' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'permanently_failed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(facts_extracted), 0),
                    COALESCE(SUM(entities_extracted), 0),
                    COALESCE(SUM(relationships_extracted), 0),
                    COALESCE(SUM(episode_created), 0)
                 FROM distillation_runs",
                [],
                |row| {
                    Ok(DistillationStats {
                        success_count: row.get(0)?,
                        failed_count: row.get(1)?,
                        skipped_count: row.get(2)?,
                        permanently_failed_count: row.get(3)?,
                        total_facts: row.get(4)?,
                        total_entities: row.get(5)?,
                        total_relationships: row.get(6)?,
                        total_episodes: row.get(7)?,
                    })
                },
            )?;

            Ok(stats)
        })
    }
}

// ============================================================================
// HELPERS
// ============================================================================

/// Map a database row to a DistillationRun struct.
fn row_to_distillation_run(row: &rusqlite::Row) -> Result<DistillationRun, rusqlite::Error> {
    Ok(DistillationRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        status: row.get(2)?,
        facts_extracted: row.get(3)?,
        entities_extracted: row.get(4)?,
        relationships_extracted: row.get(5)?,
        episode_created: row.get(6)?,
        error: row.get(7)?,
        retry_count: row.get(8)?,
        duration_ms: row.get(9)?,
        created_at: row.get(10)?,
    })
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Arc<DatabaseManager> {
        use gateway_services::VaultPaths;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(temp_dir.path().to_path_buf()));
        let _ = temp_dir.keep();
        let db = DatabaseManager::new(paths).unwrap();
        Arc::new(db)
    }

    fn make_run(session_id: &str, status: &str) -> DistillationRun {
        DistillationRun {
            id: format!("dr-{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            status: status.to_string(),
            facts_extracted: 0,
            entities_extracted: 0,
            relationships_extracted: 0,
            episode_created: 0,
            error: None,
            retry_count: 0,
            duration_ms: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Helper to insert a session row so foreign-key-free LEFT JOINs work.
    fn insert_session(db: &Arc<DatabaseManager>, session_id: &str) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, root_agent_id, created_at) VALUES (?1, ?2, ?3)",
                params![session_id, "agent-1", chrono::Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_insert_and_get_by_session_id() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let run = make_run("sess-1", "failed");
        repo.insert(&run).unwrap();

        let fetched = repo.get_by_session_id("sess-1").unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.session_id, "sess-1");
        assert_eq!(fetched.status, "failed");
        assert_eq!(fetched.id, run.id);
    }

    #[test]
    fn test_get_by_session_id_not_found() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let fetched = repo.get_by_session_id("nonexistent").unwrap();
        assert!(fetched.is_none());
    }

    /// Regression: a second `insert` for the same session_id (re-firing
    /// distillation on a continuation) must refresh the row in place
    /// rather than fail with UNIQUE constraint. The `session_id` identity
    /// is preserved; mutable fields reflect the latest attempt.
    #[test]
    fn insert_is_idempotent_by_session_id() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let first = make_run("sess-same", "failed");
        repo.insert(&first).expect("first insert");

        // Second attempt: different id, different state (the caller reset
        // to "failed / Distillation in progress" each time record_pending
        // fires). Must succeed.
        let mut second = make_run("sess-same", "failed");
        second.error = Some("Distillation in progress".to_string());
        second.retry_count = 1;
        repo.insert(&second)
            .expect("second insert must upsert, not fail");

        // Exactly one row by session.
        let fetched = repo.get_by_session_id("sess-same").unwrap().unwrap();
        assert_eq!(fetched.session_id, "sess-same");
        // Mutable fields reflect the second insert.
        assert_eq!(fetched.error.as_deref(), Some("Distillation in progress"));
        assert_eq!(fetched.retry_count, 1);
    }

    #[test]
    fn test_update_retry() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let run = make_run("sess-retry", "failed");
        repo.insert(&run).unwrap();

        repo.update_retry("sess-retry", "failed", 2, Some("timeout"))
            .unwrap();

        let fetched = repo.get_by_session_id("sess-retry").unwrap().unwrap();
        assert_eq!(fetched.status, "failed");
        assert_eq!(fetched.retry_count, 2);
        assert_eq!(fetched.error.as_deref(), Some("timeout"));
    }

    #[test]
    fn test_update_success() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let run = make_run("sess-success", "failed");
        repo.insert(&run).unwrap();

        repo.update_success("sess-success", 5, 3, 2, true, 1500)
            .unwrap();

        let fetched = repo.get_by_session_id("sess-success").unwrap().unwrap();
        assert_eq!(fetched.status, "success");
        assert_eq!(fetched.facts_extracted, 5);
        assert_eq!(fetched.entities_extracted, 3);
        assert_eq!(fetched.relationships_extracted, 2);
        assert_eq!(fetched.episode_created, 1);
        assert_eq!(fetched.duration_ms, Some(1500));
        assert!(fetched.error.is_none());
    }

    #[test]
    fn test_get_failed_for_retry() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        // Insert a failed run with 0 retries
        let run1 = make_run("sess-fail-1", "failed");
        repo.insert(&run1).unwrap();

        // Insert a failed run with 2 retries
        let mut run2 = make_run("sess-fail-2", "failed");
        run2.retry_count = 2;
        repo.insert(&run2).unwrap();

        // Insert a failed run with 5 retries (should be excluded at max=3)
        let mut run3 = make_run("sess-fail-3", "failed");
        run3.retry_count = 5;
        repo.insert(&run3).unwrap();

        // Insert a success run (should be excluded)
        let run4 = make_run("sess-ok", "success");
        repo.insert(&run4).unwrap();

        let retryable = repo.get_failed_for_retry(3).unwrap();
        assert_eq!(retryable.len(), 2);
        assert_eq!(retryable[0].session_id, "sess-fail-1");
        assert_eq!(retryable[1].session_id, "sess-fail-2");
    }

    #[test]
    fn test_get_undistilled_session_ids() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db.clone());

        // Create sessions
        insert_session(&db, "sess-a");
        insert_session(&db, "sess-b");
        insert_session(&db, "sess-c");

        // Mark sess-a as distilled
        let run = make_run("sess-a", "success");
        repo.insert(&run).unwrap();

        // sess-b and sess-c should be undistilled
        let undistilled = repo.get_undistilled_session_ids().unwrap();
        assert_eq!(undistilled.len(), 2);
        assert!(undistilled.contains(&"sess-b".to_string()));
        assert!(undistilled.contains(&"sess-c".to_string()));
    }

    #[test]
    fn test_get_stats_empty() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let stats = repo.get_stats().unwrap();
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.failed_count, 0);
        assert_eq!(stats.skipped_count, 0);
        assert_eq!(stats.permanently_failed_count, 0);
        assert_eq!(stats.total_facts, 0);
        assert_eq!(stats.total_entities, 0);
        assert_eq!(stats.total_relationships, 0);
        assert_eq!(stats.total_episodes, 0);
    }

    #[test]
    fn test_get_stats_with_data() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        // Insert a success run with extraction counts
        let mut run1 = make_run("sess-s1", "success");
        run1.facts_extracted = 5;
        run1.entities_extracted = 3;
        run1.relationships_extracted = 2;
        run1.episode_created = 1;
        repo.insert(&run1).unwrap();

        // Insert another success run
        let mut run2 = make_run("sess-s2", "success");
        run2.facts_extracted = 10;
        run2.entities_extracted = 7;
        run2.relationships_extracted = 4;
        run2.episode_created = 1;
        repo.insert(&run2).unwrap();

        // Insert a failed run
        let mut run3 = make_run("sess-f1", "failed");
        run3.error = Some("LLM timeout".to_string());
        repo.insert(&run3).unwrap();

        // Insert a skipped run
        let run4 = make_run("sess-sk1", "skipped");
        repo.insert(&run4).unwrap();

        // Insert a permanently_failed run
        let run5 = make_run("sess-pf1", "permanently_failed");
        repo.insert(&run5).unwrap();

        let stats = repo.get_stats().unwrap();
        assert_eq!(stats.success_count, 2);
        assert_eq!(stats.failed_count, 1);
        assert_eq!(stats.skipped_count, 1);
        assert_eq!(stats.permanently_failed_count, 1);
        assert_eq!(stats.total_facts, 15);
        assert_eq!(stats.total_entities, 10);
        assert_eq!(stats.total_relationships, 6);
        assert_eq!(stats.total_episodes, 2);
    }

    /// Previously asserted that a duplicate session_id was rejected.
    /// After the upsert fix (`ON CONFLICT(session_id) DO UPDATE`), the
    /// second insert overwrites the row instead — re-running distillation
    /// on a continuation must not fail with UNIQUE constraint. Updated
    /// to lock in the new semantics.
    #[test]
    fn test_insert_duplicate_session_id_overwrites() {
        let db = create_test_db();
        let repo = DistillationRepository::new(db);

        let run1 = make_run("sess-dup", "failed");
        repo.insert(&run1).unwrap();

        let run2 = make_run("sess-dup", "success");
        repo.insert(&run2)
            .expect("upsert must replace the existing row, not fail");

        let fetched = repo.get_by_session_id("sess-dup").unwrap().unwrap();
        assert_eq!(fetched.status, "success");
    }
}
