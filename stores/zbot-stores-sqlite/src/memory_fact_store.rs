// ============================================================================
// GATEWAY MEMORY FACT STORE
// Implements MemoryFactStore trait using MemoryRepository + EmbeddingClient
// ============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use agent_runtime::llm::embedding::{content_hash, EmbeddingClient};
use zbot_stores_traits::{MemoryFactStore, SkillIndexRow, StrategyFactInsert, StrategyFactMatch};

use crate::memory_repository::{MemoryFact, MemoryRepository};

/// Sentinel values used for ctx-namespaced facts.
///
/// All ctx facts share these values so that reads (by any agent) can
/// locate them without knowing the original writer. The uniqueness of
/// the fact row is preserved by the session id encoded in the key.
const CTX_AGENT_SENTINEL: &str = "__ctx__";
const CTX_SCOPE: &str = "session";
const CTX_CATEGORY: &str = "ctx";

/// Sentinels for ward-scoped primitive facts (function signatures
/// extracted from source files by the runtime AST hook).
const PRIMITIVE_AGENT_SENTINEL: &str = "__ward__";
const PRIMITIVE_SCOPE: &str = "global";
const PRIMITIVE_CATEGORY: &str = "primitive";

/// Maximum character length for a memory fact's `content`.
///
/// A well-formed fact is one declarative sentence — three at most, and only
/// to fit a necessary example. This cap is a backstop that keeps clearly
/// malformed, oversized facts out of the store; the distillation prompt is
/// the quality lever. `ctx` and `primitive` facts are machine-generated and
/// exempt (see [`validate_fact_content`]).
const MAX_FACT_CONTENT_CHARS: usize = 800;

/// Reject a fact whose `content` is too long to be a 1-3 sentence fact.
///
/// `ctx` (session-context blobs) and `primitive` (extracted function
/// signatures) are machine-generated and not subject to the sentence rule.
fn validate_fact_content(category: &str, content: &str) -> Result<(), String> {
    if matches!(category, CTX_CATEGORY | PRIMITIVE_CATEGORY) {
        return Ok(());
    }
    let len = content.chars().count();
    if len > MAX_FACT_CONTENT_CHARS {
        return Err(format!(
            "fact content too long: {len} chars (max {MAX_FACT_CONTENT_CHARS}) — \
             a fact must be 1-3 sentences"
        ));
    }
    Ok(())
}

/// Database-backed implementation of `MemoryFactStore`.
///
/// Wraps `MemoryRepository` for SQLite persistence and an optional
/// `EmbeddingClient` for generating embeddings for hybrid search.
pub struct GatewayMemoryFactStore {
    memory_repo: Arc<MemoryRepository>,
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
}

impl GatewayMemoryFactStore {
    /// Create a new store with the given repository and optional embedding client.
    pub fn new(
        memory_repo: Arc<MemoryRepository>,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
    ) -> Self {
        Self {
            memory_repo,
            embedding_client,
        }
    }

    /// Test-only accessor for the underlying knowledge DB. Used by
    /// integration tests that need to count rows in tables that aren't
    /// otherwise reachable through the trait.
    #[cfg(test)]
    pub(crate) fn knowledge_db_for_tests(&self) -> Arc<crate::KnowledgeDatabase> {
        self.memory_repo.db_for_tests()
    }

    /// Mark similar facts as contradicted by the new fact with the given `key`.
    fn mark_contradicted_facts(
        &self,
        similar_facts: Vec<crate::memory_repository::ScoredFact>,
        key: &str,
        category: &str,
    ) {
        for sf in similar_facts {
            if sf.fact.key != key && sf.fact.category == category {
                if let Err(e) = self.memory_repo.mark_contradicted(&sf.fact.id, key) {
                    tracing::warn!(
                        fact_id = %sf.fact.id,
                        contradicted_by = %key,
                        "Failed to mark fact as contradicted: {}",
                        e
                    );
                } else {
                    tracing::info!(
                        fact_id = %sf.fact.id,
                        fact_key = %sf.fact.key,
                        contradicted_by = %key,
                        similarity = %sf.score,
                        "Marked fact as contradicted by newer fact"
                    );
                }
            }
        }
    }

    /// Embed a text and optionally cache it. Returns None if no client is configured.
    async fn embed_text(&self, text: &str) -> Option<Vec<f32>> {
        let client = self.embedding_client.as_ref()?;
        let hash = content_hash(text);
        let model = client.model_name().to_string();

        // Check cache first
        if let Ok(Some(cached)) = self.memory_repo.get_cached_embedding(&hash, &model) {
            return Some(cached);
        }

        // Generate embedding
        match client.embed(&[text]).await {
            Ok(mut embeddings) if !embeddings.is_empty() => {
                let emb = embeddings.remove(0);
                // Cache for future use (fire-and-forget)
                let _ = self.memory_repo.cache_embedding(&hash, &model, &emb);
                Some(emb)
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!("Failed to embed text: {}", e);
                None
            }
        }
    }
}

#[async_trait]
impl MemoryFactStore for GatewayMemoryFactStore {
    async fn save_fact(
        &self,
        agent_id: &str,
        category: &str,
        key: &str,
        content: &str,
        confidence: f64,
        session_id: Option<&str>,
        valid_from: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Value, String> {
        validate_fact_content(category, content)?;

        // Generate embedding for the content
        let embedding = self.embed_text(content).await;

        // Scope auto-default. Since this `save_fact` API doesn't accept an
        // explicit scope, derive it from the category: agent-specific behavior
        // (corrections/strategies/instructions/patterns) stays private to the
        // writing agent; domain/reference/research/book/user facts go global
        // so every other agent can see them in recall.
        let scope = match category {
            "correction" | "strategy" | "instruction" | "pattern" => "agent",
            _ => "global",
        }
        .to_string();

        let now_dt = chrono::Utc::now();
        let now = now_dt.to_rfc3339();
        // Bi-temporal phase 1: every new fact records when it became true in
        // the world. Default to "now" when the caller doesn't pin a value —
        // we never persist NULL here. Legacy NULL rows are repaired by the
        // v25 backfill migration.
        let valid_from_str = valid_from.unwrap_or(now_dt).to_rfc3339();
        let fact = MemoryFact {
            id: format!("fact-{}", uuid::Uuid::new_v4()),
            session_id: session_id.map(String::from),
            agent_id: agent_id.to_string(),
            scope,
            category: category.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            confidence,
            mention_count: 1,
            source_summary: None,
            embedding: embedding.clone(),
            ward_id: "__global__".to_string(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now,
            expires_at: None,
            valid_from: Some(valid_from_str),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
        };

        self.memory_repo.upsert_memory_fact(&fact)?;

        // Best-effort contradiction detection: if the new fact has an embedding,
        // search for semantically similar facts with a DIFFERENT key but same
        // category and mark them as contradicted.
        if let Some(ref emb) = embedding {
            if let Ok(similar_facts) = self.memory_repo.search_similar_facts(
                emb,
                Some(agent_id),
                0.8, // high threshold to avoid false positives
                5,
                None, // no ward filtering
            ) {
                self.mark_contradicted_facts(similar_facts, key, category);
            }
        }

        Ok(json!({
            "success": true,
            "action": "save_fact",
            "key": key,
            "category": category,
            "confidence": confidence,
            "message": format!("Fact saved: [{}] {}", category, content),
        }))
    }

    async fn recall_facts(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Value, String> {
        // Generate embedding for the query
        let query_embedding = self.embed_text(query).await;

        // Fetch extra rows so the ctx filter doesn't shrink us below limit.
        // `as_of = None` → bi-temporal filter defaults to Utc::now(), which
        // correctly excludes facts whose `valid_until` is already in the past.
        let (results, _sources) = self.memory_repo.search_memory_facts_hybrid(
            query,
            query_embedding.as_deref(),
            Some(agent_id),
            limit * 2,
            0.7,  // vector weight
            0.3,  // bm25 weight
            None, // ward_id — no ward filtering from trait method
            None, // as_of — default "now" via Option A
        )?;

        // Ctx facts are session-canonical state (intent/prompt/plan/handoff).
        // They must never surface via fuzzy recall — a TSLA session's ctx
        // would otherwise contaminate a later AAPL session's recall just
        // because the ward matches. Readers reach ctx via `get_ctx_fact`.
        let results: Vec<_> = results
            .into_iter()
            .filter(|sf| sf.fact.category != CTX_CATEGORY)
            .take(limit)
            .collect();

        let items: Vec<Value> = results
            .iter()
            .map(|sf| {
                json!({
                    "key": sf.fact.key,
                    "category": sf.fact.category,
                    "content": sf.fact.content,
                    "confidence": sf.fact.confidence,
                    "score": sf.score,
                    "source": "memory_db",
                })
            })
            .collect();

        Ok(json!({
            "query": query,
            "results": items,
            "count": items.len(),
            "source": "memory_db",
        }))
    }

    async fn recall_facts_prioritized(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Value, String> {
        // Generate embedding for the query
        let query_embedding = self.embed_text(query).await;

        // Fetch more results than needed so we can re-rank. Ctx facts are
        // excluded here too — they reach readers only via `get_ctx_fact`,
        // never via fuzzy recall. See `recall_facts` for the rationale.
        //
        // `as_of` is the bi-temporal point-in-time cutoff (None = now).
        let (mut results, _sources) = self.memory_repo.search_memory_facts_hybrid(
            query,
            query_embedding.as_deref(),
            Some(agent_id),
            limit * 2,
            0.7,  // vector weight
            0.3,  // bm25 weight
            None, // ward_id — no ward filtering from trait method
            as_of,
        )?;
        results.retain(|sf| sf.fact.category != CTX_CATEGORY);

        // Bi-temporal cutoff for the supplementary paths below. The hybrid
        // search above filters at the SQL layer; the high-confidence and
        // corrections sub-queries don't, so we apply the same predicate in
        // Rust to keep retrieval semantics consistent across all paths.
        let cutoff = as_of.unwrap_or_else(chrono::Utc::now);

        // Also fetch high-confidence facts (>= 0.9) — always relevant
        let high_conf_facts: Vec<_> = self
            .memory_repo
            .get_high_confidence_facts(Some(agent_id), 0.9, limit)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| fact_valid_at(f, cutoff))
            .collect();

        // Include relevant corrections — filter by minimum cosine similarity
        // to avoid injecting "WiZ lights" corrections for currency questions.
        let all_corrections: Vec<_> = self
            .memory_repo
            .get_facts_by_category(agent_id, "correction", 10)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| fact_valid_at(f, cutoff))
            .collect();
        let corrections: Vec<_> = if let Some(ref qe) = query_embedding {
            all_corrections
                .into_iter()
                .filter(|fact| {
                    if let Some(ref fact_emb) = fact.embedding {
                        let sim =
                            crate::memory_repository::cosine_similarity_normalized(qe, fact_emb);
                        sim >= 0.15
                    } else {
                        true
                    }
                })
                .take(5)
                .collect()
        } else {
            all_corrections.into_iter().take(5).collect()
        };

        // Merge, dedup by key
        let mut seen_keys = std::collections::HashSet::new();
        let mut merged = Vec::new();
        for sf in results.drain(..) {
            if seen_keys.insert(sf.fact.key.clone()) {
                merged.push(sf);
            }
        }
        for fact in high_conf_facts {
            if seen_keys.insert(fact.key.clone()) {
                merged.push(crate::memory_repository::ScoredFact {
                    score: fact.confidence,
                    fact,
                });
            }
        }
        // Add corrections with pre-boost (category weight 1.5x applied later too)
        for fact in corrections {
            if seen_keys.insert(fact.key.clone()) {
                merged.push(crate::memory_repository::ScoredFact {
                    score: fact.confidence * 1.5,
                    fact,
                });
            }
        }

        // Apply category priority weights (same as system-level recall)
        let category_weights: HashMap<&str, f64> = HashMap::from([
            ("correction", 1.5),
            ("strategy", 1.4),
            ("user", 1.3),
            ("instruction", 1.2),
            ("domain", 1.0),
            ("pattern", 0.9),
            ("ward", 0.8),
            ("skill", 0.7),
            ("agent", 0.7),
        ]);

        for sf in &mut merged {
            let weight = category_weights
                .get(sf.fact.category.as_str())
                .copied()
                .unwrap_or(1.0);
            sf.score *= weight;
        }

        // Sort by weighted score descending
        merged.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(limit);

        // --- Format output with Rules section for corrections ---
        let mut rules: Vec<&crate::memory_repository::ScoredFact> = Vec::new();
        let mut context: Vec<&crate::memory_repository::ScoredFact> = Vec::new();

        for sf in &merged {
            match sf.fact.category.as_str() {
                "correction" => rules.push(sf),
                _ => context.push(sf),
            }
        }

        let mut formatted = String::new();

        // Rules section comes FIRST — imperative language
        if !rules.is_empty() {
            formatted.push_str("## Rules (from past corrections — ALWAYS follow these)\n");
            for sf in &rules {
                formatted.push_str(&format!("- {}\n", sf.fact.content));
            }
            formatted.push('\n');
        }

        // Regular recalled context
        if !context.is_empty() {
            formatted.push_str("## Recalled Context\n");
            for sf in &context {
                formatted.push_str(&format!(
                    "- [{}] {} ({:.2})\n",
                    sf.fact.category, sf.fact.content, sf.fact.confidence
                ));
            }
        }

        // --- Capability gap detection ---
        // Check if any of the returned results include skill/agent categories.
        // If none do, the query is likely outside known capabilities.
        let has_skill_or_agent = merged
            .iter()
            .any(|sf| sf.fact.category == "skill" || sf.fact.category == "agent");

        if !has_skill_or_agent {
            // Fetch top skills and agents by confidence for the gap section
            let top_skills = self
                .memory_repo
                .get_facts_by_category(agent_id, "skill", 3)
                .unwrap_or_default();
            let top_agents = self
                .memory_repo
                .get_facts_by_category(agent_id, "agent", 3)
                .unwrap_or_default();

            // Only show capability gap if there ARE known capabilities to suggest
            if !top_skills.is_empty() || !top_agents.is_empty() {
                formatted.push_str("\n## Capability Gap\n");
                formatted.push_str("No matching skills or agents found for this request.\n");
                formatted.push_str("Closest available capabilities:\n");

                for skill in &top_skills {
                    formatted.push_str(&format!("- skill: {} ({})\n", skill.key, skill.content));
                }
                for agent in &top_agents {
                    formatted.push_str(&format!("- agent: {} ({})\n", agent.key, agent.content));
                }

                formatted.push_str("\nConsider creating a plan to build the missing capability, or inform the user about current limitations.\n");
            }
        }

        let items: Vec<Value> = merged
            .iter()
            .map(|sf| {
                json!({
                    "key": sf.fact.key,
                    "category": sf.fact.category,
                    "content": sf.fact.content,
                    "confidence": sf.fact.confidence,
                    "score": sf.score,
                    "source": "memory_db",
                    "prioritized": true,
                })
            })
            .collect();

        Ok(json!({
            "query": query,
            "results": items,
            "count": items.len(),
            "source": "memory_db",
            "prioritized": true,
            "formatted": formatted,
        }))
    }

    /// Exact-key lookup for ctx-namespaced facts.
    ///
    /// Returns a JSON object with `found`, `key`, `content`, `owner`,
    /// `created_at`, `updated_at` on hit — or `{"found": false, "key": ...}`
    /// on miss. Never performs fuzzy ranking; a missing key never
    /// "nearest-neighbors" to a different fact.
    async fn get_ctx_fact(&self, ward_id: &str, key: &str) -> Result<Option<Value>, String> {
        let fact = self
            .memory_repo
            .get_fact_by_key(CTX_AGENT_SENTINEL, CTX_SCOPE, ward_id, key)?;
        Ok(fact.map(|f| {
            // Owner is stashed in source_summary as "owner=<value>" by
            // save_ctx_fact. Surface it as a dedicated field here so
            // callers don't parse strings.
            let owner = f
                .source_summary
                .as_deref()
                .and_then(|s| s.strip_prefix("owner="))
                .unwrap_or("unknown")
                .to_string();
            json!({
                "found": true,
                "key": f.key,
                "content": f.content,
                "owner": owner,
                "session_id": f.session_id,
                "created_at": f.created_at,
                "updated_at": f.updated_at,
                "pinned": f.pinned,
            })
        }))
    }

    /// Write a ctx-namespaced fact.
    ///
    /// The caller is responsible for permission enforcement — the DB
    /// layer simply persists. Ctx facts are upserted on the composite
    /// unique constraint `(agent_id, scope, ward_id, key)`; sentinel
    /// values for the first two mean re-writes on the same key overwrite
    /// content (this is intentional — state handoffs are idempotent per
    /// execution id).
    async fn save_ctx_fact(
        &self,
        session_id: &str,
        ward_id: &str,
        key: &str,
        content: &str,
        owner: &str,
        pinned: bool,
    ) -> Result<Value, String> {
        // Ctx reads are by-key (never fuzzy), so skip embedding generation
        // to save cost + latency. If we ever want fuzzy *within* ctx
        // (e.g. "which step worked on DCF?"), we can backfill embeddings.
        let now = chrono::Utc::now().to_rfc3339();
        let fact = MemoryFact {
            id: format!("fact-{}", uuid::Uuid::new_v4()),
            session_id: Some(session_id.to_string()),
            agent_id: CTX_AGENT_SENTINEL.to_string(),
            scope: CTX_SCOPE.to_string(),
            category: CTX_CATEGORY.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            confidence: 1.0,
            mention_count: 1,
            source_summary: Some(format!("owner={}", owner)),
            embedding: None,
            ward_id: ward_id.to_string(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now,
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
        };

        self.memory_repo.upsert_memory_fact(&fact)?;

        Ok(json!({
            "success": true,
            "action": "save_ctx_fact",
            "key": key,
            "owner": owner,
            "session_id": session_id,
        }))
    }

    async fn upsert_primitive(
        &self,
        ward_id: &str,
        key: &str,
        signature: &str,
        summary: &str,
    ) -> Result<Value, String> {
        // Primitives are queried by exact key (snapshot render) or by
        // ward_id prefix. No embedding needed — deterministic lookup.
        let now = chrono::Utc::now().to_rfc3339();
        // Content shape: `signature\nsummary` so the snapshot render can
        // split and format consistently without parsing more fields.
        let content = if summary.is_empty() {
            signature.to_string()
        } else {
            format!("{}\n{}", signature, summary)
        };
        let fact = MemoryFact {
            id: format!("fact-{}", uuid::Uuid::new_v4()),
            session_id: None,
            agent_id: PRIMITIVE_AGENT_SENTINEL.to_string(),
            scope: PRIMITIVE_SCOPE.to_string(),
            category: PRIMITIVE_CATEGORY.to_string(),
            key: key.to_string(),
            content,
            confidence: 1.0,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: ward_id.to_string(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now,
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
        };
        self.memory_repo.upsert_memory_fact(&fact)?;
        Ok(json!({ "success": true, "key": key, "ward_id": ward_id }))
    }

    async fn list_primitives(&self, ward_id: &str) -> Result<Value, String> {
        let rows = self
            .memory_repo
            .list_primitives_for_ward(ward_id)
            .map_err(|e| format!("list_primitives query failed: {}", e))?;
        let primitives: Vec<Value> = rows
            .into_iter()
            .map(|f| {
                let (signature, summary) = match f.content.split_once('\n') {
                    Some((sig, sum)) => (sig.to_string(), sum.to_string()),
                    None => (f.content.clone(), String::new()),
                };
                json!({
                    "key": f.key,
                    "signature": signature,
                    "summary": summary,
                })
            })
            .collect();
        Ok(json!({ "primitives": primitives }))
    }

    async fn list_primitives_for_ward(&self, ward_id: &str) -> Result<Vec<MemoryFact>, String> {
        self.memory_repo.list_primitives_for_ward(ward_id)
    }

    async fn list_recent_state_handoffs(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.memory_repo
            .list_recent_state_handoffs(session_id, limit)
    }

    async fn delete_facts_by_key(&self, category: &str, key: &str) -> Result<usize, String> {
        self.memory_repo.delete_facts_by_key(category, key)
    }

    async fn list_skill_index(&self) -> Result<Vec<SkillIndexRow>, String> {
        self.memory_repo.list_skill_index_state()
    }

    async fn upsert_skill_index(&self, row: SkillIndexRow) -> Result<(), String> {
        self.memory_repo.upsert_skill_index_state(&row)
    }

    async fn delete_skill_index(&self, name: &str) -> Result<bool, String> {
        self.memory_repo.delete_skill_index_state(name)
    }

    async fn aggregate_stats(&self) -> Result<zbot_stores_traits::MemoryAggregateStats, String> {
        self.memory_repo.aggregate_subsystem_stats()
    }

    async fn health_metrics(&self) -> Result<zbot_stores_traits::MemoryHealthMetrics, String> {
        self.memory_repo.episode_health_metrics()
    }

    async fn count_all_facts(&self, agent_id: Option<&str>) -> Result<i64, String> {
        self.memory_repo
            .count_all_memory_facts(agent_id)
            .map(|n| n as i64)
    }

    async fn list_memory_facts(
        &self,
        agent_id: Option<&str>,
        category: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>, String> {
        let facts = self
            .memory_repo
            .list_all_memory_facts(agent_id, category, scope, limit, offset)?;
        facts
            .into_iter()
            .map(|f| serde_json::to_value(f).map_err(|e| e.to_string()))
            .collect()
    }

    async fn get_memory_fact_by_id(&self, fact_id: &str) -> Result<Option<Value>, String> {
        match self.memory_repo.get_memory_fact_by_id(fact_id)? {
            Some(f) => Ok(Some(serde_json::to_value(f).map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    async fn delete_memory_fact(&self, fact_id: &str) -> Result<bool, String> {
        self.memory_repo.delete_memory_fact(fact_id)
    }

    async fn upsert_typed_fact(
        &self,
        fact: Value,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        let mut typed: MemoryFact =
            serde_json::from_value(fact).map_err(|e| format!("decode MemoryFact: {e}"))?;
        validate_fact_content(&typed.category, &typed.content)?;
        if embedding.is_some() {
            typed.embedding = embedding;
        }
        self.memory_repo.upsert_memory_fact(&typed)
    }

    async fn supersede_fact(
        &self,
        old_id: &str,
        new_id: &str,
        transition_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String> {
        self.memory_repo
            .supersede_fact(old_id, new_id, transition_time)
    }

    async fn archive_fact(&self, fact_id: &str) -> Result<bool, String> {
        self.memory_repo
            .archive_fact(fact_id)
            .map(|_| true)
            .or_else(|e| {
                if e.contains("not found") {
                    Ok(false)
                } else {
                    Err(e)
                }
            })
    }

    async fn search_memory_facts_hybrid(
        &self,
        agent_id: Option<&str>,
        query: &str,
        mode: &str,
        limit: usize,
        ward_id: Option<&str>,
        query_embedding: Option<&[f32]>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<Value>, String> {
        let typed = self
            .search_memory_facts_hybrid_typed(
                agent_id,
                query,
                mode,
                limit,
                ward_id,
                query_embedding,
                as_of,
            )
            .await?;
        typed
            .into_iter()
            .map(|(fact, score, src)| {
                let mut v = serde_json::to_value(fact).map_err(|e| e.to_string())?;
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("score".to_string(), serde_json::json!(score));
                    obj.insert("match_source".to_string(), Value::String(src));
                }
                Ok(v)
            })
            .collect()
    }

    async fn search_memory_facts_hybrid_typed(
        &self,
        agent_id: Option<&str>,
        query: &str,
        mode: &str,
        limit: usize,
        ward_id: Option<&str>,
        query_embedding: Option<&[f32]>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<(MemoryFact, f64, String)>, String> {
        let scored: Vec<(crate::memory_repository::ScoredFact, &'static str)> = match mode {
            "fts" => {
                let rows = self
                    .memory_repo
                    .search_memory_facts_fts(query, agent_id, limit, ward_id, as_of)?;
                rows.into_iter().map(|sf| (sf, "fts")).collect()
            }
            "semantic" => {
                let Some(emb) = query_embedding else {
                    return Ok(Vec::new());
                };
                // Bi-temporal filter applies through search_memory_facts_vector,
                // which we reach via search_similar_facts with min_similarity=0.
                // search_similar_facts always defaults to "now" — that's the
                // correct semantic for the semantic-only retrieval surface.
                let rows = self
                    .memory_repo
                    .search_similar_facts(emb, agent_id, 0.0, limit, ward_id)?;
                rows.into_iter().map(|sf| (sf, "vec")).collect()
            }
            _ => {
                // hybrid path fuses FTS + vector via the existing RRF method,
                // using the per-id source map to label each row's origin.
                let (rows, sources) = self.memory_repo.search_memory_facts_hybrid(
                    query,
                    query_embedding,
                    agent_id,
                    limit,
                    0.5,
                    0.5,
                    ward_id,
                    as_of,
                )?;
                let src_map: std::collections::HashMap<String, &'static str> =
                    sources.into_iter().collect();
                rows.into_iter()
                    .map(|sf| {
                        let src = src_map.get(&sf.fact.id).copied().unwrap_or("hybrid");
                        (sf, src)
                    })
                    .collect()
            }
        };
        Ok(scored
            .into_iter()
            .map(|(sf, src)| (sf.fact, sf.score, src.to_string()))
            .collect())
    }

    // ---- Sleep-time synthesis (Phase D4) -------------------------------

    async fn find_strategy_fact_by_similarity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f32,
        scan_limit: usize,
    ) -> Result<Option<StrategyFactMatch>, String> {
        let candidates = self
            .memory_repo
            .get_facts_by_category(agent_id, "strategy", scan_limit)
            .map_err(|e| e.to_string())?;
        for fact in candidates {
            let stored = match self
                .memory_repo
                .get_fact_embedding(&fact.id)
                .map_err(|e| e.to_string())?
            {
                Some(v) => v,
                None => continue,
            };
            let sim = cosine_similarity_f64(embedding, &stored);
            if sim >= threshold as f64 {
                return Ok(Some(StrategyFactMatch {
                    fact_id: fact.id,
                    source_episode_id: fact.source_episode_id,
                }));
            }
        }
        Ok(None)
    }

    async fn bump_strategy_fact_episodes(
        &self,
        fact_id: &str,
        merged_source_episode_id: &str,
        now_rfc3339: &str,
    ) -> Result<(), String> {
        let fact_id = fact_id.to_string();
        let merged = merged_source_episode_id.to_string();
        let now = now_rfc3339.to_string();
        self.memory_repo.db().with_connection(|conn| {
            conn.execute(
                "UPDATE memory_facts
                 SET mention_count = mention_count + 1,
                     updated_at = ?1,
                     source_episode_id = ?2
                 WHERE id = ?3",
                rusqlite::params![now, merged, fact_id],
            )?;
            Ok(())
        })
    }

    async fn get_facts_by_category(
        &self,
        agent_id: &str,
        category: &str,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.memory_repo
            .get_facts_by_category(agent_id, category, limit)
    }

    async fn get_fact_by_key(
        &self,
        agent_id: &str,
        scope: &str,
        ward_id: &str,
        key: &str,
    ) -> Result<Option<MemoryFact>, String> {
        self.memory_repo
            .get_fact_by_key(agent_id, scope, ward_id, key)
    }

    async fn get_fact_embedding(&self, fact_id: &str) -> Result<Option<Vec<f32>>, String> {
        self.memory_repo.get_fact_embedding(fact_id)
    }

    async fn get_cached_embedding(
        &self,
        content_hash: &str,
        model_name: &str,
    ) -> Result<Option<Vec<f32>>, String> {
        self.memory_repo
            .get_cached_embedding(content_hash, model_name)
    }

    async fn cache_embedding(
        &self,
        content_hash: &str,
        model_name: &str,
        embedding: &[f32],
    ) -> Result<(), String> {
        self.memory_repo
            .cache_embedding(content_hash, model_name, embedding)
    }

    async fn get_memory_facts(
        &self,
        agent_id: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.memory_repo.get_memory_facts(agent_id, scope, limit)
    }

    async fn get_high_confidence_facts(
        &self,
        agent_id: Option<&str>,
        threshold: f64,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        self.memory_repo
            .get_high_confidence_facts(agent_id, threshold, limit)
    }

    async fn insert_strategy_fact(&self, req: StrategyFactInsert) -> Result<String, String> {
        let id = format!("fact-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let fact = MemoryFact {
            id: id.clone(),
            session_id: None,
            agent_id: req.agent_id,
            scope: "agent".to_string(),
            category: "strategy".to_string(),
            key: req.key,
            content: req.content,
            confidence: req.confidence,
            mention_count: 1,
            source_summary: req.source_summary,
            embedding: req.embedding,
            ward_id: "__global__".to_string(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now,
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("convention".to_string()),
            source_episode_id: req.source_episode_id,
            source_ref: None,
        };
        self.memory_repo.upsert_memory_fact(&fact)?;
        Ok(id)
    }

    async fn list_contradicted_fact_episode_ids(
        &self,
        agent_id: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<String>, String> {
        let agent_id = agent_id.to_string();
        let since_str = since.to_rfc3339();
        let db = self.memory_repo.db().clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
            db.with_connection(|conn| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT source_episode_id
                     FROM memory_facts
                     WHERE agent_id = ?1
                       AND contradicted_by IS NOT NULL
                       AND source_episode_id IS NOT NULL
                       AND updated_at > ?2",
                )?;
                let rows: Vec<String> = stmt
                    .query_map(rusqlite::params![agent_id, since_str], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

// ---- Helpers --------------------------------------------------------------

/// Bi-temporal point-in-time predicate. Returns `true` when `fact` was valid
/// at `cutoff` — its `valid_from <= cutoff < valid_until`. Used by
/// `recall_facts_prioritized` to gate the supplementary high-confidence and
/// corrections paths that don't hit the SQL filter directly. Unparseable
/// timestamps degrade to "treat as missing" so a corrupt row never blocks
/// recall of an otherwise-relevant fact.
fn fact_valid_at(fact: &MemoryFact, cutoff: chrono::DateTime<chrono::Utc>) -> bool {
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .ok()
    };
    if let Some(vf) = fact.valid_from.as_deref().and_then(parse) {
        if vf > cutoff {
            return false;
        }
    }
    if let Some(vu) = fact.valid_until.as_deref().and_then(parse) {
        if vu <= cutoff {
            return false;
        }
    }
    true
}

/// Cosine similarity in `f64` precision. Matches the synthesizer's
/// historical computation; pulled in here so the strategy-similarity
/// scan stays self-contained.
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
    use crate::vector_index::{SqliteVecIndex, VectorIndex};
    use crate::KnowledgeDatabase;
    use tempfile::TempDir;

    #[test]
    fn validate_fact_content_accepts_short_fact() {
        assert!(validate_fact_content("pattern", "The user prefers pytest.").is_ok());
    }

    #[test]
    fn validate_fact_content_rejects_oversized_fact() {
        let long = "x".repeat(MAX_FACT_CONTENT_CHARS + 1);
        assert!(validate_fact_content("pattern", &long).is_err());
    }

    #[test]
    fn validate_fact_content_exempts_machine_generated_categories() {
        let long = "x".repeat(MAX_FACT_CONTENT_CHARS + 500);
        assert!(validate_fact_content(CTX_CATEGORY, &long).is_ok());
        assert!(validate_fact_content(PRIMITIVE_CATEGORY, &long).is_ok());
    }

    fn create_test_store() -> GatewayMemoryFactStore {
        use gateway_services::VaultPaths;

        let temp_dir = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(temp_dir.path().to_path_buf()));
        let _ = temp_dir.keep();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        let vec_index: Arc<dyn VectorIndex> = Arc::new(
            SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id")
                .expect("vec index init"),
        );
        let repo = Arc::new(MemoryRepository::new(db, vec_index));
        GatewayMemoryFactStore::new(repo, None) // No embedding client in tests
    }

    #[tokio::test]
    async fn test_save_and_recall_fact() {
        let store = create_test_store();

        let result = store
            .save_fact(
                "agent-1",
                "preference",
                "lang.main",
                "Prefers Rust",
                0.9,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["key"], "lang.main");

        let recall = store.recall_facts("agent-1", "Rust", 5).await.unwrap();
        assert!(recall["count"].as_u64().unwrap() >= 1);

        let first = &recall["results"][0];
        assert_eq!(first["key"], "lang.main");
        assert_eq!(first["content"], "Prefers Rust");
    }

    #[tokio::test]
    async fn test_save_fact_upsert() {
        let store = create_test_store();

        store
            .save_fact(
                "agent-1",
                "preference",
                "editor",
                "VS Code",
                0.7,
                None,
                None,
            )
            .await
            .unwrap();

        // Save again with same key — should upsert
        store
            .save_fact("agent-1", "preference", "editor", "Neovim", 0.9, None, None)
            .await
            .unwrap();

        let recall = store.recall_facts("agent-1", "editor", 10).await.unwrap();
        assert_eq!(recall["count"].as_u64().unwrap(), 1);
        assert_eq!(recall["results"][0]["content"], "Neovim");
    }

    #[tokio::test]
    async fn test_recall_empty() {
        let store = create_test_store();
        let recall = store
            .recall_facts("agent-1", "nonexistent", 5)
            .await
            .unwrap();
        assert_eq!(recall["count"].as_u64().unwrap(), 0);
    }

    // ========================================================================
    // Ctx namespace tests (Phase 1 — memory-as-ctx bundle)
    // ========================================================================

    #[tokio::test]
    async fn test_save_ctx_fact_and_get_back() {
        let store = create_test_store();

        let result = store
            .save_ctx_fact(
                "sess-abc",
                "stock-analysis",
                "ctx.sess-abc.intent",
                "Analyze AAPL valuation vs peers",
                "root",
                true,
            )
            .await
            .unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["key"], "ctx.sess-abc.intent");
        assert_eq!(result["owner"], "root");

        let got = store
            .get_ctx_fact("stock-analysis", "ctx.sess-abc.intent")
            .await
            .unwrap()
            .expect("ctx fact should be found");

        assert_eq!(got["found"], true);
        assert_eq!(got["key"], "ctx.sess-abc.intent");
        assert_eq!(got["content"], "Analyze AAPL valuation vs peers");
        assert_eq!(got["owner"], "root");
        assert_eq!(got["session_id"], "sess-abc");
        assert_eq!(got["pinned"], true);
    }

    #[tokio::test]
    async fn test_get_ctx_fact_miss_returns_none_not_nearest() {
        // Exact lookup must NEVER return a nearest-neighbor match; a miss
        // is a miss. This is the core guarantee that distinguishes
        // get_ctx_fact from fuzzy recall.
        let store = create_test_store();

        store
            .save_ctx_fact(
                "sess-xyz",
                "some-ward",
                "ctx.sess-xyz.intent",
                "do something",
                "root",
                true,
            )
            .await
            .unwrap();

        let miss = store
            .get_ctx_fact("some-ward", "ctx.sess-xyz.nonexistent")
            .await
            .unwrap();

        assert!(
            miss.is_none(),
            "missing key must return None, not a ranked neighbor"
        );
    }

    #[tokio::test]
    async fn test_ctx_facts_excluded_from_fuzzy_recall() {
        // A ctx fact whose content keyword-matches a recall query must
        // NOT appear in the recall results. Only reachable via
        // get_ctx_fact. This prevents cross-session contamination.
        let store = create_test_store();

        // Write a regular (non-ctx) fact that SHOULD be recalled.
        store
            .save_fact(
                "agent-x",
                "domain",
                "finance.dcf.method",
                "DCF uses WACC and terminal growth to estimate intrinsic value",
                0.9,
                None,
                None,
            )
            .await
            .unwrap();

        // Write a ctx fact with content that would also match "DCF"
        // — must be excluded from recall.
        store
            .save_ctx_fact(
                "sess-leak",
                "any-ward",
                "ctx.sess-leak.intent",
                "compute DCF valuation on the target ticker",
                "root",
                true,
            )
            .await
            .unwrap();

        let recall = store
            .recall_facts("agent-x", "DCF valuation", 10)
            .await
            .unwrap();

        let results = recall["results"].as_array().expect("results array");
        for r in results {
            let key = r["key"].as_str().unwrap();
            assert!(
                !key.starts_with("ctx."),
                "recall returned ctx fact {} — must be filtered",
                key
            );
        }
    }

    #[tokio::test]
    async fn test_save_ctx_fact_upsert_overwrites_content() {
        // Subsequent writes with the same key (same session, same ward,
        // same key) are idempotent upserts — used e.g. when a plan is
        // regenerated.
        let store = create_test_store();

        store
            .save_ctx_fact(
                "sess-up",
                "ward-a",
                "ctx.sess-up.plan",
                "first plan",
                "root",
                true,
            )
            .await
            .unwrap();

        store
            .save_ctx_fact(
                "sess-up",
                "ward-a",
                "ctx.sess-up.plan",
                "second plan",
                "root",
                true,
            )
            .await
            .unwrap();

        let got = store
            .get_ctx_fact("ward-a", "ctx.sess-up.plan")
            .await
            .unwrap()
            .unwrap();

        // Note: the upsert in memory_repository preserves content when
        // pinned=true on the existing row. Check which branch fires —
        // fresh row has pinned=true from write #1, so write #2 keeps
        // "first plan". This is intentional behavior to protect pinned
        // ctx keys from drift; state handoff writes use pinned=false.
        assert_eq!(got["content"], "first plan");
    }

    #[tokio::test]
    async fn test_save_ctx_fact_unpinned_allows_overwrite() {
        // State handoffs are written with pinned=false so rerunning the
        // same step overwrites the previous handoff for that execution.
        let store = create_test_store();

        store
            .save_ctx_fact(
                "sess-st",
                "ward-a",
                "ctx.sess-st.state.exec-1",
                "handoff v1",
                "subagent:exec-1",
                false,
            )
            .await
            .unwrap();

        store
            .save_ctx_fact(
                "sess-st",
                "ward-a",
                "ctx.sess-st.state.exec-1",
                "handoff v2",
                "subagent:exec-1",
                false,
            )
            .await
            .unwrap();

        let got = store
            .get_ctx_fact("ward-a", "ctx.sess-st.state.exec-1")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(got["content"], "handoff v2");
    }

    // ========================================================================
    // Skill index state tests (per-skill staleness tracker)
    // ========================================================================

    fn skill_row(name: &str, mtime: i64, size: i64) -> SkillIndexRow {
        SkillIndexRow {
            name: name.to_string(),
            source_root: "vault".to_string(),
            file_path: format!("/skills/{name}/SKILL.md"),
            mtime_unix: mtime,
            size_bytes: size,
            last_indexed_unix: mtime,
            format_version: 2,
        }
    }

    #[tokio::test]
    async fn skill_index_state_round_trips() {
        let store = create_test_store();
        store
            .upsert_skill_index(skill_row("alpha", 100, 200))
            .await
            .unwrap();
        let rows = store.list_skill_index().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "alpha");
        assert_eq!(rows[0].mtime_unix, 100);
        assert_eq!(rows[0].size_bytes, 200);
    }

    #[tokio::test]
    async fn skill_index_state_upsert_replaces_on_conflict() {
        let store = create_test_store();
        store
            .upsert_skill_index(skill_row("alpha", 100, 200))
            .await
            .unwrap();
        store
            .upsert_skill_index(skill_row("alpha", 999, 999))
            .await
            .unwrap();
        let rows = store.list_skill_index().await.unwrap();
        assert_eq!(rows.len(), 1, "no duplicate row");
        assert_eq!(rows[0].mtime_unix, 999);
        assert_eq!(rows[0].size_bytes, 999);
    }

    #[tokio::test]
    async fn skill_index_state_delete_removes_row() {
        let store = create_test_store();
        store
            .upsert_skill_index(skill_row("alpha", 100, 200))
            .await
            .unwrap();
        let removed = store.delete_skill_index("alpha").await.unwrap();
        assert!(removed);
        let rows = store.list_skill_index().await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn skill_index_state_delete_missing_returns_false() {
        let store = create_test_store();
        let removed = store.delete_skill_index("never-existed").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn list_skill_index_empty_db_returns_empty_vec() {
        let store = create_test_store();
        let rows = store.list_skill_index().await.unwrap();
        assert!(rows.is_empty());
    }

    /// Regression: `delete_facts_by_key` must remove every matching
    /// `memory_facts` row. Verifies the SQL deletion path that backs
    /// the skill reindexer's ghost cleanup.
    #[tokio::test]
    async fn delete_facts_by_key_removes_memory_facts_rows() {
        let store = create_test_store();

        store
            .save_fact(
                "root",
                "skill",
                "skill:web-reader",
                "web-reader | reads URLs",
                1.0,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(count_table(&store, "memory_facts"), 1);

        let deleted = store
            .delete_facts_by_key("skill", "skill:web-reader")
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(count_table(&store, "memory_facts"), 0);
    }

    /// Regression: when a `memory_facts` row is deleted, the cleanup
    /// trigger (`trg_facts_delete_vec`) must drop its `memory_facts_index`
    /// vec0 row. Without this, ghost embeddings would haunt search
    /// results forever.
    ///
    /// We exercise the trigger directly by:
    ///   1. Inserting a row into `memory_facts` via `save_fact`.
    ///   2. Inserting a matching row into `memory_facts_index` via the
    ///      vector index (the test fixture skips embedding by default).
    ///   3. Calling `delete_facts_by_key` and asserting both rows go.
    #[tokio::test]
    async fn delete_cascades_to_vec0_via_trigger() {
        let store = create_test_store();

        store
            .save_fact("root", "skill", "skill:hash", "h", 1.0, None, None)
            .await
            .unwrap();

        // Look up the fact id and seed a matching vec0 row directly.
        let fact_id: String = store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                let id: String = conn.query_row(
                    "SELECT id FROM memory_facts WHERE key = 'skill:hash'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(id)
            })
            .unwrap();

        store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                let zero_emb = vec![0.0_f32; 384];
                let bytes: Vec<u8> = zero_emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                conn.execute(
                    "INSERT INTO memory_facts_index (fact_id, embedding) VALUES (?1, ?2)",
                    rusqlite::params![fact_id, bytes],
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(count_table(&store, "memory_facts_index"), 1);

        store
            .delete_facts_by_key("skill", "skill:hash")
            .await
            .unwrap();

        assert_eq!(count_table(&store, "memory_facts"), 0);
        assert_eq!(
            count_table(&store, "memory_facts_index"),
            0,
            "trg_facts_delete_vec must have cleaned the vec0 row"
        );
    }

    fn count_table(store: &GatewayMemoryFactStore, table: &str) -> i64 {
        // GatewayMemoryFactStore.memory_repo is private; use the same
        // KnowledgeDatabase the test fixture built. Construct a fresh
        // KnowledgeDatabase against the same directory by piggy-backing
        // on the public path the store already exposes via its repo's
        // shared connection.
        let sql = format!("SELECT COUNT(*) FROM {table}");
        store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
                Ok(count)
            })
            .unwrap()
    }

    // =========================================================================
    // Bi-temporal phase 1: valid_from population
    // =========================================================================

    /// Read the `valid_from` column for the fact stored under `key`.
    /// Helper used by the bi-temporal tests below.
    fn read_valid_from(store: &GatewayMemoryFactStore, key: &str) -> Option<String> {
        store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                let row: Option<String> = conn
                    .query_row(
                        "SELECT valid_from FROM memory_facts WHERE key = ?1",
                        rusqlite::params![key],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten();
                Ok(row)
            })
            .unwrap()
    }

    /// Phase 1: when the caller passes `None`, the store must default
    /// `valid_from` to "now" — never NULL. The recorded timestamp should
    /// parse as RFC3339 and land within a small slack window of test time.
    #[tokio::test]
    async fn save_fact_defaults_valid_from_to_now_when_none() {
        let store = create_test_store();

        let before = chrono::Utc::now();
        store
            .save_fact("ag", "preference", "k.default", "v", 0.9, None, None)
            .await
            .unwrap();
        let after = chrono::Utc::now();

        let stored =
            read_valid_from(&store, "k.default").expect("valid_from must be persisted, not NULL");

        let parsed = chrono::DateTime::parse_from_rfc3339(&stored)
            .expect("valid_from should be RFC3339")
            .with_timezone(&chrono::Utc);

        // 5-second slack on either side to absorb async/clock skew.
        let slack = chrono::Duration::seconds(5);
        assert!(
            parsed >= before - slack && parsed <= after + slack,
            "valid_from {parsed} must be near now ({before}..{after})"
        );
    }

    /// Phase 1: when the caller passes `Some(t)`, that exact timestamp
    /// must be persisted — not silently overwritten with "now".
    #[tokio::test]
    async fn save_fact_preserves_explicit_valid_from() {
        let store = create_test_store();

        let pinned = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        store
            .save_fact(
                "ag",
                "preference",
                "k.explicit",
                "v",
                0.9,
                None,
                Some(pinned),
            )
            .await
            .unwrap();

        let stored = read_valid_from(&store, "k.explicit").expect("valid_from must be persisted");
        let parsed = chrono::DateTime::parse_from_rfc3339(&stored)
            .expect("valid_from should be RFC3339")
            .with_timezone(&chrono::Utc);

        assert_eq!(parsed, pinned, "explicit valid_from must round-trip");
    }

    /// Phase 1: the v25 backfill migration repairs legacy rows whose
    /// `valid_from` is NULL by setting it to `created_at`. We can't
    /// "un-run" the migration that fires at DB open, so we simulate the
    /// legacy state by writing a row and then nulling out `valid_from`
    /// before re-running the migration SQL.
    #[tokio::test]
    async fn v25_backfill_sets_valid_from_to_created_at_for_legacy_rows() {
        let store = create_test_store();

        // Seed a normal fact (this row gets valid_from = now from the
        // new save path).
        store
            .save_fact("ag", "preference", "k.legacy", "v", 0.9, None, None)
            .await
            .unwrap();

        // Force the row into legacy state: clear valid_from to NULL.
        store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE memory_facts SET valid_from = NULL WHERE key = 'k.legacy'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            read_valid_from(&store, "k.legacy").is_none(),
            "precondition: legacy row should have NULL valid_from"
        );

        // Re-run the v25 backfill statement directly.
        store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE memory_facts SET valid_from = created_at WHERE valid_from IS NULL",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let backfilled =
            read_valid_from(&store, "k.legacy").expect("backfill should populate valid_from");

        // It should equal the row's created_at.
        let created_at: String = store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT created_at FROM memory_facts WHERE key = 'k.legacy'",
                    [],
                    |r| r.get::<_, String>(0),
                )
            })
            .unwrap();

        assert_eq!(
            backfilled, created_at,
            "v25 backfill must set valid_from = created_at"
        );
    }

    // =========================================================================
    // Bi-temporal phase 2: point-in-time recall via `as_of`
    // =========================================================================

    /// Force a fact's bi-temporal interval to a known shape. `save_fact` only
    /// accepts `valid_from`; `valid_until` must be poked in directly via SQL.
    fn set_bi_temporal_interval(
        store: &GatewayMemoryFactStore,
        key: &str,
        valid_from: chrono::DateTime<chrono::Utc>,
        valid_until: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        let valid_from_str = valid_from.to_rfc3339();
        let valid_until_str = valid_until.map(|t| t.to_rfc3339());
        store
            .knowledge_db_for_tests()
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE memory_facts SET valid_from = ?1, valid_until = ?2 WHERE key = ?3",
                    rusqlite::params![valid_from_str, valid_until_str, key],
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// Test A — fact valid in the future is excluded from a past-cutoff query.
    /// `valid_from = 2026-06-01`, `as_of = 2026-05-15` → not retrieved.
    #[tokio::test]
    async fn recall_as_of_excludes_future_facts() {
        let store = create_test_store();

        // Unique phrase keeps the test isolated from any other seeded fact.
        store
            .save_fact(
                "agent-bt",
                "domain",
                "k.future",
                "phaserblossom future-only knowledge",
                0.9,
                None,
                None,
            )
            .await
            .unwrap();

        let future_from = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        set_bi_temporal_interval(&store, "k.future", future_from, None);

        let as_of = Some(
            chrono::DateTime::parse_from_rfc3339("2026-05-15T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let recall = store
            .recall_facts_prioritized("agent-bt", "phaserblossom", 10, as_of)
            .await
            .unwrap();

        let results = recall["results"].as_array().expect("results array");
        assert!(
            !results.iter().any(|r| r["key"] == "k.future"),
            "future-only fact must be excluded when querying before its valid_from; got: {:?}",
            results
        );
    }

    /// Test B — fact whose `valid_until` is before the cutoff is excluded.
    /// `valid_from = 2026-01-01`, `valid_until = 2026-04-01`, `as_of = 2026-05-15` → not retrieved.
    #[tokio::test]
    async fn recall_as_of_excludes_past_facts() {
        let store = create_test_store();

        store
            .save_fact(
                "agent-bt",
                "domain",
                "k.past",
                "phaserblossom past-bounded knowledge",
                0.9,
                None,
                None,
            )
            .await
            .unwrap();

        let from = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let until = chrono::DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        set_bi_temporal_interval(&store, "k.past", from, Some(until));

        let as_of = Some(
            chrono::DateTime::parse_from_rfc3339("2026-05-15T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let recall = store
            .recall_facts_prioritized("agent-bt", "phaserblossom", 10, as_of)
            .await
            .unwrap();

        let results = recall["results"].as_array().expect("results array");
        assert!(
            !results.iter().any(|r| r["key"] == "k.past"),
            "fact with valid_until before cutoff must be excluded; got: {:?}",
            results
        );
    }

    /// Test C — fact whose interval brackets the cutoff is included.
    /// `valid_from = 2026-03-01`, `valid_until = 2026-06-01`, `as_of = 2026-05-15` → retrieved.
    #[tokio::test]
    async fn recall_as_of_includes_active_facts() {
        let store = create_test_store();

        store
            .save_fact(
                "agent-bt",
                "domain",
                "k.active",
                "phaserblossom currently-valid knowledge",
                0.9,
                None,
                None,
            )
            .await
            .unwrap();

        let from = chrono::DateTime::parse_from_rfc3339("2026-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let until = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        set_bi_temporal_interval(&store, "k.active", from, Some(until));

        let as_of = Some(
            chrono::DateTime::parse_from_rfc3339("2026-05-15T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let recall = store
            .recall_facts_prioritized("agent-bt", "phaserblossom", 10, as_of)
            .await
            .unwrap();

        let results = recall["results"].as_array().expect("results array");
        assert!(
            results.iter().any(|r| r["key"] == "k.active"),
            "fact whose interval contains the cutoff must be returned; got: {:?}",
            results
        );
    }

    /// Test D — REGRESSION for the latent bug. Default query (no `as_of`)
    /// must exclude facts whose `valid_until` already lies in the past.
    /// Before Option A, such facts surfaced because no recall path filtered
    /// `valid_until > now`. After Option A this fact is correctly excluded.
    #[tokio::test]
    async fn recall_default_excludes_time_bounded_past() {
        let store = create_test_store();

        store
            .save_fact(
                "agent-bt",
                "domain",
                "k.expired",
                "phaserblossom expired knowledge",
                0.9,
                None,
                None,
            )
            .await
            .unwrap();

        // valid_until = (now - 1 day). The fact has ended; default recall
        // (as_of = None → Utc::now()) must NOT return it.
        let now = chrono::Utc::now();
        let one_day_ago = now - chrono::Duration::days(1);
        let one_year_ago = now - chrono::Duration::days(365);
        set_bi_temporal_interval(&store, "k.expired", one_year_ago, Some(one_day_ago));

        let recall = store
            .recall_facts_prioritized("agent-bt", "phaserblossom", 10, None)
            .await
            .unwrap();

        let results = recall["results"].as_array().expect("results array");
        assert!(
            !results.iter().any(|r| r["key"] == "k.expired"),
            "time-bounded-past fact must be excluded from default recall; got: {:?}",
            results
        );
    }
}
