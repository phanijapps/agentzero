//! Repository for kg_episodes — records every extraction event for
//! provenance tracking. Facts, entities, and relationships reference
//! an episode ID so we can always answer "where did this come from?"

use crate::KnowledgeDatabase;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

// `EpisodeSource` and `KgEpisode` live in `zbot-stores-domain` so any
// backend impl can round-trip them without depending on this
// SQLite-coupled crate.
pub use zbot_stores_domain::{EpisodeSource, KgEpisode};

/// Aggregate counts of episodes by status, useful for progress reporting.
#[derive(Debug, Default, Clone)]
pub struct StatusCounts {
    pub pending: u64,
    pub running: u64,
    pub done: u64,
    pub failed: u64,
}

pub struct KgEpisodeRepository {
    db: Arc<KnowledgeDatabase>,
}

impl KgEpisodeRepository {
    pub fn new(db: Arc<KnowledgeDatabase>) -> Self {
        Self { db }
    }

    /// Insert an episode. Returns Ok(true) if inserted, Ok(false) if a duplicate
    /// (same content_hash + source_type) already exists.
    pub fn upsert_episode(&self, ep: &KgEpisode) -> Result<bool, String> {
        self.db.with_connection(|conn| {
            let changed = conn.execute(
                "INSERT OR IGNORE INTO kg_episodes \
                 (id, source_type, source_ref, content_hash, session_id, agent_id, \
                  status, retry_count, error, created_at, started_at, completed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    ep.id,
                    ep.source_type,
                    ep.source_ref,
                    ep.content_hash,
                    ep.session_id,
                    ep.agent_id,
                    ep.status,
                    ep.retry_count as i64,
                    ep.error,
                    ep.created_at,
                    ep.started_at,
                    ep.completed_at,
                ],
            )?;
            Ok(changed > 0)
        })
    }

    /// Look up an episode by content_hash + source_type. Used for dedup
    /// before extraction: if content hasn't changed, skip re-extraction.
    pub fn get_by_content_hash(
        &self,
        content_hash: &str,
        source_type: &str,
    ) -> Result<Option<KgEpisode>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, source_type, source_ref, content_hash, session_id, agent_id, \
                        status, retry_count, error, created_at, started_at, completed_at \
                 FROM kg_episodes WHERE content_hash = ?1 AND source_type = ?2",
            )?;
            let result = stmt
                .query_row(params![content_hash, source_type], row_to_episode)
                .optional()?;
            Ok(result)
        })
    }

    /// Get all episodes for a session.
    pub fn list_by_session(&self, session_id: &str) -> Result<Vec<KgEpisode>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, source_type, source_ref, content_hash, session_id, agent_id, \
                        status, retry_count, error, created_at, started_at, completed_at \
                 FROM kg_episodes WHERE session_id = ?1 ORDER BY created_at",
            )?;
            let rows = stmt
                .query_map(params![session_id], row_to_episode)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Get a single episode by ID.
    pub fn get(&self, id: &str) -> Result<Option<KgEpisode>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, source_type, source_ref, content_hash, session_id, agent_id, \
                        status, retry_count, error, created_at, started_at, completed_at \
                 FROM kg_episodes WHERE id = ?1",
            )?;
            let result = stmt.query_row(params![id], row_to_episode).optional()?;
            Ok(result)
        })
    }

    /// Insert a new episode with status='pending', deduplicating by (content_hash, source_type).
    /// Returns the episode ID — either the newly created one or the existing duplicate's ID.
    pub fn upsert_pending(
        &self,
        source_type: &str,
        source_ref: &str,
        content_hash: &str,
        session_id: Option<&str>,
        agent_id: &str,
    ) -> Result<String, String> {
        let id = format!("ep-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            let changed = conn.execute(
                "INSERT OR IGNORE INTO kg_episodes (
                     id, source_type, source_ref, content_hash, session_id, agent_id,
                     status, retry_count, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7)",
                params![
                    id,
                    source_type,
                    source_ref,
                    content_hash,
                    session_id,
                    agent_id,
                    now
                ],
            )?;
            if changed == 0 {
                // Duplicate content hash — return existing id.
                let existing: String = conn.query_row(
                    "SELECT id FROM kg_episodes WHERE content_hash = ?1 AND source_type = ?2",
                    params![content_hash, source_type],
                    |r| r.get(0),
                )?;
                Ok(existing)
            } else {
                Ok(id)
            }
        })
    }

    /// Atomically claim the next pending episode. Marks it `running` and stamps `started_at`.
    pub fn claim_next_pending(&self) -> Result<Option<KgEpisode>, String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut row: Option<KgEpisode> = match tx.query_row(
                "SELECT id, source_type, source_ref, content_hash, session_id, agent_id,
                        status, retry_count, error, created_at, started_at, completed_at
                 FROM kg_episodes
                 WHERE status = 'pending'
                 ORDER BY created_at ASC
                 LIMIT 1",
                [],
                row_to_episode,
            ) {
                Ok(e) => Some(e),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e),
            };
            if let Some(ref mut ep) = row {
                tx.execute(
                    "UPDATE kg_episodes SET status = 'running', started_at = ?1 WHERE id = ?2",
                    params![now, ep.id],
                )?;
                tx.commit()?;
                ep.status = "running".to_string();
                ep.started_at = Some(now);
            }
            Ok(row)
        })
    }

    /// Mark an episode as successfully completed.
    pub fn mark_done(&self, episode_id: &str) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE kg_episodes SET status = 'done', completed_at = ?1 WHERE id = ?2",
                params![now, episode_id],
            )?;
            Ok(())
        })
    }

    /// Mark an episode as failed, record the error message, and increment retry_count.
    pub fn mark_failed(&self, episode_id: &str, error: &str) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE kg_episodes
                 SET status = 'failed', error = ?1, completed_at = ?2,
                     retry_count = retry_count + 1
                 WHERE id = ?3",
                params![error, now, episode_id],
            )?;
            Ok(())
        })
    }

    /// Reset a failed episode back to pending if retry_count < max_retries.
    /// Returns true if the retry was scheduled, false if the episode has exhausted retries
    /// or does not exist.
    pub fn retry_if_eligible(&self, episode_id: &str, max_retries: u32) -> Result<bool, String> {
        self.db.with_connection(|conn| {
            let retry_count: u32 = match conn.query_row(
                "SELECT retry_count FROM kg_episodes WHERE id = ?1",
                params![episode_id],
                |r| r.get::<_, i64>(0),
            ) {
                Ok(n) => n.max(0) as u32,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
                Err(e) => return Err(e),
            };
            if retry_count >= max_retries {
                return Ok(false);
            }
            conn.execute(
                "UPDATE kg_episodes SET status = 'pending', error = NULL WHERE id = ?1",
                params![episode_id],
            )?;
            Ok(true)
        })
    }

    /// Store the raw chunk text for an episode. Called by the producer
    /// immediately after `upsert_pending`.
    pub fn set_payload(&self, episode_id: &str, text: &str) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO kg_episode_payloads (episode_id, text, created_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![episode_id, text, now],
            )?;
            Ok(())
        })
    }

    /// Fetch the stored chunk text for an episode. Returns None if absent
    /// (e.g. synchronous episodes — ward_artifact_indexer doesn't set a payload).
    pub fn get_payload(&self, episode_id: &str) -> Result<Option<String>, String> {
        self.db.with_connection(|conn| {
            match conn.query_row(
                "SELECT text FROM kg_episode_payloads WHERE episode_id = ?1",
                rusqlite::params![episode_id],
                |r| r.get::<_, String>(0),
            ) {
                Ok(s) => Ok(Some(s)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Count all episodes currently in pending or running state across all sources.
    pub fn count_pending_global(&self) -> Result<u64, String> {
        self.db.with_connection(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM kg_episodes WHERE status IN ('pending', 'running')",
                [],
                |r| r.get(0),
            )?;
            Ok(n.max(0) as u64)
        })
    }

    /// Count pending or running episodes whose source_ref starts with the given prefix.
    pub fn count_pending_for_source(&self, source_ref_prefix: &str) -> Result<u64, String> {
        self.db.with_connection(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM kg_episodes
                 WHERE source_ref LIKE ?1 || '%' AND status IN ('pending', 'running')",
                rusqlite::params![source_ref_prefix],
                |r| r.get(0),
            )?;
            Ok(n.max(0) as u64)
        })
    }

    /// Aggregate status counts for episodes whose source_ref starts with a given prefix.
    pub fn status_counts_for_source(
        &self,
        source_ref_prefix: &str,
    ) -> Result<StatusCounts, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT status, COUNT(*) FROM kg_episodes
                 WHERE source_ref LIKE ?1 || '%'
                 GROUP BY status",
            )?;
            let rows = stmt.query_map(params![source_ref_prefix], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            let mut counts = StatusCounts::default();
            for row in rows {
                let (s, n) = row?;
                let n = n.max(0) as u64;
                match s.as_str() {
                    "pending" => counts.pending = n,
                    "running" => counts.running = n,
                    "done" => counts.done = n,
                    "failed" => counts.failed = n,
                    _ => {}
                }
            }
            Ok(counts)
        })
    }
}

fn row_to_episode(row: &rusqlite::Row) -> rusqlite::Result<KgEpisode> {
    Ok(KgEpisode {
        id: row.get(0)?,
        source_type: row.get(1)?,
        source_ref: row.get(2)?,
        content_hash: row.get(3)?,
        session_id: row.get(4)?,
        agent_id: row.get(5)?,
        status: row.get(6)?,
        retry_count: row.get::<_, i64>(7)? as u32,
        error: row.get(8)?,
        created_at: row.get(9)?,
        started_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

#[cfg(test)]
impl KgEpisodeRepository {
    pub fn db_for_test(&self) -> &Arc<KnowledgeDatabase> {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeDatabase;
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, KgEpisodeRepository) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let repo = KgEpisodeRepository::new(db);
        (tmp, repo)
    }

    fn sample_episode() -> KgEpisode {
        KgEpisode {
            id: "ep-1".into(),
            source_type: "ward_file".into(),
            source_ref: "timeline.json".into(),
            content_hash: "abc123".into(),
            session_id: Some("sess-1".into()),
            agent_id: "root".into(),
            status: "pending".into(),
            retry_count: 0,
            error: None,
            created_at: "2026-04-12T00:00:00Z".into(),
            started_at: None,
            completed_at: None,
        }
    }

    #[test]
    fn upsert_and_get_by_id() {
        let (_tmp, repo) = setup();
        let ep = sample_episode();
        let inserted = repo.upsert_episode(&ep).unwrap();
        assert!(inserted);
        let fetched = repo.get("ep-1").unwrap().unwrap();
        assert_eq!(fetched.source_type, "ward_file");
        assert_eq!(fetched.source_ref, "timeline.json");
    }

    #[test]
    fn duplicate_content_hash_returns_false() {
        let (_tmp, repo) = setup();
        let ep = sample_episode();
        assert!(repo.upsert_episode(&ep).unwrap());
        let ep2 = KgEpisode {
            id: "ep-2".into(),
            ..ep
        };
        assert!(!repo.upsert_episode(&ep2).unwrap());
    }

    #[test]
    fn get_by_content_hash() {
        let (_tmp, repo) = setup();
        let ep = sample_episode();
        repo.upsert_episode(&ep).unwrap();
        let found = repo
            .get_by_content_hash("abc123", "ward_file")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "ep-1");
        assert!(repo
            .get_by_content_hash("abc123", "tool_result")
            .unwrap()
            .is_none());
    }

    #[test]
    fn list_by_session_returns_in_order() {
        let (_tmp, repo) = setup();
        for i in 0..3 {
            let ep = KgEpisode {
                id: format!("ep-{i}"),
                content_hash: format!("hash-{i}"),
                created_at: format!("2026-04-12T00:00:0{i}Z"),
                ..sample_episode()
            };
            repo.upsert_episode(&ep).unwrap();
        }
        let eps = repo.list_by_session("sess-1").unwrap();
        assert_eq!(eps.len(), 3);
        assert_eq!(eps[0].id, "ep-0");
        assert_eq!(eps[2].id, "ep-2");
    }

    #[test]
    fn get_missing_returns_none() {
        let (_tmp, repo) = setup();
        assert!(repo.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn episode_source_as_str_roundtrip() {
        assert_eq!(EpisodeSource::ToolResult.as_str(), "tool_result");
        assert_eq!(EpisodeSource::WardFile.as_str(), "ward_file");
        assert_eq!(EpisodeSource::Session.as_str(), "session");
        assert_eq!(EpisodeSource::Distillation.as_str(), "distillation");
        assert_eq!(EpisodeSource::UserInput.as_str(), "user_input");
    }

    #[test]
    fn pending_claim_done_happy_path() {
        let (_tmp, repo) = setup();

        let id = repo
            .upsert_pending("document", "src#chunk-0", "hash0", None, "root")
            .expect("upsert");

        let claimed = repo.claim_next_pending().expect("claim").expect("some");
        assert_eq!(claimed.id, id);
        assert_eq!(claimed.status, "running");

        // Second claim returns None — only one pending.
        let second = repo.claim_next_pending().expect("claim2");
        assert!(second.is_none());

        repo.mark_done(&id).expect("done");
        let counts = repo.status_counts_for_source("src").expect("counts");
        assert_eq!(counts.pending, 0);
        assert_eq!(counts.running, 0);
        assert_eq!(counts.done, 1);
        assert_eq!(counts.failed, 0);
    }

    #[test]
    fn mark_failed_increments_retry_count() {
        let (_tmp, repo) = setup();

        let id = repo
            .upsert_pending("document", "src#chunk-1", "hash1", None, "root")
            .expect("upsert");
        let _ = repo.claim_next_pending().expect("claim");
        repo.mark_failed(&id, "test error").expect("fail");

        // retry_if_eligible with max=3 → should succeed; status back to pending.
        let retried = repo.retry_if_eligible(&id, 3).expect("retry");
        assert!(retried);

        // Claim it again and fail; retry_count should be 1 now.
        let _ = repo.claim_next_pending().expect("claim2");
        repo.mark_failed(&id, "test error 2").expect("fail2");

        // With max=2, retry_count=2 is NOT < 2, so no retry.
        let retried2 = repo.retry_if_eligible(&id, 2).expect("retry2");
        assert!(!retried2);
    }

    #[test]
    fn upsert_pending_deduplicates_by_content_hash() {
        let (_tmp, repo) = setup();

        let id1 = repo
            .upsert_pending("document", "a", "same_hash", None, "root")
            .expect("up1");
        let id2 = repo
            .upsert_pending("document", "b", "same_hash", None, "root")
            .expect("up2");
        assert_eq!(id1, id2, "same content_hash + source_type should dedup");
    }

    #[test]
    fn payload_roundtrip() {
        let (_tmp, repo) = setup();
        let id = repo
            .upsert_pending("document", "src#0", "hash0", None, "root")
            .unwrap();

        // Initially missing.
        assert_eq!(repo.get_payload(&id).unwrap(), None);

        // Set and read.
        repo.set_payload(&id, "the quick brown fox").unwrap();
        assert_eq!(
            repo.get_payload(&id).unwrap().as_deref(),
            Some("the quick brown fox")
        );

        // Overwrite.
        repo.set_payload(&id, "updated").unwrap();
        assert_eq!(repo.get_payload(&id).unwrap().as_deref(), Some("updated"));
    }

    #[test]
    fn count_pending_filters_by_status_and_source() {
        let (_tmp, repo) = setup();

        let _id1 = repo
            .upsert_pending("document", "book-rise#chunk-0", "h0", None, "root")
            .unwrap();
        let id2 = repo
            .upsert_pending("document", "book-rise#chunk-1", "h1", None, "root")
            .unwrap();
        let _id3 = repo
            .upsert_pending("document", "other-source#chunk-0", "h2", None, "root")
            .unwrap();

        assert_eq!(repo.count_pending_global().unwrap(), 3);
        assert_eq!(repo.count_pending_for_source("book-rise").unwrap(), 2);
        assert_eq!(repo.count_pending_for_source("other-source").unwrap(), 1);

        // Mark one done — count drops.
        repo.mark_done(&id2).unwrap();
        assert_eq!(repo.count_pending_global().unwrap(), 2);
        assert_eq!(repo.count_pending_for_source("book-rise").unwrap(), 1);
    }

    #[test]
    fn payload_cascade_delete_when_episode_removed() {
        let (_tmp, repo) = setup();
        let id = repo
            .upsert_pending("document", "src#0", "hash0", None, "root")
            .unwrap();
        repo.set_payload(&id, "text").unwrap();

        // Delete the episode row directly — trigger cascade via FK ON DELETE CASCADE.
        repo.db_for_test()
            .with_connection(|conn| {
                conn.execute(
                    "DELETE FROM kg_episodes WHERE id = ?1",
                    rusqlite::params![id],
                )?;
                Ok(())
            })
            .unwrap();

        assert_eq!(repo.get_payload(&id).unwrap(), None);
    }
}
