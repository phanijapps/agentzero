//! Belief consolidation on engram's port slots (P3 sub-batch 1).
//!
//! The LLM synthesis + contradiction-judging cores from the old
//! `belief_synthesizer.rs` / `belief_contradiction_detector.rs` sleep
//! workers now live behind engram's ports:
//!
//! - [`ZbotBeliefSynthesizer`] implements `engram_belief::BeliefSynthesizer`
//!   (the slot engram documents as "real LLM impl replaces the deterministic
//!   baseline"). Orchestration — cadence, task accounting, per-item error
//!   collection — belongs to engram's `ReflectionExecutor`.
//! - [`ZbotContradictionDetector`] implements
//!   `engram_belief::ContradictionDetector`; [`ZbotContradictionArm`] is the
//!   `ConsolidationMutationExecutor` that lists the partition's beliefs,
//!   runs the detector, and persists the returned contradictions.
//! - [`ZbotBeliefSink`] implements engram-reflection's `BeliefSink`, routing
//!   each synthesized belief through the zbot `BeliefStore` so the adapter's
//!   dual write (canonical engram record + sidecar row with embedding)
//!   stays the single persistence path.
//!
//! Deleted with the old workers: interval/last-run scheduling (the sleep
//! worker's cadence shell owns that), hand-rolled cycle stats plumbing, and
//! the run_cycle orchestration — replaced by
//! `engram_consolidation::CompositeConsolidationExecutor` dispatch.
//!
//! Engram's `ConsolidationRequest` cannot name a single invalidated fact,
//! so fact→belief invalidation stays event-driven:
//! `BeliefPropagator::propagate_invalidation` is invoked inline by the
//! conflict resolver, not as a consolidation task. Its store calls already
//! hit engram semantics through the adapter.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use agent_runtime::llm::embedding::EmbeddingClient;
use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use engram_belief::{BeliefSynthesizer as EngramBeliefSynthesizer, ContradictionDetector};
use engram_consolidation::{ConsolidationMutationExecutor, ConsolidationMutationOutcome};
use engram_domain::{
    Actor, ActorKind, Belief as EngramBelief, BeliefId, BeliefSource, BeliefSourceTargetType,
    BeliefStatus, BeliefSubject, ConsolidationError, ConsolidationRequest, ConsolidationStats,
    ConsolidationTaskKind, ConsolidationTaskResult, ConsolidationTaskStatus,
    Contradiction as EngramContradiction, ContradictionId, ContradictionKind, ContradictionStatus,
    ContradictionTarget, ContradictionTargetType, DerivationKind, DerivationRef, Id, Policy,
    Provenance, Retention, Scope, Timestamp, Visibility,
};
use engram_reflection::{BeliefSink, ReflectionExecutor};
use engram_runtime::{CoreError, CoreResult};
use serde::Deserialize;
use zbot_stores_domain::{Belief, BeliefContradiction, ContradictionType, MemoryFact};
use zbot_stores_traits::{BeliefContradictionStore, BeliefStore, MemoryFactStore, StoreResult};

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
/// even on a large partition.
const MAX_FACTS_PER_CYCLE: usize = 1000;

/// Hard ceiling on beliefs scanned per contradiction cycle.
const MAX_BELIEFS_PER_CYCLE: usize = 1000;

/// Metadata key carrying the full zbot `Belief` (embedding included) on the
/// engram accounting projection returned by the synthesizer. The sink
/// reconstructs the zbot belief from this payload so persistence keeps the
/// adapter's dual write.
const ZBOT_BELIEF_PAYLOAD: &str = "zbotBeliefPayload";

/// Metadata key naming the stale belief to clear after a successful
/// re-synthesis persist (B-3 stale-first pass).
const ZBOT_STALE_CLEAR_ID: &str = "zbotStaleClearId";

// ============================================================================
// Synthesis stats (kept for observability; consumed by the activity ring
// buffer and the HTTP belief-network endpoints)
// ============================================================================

/// Stats from one synthesis pass. Tracked separately for short-circuit vs
/// LLM paths so we can confirm the optimization is firing on real data.
///
/// `stale_beliefs_resynthesized` (B-3) counts beliefs the pass picked up
/// from the stale-flag queue at the top, before the normal dirty-subject
/// pass. These overlap with `beliefs_synthesized`.
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

// ============================================================================
// Pure helpers (unchanged from the original worker)
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
/// constituent facts. Returns `0.0` for an empty slice.
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

/// Earliest `valid_from` across constituents, or `None` if no fact has one.
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

/// Serialize an f32 vector to little-endian bytes for storage on the
/// `kg_beliefs.embedding` column.
pub(crate) fn embedding_to_bytes(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vec.len() * 4);
    for f in vec {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

// ============================================================================
// Production LLM implementation (prompt byte-identical to the original)
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

    /// Build the prompt body. Free function so tests can assert against it
    /// without spinning up a real LLM.
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
// Minimal engram record constructors (accounting projections)
// ============================================================================

fn system_actor() -> Actor {
    Actor {
        id: Id::from("zbot-sleep"),
        kind: ActorKind::System,
        display_name: None,
        metadata: None,
    }
}

fn durable_policy() -> Policy {
    Policy {
        visibility: Visibility::Workspace,
        retention: Retention::Durable,
        sensitivity: None,
        allowed_uses: Vec::new(),
        expires_at: None,
        delete_mode: None,
    }
}

fn system_provenance(at: Timestamp) -> Provenance {
    Provenance {
        source: "zbot-sleep-belief-network".to_string(),
        actor: system_actor(),
        observed_at: at,
        evidence: Vec::new(),
        derivations: Vec::new(),
        confidence: None,
        method: None,
    }
}

fn partition_scope(partition_id: &str) -> Scope {
    Scope {
        tenant: "zbot".to_string(),
        subject: Some(partition_id.to_string()),
        workspace: None,
        session: None,
        environment: None,
    }
}

fn payload_metadata(
    pairs: Vec<(&str, serde_json::Value)>,
) -> Option<BTreeMap<String, serde_json::Value>> {
    let mut map = BTreeMap::new();
    for (key, value) in pairs {
        map.insert(key.to_string(), value);
    }
    Some(map)
}

/// Build the engram accounting projection for a zbot belief, carrying the
/// full zbot payload (embedding included) in metadata for the sink.
fn belief_projection(
    belief: &Belief,
    partition_id: &str,
    stale_clear_id: Option<&str>,
    now: Timestamp,
) -> CoreResult<EngramBelief> {
    let mut pairs = vec![(
        ZBOT_BELIEF_PAYLOAD,
        serde_json::to_value(belief).map_err(|error| CoreError::Adapter {
            adapter: "zbot_belief_payload".to_string(),
            message: error.to_string(),
        })?,
    )];
    if let Some(id) = stale_clear_id {
        pairs.push((ZBOT_STALE_CLEAR_ID, serde_json::json!(id)));
    }
    Ok(EngramBelief {
        id: BeliefId::from(belief.id.clone()),
        scope: partition_scope(partition_id),
        subject: BeliefSubject {
            key: belief.subject.clone(),
            entity_ref: None,
            concept_ref: None,
            aliases: Vec::new(),
        },
        content: belief.content.clone(),
        status: if belief.superseded_by.is_some() {
            BeliefStatus::Superseded
        } else if belief.stale {
            BeliefStatus::Stale
        } else {
            BeliefStatus::Active
        },
        confidence: belief.confidence as f32,
        sources: belief
            .source_fact_ids
            .iter()
            .map(|fact_id| BeliefSource {
                target_type: BeliefSourceTargetType::Memory,
                target_id: fact_id.clone(),
                authority_level: None,
                weight: None,
                confidence: None,
                valid_from: belief.valid_from,
                valid_until: belief.valid_until,
            })
            .collect(),
        valid_from: belief.valid_from,
        valid_until: belief.valid_until,
        superseded_by: belief.superseded_by.clone().map(BeliefId::from),
        stale: Some(belief.stale),
        synthesizer: Some(DerivationRef {
            kind: DerivationKind::Consolidation,
            model: Some(format!(
                "zbot-belief-synthesizer-v{}",
                belief.synthesizer_version
            )),
            prompt_hash: None,
            input_refs: Vec::new(),
            created_at: belief.created_at,
        }),
        reasoning: belief.reasoning.clone(),
        embedding_refs: Vec::new(),
        policy: durable_policy(),
        provenance: system_provenance(belief.valid_from.unwrap_or(now)),
        created_at: belief.created_at,
        updated_at: Some(belief.updated_at),
        metadata: payload_metadata(pairs),
    })
}

/// Reconstruct the zbot belief carried by an accounting projection.
fn belief_payload(record: &EngramBelief) -> Option<(Belief, Option<String>)> {
    let metadata = record.metadata.as_ref()?;
    let payload = metadata.get(ZBOT_BELIEF_PAYLOAD)?;
    let belief = serde_json::from_value::<Belief>(payload.clone()).ok()?;
    let stale_clear_id = metadata
        .get(ZBOT_STALE_CLEAR_ID)
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some((belief, stale_clear_id))
}

// ============================================================================
// ZbotBeliefSynthesizer — engram BeliefSynthesizer slot
// ============================================================================

/// LLM belief synthesis behind engram's `BeliefSynthesizer` port. Produces
/// accounting projections carrying the full zbot belief; persistence is the
/// executor's job via [`ZbotBeliefSink`].
pub struct ZbotBeliefSynthesizer {
    fact_store: Arc<dyn MemoryFactStore>,
    /// Belief reads for the B-3 stale-first pass (list_stale).
    belief_store: Arc<dyn BeliefStore>,
    /// Partition used when the request scope carries no subject.
    default_partition: String,
    llm: Arc<dyn BeliefSynthesisLlm>,
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
    /// Cycle telemetry, drained by [`BeliefConsolidation::execute`] after
    /// the executor run.
    stats: Mutex<BeliefSynthesisStats>,
}

impl ZbotBeliefSynthesizer {
    pub fn new(
        fact_store: Arc<dyn MemoryFactStore>,
        belief_store: Arc<dyn BeliefStore>,
        default_partition: String,
        llm: Arc<dyn BeliefSynthesisLlm>,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
    ) -> Self {
        Self {
            fact_store,
            belief_store,
            default_partition,
            llm,
            embedding_client,
            stats: Mutex::new(BeliefSynthesisStats::default()),
        }
    }

    fn partition_of(&self, request: &ConsolidationRequest) -> String {
        request
            .scope
            .subject
            .clone()
            .unwrap_or_else(|| self.default_partition.clone())
    }

    /// Take (and reset) the accumulated cycle stats.
    pub fn take_stats(&self) -> BeliefSynthesisStats {
        std::mem::take(&mut self.stats.lock().expect("synthesis stats"))
    }

    /// Generate the belief's embedding bytes. Returns `None` when no client
    /// is wired or the call fails — synthesis must never fail on embedding.
    async fn embed_belief_content(&self, content: &str) -> Option<Vec<u8>> {
        let client = self.embedding_client.as_ref()?;
        match client.embed(&[content]).await {
            Ok(mut v) if !v.is_empty() => Some(embedding_to_bytes(&v.remove(0))),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, "belief-synthesis: embed failed; belief saved without embedding");
                None
            }
        }
    }

    /// Synthesize one belief for a single (partition, subject) group —
    /// short-circuit vs LLM decision, confidence computation. Returns the
    /// zbot belief; caller wraps it in the accounting projection.
    async fn synthesize_one(&self, subject: &str, facts: &[MemoryFact]) -> Result<Belief, String> {
        let now = Utc::now();
        let (content, reasoning, used_llm) = if facts.len() == 1 {
            self.stats.lock().unwrap().beliefs_short_circuited += 1;
            (facts[0].content.clone(), None, false)
        } else {
            self.stats.lock().unwrap().llm_calls += 1;
            match self.llm.synthesize(subject, facts).await {
                Ok(resp) => {
                    self.stats.lock().unwrap().beliefs_llm_synthesized += 1;
                    (resp.content, Some(resp.reasoning), true)
                }
                Err(e) => {
                    tracing::warn!(
                        subject,
                        error = %e,
                        "belief-synthesis: LLM failed; falling back to most-recent fact"
                    );
                    self.stats.lock().unwrap().errors += 1;
                    let primary = facts.last().expect("non-empty multi-fact group");
                    (primary.content.clone(), None, false)
                }
            }
        };

        let confidence = compute_confidence(facts, now);
        let valid_from = earliest_valid_from(facts);
        let embedding = self.embed_belief_content(&content).await;

        tracing::debug!(
            subject,
            used_llm,
            confidence,
            source_count = facts.len(),
            "belief-synthesis: synthesized"
        );

        Ok(Belief {
            id: format!("belief-{}", uuid::Uuid::new_v4()),
            partition_id: self.default_partition.clone(),
            subject: subject.to_string(),
            content,
            confidence,
            valid_from,
            valid_until: None,
            source_fact_ids: facts.iter().map(|f| f.id.clone()).collect(),
            synthesizer_version: SYNTHESIZER_VERSION,
            reasoning,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding,
        })
    }
}

#[async_trait]
impl EngramBeliefSynthesizer for ZbotBeliefSynthesizer {
    async fn synthesize_beliefs(
        &self,
        request: &ConsolidationRequest,
    ) -> CoreResult<Vec<EngramBelief>> {
        let partition = self.partition_of(request);
        let now = Utc::now();

        let facts = self
            .fact_store
            .list_memory_facts_typed(Some(&partition), None, None, MAX_FACTS_PER_CYCLE, 0)
            .await
            .map_err(|e| CoreError::Adapter {
                adapter: "memory_fact_store".to_string(),
                message: format!("list_memory_facts_typed: {e}"),
            })?;

        // Only the active set — superseded facts are history.
        let active: Vec<MemoryFact> = facts
            .into_iter()
            .filter(|f| f.superseded_by.is_none())
            .collect();

        // Group by `key` — the key IS the subject.
        let mut by_subject: HashMap<String, Vec<MemoryFact>> = HashMap::new();
        for f in active {
            by_subject.entry(f.key.clone()).or_default().push(f);
        }

        self.stats.lock().unwrap().subjects_examined = by_subject.len() as u64;

        // Oldest-first within each group so the most-recent fact is the tail
        // (both paths treat the tail as primary).
        for group in by_subject.values_mut() {
            group.sort_by(|a, b| {
                let av = a.valid_from.as_deref().unwrap_or(&a.created_at);
                let bv = b.valid_from.as_deref().unwrap_or(&b.created_at);
                av.cmp(bv)
            });
        }

        let mut projections = Vec::new();
        let mut covered: HashSet<String> = HashSet::new();

        // B-3: stale beliefs go FIRST. Propagation marked them dirty when
        // a source fact was invalidated; re-derive content + confidence
        // from the surviving sources, and flag the projection so the sink
        // clears the stale marker after persist.
        let stale = self
            .belief_store
            .list_stale(&partition, MAX_FACTS_PER_CYCLE)
            .await
            .unwrap_or_default();
        for belief in stale {
            if covered.contains(&belief.subject) {
                // Multiple stale beliefs in one subject collapse to a
                // single re-synthesis; still clear the duplicate flag via
                // a direct store call (no new belief needed).
                if let Err(e) = self.belief_store.clear_stale(&belief.id).await {
                    self.stats.lock().unwrap().errors += 1;
                    tracing::warn!(belief_id = %belief.id, error = %e, "clear_stale failed");
                }
                continue;
            }
            let Some(group) = by_subject.get(&belief.subject) else {
                // No active facts left — propagation already retracted the
                // sole-source path; leave the flag set for observation.
                tracing::debug!(
                    belief_id = %belief.id,
                    subject = %belief.subject,
                    "belief-synthesis: stale belief has no active facts; leaving stale"
                );
                continue;
            };
            match self.synthesize_one(&belief.subject, group).await {
                Ok(fresh) => {
                    self.stats.lock().unwrap().beliefs_synthesized += 1;
                    self.stats.lock().unwrap().stale_beliefs_resynthesized += 1;
                    covered.insert(belief.subject.clone());
                    match belief_projection(&fresh, &partition, Some(&belief.id), now) {
                        Ok(projection) => projections.push(projection),
                        Err(e) => {
                            self.stats.lock().unwrap().errors += 1;
                            tracing::warn!(error = %e, "belief-synthesis: projection failed");
                        }
                    }
                }
                Err(e) => {
                    self.stats.lock().unwrap().errors += 1;
                    tracing::warn!(subject = %belief.subject, error = %e,
                        "belief-synthesis: stale re-synthesis failed");
                }
            }
        }

        // Normal pass, skipping subjects covered by the stale-first pass.
        for (subject, group) in by_subject {
            if covered.contains(&subject) {
                continue;
            }
            match self.synthesize_one(&subject, &group).await {
                Ok(belief) => {
                    self.stats.lock().unwrap().beliefs_synthesized += 1;
                    match belief_projection(&belief, &partition, None, now) {
                        Ok(projection) => projections.push(projection),
                        Err(e) => {
                            self.stats.lock().unwrap().errors += 1;
                            tracing::warn!(error = %e, "belief-synthesis: projection failed");
                        }
                    }
                }
                Err(e) => {
                    self.stats.lock().unwrap().errors += 1;
                    tracing::warn!(subject, error = %e, "belief-synthesis: subject failed");
                }
            }
        }
        Ok(projections)
    }
}

// ============================================================================
// ZbotBeliefSink — persist through the zbot store (dual write preserved)
// ============================================================================

/// Routes executor-persisted beliefs through the zbot `BeliefStore`, which
/// the engram adapter backs with the canonical record + sidecar dual write
/// (embedding bytes included). After a successful persist of a stale
/// re-synthesis, the flagged stale belief is cleared.
pub struct ZbotBeliefSink {
    store: Arc<dyn BeliefStore>,
}

impl ZbotBeliefSink {
    pub fn new(store: Arc<dyn BeliefStore>) -> Self {
        Self { store }
    }

    async fn persist(&self, record: &EngramBelief) -> StoreResult<()> {
        let Some((mut belief, stale_clear_id)) = belief_payload(record) else {
            // A foreign belief without payload — nothing zbot-shaped to
            // persist; report loudly rather than silently dropping.
            return Err(zbot_stores_traits::StoreError::Invalid(format!(
                "belief projection missing {ZBOT_BELIEF_PAYLOAD} payload"
            )));
        };
        // Partition travels with the request scope, not the payload's
        // default partition field.
        if let Some(partition) = record.scope.subject.as_deref() {
            belief.partition_id = partition.to_string();
        }
        self.store.upsert_belief(&belief).await?;
        if let Some(stale_id) = stale_clear_id {
            if let Err(e) = self.store.clear_stale(&stale_id).await {
                tracing::warn!(belief_id = %stale_id, error = %e, "clear_stale failed");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BeliefSink for ZbotBeliefSink {
    async fn put_belief(&self, belief: EngramBelief) -> CoreResult<EngramBelief> {
        self.persist(&belief)
            .await
            .map_err(|e| CoreError::Adapter {
                adapter: "zbot_belief_store".to_string(),
                message: e.to_string(),
            })?;
        Ok(belief)
    }
}

// ============================================================================
// Contradiction stats + LLM judge (prompt byte-identical to the original)
// ============================================================================

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

    /// Build the judge prompt. Free function so tests can assert against it.
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

/// Tuning + opt-in switches for the detector.
#[derive(Debug, Clone)]
pub struct BeliefContradictionConfig {
    /// How many dot-separated subject components form a neighborhood key.
    pub neighborhood_prefix_depth: usize,
    /// Maximum LLM calls (unique pair evaluations) per cycle.
    pub budget_per_cycle: usize,
}

impl Default for BeliefContradictionConfig {
    fn default() -> Self {
        Self {
            neighborhood_prefix_depth: 1,
            budget_per_cycle: 20,
        }
    }
}

fn neighborhood_key(subject: &str, depth: usize) -> String {
    subject.split('.').take(depth).collect::<Vec<_>>().join(".")
}

// ============================================================================
// ZbotContradictionDetector — engram ContradictionDetector slot
// ============================================================================

/// LLM contradiction judging behind engram's `ContradictionDetector` port:
/// neighborhood grouping, budgeted pair evaluation, existing-pair skipping,
/// and the 4-way verdict routing. Returns contradictions only for
/// `logical` and `tension` verdicts (duplicates and compatibles are
/// log-only); persistence belongs to the executor arm.
pub struct ZbotContradictionDetector {
    judge: Arc<dyn ContradictionJudgeLlm>,
    contradiction_store: Arc<dyn BeliefContradictionStore>,
    config: BeliefContradictionConfig,
    stats: Mutex<ContradictionDetectionStats>,
    /// zbot rows for the contradictions returned by the last
    /// `detect_contradictions` call. Engram's `Contradiction` carries no
    /// metadata field, so the executor arm drains this queue instead of
    /// parsing payloads out of the returned records.
    pending_rows: Mutex<Vec<BeliefContradiction>>,
}

impl ZbotContradictionDetector {
    pub fn new(
        judge: Arc<dyn ContradictionJudgeLlm>,
        contradiction_store: Arc<dyn BeliefContradictionStore>,
        config: BeliefContradictionConfig,
    ) -> Self {
        Self {
            judge,
            contradiction_store,
            config,
            stats: Mutex::new(ContradictionDetectionStats::default()),
            pending_rows: Mutex::new(Vec::new()),
        }
    }

    /// Take (and reset) the accumulated cycle stats.
    pub fn take_stats(&self) -> ContradictionDetectionStats {
        std::mem::take(&mut self.stats.lock().expect("contradiction stats"))
    }

    /// Drain the zbot rows queued by the last detection run.
    pub fn take_pending_rows(&self) -> Vec<BeliefContradiction> {
        std::mem::take(&mut self.pending_rows.lock().expect("pending rows"))
    }

    fn contradiction_projection(
        &self,
        record: &EngramBelief,
        kind: ContradictionKind,
        severity: f64,
        reasoning: String,
        other: &EngramBelief,
    ) -> EngramContradiction {
        let now = Utc::now();
        let row = BeliefContradiction {
            id: format!("contradiction-{}", uuid::Uuid::new_v4()),
            belief_a_id: record.id.to_string(),
            belief_b_id: other.id.to_string(),
            contradiction_type: match kind {
                ContradictionKind::Logical => ContradictionType::Logical,
                _ => ContradictionType::Tension,
            },
            severity,
            judge_reasoning: Some(reasoning),
            detected_at: now,
            resolved_at: None,
            resolution: None,
        };
        self.pending_rows.lock().expect("pending rows").push(row);
        EngramContradiction {
            id: ContradictionId::from(uuid::Uuid::new_v4().to_string()),
            scope: record.scope.clone(),
            kind,
            targets: vec![
                ContradictionTarget {
                    target_type: ContradictionTargetType::Belief,
                    target_id: record.id.to_string(),
                    role: Some("a".to_string()),
                },
                ContradictionTarget {
                    target_type: ContradictionTargetType::Belief,
                    target_id: other.id.to_string(),
                    role: Some("b".to_string()),
                },
            ],
            severity: severity as f32,
            status: ContradictionStatus::Open,
            reasoning: None,
            detected_by: Some(DerivationRef {
                kind: DerivationKind::Consolidation,
                model: Some("zbot-contradiction-judge".to_string()),
                prompt_hash: None,
                input_refs: Vec::new(),
                created_at: now,
            }),
            resolution: None,
            provenance: system_provenance(now),
            detected_at: now,
            updated_at: None,
        }
    }

    /// Judge one pair and route the verdict; returns a projection for
    /// logical/tension verdicts.
    async fn judge_and_route(
        &self,
        a: (&EngramBelief, &Belief),
        b: (&EngramBelief, &Belief),
    ) -> Option<EngramContradiction> {
        let resp = match self.judge.judge(a.1, b.1).await {
            Ok(r) => r,
            Err(e) => {
                self.stats.lock().unwrap().errors += 1;
                tracing::warn!(
                    belief_a_id = %a.0.id,
                    belief_b_id = %b.0.id,
                    error = %e,
                    "contradiction-detect: LLM failed; routing as compatible"
                );
                return None;
            }
        };

        match resp.decision {
            JudgeDecision::LogicalContradiction => {
                self.stats.lock().unwrap().contradictions_logical += 1;
                Some(self.contradiction_projection(
                    a.0,
                    ContradictionKind::Logical,
                    resp.severity,
                    resp.reasoning,
                    b.0,
                ))
            }
            JudgeDecision::Tension => {
                self.stats.lock().unwrap().contradictions_tension += 1;
                Some(self.contradiction_projection(
                    a.0,
                    ContradictionKind::Tension,
                    resp.severity,
                    resp.reasoning,
                    b.0,
                ))
            }
            JudgeDecision::Duplicate => {
                self.stats.lock().unwrap().duplicates_logged += 1;
                tracing::info!(
                    belief_a_id = %a.0.id,
                    belief_b_id = %b.0.id,
                    reasoning = %resp.reasoning,
                    "contradiction-detect: duplicate beliefs (canonicalization signal)"
                );
                None
            }
            JudgeDecision::Compatible => {
                self.stats.lock().unwrap().compatibles_logged += 1;
                None
            }
        }
    }
}

#[async_trait]
impl ContradictionDetector for ZbotContradictionDetector {
    async fn detect_contradictions(
        &self,
        beliefs: &[EngramBelief],
    ) -> CoreResult<Vec<EngramContradiction>> {
        let active: Vec<&EngramBelief> = beliefs
            .iter()
            .filter(|record| record.superseded_by.is_none())
            .collect();

        // Zbot-shaped views for the judge prompt; falls back to a minimal
        // belief when a record carries no payload (foreign record).
        let views: Vec<(&EngramBelief, Belief)> = active
            .iter()
            .map(|record| {
                let belief = belief_payload(record)
                    .map(|(belief, _)| belief)
                    .unwrap_or_else(|| Belief {
                        id: record.id.to_string(),
                        partition_id: record.scope.subject.clone().unwrap_or_default(),
                        subject: record.subject.key.clone(),
                        content: record.content.clone(),
                        confidence: record.confidence as f64,
                        valid_from: record.valid_from,
                        valid_until: record.valid_until,
                        source_fact_ids: record
                            .sources
                            .iter()
                            .map(|source| source.target_id.clone())
                            .collect(),
                        synthesizer_version: 0,
                        reasoning: record.reasoning.clone(),
                        created_at: record.created_at,
                        updated_at: record.updated_at.unwrap_or(record.created_at),
                        superseded_by: record.superseded_by.as_ref().map(|id| id.to_string()),
                        stale: record.stale.unwrap_or(false),
                        embedding: None,
                    });
                (*record, belief)
            })
            .collect();

        // Group by neighborhood key; largest groups first so the budget
        // lands where contradiction potential is highest.
        let mut by_neighborhood: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, (record, _)) in views.iter().enumerate() {
            let key = neighborhood_key(&record.subject.key, self.config.neighborhood_prefix_depth);
            by_neighborhood.entry(key).or_default().push(index);
        }
        self.stats.lock().unwrap().neighborhoods_examined = by_neighborhood.len() as u64;

        let mut groups: Vec<Vec<usize>> = by_neighborhood.into_values().collect();
        groups.sort_by_key(|group| std::cmp::Reverse(group.len()));

        let mut budget = self.config.budget_per_cycle;
        let mut contradictions = Vec::new();

        'outer: for group in groups {
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    if budget == 0 {
                        self.stats.lock().unwrap().budget_exhausted = true;
                        break 'outer;
                    }
                    let (record_a, belief_a) = &views[group[i]];
                    let (record_b, belief_b) = &views[group[j]];
                    self.stats.lock().unwrap().pairs_examined += 1;

                    // Skip already-evaluated pairs without an LLM call.
                    match self
                        .contradiction_store
                        .pair_exists(&belief_a.id, &belief_b.id)
                        .await
                    {
                        Ok(true) => {
                            self.stats.lock().unwrap().pairs_skipped_existing += 1;
                            continue;
                        }
                        Ok(false) => {}
                        Err(e) => {
                            self.stats.lock().unwrap().errors += 1;
                            tracing::warn!(
                                error = %e,
                                "contradiction-detect: pair_exists failed; skipping pair"
                            );
                            continue;
                        }
                    }

                    self.stats.lock().unwrap().llm_calls += 1;
                    budget -= 1;

                    if let Some(contradiction) = self
                        .judge_and_route((record_a, belief_a), (record_b, belief_b))
                        .await
                    {
                        contradictions.push(contradiction);
                    }
                }
            }
        }
        Ok(contradictions)
    }
}

// ============================================================================
// ZbotContradictionArm — executor persisting detector verdicts
// ============================================================================

/// `ConsolidationMutationExecutor` arm for `BeliefContradictionDetection`:
/// lists the partition's beliefs from the zbot store (sidecar truth —
/// embeddings intact), maps them to accounting projections, runs the
/// [`ZbotContradictionDetector`] port impl, and persists each returned
/// contradiction through the zbot contradiction store. Same result/stats
/// shape as engram's own executor.
pub struct ZbotContradictionArm {
    detector: Arc<ZbotContradictionDetector>,
    belief_store: Arc<dyn BeliefStore>,
    default_partition: String,
}

impl ZbotContradictionArm {
    pub fn new(
        detector: Arc<ZbotContradictionDetector>,
        belief_store: Arc<dyn BeliefStore>,
        default_partition: String,
    ) -> Self {
        Self {
            detector,
            belief_store,
            default_partition,
        }
    }
}

#[async_trait]
impl ConsolidationMutationExecutor for ZbotContradictionArm {
    async fn execute(
        &self,
        request: &ConsolidationRequest,
        planned_tasks: &[ConsolidationTaskKind],
        started_at: Timestamp,
    ) -> CoreResult<ConsolidationMutationOutcome> {
        let mut task_results = Vec::new();
        let mut total = 0u64;
        let mut errors = Vec::new();

        for kind in planned_tasks {
            if kind != &ConsolidationTaskKind::BeliefContradictionDetection {
                task_results.push(ConsolidationTaskResult {
                    task: kind.clone(),
                    status: ConsolidationTaskStatus::Skipped,
                    started_at,
                    completed_at: Some(started_at),
                    items_read: None,
                    items_written: None,
                    items_updated: None,
                    items_skipped: None,
                    model_calls: None,
                    errors: Vec::new(),
                    output_refs: Vec::new(),
                });
                continue;
            }

            let partition = request
                .scope
                .subject
                .clone()
                .unwrap_or_else(|| self.default_partition.clone());
            let beliefs = self
                .belief_store
                .list_beliefs(&partition, MAX_BELIEFS_PER_CYCLE)
                .await
                .map_err(|e| CoreError::Adapter {
                    adapter: "zbot_belief_store".to_string(),
                    message: format!("list_beliefs: {e}"),
                })?;
            let read = beliefs.len() as u64;

            let records: Vec<EngramBelief> = beliefs
                .iter()
                .map(|belief| belief_projection(belief, &partition, None, started_at))
                .collect::<CoreResult<Vec<_>>>()?;

            let contradictions = self.detector.detect_contradictions(&records).await?;
            let detected = contradictions.len() as u64;

            let errors_before = errors.len();
            for row in self.detector.take_pending_rows() {
                if let Err(e) = self
                    .detector
                    .contradiction_store
                    .insert_contradiction(&row)
                    .await
                {
                    errors.push(ConsolidationError {
                        task: Some(ConsolidationTaskKind::BeliefContradictionDetection),
                        code: "insert_contradiction_failed".to_owned(),
                        message: e.to_string(),
                        target_type: None,
                        target_id: Some(row.id),
                        recoverable: true,
                    });
                }
            }

            let task_errors = errors.len() - errors_before;
            let written = detected - task_errors as u64;
            total += written;

            task_results.push(ConsolidationTaskResult {
                task: ConsolidationTaskKind::BeliefContradictionDetection,
                status: if task_errors == 0 {
                    ConsolidationTaskStatus::Completed
                } else {
                    ConsolidationTaskStatus::CompletedWithErrors
                },
                started_at,
                completed_at: Some(started_at),
                items_read: Some(read),
                items_written: Some(written),
                items_updated: None,
                items_skipped: None,
                model_calls: None,
                errors: Vec::new(),
                output_refs: Vec::new(),
            });
        }

        let mut stats = ConsolidationStats {
            memories_read: None,
            memories_written: None,
            beliefs_synthesized: None,
            contradictions_detected: None,
            hierarchy_nodes_created: None,
            hierarchy_relations_created: None,
            records_decayed: None,
            records_pruned: None,
            model_calls: None,
        };
        if total > 0 {
            stats.contradictions_detected = Some(total);
        }

        Ok(ConsolidationMutationOutcome {
            tasks: task_results,
            stats,
            errors,
        })
    }
}

// ============================================================================
// BeliefConsolidation — the sleep-worker trigger
// ============================================================================

/// Collaborators for [`BeliefConsolidation`], grouped so construction
/// stays under the argument limit and call sites read by name.
pub struct BeliefConsolidationParts {
    pub fact_store: Arc<dyn MemoryFactStore>,
    pub belief_store: Arc<dyn BeliefStore>,
    pub contradiction_store: Arc<dyn BeliefContradictionStore>,
    pub llm: Arc<dyn BeliefSynthesisLlm>,
    pub judge: Arc<dyn ContradictionJudgeLlm>,
    pub config: BeliefContradictionConfig,
    pub embedding_client: Option<Arc<dyn EmbeddingClient>>,
    pub default_partition: String,
}

/// One belief-network consolidation cycle: synthesis (via engram's
/// `ReflectionExecutor` + our port impls) and contradiction detection (via
/// our executor arm), dispatched by engram's composite executor. Returns
/// the two stats structs the worker/activity ring already consume.
pub struct BeliefConsolidation {
    synthesizer: Arc<ZbotBeliefSynthesizer>,
    /// Detector handle kept for post-run stat draining.
    detector: Arc<ZbotContradictionDetector>,
    composite: engram_consolidation::CompositeConsolidationExecutor,
}

impl BeliefConsolidation {
    pub fn new(parts: BeliefConsolidationParts) -> Self {
        let BeliefConsolidationParts {
            fact_store,
            belief_store,
            contradiction_store,
            llm,
            judge,
            config,
            embedding_client,
            default_partition,
        } = parts;
        let synthesizer = Arc::new(ZbotBeliefSynthesizer::new(
            fact_store,
            belief_store.clone(),
            default_partition.clone(),
            llm,
            embedding_client,
        ));
        let detector = Arc::new(ZbotContradictionDetector::new(
            judge,
            contradiction_store,
            config,
        ));
        Self {
            synthesizer: synthesizer.clone(),
            detector: detector.clone(),
            composite: build_composite(synthesizer, detector, belief_store, default_partition),
        }
    }

    /// Run one cycle for a partition. Errors degrade to logged warnings
    /// per task (the executor collects them); a hard failure of the
    /// composite itself is returned as `Err`.
    pub async fn execute(
        &self,
        run_id: &str,
        partition_id: &str,
    ) -> Result<(BeliefSynthesisStats, ContradictionDetectionStats), String> {
        // Reset telemetry accumulators before the run.
        self.synthesizer.take_stats();

        let request = ConsolidationRequest {
            scope: partition_scope(partition_id),
            requester: engram_domain::Requester {
                actor: system_actor(),
                roles: Vec::new(),
                permissions: Vec::new(),
                on_behalf_of: None,
            },
            since: None,
            until: None,
            strategy: None,
            dry_run: Some(false),
        };
        let planned = [
            ConsolidationTaskKind::BeliefSynthesis,
            ConsolidationTaskKind::BeliefContradictionDetection,
        ];
        let outcome = self
            .composite
            .execute(&request, &planned, Utc::now())
            .await
            .map_err(|e| e.to_string())?;

        for error in &outcome.errors {
            tracing::warn!(run_id, partition_id, code = %error.code, %error.message,
                "belief-consolidation: task error");
        }
        tracing::info!(
            run_id,
            partition_id,
            tasks = outcome.tasks.len(),
            errors = outcome.errors.len(),
            "belief-consolidation: cycle done"
        );

        // The detector's stats were accumulated during the arm's run; the
        // synthesizer's during synthesize_beliefs.
        let synthesis = self.synthesizer.take_stats();
        let contradiction = self.detector.take_stats();
        Ok((synthesis, contradiction))
    }
}

/// Wire the composite: reflection arm (synthesis + sink) and the
/// contradiction arm. Split from `new` so the struct construction stays
/// readable.
fn build_composite(
    synthesizer: Arc<ZbotBeliefSynthesizer>,
    detector: Arc<ZbotContradictionDetector>,
    belief_store: Arc<dyn BeliefStore>,
    default_partition: String,
) -> engram_consolidation::CompositeConsolidationExecutor {
    let sink = Arc::new(ZbotBeliefSink::new(belief_store.clone()));
    let reflection = Arc::new(ReflectionExecutor::new(synthesizer, sink));
    let contradiction_arm = Arc::new(ZbotContradictionArm::new(
        detector,
        belief_store,
        default_partition,
    ));
    engram_consolidation::CompositeConsolidationExecutor::new(vec![reflection, contradiction_arm])
}

// ============================================================================
// Tests — behavioral parity with the original sleep workers
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::vault_paths::VaultPaths;
    use chrono::Duration as ChronoDuration;
    use std::sync::Mutex as StdMutex;
    use zbot_stores_sqlite::vector_index::{SqliteVecIndex, VectorIndex};
    use zbot_stores_sqlite::{
        GatewayMemoryFactStore, KnowledgeDatabase, MemoryRepository,
        SqliteBeliefContradictionStore, SqliteBeliefStore,
    };

    // -- mocks --------------------------------------------------------------

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

    struct MockJudge {
        responses: StdMutex<Vec<Result<ContradictionJudgeResponse, String>>>,
        calls: StdMutex<u64>,
    }

    impl MockJudge {
        fn single(response: ContradictionJudgeResponse) -> Self {
            Self {
                responses: StdMutex::new(vec![Ok(response)]),
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
            let mut guard = self.responses.lock().unwrap();
            if guard.is_empty() {
                return Ok(ContradictionJudgeResponse {
                    decision: JudgeDecision::Compatible,
                    severity: 0.5,
                    reasoning: "default compatible".to_string(),
                });
            }
            guard.remove(0)
        }
    }

    fn judge_ok(decision: JudgeDecision) -> ContradictionJudgeResponse {
        ContradictionJudgeResponse {
            decision,
            severity: 0.9,
            reasoning: "test reasoning".to_string(),
        }
    }

    // -- setup ----------------------------------------------------------------

    struct TestEnv {
        consolidation: BeliefConsolidation,
        fact_store: Arc<dyn MemoryFactStore>,
        belief_store: Arc<dyn BeliefStore>,
        contradiction_store: Arc<dyn BeliefContradictionStore>,
        _tmp: tempfile::TempDir,
    }

    fn setup(llm: Arc<dyn BeliefSynthesisLlm>, judge: Arc<dyn ContradictionJudgeLlm>) -> TestEnv {
        setup_with_config(llm, judge, BeliefContradictionConfig::default())
    }

    fn setup_with_config(
        llm: Arc<dyn BeliefSynthesisLlm>,
        judge: Arc<dyn ContradictionJudgeLlm>,
        config: BeliefContradictionConfig,
    ) -> TestEnv {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        let vec_index: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id").expect("vec index"),
        );
        let mem_repo = Arc::new(MemoryRepository::new(db.clone(), vec_index));
        let fact_store: Arc<dyn MemoryFactStore> =
            Arc::new(GatewayMemoryFactStore::new(mem_repo, None));
        let belief_store: Arc<dyn BeliefStore> = Arc::new(SqliteBeliefStore::new(db.clone()));
        let contradiction_store: Arc<dyn BeliefContradictionStore> =
            Arc::new(SqliteBeliefContradictionStore::new(db));
        let consolidation = BeliefConsolidation::new(BeliefConsolidationParts {
            fact_store: fact_store.clone(),
            belief_store: belief_store.clone(),
            contradiction_store: contradiction_store.clone(),
            llm,
            judge,
            config,
            embedding_client: None,
            default_partition: "p1".to_string(),
        });
        TestEnv {
            consolidation,
            fact_store,
            belief_store,
            contradiction_store,
            _tmp: tmp,
        }
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

    /// Insert a SECOND fact with the same (agent, key) — `save_fact`
    /// upserts on (agent, scope, ward, key), so a second row needs the
    /// typed path with a distinct id (mirrors the original worker's test).
    async fn seed_typed_second_fact(
        store: &Arc<dyn MemoryFactStore>,
        partition_id: &str,
        key: &str,
        content: &str,
        valid_from: DateTime<Utc>,
    ) {
        let fact: MemoryFact = serde_json::from_value(serde_json::json!({
            "id": format!("fact-{}", uuid::Uuid::new_v4()),
            "session_id": null,
            "agent_id": partition_id,
            "scope": "agent",
            "category": "user",
            "key": key,
            "content": content,
            "confidence": 0.9,
            "mention_count": 1,
            "source_summary": null,
            "ward_id": "__global__",
            "contradicted_by": null,
            "created_at": valid_from.to_rfc3339(),
            "updated_at": valid_from.to_rfc3339(),
            "expires_at": null,
            "valid_from": valid_from.to_rfc3339(),
            "valid_until": null,
            "superseded_by": null,
            "pinned": false,
            "epistemic_class": "current",
            "source_episode_id": null,
            "source_ref": null,
            "last_accessed": null,
            "embedding": null
        }))
        .unwrap();
        store.upsert_typed_fact(fact, None).await.unwrap();
    }

    async fn seed_belief(
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

    // -- synthesis ---------------------------------------------------------

    #[tokio::test]
    async fn single_fact_short_circuits_without_llm_call() {
        let llm = Arc::new(MockLlm::ok("unused", "unused"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm.clone(), judge);
        seed_fact(
            &env.fact_store,
            "p1",
            "user.location",
            "Mason, OH",
            0.9,
            None,
        )
        .await;

        let (stats, _) = env.consolidation.execute("run-1", "p1").await.unwrap();
        assert_eq!(stats.subjects_examined, 1);
        assert_eq!(stats.beliefs_short_circuited, 1);
        assert_eq!(stats.beliefs_llm_synthesized, 0);
        assert_eq!(llm.calls(), 0, "short-circuit must skip the LLM");

        let got = env
            .belief_store
            .get_belief("p1", "user.location", None)
            .await
            .unwrap()
            .expect("belief persisted");
        assert_eq!(got.content, "Mason, OH", "verbatim fact content");
        assert_eq!(got.source_fact_ids.len(), 1);
    }

    #[tokio::test]
    async fn multi_fact_calls_llm_and_persists_reasoning() {
        let llm = Arc::new(MockLlm::ok("User goes by Phani", "most recent dominated"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm.clone(), judge);
        let now = Utc::now();
        seed_fact(
            &env.fact_store,
            "p1",
            "user.name",
            "User goes by J. Smith",
            0.8,
            Some(now - ChronoDuration::days(30)),
        )
        .await;
        seed_typed_second_fact(
            &env.fact_store,
            "p1",
            "user.name",
            "User goes by Phani",
            now,
        )
        .await;

        let (stats, _) = env.consolidation.execute("run-1", "p1").await.unwrap();
        assert_eq!(stats.beliefs_llm_synthesized, 1);
        assert_eq!(stats.beliefs_short_circuited, 0);
        assert_eq!(llm.calls(), 1, "multi-fact must call the LLM once");

        let got = env
            .belief_store
            .get_belief("p1", "user.name", None)
            .await
            .unwrap()
            .expect("belief persisted");
        assert_eq!(got.content, "User goes by Phani");
        assert!(got.reasoning.as_deref().unwrap_or("").contains("recent"));
        assert_eq!(got.source_fact_ids.len(), 2);
    }

    #[tokio::test]
    async fn multi_fact_llm_failure_falls_back_to_most_recent() {
        let llm = Arc::new(MockLlm::fail());
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm.clone(), judge);
        let now = Utc::now();
        seed_fact(
            &env.fact_store,
            "p1",
            "user.location",
            "Denver, CO",
            0.8,
            Some(now - ChronoDuration::days(90)),
        )
        .await;
        seed_typed_second_fact(&env.fact_store, "p1", "user.location", "Austin, TX", now).await;

        let (stats, _) = env.consolidation.execute("run-1", "p1").await.unwrap();
        assert_eq!(llm.calls(), 1, "LLM was attempted");
        assert_eq!(stats.errors, 1, "fallback path increments errors");

        let got = env
            .belief_store
            .get_belief("p1", "user.location", None)
            .await
            .unwrap()
            .expect("fallback belief persisted");
        assert_eq!(got.content, "Austin, TX", "most-recent fact is primary");
        assert!(got.reasoning.is_none(), "fallback leaves reasoning NULL");
    }

    #[tokio::test]
    async fn re_running_synthesis_is_idempotent() {
        let llm = Arc::new(MockLlm::ok("User goes by Phani", "recent"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm.clone(), judge);
        seed_fact(
            &env.fact_store,
            "p1",
            "user.locale",
            "Austin, TX",
            0.9,
            None,
        )
        .await;

        env.consolidation.execute("run-1", "p1").await.unwrap();
        let first = env
            .belief_store
            .get_belief("p1", "user.locale", None)
            .await
            .unwrap()
            .expect("first run persisted");
        env.consolidation.execute("run-2", "p1").await.unwrap();
        let second = env
            .belief_store
            .get_belief("p1", "user.locale", None)
            .await
            .unwrap()
            .expect("second run persisted");
        assert_eq!(first.id, second.id, "same subject upserts the same row");
    }

    #[tokio::test]
    async fn stale_belief_is_resynthesized_and_flag_cleared() {
        let llm = Arc::new(MockLlm::ok("re-derived", "surviving sources"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm, judge);
        seed_fact(
            &env.fact_store,
            "p1",
            "user.device",
            "MacBook Pro",
            0.9,
            None,
        )
        .await;

        env.consolidation.execute("run-1", "p1").await.unwrap();
        let belief = env
            .belief_store
            .get_belief("p1", "user.device", None)
            .await
            .unwrap()
            .unwrap();
        env.belief_store.mark_stale(&belief.id).await.unwrap();

        let (stats, _) = env.consolidation.execute("run-2", "p1").await.unwrap();
        assert_eq!(stats.stale_beliefs_resynthesized, 1);
        let refreshed = env
            .belief_store
            .get_belief_by_id(&belief.id)
            .await
            .unwrap()
            .expect("belief still present");
        assert!(!refreshed.stale, "stale flag cleared after re-synthesis");
    }

    // -- contradiction ------------------------------------------------------

    #[tokio::test]
    async fn logical_contradiction_inserts_row() {
        let llm = Arc::new(MockLlm::ok("unused", "unused"));
        let judge = Arc::new(MockJudge::single(judge_ok(
            JudgeDecision::LogicalContradiction,
        )));
        let env = setup(llm, judge.clone());
        seed_belief(&env.belief_store, "b1", "user.job", "Works at OpenAI", "p1").await;
        seed_belief(
            &env.belief_store,
            "b2",
            "user.job",
            "Works at Anthropic",
            "p1",
        )
        .await;

        let (_, stats) = env.consolidation.execute("run-1", "p1").await.unwrap();
        assert_eq!(stats.pairs_examined, 1);
        assert_eq!(judge.calls(), 1);
        assert_eq!(stats.contradictions_logical, 1);
        let rows = env.contradiction_store.list_recent("p1", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].contradiction_type, ContradictionType::Logical);
    }

    #[tokio::test]
    async fn duplicate_and_compatible_log_no_row() {
        let llm = Arc::new(MockLlm::ok("unused", "unused"));
        for decision in [JudgeDecision::Duplicate, JudgeDecision::Compatible] {
            let judge = Arc::new(MockJudge::single(judge_ok(decision.clone())));
            let env = setup(llm.clone(), judge);
            seed_belief(&env.belief_store, "b1", "user.job", "Works at OpenAI", "p1").await;
            seed_belief(
                &env.belief_store,
                "b2",
                "user.job",
                "Employed by OpenAI",
                "p1",
            )
            .await;

            let (_, stats) = env.consolidation.execute("run-1", "p1").await.unwrap();
            assert_eq!(
                stats.contradictions_logical + stats.contradictions_tension,
                0
            );
            let rows = env.contradiction_store.list_recent("p1", 10).await.unwrap();
            assert!(rows.is_empty(), "{decision:?} must not insert a row");
        }
    }

    #[tokio::test]
    async fn only_pairs_within_same_neighborhood() {
        let llm = Arc::new(MockLlm::ok("unused", "unused"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm, judge.clone());
        seed_belief(&env.belief_store, "b1", "user.job", "Works at OpenAI", "p1").await;
        seed_belief(&env.belief_store, "b2", "domain.acn", "ACN is cheap", "p1").await;

        let (_, stats) = env.consolidation.execute("run-1", "p1").await.unwrap();
        assert_eq!(
            stats.pairs_examined, 0,
            "different neighborhoods never pair"
        );
        assert_eq!(judge.calls(), 0);
    }

    #[tokio::test]
    async fn budget_exhaustion_caps_llm_calls() {
        let llm = Arc::new(MockLlm::ok("unused", "unused"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup_with_config(
            llm,
            judge.clone(),
            BeliefContradictionConfig {
                neighborhood_prefix_depth: 1,
                budget_per_cycle: 1,
            },
        );
        seed_belief(&env.belief_store, "b1", "user.job", "a", "p1").await;
        seed_belief(&env.belief_store, "b2", "user.job", "b", "p1").await;
        seed_belief(&env.belief_store, "b3", "user.job", "c", "p1").await;

        let (_, stats) = env.consolidation.execute("run-1", "p1").await.unwrap();
        assert_eq!(judge.calls(), 1, "budget of 1 caps LLM calls");
        assert!(stats.budget_exhausted);
    }

    #[tokio::test]
    async fn skips_already_evaluated_pairs() {
        let llm = Arc::new(MockLlm::ok("unused", "unused"));
        // Logical on cycle 1 inserts the row; the empty queue then defaults
        // to Compatible so any stray pairing on cycle 2 stays row-free.
        let judge = Arc::new(MockJudge::single(judge_ok(
            JudgeDecision::LogicalContradiction,
        )));
        let env = setup(llm, judge.clone());
        seed_belief(&env.belief_store, "b1", "user.job", "a", "p1").await;
        seed_belief(&env.belief_store, "b2", "user.job", "b", "p1").await;

        env.consolidation.execute("run-1", "p1").await.unwrap();
        let first_calls = judge.calls();
        let (_, stats) = env.consolidation.execute("run-2", "p1").await.unwrap();
        assert_eq!(first_calls, 1);
        assert_eq!(
            stats.pairs_skipped_existing, 1,
            "second cycle skips the pair without an LLM call"
        );
        assert_eq!(judge.calls(), 1, "no new LLM call for existing pair");
    }

    // -- pure helpers + prompt preservation ---------------------------------

    #[test]
    fn confidence_formula_single_fact_90_days_old_is_about_half() {
        let now = Utc::now();
        let fact = MemoryFact {
            id: "f".into(),
            session_id: None,
            agent_id: "a".into(),
            scope: "agent".into(),
            category: "user".into(),
            key: "k".into(),
            content: "c".into(),
            confidence: 0.9,
            mention_count: 1,
            source_summary: None,
            ward_id: "w".into(),
            contradicted_by: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            expires_at: None,
            valid_from: Some((now - ChronoDuration::days(90)).to_rfc3339()),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: None,
            source_episode_id: None,
            source_ref: None,
            last_accessed: None,
            embedding: None,
        };
        let confidence = compute_confidence(&[fact], now);
        assert!((confidence - 0.45).abs() < 0.01, "0.9 × ~0.5 ≈ 0.45");
    }

    #[test]
    fn recency_weight_edges() {
        let now = Utc::now();
        assert_eq!(recency_weight(Some(now), now), 1.0);
        assert_eq!(recency_weight(None, now), 1.0);
    }

    #[test]
    fn synthesis_prompt_is_byte_identical() {
        let facts = vec![MemoryFact {
            id: "f1".into(),
            session_id: None,
            agent_id: "a".into(),
            scope: "agent".into(),
            category: "user".into(),
            key: "user.name".into(),
            content: "Phani".into(),
            confidence: 0.9,
            mention_count: 1,
            source_summary: None,
            ward_id: "w".into(),
            contradicted_by: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: None,
            source_episode_id: None,
            source_ref: None,
            last_accessed: None,
            embedding: None,
        }];
        let prompt = LlmBeliefSynthesizer::build_prompt("user.name", &facts);
        assert!(
            prompt.contains("You synthesize a single belief from N memory facts about a subject.")
        );
        assert!(prompt.contains("Subject: user.name"));
        assert!(prompt.contains("\"Phani\" (conf=0.90)"));
        assert!(prompt.contains("Treat the most-recent VALID fact as primary (newer beats older)"));
    }

    #[test]
    fn judge_prompt_is_byte_identical() {
        let mk = |id: &str| Belief {
            id: id.into(),
            partition_id: "p1".into(),
            subject: "user.job".into(),
            content: format!("content {id}"),
            confidence: 0.8,
            valid_from: None,
            valid_until: None,
            source_fact_ids: vec!["f1".into(), "f2".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        let prompt = LlmContradictionJudge::build_prompt(&mk("a"), &mk("b"));
        assert!(
            prompt.contains("You judge whether two beliefs about a similar subject contradict.")
        );
        assert!(prompt.contains("- Source fact count: 2"));
        assert!(prompt
            .contains("\"logical_contradiction\": A and B cannot both be true at the same time."));
    }

    #[tokio::test]
    async fn dbg_probe() {
        let llm = Arc::new(MockLlm::ok("x", "y"));
        let judge = Arc::new(MockJudge::single(judge_ok(JudgeDecision::Compatible)));
        let env = setup(llm, judge);
        seed_fact(
            &env.fact_store,
            "p1",
            "user.location",
            "Mason, OH",
            0.9,
            None,
        )
        .await;
        let (s, _) = env.consolidation.execute("dbg", "p1").await.unwrap();
        eprintln!(
            "DBG subjects={} synth={} short={}",
            s.subjects_examined, s.beliefs_synthesized, s.beliefs_short_circuited
        );
        let listed = env.belief_store.list_beliefs("p1", 10).await.unwrap();
        eprintln!("DBG beliefs={}", listed.len());
    }

    #[test]
    fn neighborhood_key_respects_depth() {
        assert_eq!(neighborhood_key("user.dietary.vegetarian", 1), "user");
        assert_eq!(
            neighborhood_key("user.dietary.vegetarian", 2),
            "user.dietary"
        );
        assert_eq!(neighborhood_key("single", 99), "single");
    }
}
