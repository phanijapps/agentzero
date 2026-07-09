//! Repository for learned procedure patterns with CRUD and vector search.
//!
//! Phase 1b (v22): constructs on `KnowledgeDatabase` and stores embeddings in
//! the `procedures_index` vec0 virtual table through the `VectorIndex` trait.
//! The `embedding` column on `procedures` is gone; callers write normalized
//! vectors through `upsert_procedure`, which delegates to the injected
//! `VectorIndex`. Vectors MUST be L2-normalized by the caller.
//!
//! To read an embedding back, use [`ProcedureRepository::get_procedure_embedding`].

use crate::vector_index::VectorIndex;
use crate::KnowledgeDatabase;
use chrono::Utc;
use rusqlite::params;
use std::sync::Arc;

// Procedure moved to `zbot-stores-domain` (Phase D4).
pub use zbot_stores_domain::Procedure;

/// Repository for procedure CRUD and vector search.
pub struct ProcedureRepository {
    db: Arc<KnowledgeDatabase>,
    vec_index: Arc<dyn VectorIndex>,
}

impl ProcedureRepository {
    /// Create a new procedure repository.
    ///
    /// `vec_index` must wrap the `procedures_index` vec0 table (384-dim).
    pub fn new(db: Arc<KnowledgeDatabase>, vec_index: Arc<dyn VectorIndex>) -> Self {
        Self { db, vec_index }
    }

    /// Borrow the underlying knowledge database. Used by trait
    /// adapters that need to reach into the connection for queries
    /// not yet covered by named methods.
    pub fn db(&self) -> &Arc<KnowledgeDatabase> {
        &self.db
    }

    /// Insert or replace a procedure.
    ///
    /// If `proc.embedding` is `Some(v)`, the vector is written to
    /// `procedures_index` via the injected `VectorIndex`. **Callers must
    /// L2-normalize the vector first**.
    pub fn upsert_procedure(&self, proc: &Procedure) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO procedures \
                 (id, agent_id, ward_id, name, description, trigger_pattern, steps, \
                  parameters, success_count, failure_count, avg_duration_ms, avg_token_cost, \
                  last_used, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    proc.id,
                    proc.agent_id,
                    proc.ward_id,
                    proc.name,
                    proc.description,
                    proc.trigger_pattern,
                    proc.steps,
                    proc.parameters,
                    proc.success_count,
                    proc.failure_count,
                    proc.avg_duration_ms,
                    proc.avg_token_cost,
                    proc.last_used,
                    proc.created_at,
                    proc.updated_at,
                ],
            )?;
            Ok(())
        })?;

        if let Some(emb) = proc.embedding.as_ref() {
            self.vec_index.upsert(&proc.id, emb)?;
        }

        Ok(())
    }

    /// Get a procedure by ID.
    pub fn get_procedure(&self, id: &str) -> Result<Option<Procedure>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, ward_id, name, description, trigger_pattern, steps, \
                 parameters, success_count, failure_count, avg_duration_ms, avg_token_cost, \
                 last_used, created_at, updated_at \
                 FROM procedures WHERE id = ?1",
            )?;

            let mut rows = stmt.query_map(params![id], |row| Ok(Self::row_to_procedure(row)))?;

            match rows.next() {
                Some(Ok(proc)) => Ok(Some(proc)),
                Some(Err(e)) => Err(e),
                None => Ok(None),
            }
        })
    }

    /// List procedures for an agent, optionally filtered by ward.
    pub fn list_procedures(
        &self,
        agent_id: &str,
        ward_id: Option<&str>,
    ) -> Result<Vec<Procedure>, String> {
        self.db.with_connection(|conn| {
            if let Some(ward) = ward_id {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_id, ward_id, name, description, trigger_pattern, steps, \
                     parameters, success_count, failure_count, avg_duration_ms, avg_token_cost, \
                     last_used, created_at, updated_at \
                     FROM procedures WHERE agent_id = ?1 AND ward_id = ?2 ORDER BY name",
                )?;
                let procs = stmt
                    .query_map(params![agent_id, ward], |row| {
                        Ok(Self::row_to_procedure(row))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(procs)
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_id, ward_id, name, description, trigger_pattern, steps, \
                     parameters, success_count, failure_count, avg_duration_ms, avg_token_cost, \
                     last_used, created_at, updated_at \
                     FROM procedures WHERE agent_id = ?1 ORDER BY name",
                )?;
                let procs = stmt
                    .query_map(params![agent_id], |row| Ok(Self::row_to_procedure(row)))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(procs)
            }
        })
    }

    /// List procedures for a ward across all agents (ordered by `name`).
    ///
    /// Used by the ward content aggregator.
    pub fn list_by_ward(&self, ward_id: &str, limit: usize) -> Result<Vec<Procedure>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, ward_id, name, description, trigger_pattern, steps, \
                 parameters, success_count, failure_count, avg_duration_ms, avg_token_cost, \
                 last_used, created_at, updated_at \
                 FROM procedures WHERE ward_id = ?1 ORDER BY name LIMIT ?2",
            )?;
            let procs = stmt
                .query_map(params![ward_id, limit as i64], |row| {
                    Ok(Self::row_to_procedure(row))
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(procs)
        })
    }

    /// Search procedures by embedding similarity for an agent/ward.
    ///
    /// Performs a nearest-neighbor query through `VectorIndex`, then loads the
    /// matching `procedures` rows and filters by agent/ward in Rust. The
    /// returned score is cosine similarity (`1 - L2_sq / 2`), valid because
    /// stored and query vectors are required to be L2-normalized.
    pub fn search_by_similarity(
        &self,
        query_embedding: &[f32],
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(Procedure, f64)>, String> {
        // Over-fetch so post-filtering by agent/ward still returns `limit` hits.
        let fetch = limit.saturating_mul(4).max(limit);
        let nearest = self.vec_index.query_nearest(query_embedding, fetch)?;
        if nearest.is_empty() {
            return Ok(Vec::new());
        }

        let ids: Vec<String> = nearest.iter().map(|(id, _)| id.clone()).collect();
        let dist_by_id: std::collections::HashMap<String, f32> =
            nearest.iter().map(|(id, d)| (id.clone(), *d)).collect();

        let placeholders = (0..ids.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, agent_id, ward_id, name, description, trigger_pattern, steps, \
             parameters, success_count, failure_count, avg_duration_ms, avg_token_cost, \
             last_used, created_at, updated_at \
             FROM procedures WHERE id IN ({placeholders})"
        );

        let procedures: Vec<Procedure> = self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let params_iter = rusqlite::params_from_iter(ids.iter());
            let rows = stmt.query_map(params_iter, |row| Ok(Self::row_to_procedure(row)))?;
            Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        })?;

        let mut scored: Vec<(Procedure, f64)> = procedures
            .into_iter()
            .filter(|p| {
                p.agent_id == agent_id && ward_id.is_none_or(|w| p.ward_id.as_deref() == Some(w))
            })
            .map(|p| {
                let dist = dist_by_id.get(&p.id).copied().unwrap_or(f32::MAX);
                // L2 squared on normalized vectors → cosine = 1 - dist/2.
                let score = 1.0 - (dist as f64) / 2.0;
                (p, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }

    /// Increment success count and update running averages.
    pub fn increment_success(
        &self,
        id: &str,
        duration_ms: Option<i64>,
        token_cost: Option<i64>,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE procedures SET \
                 success_count = success_count + 1, \
                 last_used = ?4, \
                 avg_duration_ms = CASE \
                     WHEN avg_duration_ms IS NULL THEN ?2 \
                     WHEN ?2 IS NULL THEN avg_duration_ms \
                     ELSE (avg_duration_ms * (success_count - 1) + ?2) / success_count \
                 END, \
                 avg_token_cost = CASE \
                     WHEN avg_token_cost IS NULL THEN ?3 \
                     WHEN ?3 IS NULL THEN avg_token_cost \
                     ELSE (avg_token_cost * (success_count - 1) + ?3) / success_count \
                 END, \
                 updated_at = ?4 \
                 WHERE id = ?1",
                params![id, duration_ms, token_cost, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    /// Collapse duplicates by `(agent_id, name)`. For each name group with
    /// 2+ rows, keeps the row with the highest `success_count` (ties broken
    /// by most recent `created_at`) and deletes the rest. Cleans up
    /// `procedures_index` rows for deleted ids so similarity search stays
    /// consistent with the canonical row.
    ///
    /// Returns the number of rows deleted. Idempotent — a second call on a
    /// deduped table is a no-op.
    pub fn dedupe_by_name(&self) -> Result<usize, String> {
        let vec_index = &self.vec_index;
        // Phase 1: identify the canonical id per (agent_id, name) group via
        // a window function. Anything not in the keep-set is deleted.
        let to_delete: Vec<String> = self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id FROM (
                     SELECT id,
                            ROW_NUMBER() OVER (
                              PARTITION BY agent_id, name
                              ORDER BY success_count DESC, created_at DESC, id ASC
                            ) AS rn
                     FROM procedures
                 )
                 WHERE rn > 1",
            )?;
            let ids = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            Ok(ids)
        })?;
        if to_delete.is_empty() {
            return Ok(0);
        }
        // Phase 2: delete in batches (SQLite parameter limit ~999).
        let mut deleted = 0usize;
        for chunk in to_delete.chunks(500) {
            let placeholders = (0..chunk.len())
                .map(|i| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("DELETE FROM procedures WHERE id IN ({placeholders})");
            let chunk_ids: Vec<String> = chunk.to_vec();
            let n = self.db.with_connection(|conn| {
                let mut stmt = conn.prepare(&sql)?;
                let params_iter = rusqlite::params_from_iter(chunk_ids.iter());
                let n = stmt.execute(params_iter)?;
                Ok(n)
            })?;
            deleted += n;
            // Phase 3: clean up the vec index so the deleted procedure_ids
            // don't haunt similarity search. vec_index.delete is a no-op for
            // missing ids, so safe to call unconditionally.
            for id in chunk {
                vec_index.delete(id).ok();
            }
        }
        Ok(deleted)
    }

    /// Increment failure count.
    pub fn increment_failure(&self, id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE procedures SET \
                 failure_count = failure_count + 1, \
                 updated_at = ?2 \
                 WHERE id = ?1",
                params![id, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    /// Fetch the stored embedding for a procedure, if present in `procedures_index`.
    /// Returns `None` if the procedure has never been indexed.
    ///
    /// `sqlite-vec` stores vectors as `FLOAT[N]` BLOBs (little-endian f32s);
    /// we decode the raw bytes back to `Vec<f32>`.
    pub fn get_procedure_embedding(&self, procedure_id: &str) -> Result<Option<Vec<f32>>, String> {
        self.db.with_connection(|conn| {
            let r = conn.query_row(
                "SELECT embedding FROM procedures_index WHERE procedure_id = ?1",
                params![procedure_id],
                |row| row.get::<_, Vec<u8>>(0),
            );
            match r {
                Ok(blob) => Ok(Some(blob_to_f32_vec(&blob))),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    fn row_to_procedure(row: &rusqlite::Row) -> Procedure {
        Procedure {
            id: row.get(0).unwrap_or_default(),
            agent_id: row.get(1).unwrap_or_default(),
            ward_id: row.get(2).ok().flatten(),
            name: row.get(3).unwrap_or_default(),
            description: row.get(4).unwrap_or_default(),
            trigger_pattern: row.get(5).ok().flatten(),
            steps: row.get(6).unwrap_or_default(),
            parameters: row.get(7).ok().flatten(),
            success_count: row.get(8).unwrap_or(1),
            failure_count: row.get(9).unwrap_or(0),
            avg_duration_ms: row.get(10).ok().flatten(),
            avg_token_cost: row.get(11).ok().flatten(),
            last_used: row.get(12).ok().flatten(),
            embedding: None,
            created_at: row.get(13).unwrap_or_default(),
            updated_at: row.get(14).unwrap_or_default(),
        }
    }
}

fn blob_to_f32_vec(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_index::SqliteVecIndex;

    fn setup() -> (tempfile::TempDir, ProcedureRepository) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        let db = Arc::new(crate::KnowledgeDatabase::new(paths).expect("knowledge db"));
        let vec_index: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "procedures_index", "procedure_id")
                .expect("vec index init"),
        );
        let repo = ProcedureRepository::new(db, vec_index);
        (tmp, repo)
    }

    fn normalized(v: Vec<f32>) -> Vec<f32> {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-9 {
            v
        } else {
            v.into_iter().map(|x| x / norm).collect()
        }
    }

    fn make_procedure(id: &str, agent_id: &str, ward_id: Option<&str>) -> Procedure {
        Procedure {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            ward_id: ward_id.map(|s| s.to_string()),
            name: format!("proc-{id}"),
            description: format!("Description for {id}"),
            trigger_pattern: None,
            steps: r#"["step1","step2"]"#.to_string(),
            parameters: None,
            success_count: 1,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: "2026-04-11".to_string(),
            updated_at: "2026-04-11".to_string(),
        }
    }

    #[test]
    fn test_upsert_and_get_round_trip() {
        let (_tmp, repo) = setup();

        let proc = make_procedure("p1", "root", Some("__global__"));
        repo.upsert_procedure(&proc).unwrap();

        let fetched = repo.get_procedure("p1").unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.name, "proc-p1");
        assert_eq!(fetched.agent_id, "root");
        assert_eq!(fetched.steps, r#"["step1","step2"]"#);
    }

    #[test]
    fn test_search_by_similarity_finds_match() {
        let (_tmp, repo) = setup();

        let emb = normalized(
            (0..384)
                .map(|i| if i == 0 { 1.0_f32 } else { 0.0_f32 })
                .collect(),
        );
        let mut proc = make_procedure("p1", "root", Some("__global__"));
        proc.embedding = Some(emb.clone());
        repo.upsert_procedure(&proc).unwrap();

        let results = repo
            .search_by_similarity(&emb, "root", Some("__global__"), 5)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1 > 0.99);
    }

    #[test]
    fn test_increment_success_updates_count() {
        let (_tmp, repo) = setup();

        let proc = make_procedure("p1", "root", Some("__global__"));
        repo.upsert_procedure(&proc).unwrap();

        repo.increment_success("p1", Some(100), Some(500)).unwrap();

        let fetched = repo.get_procedure("p1").unwrap().unwrap();
        assert_eq!(fetched.success_count, 2);
        assert!(fetched.last_used.is_some());
    }

    #[test]
    fn test_increment_failure_updates_count() {
        let (_tmp, repo) = setup();

        let proc = make_procedure("p1", "root", Some("__global__"));
        repo.upsert_procedure(&proc).unwrap();

        repo.increment_failure("p1").unwrap();

        let fetched = repo.get_procedure("p1").unwrap().unwrap();
        assert_eq!(fetched.failure_count, 1);
    }

    #[test]
    fn test_list_procedures_filters_by_agent() {
        let (_tmp, repo) = setup();

        repo.upsert_procedure(&make_procedure("p1", "agent-a", Some("__global__")))
            .unwrap();
        repo.upsert_procedure(&make_procedure("p2", "agent-a", Some("__global__")))
            .unwrap();
        repo.upsert_procedure(&make_procedure("p3", "agent-b", Some("__global__")))
            .unwrap();

        let agent_a = repo.list_procedures("agent-a", None).unwrap();
        assert_eq!(agent_a.len(), 2);

        let agent_b = repo.list_procedures("agent-b", None).unwrap();
        assert_eq!(agent_b.len(), 1);

        // Filter by ward
        let filtered = repo.list_procedures("agent-a", Some("__global__")).unwrap();
        assert_eq!(filtered.len(), 2);

        let empty = repo
            .list_procedures("agent-a", Some("nonexistent"))
            .unwrap();
        assert_eq!(empty.len(), 0);
    }
}
