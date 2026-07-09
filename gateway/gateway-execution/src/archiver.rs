// ============================================================================
// SESSION ARCHIVER
// Offloads old session transcripts to compressed JSONL files on disk,
// keeping SQLite lean for constrained environments (Raspberry Pi, etc.).
// ============================================================================

use std::io::{Read as IoRead, Write as IoWrite};
use std::path::PathBuf;
use std::sync::Arc;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use zbot_stores_sqlite::DatabaseManager;

// ============================================================================
// Types
// ============================================================================

/// Result of archiving a single session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveResult {
    pub session_id: String,
    pub messages_archived: usize,
    pub logs_archived: usize,
    pub file_size: u64,
}

/// A single line in the JSONL archive file.
#[derive(Debug, Serialize, Deserialize)]
struct ArchiveLine {
    /// "message" or "execution_log"
    record_type: String,
    data: serde_json::Value,
}

// ============================================================================
// SessionArchiver
// ============================================================================

/// Archives completed session transcripts (messages + execution logs) to
/// compressed JSONL files and removes them from SQLite.
pub struct SessionArchiver {
    db: Arc<DatabaseManager>,
    archive_path: PathBuf,
}

impl SessionArchiver {
    /// Create a new archiver.
    ///
    /// `archive_path` is the directory where `.jsonl.gz` files will be written.
    /// It will be created on first archive if it doesn't exist.
    pub fn new(db: Arc<DatabaseManager>, archive_path: PathBuf) -> Self {
        Self { db, archive_path }
    }

    /// Archive a single session: serialize messages + logs to a compressed
    /// JSONL file, then delete them from SQLite.
    ///
    /// Safety: the file is fully written and flushed before any SQLite rows
    /// are deleted. If the file write succeeds but a DELETE fails, data exists
    /// in both places (safe but wasteful).
    pub fn archive_session(&self, session_id: &str) -> Result<ArchiveResult, String> {
        // 1. Load messages for session
        let messages = self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, execution_id, session_id, role, content, created_at,
                        token_count, tool_calls, tool_results, tool_call_id
                 FROM messages WHERE session_id = ?1
                 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([session_id], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "execution_id": row.get::<_, Option<String>>(1)?,
                    "session_id": row.get::<_, Option<String>>(2)?,
                    "role": row.get::<_, String>(3)?,
                    "content": row.get::<_, String>(4)?,
                    "created_at": row.get::<_, String>(5)?,
                    "token_count": row.get::<_, i32>(6)?,
                    "tool_calls": row.get::<_, Option<String>>(7)?,
                    "tool_results": row.get::<_, Option<String>>(8)?,
                    "tool_call_id": row.get::<_, Option<String>>(9)?,
                }))
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        // 2. Load execution logs for session
        let logs = self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, conversation_id, agent_id, parent_session_id,
                        timestamp, level, category, message, metadata, duration_ms
                 FROM execution_logs WHERE session_id = ?1
                 ORDER BY timestamp ASC",
            )?;
            let rows = stmt.query_map([session_id], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "session_id": row.get::<_, String>(1)?,
                    "conversation_id": row.get::<_, Option<String>>(2)?,
                    "agent_id": row.get::<_, String>(3)?,
                    "parent_session_id": row.get::<_, Option<String>>(4)?,
                    "timestamp": row.get::<_, String>(5)?,
                    "level": row.get::<_, String>(6)?,
                    "category": row.get::<_, String>(7)?,
                    "message": row.get::<_, String>(8)?,
                    "metadata": row.get::<_, Option<String>>(9)?,
                    "duration_ms": row.get::<_, Option<i64>>(10)?,
                }))
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        let messages_count = messages.len();
        let logs_count = logs.len();

        if messages_count == 0 && logs_count == 0 {
            // Nothing to archive, just mark as archived
            self.db.with_connection(|conn| {
                conn.execute(
                    "UPDATE sessions SET archived = 1 WHERE id = ?1",
                    params![session_id],
                )?;
                Ok(())
            })?;
            return Ok(ArchiveResult {
                session_id: session_id.to_string(),
                messages_archived: 0,
                logs_archived: 0,
                file_size: 0,
            });
        }

        // 3. Ensure archive directory exists
        std::fs::create_dir_all(&self.archive_path)
            .map_err(|e| format!("Failed to create archive directory: {e}"))?;

        // 4. Serialize to compressed JSONL
        let archive_file = self.archive_path.join(format!("{}.jsonl.gz", session_id));
        let file = std::fs::File::create(&archive_file)
            .map_err(|e| format!("Failed to create archive file: {e}"))?;
        let mut encoder = GzEncoder::new(file, Compression::default());

        for msg in &messages {
            let line = ArchiveLine {
                record_type: "message".to_string(),
                data: msg.clone(),
            };
            let json = serde_json::to_string(&line)
                .map_err(|e| format!("Failed to serialize message: {e}"))?;
            encoder
                .write_all(json.as_bytes())
                .map_err(|e| format!("Failed to write message: {e}"))?;
            encoder
                .write_all(b"\n")
                .map_err(|e| format!("Failed to write newline: {e}"))?;
        }

        for log in &logs {
            let line = ArchiveLine {
                record_type: "execution_log".to_string(),
                data: log.clone(),
            };
            let json = serde_json::to_string(&line)
                .map_err(|e| format!("Failed to serialize log: {e}"))?;
            encoder
                .write_all(json.as_bytes())
                .map_err(|e| format!("Failed to write log: {e}"))?;
            encoder
                .write_all(b"\n")
                .map_err(|e| format!("Failed to write newline: {e}"))?;
        }

        encoder
            .finish()
            .map_err(|e| format!("Failed to finish compression: {e}"))?;

        // 5. Get the resulting file size
        let file_size = std::fs::metadata(&archive_file)
            .map(|m| m.len())
            .unwrap_or(0);

        // 6. Delete messages + logs and mark archived — atomically.
        //
        // Without a transaction these three statements would auto-commit
        // independently, leaving the DB in a partial state if the process
        // dies between them (messages gone but session not flagged archived,
        // or vice versa). On the next archive sweep that partial state can
        // compound. Use `unchecked_transaction()` because `with_connection`
        // hands out `&Connection` (not `&mut`). Closes TD-001 in
        // `memory-bank/tech-debt.md`.
        self.db.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM messages WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM execution_logs WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "UPDATE sessions SET archived = 1 WHERE id = ?1",
                params![session_id],
            )?;
            tx.commit()?;
            Ok(())
        })?;

        tracing::info!(
            session_id = session_id,
            messages = messages_count,
            logs = logs_count,
            file_size = file_size,
            "Session archived to {}",
            archive_file.display()
        );

        Ok(ArchiveResult {
            session_id: session_id.to_string(),
            messages_archived: messages_count,
            logs_archived: logs_count,
            file_size,
        })
    }

    /// Archive all eligible old sessions.
    ///
    /// A session is eligible when:
    /// - status is 'completed' or 'crashed'
    /// - archived = 0
    /// - completed_at is older than `older_than_days`
    /// - a distillation_runs entry with status = 'success' exists
    pub fn archive_old_sessions(&self, older_than_days: u32) -> Result<Vec<ArchiveResult>, String> {
        let session_ids: Vec<String> = self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id
                 FROM sessions s
                 INNER JOIN distillation_runs d ON d.session_id = s.id
                 WHERE s.status IN ('completed', 'crashed')
                   AND s.archived = 0
                   AND s.completed_at < datetime('now', ?1)
                   AND d.status = 'success'",
            )?;
            let offset = format!("-{} days", older_than_days);
            let rows = stmt.query_map([&offset], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        let mut results = Vec::new();
        for session_id in &session_ids {
            match self.archive_session(session_id) {
                Ok(result) => results.push(result),
                Err(e) => {
                    tracing::warn!(session_id = session_id.as_str(), "Archive failed: {}", e);
                    // Continue with remaining sessions
                }
            }
        }

        Ok(results)
    }

    /// Restore an archived session from its compressed JSONL file back into SQLite.
    ///
    /// Returns the total number of records restored (messages + logs).
    pub fn restore_session(&self, session_id: &str) -> Result<usize, String> {
        let archive_file = self.archive_path.join(format!("{}.jsonl.gz", session_id));

        if !archive_file.exists() {
            return Err(format!(
                "Archive file not found: {}",
                archive_file.display()
            ));
        }

        // 1. Read and decompress
        let file = std::fs::File::open(&archive_file)
            .map_err(|e| format!("Failed to open archive file: {e}"))?;
        let mut decoder = GzDecoder::new(file);
        let mut content = String::new();
        decoder
            .read_to_string(&mut content)
            .map_err(|e| format!("Failed to decompress archive: {e}"))?;

        // 2. Parse JSONL lines
        let mut messages_restored = 0usize;
        let mut logs_restored = 0usize;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let archive_line: ArchiveLine = serde_json::from_str(line)
                .map_err(|e| format!("Failed to parse archive line: {e}"))?;

            match archive_line.record_type.as_str() {
                "message" => {
                    self.restore_message(&archive_line.data)?;
                    messages_restored += 1;
                }
                "execution_log" => {
                    self.restore_execution_log(&archive_line.data)?;
                    logs_restored += 1;
                }
                other => {
                    tracing::warn!("Unknown record type in archive: {}", other);
                }
            }
        }

        // 3. Mark session as not archived
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET archived = 0 WHERE id = ?1",
                params![session_id],
            )?;
            Ok(())
        })?;

        // 4. Delete the archive file
        std::fs::remove_file(&archive_file)
            .map_err(|e| format!("Failed to delete archive file: {e}"))?;

        let total = messages_restored + logs_restored;
        tracing::info!(
            session_id = session_id,
            messages = messages_restored,
            logs = logs_restored,
            "Session restored ({} records)",
            total
        );

        Ok(total)
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Re-insert a single message row from its JSON representation.
    fn restore_message(&self, data: &serde_json::Value) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO messages
                    (id, execution_id, session_id, role, content, created_at,
                     token_count, tool_calls, tool_results, tool_call_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    data["id"].as_str().unwrap_or_default(),
                    data["execution_id"].as_str(),
                    data["session_id"].as_str(),
                    data["role"].as_str().unwrap_or_default(),
                    data["content"].as_str().unwrap_or_default(),
                    data["created_at"].as_str().unwrap_or_default(),
                    data["token_count"].as_i64().unwrap_or(0) as i32,
                    data["tool_calls"].as_str(),
                    data["tool_results"].as_str(),
                    data["tool_call_id"].as_str(),
                ],
            )?;
            Ok(())
        })
    }

    /// Re-insert a single execution_log row from its JSON representation.
    fn restore_execution_log(&self, data: &serde_json::Value) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO execution_logs
                    (id, session_id, conversation_id, agent_id, parent_session_id,
                     timestamp, level, category, message, metadata, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    data["id"].as_str().unwrap_or_default(),
                    data["session_id"].as_str().unwrap_or_default(),
                    data["conversation_id"].as_str(),
                    data["agent_id"].as_str().unwrap_or_default(),
                    data["parent_session_id"].as_str(),
                    data["timestamp"].as_str().unwrap_or_default(),
                    data["level"].as_str().unwrap_or_default(),
                    data["category"].as_str().unwrap_or_default(),
                    data["message"].as_str().unwrap_or_default(),
                    data["metadata"].as_str(),
                    data["duration_ms"].as_i64(),
                ],
            )?;
            Ok(())
        })
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::VaultPaths;
    use rusqlite::params;
    use tempfile::TempDir;

    /// Build an in-memory archiver against a tempdir-backed DB.
    /// Returns the archiver, the DB handle (so tests can seed rows), and
    /// the tempdir guard (kept alive for the test's duration).
    fn fresh() -> (SessionArchiver, Arc<DatabaseManager>, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = Arc::new(DatabaseManager::new(paths).expect("db init"));
        let archive_dir = tmp.path().join("archives");
        let archiver = SessionArchiver::new(db.clone(), archive_dir);
        (archiver, db, tmp)
    }

    /// Insert a session row with the chosen `status`, `archived` flag, and
    /// `completed_at`. `completed_at` accepts a SQLite-compatible datetime
    /// string (e.g. `datetime('now', '-30 days')`) by passing the modifier.
    fn seed_session(
        db: &Arc<DatabaseManager>,
        id: &str,
        status: &str,
        archived: i64,
        age_days: i64,
    ) {
        db.with_connection(|conn| {
            // Use SQLite's datetime() so the comparison in the WHERE clause
            // (`s.completed_at < datetime('now', ?1)`) operates on the same
            // wall clock the archiver uses.
            let completed_at: String = conn.query_row(
                &format!("SELECT datetime('now', '-{age_days} days')"),
                [],
                |row| row.get(0),
            )?;
            conn.execute(
                "INSERT INTO sessions (id, status, root_agent_id, created_at, completed_at, archived) \
                 VALUES (?1, ?2, 'root', datetime('now'), ?3, ?4)",
                params![id, status, completed_at, archived],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// Insert a `distillation_runs` row for the session.
    fn seed_distillation_run(db: &Arc<DatabaseManager>, session_id: &str, status: &str) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO distillation_runs \
                   (id, session_id, status, facts_extracted, entities_extracted, relationships_extracted, episode_created, retry_count, created_at) \
                 VALUES (?1, ?2, ?3, 0, 0, 0, 0, 0, datetime('now'))",
                params![format!("dr-{session_id}"), session_id, status],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// Run `archive_old_sessions` and return the set of session IDs that
    /// were selected. Use a tiny `older_than_days` so tests with seeded
    /// `age_days >= 1` are eligible.
    fn archive(archiver: &SessionArchiver) -> Vec<String> {
        archiver
            .archive_old_sessions(1)
            .expect("archive run ok")
            .into_iter()
            .map(|r| r.session_id)
            .collect()
    }

    #[test]
    fn archives_only_completed_or_crashed_sessions() {
        let (archiver, db, _tmp) = fresh();
        // Mix of statuses, all 30 days old, all with successful distillation.
        seed_session(&db, "s-running", "running", 0, 30);
        seed_session(&db, "s-paused", "paused", 0, 30);
        seed_session(&db, "s-completed", "completed", 0, 30);
        seed_session(&db, "s-crashed", "crashed", 0, 30);
        for id in &["s-running", "s-paused", "s-completed", "s-crashed"] {
            seed_distillation_run(&db, id, "success");
        }

        let archived = archive(&archiver);
        let mut a = archived.clone();
        a.sort();
        assert_eq!(
            a,
            vec!["s-completed".to_string(), "s-crashed".to_string()],
            "only terminal sessions should archive: {archived:?}"
        );
    }

    #[test]
    fn skips_sessions_already_archived() {
        let (archiver, db, _tmp) = fresh();
        seed_session(&db, "s-fresh", "completed", 0, 30);
        seed_session(&db, "s-already", "completed", 1, 30);
        seed_distillation_run(&db, "s-fresh", "success");
        seed_distillation_run(&db, "s-already", "success");

        let archived = archive(&archiver);
        assert_eq!(archived, vec!["s-fresh".to_string()]);
    }

    #[test]
    fn skips_sessions_younger_than_threshold() {
        let (archiver, db, _tmp) = fresh();
        // Threshold = 1 day. age 0 is younger; age 30 is older.
        seed_session(&db, "s-recent", "completed", 0, 0);
        seed_session(&db, "s-old", "completed", 0, 30);
        seed_distillation_run(&db, "s-recent", "success");
        seed_distillation_run(&db, "s-old", "success");

        let archived = archive(&archiver);
        assert_eq!(archived, vec!["s-old".to_string()]);
    }

    #[test]
    fn requires_distillation_runs_status_success() {
        let (archiver, db, _tmp) = fresh();
        seed_session(&db, "s-ok", "completed", 0, 30);
        seed_session(&db, "s-failed-distill", "completed", 0, 30);
        seed_session(&db, "s-skipped-distill", "completed", 0, 30);
        seed_distillation_run(&db, "s-ok", "success");
        seed_distillation_run(&db, "s-failed-distill", "failed");
        seed_distillation_run(&db, "s-skipped-distill", "skipped");

        let archived = archive(&archiver);
        assert_eq!(
            archived,
            vec!["s-ok".to_string()],
            "non-success distillation_runs must be excluded: {archived:?}"
        );
    }

    #[test]
    fn skips_sessions_with_no_distillation_run_record() {
        let (archiver, db, _tmp) = fresh();
        // Session is otherwise eligible — but no distillation_runs row.
        // INNER JOIN should drop it.
        seed_session(&db, "s-no-distill", "completed", 0, 30);

        let archived = archive(&archiver);
        assert!(
            archived.is_empty(),
            "session without distillation_runs row must NOT archive: {archived:?}"
        );
    }

    #[test]
    fn archives_nothing_when_pool_empty() {
        let (archiver, _db, _tmp) = fresh();
        let archived = archive(&archiver);
        assert!(archived.is_empty());
    }

    #[test]
    fn full_eligibility_matrix() {
        // All four conditions must hold simultaneously. Verify each axis
        // independently disqualifies one of an otherwise-identical pair.
        let (archiver, db, _tmp) = fresh();
        // The one session that satisfies ALL four conditions:
        seed_session(&db, "s-pass", "completed", 0, 30);
        seed_distillation_run(&db, "s-pass", "success");

        // Plus one fail-on-each-axis decoy:
        seed_session(&db, "s-fail-status", "running", 0, 30);
        seed_distillation_run(&db, "s-fail-status", "success");
        seed_session(&db, "s-fail-archived", "completed", 1, 30);
        seed_distillation_run(&db, "s-fail-archived", "success");
        seed_session(&db, "s-fail-age", "completed", 0, 0);
        seed_distillation_run(&db, "s-fail-age", "success");
        seed_session(&db, "s-fail-distill", "completed", 0, 30);
        seed_distillation_run(&db, "s-fail-distill", "permanently_failed");

        let archived = archive(&archiver);
        assert_eq!(
            archived,
            vec!["s-pass".to_string()],
            "exactly one session matches all four conditions: {archived:?}"
        );
    }

    /// Insert an `agent_executions` row so messages can FK-reference it.
    fn seed_execution(db: &Arc<DatabaseManager>, session_id: &str, exec_id: &str) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_executions \
                   (id, session_id, agent_id, status, delegation_type, started_at) \
                 VALUES (?1, ?2, 'root', 'completed', 'root', datetime('now'))",
                params![exec_id, session_id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// Insert a message row tied to a session + execution.
    fn seed_message(db: &Arc<DatabaseManager>, session_id: &str, exec_id: &str, idx: usize) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO messages (id, execution_id, session_id, role, content, created_at) \
                 VALUES (?1, ?2, ?3, 'user', 'hello', datetime('now'))",
                params![format!("msg-{session_id}-{idx}"), exec_id, session_id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// Insert an execution_logs row tied to a session.
    fn seed_log(db: &Arc<DatabaseManager>, session_id: &str, idx: usize) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO execution_logs \
                   (id, session_id, agent_id, timestamp, level, category, message) \
                 VALUES (?1, ?2, 'root', datetime('now'), 'info', 'test', 'log line')",
                params![format!("log-{session_id}-{idx}"), session_id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn count_rows(db: &Arc<DatabaseManager>, sql: &str, session_id: &str) -> i64 {
        db.with_connection(|conn| conn.query_row(sql, params![session_id], |row| row.get(0)))
            .unwrap()
    }

    /// Regression for TD-001: archive_session must atomically delete
    /// messages, delete execution_logs, and mark the session archived.
    /// Before the transaction wrapper, these were three independent
    /// auto-committed statements; a crash between them left the DB
    /// inconsistent. After the fix, the happy path still works end-to-end
    /// — that's what this test verifies.
    #[test]
    fn archive_session_atomically_deletes_and_marks() {
        let (archiver, db, _tmp) = fresh();
        seed_session(&db, "s1", "completed", 0, 30);
        seed_execution(&db, "s1", "exec-s1");
        seed_message(&db, "s1", "exec-s1", 0);
        seed_message(&db, "s1", "exec-s1", 1);
        seed_log(&db, "s1", 0);
        seed_log(&db, "s1", 1);
        seed_log(&db, "s1", 2);

        let result = archiver.archive_session("s1").expect("archive ok");
        assert_eq!(result.messages_archived, 2);
        assert_eq!(result.logs_archived, 3);
        assert!(result.file_size > 0, "archive file should have bytes");

        assert_eq!(
            count_rows(
                &db,
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                "s1"
            ),
            0,
            "messages must be gone"
        );
        assert_eq!(
            count_rows(
                &db,
                "SELECT COUNT(*) FROM execution_logs WHERE session_id = ?1",
                "s1"
            ),
            0,
            "logs must be gone"
        );
        assert_eq!(
            count_rows(&db, "SELECT archived FROM sessions WHERE id = ?1", "s1"),
            1,
            "session must be marked archived"
        );
    }
}
