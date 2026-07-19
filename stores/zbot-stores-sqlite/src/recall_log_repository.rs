// ============================================================================
// RECALL LOG REPOSITORY
// Track which memory facts were recalled in each session
// ============================================================================

use rusqlite::params;
use std::collections::HashMap;
use std::sync::Arc;

use crate::DatabaseManager;

// ============================================================================
// RECALL LOG REPOSITORY
// ============================================================================

/// Repository for recall log operations — tracks which facts were surfaced per session.
pub struct RecallLogRepository {
    db: Arc<DatabaseManager>,
}

impl RecallLogRepository {
    /// Create a new recall log repository.
    pub fn new(db: Arc<DatabaseManager>) -> Self {
        Self { db }
    }

    /// Log that a fact was recalled in a session. Idempotent (INSERT OR IGNORE).
    pub fn log_recall(&self, session_id: &str, fact_key: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO recall_log (session_id, fact_key, recalled_at)
                 VALUES (?1, ?2, ?3)",
                params![session_id, fact_key, chrono::Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    /// Get all fact keys recalled in a specific session.
    pub fn get_keys_for_session(&self, session_id: &str) -> Result<Vec<String>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT fact_key FROM recall_log WHERE session_id = ?1")?;

            let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Get fact keys recalled across multiple sessions with occurrence counts.
    /// Returns HashMap<fact_key, count_of_sessions_that_recalled_it>.
    pub fn get_keys_for_sessions(
        &self,
        session_ids: &[&str],
    ) -> Result<HashMap<String, usize>, String> {
        if session_ids.is_empty() {
            return Ok(HashMap::new());
        }

        self.db.with_connection(|conn| {
            // Build placeholders: (?1, ?2, ?3, ...)
            let placeholders: Vec<String> =
                (1..=session_ids.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!(
                "SELECT fact_key, COUNT(DISTINCT session_id) as cnt
                 FROM recall_log
                 WHERE session_id IN ({})
                 GROUP BY fact_key",
                placeholders.join(", ")
            );

            let mut stmt = conn.prepare(&sql)?;

            let params: Vec<&dyn rusqlite::types::ToSql> = session_ids
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();

            let rows = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })?;

            let mut map = HashMap::new();
            for row in rows {
                let (key, count) = row?;
                map.insert(key, count);
            }
            Ok(map)
        })
    }
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

    #[test]
    fn test_log_and_get_keys() {
        let db = create_test_db();
        let repo = RecallLogRepository::new(db);

        // Log 2 keys for sess-1
        repo.log_recall("sess-1", "user::name").unwrap();
        repo.log_recall("sess-1", "user::email").unwrap();

        // Log 1 key for sess-2 (overlapping with sess-1)
        repo.log_recall("sess-2", "user::name").unwrap();

        // Verify get_keys_for_session
        let keys = repo.get_keys_for_session("sess-1").unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"user::name".to_string()));
        assert!(keys.contains(&"user::email".to_string()));

        let keys2 = repo.get_keys_for_session("sess-2").unwrap();
        assert_eq!(keys2.len(), 1);
        assert!(keys2.contains(&"user::name".to_string()));

        // Verify get_keys_for_sessions counts correctly
        let counts = repo.get_keys_for_sessions(&["sess-1", "sess-2"]).unwrap();
        assert_eq!(counts.get("user::name"), Some(&2)); // recalled in both sessions
        assert_eq!(counts.get("user::email"), Some(&1)); // recalled in sess-1 only
    }

    #[test]
    fn test_log_recall_idempotent() {
        let db = create_test_db();
        let repo = RecallLogRepository::new(db);

        // Log same key twice for same session — should not error
        repo.log_recall("sess-1", "user::name").unwrap();
        repo.log_recall("sess-1", "user::name").unwrap();

        // Should still return only 1 entry
        let keys = repo.get_keys_for_session("sess-1").unwrap();
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_empty_sessions() {
        let db = create_test_db();
        let repo = RecallLogRepository::new(db);

        // Non-existent session returns empty vec
        let keys = repo.get_keys_for_session("nonexistent").unwrap();
        assert!(keys.is_empty());

        // Empty slice returns empty map
        let counts = repo.get_keys_for_sessions(&[]).unwrap();
        assert!(counts.is_empty());
    }
}
