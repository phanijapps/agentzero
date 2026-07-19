// ============================================================================
// MEMORY FACTS REPOSITORY
// CRUD, hybrid search, and embedding cache for the memory evolution system.
//
// Phase 1b (v22): constructs on `KnowledgeDatabase` and stores embeddings in
// the `memory_facts_index` vec0 virtual table through the `VectorIndex` trait.
// The `embedding` column on `memory_facts` is gone; callers write normalized
// vectors through `upsert_memory_fact`/`update_fact_embedding`, which delegate
// to the injected `VectorIndex`.
// ============================================================================

use chrono::{DateTime, Utc};
use rusqlite::params;
use std::sync::Arc;

use crate::vector_index::VectorIndex;
use crate::KnowledgeDatabase;

// ============================================================================
// FTS5 QUERY SANITIZATION
// ============================================================================

/// Sanitize a raw user message for FTS5 MATCH queries.
/// Extracts alphanumeric words (>2 chars), joins with OR.
/// Raw messages contain commas, parens, dashes, dollar signs that break FTS5 syntax.
pub fn sanitize_fts_query(raw: &str) -> String {
    let words: Vec<&str> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|w| w.trim())
        .filter(|w| w.len() > 2)
        .filter(|w| {
            ![
                "the", "and", "for", "with", "that", "this", "from", "have", "been", "will",
                "should", "would", "could", "their", "there", "not", "are", "was", "can", "all",
                "has", "its", "than",
            ]
            .contains(w)
        })
        .collect();
    words.join(" OR ")
}

// ============================================================================
// TYPES
// ============================================================================

// `SkillIndexRow` is defined in `zbot-stores-traits` so the
// `MemoryFactStore` trait can use it in its method signatures without a
// reverse dependency on this crate. Re-exported via this module for callers
// that still use the repository module as their persistence prelude.
pub use zbot_stores_traits::SkillIndexRow;

// MemoryFact + ScoredFact moved to `zbot-stores-domain` (Phase D1) so any
// backend impl can round-trip them without depending on this SQLite-coupled
// crate. Re-exported here so the 80+ existing import sites of
// `gateway_database::MemoryFact` continue to compile unchanged.
pub use zbot_stores_domain::{MemoryFact, ScoredFact};

// ============================================================================
// MEMORY REPOSITORY
// ============================================================================

/// SELECT column list (in positional order) for a `memory_facts` row — no
/// `embedding` column (it moved to `memory_facts_index` in v22).
const FACT_COLUMNS: &str = "id, session_id, agent_id, scope, category, key, content, confidence, \
    mention_count, source_summary, ward_id, contradicted_by, created_at, updated_at, expires_at, \
    valid_from, valid_until, superseded_by, pinned, epistemic_class, source_episode_id, source_ref";

/// Same columns, prefixed with `mf.` for use in JOIN queries.
const FACT_COLUMNS_MF: &str = "mf.id, mf.session_id, mf.agent_id, mf.scope, mf.category, mf.key, \
    mf.content, mf.confidence, mf.mention_count, mf.source_summary, mf.ward_id, mf.contradicted_by, \
    mf.created_at, mf.updated_at, mf.expires_at, mf.valid_from, mf.valid_until, mf.superseded_by, \
    mf.pinned, mf.epistemic_class, mf.source_episode_id, mf.source_ref";

/// Bi-temporal point-in-time recall filter. Returns the SQL WHERE fragment
/// (column-prefixed for joined queries; pass `""` for un-aliased queries) and
/// the resolved RFC3339 timestamp the caller must bind once. `as_of = None`
/// defaults to `Utc::now()` — Option A from the bi-temporal design doc:
/// default recall always excludes facts whose `valid_until` is in the past,
/// fixing the latent bug where time-bounded-past facts were still returned.
fn as_of_filter(
    as_of: Option<chrono::DateTime<chrono::Utc>>,
    column_prefix: &str,
) -> (String, String) {
    let ts = as_of.unwrap_or_else(chrono::Utc::now).to_rfc3339();
    let filter = format!(
        "({prefix}valid_from IS NULL OR {prefix}valid_from <= ?) \
         AND ({prefix}valid_until IS NULL OR {prefix}valid_until > ?)",
        prefix = column_prefix,
    );
    (filter, ts)
}

/// Repository for memory fact operations.
pub struct MemoryRepository {
    db: Arc<KnowledgeDatabase>,
    vec_index: Arc<dyn VectorIndex>,
}

impl MemoryRepository {
    /// Create a new memory repository.
    ///
    /// `vec_index` must wrap the `memory_facts_index` vec0 table (384-dim).
    pub fn new(db: Arc<KnowledgeDatabase>, vec_index: Arc<dyn VectorIndex>) -> Self {
        Self { db, vec_index }
    }

    /// Internal accessor for the vector index, used by the hybrid search path.
    #[allow(dead_code)]
    fn vec_index(&self) -> &Arc<dyn VectorIndex> {
        &self.vec_index
    }

    /// Test-only accessor for the underlying KnowledgeDatabase.
    #[cfg(test)]
    pub(crate) fn db_for_tests(&self) -> Arc<crate::KnowledgeDatabase> {
        self.db.clone()
    }

    /// Borrow the underlying knowledge database. Used by trait
    /// adapters that need to reach into the connection for queries
    /// not yet covered by named methods.
    pub fn db(&self) -> &Arc<crate::KnowledgeDatabase> {
        &self.db
    }

    /// Aggregate counts for the memory subsystem stats endpoint.
    /// Returns zeros for tables that don't exist (handles fresh DBs).
    pub fn aggregate_subsystem_stats(
        &self,
    ) -> Result<zbot_stores_traits::MemoryAggregateStats, String> {
        self.db.with_connection(|conn| {
            let row =
                |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) };
            Ok(zbot_stores_traits::MemoryAggregateStats {
                facts: row("SELECT COUNT(*) FROM memory_facts"),
                episodes: row("SELECT COUNT(*) FROM kg_episodes"),
                procedures: row("SELECT COUNT(*) FROM procedures"),
                wiki_articles: row("SELECT COUNT(*) FROM ward_wiki_articles"),
                goals_active: row("SELECT COUNT(*) FROM kg_goals WHERE state = 'active'"),
            })
        })
    }

    /// Episode-pipeline health snapshot for the memory health endpoint.
    ///
    /// `queue_pending` matches the historical
    /// `KgEpisodeRepository::count_pending_global` semantics — it counts
    /// rows with `status IN ('pending', 'running')`. `queue_running`
    /// then breaks out the running subset so the UI can show how many
    /// are actively in flight vs. waiting. `failed_recent` counts
    /// `status = 'failed'`. The handler-side wire shape was preserved
    /// during the trait migration (see `gateway/src/http/memory.rs::health`).
    pub fn episode_health_metrics(
        &self,
    ) -> Result<zbot_stores_traits::MemoryHealthMetrics, String> {
        self.db.with_connection(|conn| {
            let row =
                |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) };
            let pending_or_running =
                row("SELECT COUNT(*) FROM kg_episodes WHERE status IN ('pending', 'running')");
            let running = row("SELECT COUNT(*) FROM kg_episodes WHERE status = 'running'");
            let failed = row("SELECT COUNT(*) FROM kg_episodes WHERE status = 'failed'");
            Ok(zbot_stores_traits::MemoryHealthMetrics {
                queue_pending: pending_or_running.max(0) as u64,
                queue_running: running.max(0) as u64,
                failed_recent: failed.max(0) as u64,
            })
        })
    }

    // =========================================================================
    // FACT CRUD
    // =========================================================================

    /// Upsert a memory fact.
    ///
    /// On conflict (same agent_id + scope + ward_id + key), updates content,
    /// bumps mention_count, and refreshes updated_at. If `fact.embedding` is
    /// `Some(v)`, the vector is written to `memory_facts_index` via the
    /// injected `VectorIndex`. **Callers must L2-normalize the vector first**
    /// — the index stores it verbatim, and cosine similarity derived from L2
    /// distance is only correct for normalized inputs.
    pub fn upsert_memory_fact(&self, fact: &MemoryFact) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO memory_facts (id, session_id, agent_id, scope, category, key, content, confidence, mention_count, source_summary, ward_id, contradicted_by, created_at, updated_at, expires_at, valid_from, valid_until, superseded_by, pinned, epistemic_class, source_episode_id, source_ref)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
                 ON CONFLICT(agent_id, scope, ward_id, key) DO UPDATE SET
                    content = CASE WHEN memory_facts.pinned = 1 THEN memory_facts.content ELSE excluded.content END,
                    confidence = CASE WHEN memory_facts.pinned = 1 THEN memory_facts.confidence ELSE MAX(memory_facts.confidence, excluded.confidence) END,
                    mention_count = memory_facts.mention_count + 1,
                    source_summary = COALESCE(excluded.source_summary, memory_facts.source_summary),
                    updated_at = excluded.updated_at,
                    session_id = COALESCE(excluded.session_id, memory_facts.session_id)",
                params![
                    fact.id,
                    fact.session_id,
                    fact.agent_id,
                    fact.scope,
                    fact.category,
                    fact.key,
                    fact.content,
                    fact.confidence,
                    fact.mention_count,
                    fact.source_summary,
                    fact.ward_id,
                    fact.contradicted_by,
                    fact.created_at,
                    fact.updated_at,
                    fact.expires_at,
                    fact.valid_from,
                    fact.valid_until,
                    fact.superseded_by,
                    fact.pinned as i32,
                    fact.epistemic_class,
                    fact.source_episode_id,
                    fact.source_ref,
                ],
            )?;
            Ok(())
        })?;

        if let Some(emb) = fact.embedding.as_ref() {
            self.vec_index.upsert(&fact.id, emb)?;
        }

        Ok(())
    }

    /// Get memory facts for an agent, optionally filtered by scope.
    pub fn get_memory_facts(
        &self,
        agent_id: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(s) = scope {
                    (
                        format!(
                            "SELECT {FACT_COLUMNS}
                         FROM memory_facts
                         WHERE agent_id = ?1 AND scope = ?2
                         ORDER BY updated_at DESC
                         LIMIT ?3"
                        ),
                        vec![
                            Box::new(agent_id.to_string()),
                            Box::new(s.to_string()),
                            Box::new(limit as i64),
                        ],
                    )
                } else {
                    (
                        format!(
                            "SELECT {FACT_COLUMNS}
                         FROM memory_facts
                         WHERE agent_id = ?1
                         ORDER BY updated_at DESC
                         LIMIT ?2"
                        ),
                        vec![Box::new(agent_id.to_string()), Box::new(limit as i64)],
                    )
                };

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Delete a memory fact by ID (and its vector index entry, if any).
    pub fn delete_memory_fact(&self, id: &str) -> Result<bool, String> {
        let deleted = self.db.with_connection(|conn| {
            let count = conn.execute("DELETE FROM memory_facts WHERE id = ?1", params![id])?;
            Ok(count > 0)
        })?;
        if deleted {
            // Best-effort: drop the vec0 row too. If already absent, VectorIndex
            // implementations are expected to no-op.
            self.vec_index.delete(id)?;
        }
        Ok(deleted)
    }

    /// Delete every memory fact matching `(category, key)` (and their vec0
    /// rows). Returns the number of facts removed. Used by the skill
    /// reindexer to clear ghost embeddings when a skill is removed from
    /// disk.
    pub fn delete_facts_by_key(&self, category: &str, key: &str) -> Result<usize, String> {
        // Collect ids first so we can also drop their vec0 rows.
        let ids: Vec<String> = self.db.with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT id FROM memory_facts WHERE category = ?1 AND key = ?2")?;
            let rows = stmt.query_map(params![category, key], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        if ids.is_empty() {
            return Ok(0);
        }

        // Delete rows. The `trg_facts_delete_vec` trigger handles the vec0
        // cleanup automatically; the explicit `vec_index.delete` below is
        // belt-and-suspenders for VectorIndex implementations that bypass
        // SQLite triggers (e.g. test stubs).
        let count = self.db.with_connection(|conn| {
            let mut total = 0usize;
            for id in &ids {
                total += conn.execute("DELETE FROM memory_facts WHERE id = ?1", params![id])?;
            }
            Ok(total)
        })?;

        for id in &ids {
            let _ = self.vec_index.delete(id);
        }

        Ok(count)
    }

    // =========================================================================
    // SKILL INDEX STATE
    // Per-skill staleness tracker for the incremental skill reindex.
    // =========================================================================

    /// Insert or replace one row in `skill_index_state`.
    pub fn upsert_skill_index_state(&self, row: &SkillIndexRow) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO skill_index_state
                    (name, source_root, file_path, mtime_unix, size_bytes, last_indexed_unix, format_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(name) DO UPDATE SET
                    source_root = excluded.source_root,
                    file_path = excluded.file_path,
                    mtime_unix = excluded.mtime_unix,
                    size_bytes = excluded.size_bytes,
                    last_indexed_unix = excluded.last_indexed_unix,
                    format_version = excluded.format_version",
                params![
                    row.name,
                    row.source_root,
                    row.file_path,
                    row.mtime_unix,
                    row.size_bytes,
                    row.last_indexed_unix,
                    row.format_version,
                ],
            )?;
            Ok(())
        })
    }

    /// List every row in `skill_index_state` for the diff at session start.
    pub fn list_skill_index_state(&self) -> Result<Vec<SkillIndexRow>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name, source_root, file_path, mtime_unix, size_bytes,
                        last_indexed_unix, format_version
                 FROM skill_index_state",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(SkillIndexRow {
                    name: row.get(0)?,
                    source_root: row.get(1)?,
                    file_path: row.get(2)?,
                    mtime_unix: row.get(3)?,
                    size_bytes: row.get(4)?,
                    last_indexed_unix: row.get(5)?,
                    format_version: row.get(6)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Remove a single skill from the staleness tracker. Used after the
    /// reindexer notices the on-disk file is gone.
    pub fn delete_skill_index_state(&self, name: &str) -> Result<bool, String> {
        self.db.with_connection(|conn| {
            let n = conn.execute(
                "DELETE FROM skill_index_state WHERE name = ?1",
                params![name],
            )?;
            Ok(n > 0)
        })
    }

    /// Get a single memory fact by ID.
    pub fn get_memory_fact_by_id(&self, id: &str) -> Result<Option<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let sql = format!(
                "SELECT {FACT_COLUMNS}
                 FROM memory_facts
                 WHERE id = ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let result = stmt.query_row(params![id], row_to_memory_fact);
            match result {
                Ok(fact) => Ok(Some(fact)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Get the currently-valid fact for a given key (not yet superseded).
    ///
    /// Returns `None` if no active fact exists for this agent/scope/ward/key combo.
    pub fn get_fact_by_key(
        &self,
        agent_id: &str,
        scope: &str,
        ward_id: &str,
        key: &str,
    ) -> Result<Option<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let sql = format!(
                "SELECT {FACT_COLUMNS}
                 FROM memory_facts
                 WHERE agent_id = ?1 AND scope = ?2 AND ward_id = ?3 AND key = ?4 AND valid_until IS NULL
                 LIMIT 1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let result = stmt.query_row(params![agent_id, scope, ward_id, key], row_to_memory_fact);
            match result {
                Ok(fact) => Ok(Some(fact)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// List primitive facts for a ward (function signatures extracted
    /// from the ward's source files by the AST post-write hook).
    /// Ordered by key for deterministic snapshot rendering.
    pub fn list_primitives_for_ward(&self, ward_id: &str) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let sql = format!(
                "SELECT {FACT_COLUMNS}
                 FROM memory_facts
                 WHERE category = 'primitive' AND ward_id = ?1 AND valid_until IS NULL
                 ORDER BY key"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![ward_id], row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// List recent ctx state-handoff facts for a session, newest first.
    ///
    /// Used by the ward-snapshot preamble builder to surface what prior
    /// subagents in the same session produced. Matches keys shaped as
    /// `ctx.<session_id>.state.<execution_id>`.
    pub fn list_recent_state_handoffs(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let pattern = format!("ctx.{}.state.%", session_id);
            let sql = format!(
                "SELECT {FACT_COLUMNS}
                 FROM memory_facts
                 WHERE category = 'ctx' AND key LIKE ?1 AND valid_until IS NULL
                 ORDER BY created_at DESC
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![pattern, limit], row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Mark an existing fact as superseded by a newer fact.
    ///
    /// Sets `valid_until = transition_time` and records the new fact's ID in
    /// `superseded_by`. `transition_time` is the moment the old fact stops
    /// being valid — usually `Utc::now()` (distillation), but the
    /// ConflictResolver passes the winner's `created_at` so the old fact's
    /// truth-interval closes at the precise moment the contradicting belief
    /// arrived. That preserves bi-temporal continuity: no gap, no overlap.
    pub fn supersede_fact(
        &self,
        old_id: &str,
        new_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE memory_facts SET valid_until = ?3, superseded_by = ?1 WHERE id = ?2",
                params![new_id, old_id, transition_time.to_rfc3339()],
            )?;
            Ok(())
        })
    }

    /// List memory facts with optional filters (category, scope) and pagination.
    pub fn list_memory_facts(
        &self,
        agent_id: &str,
        category: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                match (category, scope) {
                    (Some(cat), Some(scp)) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1 AND category = ?2 AND scope = ?3
                             ORDER BY updated_at DESC
                             LIMIT ?4 OFFSET ?5"
                        ),
                        vec![
                            Box::new(agent_id.to_string()),
                            Box::new(cat.to_string()),
                            Box::new(scp.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (Some(cat), None) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1 AND category = ?2
                             ORDER BY updated_at DESC
                             LIMIT ?3 OFFSET ?4"
                        ),
                        vec![
                            Box::new(agent_id.to_string()),
                            Box::new(cat.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (None, Some(scp)) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1 AND scope = ?2
                             ORDER BY updated_at DESC
                             LIMIT ?3 OFFSET ?4"
                        ),
                        vec![
                            Box::new(agent_id.to_string()),
                            Box::new(scp.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (None, None) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1
                             ORDER BY updated_at DESC
                             LIMIT ?2 OFFSET ?3"
                        ),
                        vec![
                            Box::new(agent_id.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                };

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// List memory facts for a ward (across all agents).
    ///
    /// Returns up to `limit` facts with `ward_id = ?1`, ordered by `updated_at`
    /// descending. Used by the ward content aggregator (`GET
    /// /api/wards/:ward_id/content`).
    pub fn list_by_ward(&self, ward_id: &str, limit: usize) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let sql = format!(
                "SELECT {FACT_COLUMNS}
                 FROM memory_facts
                 WHERE ward_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![ward_id, limit as i64], row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// List distinct wards with their fact counts.
    ///
    /// Returns one row per non-empty `ward_id`, sorted by `ward_id` ascending.
    /// Used by the command-deck ward navigator (`GET /api/wards`).
    pub fn list_wards(&self) -> Result<Vec<(String, usize)>, String> {
        // Union ward_ids across all content tables so the command-deck rail
        // shows wards that hold any content, not just memory_facts. The count
        // is summed across facts + wiki + procedures + episodes.
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT ward_id, SUM(c) AS total FROM (
                    SELECT ward_id, COUNT(*) AS c FROM memory_facts
                      WHERE ward_id IS NOT NULL AND ward_id != '' GROUP BY ward_id
                    UNION ALL
                    SELECT ward_id, COUNT(*) AS c FROM ward_wiki_articles
                      WHERE ward_id IS NOT NULL AND ward_id != '' GROUP BY ward_id
                    UNION ALL
                    SELECT ward_id, COUNT(*) AS c FROM procedures
                      WHERE ward_id IS NOT NULL AND ward_id != '' GROUP BY ward_id
                    UNION ALL
                    SELECT ward_id, COUNT(*) AS c FROM session_episodes
                      WHERE ward_id IS NOT NULL AND ward_id != '' GROUP BY ward_id
                 )
                 GROUP BY ward_id
                 ORDER BY ward_id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                let id: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((id, count as usize))
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Decay confidence of stale facts.
    ///
    /// Facts not updated in `older_than_days` have their confidence multiplied
    /// by `decay_factor` (e.g., 0.95). Returns number of facts decayed.
    pub fn decay_stale_facts(
        &self,
        older_than_days: u32,
        decay_factor: f64,
    ) -> Result<usize, String> {
        // Pre-compute the staleness cutoff in Rust so the SQL stays portable
        // (no SQLite-specific `julianday()` time arithmetic). Matches the
        // Phase 2 `datetime('now')` cleanup pattern. (TD-042)
        let cutoff = (Utc::now() - chrono::Duration::days(older_than_days as i64)).to_rfc3339();
        let now = Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            let count = conn.execute(
                "UPDATE memory_facts SET confidence = confidence * ?1, updated_at = ?2
                 WHERE updated_at < ?3
                 AND confidence > 0.1",
                params![decay_factor, now, cutoff],
            )?;
            Ok(count)
        })
    }

    /// Mark a fact as contradicted by a newer fact with the given key.
    ///
    /// Reduces confidence by 0.15 (floor of 0.1) and records which key contradicted it.
    pub fn mark_contradicted(
        &self,
        fact_id: &str,
        contradicted_by_key: &str,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE memory_facts SET contradicted_by = ?1, confidence = MAX(0.1, confidence - 0.15) WHERE id = ?2",
                params![contradicted_by_key, fact_id],
            )?;
            Ok(())
        })
    }

    /// Archive a fact by moving it from `memory_facts` to `memory_facts_archive`.
    ///
    /// Performs an atomic INSERT-then-DELETE within a single connection so the
    /// fact is never lost. Used by the pruning subsystem to keep the active
    /// store lean without discarding data. The archive table (v22) no longer
    /// carries an `embedding` column; the vec0 row is dropped alongside.
    pub fn archive_fact(&self, fact_id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO memory_facts_archive
                     (id, session_id, agent_id, scope, category, key, content, confidence,
                      ward_id, epistemic_class, archived_at)
                 SELECT id, session_id, agent_id, scope, category, key, content, confidence,
                        ward_id, COALESCE(epistemic_class, 'current'), ?2
                 FROM memory_facts WHERE id = ?1",
                params![fact_id, Utc::now().to_rfc3339()],
            )?;
            conn.execute("DELETE FROM memory_facts WHERE id = ?1", params![fact_id])?;
            Ok(())
        })?;
        // Drop the vec0 row so it doesn't outlive the archived fact.
        self.vec_index.delete(fact_id)?;
        Ok(())
    }

    /// Search for facts similar to the given embedding, filtered by minimum similarity threshold.
    ///
    /// Returns facts with cosine similarity >= `min_similarity`. Used for contradiction detection.
    ///
    /// When `agent_id` is `Some(a)`, results are scoped to facts visible to that
    /// agent: the agent's private facts (`scope='agent'`) plus any `scope='global'`
    /// facts. When `None`, no scope gate is applied (admin/debug path).
    pub fn search_similar_facts(
        &self,
        query_embedding: &[f32],
        agent_id: Option<&str>,
        min_similarity: f64,
        limit: usize,
        ward_id: Option<&str>,
    ) -> Result<Vec<ScoredFact>, String> {
        // Contradiction detection compares against currently-valid facts; the
        // bi-temporal filter defaults to Utc::now() (Option A).
        let mut results =
            self.search_memory_facts_vector(query_embedding, agent_id, limit, ward_id, None)?;
        results.retain(|sf| sf.score >= min_similarity);
        Ok(results)
    }

    // =========================================================================
    // FTS5 KEYWORD SEARCH
    // =========================================================================

    /// Search memory facts using FTS5 BM25 keyword matching.
    ///
    /// When `ward_id` is `Some(w)`, results are filtered to facts belonging to
    /// the `__global__` ward **or** the specified ward. When `None`, all wards
    /// are returned (no ward filtering).
    /// FTS5 keyword search across ALL agents (no agent_id filter).
    pub fn search_all_memory_facts_fts(
        &self,
        query: &str,
        limit: usize,
        category: Option<&str>,
    ) -> Result<Vec<ScoredFact>, String> {
        let sanitized_query = sanitize_fts_query(query);
        if sanitized_query.is_empty() {
            return Ok(Vec::new());
        }
        let query = &sanitized_query;

        self.db.with_connection(|conn| {
            let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(cat) = category {
                    (
                        format!(
                            "SELECT {FACT_COLUMNS_MF}, rank
                         FROM memory_facts_fts fts
                         JOIN memory_facts mf ON mf.rowid = fts.rowid
                         WHERE memory_facts_fts MATCH ?1 AND mf.category = ?2
                         ORDER BY rank
                         LIMIT ?3"
                        ),
                        vec![
                            Box::new(query.to_string()),
                            Box::new(cat.to_string()),
                            Box::new(limit as i64),
                        ],
                    )
                } else {
                    (
                        format!(
                            "SELECT {FACT_COLUMNS_MF}, rank
                         FROM memory_facts_fts fts
                         JOIN memory_facts mf ON mf.rowid = fts.rowid
                         WHERE memory_facts_fts MATCH ?1
                         ORDER BY rank
                         LIMIT ?2"
                        ),
                        vec![Box::new(query.to_string()), Box::new(limit as i64)],
                    )
                };

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                let fact = row_to_memory_fact(row)?;
                let bm25: f64 = row.get(22)?;
                Ok(ScoredFact { fact, score: -bm25 })
            })?;

            let results: Vec<ScoredFact> = rows.filter_map(|r| r.ok()).collect();
            Ok(results)
        })
    }

    /// FTS5 keyword search with scope-aware visibility.
    ///
    /// When `agent_id` is `Some(a)`, results include the agent's private facts
    /// (`scope='agent'`) AND any facts marked `scope='global'` (visible to
    /// every agent — domain knowledge, research, reference material, etc.).
    /// When `None`, no agent/scope filter is applied (admin/debug path).
    pub fn search_memory_facts_fts(
        &self,
        query: &str,
        agent_id: Option<&str>,
        limit: usize,
        ward_id: Option<&str>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ScoredFact>, String> {
        // Sanitize query for FTS5: extract alphanumeric words, join with OR.
        // Raw user messages contain commas, parens, dashes that break FTS5 syntax.
        // Using OR ensures any matching term contributes to results.
        let sanitized_query = sanitize_fts_query(query);
        if sanitized_query.is_empty() {
            return Ok(Vec::new());
        }
        let query = &sanitized_query;

        // Bi-temporal point-in-time gate. Option A: always apply (defaults to
        // Utc::now()) so facts with valid_until in the past are correctly
        // excluded from default recall. The fragment binds the `as_of`
        // timestamp twice (once per inequality).
        let (as_of_clause, as_of_ts) = as_of_filter(as_of, "mf.");

        self.db.with_connection(|conn| {
            let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                match (agent_id, ward_id) {
                    (Some(a), Some(w)) => (
                        format!(
                            "SELECT {FACT_COLUMNS_MF}, rank
                             FROM memory_facts_fts fts
                             JOIN memory_facts mf ON mf.rowid = fts.rowid
                             WHERE memory_facts_fts MATCH ?
                               AND ((mf.agent_id = ? AND mf.scope = 'agent') OR mf.scope = 'global')
                               AND (mf.ward_id = '__global__' OR mf.ward_id = ?)
                               AND {as_of_clause}
                             ORDER BY rank
                             LIMIT ?"
                        ),
                        vec![
                            Box::new(query.to_string()),
                            Box::new(a.to_string()),
                            Box::new(w.to_string()),
                            Box::new(as_of_ts.clone()),
                            Box::new(as_of_ts.clone()),
                            Box::new(limit as i64),
                        ],
                    ),
                    (Some(a), None) => (
                        format!(
                            "SELECT {FACT_COLUMNS_MF}, rank
                             FROM memory_facts_fts fts
                             JOIN memory_facts mf ON mf.rowid = fts.rowid
                             WHERE memory_facts_fts MATCH ?
                               AND ((mf.agent_id = ? AND mf.scope = 'agent') OR mf.scope = 'global')
                               AND {as_of_clause}
                             ORDER BY rank
                             LIMIT ?"
                        ),
                        vec![
                            Box::new(query.to_string()),
                            Box::new(a.to_string()),
                            Box::new(as_of_ts.clone()),
                            Box::new(as_of_ts.clone()),
                            Box::new(limit as i64),
                        ],
                    ),
                    (None, Some(w)) => (
                        format!(
                            "SELECT {FACT_COLUMNS_MF}, rank
                             FROM memory_facts_fts fts
                             JOIN memory_facts mf ON mf.rowid = fts.rowid
                             WHERE memory_facts_fts MATCH ?
                               AND (mf.ward_id = '__global__' OR mf.ward_id = ?)
                               AND {as_of_clause}
                             ORDER BY rank
                             LIMIT ?"
                        ),
                        vec![
                            Box::new(query.to_string()),
                            Box::new(w.to_string()),
                            Box::new(as_of_ts.clone()),
                            Box::new(as_of_ts.clone()),
                            Box::new(limit as i64),
                        ],
                    ),
                    (None, None) => (
                        format!(
                            "SELECT {FACT_COLUMNS_MF}, rank
                             FROM memory_facts_fts fts
                             JOIN memory_facts mf ON mf.rowid = fts.rowid
                             WHERE memory_facts_fts MATCH ?
                               AND {as_of_clause}
                             ORDER BY rank
                             LIMIT ?"
                        ),
                        vec![
                            Box::new(query.to_string()),
                            Box::new(as_of_ts.clone()),
                            Box::new(as_of_ts.clone()),
                            Box::new(limit as i64),
                        ],
                    ),
                };

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                let fact = row_to_memory_fact(row)?;
                let rank: f64 = row.get(22)?;
                // FTS5 rank is negative (lower = better). Normalize to 0..1 range.
                let bm25_score = (-rank).min(30.0) / 30.0;
                Ok(ScoredFact {
                    fact,
                    score: bm25_score,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    // =========================================================================
    // HYBRID SEARCH (FTS5 + VECTOR)
    // =========================================================================

    /// Hybrid search combining FTS5 keyword matching and vector similarity
    /// via Reciprocal Rank Fusion (Cormack & Clarke 2009).
    ///
    /// 1. Run FTS5 to get top-K keyword matches by BM25.
    /// 2. Run vector search via `VectorIndex` to get top-K by cosine similarity.
    /// 3. Fuse by rank: `rrf(d) = Σ_arm 1 / (k + rank_arm(d))`, `k = 60`.
    /// 4. Apply `confidence × recency × mention_boost` as relevance modulators.
    ///
    /// RRF is rank-based, so FTS's BM25 and vector cosine do not need to be in
    /// commensurable score regimes. This avoids the one-arm-zero collapse of
    /// weighted-score fusion, where an exact keyword hit absent from the vector
    /// top-K loses to semantic near-misses that happen to rank in both arms.
    ///
    /// `vector_weight` and `bm25_weight` are accepted for API-compat and IGNORED;
    /// RRF has no tunable weights.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn search_memory_facts_hybrid(
        &self,
        query_text: &str,
        query_embedding: Option<&[f32]>,
        agent_id: Option<&str>,
        limit: usize,
        _vector_weight: f64,
        _bm25_weight: f64,
        ward_id: Option<&str>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(Vec<ScoredFact>, Vec<(String, &'static str)>), String> {
        // Standard RRF constant. Dampens the rank-1 vs rank-20 gap so that
        // showing up in both arms (even mid-rank) outranks a single-arm top-1.
        const RRF_K: f64 = 60.0;

        // Retrieval pool: larger than output so fusion has room to reorder.
        let retrieval_k = (limit * 3).max(30);

        // Step 1: FTS5 keyword results.
        let fts_results = self
            .search_memory_facts_fts(query_text, agent_id, retrieval_k, ward_id, as_of)
            .unwrap_or_default();

        // Step 2: Vector results (if embedding provided).
        let vec_results = if let Some(qe) = query_embedding {
            self.search_memory_facts_vector(qe, agent_id, retrieval_k, ward_id, as_of)?
        } else {
            Vec::new()
        };

        // Step 3: Fuse by rank. Source classification:
        //   "fts"    — keyword-only match
        //   "vec"    — vector-only match
        //   "hybrid" — present in both arms
        type ScoreSlot = (f64, MemoryFact, &'static str);
        let mut score_map: std::collections::HashMap<String, ScoreSlot> =
            std::collections::HashMap::new();

        for (idx, sf) in fts_results.iter().enumerate() {
            let rank = (idx + 1) as f64;
            let contribution = 1.0 / (RRF_K + rank);
            score_map.insert(sf.fact.id.clone(), (contribution, sf.fact.clone(), "fts"));
        }

        for (idx, sf) in vec_results.into_iter().enumerate() {
            let rank = (idx + 1) as f64;
            let contribution = 1.0 / (RRF_K + rank);
            score_map
                .entry(sf.fact.id.clone())
                .and_modify(|(rrf, _, src)| {
                    *rrf += contribution;
                    *src = "hybrid";
                })
                .or_insert((contribution, sf.fact, "vec"));
        }

        // Step 4: Apply modulators. These are orthogonal to fusion — they
        // downweight stale / low-confidence facts and upweight frequently
        // referenced ones, independent of how a fact was retrieved.
        let now = chrono::Utc::now();
        let mut ranked: Vec<(ScoredFact, &'static str)> = score_map
            .into_values()
            .map(|(rrf_score, fact, src)| {
                let conf = fact.confidence;

                let days_old = chrono::DateTime::parse_from_rfc3339(&fact.updated_at)
                    .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_days() as f64)
                    .unwrap_or(30.0);
                let recency = 1.0 / (1.0 + days_old * 0.01);

                let mention_boost = 1.0 + (fact.mention_count as f64 + 1.0).ln() / 10.0_f64.ln();

                let final_score = rrf_score * conf * recency * mention_boost;

                (
                    ScoredFact {
                        fact,
                        score: final_score,
                    },
                    src,
                )
            })
            .collect();

        // Sort by score descending, take top-K
        ranked.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(limit);

        let sources: Vec<(String, &'static str)> = ranked
            .iter()
            .map(|(sf, src)| (sf.fact.id.clone(), *src))
            .collect();
        let results: Vec<ScoredFact> = ranked.into_iter().map(|(sf, _)| sf).collect();

        Ok((results, sources))
    }

    /// Search memory facts by vector similarity via `memory_facts_index` (vec0).
    ///
    /// Performs a nearest-neighbor query through `VectorIndex`, then loads the
    /// matching `memory_facts` rows and filters by agent / ward in Rust. The
    /// returned score is cosine similarity (`1 - L2_sq / 2`), valid because
    /// stored and query vectors are required to be L2-normalized.
    ///
    /// When `agent_id` is `Some(a)`, results include the agent's private facts
    /// (`scope='agent'`) AND any facts marked `scope='global'`. When `None`,
    /// no agent/scope filter is applied (admin/debug path).
    fn search_memory_facts_vector(
        &self,
        query_embedding: &[f32],
        agent_id: Option<&str>,
        limit: usize,
        ward_id: Option<&str>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ScoredFact>, String> {
        // Over-fetch so post-filtering by agent/ward (and bi-temporal as_of)
        // still returns `limit` hits.
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
        // Bi-temporal point-in-time gate. The two `?` placeholders bind the
        // same `as_of` timestamp; they sit AFTER the id placeholders so the
        // existing positional binding for `ids` is preserved.
        let (as_of_clause, as_of_ts) = as_of_filter(as_of, "");
        let sql = format!(
            "SELECT {FACT_COLUMNS} FROM memory_facts WHERE id IN ({placeholders}) AND {as_of_clause}"
        );

        let facts: Vec<MemoryFact> = self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            // Bind: id_1, ..., id_n, as_of, as_of
            let mut param_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
                ids.iter().map(|i| Box::new(i.clone()) as _).collect();
            param_vec.push(Box::new(as_of_ts.clone()));
            param_vec.push(Box::new(as_of_ts.clone()));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        let mut scored: Vec<ScoredFact> = facts
            .into_iter()
            // Scope-aware visibility: when agent_id is specified, return the
            // agent's own private facts plus any facts flagged as global. When
            // None, no agent/scope gate (admin/debug path).
            .filter(|f| match agent_id {
                Some(a) => (f.agent_id == a && f.scope == "agent") || f.scope == "global",
                None => true,
            })
            .filter(|f| match ward_id {
                Some(w) => f.ward_id == "__global__" || f.ward_id == w,
                None => true,
            })
            .map(|f| {
                let dist = dist_by_id.get(&f.id).copied().unwrap_or(f32::MAX);
                // L2 squared on normalized vectors → cosine = 1 - dist/2.
                let score = 1.0 - (dist as f64) / 2.0;
                ScoredFact { fact: f, score }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    // =========================================================================
    // EMBEDDING CACHE
    // =========================================================================

    /// Look up a cached embedding by content hash and model.
    pub fn get_cached_embedding(
        &self,
        content_hash: &str,
        model: &str,
    ) -> Result<Option<Vec<f32>>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT embedding FROM embedding_cache WHERE content_hash = ?1 AND model = ?2",
            )?;

            let result = stmt.query_row(params![content_hash, model], |row| {
                let blob: Vec<u8> = row.get(0)?;
                Ok(blob_to_f32_vec(&blob))
            });

            match result {
                Ok(vec) => Ok(Some(vec)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Cache an embedding for future reuse.
    pub fn cache_embedding(
        &self,
        content_hash: &str,
        model: &str,
        embedding: &[f32],
    ) -> Result<(), String> {
        let blob = f32_vec_to_blob(embedding);
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO embedding_cache (content_hash, model, embedding, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![content_hash, model, blob, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    /// Update the embedding for an existing fact. Writes through `VectorIndex`.
    /// Caller is responsible for L2-normalizing the vector.
    pub fn update_fact_embedding(&self, fact_id: &str, embedding: &[f32]) -> Result<(), String> {
        self.vec_index.upsert(fact_id, embedding)
    }

    /// Fetch the stored embedding for a fact, if present in `memory_facts_index`.
    /// Returns `None` if the fact has never been indexed.
    ///
    /// `sqlite-vec` stores vectors as `FLOAT[N]` BLOBs (little-endian f32s);
    /// we decode the raw bytes back to `Vec<f32>`.
    pub fn get_fact_embedding(&self, fact_id: &str) -> Result<Option<Vec<f32>>, String> {
        self.db.with_connection(|conn| {
            let r = conn.query_row(
                "SELECT embedding FROM memory_facts_index WHERE fact_id = ?1",
                params![fact_id],
                |row| row.get::<_, Vec<u8>>(0),
            );
            match r {
                Ok(blob) => Ok(Some(blob_to_f32_vec(&blob))),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Get high-confidence facts that should always be included in recall.
    ///
    /// When `agent_id` is `Some(a)`, results include the agent's private facts
    /// (`scope='agent'`) plus any `scope='global'` facts. When `None`, no
    /// agent/scope filter is applied (admin/debug path).
    pub fn get_high_confidence_facts(
        &self,
        agent_id: Option<&str>,
        min_confidence: f64,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        let now = Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(a) = agent_id {
                    (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts mf
                             WHERE ((mf.agent_id = ?1 AND mf.scope = 'agent') OR mf.scope = 'global')
                               AND confidence >= ?2
                               AND (expires_at IS NULL OR expires_at > ?4)
                             ORDER BY confidence DESC, mention_count DESC
                             LIMIT ?3"
                        ),
                        vec![
                            Box::new(a.to_string()),
                            Box::new(min_confidence),
                            Box::new(limit as i64),
                            Box::new(now.clone()),
                        ],
                    )
                } else {
                    (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE confidence >= ?1
                               AND (expires_at IS NULL OR expires_at > ?3)
                             ORDER BY confidence DESC, mention_count DESC
                             LIMIT ?2"
                        ),
                        vec![
                            Box::new(min_confidence),
                            Box::new(limit as i64),
                            Box::new(now.clone()),
                        ],
                    )
                };
            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Get top facts for a specific category, ordered by confidence then recency.
    ///
    /// Used to always-inject corrections into recall (regardless of query
    /// similarity) and for capability gap detection (skill/agent categories).
    pub fn get_facts_by_category(
        &self,
        agent_id: &str,
        category: &str,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let sql = format!(
                "SELECT {FACT_COLUMNS}
                 FROM memory_facts
                 WHERE agent_id = ?1 AND category = ?2
                   AND (expires_at IS NULL OR expires_at > ?4)
                 ORDER BY confidence DESC, updated_at DESC
                 LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![agent_id, category, limit as i64, Utc::now().to_rfc3339()],
                row_to_memory_fact,
            )?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Count total memory facts for an agent.
    pub fn count_memory_facts(&self, agent_id: &str) -> Result<usize, String> {
        self.db.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory_facts WHERE agent_id = ?1",
                params![agent_id],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
    }

    /// List all memory facts across all agents with optional filters and pagination.
    pub fn list_all_memory_facts(
        &self,
        agent_id: Option<&str>,
        category: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.db.with_connection(|conn| {
            let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                match (agent_id, category, scope) {
                    (Some(aid), Some(cat), Some(scp)) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1 AND category = ?2 AND scope = ?3
                             ORDER BY updated_at DESC
                             LIMIT ?4 OFFSET ?5"
                        ),
                        vec![
                            Box::new(aid.to_string()),
                            Box::new(cat.to_string()),
                            Box::new(scp.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (Some(aid), Some(cat), None) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1 AND category = ?2
                             ORDER BY updated_at DESC
                             LIMIT ?3 OFFSET ?4"
                        ),
                        vec![
                            Box::new(aid.to_string()),
                            Box::new(cat.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (Some(aid), None, Some(scp)) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1 AND scope = ?2
                             ORDER BY updated_at DESC
                             LIMIT ?3 OFFSET ?4"
                        ),
                        vec![
                            Box::new(aid.to_string()),
                            Box::new(scp.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (Some(aid), None, None) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE agent_id = ?1
                             ORDER BY updated_at DESC
                             LIMIT ?2 OFFSET ?3"
                        ),
                        vec![
                            Box::new(aid.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (None, Some(cat), Some(scp)) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE category = ?1 AND scope = ?2
                             ORDER BY updated_at DESC
                             LIMIT ?3 OFFSET ?4"
                        ),
                        vec![
                            Box::new(cat.to_string()),
                            Box::new(scp.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (None, Some(cat), None) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE category = ?1
                             ORDER BY updated_at DESC
                             LIMIT ?2 OFFSET ?3"
                        ),
                        vec![
                            Box::new(cat.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (None, None, Some(scp)) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             WHERE scope = ?1
                             ORDER BY updated_at DESC
                             LIMIT ?2 OFFSET ?3"
                        ),
                        vec![
                            Box::new(scp.to_string()),
                            Box::new(limit as i64),
                            Box::new(offset as i64),
                        ],
                    ),
                    (None, None, None) => (
                        format!(
                            "SELECT {FACT_COLUMNS}
                             FROM memory_facts
                             ORDER BY updated_at DESC
                             LIMIT ?1 OFFSET ?2"
                        ),
                        vec![Box::new(limit as i64), Box::new(offset as i64)],
                    ),
                };

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), row_to_memory_fact)?;
            rows.collect::<Result<Vec<_>, _>>()
        })
    }

    /// Count all memory facts across all agents.
    pub fn count_all_memory_facts(&self, agent_id: Option<&str>) -> Result<usize, String> {
        self.db.with_connection(|conn| {
            let count: i64 = if let Some(aid) = agent_id {
                conn.query_row(
                    "SELECT COUNT(*) FROM memory_facts WHERE agent_id = ?1",
                    params![aid],
                    |row| row.get(0),
                )?
            } else {
                conn.query_row("SELECT COUNT(*) FROM memory_facts", [], |row| row.get(0))?
            };
            Ok(count as usize)
        })
    }
}

// ============================================================================
// HELPERS
// ============================================================================

/// Map a database row to a `MemoryFact`. Columns must match `FACT_COLUMNS`
/// (no `embedding` — that lives in `memory_facts_index`).
fn row_to_memory_fact(row: &rusqlite::Row) -> Result<MemoryFact, rusqlite::Error> {
    Ok(MemoryFact {
        id: row.get(0)?,
        session_id: row.get(1)?,
        agent_id: row.get(2)?,
        scope: row.get(3)?,
        category: row.get(4)?,
        key: row.get(5)?,
        content: row.get(6)?,
        confidence: row.get(7)?,
        mention_count: row.get(8)?,
        source_summary: row.get(9)?,
        embedding: None,
        ward_id: row.get(10)?,
        contradicted_by: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        expires_at: row.get(14)?,
        valid_from: row.get(15).unwrap_or(None),
        valid_until: row.get(16).unwrap_or(None),
        superseded_by: row.get(17).unwrap_or(None),
        pinned: row.get::<_, i32>(18).unwrap_or(0) != 0,
        epistemic_class: row.get(19).ok().flatten(),
        source_episode_id: row.get(20).ok().flatten(),
        source_ref: row.get(21).ok().flatten(),
    })
}

/// Convert f32 vector to raw bytes (little-endian) for SQLite BLOB storage.
/// Used by the `embedding_cache` table (content-hash-keyed cache), not by
/// `memory_facts_index` (which stores JSON via `sqlite-vec`).
fn f32_vec_to_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Convert raw bytes (little-endian) back to f32 vector.
pub fn blob_to_f32_vec(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Cosine similarity between two L2-normalised embedding vectors.
///
/// For unit-length vectors, cosine similarity equals the dot product and is
/// equivalent to `1 - L2_sq / 2` (the same formula used by
/// `search_memory_facts_vector` when converting sqlite-vec distances).
/// Returns a value in `[-1.0, 1.0]`.
pub fn cosine_similarity_normalized(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    dot as f64
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_index::SqliteVecIndex;

    fn setup() -> (tempfile::TempDir, MemoryRepository) {
        use gateway_services::VaultPaths;

        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let vec_index: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id")
                .expect("vec index init"),
        );
        let repo = MemoryRepository::new(db, vec_index);
        (tmp, repo)
    }

    fn make_fact(agent_id: &str, key: &str, content: &str, category: &str) -> MemoryFact {
        MemoryFact {
            id: format!("fact-{}", uuid::Uuid::new_v4()),
            session_id: None,
            agent_id: agent_id.to_string(),
            scope: "agent".to_string(),
            category: category.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            confidence: 0.8,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "__global__".to_string(),
            contradicted_by: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
        }
    }

    /// Build an L2-normalized 384-dim vector whose first three components
    /// point roughly in the direction of `(x, y, z)` — useful for tests that
    /// want a deterministic "this fact matches that query" relation without
    /// caring about the full 384-D geometry.
    fn normalized_384(x: f32, y: f32, z: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = x;
        v[1] = y;
        v[2] = z;
        let norm = v.iter().map(|f| f * f).sum::<f32>().sqrt();
        if norm > 0.0 {
            for f in &mut v {
                *f /= norm;
            }
        }
        v
    }

    #[test]
    fn test_upsert_and_get_fact() {
        let (_tmp, repo) = setup();

        let fact = make_fact("agent-1", "user.name", "User's name is Alice", "preference");
        repo.upsert_memory_fact(&fact).expect("upsert");

        let facts = repo.get_memory_facts("agent-1", None, 10).expect("get");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "User's name is Alice");
        assert_eq!(facts[0].key, "user.name");
    }

    #[test]
    fn test_upsert_dedup_bumps_mention_count() {
        let (_tmp, repo) = setup();

        let fact1 = make_fact("agent-1", "lang.preferred", "Python", "preference");
        repo.upsert_memory_fact(&fact1).expect("upsert 1");

        // Same key, different content — should update
        let mut fact2 = make_fact("agent-1", "lang.preferred", "Rust", "preference");
        fact2.confidence = 0.9;
        repo.upsert_memory_fact(&fact2).expect("upsert 2");

        let facts = repo.get_memory_facts("agent-1", None, 10).expect("get");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "Rust");
        assert_eq!(facts[0].mention_count, 2);
        assert_eq!(facts[0].confidence, 0.9); // MAX of 0.8 and 0.9
    }

    #[test]
    fn test_delete_fact() {
        let (_tmp, repo) = setup();

        let fact = make_fact("agent-1", "test.delete", "will be deleted", "entity");
        repo.upsert_memory_fact(&fact).expect("upsert");

        assert!(repo.delete_memory_fact(&fact.id).expect("delete"));
        assert_eq!(repo.count_memory_facts("agent-1").expect("count"), 0);
    }

    #[test]
    fn test_scope_isolation() {
        let (_tmp, repo) = setup();

        let mut fact1 = make_fact("agent-1", "shared.key", "shared fact", "entity");
        fact1.scope = "shared".to_string();
        repo.upsert_memory_fact(&fact1).expect("upsert 1");

        let fact2 = make_fact("agent-1", "agent.key", "agent fact", "entity");
        repo.upsert_memory_fact(&fact2).expect("upsert 2");

        let shared = repo
            .get_memory_facts("agent-1", Some("shared"), 10)
            .expect("get shared");
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].key, "shared.key");

        let agent = repo
            .get_memory_facts("agent-1", Some("agent"), 10)
            .expect("get agent");
        assert_eq!(agent.len(), 1);
        assert_eq!(agent[0].key, "agent.key");
    }

    #[test]
    fn test_fts5_search() {
        let (_tmp, repo) = setup();

        repo.upsert_memory_fact(&make_fact(
            "agent-1",
            "build.tool",
            "Uses cargo for building Rust projects",
            "pattern",
        ))
        .expect("upsert 1");
        repo.upsert_memory_fact(&make_fact(
            "agent-1",
            "editor.pref",
            "Prefers VS Code for editing",
            "preference",
        ))
        .expect("upsert 2");
        repo.upsert_memory_fact(&make_fact(
            "agent-1",
            "lang.main",
            "Primary language is Rust",
            "decision",
        ))
        .expect("upsert 3");

        let results = repo
            .search_memory_facts_fts("Rust", Some("agent-1"), 10, None, None)
            .expect("fts");
        assert!(
            results.len() >= 2,
            "Should find 'Rust' in at least 2 facts, got {}",
            results.len()
        );
    }

    #[test]
    fn test_embedding_storage_and_vector_search() {
        let (_tmp, repo) = setup();

        // Create facts with (normalized) embeddings pointing in different directions.
        let mut fact1 = make_fact("agent-1", "vec.test1", "hello world", "entity");
        fact1.embedding = Some(normalized_384(1.0, 0.0, 0.0));
        repo.upsert_memory_fact(&fact1).expect("upsert 1");

        let mut fact2 = make_fact("agent-1", "vec.test2", "goodbye world", "entity");
        fact2.embedding = Some(normalized_384(0.0, 1.0, 0.0));
        repo.upsert_memory_fact(&fact2).expect("upsert 2");

        // Query close to fact1.
        let query = normalized_384(0.9, 0.1, 0.0);
        let (results, sources) = repo
            .search_memory_facts_hybrid(
                "hello",
                Some(&query),
                Some("agent-1"),
                10,
                0.7,
                0.3,
                None,
                None,
            )
            .expect("hybrid");

        assert!(!results.is_empty(), "Should find at least one result");
        // fact1 should outrank fact2 because the query is closer to it.
        assert_eq!(results[0].fact.id, fact1.id);
        assert_eq!(sources.len(), results.len());
        assert!(sources.iter().any(|(id, _)| id == &fact1.id));

        // Round-trip check: stored embedding is retrievable.
        let stored = repo
            .get_fact_embedding(&fact1.id)
            .expect("get emb")
            .expect("some emb");
        assert!(!stored.is_empty(), "stored embedding should be non-empty");
    }

    #[test]
    fn test_embedding_cache() {
        let (_tmp, repo) = setup();

        let hash = "abc123";
        let model = "all-MiniLM-L6-v2";
        let embedding = vec![0.1_f32, 0.2, 0.3, 0.4];

        // Cache miss
        assert!(repo
            .get_cached_embedding(hash, model)
            .expect("get miss")
            .is_none());

        // Cache write
        repo.cache_embedding(hash, model, &embedding)
            .expect("cache write");

        // Cache hit
        let cached = repo
            .get_cached_embedding(hash, model)
            .expect("get hit")
            .expect("cache value");
        assert_eq!(cached.len(), 4);
        assert!((cached[0] - 0.1).abs() < 0.001);
        assert!((cached[3] - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_f32_blob_roundtrip() {
        #[allow(clippy::approx_constant)]
        let pi_approx = 3.14159_f32;
        let original = vec![1.5_f32, -2.5, 0.0, pi_approx];
        let blob = f32_vec_to_blob(&original);
        let recovered = blob_to_f32_vec(&blob);
        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 0.0001);
        }
    }

    #[test]
    fn test_sanitize_fts_query() {
        assert_eq!(sanitize_fts_query("hello world"), "hello OR world");
        assert_eq!(
            sanitize_fts_query("PTON, NVDA, TSLA"),
            "PTON OR NVDA OR TSLA"
        );
        assert_eq!(
            sanitize_fts_query("portfolio risk (VaR 95%)"),
            "portfolio OR risk OR VaR"
        );
        assert_eq!(sanitize_fts_query(""), "");
        assert_eq!(sanitize_fts_query("a b"), ""); // words <= 2 chars filtered
    }

    #[test]
    fn test_high_confidence_facts() {
        let (_tmp, repo) = setup();

        let mut high = make_fact(
            "agent-1",
            "important",
            "always remember this",
            "instruction",
        );
        high.confidence = 0.95;
        repo.upsert_memory_fact(&high).expect("upsert high");

        let mut low = make_fact("agent-1", "maybe", "might be useful", "pattern");
        low.confidence = 0.3;
        repo.upsert_memory_fact(&low).expect("upsert low");

        let facts = repo
            .get_high_confidence_facts(Some("agent-1"), 0.9, 10)
            .expect("high conf");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key, "important");
    }

    // ========================================================================
    // BI-TEMPORAL CONFLICT TRANSITION (Phase 4)
    // ========================================================================

    /// Helper: insert a fact at a specific RFC3339 created_at/updated_at and
    /// optionally a valid_from. The fact otherwise mirrors `make_fact`.
    fn make_fact_at(
        agent_id: &str,
        key: &str,
        content: &str,
        created_at: &str,
        valid_from: Option<&str>,
    ) -> MemoryFact {
        MemoryFact {
            id: format!("fact-{}", uuid::Uuid::new_v4()),
            session_id: None,
            agent_id: agent_id.to_string(),
            scope: "agent".to_string(),
            category: "schema".to_string(),
            key: key.to_string(),
            content: content.to_string(),
            confidence: 0.8,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "__global__".to_string(),
            contradicted_by: None,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
            expires_at: None,
            valid_from: valid_from.map(|s| s.to_string()),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
        }
    }

    /// Test A — `supersede_fact` respects the passed `transition_time` rather
    /// than always using `Utc::now()`. With path (b), ConflictResolver passes
    /// the winner's `created_at`; the loser's `valid_until` must match exactly.
    #[test]
    fn supersede_fact_respects_transition_time() {
        let (_tmp, repo) = setup();

        let a = make_fact_at(
            "agent-bt",
            "policy.rebase",
            "Always rebase",
            "2026-01-01T00:00:00+00:00",
            Some("2026-01-01T00:00:00+00:00"),
        );
        let b = make_fact_at(
            "agent-bt",
            "policy.rebase.b",
            "Never rebase",
            "2026-02-01T00:00:00+00:00",
            Some("2026-02-01T00:00:00+00:00"),
        );
        repo.upsert_memory_fact(&a).expect("upsert a");
        repo.upsert_memory_fact(&b).expect("upsert b");

        let b_created_at = DateTime::parse_from_rfc3339(&b.created_at)
            .expect("parse b.created_at")
            .with_timezone(&Utc);

        repo.supersede_fact(&a.id, &b.id, b_created_at)
            .expect("supersede");

        let loaded_a = repo
            .get_memory_fact_by_id(&a.id)
            .expect("load a")
            .expect("a present");
        assert_eq!(
            loaded_a.superseded_by.as_deref(),
            Some(b.id.as_str()),
            "loser.superseded_by must point at winner"
        );
        let valid_until = loaded_a.valid_until.expect("valid_until set");
        let parsed = DateTime::parse_from_rfc3339(&valid_until)
            .expect("valid_until parses")
            .with_timezone(&Utc);
        assert_eq!(
            parsed, b_created_at,
            "loser.valid_until must equal winner.created_at (bi-temporal transition)"
        );
    }

    /// Test B — point-in-time recall returns the correct fact across the
    /// transition. Before the transition: only A is valid. After: only B.
    /// No overlap interval exists.
    #[test]
    fn point_in_time_recall_across_conflict_transition() {
        let (_tmp, repo) = setup();

        // Different keys so the upsert ON CONFLICT clause doesn't merge them.
        let a = make_fact_at(
            "agent-pit",
            "policy.a",
            "Belief A",
            "2026-01-01T00:00:00+00:00",
            Some("2026-01-01T00:00:00+00:00"),
        );
        let b = make_fact_at(
            "agent-pit",
            "policy.b",
            "Belief B",
            "2026-02-01T00:00:00+00:00",
            Some("2026-02-01T00:00:00+00:00"),
        );
        repo.upsert_memory_fact(&a).expect("upsert a");
        repo.upsert_memory_fact(&b).expect("upsert b");

        let b_created_at = DateTime::parse_from_rfc3339(&b.created_at)
            .unwrap()
            .with_timezone(&Utc);
        repo.supersede_fact(&a.id, &b.id, b_created_at)
            .expect("supersede");

        // Query helper: returns the set of fact ids visible at `as_of` by
        // applying the same bi-temporal predicate the recall paths use.
        let db = repo.db_for_tests();
        let visible_at = |as_of: DateTime<Utc>| -> Vec<String> {
            let ts = as_of.to_rfc3339();
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM memory_facts \
                     WHERE agent_id = ?1 \
                     AND (valid_from IS NULL OR valid_from <= ?2) \
                     AND (valid_until IS NULL OR valid_until > ?2)",
                )?;
                let rows =
                    stmt.query_map(params!["agent-pit", ts], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()
            })
            .expect("query")
        };

        // Mid-January: only A is valid.
        let mid_jan = DateTime::parse_from_rfc3339("2026-01-15T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let jan = visible_at(mid_jan);
        assert!(jan.contains(&a.id), "A must be visible mid-January");
        assert!(!jan.contains(&b.id), "B must NOT be visible mid-January");

        // Mid-February: only B is valid.
        let mid_feb = DateTime::parse_from_rfc3339("2026-02-15T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let feb = visible_at(mid_feb);
        assert!(feb.contains(&b.id), "B must be visible mid-February");
        assert!(!feb.contains(&a.id), "A must NOT be visible mid-February");

        // Exactly at the transition (B.created_at): A.valid_until = B.created_at
        // so the half-open interval `valid_until > as_of` excludes A; B is
        // already valid (valid_from = B.created_at). No overlap.
        let at_transition = b_created_at;
        let transition = visible_at(at_transition);
        assert!(
            !transition.contains(&a.id),
            "A must NOT be visible at the transition instant (valid_until is exclusive)"
        );
        assert!(
            transition.contains(&b.id),
            "B must be visible at the transition instant (valid_from is inclusive)"
        );
    }

    /// Test C — backward compatibility: the distillation-style call with
    /// `transition_time = Utc::now()` still produces the pre-Phase-4 semantics
    /// (loser's `valid_until` is approximately "now").
    #[test]
    fn supersede_fact_with_now_preserves_default_semantics() {
        let (_tmp, repo) = setup();

        let a = make_fact("agent-now", "k.a", "A", "schema");
        let b = make_fact("agent-now", "k.b", "B", "schema");
        repo.upsert_memory_fact(&a).expect("upsert a");
        repo.upsert_memory_fact(&b).expect("upsert b");

        let before = Utc::now();
        repo.supersede_fact(&a.id, &b.id, Utc::now())
            .expect("supersede");
        let after = Utc::now();

        let loaded_a = repo
            .get_memory_fact_by_id(&a.id)
            .expect("load a")
            .expect("a present");
        assert_eq!(loaded_a.superseded_by.as_deref(), Some(b.id.as_str()));
        let valid_until = loaded_a.valid_until.expect("valid_until set");
        let parsed = DateTime::parse_from_rfc3339(&valid_until)
            .expect("valid_until parses")
            .with_timezone(&Utc);
        assert!(
            parsed >= before && parsed <= after,
            "valid_until must fall within the call window when transition_time = Utc::now()"
        );
    }
}
