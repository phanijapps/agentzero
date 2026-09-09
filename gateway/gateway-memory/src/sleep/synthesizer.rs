//! Synthesizer — extracts cross-session strategy facts.
//!
//! Runs during sleep-time maintenance. For each entity that appears across
//! at least 2 distinct sessions in the last 30 days, asks an LLM whether the
//! pattern warrants a `category='strategy'` memory fact. Conservative:
//! any LLM/DB/parse error skips the candidate — the whole cycle never
//! fails hard.
//!
//! Phase D4: trait-routed. The kg / episode / memory reads + writes go
//! through `Arc<dyn ...>` so the synthesis cycle runs against either
//! the configured backend. No SQL bodies live here anymore — each backend
//! implements the underlying operations natively.

use std::sync::Arc;

use agent_runtime::llm::embedding::EmbeddingClient;
use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use knowledge_graph::kg_trait::{KnowledgeGraphStore, StrategyCandidate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zbot_stores_traits::{
    CompactionStore, EmbeddingQueryIdentity, EpisodeStore, MemoryFactStore, StrategyFactInsert,
};

use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

/// Maximum candidates fetched from the DB per cycle.
const CANDIDATE_LIMIT: usize = 20;
/// Maximum LLM calls per cycle (budget).
const MAX_LLM_CALLS_PER_CYCLE: usize = 10;
/// Minimum confidence required to insert a synthesis fact.
const MIN_CONFIDENCE: f64 = 0.7;
/// Cosine threshold for dedup against existing strategy facts.
const DEDUP_COSINE_THRESHOLD: f64 = 0.88;
/// Time window the synthesizer scans for cross-session activity.
const LOOKBACK_DAYS: i64 = 30;

/// Stats returned from one synthesis cycle.
#[derive(Debug, Default, Clone)]
pub struct SynthesisStats {
    pub candidates_considered: u64,
    pub llm_calls_made: u64,
    pub facts_inserted: u64,
    pub facts_bumped: u64,
    pub skipped_low_confidence: u64,
    pub skipped_llm_or_parse_error: u64,
}

/// Parsed LLM response shape.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SynthesisResponse {
    pub strategy: String,
    pub confidence: f64,
    pub key_fact: String,
    pub decision: String, // "synthesize" | "skip"
}

/// Neighborhood context sent to the LLM for a single candidate.
#[derive(Debug, Clone)]
pub struct SynthesisInput {
    pub entity_name: String,
    pub entity_type: String,
    pub session_count: u64,
    pub task_summaries: Vec<String>,
    pub relationship_summaries: Vec<String>,
}

/// Abstraction so tests can inject a mock LLM without touching the network.
/// Production impl wraps an OpenAI-compatible client.
#[async_trait]
pub trait SynthesisLlm: Send + Sync {
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResponse, String>;
}

/// Cross-session strategy synthesizer.
///
/// Phase D4: trait-routed. All KG / episode / memory reads + writes
/// flow through trait objects; each backend implements them natively.
pub struct Synthesizer {
    kg_store: Arc<dyn KnowledgeGraphStore>,
    episode_store: Arc<dyn EpisodeStore>,
    memory_store: Arc<dyn MemoryFactStore>,
    compaction_store: Arc<dyn CompactionStore>,
    llm: Arc<dyn SynthesisLlm>,
    /// Optional embedding client used for cosine dedup. When absent,
    /// dedup falls back to the unique `(agent_id, scope, ward_id, key)`
    /// constraint on `memory_facts` (via upsert).
    embedder: Option<Arc<dyn EmbeddingClient>>,
}

impl Synthesizer {
    pub fn new(
        kg_store: Arc<dyn KnowledgeGraphStore>,
        episode_store: Arc<dyn EpisodeStore>,
        memory_store: Arc<dyn MemoryFactStore>,
        compaction_store: Arc<dyn CompactionStore>,
        llm: Arc<dyn SynthesisLlm>,
        embedder: Option<Arc<dyn EmbeddingClient>>,
    ) -> Self {
        Self {
            kg_store,
            episode_store,
            memory_store,
            compaction_store,
            llm,
            embedder,
        }
    }

    /// Run one synthesis cycle. Returns aggregate stats. Any per-candidate
    /// error is logged and skipped — the cycle never fails hard.
    pub async fn run_cycle(&self, run_id: &str) -> Result<SynthesisStats, String> {
        let mut stats = SynthesisStats::default();
        let candidates = self
            .kg_store
            .list_strategy_candidates(2, LOOKBACK_DAYS, CANDIDATE_LIMIT)
            .await
            .map_err(|e| format!("list_strategy_candidates: {e}"))?;
        for cand in candidates.into_iter().take(MAX_LLM_CALLS_PER_CYCLE) {
            stats.candidates_considered += 1;
            self.process_candidate(run_id, &cand, &mut stats).await;
        }
        Ok(stats)
    }

    async fn process_candidate(
        &self,
        run_id: &str,
        cand: &StrategyCandidate,
        stats: &mut SynthesisStats,
    ) {
        let input = match self.build_input(cand).await {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(entity = %cand.entity_id, error = %e, "synth: build_input failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };
        let episode_ids = match self
            .kg_store
            .episode_ids_for_entity(&cand.entity_id, LOOKBACK_DAYS)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!(entity = %cand.entity_id, error = %e, "synth: episode_ids failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };

        stats.llm_calls_made += 1;
        let resp = match self.llm.synthesize(&input).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(entity = %cand.entity_id, error = %e, "synth: LLM failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };

        if resp.decision != "synthesize" || resp.confidence < MIN_CONFIDENCE {
            stats.skipped_low_confidence += 1;
            return;
        }

        self.commit_synthesis(run_id, cand, &resp, &episode_ids, stats)
            .await;
    }

    async fn commit_synthesis(
        &self,
        run_id: &str,
        cand: &StrategyCandidate,
        resp: &SynthesisResponse,
        episode_ids: &[String],
        stats: &mut SynthesisStats,
    ) {
        // Dedup step (optional — requires embedder)
        let embedding = self.embed_content(&resp.key_fact).await;
        if let Some(ref emb) = embedding {
            let query_identity = self.embedding_query_identity();
            match self
                .memory_store
                .find_strategy_fact_by_similarity_with_identity(
                    &cand.agent_id,
                    emb,
                    query_identity.as_ref(),
                    DEDUP_COSINE_THRESHOLD as f32,
                    50,
                )
                .await
            {
                Ok(Some(existing)) => {
                    let merged =
                        merge_episode_ids(existing.source_episode_id.as_deref(), episode_ids);
                    let now = chrono::Utc::now().to_rfc3339();
                    if let Err(e) = self
                        .memory_store
                        .bump_strategy_fact_episodes(&existing.fact_id, &merged, &now)
                        .await
                    {
                        tracing::warn!(fact_id = %existing.fact_id, error = %e, "synth: bump failed");
                    }
                    stats.facts_bumped += 1;
                    self.audit(run_id, &existing.fact_id, resp, "bumped existing")
                        .await;
                    return;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "synth: dedup lookup failed; will upsert");
                }
            }
        }

        let fact_id = match self.insert_new(cand, resp, episode_ids, embedding).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(entity = %cand.entity_id, error = %e, "synth: insert failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };
        stats.facts_inserted += 1;
        self.audit(run_id, &fact_id, resp, "new synthesis").await;
    }

    async fn audit(&self, run_id: &str, fact_id: &str, resp: &SynthesisResponse, note: &str) {
        let reason = format!(
            "{note}: strategy={} confidence={:.2}",
            resp.strategy, resp.confidence
        );
        if let Err(e) = self
            .compaction_store
            .record_synthesis(run_id, fact_id, &reason)
            .await
        {
            tracing::warn!(fact_id = %fact_id, error = %e, "synth: record_synthesis failed");
        }
    }

    async fn insert_new(
        &self,
        cand: &StrategyCandidate,
        resp: &SynthesisResponse,
        episode_ids: &[String],
        embedding: Option<Vec<f32>>,
    ) -> Result<String, String> {
        let hash8 = short_hash(&resp.key_fact);
        let slug = slugify(&cand.name);
        let key = format!("strategy.synthesis.{slug}.{hash8}");
        let source_episode_id = Some(encode_episode_ids(episode_ids));

        self.memory_store
            .insert_strategy_fact(StrategyFactInsert {
                agent_id: cand.agent_id.clone(),
                key,
                content: resp.key_fact.clone(),
                confidence: resp.confidence,
                source_summary: Some(format!(
                    "cross-session synthesis over {} sessions (entity: {})",
                    cand.n_sessions, cand.name
                )),
                embedding,
                source_episode_id,
            })
            .await
            .map_err(|e| e.to_string())
    }

    async fn embed_content(&self, text: &str) -> Option<Vec<f32>> {
        let client = self.embedder.as_ref()?;
        match client.embed(&[text]).await {
            Ok(mut v) if !v.is_empty() => Some(v.remove(0)),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, "synth: embed failed");
                None
            }
        }
    }

    fn embedding_query_identity(&self) -> Option<EmbeddingQueryIdentity> {
        let client = self.embedder.as_ref()?;
        Some(EmbeddingQueryIdentity {
            provider_type: client.provider_type(),
            model: client.model_name(),
            dimensions: client.dimensions() as u32,
            prompt_profile: client.prompt_profile(),
            normalization: client.normalization(),
        })
    }

    async fn build_input(&self, cand: &StrategyCandidate) -> Result<SynthesisInput, String> {
        let ctx = self
            .kg_store
            .relationship_context_for_entity(&cand.entity_id, LOOKBACK_DAYS, 50)
            .await
            .map_err(|e| format!("relationship_context_for_entity: {e}"))?;
        let task_summaries = self
            .episode_store
            .task_summaries_for_sessions(&ctx.session_ids)
            .await
            .map_err(|e| e.to_string())?;
        Ok(SynthesisInput {
            entity_name: cand.name.clone(),
            entity_type: cand.entity_type.clone(),
            session_count: cand.n_sessions as u64,
            task_summaries,
            relationship_summaries: ctx.summaries,
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn encode_episode_ids(ids: &[String]) -> String {
    // Comma-joined; decoded by merge_episode_ids. We avoid JSON to match the
    // convention used elsewhere in the schema (`kg_relationships.source_episode_ids`).
    ids.join(",")
}

fn merge_episode_ids(existing: Option<&str>, incoming: &[String]) -> String {
    let mut set: Vec<String> = existing
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();
    set.retain(|s| !s.is_empty());
    for id in incoming {
        if !set.iter().any(|s| s == id) {
            set.push(id.clone());
        }
    }
    set.join(",")
}

fn short_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:08x}", (h.finish() & 0xFFFF_FFFF) as u32)
}

fn slugify(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "entity".to_string()
    } else {
        trimmed
    }
}

// ============================================================================
// LLM-backed implementation
// ============================================================================

/// LLM-backed `SynthesisLlm` wired to the injected `MemoryLlmFactory`.
/// Conservative on failure — propagates `Err` so `run_cycle` can log+skip.
pub struct LlmSynthesizer {
    client: CachedLlmClient,
}

impl LlmSynthesizer {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 512)),
        }
    }
}

#[async_trait]
impl SynthesisLlm for LlmSynthesizer {
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResponse, String> {
        let client = self.client.get().await?;
        let prompt = format!(
            "You identify reusable cross-session strategies from an agent's knowledge graph.\n\
             The entity below has appeared across {n} distinct sessions within the last 30 days.\n\
             Decide whether the repeated co-occurrence reveals a *strategy* worth memorising \
             as a stable rule (e.g. \"when X times out, retry with backoff\").\n\n\
             Return ONLY JSON: {{\"strategy\": string, \"confidence\": 0.0-1.0, \
             \"key_fact\": string, \"decision\": \"synthesize\" | \"skip\"}}.\n\n\
             Entity: name={name:?} type={etype}\n\
             Recent task summaries:\n{tasks}\n\n\
             Relationships:\n{rels}",
            n = input.session_count,
            name = input.entity_name,
            etype = input.entity_type,
            tasks = input
                .task_summaries
                .iter()
                .map(|t| format!("- {t}"))
                .collect::<Vec<_>>()
                .join("\n"),
            rels = input
                .relationship_summaries
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        // Plain chat + parse — the `submit`-tool/extractor approach was tried
        // but the tool's presence made the model return short non-JSON on Z.AI
        // (same failure mode as intent analysis). The existing "Return ONLY
        // JSON" prompt + plain chat is reliable.
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        parse_llm_json::<SynthesisResponse>(&response.content)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::test_support;
    use agent_primitives::vault_paths::VaultPaths;
    use rusqlite::params;
    use std::sync::Mutex;
    use zbot_stores_sqlite::kg::storage::GraphStorage;
    use zbot_stores_sqlite::vector_index::{SqliteVecIndex, VectorIndex};
    use zbot_stores_sqlite::EpisodeRepository;
    use zbot_stores_sqlite::{
        CompactionRepository, GatewayCompactionStore, GatewayEpisodeStore, KnowledgeDatabase,
        SqliteKgStore,
    };

    struct MockLlm {
        response: Mutex<SynthesisResponse>,
        calls: Mutex<u64>,
    }

    impl MockLlm {
        fn new(resp: SynthesisResponse) -> Self {
            Self {
                response: Mutex::new(resp),
                calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl SynthesisLlm for MockLlm {
        async fn synthesize(&self, _input: &SynthesisInput) -> Result<SynthesisResponse, String> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.response.lock().unwrap().clone())
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        db: Arc<KnowledgeDatabase>,
        memory_store: Arc<dyn MemoryFactStore>,
        compaction_repo: Arc<CompactionRepository>,
        kg_store: Arc<dyn KnowledgeGraphStore>,
        episode_store: Arc<dyn EpisodeStore>,
        compaction_store: Arc<dyn CompactionStore>,
    }

    fn setup() -> Harness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let compaction_repo = Arc::new(CompactionRepository::new(db.clone()));
        let graph = Arc::new(GraphStorage::new(db.clone()).expect("graph"));
        let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph));
        let episode_vec: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "session_episodes_index", "episode_id")
                .expect("vec index init"),
        );
        let episode_repo = Arc::new(EpisodeRepository::new(db.clone(), episode_vec));
        let episode_store: Arc<dyn EpisodeStore> = Arc::new(GatewayEpisodeStore::new(episode_repo));
        let compaction_store: Arc<dyn CompactionStore> =
            Arc::new(GatewayCompactionStore::new(compaction_repo.clone()));
        let memory_store = test_support::fact_store(&tmp);
        Harness {
            _tmp: tmp,
            db,
            memory_store,
            compaction_repo,
            kg_store,
            episode_store,
            compaction_store,
        }
    }

    /// Seed kg_entities, kg_relationships, kg_episodes such that an entity
    /// "postgres-timeout" spans 3 distinct sessions within the last 30 days.
    fn seed_cross_session(db: &KnowledgeDatabase, agent_id: &str) -> Vec<String> {
        let now = chrono::Utc::now().to_rfc3339();
        // Entity
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_entities
                    (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                     epistemic_class, confidence, mention_count, access_count,
                     first_seen_at, last_seen_at)
                 VALUES ('ent-pg', ?1, 'Concept', 'postgres-timeout', 'postgres-timeout',
                         'hash-pg', 'current', 0.9, 3, 0, ?2, ?2)",
                params![agent_id, now],
            )?;
            // 3 episodes, one per session.
            for (i, sid) in ["sess-1", "sess-2", "sess-3"].iter().enumerate() {
                conn.execute(
                    "INSERT INTO kg_episodes
                        (id, source_type, source_ref, content_hash, session_id, agent_id,
                         status, retry_count, created_at, completed_at)
                     VALUES (?1, 'session', ?2, ?3, ?4, ?5, 'done', 0, ?6, ?6)",
                    params![
                        format!("ep-{i}"),
                        format!("ref-{i}"),
                        format!("hash-{i}"),
                        sid,
                        agent_id,
                        now,
                    ],
                )?;
                // Also seed a session_episodes row with a task_summary.
                conn.execute(
                    "INSERT INTO session_episodes
                        (id, session_id, agent_id, task_summary, outcome, created_at)
                     VALUES (?1, ?2, ?3, ?4, 'success', ?5)",
                    params![
                        format!("se-{i}"),
                        sid,
                        agent_id,
                        format!("Investigated postgres timeout issue #{i}"),
                        now,
                    ],
                )?;
            }
            // A self-related relationship whose source_episode_ids references
            // all three episodes (comma-separated, matching production convention).
            conn.execute(
                "INSERT INTO kg_entities
                    (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                     epistemic_class, confidence, mention_count, access_count,
                     first_seen_at, last_seen_at)
                 VALUES ('ent-svc', ?1, 'Concept', 'service-layer', 'service-layer',
                         'hash-svc', 'current', 0.9, 1, 0, ?2, ?2)",
                params![agent_id, now],
            )?;
            conn.execute(
                "INSERT INTO kg_relationships
                    (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                     epistemic_class, confidence, mention_count, access_count,
                     first_seen_at, last_seen_at, valid_from, source_episode_ids)
                 VALUES ('rel-1', ?1, 'ent-pg', 'ent-svc', 'affects',
                         'current', 0.9, 3, 0, ?2, ?2, ?2, 'ep-0,ep-1,ep-2')",
                params![agent_id, now],
            )?;
            Ok(())
        })
        .expect("seed");
        vec!["ep-0".to_string(), "ep-1".to_string(), "ep-2".to_string()]
    }

    #[tokio::test]
    async fn synthesizes_strategy_across_sessions() {
        let h = setup();
        let agent_id = "agent-synth";
        let episode_ids = seed_cross_session(&h.db, agent_id);

        let mock = Arc::new(MockLlm::new(SynthesisResponse {
            strategy: "retry postgres with backoff".into(),
            confidence: 0.85,
            key_fact: "When postgres-timeout recurs, prefer jittered exponential backoff".into(),
            decision: "synthesize".into(),
        }));

        let synth = Synthesizer::new(
            h.kg_store.clone(),
            h.episode_store.clone(),
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock.clone(),
            None, // no embedder -> key-based dedup path
        );

        let run_id = "run-synth-test";
        let stats = synth.run_cycle(run_id).await.expect("run_cycle");

        // Both endpoints of the relationship ("postgres-timeout" and
        // "service-layer") legitimately span 3 sessions via the single
        // 'affects' edge, so we expect 2 candidates, one fact each.
        assert_eq!(stats.candidates_considered, 2, "candidates: {stats:?}");
        assert_eq!(stats.llm_calls_made, 2);
        assert_eq!(stats.facts_inserted, 2);
        assert_eq!(*mock.calls.lock().unwrap(), 2);

        // Memory fact rows (via the production trait surface).
        let fact_values = h
            .memory_store
            .list_memory_facts(Some(agent_id), Some("strategy"), None, 10, 0)
            .await
            .expect("facts");
        let facts: Vec<zbot_stores_domain::MemoryFact> = fact_values
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        assert_eq!(facts.len(), 2);
        let pg_fact = facts
            .iter()
            .find(|f| f.key.contains("postgres-timeout"))
            .expect("postgres-timeout fact");
        assert_eq!(pg_fact.category, "strategy");
        let sei = pg_fact.source_episode_id.clone().unwrap_or_default();
        for ep in &episode_ids {
            assert!(sei.contains(ep), "episode {ep} missing from {sei}");
        }
        assert!(pg_fact.key.starts_with("strategy.synthesis."));
        assert!(pg_fact.content.contains("postgres-timeout"));

        // Compaction audit rows: one per inserted fact.
        let rows = h.compaction_repo.list_run(run_id).expect("list_run");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.operation == "synthesize"));
        let fact_ids: std::collections::HashSet<_> = facts.iter().map(|f| f.id.clone()).collect();
        for row in &rows {
            assert!(
                fact_ids.contains(row.entity_id.as_deref().unwrap_or("")),
                "audit row must reference an inserted fact id"
            );
        }
    }

    #[tokio::test]
    async fn skips_when_decision_is_skip() {
        let h = setup();
        seed_cross_session(&h.db, "agent-skip");
        let mock = Arc::new(MockLlm::new(SynthesisResponse {
            strategy: "".into(),
            confidence: 0.99,
            key_fact: "".into(),
            decision: "skip".into(),
        }));
        let synth = Synthesizer::new(
            h.kg_store.clone(),
            h.episode_store.clone(),
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            None,
        );
        let stats = synth.run_cycle("run-skip").await.expect("run_cycle");
        assert_eq!(stats.skipped_low_confidence, 2);
        assert_eq!(stats.facts_inserted, 0);
        let rows = h.compaction_repo.list_run("run-skip").expect("rows");
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn skips_when_confidence_below_threshold() {
        let h = setup();
        seed_cross_session(&h.db, "agent-lowconf");
        let mock = Arc::new(MockLlm::new(SynthesisResponse {
            strategy: "maybe".into(),
            confidence: 0.5,
            key_fact: "some fact".into(),
            decision: "synthesize".into(),
        }));
        let synth = Synthesizer::new(
            h.kg_store.clone(),
            h.episode_store.clone(),
            h.memory_store.clone(),
            h.compaction_store.clone(),
            mock,
            None,
        );
        let stats = synth.run_cycle("run-lowconf").await.expect("run_cycle");
        assert_eq!(stats.skipped_low_confidence, 2);
        assert_eq!(stats.facts_inserted, 0);
    }

    #[test]
    fn merge_episode_ids_preserves_order_and_dedups() {
        let merged = merge_episode_ids(Some("ep-a,ep-b"), &["ep-b".into(), "ep-c".into()]);
        assert_eq!(merged, "ep-a,ep-b,ep-c");
    }

    #[test]
    fn slugify_replaces_nonalnum() {
        assert_eq!(slugify("Postgres Timeout!"), "postgres-timeout");
        assert_eq!(slugify("   "), "entity");
    }
}
