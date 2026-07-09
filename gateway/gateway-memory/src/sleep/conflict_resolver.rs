//! Conflict Resolver — supersedes contradicting schema facts at sleep-time.
//!
//! Pairwise embedding similarity + LLM judge. When two schema facts about
//! the same topic disagree, the lower-confidence/older one is marked with
//! `superseded_by` pointing to the winner. Recall filters superseded facts.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use serde::Deserialize;
use zbot_stores_traits::{CompactionStore, MemoryFact, MemoryFactStore};

use crate::sleep::belief_propagator::BeliefPropagator;
use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

const MAX_SCHEMA_FACTS_PER_CYCLE: usize = 50;
const MAX_LLM_CALLS_PER_CYCLE: usize = 10;
const MIN_SIMILARITY: f32 = 0.85;
const MIN_CONFIDENCE: f64 = 0.7;

/// Stats returned from one resolution cycle.
#[derive(Debug, Default, Clone)]
pub struct ConflictStats {
    pub facts_considered: u64,
    pub pairs_examined: u64,
    pub llm_calls_made: u64,
    pub conflicts_resolved: u64,
    pub skipped_compatible: u64,
    pub skipped_low_confidence: u64,
    pub skipped_llm_error: u64,
}

/// Parsed LLM response shape.
#[derive(Debug, Clone, Deserialize)]
pub struct ConflictResponse {
    pub decision: String, // "contradicts" | "compatible"
    pub confidence: f64,
    pub reason: String,
}

/// LLM judge that decides whether a pair of facts actually contradict.
#[async_trait]
pub trait ConflictJudgeLlm: Send + Sync {
    async fn judge(&self, a: &str, b: &str) -> Result<ConflictResponse, String>;
}

/// Sleep-time component that supersedes contradicting schema facts.
///
/// When the Belief Network is enabled, ConflictResolver carries an
/// `Option<Arc<BeliefPropagator>>` and fires
/// [`BeliefPropagator::propagate_invalidation`] inline after every
/// successful `supersede_fact` call. Beliefs derived from the losing
/// fact get retracted (sole source) or marked stale (multi-source)
/// before the next sleep cycle. The propagator never bubbles errors —
/// supersession always succeeds even if belief-side propagation fails.
pub struct ConflictResolver {
    memory_store: Arc<dyn MemoryFactStore>,
    compaction_store: Arc<dyn CompactionStore>,
    llm: Arc<dyn ConflictJudgeLlm>,
    /// Minimum time between LLM judge passes. `Duration::ZERO` = every cycle.
    interval: Duration,
    last_run: Mutex<Option<Instant>>,
    /// B-3: optional propagator. `None` when the Belief Network is
    /// disabled or its store isn't wired — the resolver skips the
    /// propagation call entirely.
    belief_propagator: Option<Arc<BeliefPropagator>>,
}

impl ConflictResolver {
    pub fn new(
        memory_store: Arc<dyn MemoryFactStore>,
        compaction_store: Arc<dyn CompactionStore>,
        llm: Arc<dyn ConflictJudgeLlm>,
        interval: Duration,
    ) -> Self {
        Self {
            memory_store,
            compaction_store,
            llm,
            interval,
            last_run: Mutex::new(None),
            belief_propagator: None,
        }
    }

    /// Builder-style: attach a [`BeliefPropagator`] so successful
    /// supersessions fire B-3 propagation inline. Pass `None` to leave
    /// the resolver in B-2 behavior.
    #[must_use]
    pub fn with_belief_propagator(mut self, propagator: Option<Arc<BeliefPropagator>>) -> Self {
        self.belief_propagator = propagator;
        self
    }

    /// Run one resolution cycle. Returns aggregate stats. Conservative:
    /// any per-pair error is logged and skipped — cycle never fails hard.
    pub async fn run_cycle(&self, run_id: &str, agent_id: &str) -> Result<ConflictStats, String> {
        if !self.interval.is_zero() {
            if let Some(last) = *self.last_run.lock().unwrap() {
                if last.elapsed() < self.interval {
                    return Ok(ConflictStats::default());
                }
            }
        }

        let mut stats = ConflictStats::default();

        let raw_facts = self
            .memory_store
            .get_facts_by_category(agent_id, "schema", MAX_SCHEMA_FACTS_PER_CYCLE)
            .await
            .map_err(|e| format!("get_facts_by_category: {e}"))?;
        // Drop already-superseded facts — they're losers from a prior cycle.
        // Hydrate embeddings from the vector index (stored separately from the
        // facts table; `get_facts_by_category` always returns embedding: None).
        let mut facts: Vec<MemoryFact> = Vec::with_capacity(raw_facts.len());
        for mut f in raw_facts {
            if f.superseded_by.is_some() {
                continue;
            }
            if f.embedding.is_none() {
                if let Ok(Some(emb)) = self.memory_store.get_fact_embedding(&f.id).await {
                    f.embedding = Some(emb);
                }
            }
            facts.push(f);
        }
        stats.facts_considered = facts.len() as u64;

        if facts.len() < 2 {
            *self.last_run.lock().unwrap() = Some(Instant::now());
            return Ok(stats);
        }

        let mut llm_budget = MAX_LLM_CALLS_PER_CYCLE;
        let mut superseded_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for i in 0..facts.len() {
            if llm_budget == 0 {
                break;
            }
            if superseded_ids.contains(&facts[i].id) {
                continue;
            }
            for j in (i + 1)..facts.len() {
                if llm_budget == 0 {
                    break;
                }
                if superseded_ids.contains(&facts[j].id) {
                    continue;
                }
                let sim = match (facts[i].embedding.as_ref(), facts[j].embedding.as_ref()) {
                    (Some(a), Some(b)) => cosine(a, b),
                    _ => continue,
                };
                if sim < MIN_SIMILARITY {
                    continue;
                }
                stats.pairs_examined += 1;
                stats.llm_calls_made += 1;
                llm_budget -= 1;

                let resp = match self.llm.judge(&facts[i].content, &facts[j].content).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            agent_id, error = %e,
                            "conflict-resolver: LLM judge failed"
                        );
                        stats.skipped_llm_error += 1;
                        continue;
                    }
                };
                if resp.decision != "contradicts" {
                    stats.skipped_compatible += 1;
                    continue;
                }
                if resp.confidence < MIN_CONFIDENCE {
                    stats.skipped_low_confidence += 1;
                    continue;
                }

                let (winner, loser) = pick_winner(&facts[i], &facts[j]);
                // Bi-temporal transition: loser's truth-interval closes at the
                // moment the system actually learned otherwise — i.e. when the
                // winning fact was first recorded. Falls back to `now()` if the
                // winner's `created_at` is unparseable so a malformed row
                // can't break the resolution cycle.
                let transition_time = match chrono::DateTime::parse_from_rfc3339(&winner.created_at)
                {
                    Ok(dt) => dt.with_timezone(&chrono::Utc),
                    Err(e) => {
                        tracing::warn!(
                            winner_id = %winner.id,
                            created_at = %winner.created_at,
                            error = %e,
                            "conflict-resolver: winner.created_at unparseable; falling back to now()"
                        );
                        chrono::Utc::now()
                    }
                };
                if let Err(e) = self
                    .memory_store
                    .supersede_fact(&loser.id, &winner.id, transition_time)
                    .await
                {
                    tracing::warn!(
                        loser_id = %loser.id,
                        winner_id = %winner.id,
                        error = %e,
                        "conflict-resolver: supersede_fact failed"
                    );
                    continue;
                }
                stats.conflicts_resolved += 1;
                superseded_ids.insert(loser.id.clone());

                // B-3: propagate the invalidation to dependent beliefs.
                // The propagator never bubbles errors — supersession
                // already succeeded above; belief-side failures are
                // logged inside `propagate_invalidation` and the cycle
                // continues.
                if let Some(propagator) = self.belief_propagator.as_ref() {
                    let prop_stats = propagator
                        .propagate_invalidation(&loser.id, transition_time)
                        .await;
                    if prop_stats.beliefs_invalidated > 0 || prop_stats.errors > 0 {
                        tracing::debug!(
                            loser_id = %loser.id,
                            invalidated = prop_stats.beliefs_invalidated,
                            retracted = prop_stats.beliefs_retracted,
                            stale = prop_stats.beliefs_marked_stale,
                            errors = prop_stats.errors,
                            "conflict-resolver: belief propagation fired"
                        );
                    }
                }

                let reason = format!(
                    "superseded by {} (sim={:.2}, judge_conf={:.2}): {}",
                    winner.id, sim, resp.confidence, resp.reason
                );
                let _ = self
                    .compaction_store
                    .record_synthesis(run_id, &loser.id, &reason)
                    .await;
            }
        }

        *self.last_run.lock().unwrap() = Some(Instant::now());
        Ok(stats)
    }
}

/// Pick the winner of a contradicting pair. Higher confidence wins; tie
/// broken by newer `updated_at`. Returns `(winner, loser)`.
fn pick_winner<'a>(a: &'a MemoryFact, b: &'a MemoryFact) -> (&'a MemoryFact, &'a MemoryFact) {
    if a.confidence > b.confidence {
        (a, b)
    } else if b.confidence > a.confidence {
        (b, a)
    } else if a.updated_at >= b.updated_at {
        (a, b)
    } else {
        (b, a)
    }
}

/// Cosine similarity between two `f32` vectors. Returns 0.0 for empty or
/// mismatched-length inputs (caller treats below-threshold as no candidate).
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

// ============================================================================
// LLM-backed implementation
// ============================================================================

/// Production conflict judge wired to the injected `MemoryLlmFactory`.
pub struct LlmConflictJudge {
    client: CachedLlmClient,
}

impl LlmConflictJudge {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 256)),
        }
    }
}

#[async_trait]
impl ConflictJudgeLlm for LlmConflictJudge {
    async fn judge(&self, a: &str, b: &str) -> Result<ConflictResponse, String> {
        let client = self.client.get().await?;
        let prompt = format!(
            "You judge whether two principles for an AI agent contradict each other.\n\
             Two principles can be about the same topic and NOT contradict (they may\n\
             cover different cases). Only say \"contradicts\" if one principle's\n\
             prescription would violate the other.\n\n\
             Return ONLY JSON: \
             {{\"decision\": \"contradicts\" | \"compatible\", \
             \"confidence\": 0.0-1.0, \"reason\": string}}.\n\n\
             Principle A: {a}\n\
             Principle B: {b}",
        );
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        parse_llm_json::<ConflictResponse>(&response.content)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::VaultPaths;
    use std::sync::Mutex;
    use zbot_stores_sqlite::vector_index::{SqliteVecIndex, VectorIndex};
    use zbot_stores_sqlite::{
        CompactionRepository, GatewayCompactionStore, GatewayMemoryFactStore, KnowledgeDatabase,
        MemoryRepository,
    };

    struct MockJudge {
        response: Mutex<ConflictResponse>,
    }

    impl MockJudge {
        fn new(resp: ConflictResponse) -> Self {
            Self {
                response: Mutex::new(resp),
            }
        }
    }

    #[async_trait]
    impl ConflictJudgeLlm for MockJudge {
        async fn judge(&self, _a: &str, _b: &str) -> Result<ConflictResponse, String> {
            Ok(self.response.lock().unwrap().clone())
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        memory_store: Arc<dyn MemoryFactStore>,
        compaction_store: Arc<dyn CompactionStore>,
        knowledge_db: Arc<KnowledgeDatabase>,
    }

    fn setup() -> Harness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("db"));
        let vec_index: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id")
                .expect("vec index init"),
        );
        let memory_repo = Arc::new(MemoryRepository::new(db.clone(), vec_index));
        let compaction_repo = Arc::new(CompactionRepository::new(db.clone()));
        Harness {
            _tmp: tmp,
            memory_store: Arc::new(GatewayMemoryFactStore::new(memory_repo, None)),
            compaction_store: Arc::new(GatewayCompactionStore::new(compaction_repo)),
            knowledge_db: db,
        }
    }

    /// Seed two schema facts with identical embeddings → guaranteed similar
    /// (cosine == 1.0, well above MIN_SIMILARITY = 0.85). Uses
    /// `upsert_typed_fact` so the embedding is persisted regardless of whether
    /// the test harness has an embedder client configured.
    async fn seed_two_schemas(
        store: &Arc<dyn MemoryFactStore>,
        agent_id: &str,
        a_content: &str,
        b_content: &str,
    ) {
        use serde_json::json;
        let now = chrono::Utc::now().to_rfc3339();
        // 384-dim unit vector along axis 0 — matches the sqlite-vec DDL dimension
        // and cosine(v, v) == 1.0, well above MIN_SIMILARITY = 0.85.
        let mut embedding: Vec<f32> = vec![0.0; 384];
        embedding[0] = 1.0;

        for (key, content, confidence) in [
            ("schema.a", a_content, 0.9_f64),
            ("schema.b", b_content, 0.8_f64),
        ] {
            let id = format!("fact-{}", uuid::Uuid::new_v4());
            let fact = json!({
                "id": id,
                "session_id": null,
                "agent_id": agent_id,
                "scope": "agent",
                "category": "schema",
                "key": key,
                "content": content,
                "confidence": confidence,
                "mention_count": 1,
                "source_summary": null,
                "ward_id": "__global__",
                "contradicted_by": null,
                "created_at": now,
                "updated_at": now,
                "expires_at": null,
                "valid_from": null,
                "valid_until": null,
                "superseded_by": null,
                "pinned": false,
                "epistemic_class": "current",
                "source_episode_id": null,
                "source_ref": null,
            });
            store
                .upsert_typed_fact(fact, Some(embedding.clone()))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn cosine_handles_empty_and_mismatched() {
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        let v = vec![1.0_f32, 0.0, 0.0];
        // identical vectors → 1.0
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pick_winner_prefers_higher_confidence() {
        let now = chrono::Utc::now().to_rfc3339();
        let high = MemoryFact {
            id: "hi".into(),
            session_id: None,
            agent_id: "a".into(),
            scope: "agent".into(),
            category: "schema".into(),
            key: "k1".into(),
            content: "c".into(),
            confidence: 0.9,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "__global__".into(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".into()),
            source_episode_id: None,
            source_ref: None,
        };
        let mut low = high.clone();
        low.id = "lo".into();
        low.confidence = 0.5;
        let (w, l) = pick_winner(&high, &low);
        assert_eq!(w.id, "hi");
        assert_eq!(l.id, "lo");
    }

    #[tokio::test]
    async fn resolves_contradicting_schemas() {
        let h = setup();
        seed_two_schemas(
            &h.memory_store,
            "agent-c",
            "Always use rebase merges",
            "Never use rebase merges",
        )
        .await;

        let judge = Arc::new(MockJudge::new(ConflictResponse {
            decision: "contradicts".into(),
            confidence: 0.9,
            reason: "opposite prescriptions".into(),
        }));
        let resolver = ConflictResolver::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            judge,
            Duration::ZERO,
        );

        let stats = resolver.run_cycle("run-c", "agent-c").await.unwrap();
        assert_eq!(stats.facts_considered, 2);
        assert_eq!(stats.pairs_examined, 1);
        assert_eq!(stats.conflicts_resolved, 1);
        let facts = h
            .memory_store
            .get_facts_by_category("agent-c", "schema", 10)
            .await
            .unwrap();
        let superseded = facts.iter().filter(|f| f.superseded_by.is_some()).count();
        assert_eq!(superseded, 1, "exactly one schema should be superseded");
    }

    #[tokio::test]
    async fn does_not_resolve_when_judge_says_compatible() {
        let h = setup();
        seed_two_schemas(
            &h.memory_store,
            "agent-compat",
            "Use rebase for feature branches",
            "Use merge commits for release branches",
        )
        .await;

        let judge = Arc::new(MockJudge::new(ConflictResponse {
            decision: "compatible".into(),
            confidence: 0.9,
            reason: "different scopes".into(),
        }));
        let resolver = ConflictResolver::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            judge,
            Duration::ZERO,
        );

        let stats = resolver
            .run_cycle("run-compat", "agent-compat")
            .await
            .unwrap();
        assert_eq!(stats.conflicts_resolved, 0);
        let facts = h
            .memory_store
            .get_facts_by_category("agent-compat", "schema", 10)
            .await
            .unwrap();
        assert!(facts.iter().all(|f| f.superseded_by.is_none()));
    }

    #[tokio::test]
    async fn throttle_skips_when_interval_not_elapsed() {
        let h = setup();
        seed_two_schemas(&h.memory_store, "agent-t", "Foo", "Foo").await;

        let judge = Arc::new(MockJudge::new(ConflictResponse {
            decision: "compatible".into(),
            confidence: 0.5,
            reason: "test".into(),
        }));
        let resolver = ConflictResolver::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            judge,
            Duration::from_secs(3600),
        );

        // First call records last_run.
        let _ = resolver.run_cycle("r1", "agent-t").await.unwrap();
        // Immediate second call: throttled, returns default zero stats.
        let s2 = resolver.run_cycle("r2", "agent-t").await.unwrap();
        assert_eq!(s2.facts_considered, 0);
        assert_eq!(s2.pairs_examined, 0);
    }

    // ------------------------------------------------------------------
    // B-3: supersession fires belief propagation.
    // ------------------------------------------------------------------

    /// When ConflictResolver supersedes a fact, the attached
    /// BeliefPropagator fires. A belief whose sole source is the loser
    /// fact is retracted (valid_until set); a multi-source belief is
    /// marked stale.
    #[tokio::test]
    async fn supersession_fires_belief_propagation() {
        use zbot_stores_sqlite::SqliteBeliefStore;
        use zbot_stores_traits::Belief;
        use zbot_stores_traits::BeliefStore;

        let h = setup();
        seed_two_schemas(
            &h.memory_store,
            "agent-prop",
            "Always rebase",
            "Never rebase",
        )
        .await;

        // Look up the loser fact's id so we can wire a belief that
        // sources from it. The lower-confidence fact (`schema.b`, 0.8)
        // will be superseded by `schema.a` (0.9).
        let facts = h
            .memory_store
            .get_facts_by_category("agent-prop", "schema", 10)
            .await
            .unwrap();
        let loser_id = facts
            .iter()
            .find(|f| f.key == "schema.b")
            .expect("schema.b seeded")
            .id
            .clone();
        let winner_id = facts
            .iter()
            .find(|f| f.key == "schema.a")
            .unwrap()
            .id
            .clone();

        // Wire a real SqliteBeliefStore against the same KnowledgeDatabase
        // the memory store uses.
        let knowledge_db = h.knowledge_db.clone();
        let belief_store: Arc<dyn BeliefStore> = Arc::new(SqliteBeliefStore::new(knowledge_db));
        let now = chrono::Utc::now();
        let sole_belief = Belief {
            id: "belief-sole".into(),
            partition_id: "agent-prop".into(),
            subject: "schema.b".into(),
            content: "Never rebase".into(),
            confidence: 0.8,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec![loser_id.clone()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        let multi_belief = Belief {
            id: "belief-multi".into(),
            partition_id: "agent-prop".into(),
            subject: "schema.related".into(),
            content: "Either way is fine".into(),
            confidence: 0.7,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec![loser_id.clone(), winner_id.clone()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        belief_store.upsert_belief(&sole_belief).await.unwrap();
        belief_store.upsert_belief(&multi_belief).await.unwrap();

        let propagator = Arc::new(crate::sleep::belief_propagator::BeliefPropagator::new(
            belief_store.clone(),
            true,
        ));

        let judge = Arc::new(MockJudge::new(ConflictResponse {
            decision: "contradicts".into(),
            confidence: 0.9,
            reason: "opposite prescriptions".into(),
        }));
        let resolver = ConflictResolver::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            judge,
            Duration::ZERO,
        )
        .with_belief_propagator(Some(propagator));

        let stats = resolver.run_cycle("run-prop", "agent-prop").await.unwrap();
        assert_eq!(stats.conflicts_resolved, 1);

        // Sole-source belief: retracted (valid_until set).
        let sole_after = belief_store
            .get_belief_by_id("belief-sole")
            .await
            .unwrap()
            .expect("sole belief persists");
        assert!(
            sole_after.valid_until.is_some(),
            "sole-source belief must be retracted; got: {:?}",
            sole_after.valid_until
        );

        // Multi-source belief: marked stale.
        let multi_after = belief_store
            .get_belief_by_id("belief-multi")
            .await
            .unwrap()
            .expect("multi belief persists");
        assert!(
            multi_after.stale,
            "multi-source belief must be marked stale"
        );
        assert!(
            multi_after.valid_until.is_none(),
            "multi-source belief must NOT be retracted"
        );
    }

    /// When the propagator is `None` (Belief Network disabled), the
    /// resolver still supersedes facts but does not touch beliefs.
    #[tokio::test]
    async fn supersession_without_propagator_is_unchanged() {
        let h = setup();
        seed_two_schemas(
            &h.memory_store,
            "agent-nop",
            "Always rebase",
            "Never rebase",
        )
        .await;

        let judge = Arc::new(MockJudge::new(ConflictResponse {
            decision: "contradicts".into(),
            confidence: 0.9,
            reason: "test".into(),
        }));
        // No propagator wired — `belief_propagator: None` path.
        let resolver = ConflictResolver::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            judge,
            Duration::ZERO,
        );

        let stats = resolver.run_cycle("run-nop", "agent-nop").await.unwrap();
        assert_eq!(stats.conflicts_resolved, 1, "supersession still runs");
    }
}
