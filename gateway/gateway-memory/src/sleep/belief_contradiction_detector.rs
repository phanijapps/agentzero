//! Belief Contradiction Detector — sleep-time worker (Phase B-2 of the
//! Belief Network).
//!
//! Pairs up beliefs in the same "topical neighborhood" (subject-prefix
//! match, NOT KG-edge traversal — see design doc 2026-05-15) and asks
//! the LLM judge whether they contradict.
//!
//! Decisions:
//! - `logical_contradiction` → insert a `kg_belief_contradictions` row
//!   with `contradiction_type='logical'`.
//! - `tension` → insert with `contradiction_type='tension'`.
//! - `duplicate` → log at INFO; no row (auto-merge is a future phase).
//! - `compatible` → log at DEBUG; no row.
//!
//! Pair canonicalization happens at the store layer — the detector just
//! passes the pair in whatever order the iteration produced.
//!
//! Budget: per-cycle cap on LLM calls (default 20). Pairs that already
//! have a row are skipped without invoking the LLM. Neighborhoods are
//! processed largest-first so the budget is spent where contradictions
//! are most likely.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use zbot_stores_domain::{Belief, BeliefContradiction, ContradictionType};
use zbot_stores_traits::{BeliefContradictionStore, BeliefStore};

use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

/// Hard ceiling on beliefs scanned per cycle. Mirrors the synthesizer's
/// `MAX_FACTS_PER_CYCLE` — large enough for normal usage, small enough
/// to keep the SQLite scan bounded.
const MAX_BELIEFS_PER_CYCLE: usize = 1000;

/// Tuning + opt-in switches for the detector.
#[derive(Debug, Clone)]
pub struct BeliefContradictionConfig {
    /// Master switch — when false, every `run_cycle` call returns
    /// immediately with empty stats. Mirrors the B-1 pattern.
    pub enabled: bool,
    /// How many dot-separated subject components form a neighborhood
    /// key. `1` → "user.dietary.x" + "user.preferences.y" share the
    /// "user" neighborhood. `2` → finer-grained.
    pub neighborhood_prefix_depth: usize,
    /// Maximum LLM calls (i.e. unique pair evaluations) per cycle.
    pub budget_per_cycle: usize,
}

impl Default for BeliefContradictionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            neighborhood_prefix_depth: 1,
            budget_per_cycle: 20,
        }
    }
}

/// One-cycle stats. Tracked separately by decision so we can see at a
/// glance which branch the LLM is taking on real data.
#[derive(Debug, Default, Clone)]
pub struct ContradictionDetectionStats {
    pub neighborhoods_examined: u64,
    pub pairs_examined: u64,
    pub pairs_skipped_existing: u64,
    pub llm_calls: u64,
    pub contradictions_logical: u64,
    pub contradictions_tension: u64,
    pub duplicates_logged: u64,
    pub compatibles_logged: u64,
    pub errors: u64,
    pub budget_exhausted: bool,
}

/// 4-way LLM judge decision. Matches the prompt's `decision` enum.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JudgeDecision {
    LogicalContradiction,
    Tension,
    Compatible,
    Duplicate,
}

/// Parsed LLM judge response.
#[derive(Debug, Clone, Deserialize)]
pub struct ContradictionJudgeResponse {
    pub decision: JudgeDecision,
    pub severity: f64,
    pub reasoning: String,
}

/// LLM abstraction so tests can inject a mock without a real model.
#[async_trait]
pub trait ContradictionJudgeLlm: Send + Sync {
    async fn judge(&self, a: &Belief, b: &Belief) -> Result<ContradictionJudgeResponse, String>;
}

/// Sleep-time worker that scans beliefs and detects contradictions.
pub struct BeliefContradictionDetector {
    belief_store: Arc<dyn BeliefStore>,
    contradiction_store: Arc<dyn BeliefContradictionStore>,
    llm: Arc<dyn ContradictionJudgeLlm>,
    config: BeliefContradictionConfig,
    interval: Duration,
    last_run: Mutex<Option<Instant>>,
}

impl BeliefContradictionDetector {
    pub fn new(
        belief_store: Arc<dyn BeliefStore>,
        contradiction_store: Arc<dyn BeliefContradictionStore>,
        llm: Arc<dyn ContradictionJudgeLlm>,
        config: BeliefContradictionConfig,
        interval: Duration,
    ) -> Self {
        Self {
            belief_store,
            contradiction_store,
            llm,
            config,
            interval,
            last_run: Mutex::new(None),
        }
    }

    /// Run one detection cycle for a partition.
    ///
    /// Conservative: per-pair errors are logged + counted; the cycle
    /// never fails hard. Returns aggregate stats so the orchestrator
    /// can surface them.
    pub async fn run_cycle(
        &self,
        run_id: &str,
        partition_id: &str,
    ) -> Result<ContradictionDetectionStats, String> {
        if !self.config.enabled {
            return Ok(ContradictionDetectionStats::default());
        }
        if !self.interval_elapsed() {
            return Ok(ContradictionDetectionStats::default());
        }

        let mut stats = ContradictionDetectionStats::default();

        let beliefs = self
            .belief_store
            .list_beliefs(partition_id, MAX_BELIEFS_PER_CYCLE)
            .await
            .map_err(|e| format!("list_beliefs: {e}"))?;

        // Filter superseded beliefs — they're history, not the current
        // stance. Mirrors `BeliefSynthesizer::run_cycle`'s active-set
        // selection.
        let active: Vec<Belief> = beliefs
            .into_iter()
            .filter(|b| b.superseded_by.is_none())
            .collect();

        // Group by neighborhood key (subject-prefix). Largest groups
        // first → most contradiction potential per LLM call.
        let mut by_neighborhood: HashMap<String, Vec<Belief>> = HashMap::new();
        for b in active {
            let key = self.neighborhood_key(&b.subject);
            by_neighborhood.entry(key).or_default().push(b);
        }
        stats.neighborhoods_examined = by_neighborhood.len() as u64;

        let mut groups: Vec<(String, Vec<Belief>)> = by_neighborhood.into_iter().collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        let mut budget_remaining = self.config.budget_per_cycle;

        for (_nbhd, group) in groups {
            if budget_remaining == 0 {
                stats.budget_exhausted = true;
                break;
            }
            self.process_neighborhood(
                run_id,
                partition_id,
                &group,
                &mut stats,
                &mut budget_remaining,
            )
            .await;
        }

        if budget_remaining == 0 && !stats.budget_exhausted {
            // Edge: budget hit zero on the last pair of the last group.
            stats.budget_exhausted = true;
        }

        *self.last_run.lock().unwrap() = Some(Instant::now());
        tracing::info!(
            run_id,
            partition_id,
            neighborhoods = stats.neighborhoods_examined,
            pairs = stats.pairs_examined,
            llm_calls = stats.llm_calls,
            logical = stats.contradictions_logical,
            tension = stats.contradictions_tension,
            duplicates = stats.duplicates_logged,
            compatibles = stats.compatibles_logged,
            errors = stats.errors,
            budget_exhausted = stats.budget_exhausted,
            "belief-contradiction-detector: cycle done"
        );
        Ok(stats)
    }

    /// Compute the neighborhood key for a subject. Pure helper — tests
    /// exercise it directly.
    pub fn neighborhood_key(&self, subject: &str) -> String {
        subject
            .split('.')
            .take(self.config.neighborhood_prefix_depth)
            .collect::<Vec<_>>()
            .join(".")
    }

    fn interval_elapsed(&self) -> bool {
        if self.interval.is_zero() {
            return true;
        }
        match *self.last_run.lock().unwrap() {
            Some(last) => last.elapsed() >= self.interval,
            None => true,
        }
    }

    /// Enumerate unordered pairs in a neighborhood and judge each one,
    /// respecting the per-cycle budget. Failures degrade gracefully.
    async fn process_neighborhood(
        &self,
        run_id: &str,
        partition_id: &str,
        group: &[Belief],
        stats: &mut ContradictionDetectionStats,
        budget_remaining: &mut usize,
    ) {
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                if *budget_remaining == 0 {
                    stats.budget_exhausted = true;
                    return;
                }
                stats.pairs_examined += 1;
                let a = &group[i];
                let b = &group[j];

                // Skip already-evaluated pairs without an LLM call.
                match self.contradiction_store.pair_exists(&a.id, &b.id).await {
                    Ok(true) => {
                        stats.pairs_skipped_existing += 1;
                        continue;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        stats.errors += 1;
                        tracing::warn!(
                            run_id,
                            partition_id,
                            error = %e,
                            "belief-contradiction-detector: pair_exists failed; skipping pair"
                        );
                        continue;
                    }
                }

                stats.llm_calls += 1;
                *budget_remaining = budget_remaining.saturating_sub(1);

                self.judge_and_route(run_id, partition_id, a, b, stats)
                    .await;
            }
        }
    }

    /// Call the LLM judge and route the decision. All errors are absorbed
    /// (logged + counted) so the cycle continues.
    async fn judge_and_route(
        &self,
        run_id: &str,
        partition_id: &str,
        a: &Belief,
        b: &Belief,
        stats: &mut ContradictionDetectionStats,
    ) {
        let resp = match self.llm.judge(a, b).await {
            Ok(r) => r,
            Err(e) => {
                stats.errors += 1;
                tracing::warn!(
                    run_id,
                    partition_id,
                    belief_a_id = %a.id,
                    belief_b_id = %b.id,
                    error = %e,
                    "belief-contradiction-detector: LLM failed; routing as compatible"
                );
                return;
            }
        };

        match resp.decision {
            JudgeDecision::LogicalContradiction => {
                stats.contradictions_logical += 1;
                self.insert_row(
                    run_id,
                    partition_id,
                    a,
                    b,
                    ContradictionType::Logical,
                    resp.severity,
                    resp.reasoning,
                    stats,
                )
                .await;
            }
            JudgeDecision::Tension => {
                stats.contradictions_tension += 1;
                self.insert_row(
                    run_id,
                    partition_id,
                    a,
                    b,
                    ContradictionType::Tension,
                    resp.severity,
                    resp.reasoning,
                    stats,
                )
                .await;
            }
            JudgeDecision::Duplicate => {
                stats.duplicates_logged += 1;
                tracing::info!(
                    run_id,
                    partition_id,
                    belief_a_id = %a.id,
                    belief_b_id = %b.id,
                    reasoning = %resp.reasoning,
                    "belief-contradiction-detector: duplicate beliefs (canonicalization signal)"
                );
            }
            JudgeDecision::Compatible => {
                stats.compatibles_logged += 1;
                tracing::debug!(
                    run_id,
                    partition_id,
                    belief_a_id = %a.id,
                    belief_b_id = %b.id,
                    "belief-contradiction-detector: beliefs compatible"
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_row(
        &self,
        run_id: &str,
        partition_id: &str,
        a: &Belief,
        b: &Belief,
        t: ContradictionType,
        severity: f64,
        reasoning: String,
        stats: &mut ContradictionDetectionStats,
    ) {
        let row = BeliefContradiction {
            id: format!("contradiction-{}", uuid::Uuid::new_v4()),
            belief_a_id: a.id.clone(),
            belief_b_id: b.id.clone(),
            contradiction_type: t,
            severity,
            judge_reasoning: Some(reasoning),
            detected_at: Utc::now(),
            resolved_at: None,
            resolution: None,
        };
        if let Err(e) = self.contradiction_store.insert_contradiction(&row).await {
            stats.errors += 1;
            tracing::warn!(
                run_id,
                partition_id,
                belief_a_id = %a.id,
                belief_b_id = %b.id,
                error = %e,
                "belief-contradiction-detector: insert failed"
            );
        }
    }
}

// ============================================================================
// Production LLM-backed judge
// ============================================================================

/// Production `ContradictionJudgeLlm` wired to the injected `MemoryLlmFactory`.
pub struct LlmContradictionJudge {
    client: CachedLlmClient,
}

impl LlmContradictionJudge {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 256)),
        }
    }

    /// Build the judge prompt. Pulled out as a free function so tests
    /// can assert against it without spinning up a real LLM.
    pub(crate) fn build_prompt(a: &Belief, b: &Belief) -> String {
        let source_count_a = a.source_fact_ids.len();
        let source_count_b = b.source_fact_ids.len();
        format!(
            "You judge whether two beliefs about a similar subject contradict.\n\
             \n\
             Belief A:\n\
             - Subject: {subj_a}\n\
             - Content: \"{content_a}\"\n\
             - Confidence: {conf_a:.2}\n\
             - Source fact count: {source_count_a}\n\
             \n\
             Belief B:\n\
             - Subject: {subj_b}\n\
             - Content: \"{content_b}\"\n\
             - Confidence: {conf_b:.2}\n\
             - Source fact count: {source_count_b}\n\
             \n\
             Output JSON only, no prose:\n\
             {{\"decision\": \"logical_contradiction\" | \"tension\" | \"compatible\" | \"duplicate\", \
             \"severity\": <0.0..1.0>, \
             \"reasoning\": \"<one short sentence>\"}}\n\
             \n\
             Rules:\n\
             - \"logical_contradiction\": A and B cannot both be true at the same time. \
             Example: different current employers, different \"lives in\" cities.\n\
             - \"tension\": Different facets of the same subject; could both be true with context. \
             Example: \"prefers dark mode\" + \"prefers light mode\" (context-dependent).\n\
             - \"compatible\": About different things, or fully consistent statements that don't conflict.\n\
             - \"duplicate\": Same content meaning, different subject key naming. Canonicalization signal.\n\
             - severity = your confidence in the classification (NOT severity of disagreement). \
             Low severity = unsure.\n\
             \n\
             Example:\n\
             Belief A: subject=\"user.employment\", content=\"User works at OpenAI\"\n\
             Belief B: subject=\"user.employment\", content=\"User works at Anthropic\"\n\
             Output: {{\"decision\": \"logical_contradiction\", \"severity\": 0.95, \
             \"reasoning\": \"Two different current employers cannot both be true.\"}}",
            subj_a = a.subject,
            content_a = a.content,
            conf_a = a.confidence,
            subj_b = b.subject,
            content_b = b.content,
            conf_b = b.confidence,
        )
    }
}

#[async_trait]
impl ContradictionJudgeLlm for LlmContradictionJudge {
    async fn judge(&self, a: &Belief, b: &Belief) -> Result<ContradictionJudgeResponse, String> {
        let client = self.client.get().await?;
        let prompt = Self::build_prompt(a, b);
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        parse_llm_json::<ContradictionJudgeResponse>(&response.content)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::VaultPaths;
    use std::sync::Mutex as StdMutex;
    use zbot_stores_sqlite::{
        KnowledgeDatabase, SqliteBeliefContradictionStore, SqliteBeliefStore,
    };

    /// Mock LLM judge — returns a queue of canned responses (front is
    /// next), or a single response repeated. `calls` counter exposes
    /// the number of times the judge was actually invoked.
    struct MockJudge {
        responses: StdMutex<Vec<Result<ContradictionJudgeResponse, String>>>,
        calls: StdMutex<u64>,
    }

    impl MockJudge {
        fn single(resp: ContradictionJudgeResponse) -> Self {
            Self {
                responses: StdMutex::new(vec![Ok(resp)]),
                calls: StdMutex::new(0),
            }
        }

        fn err() -> Self {
            Self {
                responses: StdMutex::new(vec![Err("induced".to_string())]),
                calls: StdMutex::new(0),
            }
        }

        fn calls(&self) -> u64 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl ContradictionJudgeLlm for MockJudge {
        async fn judge(
            &self,
            _a: &Belief,
            _b: &Belief,
        ) -> Result<ContradictionJudgeResponse, String> {
            *self.calls.lock().unwrap() += 1;
            // If multiple responses queued, pop the front; otherwise
            // return the single response repeatedly.
            let mut guard = self.responses.lock().unwrap();
            if guard.len() > 1 {
                guard.remove(0)
            } else {
                guard[0].clone()
            }
        }
    }

    fn judge_ok(decision: JudgeDecision) -> ContradictionJudgeResponse {
        ContradictionJudgeResponse {
            decision,
            severity: 0.9,
            reasoning: "test reasoning".to_string(),
        }
    }

    fn config(enabled: bool, depth: usize, budget: usize) -> BeliefContradictionConfig {
        BeliefContradictionConfig {
            enabled,
            neighborhood_prefix_depth: depth,
            budget_per_cycle: budget,
        }
    }

    /// Build a fully wired detector + the live stores so tests can seed
    /// beliefs and assert on the contradiction table.
    fn setup(
        judge: Arc<dyn ContradictionJudgeLlm>,
        cfg: BeliefContradictionConfig,
    ) -> (
        BeliefContradictionDetector,
        Arc<dyn BeliefStore>,
        Arc<dyn BeliefContradictionStore>,
        tempfile::TempDir,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        let belief_store: Arc<dyn BeliefStore> = Arc::new(SqliteBeliefStore::new(db.clone()));
        let contradiction_store: Arc<dyn BeliefContradictionStore> =
            Arc::new(SqliteBeliefContradictionStore::new(db));
        let detector = BeliefContradictionDetector::new(
            belief_store.clone(),
            contradiction_store.clone(),
            judge,
            cfg,
            Duration::ZERO,
        );
        (detector, belief_store, contradiction_store, tmp)
    }

    async fn seed(
        store: &Arc<dyn BeliefStore>,
        id: &str,
        subject: &str,
        content: &str,
        partition: &str,
    ) {
        let now = Utc::now();
        let b = Belief {
            id: id.to_string(),
            partition_id: partition.to_string(),
            subject: subject.to_string(),
            content: content.to_string(),
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
        store.upsert_belief(&b).await.unwrap();
    }

    // ------------------------------------------------------------------
    // neighborhood_key — depth controls how many prefix components stay.
    // ------------------------------------------------------------------

    #[test]
    fn neighborhood_key_respects_depth() {
        let (det1, _b, _c, _tmp) = setup(
            Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible))),
            config(false, 1, 20),
        );
        assert_eq!(det1.neighborhood_key("user.dietary.vegetarian"), "user");
        assert_eq!(
            det1.neighborhood_key("domain.finance.acn.valuation"),
            "domain"
        );

        let (det2, _b, _c, _tmp) = setup(
            Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible))),
            config(false, 2, 20),
        );
        assert_eq!(
            det2.neighborhood_key("user.dietary.vegetarian"),
            "user.dietary"
        );
        assert_eq!(
            det2.neighborhood_key("domain.finance.acn.valuation"),
            "domain.finance"
        );

        let (det99, _b, _c, _tmp) = setup(
            Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible))),
            config(false, 99, 20),
        );
        assert_eq!(
            det99.neighborhood_key("user.dietary.vegetarian"),
            "user.dietary.vegetarian"
        );
        assert_eq!(det99.neighborhood_key("single"), "single");
    }

    // ------------------------------------------------------------------
    // Disabled-by-default — cycle returns immediately, no LLM calls.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn disabled_detector_returns_empty_stats() {
        let judge = Arc::new(MockJudge::single(judge_ok(
            JudgeDecision::LogicalContradiction,
        )));
        let (det, beliefs, _c, _tmp) = setup(judge.clone(), config(false, 1, 20));
        seed(&beliefs, "b-1", "user.x", "x", "p").await;
        seed(&beliefs, "b-2", "user.y", "y", "p").await;
        let stats = det.run_cycle("run-disabled", "p").await.unwrap();
        assert_eq!(stats.pairs_examined, 0);
        assert_eq!(stats.llm_calls, 0);
        assert_eq!(judge.calls(), 0);
    }

    // ------------------------------------------------------------------
    // Neighborhood grouping — beliefs in different prefixes don't pair.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn groups_beliefs_into_neighborhoods_and_only_pairs_within() {
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let (det, beliefs, _c, _tmp) = setup(judge.clone(), config(true, 1, 20));
        // 2 in "user" — should produce 1 pair.
        seed(&beliefs, "b-u1", "user.location", "Mason", "p").await;
        seed(&beliefs, "b-u2", "user.employer", "ACN", "p").await;
        // 1 in "domain" — should produce 0 pairs (singleton).
        seed(&beliefs, "b-d1", "domain.finance.x", "...", "p").await;

        let stats = det.run_cycle("run-nbhd", "p").await.unwrap();
        assert_eq!(
            stats.neighborhoods_examined, 2,
            "two distinct top-level prefixes"
        );
        assert_eq!(
            stats.pairs_examined, 1,
            "only the 2-element neighborhood produces a pair"
        );
        assert_eq!(judge.calls(), 1);
    }

    // ------------------------------------------------------------------
    // LLM logical_contradiction → row with type=logical.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn logical_contradiction_inserts_row() {
        let judge = Arc::new(MockJudge::single(judge_ok(
            JudgeDecision::LogicalContradiction,
        )));
        let (det, beliefs, contradictions, _tmp) = setup(judge, config(true, 1, 20));
        seed(&beliefs, "b-1", "user.employer", "OpenAI", "p").await;
        seed(&beliefs, "b-2", "user.employer", "Anthropic", "p").await;

        let stats = det.run_cycle("run", "p").await.unwrap();
        assert_eq!(stats.contradictions_logical, 1);
        assert_eq!(stats.contradictions_tension, 0);

        let listed = contradictions.list_recent("p", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].contradiction_type, ContradictionType::Logical);
    }

    // ------------------------------------------------------------------
    // LLM tension → row with type=tension.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn tension_inserts_row_with_correct_type() {
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Tension)));
        let (det, beliefs, contradictions, _tmp) = setup(judge, config(true, 1, 20));
        seed(&beliefs, "b-1", "user.preferences.theme", "dark", "p").await;
        seed(&beliefs, "b-2", "user.preferences.theme", "light", "p").await;

        let stats = det.run_cycle("run", "p").await.unwrap();
        assert_eq!(stats.contradictions_tension, 1);
        assert_eq!(stats.contradictions_logical, 0);

        let listed = contradictions.list_recent("p", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].contradiction_type, ContradictionType::Tension);
    }

    // ------------------------------------------------------------------
    // LLM duplicate → no row, stats counter ticks.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn duplicate_logs_no_row_inserted() {
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Duplicate)));
        let (det, beliefs, contradictions, _tmp) = setup(judge, config(true, 1, 20));
        seed(&beliefs, "b-1", "user.name", "Phani", "p").await;
        seed(&beliefs, "b-2", "user.full_name", "Phani", "p").await;

        let stats = det.run_cycle("run", "p").await.unwrap();
        assert_eq!(stats.duplicates_logged, 1);
        let listed = contradictions.list_recent("p", 10).await.unwrap();
        assert!(listed.is_empty(), "duplicate must NOT insert a row");
    }

    // ------------------------------------------------------------------
    // LLM compatible → no row, no insertion.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn compatible_logs_no_row_inserted() {
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let (det, beliefs, contradictions, _tmp) = setup(judge, config(true, 1, 20));
        seed(&beliefs, "b-1", "user.x", "alpha", "p").await;
        seed(&beliefs, "b-2", "user.y", "beta", "p").await;

        let stats = det.run_cycle("run", "p").await.unwrap();
        assert_eq!(stats.compatibles_logged, 1);
        let listed = contradictions.list_recent("p", 10).await.unwrap();
        assert!(listed.is_empty());
    }

    // ------------------------------------------------------------------
    // Budget exhaustion — only `budget` LLM calls, regardless of pairs.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn budget_exhaustion_caps_llm_calls() {
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let (det, beliefs, _c, _tmp) = setup(judge.clone(), config(true, 1, 2));
        // 5 beliefs in one neighborhood → C(5,2) = 10 unique pairs.
        for i in 0..5 {
            seed(&beliefs, &format!("b-{i}"), &format!("user.k{i}"), "v", "p").await;
        }

        let stats = det.run_cycle("run-budget", "p").await.unwrap();
        assert_eq!(stats.llm_calls, 2, "budget caps LLM calls at 2");
        assert!(stats.budget_exhausted, "budget_exhausted flag set");
        assert_eq!(judge.calls(), 2);
    }

    // ------------------------------------------------------------------
    // Already-evaluated pair — pair_exists short-circuits the LLM.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn skips_already_evaluated_pairs() {
        let judge = Arc::new(MockJudge::single(judge_ok(
            JudgeDecision::LogicalContradiction,
        )));
        let (det, beliefs, contradictions, _tmp) = setup(judge.clone(), config(true, 1, 20));
        seed(&beliefs, "b-1", "user.employer", "X", "p").await;
        seed(&beliefs, "b-2", "user.employer", "Y", "p").await;

        // Pre-seed a contradiction row so pair_exists returns true.
        contradictions
            .insert_contradiction(&BeliefContradiction {
                id: "pre-existing".to_string(),
                belief_a_id: "b-1".to_string(),
                belief_b_id: "b-2".to_string(),
                contradiction_type: ContradictionType::Logical,
                severity: 0.9,
                judge_reasoning: Some("preseeded".to_string()),
                detected_at: Utc::now(),
                resolved_at: None,
                resolution: None,
            })
            .await
            .unwrap();

        let stats = det.run_cycle("run-skip", "p").await.unwrap();
        assert_eq!(stats.pairs_examined, 1);
        assert_eq!(
            stats.pairs_skipped_existing, 1,
            "existing pair should short-circuit"
        );
        assert_eq!(stats.llm_calls, 0, "no LLM call when pair_exists");
        assert_eq!(judge.calls(), 0);
    }

    // ------------------------------------------------------------------
    // LLM failure → cycle continues, errors counter ticks.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn llm_failure_increments_errors_and_continues() {
        let judge = Arc::new(MockJudge::err());
        let (det, beliefs, contradictions, _tmp) = setup(judge, config(true, 1, 20));
        seed(&beliefs, "b-1", "user.x", "a", "p").await;
        seed(&beliefs, "b-2", "user.y", "b", "p").await;

        let stats = det.run_cycle("run-fail", "p").await.unwrap();
        assert_eq!(stats.errors, 1, "LLM failure increments errors");
        let listed = contradictions.list_recent("p", 10).await.unwrap();
        assert!(listed.is_empty(), "no row on LLM failure");
    }
}
