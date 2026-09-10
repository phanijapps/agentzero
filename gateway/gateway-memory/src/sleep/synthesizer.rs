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

    // NOTE (KG-lane port): the cross-session strategy tests that lived here
    // exercised `list_strategy_candidates` /
    // `relationship_context_for_entity` / `episode_ids_for_entity` — the
    // episode-attribution pathway. The engram write path never records
    // `source_episode_ids` (verified against production: 0 of 2,915
    // relationships carry attribution), so those trait ops remain no-op
    // defaults on the adapter and the synthesizer cycle is a quiet no-op in
    // production. The tests died with the sqlite reference implementation;
    // restoring them requires recording episode attribution on the engram
    // write path first.

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
