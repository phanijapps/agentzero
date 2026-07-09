// ============================================================================
// SQLITE BELIEF CONTRADICTION STORE
// Implements BeliefContradictionStore against the v28 kg_belief_contradictions
// table (Belief Network Phase B-2).
// ============================================================================
//
// Canonical pair ordering: every read/write path canonicalizes
// `(belief_a_id, belief_b_id)` so the lexicographically-smaller ID is on
// the `belief_a_id` column. Combined with `UNIQUE(belief_a_id,
// belief_b_id)` and `INSERT ... ON CONFLICT DO NOTHING`, this makes
// insert idempotent without an explicit pre-check race.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use zbot_stores_domain::{BeliefContradiction, ContradictionType, Resolution};
use zbot_stores_traits::BeliefContradictionStore;

use crate::KnowledgeDatabase;

/// SQLite-backed `BeliefContradictionStore`. Delegates every method to a
/// single `KnowledgeDatabase` pool; each call runs against one
/// short-lived connection so calls compose cleanly with the rest of the
/// memory subsystem.
pub struct SqliteBeliefContradictionStore {
    db: Arc<KnowledgeDatabase>,
}

impl SqliteBeliefContradictionStore {
    pub fn new(db: Arc<KnowledgeDatabase>) -> Self {
        Self { db }
    }
}

/// Canonicalize an unordered pair so the lexicographically smaller ID is
/// always on the left. Pure helper — pulled out so tests can exercise it.
pub(crate) fn canonical_pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn contradiction_type_to_str(t: &ContradictionType) -> &'static str {
    match t {
        ContradictionType::Logical => "logical",
        ContradictionType::Tension => "tension",
        ContradictionType::Temporal => "temporal",
    }
}

fn contradiction_type_from_str(s: &str) -> Result<ContradictionType, String> {
    match s {
        "logical" => Ok(ContradictionType::Logical),
        "tension" => Ok(ContradictionType::Tension),
        "temporal" => Ok(ContradictionType::Temporal),
        other => Err(format!("unknown contradiction_type: {other}")),
    }
}

fn resolution_to_str(r: &Resolution) -> &'static str {
    match r {
        Resolution::AWon => "a_won",
        Resolution::BWon => "b_won",
        Resolution::Compatible => "compatible",
        Resolution::Unresolved => "unresolved",
    }
}

fn resolution_from_str(s: &str) -> Result<Resolution, String> {
    match s {
        "a_won" => Ok(Resolution::AWon),
        "b_won" => Ok(Resolution::BWon),
        "compatible" => Ok(Resolution::Compatible),
        "unresolved" => Ok(Resolution::Unresolved),
        other => Err(format!("unknown resolution: {other}")),
    }
}

#[async_trait]
impl BeliefContradictionStore for SqliteBeliefContradictionStore {
    async fn insert_contradiction(&self, c: &BeliefContradiction) -> Result<(), String> {
        // Canonicalize the pair before insert so the unique index and all
        // future lookups stay aligned regardless of caller ordering.
        let (a, b) = canonical_pair(&c.belief_a_id, &c.belief_b_id);
        let id = c.id.clone();
        let belief_a_id = a.to_string();
        let belief_b_id = b.to_string();
        let contradiction_type = contradiction_type_to_str(&c.contradiction_type).to_string();
        let severity = c.severity;
        let judge_reasoning = c.judge_reasoning.clone();
        let detected_at = c.detected_at.to_rfc3339();
        let resolved_at = c.resolved_at.map(|t| t.to_rfc3339());
        let resolution = c
            .resolution
            .as_ref()
            .map(|r| resolution_to_str(r).to_string());

        self.db.with_connection(move |conn| {
            conn.execute(
                "INSERT INTO kg_belief_contradictions (
                    id, belief_a_id, belief_b_id, contradiction_type, severity,
                    judge_reasoning, detected_at, resolved_at, resolution
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
                )
                ON CONFLICT(belief_a_id, belief_b_id) DO NOTHING",
                params![
                    id,
                    belief_a_id,
                    belief_b_id,
                    contradiction_type,
                    severity,
                    judge_reasoning,
                    detected_at,
                    resolved_at,
                    resolution,
                ],
            )?;
            Ok(())
        })
    }

    async fn for_belief(&self, belief_id: &str) -> Result<Vec<BeliefContradiction>, String> {
        let belief_id = belief_id.to_string();
        let rows = self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, belief_a_id, belief_b_id, contradiction_type, severity,
                        judge_reasoning, detected_at, resolved_at, resolution
                 FROM kg_belief_contradictions
                 WHERE belief_a_id = ?1 OR belief_b_id = ?1
                 ORDER BY detected_at DESC",
            )?;
            let rows = stmt
                .query_map(params![belief_id], row_to_contradiction)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.into_iter().collect()
    }

    async fn list_recent(
        &self,
        partition_id: &str,
        limit: usize,
    ) -> Result<Vec<BeliefContradiction>, String> {
        let partition_id = partition_id.to_string();
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = self.db.with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT bc.id, bc.belief_a_id, bc.belief_b_id, bc.contradiction_type,
                        bc.severity, bc.judge_reasoning, bc.detected_at,
                        bc.resolved_at, bc.resolution
                 FROM kg_belief_contradictions bc
                 JOIN kg_beliefs b ON b.id = bc.belief_a_id
                 WHERE b.partition_id = ?1
                 ORDER BY bc.detected_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![partition_id, limit_i64], row_to_contradiction)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.into_iter().collect()
    }

    async fn pair_exists(&self, belief_a_id: &str, belief_b_id: &str) -> Result<bool, String> {
        let (a, b) = canonical_pair(belief_a_id, belief_b_id);
        let a = a.to_string();
        let b = b.to_string();
        self.db.with_connection(move |conn| {
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM kg_belief_contradictions \
                     WHERE belief_a_id = ?1 AND belief_b_id = ?2",
                    params![a, b],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(exists.is_some())
        })
    }

    async fn resolve(&self, contradiction_id: &str, resolution: Resolution) -> Result<(), String> {
        let id = contradiction_id.to_string();
        let resolution_str = resolution_to_str(&resolution).to_string();
        let now = Utc::now().to_rfc3339();
        self.db.with_connection(move |conn| {
            conn.execute(
                "UPDATE kg_belief_contradictions
                 SET resolution = ?1, resolved_at = ?2
                 WHERE id = ?3",
                params![resolution_str, now, id],
            )?;
            Ok(())
        })
    }
}

/// Map one row of `kg_belief_contradictions` to a `BeliefContradiction`.
/// Returns `Result<_, String>` per row so callers can fail loud on
/// malformed timestamps / enum strings rather than silently dropping rows.
fn row_to_contradiction(
    row: &rusqlite::Row,
) -> rusqlite::Result<Result<BeliefContradiction, String>> {
    let id: String = row.get(0)?;
    let belief_a_id: String = row.get(1)?;
    let belief_b_id: String = row.get(2)?;
    let contradiction_type_s: String = row.get(3)?;
    let severity: f64 = row.get(4)?;
    let judge_reasoning: Option<String> = row.get(5)?;
    let detected_at_s: String = row.get(6)?;
    let resolved_at_s: Option<String> = row.get(7)?;
    let resolution_s: Option<String> = row.get(8)?;

    let contradiction_type = match contradiction_type_from_str(&contradiction_type_s) {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };

    let parse_dt = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| format!("parse timestamp {s}: {e}"))
    };

    let detected_at = match parse_dt(&detected_at_s) {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };

    let resolved_at = match resolved_at_s.as_deref().map(parse_dt).transpose() {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };

    let resolution = match resolution_s.as_deref().map(resolution_from_str).transpose() {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };

    Ok(Ok(BeliefContradiction {
        id,
        belief_a_id,
        belief_b_id,
        contradiction_type,
        severity,
        judge_reasoning,
        detected_at,
        resolved_at,
        resolution,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnowledgeDatabase, SqliteBeliefStore};
    use gateway_services::VaultPaths;
    use tempfile::TempDir;
    use zbot_stores_domain::Belief;
    use zbot_stores_traits::BeliefStore;

    fn make_stores() -> (SqliteBeliefContradictionStore, SqliteBeliefStore, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        (
            SqliteBeliefContradictionStore::new(db.clone()),
            SqliteBeliefStore::new(db),
            tmp,
        )
    }

    async fn seed_belief(
        belief_store: &SqliteBeliefStore,
        id: &str,
        subject: &str,
        partition: &str,
    ) {
        let now = Utc::now();
        let b = Belief {
            id: id.to_string(),
            partition_id: partition.to_string(),
            subject: subject.to_string(),
            content: format!("{subject} content"),
            confidence: 0.8,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec![format!("fact-{id}")],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        belief_store.upsert_belief(&b).await.unwrap();
    }

    fn sample_contradiction(
        id: &str,
        a: &str,
        b: &str,
        t: ContradictionType,
    ) -> BeliefContradiction {
        BeliefContradiction {
            id: id.to_string(),
            belief_a_id: a.to_string(),
            belief_b_id: b.to_string(),
            contradiction_type: t,
            severity: 0.85,
            judge_reasoning: Some("test reasoning".to_string()),
            detected_at: Utc::now(),
            resolved_at: None,
            resolution: None,
        }
    }

    // ------------------------------------------------------------------
    // Lex ordering — insert with (b, a) where a < b must store with a on
    // the belief_a_id column.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn insert_enforces_lex_ordering_when_caller_swaps() {
        let (store, beliefs, _tmp) = make_stores();
        seed_belief(&beliefs, "b-aaa", "subj.aaa", "p").await;
        seed_belief(&beliefs, "b-zzz", "subj.zzz", "p").await;

        // Caller passes (larger, smaller) — store must canonicalize.
        let c = sample_contradiction("c-1", "b-zzz", "b-aaa", ContradictionType::Logical);
        store.insert_contradiction(&c).await.unwrap();

        let listed = store.list_recent("p", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].belief_a_id, "b-aaa",
            "lex-smaller must be on left"
        );
        assert_eq!(listed[0].belief_b_id, "b-zzz");
    }

    // ------------------------------------------------------------------
    // Idempotency — re-insert same canonical pair is no-op.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn insert_is_idempotent_on_unique_pair() {
        let (store, beliefs, _tmp) = make_stores();
        seed_belief(&beliefs, "b-1", "s.1", "p").await;
        seed_belief(&beliefs, "b-2", "s.2", "p").await;

        let c1 = sample_contradiction("c-1", "b-1", "b-2", ContradictionType::Logical);
        store.insert_contradiction(&c1).await.unwrap();

        // Second insert with a different id but the same pair must be a
        // no-op — UNIQUE constraint + ON CONFLICT DO NOTHING.
        let c2 = sample_contradiction("c-2", "b-2", "b-1", ContradictionType::Tension);
        store.insert_contradiction(&c2).await.unwrap();

        let listed = store.list_recent("p", 10).await.unwrap();
        assert_eq!(listed.len(), 1, "second insert was a no-op");
        // First insert's id wins.
        assert_eq!(listed[0].id, "c-1");
        assert_eq!(listed[0].contradiction_type, ContradictionType::Logical);
    }

    // ------------------------------------------------------------------
    // pair_exists honors canonical ordering.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn pair_exists_works_in_both_orders() {
        let (store, beliefs, _tmp) = make_stores();
        seed_belief(&beliefs, "b-1", "s.1", "p").await;
        seed_belief(&beliefs, "b-2", "s.2", "p").await;

        let c = sample_contradiction("c-1", "b-1", "b-2", ContradictionType::Tension);
        store.insert_contradiction(&c).await.unwrap();

        assert!(store.pair_exists("b-1", "b-2").await.unwrap());
        assert!(store.pair_exists("b-2", "b-1").await.unwrap());
        assert!(!store.pair_exists("b-1", "b-3").await.unwrap());
    }

    // ------------------------------------------------------------------
    // for_belief returns rows regardless of which side the belief sits on.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn for_belief_returns_rows_on_either_side() {
        let (store, beliefs, _tmp) = make_stores();
        seed_belief(&beliefs, "b-aaa", "s.aaa", "p").await;
        seed_belief(&beliefs, "b-bbb", "s.bbb", "p").await;
        seed_belief(&beliefs, "b-ccc", "s.ccc", "p").await;

        // After canonicalization: aaa < bbb < ccc.
        // c-1: (aaa, bbb) → b-bbb on the right.
        // c-2: (bbb, ccc) → b-bbb on the left.
        store
            .insert_contradiction(&sample_contradiction(
                "c-1",
                "b-aaa",
                "b-bbb",
                ContradictionType::Logical,
            ))
            .await
            .unwrap();
        store
            .insert_contradiction(&sample_contradiction(
                "c-2",
                "b-bbb",
                "b-ccc",
                ContradictionType::Tension,
            ))
            .await
            .unwrap();

        let bbb_hits = store.for_belief("b-bbb").await.unwrap();
        assert_eq!(
            bbb_hits.len(),
            2,
            "must find rows whether belief is on left or right"
        );
        let ids: std::collections::HashSet<_> = bbb_hits.iter().map(|c| c.id.clone()).collect();
        assert!(ids.contains("c-1"));
        assert!(ids.contains("c-2"));
    }

    // ------------------------------------------------------------------
    // list_recent filters by partition via the join through kg_beliefs.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn list_recent_filters_by_partition() {
        let (store, beliefs, _tmp) = make_stores();
        seed_belief(&beliefs, "b-a1", "s.a1", "part-A").await;
        seed_belief(&beliefs, "b-a2", "s.a2", "part-A").await;
        seed_belief(&beliefs, "b-b1", "s.b1", "part-B").await;
        seed_belief(&beliefs, "b-b2", "s.b2", "part-B").await;

        store
            .insert_contradiction(&sample_contradiction(
                "c-a",
                "b-a1",
                "b-a2",
                ContradictionType::Logical,
            ))
            .await
            .unwrap();
        store
            .insert_contradiction(&sample_contradiction(
                "c-b",
                "b-b1",
                "b-b2",
                ContradictionType::Tension,
            ))
            .await
            .unwrap();

        let part_a = store.list_recent("part-A", 10).await.unwrap();
        assert_eq!(part_a.len(), 1);
        assert_eq!(part_a[0].id, "c-a");

        let part_b = store.list_recent("part-B", 10).await.unwrap();
        assert_eq!(part_b.len(), 1);
        assert_eq!(part_b[0].id, "c-b");
    }

    // ------------------------------------------------------------------
    // resolve writes resolution + resolved_at.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn resolve_sets_resolution_and_timestamp() {
        let (store, beliefs, _tmp) = make_stores();
        seed_belief(&beliefs, "b-1", "s.1", "p").await;
        seed_belief(&beliefs, "b-2", "s.2", "p").await;
        store
            .insert_contradiction(&sample_contradiction(
                "c-1",
                "b-1",
                "b-2",
                ContradictionType::Logical,
            ))
            .await
            .unwrap();

        store.resolve("c-1", Resolution::AWon).await.unwrap();

        let rows = store.for_belief("b-1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].resolution, Some(Resolution::AWon));
        assert!(rows[0].resolved_at.is_some());
    }

    #[test]
    fn canonical_pair_sorts_lex() {
        assert_eq!(canonical_pair("b", "a"), ("a", "b"));
        assert_eq!(canonical_pair("a", "b"), ("a", "b"));
        assert_eq!(canonical_pair("aaa", "aab"), ("aaa", "aab"));
    }
}
