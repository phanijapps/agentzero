//! Belief Synthesizer — sleep-time worker that derives a `Belief` per
//! `(partition_id, subject)` from constituent `MemoryFact`s.
//!
//! Phase B-1 of the Belief Network (see
//! `docs/architecture/future-state/2026-05-15-belief-network-design.md`).
//!
//! Key decisions baked into this implementation (the 5 originally-open
//! questions, all resolved):
//!
//! 1. Subject canonicalization: exact key match (no embedding similarity).
//! 2. Confidence formula:
//!    `belief.confidence = avg(fact.confidence × recency_weight(fact.valid_from))`
//!    where `recency_weight = 1 / (1 + age_days / 90)`.
//! 3. Single-fact short-circuit: if a subject has only ONE fact, the LLM
//!    is skipped and the belief content / confidence are derived from
//!    that fact directly. ~95% of subjects fall into this path in real
//!    data, so the optimization is load-bearing for cost.
//! 4. Multi-fact synthesis: one LLM call (prompt below) returns
//!    `{content, reasoning}` JSON. On parse failure or LLM error, we
//!    fall back to the short-circuit path using the most-recent fact.
//! 5. Generic from day one: schema uses `partition_id`, not `ward_id` —
//!    when the R-series rename lands, beliefs won't need migration.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::llm::embedding::EmbeddingClient;
use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use zbot_stores_domain::{Belief, MemoryFact};
use zbot_stores_traits::{BeliefStore, MemoryFactStore};

use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

/// Synthesizer algorithm + prompt version. Bump when the synthesis
/// prompt or aggregation rules change so old beliefs can be flagged
/// for re-synthesis without losing the historical version.
const SYNTHESIZER_VERSION: i32 = 1;

/// Half-life (in days) used by the recency weight. Older facts contribute
/// less to the aggregate confidence; the curve is `1 / (1 + age_days / 90)`.
const RECENCY_HALF_LIFE_DAYS: f64 = 90.0;

/// Hard cap on facts scanned per cycle. Keeps the synthesizer bounded
/// even on a large partition until Phase B-2 introduces a dirty-subject
/// watermark.
const MAX_FACTS_PER_CYCLE: usize = 1000;

/// Stats returned from one synthesis cycle. Tracked separately for
/// short-circuit vs LLM paths so we can confirm the optimization is
/// firing on real data.
///
/// `stale_beliefs_resynthesized` (B-3) counts beliefs the cycle picked
/// up from the stale-flag queue at the top of the cycle, before the
/// normal dirty-subject pass. These overlap with `beliefs_synthesized`
/// (every stale resynth still increments the synthesized counter) but
/// are reported separately so operators can see the propagation
/// pathway firing.
#[derive(Debug, Default, Clone)]
pub struct BeliefSynthesisStats {
    pub subjects_examined: u64,
    pub beliefs_synthesized: u64,
    pub beliefs_short_circuited: u64,
    pub beliefs_llm_synthesized: u64,
    pub llm_calls: u64,
    pub errors: u64,
    pub stale_beliefs_resynthesized: u64,
}

/// Parsed multi-fact LLM response shape.
#[derive(Debug, Clone, Deserialize)]
pub struct SynthesisLlmResponse {
    pub content: String,
    pub reasoning: String,
}

/// LLM abstraction so tests can inject a mock.
#[async_trait]
pub trait BeliefSynthesisLlm: Send + Sync {
    async fn synthesize(
        &self,
        subject: &str,
        facts: &[MemoryFact],
    ) -> Result<SynthesisLlmResponse, String>;
}

/// Sleep-time worker that re-derives beliefs from constituent facts.
pub struct BeliefSynthesizer {
    fact_store: Arc<dyn MemoryFactStore>,
    belief_store: Arc<dyn BeliefStore>,
    llm: Arc<dyn BeliefSynthesisLlm>,
    /// Embedding client used to generate `belief.embedding` at
    /// synthesis time (Phase B-4). When `None`, beliefs are written
    /// with `embedding = None` and won't surface in semantic recall.
    /// Embedding failures are logged and degrade to `None` rather
    /// than aborting synthesis — beliefs are still useful via direct
    /// lookup.
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
    /// Minimum time between cycles. `Duration::ZERO` runs every tick.
    interval: Duration,
    last_run: Mutex<Option<Instant>>,
    /// Master enable flag — when false, every `run_cycle` call returns
    /// immediately with empty stats. Beliefs are opt-in.
    enabled: bool,
}

impl BeliefSynthesizer {
    pub fn new(
        fact_store: Arc<dyn MemoryFactStore>,
        belief_store: Arc<dyn BeliefStore>,
        llm: Arc<dyn BeliefSynthesisLlm>,
        interval: Duration,
        enabled: bool,
    ) -> Self {
        Self {
            fact_store,
            belief_store,
            llm,
            embedding_client: None,
            interval,
            last_run: Mutex::new(None),
            enabled,
        }
    }

    /// Attach an embedding client so synthesized beliefs carry a vector
    /// for semantic recall (Phase B-4). Builder-style so the existing
    /// callers that don't yet wire embeddings keep working. The
    /// generated embedding is best-effort: if the client returns an
    /// error or empty vector, the belief is still written with
    /// `embedding = None`.
    pub fn with_embedding_client(mut self, client: Option<Arc<dyn EmbeddingClient>>) -> Self {
        self.embedding_client = client;
        self
    }

    /// Run one synthesis cycle for a partition. Conservative: per-subject
    /// errors are logged and skipped; the cycle never fails hard.
    ///
    /// `run_id` is recorded in tracing logs to correlate with the
    /// surrounding sleep-time orchestration. `partition_id` is the
    /// agent_id / ward bucket to scan; for v1 we pass the agent_id.
    pub async fn run_cycle(
        &self,
        run_id: &str,
        partition_id: &str,
    ) -> Result<BeliefSynthesisStats, String> {
        if !self.enabled {
            return Ok(BeliefSynthesisStats::default());
        }
        if !self.interval_elapsed() {
            return Ok(BeliefSynthesisStats::default());
        }

        let mut stats = BeliefSynthesisStats::default();

        let facts = self
            .fact_store
            .list_memory_facts_typed(Some(partition_id), None, None, MAX_FACTS_PER_CYCLE, 0)
            .await
            .map_err(|e| format!("list_memory_facts_typed: {e}"))?;

        // Drop superseded facts — they belong to history; we synthesize
        // beliefs only over the active set.
        let active: Vec<MemoryFact> = facts
            .into_iter()
            .filter(|f| f.superseded_by.is_none())
            .collect();

        // Group facts by `key` alone — the key IS the subject. The
        // partition is the value passed into `run_cycle`, so we store
        // every belief under that partition regardless of the fact's
        // own `ward_id`. Phase B-2 may refine grouping when subject
        // canonicalization arrives.
        let mut by_subject: HashMap<String, Vec<MemoryFact>> = HashMap::new();
        for f in active {
            by_subject.entry(f.key.clone()).or_default().push(f);
        }

        stats.subjects_examined = by_subject.len() as u64;

        // B-3: stale beliefs go FIRST. Propagation marked them dirty
        // when a source fact was invalidated; we re-derive content +
        // confidence from the surviving sources, then clear the flag.
        // Subjects re-synthesized here are returned so the normal pass
        // below can skip them.
        let stale_subjects = self
            .process_stale_beliefs(run_id, partition_id, &by_subject, &mut stats)
            .await;

        // Filter the normal pass to skip already-processed stale subjects.
        for (key, mut group) in by_subject
            .into_iter()
            .filter(|(k, _)| !stale_subjects.contains(k))
        {
            // Sort oldest-first so the most-recent fact lands at the end
            // of the slice — both paths below treat the tail as primary.
            group.sort_by(|a, b| {
                let av = a.valid_from.as_deref().unwrap_or(&a.created_at);
                let bv = b.valid_from.as_deref().unwrap_or(&b.created_at);
                av.cmp(bv)
            });

            match self
                .synthesize_one(run_id, partition_id, &key, &group, &mut stats)
                .await
            {
                Ok(()) => stats.beliefs_synthesized += 1,
                Err(e) => {
                    stats.errors += 1;
                    tracing::warn!(
                        run_id,
                        partition_id,
                        subject = key,
                        error = %e,
                        "belief-synthesizer: subject failed"
                    );
                }
            }
        }

        *self.last_run.lock().unwrap() = Some(Instant::now());
        Ok(stats)
    }

    /// B-3: process stale beliefs from the propagation queue at the
    /// top of each cycle, before the normal dirty-subject pass.
    ///
    /// For each stale belief: look up its subject's facts in the
    /// already-grouped `by_subject` map, re-run synthesis via
    /// `synthesize_one` (which UPSERTs the belief), then call
    /// `clear_stale`. Subjects re-synthesized here are returned as a
    /// `HashSet` so the caller can skip them in the normal pass.
    ///
    /// Errors are logged and skipped — a single failing stale belief
    /// does not abort the cycle.
    async fn process_stale_beliefs(
        &self,
        run_id: &str,
        partition_id: &str,
        by_subject: &HashMap<String, Vec<MemoryFact>>,
        stats: &mut BeliefSynthesisStats,
    ) -> HashSet<String> {
        let mut processed = HashSet::new();
        let stale = match self
            .belief_store
            .list_stale(partition_id, MAX_FACTS_PER_CYCLE)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    run_id,
                    partition_id,
                    error = %e,
                    "belief-synthesizer: list_stale failed; skipping stale-first pass"
                );
                stats.errors += 1;
                return processed;
            }
        };

        for belief in stale {
            // Skip if we've already covered this subject this cycle —
            // multiple stale beliefs in the same subject collapse to a
            // single re-synthesis call.
            if processed.contains(&belief.subject) {
                if let Err(e) = self.belief_store.clear_stale(&belief.id).await {
                    tracing::warn!(belief_id = %belief.id, error = %e, "clear_stale failed");
                    stats.errors += 1;
                } else {
                    stats.stale_beliefs_resynthesized += 1;
                }
                continue;
            }
            let Some(group) = by_subject.get(&belief.subject) else {
                // The subject has no active facts left — propagation
                // already retracted the sole-source path; nothing to
                // re-synthesize. Leave the stale flag set so the next
                // cycle (or recall path) can observe the unresolved
                // state. This branch should be rare since sole-source
                // propagation retracts directly without setting stale.
                tracing::debug!(
                    belief_id = %belief.id,
                    subject = %belief.subject,
                    "belief-synthesizer: stale belief has no active facts; leaving stale"
                );
                continue;
            };
            // Re-synthesize with the surviving facts.
            let mut group = group.clone();
            group.sort_by(|a, b| {
                let av = a.valid_from.as_deref().unwrap_or(&a.created_at);
                let bv = b.valid_from.as_deref().unwrap_or(&b.created_at);
                av.cmp(bv)
            });
            match self
                .synthesize_one(run_id, partition_id, &belief.subject, &group, stats)
                .await
            {
                Ok(()) => {
                    stats.beliefs_synthesized += 1;
                    stats.stale_beliefs_resynthesized += 1;
                    processed.insert(belief.subject.clone());
                    if let Err(e) = self.belief_store.clear_stale(&belief.id).await {
                        tracing::warn!(belief_id = %belief.id, error = %e, "clear_stale failed");
                        stats.errors += 1;
                    }
                }
                Err(e) => {
                    stats.errors += 1;
                    tracing::warn!(
                        run_id,
                        partition_id,
                        subject = %belief.subject,
                        error = %e,
                        "belief-synthesizer: stale re-synthesis failed"
                    );
                }
            }
        }
        processed
    }

    /// Returns true if enough time has elapsed since the last run (or
    /// the worker has never run yet).
    fn interval_elapsed(&self) -> bool {
        if self.interval.is_zero() {
            return true;
        }
        match *self.last_run.lock().unwrap() {
            Some(last) => last.elapsed() >= self.interval,
            None => true,
        }
    }

    /// Synthesize one belief for a single (partition, subject) group.
    /// Decides short-circuit vs LLM, computes confidence, writes the
    /// belief via the store.
    async fn synthesize_one(
        &self,
        _run_id: &str,
        partition_id: &str,
        subject: &str,
        facts: &[MemoryFact],
        stats: &mut BeliefSynthesisStats,
    ) -> Result<(), String> {
        let now = Utc::now();
        let (content, reasoning, used_llm) = if facts.len() == 1 {
            stats.beliefs_short_circuited += 1;
            (facts[0].content.clone(), None, false)
        } else {
            // Multi-fact path — LLM call. On any failure, fall back to
            // the most-recent fact (treated as primary).
            stats.llm_calls += 1;
            match self.llm.synthesize(subject, facts).await {
                Ok(resp) => {
                    stats.beliefs_llm_synthesized += 1;
                    (resp.content, Some(resp.reasoning), true)
                }
                Err(e) => {
                    tracing::warn!(
                        partition_id,
                        subject,
                        error = %e,
                        "belief-synthesizer: LLM failed; falling back to most-recent fact"
                    );
                    stats.errors += 1;
                    // facts is sorted oldest-first; the tail is most recent.
                    let primary = facts.last().expect("non-empty multi-fact group");
                    (primary.content.clone(), None, false)
                }
            }
        };

        let confidence = compute_confidence(facts, now);
        let valid_from = earliest_valid_from(facts);
        let id = format!("belief-{}", uuid::Uuid::new_v4());

        let source_fact_ids: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();

        // B-4: best-effort embedding for semantic recall. A failure or
        // missing client falls back to `None` — the belief is still
        // useful via exact-subject lookup, just hidden from
        // similarity-driven recall.
        let embedding = self.embed_belief_content(&content).await;

        let belief = Belief {
            id,
            partition_id: partition_id.to_string(),
            subject: subject.to_string(),
            content,
            confidence,
            valid_from,
            valid_until: None,
            source_fact_ids,
            synthesizer_version: SYNTHESIZER_VERSION,
            reasoning,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding,
        };

        self.belief_store.upsert_belief(&belief).await?;

        tracing::debug!(
            partition_id,
            subject,
            used_llm,
            confidence,
            source_count = facts.len(),
            "belief-synthesizer: synthesized"
        );
        Ok(())
    }

    /// Generate the belief's embedding bytes from its content. Returns
    /// `None` when no client is wired or the call fails — synthesis
    /// must NEVER fail because embedding failed (B-4 hard rule). The
    /// bytes are little-endian f32, matching the format expected by
    /// `SqliteBeliefStore::search_beliefs`.
    async fn embed_belief_content(&self, content: &str) -> Option<Vec<u8>> {
        let client = self.embedding_client.as_ref()?;
        match client.embed(&[content]).await {
            Ok(mut v) if !v.is_empty() => Some(embedding_to_bytes(&v.remove(0))),
            Ok(_) => {
                tracing::debug!("belief-synthesizer: embed returned empty vector");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "belief-synthesizer: embed failed; belief saved without embedding");
                None
            }
        }
    }
}

/// Serialize an f32 vector to little-endian bytes for storage on the
/// `kg_beliefs.embedding` column. Mirrors the `embedding_from_bytes`
/// helper in `SqliteBeliefStore` so a round-trip preserves the vector
/// exactly.
pub(crate) fn embedding_to_bytes(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vec.len() * 4);
    for f in vec {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

// ============================================================================
// Helpers (pure functions, testable in isolation)
// ============================================================================

/// Recency weight in the range `(0, 1]`. Facts dated "now" weigh `1.0`;
/// 90-day-old facts weigh `0.5`; 180-day-old facts weigh `0.333`.
pub(crate) fn recency_weight(valid_from: Option<DateTime<Utc>>, now: DateTime<Utc>) -> f64 {
    let vf = match valid_from {
        Some(t) => t,
        None => return 1.0,
    };
    let age_days = (now - vf).num_seconds() as f64 / 86_400.0;
    if age_days <= 0.0 {
        return 1.0;
    }
    1.0 / (1.0 + age_days / RECENCY_HALF_LIFE_DAYS)
}

/// `avg(fact.confidence × recency_weight(fact.valid_from))` across all
/// constituent facts. Returns `0.0` for an empty slice (caller should
/// not invoke with empty input).
pub(crate) fn compute_confidence(facts: &[MemoryFact], now: DateTime<Utc>) -> f64 {
    if facts.is_empty() {
        return 0.0;
    }
    let sum: f64 = facts
        .iter()
        .map(|f| {
            let vf = f.valid_from.as_deref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            });
            f.confidence * recency_weight(vf, now)
        })
        .sum();
    sum / facts.len() as f64
}

/// Earliest `valid_from` across constituents, or `None` if no fact has
/// one. Used to set the belief's interval start so historical queries
/// surface the right slice.
fn earliest_valid_from(facts: &[MemoryFact]) -> Option<DateTime<Utc>> {
    facts
        .iter()
        .filter_map(|f| {
            f.valid_from.as_deref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            })
        })
        .min()
}

// ============================================================================
// LLM-backed implementation
// ============================================================================

/// Production `BeliefSynthesisLlm` wired to the injected `MemoryLlmFactory`.
pub struct LlmBeliefSynthesizer {
    client: CachedLlmClient,
}

impl LlmBeliefSynthesizer {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 256)),
        }
    }

    /// Build the prompt body. Pulled out as a free function so the test
    /// fixture can assert against it without spinning up a real LLM.
    fn build_prompt(subject: &str, facts: &[MemoryFact]) -> String {
        let formatted = facts
            .iter()
            .map(|f| {
                let vf = f.valid_from.as_deref().unwrap_or("unknown");
                format!("- [{vf}] \"{}\" (conf={:.2})", f.content, f.confidence)
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "You synthesize a single belief from N memory facts about a subject.\n\
             \n\
             Subject: {subject}\n\
             Facts (oldest first by valid_from):\n\
             {formatted}\n\
             \n\
             Output JSON only, no prose:\n\
             {{\"content\": \"<one declarative sentence stating the current belief>\", \
             \"reasoning\": \"<one short sentence on which fact(s) dominated>\"}}\n\
             \n\
             Rules:\n\
             - Treat the most-recent VALID fact as primary (newer beats older)\n\
             - If multiple recent facts agree, the belief reinforces that consensus\n\
             - If they conflict, prefer the newer one\n\
             - Be terse: belief content should be ONE sentence",
        )
    }
}

#[async_trait]
impl BeliefSynthesisLlm for LlmBeliefSynthesizer {
    async fn synthesize(
        &self,
        subject: &str,
        facts: &[MemoryFact],
    ) -> Result<SynthesisLlmResponse, String> {
        let client = self.client.get().await?;
        let prompt = Self::build_prompt(subject, facts);
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        parse_llm_json::<SynthesisLlmResponse>(&response.content)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use gateway_services::VaultPaths;
    use std::sync::Mutex as StdMutex;
    use zbot_stores_sqlite::vector_index::{SqliteVecIndex, VectorIndex};
    use zbot_stores_sqlite::{
        GatewayMemoryFactStore, KnowledgeDatabase, MemoryRepository, SqliteBeliefStore,
    };

    struct MockLlm {
        response: StdMutex<Result<SynthesisLlmResponse, String>>,
        calls: StdMutex<u64>,
    }

    impl MockLlm {
        fn ok(content: &str, reasoning: &str) -> Self {
            Self {
                response: StdMutex::new(Ok(SynthesisLlmResponse {
                    content: content.into(),
                    reasoning: reasoning.into(),
                })),
                calls: StdMutex::new(0),
            }
        }

        fn fail() -> Self {
            Self {
                response: StdMutex::new(Err("induced".to_string())),
                calls: StdMutex::new(0),
            }
        }

        fn calls(&self) -> u64 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl BeliefSynthesisLlm for MockLlm {
        async fn synthesize(
            &self,
            _subject: &str,
            _facts: &[MemoryFact],
        ) -> Result<SynthesisLlmResponse, String> {
            *self.calls.lock().unwrap() += 1;
            match &*self.response.lock().unwrap() {
                Ok(r) => Ok(r.clone()),
                Err(e) => Err(e.clone()),
            }
        }
    }

    /// Build a wired synthesizer over a fresh in-memory-ish DB. Returns
    /// the synthesizer + the stores so tests can assert against them.
    fn setup_with_llm(
        llm: Arc<dyn BeliefSynthesisLlm>,
        enabled: bool,
    ) -> (
        BeliefSynthesizer,
        Arc<dyn MemoryFactStore>,
        Arc<dyn BeliefStore>,
        tempfile::TempDir,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        let vec_index: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id")
                .expect("vec index init"),
        );
        let mem_repo = Arc::new(MemoryRepository::new(db.clone(), vec_index));
        let fact_store: Arc<dyn MemoryFactStore> =
            Arc::new(GatewayMemoryFactStore::new(mem_repo, None));
        let belief_store: Arc<dyn BeliefStore> = Arc::new(SqliteBeliefStore::new(db));
        let synth = BeliefSynthesizer::new(
            fact_store.clone(),
            belief_store.clone(),
            llm,
            Duration::ZERO,
            enabled,
        );
        (synth, fact_store, belief_store, tmp)
    }

    async fn seed_fact(
        store: &Arc<dyn MemoryFactStore>,
        partition_id: &str,
        key: &str,
        content: &str,
        confidence: f64,
        valid_from: Option<DateTime<Utc>>,
    ) {
        store
            .save_fact(
                partition_id,
                "user",
                key,
                content,
                confidence,
                None,
                valid_from,
            )
            .await
            .unwrap();
    }

    // ------------------------------------------------------------------
    // Disabled by default — every call is a no-op
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn disabled_synthesizer_returns_empty_stats() {
        let llm = Arc::new(MockLlm::ok("ignored", "ignored"));
        let (synth, fact_store, _belief_store, _tmp) =
            setup_with_llm(llm.clone(), /*enabled=*/ false);
        seed_fact(&fact_store, "ag", "user.name", "Phani", 0.9, None).await;

        let stats = synth.run_cycle("run-disabled", "ag").await.unwrap();
        assert_eq!(stats.subjects_examined, 0);
        assert_eq!(stats.beliefs_synthesized, 0);
        assert_eq!(llm.calls(), 0, "disabled cycle must NOT call the LLM");
    }

    // ------------------------------------------------------------------
    // Short-circuit — single fact, no LLM
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn single_fact_short_circuits_without_llm_call() {
        let llm = Arc::new(MockLlm::ok("SHOULD NOT BE USED", "no"));
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm.clone(), true);

        seed_fact(
            &fact_store,
            "ag",
            "user.location",
            "Mason, OH",
            0.9,
            Some(Utc::now()),
        )
        .await;

        let stats = synth.run_cycle("run-short", "ag").await.unwrap();
        assert_eq!(stats.subjects_examined, 1);
        assert_eq!(stats.beliefs_short_circuited, 1);
        assert_eq!(stats.beliefs_llm_synthesized, 0);
        assert_eq!(llm.calls(), 0, "short-circuit must skip the LLM");

        let got = belief_store
            .get_belief("ag", "user.location", None)
            .await
            .unwrap()
            .expect("belief present");
        assert_eq!(got.content, "Mason, OH", "verbatim fact content");
        assert!(
            got.reasoning.is_none(),
            "short-circuit leaves reasoning NULL"
        );
        assert_eq!(got.source_fact_ids.len(), 1);
    }

    // ------------------------------------------------------------------
    // Multi-fact synthesis — LLM is called and reasoning is persisted
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn multi_fact_calls_llm_and_persists_reasoning() {
        let llm = Arc::new(MockLlm::ok(
            "User goes by Phani",
            "Most recent fact wins; older alias kept as background",
        ));
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm.clone(), true);

        let older = Utc::now() - ChronoDuration::days(120);
        seed_fact(
            &fact_store,
            "ag",
            "user.name",
            "Phanindra",
            0.85,
            Some(older),
        )
        .await;

        // Save second fact with a different KEY (different subject) so we
        // can hit upsert by recreating the same key directly via the
        // typed write path. The save_fact API upserts on (agent,key) so
        // re-saving with the same key would overwrite the first row.
        // Use the typed upsert to bypass that and create a second row.
        let newer = Utc::now();
        // Force a SECOND fact for the SAME (agent, key) by using a
        // typed upsert with a distinct id. The store dedups on
        // (agent_id, scope, ward_id, key), so to model "two facts about
        // the same subject" we use separate scopes — one global, one
        // agent — both with the same key. Belief synthesis groups by
        // ward_id + key regardless of scope, which matches the design
        // doc's notion that the subject is the canonical aggregation
        // key.
        let typed_fact = serde_json::json!({
            "id": format!("fact-{}", uuid::Uuid::new_v4()),
            "session_id": null,
            "agent_id": "ag",
            "scope": "agent",
            "category": "user",
            "key": "user.name",
            "content": "Phani",
            "confidence": 0.9,
            "mention_count": 1,
            "source_summary": null,
            "ward_id": "__global__",
            "contradicted_by": null,
            "created_at": newer.to_rfc3339(),
            "updated_at": newer.to_rfc3339(),
            "expires_at": null,
            "valid_from": newer.to_rfc3339(),
            "valid_until": null,
            "superseded_by": null,
            "pinned": false,
            "epistemic_class": "current",
            "source_episode_id": null,
            "source_ref": null,
        });
        fact_store
            .upsert_typed_fact(typed_fact, None)
            .await
            .unwrap();

        let stats = synth.run_cycle("run-multi", "ag").await.unwrap();
        assert_eq!(stats.subjects_examined, 1);
        assert_eq!(stats.beliefs_llm_synthesized, 1);
        assert_eq!(stats.beliefs_short_circuited, 0);
        assert_eq!(llm.calls(), 1, "multi-fact must call the LLM once");

        let got = belief_store
            .get_belief("ag", "user.name", None)
            .await
            .unwrap()
            .expect("belief present");
        assert_eq!(got.content, "User goes by Phani");
        assert!(got.reasoning.as_deref().unwrap_or("").contains("recent"));
        assert_eq!(got.source_fact_ids.len(), 2);
    }

    // ------------------------------------------------------------------
    // LLM failure → fall back to most-recent fact verbatim
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn multi_fact_llm_failure_falls_back_to_most_recent() {
        let llm = Arc::new(MockLlm::fail());
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm.clone(), true);

        let older = Utc::now() - ChronoDuration::days(60);
        seed_fact(
            &fact_store,
            "ag",
            "user.name",
            "Old Name",
            0.85,
            Some(older),
        )
        .await;
        let newer = Utc::now();
        let typed_fact = serde_json::json!({
            "id": format!("fact-{}", uuid::Uuid::new_v4()),
            "session_id": null,
            "agent_id": "ag",
            "scope": "agent",
            "category": "user",
            "key": "user.name",
            "content": "Most Recent Name",
            "confidence": 0.9,
            "mention_count": 1,
            "source_summary": null,
            "ward_id": "__global__",
            "contradicted_by": null,
            "created_at": newer.to_rfc3339(),
            "updated_at": newer.to_rfc3339(),
            "expires_at": null,
            "valid_from": newer.to_rfc3339(),
            "valid_until": null,
            "superseded_by": null,
            "pinned": false,
            "epistemic_class": "current",
            "source_episode_id": null,
            "source_ref": null,
        });
        fact_store
            .upsert_typed_fact(typed_fact, None)
            .await
            .unwrap();

        let stats = synth.run_cycle("run-fail", "ag").await.unwrap();
        assert_eq!(llm.calls(), 1, "LLM was attempted");
        assert_eq!(stats.errors, 1, "fallback path increments errors");

        let got = belief_store
            .get_belief("ag", "user.name", None)
            .await
            .unwrap()
            .expect("belief present despite LLM failure");
        assert_eq!(
            got.content, "Most Recent Name",
            "fallback uses the most-recent fact verbatim"
        );
        assert!(got.reasoning.is_none(), "fallback leaves reasoning NULL");
    }

    // ------------------------------------------------------------------
    // Confidence formula — recency-weighted average
    // ------------------------------------------------------------------

    #[test]
    fn confidence_formula_single_fact_90_days_old_is_about_half() {
        // fact at 0.9, valid_from = 90 days ago → expect ~ 0.45
        let now = Utc::now();
        let vf = now - ChronoDuration::days(90);
        let f = MemoryFact {
            id: "f1".into(),
            session_id: None,
            agent_id: "ag".into(),
            scope: "agent".into(),
            category: "user".into(),
            key: "user.x".into(),
            content: "c".into(),
            confidence: 0.9,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "__global__".into(),
            contradicted_by: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            expires_at: None,
            valid_from: Some(vf.to_rfc3339()),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".into()),
            source_episode_id: None,
            source_ref: None,
        };
        let c = compute_confidence(&[f], now);
        assert!(
            (c - 0.45).abs() < 0.01,
            "expected confidence ~0.45 (0.9 × 0.5), got {c}"
        );
    }

    #[test]
    fn recency_weight_now_is_one() {
        let now = Utc::now();
        let w = recency_weight(Some(now), now);
        assert!((w - 1.0).abs() < 1e-9);
    }

    #[test]
    fn recency_weight_missing_valid_from_is_one() {
        let now = Utc::now();
        let w = recency_weight(None, now);
        assert!((w - 1.0).abs() < 1e-9);
    }

    // ------------------------------------------------------------------
    // Re-synthesis is idempotent — running twice produces same belief
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn re_running_synthesis_is_idempotent() {
        let llm = Arc::new(MockLlm::ok("ignored", "ignored"));
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm, true);
        let vf = Utc::now();
        seed_fact(
            &fact_store,
            "ag",
            "user.location",
            "Mason, OH",
            0.9,
            Some(vf),
        )
        .await;

        synth.run_cycle("run-1", "ag").await.unwrap();
        synth.run_cycle("run-2", "ag").await.unwrap();

        // One belief for the subject, not two — upsert key is
        // (partition_id, subject, valid_from).
        let listed = belief_store.list_beliefs("ag", 10).await.unwrap();
        assert_eq!(
            listed.len(),
            1,
            "second cycle must not create a duplicate row"
        );
        assert_eq!(listed[0].subject, "user.location");
    }

    // ------------------------------------------------------------------
    // B-3: Synthesizer picks up stale beliefs first and clears the flag.
    // ------------------------------------------------------------------

    /// A stale belief is re-synthesized at the top of the cycle and its
    /// stale flag is cleared. The fact set is unchanged, so the
    /// content stays the same — what we verify is the prioritization
    /// pathway (stats track stale_beliefs_resynthesized > 0 and the
    /// flag transitions from true → false).
    #[tokio::test]
    async fn synthesizer_picks_up_stale_and_clears_flag() {
        let llm = Arc::new(MockLlm::ok("ignored", "ignored"));
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm, true);
        let vf = Utc::now();
        seed_fact(
            &fact_store,
            "ag",
            "user.location",
            "Mason, OH",
            0.9,
            Some(vf),
        )
        .await;

        // First cycle: synthesize the belief. Then mark it stale.
        synth.run_cycle("setup", "ag").await.unwrap();
        let listed = belief_store.list_beliefs("ag", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        let belief_id = listed[0].id.clone();
        belief_store.mark_stale(&belief_id).await.unwrap();
        assert!(belief_store.list_beliefs("ag", 10).await.unwrap()[0].stale);

        // Second cycle: stale belief is picked up FIRST.
        let stats = synth.run_cycle("resynth", "ag").await.unwrap();
        assert!(
            stats.stale_beliefs_resynthesized >= 1,
            "stale resynth path must fire; got stats={stats:?}"
        );

        // Flag is cleared after the cycle.
        let after = belief_store.list_beliefs("ag", 10).await.unwrap();
        assert!(!after[0].stale, "synthesizer must clear stale flag");
    }

    /// When both a stale belief and a fresh subject exist, the stale
    /// belief is processed FIRST (stats reflect the stale pathway), and
    /// the normal pass still covers the other subject in the same cycle.
    #[tokio::test]
    async fn synthesizer_prioritizes_stale_before_dirty() {
        let llm = Arc::new(MockLlm::ok("ignored", "ignored"));
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm, true);

        // Two subjects, each with a single fact. First cycle synthesizes both.
        seed_fact(
            &fact_store,
            "ag",
            "user.location",
            "Mason, OH",
            0.9,
            Some(Utc::now()),
        )
        .await;
        seed_fact(
            &fact_store,
            "ag",
            "user.name",
            "Phani",
            0.9,
            Some(Utc::now()),
        )
        .await;
        synth.run_cycle("setup", "ag").await.unwrap();
        assert_eq!(belief_store.list_beliefs("ag", 10).await.unwrap().len(), 2);

        // Mark one of the beliefs stale.
        let listed = belief_store.list_beliefs("ag", 10).await.unwrap();
        let target = listed
            .iter()
            .find(|b| b.subject == "user.location")
            .unwrap();
        belief_store.mark_stale(&target.id).await.unwrap();

        // Re-run. Stale subject is re-synthesized AND the other subject
        // still gets its normal pass — stats reflect both.
        let stats = synth.run_cycle("mixed", "ag").await.unwrap();
        assert!(
            stats.stale_beliefs_resynthesized >= 1,
            "stale path must fire: {stats:?}"
        );
        assert!(
            stats.beliefs_synthesized >= 1,
            "normal pass also runs: {stats:?}"
        );
    }

    // ------------------------------------------------------------------
    // B-4: embedding generation at synthesis time
    // ------------------------------------------------------------------

    use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};

    /// Embedding client that returns a fixed 3-dim vector for every
    /// call. Counts invocations so tests can confirm the synthesizer
    /// actually hits it.
    struct ConstEmbed {
        calls: StdMutex<u64>,
    }

    impl ConstEmbed {
        fn new() -> Self {
            Self {
                calls: StdMutex::new(0),
            }
        }
        fn calls(&self) -> u64 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl EmbeddingClient for ConstEmbed {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            *self.calls.lock().unwrap() += 1;
            Ok(texts.iter().map(|_| vec![1.0_f32, 0.5, -0.25]).collect())
        }
        fn dimensions(&self) -> usize {
            3
        }
        fn model_name(&self) -> String {
            "const-embed-3".to_string()
        }
    }

    /// Embedding client that always errors — used to verify the
    /// best-effort fallback (B-4 hard rule: embedding failure must not
    /// abort synthesis).
    struct FailEmbed;
    #[async_trait]
    impl EmbeddingClient for FailEmbed {
        async fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            Err(EmbeddingError::ModelError("induced failure".to_string()))
        }
        fn dimensions(&self) -> usize {
            3
        }
        fn model_name(&self) -> String {
            "fail-embed".to_string()
        }
    }

    /// Single-fact short-circuit must still generate an embedding.
    /// Confirms the embed path runs even on the optimized path that
    /// skips the LLM.
    #[tokio::test]
    async fn short_circuit_path_generates_embedding() {
        let llm = Arc::new(MockLlm::ok("SHOULD NOT BE USED", "no"));
        let embedder = Arc::new(ConstEmbed::new());
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm.clone(), true);
        let synth = synth.with_embedding_client(Some(embedder.clone()));

        seed_fact(
            &fact_store,
            "ag",
            "user.location",
            "Mason, OH",
            0.9,
            Some(Utc::now()),
        )
        .await;

        let stats = synth.run_cycle("run-short-emb", "ag").await.unwrap();
        assert_eq!(stats.beliefs_short_circuited, 1);
        assert_eq!(
            embedder.calls(),
            1,
            "embedder called once even on short-circuit path"
        );
        let got = belief_store
            .get_belief("ag", "user.location", None)
            .await
            .unwrap()
            .expect("belief present");
        let bytes = got.embedding.expect("embedding bytes populated");
        assert_eq!(bytes.len(), 12, "3 × f32 = 12 bytes");
    }

    /// Multi-fact LLM path also generates an embedding for the
    /// synthesized belief content.
    #[tokio::test]
    async fn multi_fact_path_generates_embedding() {
        let llm = Arc::new(MockLlm::ok("User goes by Phani", "newer beats older"));
        let embedder = Arc::new(ConstEmbed::new());
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm.clone(), true);
        let synth = synth.with_embedding_client(Some(embedder.clone()));

        // Seed two facts for the same subject — forces multi-fact path.
        let older = Utc::now() - ChronoDuration::days(120);
        seed_fact(
            &fact_store,
            "ag",
            "user.name",
            "Phanindra",
            0.85,
            Some(older),
        )
        .await;
        let newer = Utc::now();
        let typed_fact = serde_json::json!({
            "id": format!("fact-{}", uuid::Uuid::new_v4()),
            "session_id": null,
            "agent_id": "ag",
            "scope": "agent",
            "category": "user",
            "key": "user.name",
            "content": "Phani",
            "confidence": 0.9,
            "mention_count": 1,
            "source_summary": null,
            "ward_id": "__global__",
            "contradicted_by": null,
            "created_at": newer.to_rfc3339(),
            "updated_at": newer.to_rfc3339(),
            "expires_at": null,
            "valid_from": newer.to_rfc3339(),
            "valid_until": null,
            "superseded_by": null,
            "pinned": false,
            "epistemic_class": "current",
            "source_episode_id": null,
            "source_ref": null,
        });
        fact_store
            .upsert_typed_fact(typed_fact, None)
            .await
            .unwrap();

        let stats = synth.run_cycle("run-multi-emb", "ag").await.unwrap();
        assert_eq!(stats.beliefs_llm_synthesized, 1);
        assert_eq!(embedder.calls(), 1, "embedder called once on multi-fact");
        let got = belief_store
            .get_belief("ag", "user.name", None)
            .await
            .unwrap()
            .expect("belief present");
        assert!(
            got.embedding.is_some(),
            "embedding populated on multi-fact path"
        );
    }

    /// Embedder failure must NOT abort synthesis — the belief is
    /// written with `embedding = None` and the cycle continues. B-4
    /// hard rule.
    #[tokio::test]
    async fn embed_failure_falls_back_to_null_embedding() {
        let llm = Arc::new(MockLlm::ok("ignored", "ignored"));
        let embedder = Arc::new(FailEmbed);
        let (synth, fact_store, belief_store, _tmp) = setup_with_llm(llm.clone(), true);
        let synth = synth.with_embedding_client(Some(embedder.clone()));

        seed_fact(
            &fact_store,
            "ag",
            "user.x",
            "anything",
            0.9,
            Some(Utc::now()),
        )
        .await;

        let stats = synth.run_cycle("run-embfail", "ag").await.unwrap();
        // Synthesis still succeeds.
        assert_eq!(stats.beliefs_short_circuited, 1);
        // Belief exists; embedding is None.
        let got = belief_store
            .get_belief("ag", "user.x", None)
            .await
            .unwrap()
            .expect("belief present despite embed failure");
        assert!(
            got.embedding.is_none(),
            "embed failure leaves embedding NULL"
        );
    }
}
