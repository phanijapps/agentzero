//! Corrections Abstractor — promotes repeated correction facts to schema facts.
//!
//! Runs during sleep-time. When an agent has accumulated MIN_CORRECTIONS_TO_ABSTRACT
//! (3+) correction facts, asks an LLM to identify a shared principle. If found,
//! writes a `schema` category fact via `save_fact` (upsert — idempotent on
//! repeated calls with the same corrections cluster).
//!
//! Category weights: schema (1.6) > correction (1.5) — schemas are preferred
//! in recall over the raw corrections they distill.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use serde::Deserialize;
use zbot_stores_traits::{CompactionStore, MemoryFactStore};

use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

const MIN_CORRECTIONS_TO_ABSTRACT: usize = 3;
const MAX_CORRECTIONS_PER_CALL: usize = 20;
const MIN_CONFIDENCE: f64 = 0.7;

/// Stats returned from one abstraction cycle.
#[derive(Debug, Default, Clone)]
pub struct AbstractionStats {
    pub corrections_considered: u64,
    pub schemas_abstracted: u64,
    pub skipped_low_confidence: u64,
    pub skipped_llm_error: u64,
}

/// Parsed LLM response shape.
#[derive(Debug, Clone, Deserialize)]
pub struct AbstractionResponse {
    pub schema: String,
    pub confidence: f64,
    pub key_fact: String,
    pub decision: String, // "abstract" | "skip"
}

/// Abstraction so tests can inject a mock LLM.
#[async_trait]
pub trait AbstractionLlm: Send + Sync {
    async fn abstract_corrections(
        &self,
        corrections: &[String],
    ) -> Result<AbstractionResponse, String>;
}

/// Sleep-time component that distills correction facts into schema facts.
pub struct CorrectionsAbstractor {
    memory_store: Arc<dyn MemoryFactStore>,
    compaction_store: Arc<dyn CompactionStore>,
    llm: Arc<dyn AbstractionLlm>,
    /// Minimum time between LLM calls. `Duration::ZERO` = run every cycle.
    interval: Duration,
    last_run: Mutex<Option<Instant>>,
}

impl CorrectionsAbstractor {
    pub fn new(
        memory_store: Arc<dyn MemoryFactStore>,
        compaction_store: Arc<dyn CompactionStore>,
        llm: Arc<dyn AbstractionLlm>,
        interval: Duration,
    ) -> Self {
        Self {
            memory_store,
            compaction_store,
            llm,
            interval,
            last_run: Mutex::new(None),
        }
    }

    /// Run one abstraction cycle. Returns aggregate stats. Any error is
    /// logged and the cycle returns partial stats — never fails hard.
    ///
    /// Skips the LLM call if less than `interval` has elapsed since the last
    /// successful run (throttle resets on daemon restart).
    pub async fn run_cycle(
        &self,
        run_id: &str,
        agent_id: &str,
    ) -> Result<AbstractionStats, String> {
        if !self.interval.is_zero() {
            if let Some(last) = *self.last_run.lock().unwrap() {
                if last.elapsed() < self.interval {
                    return Ok(AbstractionStats::default());
                }
            }
        }

        let mut stats = AbstractionStats::default();

        let corrections = self
            .memory_store
            .get_facts_by_category(agent_id, "correction", MAX_CORRECTIONS_PER_CALL)
            .await
            .map_err(|e| format!("get_facts_by_category: {e}"))?;

        stats.corrections_considered = corrections.len() as u64;

        if corrections.len() < MIN_CORRECTIONS_TO_ABSTRACT {
            return Ok(stats);
        }

        let contents: Vec<String> = corrections.iter().map(|f| f.content.clone()).collect();

        let resp = match self.llm.abstract_corrections(&contents).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    agent_id,
                    error = %e,
                    "corrections-abstractor: LLM failed"
                );
                stats.skipped_llm_error += 1;
                return Ok(stats);
            }
        };

        if resp.decision != "abstract" || resp.confidence < MIN_CONFIDENCE {
            stats.skipped_low_confidence += 1;
            return Ok(stats);
        }

        let key = format!("schema.corrections.{}", short_hash(&resp.key_fact));

        match self
            .memory_store
            .save_fact(
                agent_id,
                "schema",
                &key,
                &resp.key_fact,
                resp.confidence,
                None,
                None, // valid_from: defaults to Utc::now() in store impl
            )
            .await
        {
            Ok(_) => {
                stats.schemas_abstracted += 1;
                *self.last_run.lock().unwrap() = Some(Instant::now());
                let reason = format!(
                    "abstracted from {} corrections (schema={}, confidence={:.2})",
                    corrections.len(),
                    resp.schema,
                    resp.confidence
                );
                if let Ok(Some(fact)) = self
                    .memory_store
                    .get_fact_by_key(agent_id, "global", "__global__", &key)
                    .await
                {
                    let _ = self
                        .compaction_store
                        .record_synthesis(run_id, &fact.id, &reason)
                        .await;
                }
            }
            Err(e) => {
                tracing::warn!(
                    agent_id,
                    key,
                    error = %e,
                    "corrections-abstractor: save_fact failed"
                );
                stats.skipped_llm_error += 1;
            }
        }

        Ok(stats)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn short_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:08x}", (h.finish() & 0xFFFF_FFFF) as u32)
}

// ============================================================================
// LLM-backed implementation
// ============================================================================

/// Production `AbstractionLlm` wired to the injected `MemoryLlmFactory`.
pub struct LlmCorrectionsAbstractor {
    client: CachedLlmClient,
}

impl LlmCorrectionsAbstractor {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 512)),
        }
    }
}

#[async_trait]
impl AbstractionLlm for LlmCorrectionsAbstractor {
    async fn abstract_corrections(
        &self,
        corrections: &[String],
    ) -> Result<AbstractionResponse, String> {
        let client = self.client.get().await?;
        let formatted = corrections
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {c}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "You identify common principles from an AI agent's correction history.\n\
             Below are {n} correction facts the agent has accumulated.\n\
             Decide if they share a common theme expressible as one imperative principle.\n\n\
             Return ONLY JSON: \
             {{\"schema\": string, \"confidence\": 0.0-1.0, \
             \"key_fact\": string, \"decision\": \"abstract\" | \"skip\"}}.\n\
             - \"schema\": theme name in snake_case (<5 words)\n\
             - \"key_fact\": the principle as a single imperative sentence\n\
             - \"decision\": \"abstract\" if clear shared principle, \"skip\" if too diverse\n\n\
             Corrections:\n{formatted}",
            n = corrections.len(),
        );
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        parse_llm_json::<AbstractionResponse>(&response.content)
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

    struct MockLlm {
        response: Mutex<AbstractionResponse>,
    }

    impl MockLlm {
        fn new(resp: AbstractionResponse) -> Self {
            Self {
                response: Mutex::new(resp),
            }
        }

        fn always_skip() -> Self {
            Self::new(AbstractionResponse {
                schema: String::new(),
                confidence: 0.99,
                key_fact: String::new(),
                decision: "skip".into(),
            })
        }
    }

    #[async_trait]
    impl AbstractionLlm for MockLlm {
        async fn abstract_corrections(
            &self,
            _corrections: &[String],
        ) -> Result<AbstractionResponse, String> {
            Ok(self.response.lock().unwrap().clone())
        }
    }

    struct MockFailLlm;

    #[async_trait]
    impl AbstractionLlm for MockFailLlm {
        async fn abstract_corrections(
            &self,
            _corrections: &[String],
        ) -> Result<AbstractionResponse, String> {
            Err("induced failure".into())
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        memory_store: Arc<dyn MemoryFactStore>,
        compaction_store: Arc<dyn CompactionStore>,
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
        }
    }

    async fn seed_corrections(store: &Arc<dyn MemoryFactStore>, agent_id: &str, n: usize) {
        for i in 0..n {
            store
                .save_fact(
                    agent_id,
                    "correction",
                    &format!("corr-{i}"),
                    &format!("Don't do X when Y — correction {i}"),
                    0.9,
                    None,
                    None,
                )
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn inserts_schema_when_abstractions_found() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-abs", 3).await;

        let mock = Arc::new(MockLlm::new(AbstractionResponse {
            schema: "avoid-x-when-y".into(),
            confidence: 0.85,
            key_fact: "When Y is true, always avoid X".into(),
            decision: "abstract".into(),
        }));

        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            Duration::ZERO,
        );

        let stats = abs.run_cycle("run-abs", "agent-abs").await.unwrap();

        assert_eq!(stats.corrections_considered, 3);
        assert_eq!(stats.schemas_abstracted, 1);
        assert_eq!(stats.skipped_low_confidence, 0);
        assert_eq!(stats.skipped_llm_error, 0);

        let schema_facts = h
            .memory_store
            .get_facts_by_category("agent-abs", "schema", 10)
            .await
            .unwrap();
        assert_eq!(schema_facts.len(), 1);
        assert!(schema_facts[0].content.contains("avoid X"));
        assert_eq!(schema_facts[0].category, "schema");
    }

    #[tokio::test]
    async fn skips_when_fewer_than_three_corrections() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-few", 2).await;

        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            Arc::new(MockFailLlm),
            Duration::ZERO,
        );

        let stats = abs.run_cycle("run-few", "agent-few").await.unwrap();

        assert_eq!(stats.corrections_considered, 2);
        assert_eq!(stats.schemas_abstracted, 0);
        assert_eq!(stats.skipped_llm_error, 0);
    }

    #[tokio::test]
    async fn skips_when_decision_is_skip() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-skip", 4).await;

        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            Arc::new(MockLlm::always_skip()),
            Duration::ZERO,
        );

        let stats = abs.run_cycle("run-skip", "agent-skip").await.unwrap();

        assert_eq!(stats.corrections_considered, 4);
        assert_eq!(stats.schemas_abstracted, 0);
        assert_eq!(stats.skipped_low_confidence, 1);

        let schema_facts = h
            .memory_store
            .get_facts_by_category("agent-skip", "schema", 10)
            .await
            .unwrap();
        assert!(schema_facts.is_empty());
    }

    #[tokio::test]
    async fn skips_when_confidence_below_threshold() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-lowconf", 3).await;

        let mock = Arc::new(MockLlm::new(AbstractionResponse {
            schema: "something".into(),
            confidence: 0.5,
            key_fact: "some principle".into(),
            decision: "abstract".into(),
        }));

        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            Duration::ZERO,
        );

        let stats = abs.run_cycle("run-lowconf", "agent-lowconf").await.unwrap();

        assert_eq!(stats.schemas_abstracted, 0);
        assert_eq!(stats.skipped_low_confidence, 1);
    }

    #[tokio::test]
    async fn idempotent_on_second_call() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-idem", 3).await;

        let mock = Arc::new(MockLlm::new(AbstractionResponse {
            schema: "principle-x".into(),
            confidence: 0.9,
            key_fact: "Always do X before Y".into(),
            decision: "abstract".into(),
        }));

        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            Duration::ZERO,
        );

        abs.run_cycle("run-idem-1", "agent-idem").await.unwrap();
        abs.run_cycle("run-idem-2", "agent-idem").await.unwrap();

        let schema_facts = h
            .memory_store
            .get_facts_by_category("agent-idem", "schema", 10)
            .await
            .unwrap();
        assert_eq!(
            schema_facts.len(),
            1,
            "upsert must not create duplicate schema facts"
        );
    }

    #[tokio::test]
    async fn throttle_skips_when_interval_not_elapsed() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-throttle", 3).await;

        // Set a 1-hour interval — no time will pass in test, so second call is throttled.
        let interval = Duration::from_secs(3600);
        let mock = Arc::new(MockLlm::new(AbstractionResponse {
            schema: "principle".into(),
            confidence: 0.9,
            key_fact: "Always do X".into(),
            decision: "abstract".into(),
        }));
        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            interval,
        );

        // First call runs and records last_run.
        let s1 = abs.run_cycle("run-t1", "agent-throttle").await.unwrap();
        assert_eq!(s1.schemas_abstracted, 1);

        // Second call immediately after: interval hasn't elapsed → skipped.
        let s2 = abs.run_cycle("run-t2", "agent-throttle").await.unwrap();
        assert_eq!(s2.schemas_abstracted, 0);
        assert_eq!(
            s2.corrections_considered, 0,
            "throttled call must return empty stats"
        );
    }

    #[tokio::test]
    async fn zero_interval_always_runs() {
        let h = setup();
        seed_corrections(&h.memory_store, "agent-zero", 3).await;

        let mock = Arc::new(MockLlm::new(AbstractionResponse {
            schema: "principle".into(),
            confidence: 0.9,
            key_fact: "Always do Z".into(),
            decision: "abstract".into(),
        }));
        let abs = CorrectionsAbstractor::new(
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            Duration::ZERO,
        );

        // Both calls run regardless of elapsed time.
        let s1 = abs.run_cycle("run-z1", "agent-zero").await.unwrap();
        let s2 = abs.run_cycle("run-z2", "agent-zero").await.unwrap();
        assert_eq!(s1.schemas_abstracted, 1);
        assert_eq!(s2.schemas_abstracted, 1);
    }

    #[test]
    fn short_hash_is_deterministic() {
        assert_eq!(short_hash("hello"), short_hash("hello"));
        assert_ne!(short_hash("hello"), short_hash("world"));
    }
}
