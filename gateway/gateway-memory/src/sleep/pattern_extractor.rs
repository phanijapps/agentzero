//! PatternExtractor — procedural pattern extraction sleep-time op.
//!
//! Scans recent successful session_episodes, finds pairs whose task_summary
//! embeddings are semantically similar AND whose tool-call sequences share a
//! structural prefix of 3+ tools in the same order, and asks an LLM to
//! generalize each match into a `procedures` row. Conservative: any
//! per-candidate error is logged and skipped — the cycle never fails hard.
//!
//! Phase D4: trait-routed. The episode reads, conversation reads, and
//! procedure writes flow through `Arc<dyn ...>` so the cycle runs against
//! either backend; each backend implements the operations natively.

use agent_primitives::vec_math::cosine_f64;
use std::sync::Arc;

use agent_runtime::llm::embedding::EmbeddingClient;
use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use zbot_stores_traits::{
    CompactionStore, EpisodeStore, PatternProcedureInsert, PatternStep, ProcedureStore,
    SuccessfulEpisode,
};

use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

/// Maximum successful session_episodes loaded per cycle.
const CANDIDATE_LIMIT: usize = 50;
/// Maximum LLM calls per cycle.
const MAX_LLM_CALLS_PER_CYCLE: usize = 5;
/// Cosine similarity threshold between task_summary embeddings for a pair
/// to be considered semantically related.
const PAIR_COSINE_THRESHOLD: f64 = 0.82;
/// Minimum number of matching tool names (in order) to call a pair a pattern.
const MIN_PATTERN_LENGTH: usize = 3;
/// Existing procedure with `success_count` at or above this is considered
/// locked-in and will not be overwritten by a new synthesis with the same name.
const DEDUP_SUCCESS_FLOOR: i32 = 2;
/// Time window the pattern extractor scans for successful episodes.
const LOOKBACK_DAYS: i64 = 30;

/// Default ward id for synthesized procedures.
const PROC_WARD: &str = "__global__";

/// Stats returned from one extraction cycle.
#[derive(Debug, Default, Clone)]
pub struct PatternStats {
    pub episodes_considered: u64,
    pub pairs_evaluated: u64,
    pub pairs_matched: u64,
    pub llm_calls_made: u64,
    pub procedures_inserted: u64,
    pub skipped_existing: u64,
    pub skipped_llm_or_parse_error: u64,
}

/// Parsed LLM response shape.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatternResponse {
    pub name: String,
    pub description: String,
    pub trigger_pattern: String,
    #[serde(default)]
    pub parameters: Vec<String>,
    pub steps: Vec<PatternStep>,
}

/// Context passed to the LLM for generalization.
#[derive(Debug, Clone)]
pub struct PatternInput {
    pub task_summary_a: String,
    pub task_summary_b: String,
    pub tool_sequence_a: Vec<String>,
    pub tool_sequence_b: Vec<String>,
    pub matched_prefix: Vec<String>,
    /// Comma-separated list of registered tool names — the LLM must pick
    /// `action` values from this list. Populated by the composition site
    /// from the live `ToolRegistry`. Empty string is allowed for tests /
    /// boot-time degraded mode.
    pub tool_whitelist: String,
}

/// Abstraction so tests can inject a mock LLM without touching the network.
#[async_trait]
pub trait PatternExtractLlm: Send + Sync {
    async fn generalize(&self, input: &PatternInput) -> Result<PatternResponse, String>;
}

/// Procedural pattern extractor.
///
/// Phase D4: trait-routed. All reads/writes go through trait objects;
/// each backend implements them natively (SQLite via the existing
/// repos, Surreal via SurrealDB queries).
pub struct PatternExtractor {
    episode_store: Arc<dyn EpisodeStore>,
    message_store: Arc<dyn zbot_conversation::MessageStore>,
    procedure_store: Arc<dyn ProcedureStore>,
    compaction_store: Arc<dyn CompactionStore>,
    llm: Arc<dyn PatternExtractLlm>,
    /// Optional embedding client used to populate `embedding` on the
    /// synthesised `PatternProcedureInsert`. When `None` or when an
    /// `embed()` call fails, the insert still proceeds — the embedding
    /// is observability for vec-similarity recall, not correctness.
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
    /// Registered tool names the LLM may pick from when emitting
    /// procedure step `action` fields. Plumbed in from composition
    /// (`services.rs`); empty for tests / degraded mode.
    tool_whitelist: Vec<String>,
}

impl PatternExtractor {
    pub fn new(
        episode_store: Arc<dyn EpisodeStore>,
        message_store: Arc<dyn zbot_conversation::MessageStore>,
        procedure_store: Arc<dyn ProcedureStore>,
        compaction_store: Arc<dyn CompactionStore>,
        llm: Arc<dyn PatternExtractLlm>,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
        tool_whitelist: Vec<String>,
    ) -> Self {
        Self {
            episode_store,
            message_store,
            procedure_store,
            compaction_store,
            llm,
            embedding_client,
            tool_whitelist,
        }
    }

    /// Run one extraction cycle. Returns aggregate stats. Any per-candidate
    /// error is logged and skipped — the cycle never fails hard.
    pub async fn run_cycle(&self, run_id: &str) -> Result<PatternStats, String> {
        let mut stats = PatternStats::default();
        let episodes = self
            .episode_store
            .list_successful_episodes_with_embedding(LOOKBACK_DAYS, CANDIDATE_LIMIT)
            .await
            .map_err(|e| e.to_string())?;
        stats.episodes_considered = episodes.len() as u64;
        if episodes.len() < 2 {
            return Ok(stats);
        }

        let pairs = build_matching_pairs(&episodes, &mut stats);

        for pair in pairs.into_iter().take(MAX_LLM_CALLS_PER_CYCLE) {
            self.process_pair(run_id, &episodes, pair, &mut stats).await;
        }
        Ok(stats)
    }

    async fn process_pair(
        &self,
        run_id: &str,
        episodes: &[SuccessfulEpisode],
        pair: MatchedPair,
        stats: &mut PatternStats,
    ) {
        let ep_a = &episodes[pair.idx_a];
        let ep_b = &episodes[pair.idx_b];

        let tools_a = match self
            .message_store
            .tool_sequence_for_session(&ep_a.session_id)
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(session = %ep_a.session_id, error = %e, "pattern: tools_a failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };
        let tools_b = match self
            .message_store
            .tool_sequence_for_session(&ep_b.session_id)
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(session = %ep_b.session_id, error = %e, "pattern: tools_b failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };

        let matched = longest_common_prefix(&tools_a, &tools_b);
        if matched.len() < MIN_PATTERN_LENGTH {
            // Should not happen — pair only got here after prefix check — but guard.
            return;
        }

        let input = PatternInput {
            task_summary_a: ep_a.task_summary.clone(),
            task_summary_b: ep_b.task_summary.clone(),
            tool_sequence_a: tools_a,
            tool_sequence_b: tools_b,
            matched_prefix: matched,
            tool_whitelist: self.tool_whitelist.join(", "),
        };

        stats.llm_calls_made += 1;
        let resp = match self.llm.generalize(&input).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "pattern: LLM failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };

        self.commit_pattern(run_id, &ep_a.agent_id, &resp, stats)
            .await;
    }

    async fn commit_pattern(
        &self,
        run_id: &str,
        agent_id: &str,
        resp: &PatternResponse,
        stats: &mut PatternStats,
    ) {
        let name = sanitize_name(&resp.name);
        if name.is_empty() {
            stats.skipped_llm_or_parse_error += 1;
            return;
        }
        match self
            .procedure_store
            .get_procedure_summary_by_name(agent_id, &name)
            .await
        {
            Ok(Some(existing)) if existing.success_count >= DEDUP_SUCCESS_FLOOR => {
                stats.skipped_existing += 1;
                return;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "pattern: existing lookup failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        }

        let mut req = match build_procedure_insert(agent_id, &name, resp) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "pattern: build procedure failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };

        // Embed the procedure description so the SQLite vec index can
        // be populated. Graceful: warn + proceed without on failure —
        // the insert still succeeds, only vec-similarity recall is lost.
        if let Some(client) = self.embedding_client.as_ref() {
            let embed_input = format!("{}\n{}", resp.name, resp.description);
            match client.embed(&[embed_input.as_str()]).await {
                Ok(mut vecs) if !vecs.is_empty() => req.embedding = Some(vecs.remove(0)),
                Ok(_) => {
                    tracing::warn!("pattern: embedding returned no vectors; proceeding without");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "pattern: embedding failed; proceeding without");
                }
            }
        }

        let proc_id = match self.procedure_store.insert_pattern_procedure(req).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "pattern: insert_pattern_procedure failed");
                stats.skipped_llm_or_parse_error += 1;
                return;
            }
        };
        stats.procedures_inserted += 1;

        let reason = format!("pattern '{}' across 2 sessions", name);
        if let Err(e) = self
            .compaction_store
            .record_pattern(run_id, &proc_id, &reason)
            .await
        {
            tracing::warn!(error = %e, "pattern: record_pattern failed");
        }
    }
}

/// Pair of episode indices whose task_summaries are semantically similar AND
/// whose tool-call sequences share a structural prefix.
#[derive(Debug, Clone, Copy)]
struct MatchedPair {
    idx_a: usize,
    idx_b: usize,
}

/// Build candidate pairs by cosine similarity of embeddings. The structural
/// match check happens later, inside `process_pair`, once tool sequences are
/// actually loaded (to keep this function cheap).
fn build_matching_pairs(
    episodes: &[SuccessfulEpisode],
    stats: &mut PatternStats,
) -> Vec<MatchedPair> {
    let mut pairs = Vec::new();
    for i in 0..episodes.len() {
        for j in (i + 1)..episodes.len() {
            stats.pairs_evaluated += 1;
            let (a, b) = (&episodes[i], &episodes[j]);
            let (ea, eb) = match (&a.embedding, &b.embedding) {
                (Some(x), Some(y)) => (x, y),
                _ => continue,
            };
            if a.agent_id != b.agent_id {
                // Keep patterns within a single agent for now.
                continue;
            }
            if cosine_f64(ea, eb) >= PAIR_COSINE_THRESHOLD {
                stats.pairs_matched += 1;
                pairs.push(MatchedPair { idx_a: i, idx_b: j });
            }
        }
    }
    pairs
}

/// Returns the longest prefix (in order) common to both sequences.
fn longest_common_prefix<T: PartialEq + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let n = a.len().min(b.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if a[i] == b[i] {
            out.push(a[i].clone());
        } else {
            break;
        }
    }
    out
}

fn build_procedure_insert(
    agent_id: &str,
    name: &str,
    resp: &PatternResponse,
) -> Result<PatternProcedureInsert, String> {
    let steps_json =
        serde_json::to_string(&resp.steps).map_err(|e| format!("steps serialize: {e}"))?;
    let parameters_json = if resp.parameters.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&resp.parameters)
                .map_err(|e| format!("parameters serialize: {e}"))?,
        )
    };
    Ok(PatternProcedureInsert {
        agent_id: agent_id.to_string(),
        ward_id: Some(PROC_WARD.to_string()),
        name: name.to_string(),
        description: resp.description.clone(),
        trigger_pattern: Some(resp.trigger_pattern.clone()),
        steps_json,
        parameters_json,
        // The caller (`PatternExtractor::commit_pattern`) populates
        // `embedding` after this function returns — keeping the embed
        // call near the `procedure_store` write so failures degrade
        // gracefully without rebuilding the row.
        embedding: None,
        // Mining requires a matched pair of successful episodes; the procedure
        // is therefore evidenced by 2 successful sessions before it ever exists.
        // Recording sc=1 would put it below the middleware's legacy-advisory
        // floor (sc>=2) and below the promotion floor (sc>=3), making the
        // procedure invisible until manually invoked twice — defeating organic
        // discovery. Seeding at 2 lets the advisory surface fire immediately;
        // the first real invocation bumps it to 3 and the promoted recommendation
        // takes over.
        success_count: 2,
    })
}

// ============================================================================
// Helpers
// ============================================================================

fn sanitize_name(raw: &str) -> String {
    let out: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let trimmed: String = out
        .trim_matches('_')
        .chars()
        .scan(' ', |prev, c| {
            let emit = !(c == '_' && *prev == '_');
            *prev = c;
            Some(if emit { Some(c) } else { None })
        })
        .flatten()
        .collect();
    trimmed
}

// ============================================================================
// LLM-backed implementation
// ============================================================================

/// LLM-backed `PatternExtractLlm`. Conservative on failure — propagates `Err`
/// so `run_cycle` can log+skip.
pub struct LlmPatternExtractor {
    client: CachedLlmClient,
}

impl LlmPatternExtractor {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 1024)),
        }
    }
}

#[async_trait]
impl PatternExtractLlm for LlmPatternExtractor {
    async fn generalize(&self, input: &PatternInput) -> Result<PatternResponse, String> {
        let client = self.client.get().await?;
        let prompt = build_pattern_prompt(input);
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        parse_llm_json::<PatternResponse>(&response.content)
    }
}

/// Build the LLM prompt for pattern generalization. Pulled out of
/// `LlmPatternExtractor::generalize` so tests can inspect the exact
/// prompt the production extractor builds.
///
/// The prompt enforces a dispatchable shape: `action` must come from
/// the tool whitelist, `args` must be a JSON object, and `binds` lists
/// which fields of a step's result are exposed for `{step_N.field}`
/// interpolation in later steps.
pub(crate) fn build_pattern_prompt(input: &PatternInput) -> String {
    format!(
        "Two recent successful agent sessions shared a recurring tool-call sequence. \
         Generalize it into a reusable procedure that can be DISPATCHED by an automated executor.\n\n\
         STRICT REQUIREMENTS:\n\
         - Each step's `action` MUST be one of the following registered tool names: {whitelist}.\n\
         - Each step's `args` MUST be a JSON object containing the tool's required arguments.\n\
         - Use `{{step_N.field}}` (e.g., `{{step_0.stdout}}`) to reference previous step output.\n\
         - `binds` lists field names to extract from a step's result for later interpolation. \
           Omit (empty list) for steps whose output isn't referenced.\n\
         - Procedures must be parameterizable: top-level `parameters` lists `{{parameter}}` names \
           used in step args.\n\n\
         Return ONLY JSON:\n\
         {{\n\
           \"name\": snake_case_string,\n\
           \"description\": string,\n\
           \"trigger_pattern\": string,\n\
           \"parameters\": [string],\n\
           \"steps\": [\n\
             {{\"action\": string, \"args\": object, \"binds\": [string], \"note\": string|null}}\n\
           ]\n\
         }}\n\n\
         Session A task: {sa}\n\
         Session A tool sequence: {ta:?}\n\n\
         Session B task: {sb}\n\
         Session B tool sequence: {tb:?}\n\n\
         Matched prefix: {mp:?}",
        whitelist = input.tool_whitelist,
        sa = input.task_summary_a,
        ta = input.tool_sequence_a,
        sb = input.task_summary_b,
        tb = input.tool_sequence_b,
        mp = input.matched_prefix,
    )
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
    use zbot_stores_sqlite::{
        CompactionRepository, DatabaseManager, EpisodeRepository, GatewayCompactionStore,
        GatewayEpisodeStore, KnowledgeDatabase, Procedure,
    };
    use zbot_stores_traits::StoreResult;

    struct MockLlm {
        response: Mutex<PatternResponse>,
        calls: Mutex<u64>,
    }

    impl MockLlm {
        fn new(resp: PatternResponse) -> Self {
            Self {
                response: Mutex::new(resp),
                calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl PatternExtractLlm for MockLlm {
        async fn generalize(&self, _input: &PatternInput) -> Result<PatternResponse, String> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.response.lock().unwrap().clone())
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        knowledge_db: Arc<KnowledgeDatabase>,
        conversations_db: Arc<DatabaseManager>,
        compaction_repo: Arc<CompactionRepository>,
        episode_store: Arc<dyn EpisodeStore>,
        message_store: Arc<dyn zbot_conversation::MessageStore>,
        procedure_store: Arc<dyn ProcedureStore>,
        compaction_store: Arc<dyn CompactionStore>,
    }

    fn setup() -> Harness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let conversation_pool =
            zbot_conversation::open_conversation_pool(&paths.conversations_db())
                .expect("conversation pool");
        let knowledge_db = Arc::new(KnowledgeDatabase::new(paths.clone()).expect("knowledge db"));
        let conversations_db = Arc::new(DatabaseManager::new(paths).expect("convo db"));
        let compaction_repo = Arc::new(CompactionRepository::new(knowledge_db.clone()));

        let episode_vec: Arc<dyn zbot_stores_sqlite::VectorIndex> = Arc::new(
            zbot_stores_sqlite::SqliteVecIndex::new(
                knowledge_db.clone(),
                "session_episodes_index",
                "episode_id",
            )
            .expect("episode vec idx"),
        );
        let episode_repo = Arc::new(EpisodeRepository::new(knowledge_db.clone(), episode_vec));
        let episode_store: Arc<dyn EpisodeStore> = Arc::new(GatewayEpisodeStore::new(episode_repo));
        let message_store: Arc<dyn zbot_conversation::MessageStore> = Arc::new(
            zbot_conversation::SqliteMessageStore::new(conversation_pool),
        );
        let procedure_store = test_support::procedure_store(&tmp);
        let compaction_store: Arc<dyn CompactionStore> =
            Arc::new(GatewayCompactionStore::new(compaction_repo.clone()));
        Harness {
            _tmp: tmp,
            knowledge_db,
            conversations_db,
            compaction_repo,
            episode_store,
            message_store,
            procedure_store,
            compaction_store,
        }
    }

    fn normalize(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n < 1e-9 {
            v
        } else {
            v.into_iter().map(|x| x / n).collect()
        }
    }

    fn f32_to_blob(v: &[f32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(v.len() * 4);
        for f in v {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out
    }

    /// Seed two successful session_episodes whose embeddings are identical
    /// (cosine = 1.0) and corresponding messages rows with identical 4-step
    /// tool-call sequences.
    fn seed_pair(h: &Harness, agent_id: &str) -> (String, String) {
        let now = chrono::Utc::now().to_rfc3339();
        let emb: Vec<f32> = normalize((0..384).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect());
        let blob = f32_to_blob(&emb);

        for (ep_id, sess_id, summary) in [
            (
                "ep-A",
                "sess-A",
                "Investigate and fix postgres connection timeout",
            ),
            (
                "ep-B",
                "sess-B",
                "Investigate and fix postgres pool exhaustion",
            ),
        ] {
            h.knowledge_db
                .with_connection(|conn| {
                    conn.execute(
                        "INSERT INTO session_episodes
                            (id, session_id, agent_id, ward_id, task_summary, outcome, created_at)
                         VALUES (?1, ?2, ?3, '__global__', ?4, 'success', ?5)",
                        params![ep_id, sess_id, agent_id, summary, now],
                    )?;
                    conn.execute(
                        "INSERT INTO session_episodes_index (episode_id, embedding) VALUES (?1, ?2)",
                        params![ep_id, blob],
                    )?;
                    Ok(())
                })
                .expect("seed episode");
        }

        // Identical 4-step tool-call sequences for both sessions.
        let tool_seq = serde_json::json!([
            {"tool_id": "t1", "tool_name": "search_docs", "args": {}},
            {"tool_id": "t2", "tool_name": "read_file", "args": {}},
            {"tool_id": "t3", "tool_name": "run_query", "args": {}},
            {"tool_id": "t4", "tool_name": "summarize", "args": {}}
        ])
        .to_string();

        for sess_id in ["sess-A", "sess-B"] {
            h.conversations_db
                .with_connection(|conn| {
                    conn.execute(
                        "INSERT INTO sessions (id, status, source, root_agent_id, created_at)
                         VALUES (?1, 'completed', 'web', ?2, ?3)",
                        params![sess_id, agent_id, now],
                    )?;
                    conn.execute(
                        "INSERT INTO messages (id, session_id, role, content, created_at, tool_calls)
                         VALUES (?1, ?2, 'assistant', '', ?3, ?4)",
                        params![format!("msg-{sess_id}"), sess_id, now, tool_seq],
                    )?;
                    Ok(())
                })
                .expect("seed message");
        }

        ("ep-A".to_string(), "ep-B".to_string())
    }

    fn ok_response(name: &str) -> PatternResponse {
        PatternResponse {
            name: name.to_string(),
            description: "Investigate, then read, then query, then summarize".to_string(),
            trigger_pattern: "investigate postgres * issue".to_string(),
            parameters: vec!["target".to_string()],
            steps: vec![
                PatternStep {
                    action: "search_docs".to_string(),
                    args: Default::default(),
                    binds: Vec::new(),
                    agent: None,
                    note: None,
                    task_template: None,
                },
                PatternStep {
                    action: "read_file".to_string(),
                    args: Default::default(),
                    binds: Vec::new(),
                    agent: None,
                    note: None,
                    task_template: None,
                },
                PatternStep {
                    action: "run_query".to_string(),
                    args: Default::default(),
                    binds: Vec::new(),
                    agent: None,
                    note: None,
                    task_template: None,
                },
                PatternStep {
                    action: "summarize".to_string(),
                    args: Default::default(),
                    binds: Vec::new(),
                    agent: None,
                    note: None,
                    task_template: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn extracts_pattern_across_two_sessions() {
        let h = setup();
        let agent_id = "agent-px";
        seed_pair(&h, agent_id);

        let mock = Arc::new(MockLlm::new(ok_response("investigate_postgres_issue")));
        let ext = PatternExtractor::new(
            h.episode_store.clone(),
            h.message_store.clone(),
            h.procedure_store.clone(),
            h.compaction_store.clone(),
            mock.clone(),
            None,
            vec!["shell".into(), "read_file".into()],
        );

        let stats = ext.run_cycle("run-pe-1").await.expect("run_cycle");
        assert_eq!(stats.episodes_considered, 2);
        assert_eq!(stats.pairs_evaluated, 1);
        assert_eq!(stats.pairs_matched, 1);
        assert_eq!(stats.llm_calls_made, 1);
        assert_eq!(stats.procedures_inserted, 1);
        assert_eq!(*mock.calls.lock().unwrap(), 1);

        let names = h
            .procedure_store
            .list_procedure_names(agent_id, 10)
            .await
            .expect("list procedure names");
        assert_eq!(names.len(), 1, "exactly one procedure inserted: {names:?}");
        assert_eq!(names[0].0, "investigate_postgres_issue");

        let rows = h.compaction_repo.list_run("run-pe-1").expect("list_run");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].operation, "pattern_extract");
        assert_eq!(
            rows[0].entity_id.as_deref().is_some(),
            true,
            "audit row names the inserted procedure id"
        );
    }

    #[tokio::test]
    async fn skips_when_existing_procedure_is_locked() {
        let h = setup();
        let agent_id = "agent-px-dup";
        seed_pair(&h, agent_id);

        // Pre-existing procedure with success_count >= DEDUP_SUCCESS_FLOOR.
        let existing = Procedure {
            id: "proc-existing".to_string(),
            agent_id: agent_id.to_string(),
            ward_id: Some("__global__".to_string()),
            name: "investigate_postgres_issue".to_string(),
            description: "locked".to_string(),
            trigger_pattern: None,
            steps: "[]".to_string(),
            parameters: None,
            success_count: 5,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        h.procedure_store
            .upsert_procedure(existing, None)
            .await
            .expect("seed proc");

        let mock = Arc::new(MockLlm::new(ok_response("investigate_postgres_issue")));
        let ext = PatternExtractor::new(
            h.episode_store.clone(),
            h.message_store.clone(),
            h.procedure_store.clone(),
            h.compaction_store.clone(),
            mock,
            None,
            Vec::new(),
        );
        let stats = ext.run_cycle("run-pe-2").await.expect("run_cycle");
        assert_eq!(stats.procedures_inserted, 0);
        assert_eq!(stats.skipped_existing, 1);

        let rows = h.compaction_repo.list_run("run-pe-2").expect("list_run");
        assert!(rows.is_empty());
    }

    #[test]
    fn longest_common_prefix_basics() {
        let a = vec!["x", "y", "z", "q"];
        let b = vec!["x", "y", "z", "r"];
        let lcp = longest_common_prefix(&a, &b);
        assert_eq!(lcp, vec!["x", "y", "z"]);
        let c = vec!["p", "y", "z"];
        assert!(longest_common_prefix(&a, &c).is_empty());
    }

    #[test]
    fn cosine_threshold_behavior() {
        let a = normalize(vec![1.0, 0.0, 0.0]);
        let b = normalize(vec![1.0, 0.0, 0.0]);
        assert!(cosine_f64(&a, &b) >= PAIR_COSINE_THRESHOLD);
        let c = normalize(vec![0.0, 1.0, 0.0]);
        assert!(cosine_f64(&a, &c) < PAIR_COSINE_THRESHOLD);
        // ~0.85 > threshold
        let d = normalize(vec![0.85, 0.5267, 0.0]);
        assert!(cosine_f64(&a, &d) >= PAIR_COSINE_THRESHOLD);
    }

    #[test]
    fn sanitize_name_collapses_nonalnum() {
        assert_eq!(
            sanitize_name("Investigate Postgres!!Issue"),
            "investigate_postgres_issue"
        );
        assert_eq!(sanitize_name("   "), "");
        assert_eq!(sanitize_name("a b c"), "a_b_c");
    }

    // Note: `extend_tool_names_parses_stored_format` previously tested
    // a helper that lived here. The helper moved to
    // `zbot_stores_sqlite::repository::extend_tool_names_from_blob`
    // when the conversation read became trait-routed in Phase D4 —
    // see the SQLite-side `tool_sequence_for_session` impl for the
    // current behaviour test (covered indirectly via this module's
    // `extracts_pattern_across_two_sessions`).

    // ------------------------------------------------------------------
    // Task 2: verify the extractor populates `embedding` before insert.
    // ------------------------------------------------------------------

    use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
    use zbot_stores_traits::ProcedureSummary;

    /// `ProcedureStore` that captures the last `insert_pattern_procedure`
    /// request so the test can assert on the inserted shape.
    #[derive(Default)]
    struct CapturingProcedureStore {
        captured: Mutex<Option<PatternProcedureInsert>>,
    }

    #[async_trait]
    impl ProcedureStore for CapturingProcedureStore {
        async fn get_procedure_summary_by_name(
            &self,
            _agent_id: &str,
            _name: &str,
        ) -> StoreResult<Option<ProcedureSummary>> {
            Ok(None)
        }
        async fn insert_pattern_procedure(
            &self,
            req: PatternProcedureInsert,
        ) -> StoreResult<String> {
            *self.captured.lock().unwrap() = Some(req);
            Ok("proc-test".into())
        }
    }

    /// Embedding client that always returns the same fixed-length vector.
    struct FixedEmbeddingClient {
        vec: Vec<f32>,
    }
    #[async_trait]
    impl EmbeddingClient for FixedEmbeddingClient {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            Ok(texts.iter().map(|_| self.vec.clone()).collect())
        }
        fn dimensions(&self) -> usize {
            self.vec.len()
        }
        fn model_name(&self) -> String {
            "test-fixed".into()
        }
    }

    /// `CompactionStore` no-op so `commit_pattern` doesn't need a real DB.
    /// Default impls of the trait cover every method — explicit empty body
    /// here just to give us a concrete type to instantiate.
    struct NoopCompactionStore;
    #[async_trait]
    impl CompactionStore for NoopCompactionStore {}

    #[tokio::test]
    async fn extractor_passes_embedding_to_store() {
        // Drive `commit_pattern` directly — it's the helper that builds
        // and submits the `PatternProcedureInsert`, which is the exact
        // surface we want to verify carries an embedding.
        let store = Arc::new(CapturingProcedureStore::default());
        let embed = Arc::new(FixedEmbeddingClient {
            vec: vec![0.5_f32; 384],
        });

        // Stub stores for the deps `commit_pattern` doesn't actually touch.
        let h = setup();
        let mock_llm = Arc::new(MockLlm::new(ok_response("placeholder")));

        let ext = PatternExtractor::new(
            h.episode_store.clone(),
            h.message_store.clone(),
            store.clone() as Arc<dyn ProcedureStore>,
            Arc::new(NoopCompactionStore) as Arc<dyn CompactionStore>,
            mock_llm,
            Some(embed.clone() as Arc<dyn EmbeddingClient>),
            Vec::new(),
        );

        let mut stats = PatternStats::default();
        let resp = ok_response("investigate_postgres_issue");
        ext.commit_pattern("run-emb", "agent-emb", &resp, &mut stats)
            .await;

        assert_eq!(stats.procedures_inserted, 1);
        let captured = store.captured.lock().unwrap();
        let inserted = captured.as_ref().expect("no insert captured");
        assert_eq!(inserted.embedding.as_ref().map(Vec::len), Some(384));
    }

    #[test]
    fn pattern_step_deserializes_old_format_without_args_or_binds() {
        let old = r#"{"action": "read_file"}"#;
        let step: PatternStep = serde_json::from_str(old).unwrap();
        assert_eq!(step.action, "read_file");
        assert!(step.args.is_empty());
        assert!(step.binds.is_empty());
    }

    #[test]
    fn pattern_step_deserializes_new_format() {
        let new = r#"{
            "action": "shell",
            "args": {"cmd": "cargo test {test_name}"},
            "binds": ["assertion"]
        }"#;
        let step: PatternStep = serde_json::from_str(new).unwrap();
        assert_eq!(step.action, "shell");
        assert_eq!(
            step.args.get("cmd").and_then(|v| v.as_str()),
            Some("cargo test {test_name}")
        );
        assert_eq!(step.binds, vec!["assertion".to_string()]);
    }

    #[test]
    fn pattern_prompt_includes_tool_whitelist_and_strict_requirements() {
        let input = PatternInput {
            task_summary_a: "Investigate postgres timeout".into(),
            task_summary_b: "Investigate postgres pool exhaustion".into(),
            tool_sequence_a: vec!["shell".into(), "read".into()],
            tool_sequence_b: vec!["shell".into(), "read".into()],
            matched_prefix: vec!["shell".into(), "read".into()],
            tool_whitelist: "shell, read, grep, edit_file".into(),
        };

        let prompt = build_pattern_prompt(&input);

        // Anchor checks to the rendered CSV whitelist so they survive future
        // prompt edits without matching incidental substrings (e.g. "read"
        // inside a longer word).
        assert!(
            prompt.contains("shell, read, grep, edit_file"),
            "prompt should render the tool whitelist verbatim"
        );
        assert!(
            prompt.contains(", read,"),
            "prompt should include the real 'read' tool name from the whitelist"
        );
        assert!(
            prompt.contains("STRICT REQUIREMENTS"),
            "prompt should declare strict requirements"
        );
        assert!(prompt.contains("args"), "prompt should mention args field");
        assert!(
            prompt.contains("binds"),
            "prompt should mention binds field"
        );
    }
}
