// ============================================================================
// SQLITE BELIEF STORE
// Implements BeliefStore trait against the v27 kg_beliefs table.
// ============================================================================
//
// A belief row carries its bi-temporal interval (`valid_from` /
// `valid_until`) as RFC3339 strings — matching the convention used by
// `memory_facts`. The `source_fact_ids` column holds a JSON-encoded
// `Vec<String>`; callers see a typed `Vec<String>` via `Belief`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use zbot_stores_domain::{Belief, ScoredBelief};
use zbot_stores_traits::BeliefStore;

use crate::KnowledgeDatabase;

/// SQLite-backed `BeliefStore`. Delegates every method to a single
/// `KnowledgeDatabase` pool; each call runs against one short-lived
/// connection so calls compose cleanly with the rest of the memory
/// subsystem.
pub struct SqliteBeliefStore {
    db: Arc<KnowledgeDatabase>,
}

impl SqliteBeliefStore {
    pub fn new(db: Arc<KnowledgeDatabase>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl BeliefStore for SqliteBeliefStore {
    async fn get_belief(
        &self,
        partition_id: &str,
        subject: &str,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Option<Belief>, String> {
        let cutoff = as_of.unwrap_or_else(Utc::now).to_rfc3339();
        let partition_id = partition_id.to_string();
        let subject = subject.to_string();
        self.db
            .with_connection(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, partition_id, subject, content, confidence,
                        valid_from, valid_until, source_fact_ids,
                        synthesizer_version, reasoning, created_at,
                        updated_at, superseded_by, stale, embedding
                 FROM kg_beliefs
                 WHERE partition_id = ?1
                   AND subject = ?2
                   AND (valid_from IS NULL OR valid_from <= ?3)
                   AND (valid_until IS NULL OR valid_until > ?3)
                 ORDER BY COALESCE(valid_from, '') DESC
                 LIMIT 1",
                )?;
                stmt.query_row(params![partition_id, subject, cutoff], row_to_belief)
                    .optional()
            })?
            .transpose()
    }

    async fn list_beliefs(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String> {
        let partition_id = partition_id.to_string();
        // i64 cast is safe — `limit` originates from a usize controlled
        // by the caller and is bounded by realistic UI / API caps.
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, partition_id, subject, content, confidence,
                        valid_from, valid_until, source_fact_ids,
                        synthesizer_version, reasoning, created_at,
                        updated_at, superseded_by, stale, embedding
                 FROM kg_beliefs
                 WHERE partition_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![partition_id, limit_i64], row_to_belief)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        // Each row yields a Result<Belief, String> due to JSON decode in
        // row_to_belief; flatten now.
        rows.into_iter().collect()
    }

    async fn upsert_belief(&self, belief: &Belief) -> Result<(), String> {
        let source_fact_ids_json = serde_json::to_string(&belief.source_fact_ids)
            .map_err(|e| format!("encode source_fact_ids: {e}"))?;
        let valid_from = belief.valid_from.map(|t| t.to_rfc3339());
        let valid_until = belief.valid_until.map(|t| t.to_rfc3339());
        let created_at = belief.created_at.to_rfc3339();
        let updated_at = belief.updated_at.to_rfc3339();

        let id = belief.id.clone();
        let partition_id = belief.partition_id.clone();
        let subject = belief.subject.clone();
        let content = belief.content.clone();
        let confidence = belief.confidence;
        let synthesizer_version = belief.synthesizer_version;
        let reasoning = belief.reasoning.clone();
        let superseded_by = belief.superseded_by.clone();
        let stale = i32::from(belief.stale);
        let embedding = belief.embedding.clone();

        self.db.with_connection(move |conn| {
            conn.execute(
                "INSERT INTO kg_beliefs (
                    id, partition_id, subject, content, confidence,
                    valid_from, valid_until, source_fact_ids,
                    synthesizer_version, reasoning, created_at, updated_at,
                    superseded_by, stale, embedding
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
                )
                ON CONFLICT(partition_id, subject, valid_from) DO UPDATE SET
                    content = excluded.content,
                    confidence = excluded.confidence,
                    valid_until = excluded.valid_until,
                    source_fact_ids = excluded.source_fact_ids,
                    synthesizer_version = excluded.synthesizer_version,
                    reasoning = excluded.reasoning,
                    updated_at = excluded.updated_at,
                    superseded_by = excluded.superseded_by,
                    stale = excluded.stale,
                    embedding = excluded.embedding",
                params![
                    id,
                    partition_id,
                    subject,
                    content,
                    confidence,
                    valid_from,
                    valid_until,
                    source_fact_ids_json,
                    synthesizer_version,
                    reasoning,
                    created_at,
                    updated_at,
                    superseded_by,
                    stale,
                    embedding,
                ],
            )?;
            Ok(())
        })
    }

    async fn supersede_belief(
        &self,
        old_id: &str,
        new_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String> {
        let old_id = old_id.to_string();
        let new_id = new_id.to_string();
        let ts = transition_time.to_rfc3339();
        self.db.with_connection(move |conn| {
            conn.execute(
                "UPDATE kg_beliefs
                 SET valid_until = ?1,
                     superseded_by = ?2,
                     updated_at = ?1
                 WHERE id = ?3",
                params![ts, new_id, old_id],
            )?;
            Ok(())
        })
    }

    async fn mark_stale(&self, belief_id: &str) -> Result<(), String> {
        let id = belief_id.to_string();
        let now = Utc::now().to_rfc3339();
        self.db.with_connection(move |conn| {
            conn.execute(
                "UPDATE kg_beliefs SET stale = 1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    async fn retract_belief(
        &self,
        belief_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String> {
        let id = belief_id.to_string();
        let ts = transition_time.to_rfc3339();
        self.db.with_connection(move |conn| {
            conn.execute(
                "UPDATE kg_beliefs SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                params![ts, id],
            )?;
            Ok(())
        })
    }

    async fn get_belief_by_id(&self, belief_id: &str) -> Result<Option<Belief>, String> {
        let belief_id = belief_id.to_string();
        let row = self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, partition_id, subject, content, confidence,
                        valid_from, valid_until, source_fact_ids,
                        synthesizer_version, reasoning, created_at,
                        updated_at, superseded_by, stale, embedding
                 FROM kg_beliefs
                 WHERE id = ?1
                 LIMIT 1",
            )?;
            stmt.query_row(params![belief_id], row_to_belief).optional()
        })?;
        match row {
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    async fn beliefs_referencing_fact(&self, fact_id: &str) -> Result<Vec<String>, String> {
        let fact_id = fact_id.to_string();
        self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT b.id FROM kg_beliefs b
                 WHERE b.valid_until IS NULL
                   AND EXISTS (
                       SELECT 1 FROM json_each(b.source_fact_ids)
                       WHERE json_each.value = ?1
                   )",
            )?;
            let ids: Vec<String> = stmt
                .query_map(params![fact_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })
    }

    async fn list_stale(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String> {
        let partition_id = partition_id.to_string();
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, partition_id, subject, content, confidence,
                        valid_from, valid_until, source_fact_ids,
                        synthesizer_version, reasoning, created_at,
                        updated_at, superseded_by, stale, embedding
                 FROM kg_beliefs
                 WHERE partition_id = ?1 AND stale = 1
                 ORDER BY updated_at ASC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![partition_id, limit_i64], row_to_belief)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.into_iter().collect()
    }

    async fn clear_stale(&self, belief_id: &str) -> Result<(), String> {
        let id = belief_id.to_string();
        let now = Utc::now().to_rfc3339();
        self.db.with_connection(move |conn| {
            conn.execute(
                "UPDATE kg_beliefs SET stale = 0, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    async fn search_beliefs(
        &self,
        partition_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<ScoredBelief>, String> {
        // B-4 in-memory cosine: belief count is bounded (real-data has
        // ~15 multi-fact subjects, even at 100x growth ~1k beliefs).
        // A separate vec0 table would add maintenance cost for no
        // measurable benefit at this scale. We SELECT all live beliefs
        // in the partition, score them in this thread, sort, and
        // truncate.
        if query_embedding.is_empty() {
            return Ok(Vec::new());
        }
        let partition_id = partition_id.to_string();
        let now_str = Utc::now().to_rfc3339();
        let rows = self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, partition_id, subject, content, confidence,
                        valid_from, valid_until, source_fact_ids,
                        synthesizer_version, reasoning, created_at,
                        updated_at, superseded_by, stale, embedding
                 FROM kg_beliefs
                 WHERE partition_id = ?1
                   AND superseded_by IS NULL
                   AND (valid_from IS NULL OR valid_from <= ?2)
                   AND (valid_until IS NULL OR valid_until > ?2)",
            )?;
            let rows = stmt
                .query_map(params![partition_id, now_str], row_to_belief)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        // Flatten Result<Belief, String> per row before scoring.
        let beliefs: Vec<Belief> = rows.into_iter().collect::<Result<Vec<_>, _>>()?;

        let mut scored: Vec<ScoredBelief> = beliefs
            .into_iter()
            .filter_map(|b| {
                let bytes = b.embedding.as_deref()?;
                let emb = embedding_from_bytes(bytes)?;
                let score = cosine_similarity_f64(query_embedding, &emb);
                Some(ScoredBelief { belief: b, score })
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
}

/// Map one row of `kg_beliefs` to a `Belief`. The `source_fact_ids` JSON
/// decode is fallible — return a `Result<Belief, String>` per row so the
/// caller can fail loud rather than silently dropping rows.
fn row_to_belief(row: &rusqlite::Row) -> rusqlite::Result<Result<Belief, String>> {
    let id: String = row.get(0)?;
    let partition_id: String = row.get(1)?;
    let subject: String = row.get(2)?;
    let content: String = row.get(3)?;
    let confidence: f64 = row.get(4)?;
    let valid_from: Option<String> = row.get(5)?;
    let valid_until: Option<String> = row.get(6)?;
    let source_fact_ids_json: String = row.get(7)?;
    let synthesizer_version: i32 = row.get(8)?;
    let reasoning: Option<String> = row.get(9)?;
    let created_at: String = row.get(10)?;
    let updated_at: String = row.get(11)?;
    let superseded_by: Option<String> = row.get(12)?;
    let stale_int: i32 = row.get(13)?;
    let stale = stale_int != 0;
    let embedding: Option<Vec<u8>> = row.get(14)?;

    let source_fact_ids: Vec<String> = match serde_json::from_str(&source_fact_ids_json) {
        Ok(v) => v,
        Err(e) => return Ok(Err(format!("decode source_fact_ids for {id}: {e}"))),
    };

    let parse_dt = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| format!("parse timestamp {s}: {e}"))
    };

    let valid_from = match valid_from.as_deref().map(parse_dt).transpose() {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let valid_until = match valid_until.as_deref().map(parse_dt).transpose() {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let created_at = match parse_dt(&created_at) {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let updated_at = match parse_dt(&updated_at) {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };

    Ok(Ok(Belief {
        id,
        partition_id,
        subject,
        content,
        confidence,
        valid_from,
        valid_until,
        source_fact_ids,
        synthesizer_version,
        reasoning,
        created_at,
        updated_at,
        superseded_by,
        stale,
        embedding,
    }))
}

/// Decode a `Vec<u8>` of little-endian f32 bytes into a `Vec<f32>`. The
/// caller is responsible for handling NULL columns before invoking; an
/// empty or non-multiple-of-4 byte slice yields `None` (treat as
/// "embedding not available").
fn embedding_from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        // chunks_exact yields slices of exactly 4 — the unwrap is safe.
        let arr: [u8; 4] = chunk.try_into().ok()?;
        out.push(f32::from_le_bytes(arr));
    }
    Some(out)
}

/// Cosine similarity in `f64` precision between two equal-length
/// embeddings. Returns `0.0` for empty / mismatched / zero-magnitude
/// inputs so search ranks them last rather than panicking.
fn cosine_similarity_f64(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeDatabase;
    use gateway_services::VaultPaths;
    use tempfile::TempDir;

    fn make_store() -> (SqliteBeliefStore, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        (SqliteBeliefStore::new(db), tmp)
    }

    fn sample_belief(id: &str, subject: &str, valid_from: Option<DateTime<Utc>>) -> Belief {
        let now = Utc::now();
        Belief {
            id: id.to_string(),
            partition_id: "default".to_string(),
            subject: subject.to_string(),
            content: format!("{} content", subject),
            confidence: 0.8,
            valid_from,
            valid_until: None,
            source_fact_ids: vec!["fact-1".to_string()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        }
    }

    #[tokio::test]
    async fn upsert_and_get_round_trip() {
        let (store, _tmp) = make_store();
        let b = sample_belief("b1", "user.name", Some(Utc::now()));
        store.upsert_belief(&b).await.unwrap();

        let got = store
            .get_belief("default", "user.name", None)
            .await
            .unwrap()
            .expect("belief present");
        assert_eq!(got.id, "b1");
        assert_eq!(got.subject, "user.name");
        assert_eq!(got.source_fact_ids, vec!["fact-1".to_string()]);
    }

    #[tokio::test]
    async fn upsert_is_idempotent_on_unique_key() {
        let (store, _tmp) = make_store();
        let vf = Utc::now();
        let mut b = sample_belief("b1", "user.name", Some(vf));
        store.upsert_belief(&b).await.unwrap();

        // Upsert again with same (partition, subject, valid_from) but
        // different id/content — the unique constraint triggers UPDATE
        // and the row count stays at 1.
        b.id = "b1-v2".to_string();
        b.content = "second pass".to_string();
        store.upsert_belief(&b).await.unwrap();

        let listed = store.list_beliefs("default", 10).await.unwrap();
        assert_eq!(listed.len(), 1, "no duplicate rows on conflict");
        assert_eq!(listed[0].content, "second pass");
    }

    #[tokio::test]
    async fn get_belief_with_as_of_returns_historical_slice() {
        let (store, _tmp) = make_store();
        let old_vf = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let old_vu = DateTime::parse_from_rfc3339("2026-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let new_vf = DateTime::parse_from_rfc3339("2026-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut old = sample_belief("b-old", "user.employment", Some(old_vf));
        old.valid_until = Some(old_vu);
        old.content = "Anthropic".to_string();
        old.superseded_by = Some("b-new".to_string());
        let mut new_b = sample_belief("b-new", "user.employment", Some(new_vf));
        new_b.content = "OpenAI".to_string();

        store.upsert_belief(&old).await.unwrap();
        store.upsert_belief(&new_b).await.unwrap();

        // Default → "now" → returns the active belief.
        let cur = store
            .get_belief("default", "user.employment", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cur.content, "OpenAI");

        // as_of in the old window → returns the historical belief.
        let past = DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let hist = store
            .get_belief("default", "user.employment", Some(past))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hist.content, "Anthropic");
    }

    #[tokio::test]
    async fn supersede_belief_sets_valid_until_and_pointer() {
        let (store, _tmp) = make_store();
        let vf = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let b = sample_belief("b-old", "user.x", Some(vf));
        store.upsert_belief(&b).await.unwrap();

        let transition = DateTime::parse_from_rfc3339("2026-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store
            .supersede_belief("b-old", "b-new", transition)
            .await
            .unwrap();

        let listed = store.list_beliefs("default", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].valid_until, Some(transition));
        assert_eq!(listed[0].superseded_by.as_deref(), Some("b-new"));
    }

    #[tokio::test]
    async fn missing_belief_returns_none() {
        let (store, _tmp) = make_store();
        let got = store
            .get_belief("default", "no.such.subject", None)
            .await
            .unwrap();
        assert!(got.is_none());
    }

    // -----------------------------------------------------------------
    // B-3: stale / retract / clear / referencing-fact / list-stale
    // -----------------------------------------------------------------

    /// `mark_stale` flips `stale` from false to true; round-trips through
    /// `get_belief` / `list_beliefs`.
    #[tokio::test]
    async fn mark_stale_sets_the_flag() {
        let (store, _tmp) = make_store();
        let b = sample_belief("b-stale", "user.x", Some(Utc::now()));
        store.upsert_belief(&b).await.unwrap();

        let initial = store.list_beliefs("default", 10).await.unwrap();
        assert!(!initial[0].stale, "precondition: belief starts not stale");

        store.mark_stale("b-stale").await.unwrap();

        let after = store.list_beliefs("default", 10).await.unwrap();
        assert!(after[0].stale, "mark_stale must set stale=true");
    }

    /// `retract_belief` sets `valid_until` to the transition time without
    /// touching `superseded_by` (that field is reserved for the
    /// `supersede_belief` path). B-3 sole-source propagation uses this.
    #[tokio::test]
    async fn retract_belief_sets_valid_until() {
        let (store, _tmp) = make_store();
        let b = sample_belief("b-retract", "user.x", Some(Utc::now()));
        store.upsert_belief(&b).await.unwrap();

        let transition = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.retract_belief("b-retract", transition).await.unwrap();

        let listed = store.list_beliefs("default", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].valid_until, Some(transition));
        assert!(
            listed[0].superseded_by.is_none(),
            "retract is distinct from supersede: no replacement id set"
        );
    }

    /// `beliefs_referencing_fact` finds beliefs whose JSON array of source
    /// fact ids contains the queried fact. Excludes already-retracted
    /// beliefs (`valid_until IS NOT NULL`).
    #[tokio::test]
    async fn beliefs_referencing_fact_finds_via_json_array() {
        let (store, _tmp) = make_store();
        let now = Utc::now();
        let mut multi = sample_belief("b-multi", "user.x", Some(now));
        multi.source_fact_ids = vec!["F1".into(), "F2".into()];
        store.upsert_belief(&multi).await.unwrap();

        let mut other = sample_belief("b-other", "user.y", Some(now));
        other.source_fact_ids = vec!["F3".into()];
        store.upsert_belief(&other).await.unwrap();

        let hits_f1 = store.beliefs_referencing_fact("F1").await.unwrap();
        assert_eq!(hits_f1, vec!["b-multi".to_string()]);

        let hits_f2 = store.beliefs_referencing_fact("F2").await.unwrap();
        assert_eq!(hits_f2, vec!["b-multi".to_string()]);

        let hits_f3 = store.beliefs_referencing_fact("F3").await.unwrap();
        assert_eq!(hits_f3, vec!["b-other".to_string()]);

        let none = store.beliefs_referencing_fact("FX").await.unwrap();
        assert!(none.is_empty());

        // Retracted beliefs must be excluded — propagation has already
        // closed them out.
        store.retract_belief("b-multi", Utc::now()).await.unwrap();
        let after_retract = store.beliefs_referencing_fact("F1").await.unwrap();
        assert!(
            after_retract.is_empty(),
            "retracted beliefs are excluded from referencing_fact"
        );
    }

    /// `list_stale` returns only stale beliefs in the requested partition.
    /// Verifies both the flag filter and partition isolation.
    #[tokio::test]
    async fn list_stale_returns_only_stale_in_partition() {
        let (store, _tmp) = make_store();
        let now = Utc::now();

        let mut s1 = sample_belief("b-s1", "user.x", Some(now));
        s1.partition_id = "p1".into();
        let mut s2 = sample_belief("b-s2", "user.y", Some(now));
        s2.partition_id = "p1".into();
        let mut fresh = sample_belief("b-fresh", "user.z", Some(now));
        fresh.partition_id = "p1".into();
        let mut other_partition = sample_belief("b-other", "user.x", Some(now));
        other_partition.partition_id = "p2".into();
        store.upsert_belief(&s1).await.unwrap();
        store.upsert_belief(&s2).await.unwrap();
        store.upsert_belief(&fresh).await.unwrap();
        store.upsert_belief(&other_partition).await.unwrap();

        store.mark_stale("b-s1").await.unwrap();
        store.mark_stale("b-s2").await.unwrap();
        store.mark_stale("b-other").await.unwrap();

        let listed = store.list_stale("p1", 10).await.unwrap();
        let ids: Vec<&str> = listed.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(listed.len(), 2, "exactly two stale beliefs in p1");
        assert!(ids.contains(&"b-s1"));
        assert!(ids.contains(&"b-s2"));
        assert!(!ids.contains(&"b-fresh"), "fresh belief excluded");
        assert!(
            !ids.contains(&"b-other"),
            "p2's stale belief excluded by partition filter"
        );
    }

    /// `clear_stale` resets the flag — the inverse of `mark_stale`. Used
    /// by the synthesizer after re-synthesis completes.
    #[tokio::test]
    async fn clear_stale_resets_the_flag() {
        let (store, _tmp) = make_store();
        let b = sample_belief("b-clear", "user.x", Some(Utc::now()));
        store.upsert_belief(&b).await.unwrap();
        store.mark_stale("b-clear").await.unwrap();
        assert!(store.list_beliefs("default", 10).await.unwrap()[0].stale);

        store.clear_stale("b-clear").await.unwrap();
        assert!(!store.list_beliefs("default", 10).await.unwrap()[0].stale);
    }

    // -----------------------------------------------------------------
    // B-4: embedding round-trip + search_beliefs cosine + filters
    // -----------------------------------------------------------------

    /// Helper — serialize `[f32]` to little-endian bytes for embedding
    /// storage. Mirrors the helper in BeliefSynthesizer; co-located
    /// here so tests don't depend on gateway-memory.
    fn to_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// Round-trip: a belief upserted with an embedding loads back with
    /// the exact same bytes. Verifies the BLOB column accepts non-NULL
    /// data and the SELECT reads it.
    #[tokio::test]
    async fn upsert_with_embedding_round_trips_the_bytes() {
        let (store, _tmp) = make_store();
        let mut b = sample_belief("b-emb", "user.x", Some(Utc::now()));
        let emb_f32 = vec![1.0_f32, 0.5, -0.25, 0.0];
        let emb_bytes = to_bytes(&emb_f32);
        b.embedding = Some(emb_bytes.clone());
        store.upsert_belief(&b).await.unwrap();

        let got = store
            .get_belief("default", "user.x", None)
            .await
            .unwrap()
            .expect("belief present");
        assert_eq!(
            got.embedding.as_deref(),
            Some(emb_bytes.as_slice()),
            "embedding round-trips exactly"
        );
    }

    /// `search_beliefs` sorts by cosine similarity descending. Seed
    /// three beliefs with known embeddings, query with a fourth, and
    /// assert the top result is the closest one.
    #[tokio::test]
    async fn search_beliefs_sorts_by_cosine_descending() {
        let (store, _tmp) = make_store();
        let now = Utc::now();

        // Distinct (subject, valid_from) keeps the upsert key unique.
        for (id, subject, vec) in [
            ("b-near", "s.near", vec![1.0_f32, 0.0, 0.0]),
            ("b-mid", "s.mid", vec![0.7_f32, 0.7, 0.0]),
            ("b-far", "s.far", vec![0.0_f32, 0.0, 1.0]),
        ] {
            let mut b = sample_belief(id, subject, Some(now));
            b.embedding = Some(to_bytes(&vec));
            store.upsert_belief(&b).await.unwrap();
        }
        // Query parallel to b-near's vector.
        let query = vec![1.0_f32, 0.0, 0.0];
        let scored = store.search_beliefs("default", &query, 10).await.unwrap();
        let ids: Vec<&str> = scored.iter().map(|s| s.belief.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["b-near", "b-mid", "b-far"],
            "ordering must follow cosine score"
        );
        assert!(scored[0].score > scored[1].score);
        assert!(scored[1].score > scored[2].score);
    }

    /// Superseded beliefs are excluded from `search_beliefs` so the
    /// agent never sees a stance that's been replaced.
    #[tokio::test]
    async fn search_beliefs_filters_out_superseded() {
        let (store, _tmp) = make_store();
        let now = Utc::now();
        let mut live = sample_belief("b-live", "s.live", Some(now));
        live.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        let mut dead = sample_belief("b-dead", "s.dead", Some(now));
        dead.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        dead.superseded_by = Some("b-live".to_string());
        store.upsert_belief(&live).await.unwrap();
        store.upsert_belief(&dead).await.unwrap();

        let scored = store
            .search_beliefs("default", &[1.0, 0.0, 0.0], 10)
            .await
            .unwrap();
        let ids: Vec<&str> = scored.iter().map(|s| s.belief.id.as_str()).collect();
        assert_eq!(ids, vec!["b-live"], "superseded belief must be excluded");
    }

    /// Beliefs whose interval is fully in the past (closed `valid_until`
    /// before "now") must not surface — they're historical context, not
    /// active stance.
    #[tokio::test]
    async fn search_beliefs_filters_out_past_intervals() {
        let (store, _tmp) = make_store();
        let two_years_ago = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let one_year_ago = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut past = sample_belief("b-past", "s.past", Some(two_years_ago));
        past.valid_until = Some(one_year_ago);
        past.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        let mut current = sample_belief("b-curr", "s.curr", Some(Utc::now()));
        current.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        store.upsert_belief(&past).await.unwrap();
        store.upsert_belief(&current).await.unwrap();

        let scored = store
            .search_beliefs("default", &[1.0, 0.0, 0.0], 10)
            .await
            .unwrap();
        let ids: Vec<&str> = scored.iter().map(|s| s.belief.id.as_str()).collect();
        assert_eq!(ids, vec!["b-curr"], "historical belief must be excluded");
    }

    /// Beliefs with NULL embedding can't be scored semantically and
    /// must be excluded — they're still queryable via `get_belief`.
    #[tokio::test]
    async fn search_beliefs_skips_null_embeddings() {
        let (store, _tmp) = make_store();
        let now = Utc::now();
        // One belief with embedding; one without.
        let mut with_emb = sample_belief("b-emb", "s.emb", Some(now));
        with_emb.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        let without_emb = sample_belief("b-noemb", "s.noemb", Some(now));
        // embedding stays None.
        store.upsert_belief(&with_emb).await.unwrap();
        store.upsert_belief(&without_emb).await.unwrap();

        let scored = store
            .search_beliefs("default", &[1.0, 0.0, 0.0], 10)
            .await
            .unwrap();
        let ids: Vec<&str> = scored.iter().map(|s| s.belief.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["b-emb"],
            "NULL-embedding belief excluded from semantic recall"
        );
        // But `get_belief` direct lookup still works for it.
        let direct = store
            .get_belief("default", "s.noemb", None)
            .await
            .unwrap()
            .expect("direct get returns the NULL-embedding belief");
        assert_eq!(direct.id, "b-noemb");
    }

    /// Partition isolation: a query against `p1` must never return
    /// beliefs stored in `p2`. Multi-tenant safety.
    #[tokio::test]
    async fn search_beliefs_respects_partition() {
        let (store, _tmp) = make_store();
        let now = Utc::now();
        let mut in_p1 = sample_belief("b-p1", "s.x", Some(now));
        in_p1.partition_id = "p1".into();
        in_p1.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        let mut in_p2 = sample_belief("b-p2", "s.x", Some(now));
        in_p2.partition_id = "p2".into();
        in_p2.embedding = Some(to_bytes(&[1.0, 0.0, 0.0]));
        store.upsert_belief(&in_p1).await.unwrap();
        store.upsert_belief(&in_p2).await.unwrap();

        let scored = store
            .search_beliefs("p1", &[1.0, 0.0, 0.0], 10)
            .await
            .unwrap();
        let ids: Vec<&str> = scored.iter().map(|s| s.belief.id.as_str()).collect();
        assert_eq!(ids, vec!["b-p1"]);
    }
}
