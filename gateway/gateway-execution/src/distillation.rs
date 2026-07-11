// ============================================================================
// SESSION DISTILLATION
// Automatically extract durable facts from completed sessions
// ============================================================================

//! The `SessionDistiller` analyzes completed session transcripts and extracts
//! structured facts worth remembering for future sessions. This is AgentZero's
//! key advantage over other memory frameworks — we have full tool call history
//! (Session Tree) and can automatically distill it without the agent needing
//! to explicitly save.
//!
//! ## Flow
//!
//! 1. Load last N messages from a completed session
//! 2. Build a distillation prompt
//! 3. Call the LLM to extract structured facts as JSON
//! 4. Upsert each fact into `memory_facts` with embedding
//! 5. Cache the embedding for hash-based dedup

use std::sync::Arc;

use agent_runtime::llm::client::LlmClient;
use agent_runtime::llm::config::LlmConfig;
use agent_runtime::llm::embedding::EmbeddingClient;
use agent_runtime::llm::openai::OpenAiClient;
use agent_runtime::types::ChatMessage;
use gateway_services::{
    models::DEFAULT_MAX_OUTPUT_TOKENS, ProviderService, SettingsService, VaultPaths,
};
use knowledge_graph::{Entity, EntityType, Relationship, RelationshipType};
use serde::{Deserialize, Serialize};
use zbot_stores_domain::{MemoryFact, Procedure, SessionEpisode};

/// Distills completed sessions into structured memory facts.
///
/// Persistence routing: every store dependency is an `Arc<dyn ...>`
/// trait companion. AppState wires the SQLite-backed implementations
/// at construction time; consumers here only see the abstract surface.
pub struct SessionDistiller {
    provider_service: Arc<ProviderService>,
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
    session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    /// Trait-routed distillation store. Run-tracking writes flow
    /// through this handle.
    distillation_store: Option<Arc<dyn zbot_stores_traits::DistillationStore>>,
    /// Trait-routed episode store for episode storage, strategy
    /// emergence, and failure clustering.
    episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>>,
    paths: Arc<VaultPaths>,
    settings_service: Option<Arc<SettingsService>>,
    /// Trait-routed wiki store for ward-wiki compilation.
    pub wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>>,
    /// Trait-routed procedure store for procedure upsert.
    pub procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
}

/// A single fact extracted by the distillation LLM call.
#[derive(Debug, Clone, Deserialize)]
struct ExtractedFact {
    category: String,
    key: String,
    content: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
    /// Optional epistemic classification (archival|current|convention|procedural).
    /// Defaults to "current" when omitted by the LLM.
    #[serde(default)]
    epistemic_class: Option<String>,
}

/// An entity extracted by the distillation LLM call.
#[derive(Debug, Clone, Deserialize)]
struct ExtractedEntity {
    name: String,
    #[serde(rename = "type")]
    entity_type: String,
    #[serde(default)]
    properties: std::collections::HashMap<String, serde_json::Value>,
}

/// A relationship extracted by the distillation LLM call.
#[derive(Debug, Clone, Deserialize)]
struct ExtractedRelationship {
    source: String,
    target: String,
    #[serde(rename = "type")]
    relationship_type: String,
}

/// An episode assessment extracted by the distillation LLM call.
#[derive(Debug, Clone, Deserialize)]
struct ExtractedEpisode {
    task_summary: String,
    /// One of: 'success', 'partial', 'failed'
    outcome: String,
    strategy_used: Option<String>,
    key_learnings: Option<String>,
}

/// A procedure extracted by the distillation LLM call.
#[derive(Debug, Clone, Deserialize)]
struct ExtractedProcedure {
    name: String,
    description: String,
    steps: Vec<ProcedureStep>,
    #[serde(default)]
    parameters: Option<Vec<String>>,
    #[serde(default)]
    trigger_pattern: Option<String>,
}

/// A single step within an extracted procedure.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct ProcedureStep {
    action: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    task_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    note: Option<String>,
}

/// Full distillation response including facts, entities, relationships, episode, and procedure.
#[derive(Debug, Clone, Deserialize)]
struct DistillationResponse {
    #[serde(default)]
    facts: Vec<ExtractedFact>,
    #[serde(default)]
    entities: Vec<ExtractedEntity>,
    #[serde(default)]
    relationships: Vec<ExtractedRelationship>,
    #[serde(default)]
    episode: Option<ExtractedEpisode>,
    #[serde(default)]
    procedure: Option<ExtractedProcedure>,
}

fn default_confidence() -> f64 {
    0.8
}

/// Verify a distilled fact against the session transcript's tool outputs.
/// Grounded facts keep confidence; ungrounded get reduced; contradicted get discarded.
fn verify_fact_confidence(
    fact_content: &str,
    fact_confidence: f64,
    tool_outputs: &[String],
) -> f64 {
    // Extract key terms from the fact (words > 3 chars, skip stopwords)
    let stopwords = [
        "that", "this", "with", "from", "have", "been", "were", "will", "should", "would", "could",
        "their", "there", "about", "which", "when", "into", "also", "than", "then", "them", "very",
        "just",
    ];
    let key_terms: Vec<&str> = fact_content
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() > 3)
        .filter(|w| !stopwords.contains(&w.to_lowercase().as_str()))
        .collect();

    if key_terms.is_empty() {
        return fact_confidence * 0.6;
    }

    // Check how many key terms appear in tool outputs
    let mut matches = 0;
    for term in &key_terms {
        let term_lower = term.to_lowercase();
        for output in tool_outputs {
            if output.to_lowercase().contains(&term_lower) {
                matches += 1;
                break;
            }
        }
    }

    let match_ratio = matches as f64 / key_terms.len() as f64;

    if match_ratio >= 0.5 {
        // Well-grounded in tool outputs
        fact_confidence
    } else if match_ratio > 0.0 {
        // Partially grounded
        fact_confidence * 0.8
    } else {
        // Not grounded — reduce confidence significantly
        fact_confidence * 0.5
    }
}

/// Minimum number of messages in a session to trigger distillation.
/// Set low to capture learnings from even short sessions.
const MIN_MESSAGES_FOR_DISTILLATION: usize = 4;

/// Maximum messages to load for distillation (to stay within LLM context).
const MAX_MESSAGES_FOR_DISTILLATION: usize = 100;

impl SessionDistiller {
    /// Create a new session distiller with lazy LLM client resolution.
    ///
    /// The distiller resolves the default provider and creates a lightweight
    /// LLM client on-demand in `distill()`, avoiding the need for a concrete
    /// LLM client at construction time.
    pub fn new(
        provider_service: Arc<ProviderService>,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
        session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
        paths: Arc<VaultPaths>,
        settings_service: Option<Arc<SettingsService>>,
    ) -> Self {
        Self {
            provider_service,
            embedding_client,
            messages,
            session_meta,
            memory_store: None,
            kg_store: None,
            distillation_store: None,
            episode_store: None,
            paths,
            settings_service,
            wiki_store: None,
            procedure_store: None,
        }
    }

    /// Wire the trait-routed memory store. Upsert + supersede route
    /// through this handle.
    pub fn set_memory_store(&mut self, store: Arc<dyn zbot_stores::MemoryFactStore>) {
        self.memory_store = Some(store);
    }

    /// Wire the trait-routed knowledge-graph store. Entity / relationship
    /// writes route through this handle.
    pub fn set_kg_store(&mut self, store: Arc<dyn zbot_stores::KnowledgeGraphStore>) {
        self.kg_store = Some(store);
    }

    /// Wire the trait-routed episode store. Episode insert and
    /// similarity-search route through this handle.
    pub fn set_episode_store(&mut self, store: Arc<dyn zbot_stores_traits::EpisodeStore>) {
        self.episode_store = Some(store);
    }

    /// Wire the trait-routed wiki store (Phase E6b). When set, ward-wiki
    /// compilation routes through this handle. Falls back to `wiki_repo`.
    pub fn set_wiki_store(&mut self, store: Arc<dyn zbot_stores_traits::WikiStore>) {
        self.wiki_store = Some(store);
    }

    /// Wire the trait-routed distillation store (Phase E6c). Run-tracking
    /// writes (pending/skipped/success/error) flow through this when set;
    /// falls back to `distillation_repo` until E6c-4 drops the concrete
    /// field.
    pub fn set_distillation_store(
        &mut self,
        store: Arc<dyn zbot_stores_traits::DistillationStore>,
    ) {
        self.distillation_store = Some(store);
    }

    /// Wire the trait-routed procedure store (Phase E6b). Procedure
    /// upserts during distillation flow through this when set.
    pub fn set_procedure_store(&mut self, store: Arc<dyn zbot_stores_traits::ProcedureStore>) {
        self.procedure_store = Some(store);
    }

    /// Resolve the target provider ID and model for distillation.
    ///
    /// Resolution chain:
    /// 1. distillation.provider_id / distillation.model (if set)
    /// 2. orchestrator.provider_id / orchestrator.model (if set)
    /// 3. None (falls through to default provider in extract_all)
    fn resolve_distillation_target(&self) -> (Option<String>, Option<String>, u32) {
        let settings = self
            .settings_service
            .as_ref()
            .and_then(|s| s.get_execution_settings().ok());

        let settings = match settings {
            Some(s) => s,
            None => return (None, None, DEFAULT_MAX_OUTPUT_TOKENS),
        };

        let provider_id = settings
            .distillation
            .provider_id
            .clone()
            .or_else(|| settings.orchestrator.provider_id.clone());

        let model = settings
            .distillation
            .model
            .clone()
            .or_else(|| settings.orchestrator.model.clone());

        let max_tokens = settings
            .distillation
            .max_tokens
            .unwrap_or(settings.orchestrator.max_tokens);

        if provider_id.is_some() || model.is_some() {
            tracing::debug!(
                provider = ?provider_id,
                model = ?model,
                max_tokens,
                "Distillation using configured target"
            );
        }

        (provider_id, model, max_tokens)
    }

    /// Load the distillation prompt from filesystem or use embedded default.
    ///
    /// Checks for `config/distillation_prompt.md` in the vault directory.
    /// Falls back to the embedded `gateway/templates/distillation_prompt.md`
    /// (loaded via [`default_distillation_prompt`]) if not found.
    fn load_distillation_prompt(&self) -> String {
        let prompt_path = self.paths.distillation_prompt();

        match std::fs::read_to_string(&prompt_path) {
            Ok(content) if !content.trim().is_empty() => {
                tracing::info!("Loaded distillation prompt from {:?}", prompt_path);
                content
            }
            Ok(_) => {
                tracing::debug!("Distillation prompt file is empty, using default");
                default_distillation_prompt()
            }
            Err(_) => {
                // Write default to disk so user can customize it
                let default = default_distillation_prompt();
                if let Some(parent) = prompt_path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                if let Err(e) = std::fs::write(&prompt_path, &default) {
                    tracing::debug!("Failed to write default distillation prompt: {}", e);
                } else {
                    tracing::info!("Created default distillation prompt at {:?}", prompt_path);
                }
                default
            }
        }
    }

    /// Distill a completed session into memory facts.
    ///
    /// Returns the number of facts upserted. Records a `distillation_runs`
    /// entry when the repository is available — optimistic-failure pattern:
    /// insert with `status = 'failed'` up front, then update to `'success'`
    /// or `'skipped'` when the outcome is known.
    pub async fn distill(&self, session_id: &str, agent_id: &str) -> Result<usize, String> {
        let started = std::time::Instant::now();

        // 1. Load session messages
        let messages = self
            .messages
            .replay(session_id, None, MAX_MESSAGES_FOR_DISTILLATION)
            .map_err(|e| format!("Failed to load session messages: {}", e))?;

        if messages.len() < MIN_MESSAGES_FOR_DISTILLATION {
            tracing::debug!(
                session_id = %session_id,
                message_count = messages.len(),
                "Skipping distillation — too few messages"
            );
            // Record as skipped
            self.record_skipped(session_id).await;
            return Ok(0);
        }

        // Insert optimistic-failure record before attempting distillation
        self.record_pending(session_id).await;

        // Collect tool outputs from transcript for fact verification
        let tool_outputs: Vec<String> = messages
            .iter()
            .filter(|m| m.role == "tool")
            .map(|m| m.content.clone())
            .collect();

        // 2. Build transcript for the LLM
        let transcript = build_transcript(&messages);

        // 3. Call LLM for fact and entity extraction (with provider fallback)
        let response = match self.extract_all(&transcript).await {
            Ok(resp) => resp,
            Err(e) => {
                // The initial 'failed' record stays — update with error message
                self.record_error(session_id, &e).await;
                return Err(e);
            }
        };

        if response.facts.is_empty() && response.entities.is_empty() && response.episode.is_none() {
            tracing::info!(
                session_id = %session_id,
                "Distillation found nothing worth remembering"
            );
            let duration_ms = started.elapsed().as_millis() as i64;
            self.record_success(session_id, 0, 0, 0, false, duration_ms)
                .await;
            return Ok(0);
        }

        tracing::info!(
            session_id = %session_id,
            fact_count = response.facts.len(),
            entity_count = response.entities.len(),
            relationship_count = response.relationships.len(),
            "Distillation extracted {} facts, {} entities, {} relationships",
            response.facts.len(), response.entities.len(), response.relationships.len()
        );

        // 4. Upsert each fact with embedding — dedup against existing facts first
        let now = chrono::Utc::now().to_rfc3339();
        let mut upserted = 0;

        // Load existing facts for content-similarity dedup. SQLite-only —
        // the trait surface uses different listing semantics (paginated by
        // Trait-routed (Phase E6c). Backends without get_memory_facts
        // return empty, in which case distillation falls back to
        // key-equality dedup at upsert time.
        let existing_contents: Vec<(String, String)> = match self.memory_store.as_ref() {
            Some(store) => store
                .get_memory_facts(agent_id, None, 500)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|f| (f.key, f.content))
                .collect(),
            None => Vec::new(),
        };

        // Reserved key prefixes — only created via UI, never by distillation
        const RESERVED_PREFIXES: &[&str] = &["policy.", "instruction.", "user.profile"];

        for ef in &response.facts {
            // Skip reserved keys — these are user-managed via the Memory UI
            if RESERVED_PREFIXES.iter().any(|p| ef.key.starts_with(p)) {
                tracing::debug!(key = %ef.key, "Skipping reserved key (user-managed)");
                continue;
            }

            // Phase 5: distillation firewall for the ctx namespace.
            //
            // Session ctx facts (intent, prompt, plan, state.<exec>) are
            // written by runtime hooks, not by the LLM. They capture
            // SESSION-specific state that must not propagate into
            // cross-session patterns. If the distiller ever proposes a
            // category='ctx' fact — either because an LLM hallucinated one
            // or because the prompt accidentally invited it — reject it
            // here so it never reaches memory_facts.
            //
            // Inverse direction (reading): GatewayMemoryFactStore already
            // strips category='ctx' from recall results, so the distiller's
            // harvester never sees them as input. This write-side check is
            // the belt to that suspenders.
            if ef.category == "ctx" || ef.key.starts_with("ctx.") {
                tracing::warn!(
                    key = %ef.key,
                    category = %ef.category,
                    "Distillation firewall: rejected ctx-namespace write (session state must not be distilled)"
                );
                continue;
            }

            let verified_confidence =
                verify_fact_confidence(&ef.content, ef.confidence, &tool_outputs);

            // Skip facts with very low grounding
            if verified_confidence < 0.2 {
                tracing::debug!(key = %ef.key, confidence = verified_confidence, "Skipping ungrounded fact");
                continue;
            }

            // Content-similarity dedup: skip if an existing fact has 60%+ word overlap
            // (even with a different key). Prevents "user holds PTON" appearing 5 times.
            let new_words: std::collections::HashSet<&str> =
                ef.content.split_whitespace().collect();
            let is_duplicate = existing_contents
                .iter()
                .any(|(existing_key, existing_content)| {
                    if existing_key == &ef.key {
                        return false;
                    } // Same key = upsert, not dedup
                    let existing_words: std::collections::HashSet<&str> =
                        existing_content.split_whitespace().collect();
                    if new_words.is_empty() || existing_words.is_empty() {
                        return false;
                    }
                    let overlap = new_words.intersection(&existing_words).count();
                    let smaller = new_words.len().min(existing_words.len());
                    overlap as f64 / smaller as f64 > 0.6
                });
            if is_duplicate {
                tracing::debug!(key = %ef.key, "Skipping duplicate fact (60%+ content overlap with existing)");
                continue;
            }

            let fact_id = format!("fact-{}", uuid::Uuid::new_v4());

            // Embed the fact content
            let embedding = self.embed_text(&ef.content).await;

            let scope = "agent";
            let ward_id = "__global__";

            // Check if an active fact with the same key exists and has
            // different content. Trait-routed via `get_fact_by_key`;
            // backends without an impl return None and supersede
            // becomes a no-op (fresh upsert below).
            let existing_fact = match self.memory_store.as_ref() {
                Some(store) => store
                    .get_fact_by_key(agent_id, scope, ward_id, &ef.key)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };

            let fact = MemoryFact {
                id: fact_id.clone(),
                session_id: Some(session_id.to_string()),
                agent_id: agent_id.to_string(),
                scope: scope.to_string(),
                category: ef.category.clone(),
                key: ef.key.clone(),
                content: ef.content.clone(),
                confidence: verified_confidence,
                mention_count: 1,
                source_summary: Some(format!("Distilled from session {}", session_id)),
                embedding,
                ward_id: ward_id.to_string(),
                contradicted_by: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                expires_at: None,
                valid_from: Some(now.clone()),
                valid_until: None,
                superseded_by: None,
                pinned: false,
                epistemic_class: ef
                    .epistemic_class
                    .clone()
                    .or_else(|| Some("current".to_string())),
                source_episode_id: None,
                source_ref: None,
            };

            // Supersede the old fact if content differs
            if let Some(ref existing) = existing_fact {
                if existing.content != ef.content && !existing.pinned {
                    let supersede_res = match self.memory_store.as_ref() {
                        Some(store) => {
                            store
                                .supersede_fact(&existing.id, &fact_id, chrono::Utc::now())
                                .await
                        }
                        None => Err("no memory store wired".to_string()),
                    };
                    if let Err(e) = supersede_res {
                        tracing::warn!(
                            key = %ef.key,
                            old_id = %existing.id,
                            error = %e,
                            "Failed to supersede old fact"
                        );
                    } else {
                        tracing::debug!(
                            key = %ef.key,
                            old_id = %existing.id,
                            new_id = %fact_id,
                            "Superseded old fact with new content"
                        );
                    }
                }
            }

            let upsert_res = match self.memory_store.as_ref() {
                Some(store) => match serde_json::to_value(&fact) {
                    Ok(v) => store.upsert_typed_fact(v, fact.embedding.clone()).await,
                    Err(e) => Err(format!("encode fact: {e}")),
                },
                None => Err("no memory store wired".to_string()),
            };
            if let Err(e) = upsert_res {
                tracing::warn!(
                    key = %ef.key,
                    error = %e,
                    "Failed to upsert distilled fact"
                );
            } else {
                upserted += 1;
            }
        }

        // 5. Store entities and relationships in knowledge graph.
        //
        // Phase E6c: trait-routed. Block is a no-op when kg_store
        // isn't wired (defensive — production composition always
        // wires it).
        let mut kg_relationships_stored = 0_i32;
        let mut kg_relationships_dropped = 0_i32;
        if self.kg_store.is_some() {
            // Build entity map for relationship resolution
            let mut entity_map: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();

            for ee in &response.entities {
                // Check if entity already exists (dedup by name).
                let existing_id = self.find_entity_by_name(agent_id, &ee.name).await;
                match existing_id {
                    Some(id) => {
                        // Entity already exists — bump mention count and reuse ID
                        if let Err(e) = self.bump_entity_mention(&id).await {
                            tracing::warn!(entity = %ee.name, error = %e, "Failed to bump entity mention");
                        }
                        entity_map.insert(ee.name.clone(), id);
                    }
                    None => {
                        // Entity not found — create new
                        let mut entity = Entity::new(
                            agent_id.to_string(),
                            EntityType::from_str(&ee.entity_type),
                            ee.name.clone(),
                        );
                        entity.properties = ee.properties.clone();
                        match self.store_knowledge_one_entity(agent_id, entity).await {
                            Ok(id) => {
                                entity_map.insert(ee.name.clone(), id);
                            }
                            Err(e) => {
                                tracing::warn!(entity = %ee.name, error = %e, "Failed to store entity");
                            }
                        }
                    }
                }
            }

            // Canonicalize relationships before storage: fix common direction
            // errors from the LLM (passive voice inversions, agent/ward confusion).
            // See `canonicalize_relationship` for the rules.
            let canonicalized: Vec<_> = response
                .relationships
                .iter()
                .filter_map(|er| canonicalize_relationship(er, &response.entities))
                .collect();

            for er in &canonicalized {
                // Resolve both endpoints to real entity IDs (creating stubs
                // if the LLM only named them in the relationships block).
                let source_id = self
                    .resolve_relationship_endpoint(agent_id, &er.source, &mut entity_map)
                    .await;
                let target_id = self
                    .resolve_relationship_endpoint(agent_id, &er.target, &mut entity_map)
                    .await;
                let (Some(source_id), Some(target_id)) = (source_id, target_id) else {
                    kg_relationships_dropped += 1;
                    tracing::warn!(
                        source = %er.source,
                        target = %er.target,
                        kg_relationship_dropped = true,
                        "Dropped relationship because one or both endpoints were not persisted"
                    );
                    continue;
                };

                let relationship = Relationship::new(
                    agent_id.to_string(),
                    source_id,
                    target_id,
                    RelationshipType::from_str(&er.relationship_type),
                );

                if let Err(e) = self
                    .store_knowledge_one_relationship(agent_id, relationship)
                    .await
                {
                    tracing::warn!(
                        source = %er.source, target = %er.target,
                        error = %e, "Failed to store relationship"
                    );
                } else {
                    kg_relationships_stored += 1;
                }
            }
            if kg_relationships_dropped > 0 {
                tracing::warn!(
                    kg_relationship_dropped_count = kg_relationships_dropped,
                    kg_relationship_stored_count = kg_relationships_stored,
                    "Distillation dropped relationships with unresolved endpoints"
                );
            }
        }

        // 6. Store episode if extracted
        let mut episode_created = false;
        if let Some(ref extracted_episode) = response.episode {
            match self
                .store_episode(session_id, agent_id, extracted_episode, &now)
                .await
            {
                Ok(true) => {
                    episode_created = true;
                    tracing::info!(
                        session_id = %session_id,
                        outcome = %extracted_episode.outcome,
                        "Episode created from distillation"
                    );
                }
                Ok(false) => {
                    tracing::debug!(
                        session_id = %session_id,
                        "Episode extraction skipped — no episode repository"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "Failed to store episode — continuing with distillation"
                    );
                }
            }
        }

        // 6b. Store extracted procedure (if any) through the trait surface.
        if let Some(ref procedure) = response.procedure {
            let ward_id = self
                .session_meta
                .session_ward_id(session_id)
                .unwrap_or(None);
            self.process_procedure_upsert(agent_id, ward_id, procedure)
                .await;
        }

        let duration_ms = started.elapsed().as_millis() as i64;

        // 7. Record success in distillation_runs
        self.record_success(
            session_id,
            response.facts.len() as i32,
            response.entities.len() as i32,
            kg_relationships_stored,
            episode_created,
            duration_ms,
        )
        .await;

        tracing::info!(
            session_id = %session_id,
            upserted = upserted,
            episode_created = episode_created,
            duration_ms = duration_ms,
            "Session distillation complete"
        );

        // Ward memory-bank/ward.md is curated manually; distillation no longer
        // writes an auto-generated summary. Facts remain in the memory_facts DB.
        let ward_id = self
            .session_meta
            .session_ward_id(session_id)
            .unwrap_or(None);

        // 9. Compile ward wiki from extracted facts (best-effort)
        if let (Some(wiki_store), Some(ref wid)) = (&self.wiki_store, &ward_id) {
            if wid != "__global__" && wid != "scratch" {
                let fact_summaries: Vec<crate::ward_wiki::FactSummary> = response
                    .facts
                    .iter()
                    .map(|f| crate::ward_wiki::FactSummary {
                        category: f.category.clone(),
                        key: f.key.clone(),
                        content: f.content.clone(),
                    })
                    .collect();

                if !fact_summaries.is_empty() {
                    match self.build_llm_client() {
                        Ok(client) => {
                            let emb = self.embedding_client.as_deref();
                            match crate::ward_wiki::compile_ward_wiki(
                                wid,
                                agent_id,
                                &fact_summaries,
                                wiki_store.as_ref(),
                                &*client,
                                emb,
                            )
                            .await
                            {
                                Ok(count) => tracing::info!(
                                    ward = %wid, articles = count,
                                    "Wiki compilation complete"
                                ),
                                Err(e) => tracing::warn!(
                                    ward = %wid, error = %e,
                                    "Wiki compilation failed"
                                ),
                            }
                        }
                        Err(e) => {
                            tracing::warn!(ward = %wid, error = %e, "Wiki compilation skipped — no LLM client");
                        }
                    }
                }
            }
        }

        Ok(upserted)
    }

    // =========================================================================
    // Health-reporting helpers
    // =========================================================================

    /// Insert a pending/failed distillation run (optimistic failure).
    async fn record_pending(&self, session_id: &str) {
        if let Some(store) = &self.distillation_store {
            if let Err(e) = store
                .record_distillation_pending(session_id, "failed", Some("Distillation in progress"))
                .await
            {
                tracing::warn!(session_id = %session_id, error = %e, "Failed to insert distillation run record");
            }
        }
    }

    /// Record a skipped distillation (too few messages).
    async fn record_skipped(&self, session_id: &str) {
        if let Some(store) = &self.distillation_store {
            if let Err(e) = store
                .record_distillation_pending(session_id, "skipped", None)
                .await
            {
                tracing::warn!(session_id = %session_id, error = %e, "Failed to record skipped distillation");
            }
        }
    }

    /// Update an existing distillation run to success.
    async fn record_success(
        &self,
        session_id: &str,
        facts: i32,
        entities: i32,
        rels: i32,
        episode_created: bool,
        duration_ms: i64,
    ) {
        if let Some(store) = &self.distillation_store {
            if let Err(e) = store
                .record_distillation_success(
                    session_id,
                    facts,
                    entities,
                    rels,
                    episode_created,
                    duration_ms,
                )
                .await
            {
                tracing::warn!(session_id = %session_id, error = %e, "Failed to record distillation success");
            }
        }
    }

    /// Update an existing distillation run with an error message.
    async fn record_error(&self, session_id: &str, error: &str) {
        if let Some(store) = &self.distillation_store {
            if let Err(e) = store
                .record_distillation_failure(session_id, "failed", 0, Some(error))
                .await
            {
                tracing::warn!(session_id = %session_id, error = %e, "Failed to record distillation error");
            }
        }
    }

    /// Build an LLM client for wiki compilation using the distillation provider.
    fn build_llm_client(&self) -> Result<Arc<dyn LlmClient>, String> {
        let providers = self
            .provider_service
            .list()
            .map_err(|e| format!("Failed to list providers: {e}"))?;

        if providers.is_empty() {
            return Err("No providers configured".to_string());
        }

        let (target_provider_id, target_model, max_tokens) = self.resolve_distillation_target();

        // Pick target provider, or default, or first
        let provider = target_provider_id
            .as_ref()
            .and_then(|tid| {
                providers
                    .iter()
                    .find(|p| p.id.as_deref() == Some(tid.as_str()))
            })
            .or_else(|| providers.iter().find(|p| p.is_default))
            .or_else(|| providers.first())
            .ok_or_else(|| "No suitable provider found".to_string())?;

        let model = target_model.unwrap_or_else(|| provider.default_model().to_string());
        let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());

        let config = LlmConfig::new(
            provider.base_url.clone(),
            provider.api_key.clone(),
            model,
            provider_id,
        )
        .with_temperature(0.3)
        .with_max_tokens(max_tokens);

        let client =
            OpenAiClient::new(config).map_err(|e| format!("Failed to create LLM client: {e}"))?;

        Ok(Arc::new(client) as Arc<dyn LlmClient>)
    }

    /// Call the LLM to extract facts, entities, and relationships.
    ///
    /// Implements a provider fallback chain: tries the default provider first,
    /// then iterates through remaining providers if the LLM call fails.
    async fn extract_all(&self, transcript: &str) -> Result<DistillationResponse, String> {
        let providers = self
            .provider_service
            .list()
            .map_err(|e| format!("Failed to list providers: {}", e))?;

        if providers.is_empty() {
            return Err("No providers configured — cannot distill session".to_string());
        }

        // Load prompt once (shared across attempts)
        let system = self.load_distillation_prompt();

        // Compute session metrics to help the LLM decide on procedure extraction
        let metrics = compute_session_metrics(transcript);
        let user = format!(
            "## Session Metrics\n\n- Delegations: {}\n- Tool actions: {}\n- Distinct agents involved: {}\n\nProcedure extraction is REQUIRED if delegations >= 2 OR tool actions >= 3.\n\n## Session Transcript\n\n{}\n\n---\nExtract durable facts, entities, relationships, an episode assessment, AND a reusable procedure. Respond with ONLY the JSON object, nothing else.",
            metrics.delegations, metrics.tool_actions, metrics.distinct_agents, transcript
        );

        // Resolve distillation provider/model from settings chain:
        // distillation config → orchestrator config → default provider
        let (target_provider_id, target_model, max_tokens) = self.resolve_distillation_target();

        // Order providers: target first (if specified), then default, then rest
        let default_idx = providers.iter().position(|p| p.is_default);
        let target_idx = target_provider_id.as_ref().and_then(|tid| {
            providers
                .iter()
                .position(|p| p.id.as_deref() == Some(tid.as_str()))
        });

        let ordered_indices: Vec<usize> = {
            let mut indices = Vec::new();
            if let Some(idx) = target_idx {
                indices.push(idx);
            }
            if let Some(idx) = default_idx {
                if Some(idx) != target_idx {
                    indices.push(idx);
                }
            }
            for i in 0..providers.len() {
                if !indices.contains(&i) {
                    indices.push(i);
                }
            }
            indices
        };

        let mut last_error = String::new();

        for (attempt, &idx) in ordered_indices.iter().enumerate() {
            let provider = &providers[idx];
            // Use target model for first attempt (if configured), else provider default
            let model = if attempt == 0 {
                target_model
                    .clone()
                    .unwrap_or_else(|| provider.default_model().to_string())
            } else {
                provider.default_model().to_string()
            };
            let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());

            let config = LlmConfig::new(
                provider.base_url.clone(),
                provider.api_key.clone(),
                model.to_string(),
                provider_id.clone(),
            )
            .with_temperature(0.3)
            .with_max_tokens(max_tokens);

            let client = match OpenAiClient::new(config) {
                Ok(c) => Arc::new(c) as Arc<dyn LlmClient>,
                Err(e) => {
                    last_error = format!(
                        "Provider '{}': client creation failed: {}",
                        provider.name, e
                    );
                    tracing::warn!(
                        provider = %provider.name,
                        error = %e,
                        "Distillation: failed to create LLM client, trying next provider"
                    );
                    continue;
                }
            };

            let messages = vec![
                ChatMessage::system(system.clone()),
                ChatMessage::user(user.clone()),
            ];

            match client.chat(messages, None).await {
                Ok(response) => {
                    let content = &response.content;
                    tracing::debug!(
                        provider = %provider.name,
                        response_len = content.len(),
                        "Distillation LLM responded ({} chars)",
                        content.len()
                    );
                    match parse_distillation_response(content) {
                        Ok(parsed) => {
                            tracing::info!(
                                provider = %provider.name,
                                facts = parsed.facts.len(),
                                entities = parsed.entities.len(),
                                relationships = parsed.relationships.len(),
                                has_episode = parsed.episode.is_some(),
                                has_procedure = parsed.procedure.is_some(),
                                "Distillation parsed successfully"
                            );

                            // Diagnostic: if no procedure was extracted, log a preview of the
                            // raw response so we can see why. Procedure extraction is a major
                            // feature and silent failures are hard to debug otherwise.
                            if parsed.procedure.is_none() {
                                let preview = if content.len() > 1200 {
                                    &content[..1200]
                                } else {
                                    content.as_str()
                                };
                                tracing::warn!(
                                    provider = %provider.name,
                                    response_preview = %preview,
                                    "No procedure extracted — LLM did not include procedure field or returned null"
                                );
                            }

                            return Ok(parsed);
                        }
                        Err(parse_err) => {
                            // Log the raw response so we can debug what the LLM returned
                            let preview = if content.len() > 800 {
                                &content[..800]
                            } else {
                                content.as_str()
                            };
                            tracing::warn!(
                                provider = %provider.name,
                                error = %parse_err,
                                response_preview = %preview,
                                "Distillation response could not be parsed — trying next provider"
                            );
                            last_error = format!(
                                "Provider '{}': parse failed: {}",
                                provider.name, parse_err
                            );
                            continue; // Try next provider — a different model might produce parseable JSON
                        }
                    }
                }
                Err(e) => {
                    last_error = format!(
                        "Provider '{}' ({}): LLM call failed: {}",
                        provider.name, provider_id, e
                    );
                    tracing::warn!(
                        provider = %provider.name,
                        provider_id = %provider_id,
                        error = %e,
                        "Distillation LLM call failed, trying next provider"
                    );
                }
            }
        }

        Err(format!(
            "All providers failed for distillation. Last error: {}",
            last_error
        ))
    }

    // =========================================================================
    // Episode storage and strategy emergence
    // =========================================================================

    /// Store an extracted episode and attempt strategy emergence.
    ///
    /// Returns `Ok(true)` if the episode was stored, `Ok(false)` if no repo,
    /// or `Err` on failure.
    async fn store_episode(
        &self,
        session_id: &str,
        agent_id: &str,
        extracted: &ExtractedEpisode,
        now: &str,
    ) -> Result<bool, String> {
        // Episode storage runs through the trait surface.
        if self.episode_store.is_none() {
            return Ok(false);
        }

        // Look up ward_id from the sessions table
        let ward_id = self
            .session_meta
            .session_ward_id(session_id)
            .unwrap_or(None)
            .unwrap_or_else(|| "__global__".to_string());

        // Embed the task summary for similarity search
        let embedding = self.embed_text(&extracted.task_summary).await;

        let episode = SessionEpisode {
            id: format!("ep-{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            ward_id: ward_id.clone(),
            task_summary: extracted.task_summary.clone(),
            outcome: extracted.outcome.clone(),
            strategy_used: extracted.strategy_used.clone(),
            key_learnings: extracted.key_learnings.clone(),
            token_cost: None,
            embedding: embedding.clone(),
            created_at: now.to_string(),
        };

        self.insert_episode_internal(&episode).await?;

        // Attempt strategy emergence for successful episodes
        if extracted.outcome == "success" {
            if let Err(e) = self
                .try_emerge_strategy(agent_id, &ward_id, &episode, embedding.as_deref(), now)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Strategy emergence failed — non-fatal"
                );
            }
        }

        // Attempt failure clustering for failed/partial episodes
        if extracted.outcome == "failed" || extracted.outcome == "partial" {
            if let Err(e) = self
                .try_cluster_failures(agent_id, &episode, &ward_id)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Failure clustering failed (non-fatal)"
                );
            }
        }

        Ok(true)
    }

    /// Attempt to emerge a strategy from repeated successful episodes.
    ///
    /// If 2+ similar successful episodes exist for this agent, extract the
    /// common strategy pattern and upsert it as a `strategy` memory fact.
    async fn try_emerge_strategy(
        &self,
        agent_id: &str,
        ward_id: &str,
        episode: &SessionEpisode,
        embedding: Option<&[f32]>,
        now: &str,
    ) -> Result<(), String> {
        if self.episode_store.is_none() {
            return Ok(());
        }

        let query_embedding = match embedding {
            Some(emb) => emb,
            None => return Ok(()), // No embedding — cannot search by similarity
        };

        // Search for similar episodes (trait preferred)
        let similar = self
            .search_episodes_by_similarity_internal(agent_id, query_embedding, 0.7, 10)
            .await?;

        // Filter to only successful episodes (excluding the one we just inserted)
        let successful_similar: Vec<_> = similar
            .into_iter()
            .filter(|(ep, _score)| ep.outcome == "success" && ep.id != episode.id)
            .collect();

        // Need at least 2 similar successful episodes (the new one + 2 existing = pattern)
        if successful_similar.len() < 2 {
            return Ok(());
        }

        // Extract strategy: use the most recent episode's strategy_used
        let strategy_description = episode
            .strategy_used
            .as_deref()
            .or_else(|| {
                successful_similar
                    .first()
                    .and_then(|(ep, _)| ep.strategy_used.as_deref())
            })
            .unwrap_or("Repeated successful approach")
            .to_string();

        // Derive a sanitized key from the task summary
        let task_type = sanitize_task_type(&episode.task_summary);
        let fact_key = format!("strategy.{}", task_type);

        tracing::info!(
            agent_id = %agent_id,
            key = %fact_key,
            similar_count = successful_similar.len(),
            "Strategy emerged from {} similar successful episodes",
            successful_similar.len()
        );

        // Upsert the strategy as a memory fact
        let strategy_fact_id = format!("fact-{}", uuid::Uuid::new_v4());

        // Check for existing strategy fact to supersede via the
        // trait-routed memory store (Phase E6c).
        let existing_strategy = match self.memory_store.as_ref() {
            Some(store) => store
                .get_fact_by_key(agent_id, "agent", ward_id, &fact_key)
                .await
                .ok()
                .flatten(),
            None => None,
        };

        let fact = MemoryFact {
            id: strategy_fact_id.clone(),
            session_id: Some(episode.session_id.clone()),
            agent_id: agent_id.to_string(),
            scope: "agent".to_string(),
            category: "strategy".to_string(),
            key: fact_key.clone(),
            content: strategy_description.clone(),
            confidence: 0.92,
            mention_count: 1,
            source_summary: Some(format!(
                "Emerged from {} similar successful episodes in ward '{}'",
                successful_similar.len() + 1,
                ward_id,
            )),
            embedding: embedding.map(|e| e.to_vec()),
            ward_id: ward_id.to_string(),
            contradicted_by: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            expires_at: None,
            valid_from: Some(now.to_string()),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("procedural".to_string()),
            source_episode_id: None,
            source_ref: None,
        };

        // Supersede old strategy if content differs
        if let Some(ref existing) = existing_strategy {
            if existing.content != strategy_description && !existing.pinned {
                let supersede_res = match self.memory_store.as_ref() {
                    Some(store) => {
                        store
                            .supersede_fact(&existing.id, &strategy_fact_id, chrono::Utc::now())
                            .await
                    }
                    None => Err("no memory store wired".to_string()),
                };
                if let Err(e) = supersede_res {
                    tracing::warn!(
                        key = %fact_key,
                        error = %e,
                        "Failed to supersede old strategy fact"
                    );
                } else {
                    tracing::debug!(
                        key = %fact_key,
                        old_id = %existing.id,
                        new_id = %strategy_fact_id,
                        "Superseded old strategy fact"
                    );
                }
            }
        }

        match self.memory_store.as_ref() {
            Some(store) => {
                let v = serde_json::to_value(&fact).map_err(|e| format!("encode fact: {e}"))?;
                store.upsert_typed_fact(v, fact.embedding.clone()).await?;
            }
            None => return Err("no memory store wired".to_string()),
        }

        Ok(())
    }

    /// Attempt to cluster repeated failures and generate a correction fact.
    ///
    /// If 3+ similar failed/partial episodes exist for this agent, extract the
    /// common failure pattern and upsert it as a `correction` memory fact.
    async fn try_cluster_failures(
        &self,
        agent_id: &str,
        episode: &SessionEpisode,
        ward_id: &str,
    ) -> Result<(), String> {
        if self.episode_store.is_none() {
            return Ok(());
        }

        // Embed the task summary for similarity search
        let embedding = self.embed_text(&episode.task_summary).await;
        let query_embedding = match embedding.as_deref() {
            Some(emb) => emb,
            None => return Ok(()), // No embedding — cannot search by similarity
        };

        // Search for similar episodes (trait preferred; wider threshold than
        // strategy: 0.6 vs 0.7)
        let similar = self
            .search_episodes_by_similarity_internal(agent_id, query_embedding, 0.6, 20)
            .await?;

        // Filter to only failed/partial episodes (excluding the one we just inserted)
        let failed_similar: Vec<_> = similar
            .into_iter()
            .filter(|(ep, _score)| {
                (ep.outcome == "failed" || ep.outcome == "partial")
                    && ep.session_id != episode.session_id
            })
            .collect();

        // Need at least 3 similar failed episodes to form a cluster
        if failed_similar.len() < 3 {
            return Ok(());
        }

        let cluster_size = failed_similar.len();

        // Extract the common failure pattern from key_learnings
        let latest_key_learning = episode
            .key_learnings
            .as_deref()
            .or_else(|| {
                failed_similar
                    .first()
                    .and_then(|(ep, _)| ep.key_learnings.as_deref())
            })
            .unwrap_or("Repeated failure without specific learning")
            .to_string();

        // Derive a sanitized key from the task summary
        let task_type = sanitize_task_type(&episode.task_summary);
        let fact_key = format!("correction.recurring.{}", task_type);
        let now = chrono::Utc::now().to_rfc3339();

        tracing::info!(
            agent_id = %agent_id,
            key = %fact_key,
            cluster_size = cluster_size,
            "Failure cluster detected from {} similar failed episodes",
            cluster_size
        );

        // Upsert the correction as a memory fact
        let correction_fact_id = format!("fact-{}", uuid::Uuid::new_v4());
        let correction_content = format!(
            "Recurring failure ({} episodes): {}",
            cluster_size, latest_key_learning
        );

        // Check for existing correction fact to supersede via the
        // trait-routed memory store (Phase E6c).
        let existing_correction = match self.memory_store.as_ref() {
            Some(store) => store
                .get_fact_by_key(agent_id, "agent", ward_id, &fact_key)
                .await
                .ok()
                .flatten(),
            None => None,
        };

        let fact = MemoryFact {
            id: correction_fact_id.clone(),
            session_id: Some(episode.session_id.clone()),
            agent_id: agent_id.to_string(),
            scope: "agent".to_string(),
            category: "correction".to_string(),
            key: fact_key.clone(),
            content: correction_content.clone(),
            confidence: (0.85 + 0.02 * cluster_size as f64).min(0.98),
            mention_count: cluster_size as i32,
            source_summary: Some("Clustered from repeated failures".to_string()),
            embedding: embedding.clone(),
            ward_id: ward_id.to_string(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            expires_at: None,
            valid_from: Some(now),
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("convention".to_string()),
            source_episode_id: None,
            source_ref: None,
        };

        // Supersede old correction if content differs
        if let Some(ref existing) = existing_correction {
            if existing.content != correction_content && !existing.pinned {
                let supersede_res = match self.memory_store.as_ref() {
                    Some(store) => {
                        store
                            .supersede_fact(&existing.id, &correction_fact_id, chrono::Utc::now())
                            .await
                    }
                    None => Err("no memory store wired".to_string()),
                };
                if let Err(e) = supersede_res {
                    tracing::warn!(
                        key = %fact_key,
                        error = %e,
                        "Failed to supersede old correction fact"
                    );
                } else {
                    tracing::debug!(
                        key = %fact_key,
                        old_id = %existing.id,
                        new_id = %correction_fact_id,
                        "Superseded old correction fact"
                    );
                }
            }
        }

        match self.memory_store.as_ref() {
            Some(store) => {
                let v = serde_json::to_value(&fact).map_err(|e| format!("encode fact: {e}"))?;
                store.upsert_typed_fact(v, fact.embedding.clone()).await?;
            }
            None => return Err("no memory store wired".to_string()),
        }

        Ok(())
    }

    // =========================================================================
    // Episode helpers (trait-routed)
    // =========================================================================

    /// Insert a session episode. Trait-routed when `episode_store` is set;
    /// falls back to the SQLite `episode_repo`. Returns Err only when both
    /// are unwired (caller is expected to gate on availability beforehand).
    async fn insert_episode_internal(&self, episode: &SessionEpisode) -> Result<(), String> {
        if let Some(store) = &self.episode_store {
            let v = serde_json::to_value(episode).map_err(|e| format!("encode episode: {e}"))?;
            let emb = episode.embedding.clone();
            store.insert_episode(v, emb).await.map(|_| ())
        } else {
            Err("no episode store wired".to_string())
        }
    }

    /// Vector-similarity search for episodes. Trait-routed when wired,
    /// falls back to SQLite repo. Returns the canonical
    /// `Vec<(SessionEpisode, f64)>` shape regardless of backend.
    async fn search_episodes_by_similarity_internal(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f64,
        limit: usize,
    ) -> Result<Vec<(SessionEpisode, f64)>, String> {
        let store = match &self.episode_store {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        let raw = store
            .search_episodes_by_similarity(agent_id, embedding, threshold as f32, limit)
            .await?;
        // Trait emits Value with shape `{ "episode": <SessionEpisode>, "score": <f64> }`.
        let pairs: Vec<(SessionEpisode, f64)> = raw
            .into_iter()
            .filter_map(|v| {
                let score = v.get("score").and_then(|s| s.as_f64())?;
                let ep_v = v.get("episode").cloned()?;
                let ep = serde_json::from_value::<SessionEpisode>(ep_v).ok()?;
                Some((ep, score))
            })
            .collect();
        Ok(pairs)
    }

    // =========================================================================
    // Knowledge graph helpers (trait-routed)
    // =========================================================================

    /// Find an entity by exact name via the trait `kg_store`
    /// (case-insensitive name match via `search_entities_by_name`).
    async fn find_entity_by_name(&self, agent_id: &str, name: &str) -> Option<String> {
        let store = self.kg_store.as_ref()?;
        store
            .get_entity_by_name(agent_id, name)
            .await
            .ok()
            .flatten()
            .map(|e| e.id)
    }

    /// Bump an entity's mention counter via the trait.
    async fn bump_entity_mention(&self, id: &str) -> Result<(), String> {
        let store = self
            .kg_store
            .as_ref()
            .ok_or_else(|| "no kg store wired".to_string())?;
        store
            .bump_entity_mention(&zbot_stores::EntityId::from(id.to_string()))
            .await
            .map_err(|e| e.to_string())
    }

    /// Persist a single new entity via the trait.
    async fn store_knowledge_one_entity(
        &self,
        agent_id: &str,
        entity: Entity,
    ) -> Result<String, String> {
        let store = self
            .kg_store
            .as_ref()
            .ok_or_else(|| "no kg store wired".to_string())?;
        store
            .upsert_entity(agent_id, entity)
            .await
            .map(|id| id.0)
            .map_err(|e| e.to_string())
    }

    /// Persist a single new relationship via the trait.
    async fn store_knowledge_one_relationship(
        &self,
        agent_id: &str,
        rel: Relationship,
    ) -> Result<(), String> {
        let store = self
            .kg_store
            .as_ref()
            .ok_or_else(|| "no kg store wired".to_string())?;
        store
            .upsert_relationship(agent_id, rel)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Resolve a relationship endpoint (source/target name) to a real
    /// entity id. Lookup order: cached `entity_map` -> trait
    /// `find_entity_by_name` -> auto-create a stub of type `custom("unknown")`
    /// so the FK constraint on `relationship` is satisfied. Returns `None`
    /// when a stub cannot be persisted; callers must drop the relationship
    /// instead of writing an FK-invalid edge.
    async fn resolve_relationship_endpoint(
        &self,
        agent_id: &str,
        name: &str,
        entity_map: &mut std::collections::HashMap<String, String>,
    ) -> Option<String> {
        if let Some(id) = entity_map.get(name) {
            return Some(id.clone());
        }
        if let Some(existing_id) = self.find_entity_by_name(agent_id, name).await {
            entity_map.insert(name.to_string(), existing_id.clone());
            return Some(existing_id);
        }
        // Nothing found — create a stub so the relationship can be persisted.
        let stub = Entity::new(
            agent_id.to_string(),
            EntityType::Custom("unknown".to_string()),
            name.to_string(),
        );
        if let Err(e) = self.store_knowledge_one_entity(agent_id, stub).await {
            tracing::warn!(
                name = %name,
                error = %e,
                "Failed to auto-create stub entity for relationship endpoint"
            );
            None
        } else if let Some(stub_id) = self.find_entity_by_name(agent_id, name).await {
            tracing::debug!(
                name = %name,
                id = %stub_id,
                "Auto-created stub entity for undeclared relationship endpoint"
            );
            entity_map.insert(name.to_string(), stub_id.clone());
            Some(stub_id)
        } else {
            tracing::warn!(
                name = %name,
                kg_relationship_dropped = true,
                "Auto-created stub entity could not be resolved after persistence"
            );
            None
        }
    }

    // =========================================================================
    // Procedure upsert
    // =========================================================================

    /// Build the `Procedure` row and route it through the trait-routed
    /// `procedure_store`. Embeds `{name}\n{description}` before upsert so
    /// the SQLite `procedures_index` vec0 table is populated. Mirrors the
    /// `PatternExtractor` embed shape for consistency across the two
    /// write paths. Graceful: when `embed_text` returns `None` (no client
    /// or embed failure) the upsert proceeds with `None` and only
    /// vec-similarity recall is lost — identical to today's behavior.
    /// No-op when `procedure_store` is unwired.
    async fn process_procedure_upsert(
        &self,
        agent_id: &str,
        ward_id: Option<String>,
        procedure: &ExtractedProcedure,
    ) {
        let Some(store) = self.procedure_store.as_ref() else {
            return;
        };

        let steps_json = serde_json::to_string(&procedure.steps).unwrap_or_default();
        let params_json = procedure
            .parameters
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_default());

        let proc = Procedure {
            id: format!("proc-{}", uuid::Uuid::new_v4()),
            agent_id: agent_id.to_string(),
            ward_id: ward_id.or_else(|| Some("__global__".to_string())),
            name: procedure.name.clone(),
            description: procedure.description.clone(),
            trigger_pattern: procedure.trigger_pattern.clone(),
            steps: steps_json,
            parameters: params_json,
            // Distillation synthesizes from a single completed session, so the
            // evidence count is 1. PatternExtractor seeds at 2 because it
            // requires a matched pair. Either way, see `default_initial_success_count`
            // for why this is intentional rather than hardcoded.
            success_count: 1,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: Some(chrono::Utc::now().to_rfc3339()),
            embedding: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        let embed_input = format!("{}\n{}", procedure.name, procedure.description);
        let embedding = self.embed_text(&embed_input).await;

        let upsert_res = match serde_json::to_value(&proc) {
            Ok(v) => store.upsert_procedure(v, embedding).await,
            Err(e) => Err(format!("encode procedure: {e}")),
        };

        match upsert_res {
            Ok(()) => tracing::info!(
                name = %procedure.name, "Stored procedure from session"
            ),
            Err(e) => tracing::warn!(
                name = %procedure.name, error = %e, "Failed to store procedure"
            ),
        }
    }

    // =========================================================================
    // Embedding
    // =========================================================================

    /// Embed a single text, with caching via the trait surface.
    async fn embed_text(&self, text: &str) -> Option<Vec<f32>> {
        let client = self.embedding_client.as_ref()?;
        let model_name = client.model_name().to_string();
        let hash = agent_runtime::content_hash(text);

        // Cache lookup via the trait store (Phase E6c).
        if let Some(store) = self.memory_store.as_ref() {
            if let Ok(Some(cached)) = store.get_cached_embedding(&hash, &model_name).await {
                return Some(cached);
            }
        }

        // Generate embedding
        match client.embed(&[text]).await {
            Ok(mut embeddings) if !embeddings.is_empty() => {
                let emb = embeddings.remove(0);
                // Best-effort cache write — backends without a cache table no-op.
                if let Some(store) = self.memory_store.as_ref() {
                    let _ = store.cache_embedding(&hash, &model_name, &emb).await;
                }
                Some(emb)
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!("Failed to embed text for distillation: {}", e);
                None
            }
        }
    }
}

/// Test-only synchronous helper that mirrors
/// `SessionDistiller::resolve_relationship_endpoint` against a concrete
/// `GraphStorage`. Production calls the trait-routed instance method;
/// this exists so the FK-survival contract test below can run without
/// having to construct a full SessionDistiller (provider service +
/// conversation repo + paths + settings + embedding client).
#[cfg(test)]
fn resolve_relationship_endpoint(
    graph: &zbot_stores_sqlite::kg::storage::GraphStorage,
    agent_id: &str,
    name: &str,
    entity_map: &mut std::collections::HashMap<String, String>,
) -> Option<String> {
    if let Some(id) = entity_map.get(name) {
        return Some(id.clone());
    }
    if let Ok(Some(existing_id)) = graph.find_entity_by_name(agent_id, name) {
        entity_map.insert(name.to_string(), existing_id.clone());
        return Some(existing_id);
    }
    let stub = Entity::new(
        agent_id.to_string(),
        EntityType::Custom("unknown".to_string()),
        name.to_string(),
    );
    let knowledge = knowledge_graph::types::ExtractedKnowledge {
        entities: vec![stub],
        relationships: vec![],
    };
    if graph.store_knowledge(agent_id, knowledge).is_err() {
        return None;
    }
    let Ok(Some(stub_id)) = graph.find_entity_by_name(agent_id, name) else {
        return None;
    };
    entity_map.insert(name.to_string(), stub_id.clone());
    Some(stub_id)
}

/// Canonicalize a relationship extracted by the LLM — fix common direction
/// errors before persisting to the knowledge graph.
///
/// 1. Passive voice normalization (`analyzed_by`, `used_by`, `created_by`):
///    swap source and target if the LLM inverted them.
/// 2. Agent-ticker confusion: drop nonsensical
///    `agent --analyzed_by--> ticker` edges rather than store garbage.
/// 3. Bidirectional redundancy: keep only `uses` when both `uses` and
///    `used_by` appear between the same pair.
///
/// Returns `None` to drop the relationship, `Some(canonical)` to keep it.
fn canonicalize_relationship(
    er: &ExtractedRelationship,
    entities: &[ExtractedEntity],
) -> Option<ExtractedRelationship> {
    let rel = er.relationship_type.to_lowercase();
    let rel = rel.trim();

    // Rule 2: drop nonsensical agent-is-analyzed-by-ticker relationships.
    // These appear when the LLM confuses who is acting on whom.
    if (rel == "analyzed_by" || rel == "analyzedby") && is_agent_name(&er.source) {
        tracing::debug!(
            source = %er.source,
            target = %er.target,
            "Dropping nonsensical agent--analyzed_by--> relationship"
        );
        return None;
    }

    // Rule 3: canonicalize `used_by` / `usedby` into inverse `uses`.
    // `A used_by B` means `B uses A` — swap source/target and normalize type.
    if rel == "used_by" || rel == "usedby" || rel == "usedfor" || rel == "used_for" {
        return Some(ExtractedRelationship {
            source: er.target.clone(),
            target: er.source.clone(),
            relationship_type: "uses".to_string(),
        });
    }

    // Rule 3b: canonicalize `created_by` / `createdby` into inverse `created`.
    if rel == "created_by" || rel == "createdby" {
        return Some(ExtractedRelationship {
            source: er.target.clone(),
            target: er.source.clone(),
            relationship_type: "created".to_string(),
        });
    }

    // Rule 1: for `analyzed_by`, ensure target is the analyzer (ward/workspace),
    // not the analyzed thing. If source is a workspace-like entity and target
    // is a ticker/person/organization, swap them — the LLM inverted.
    if rel == "analyzed_by" || rel == "analyzedby" {
        let source_type = entity_type_for(&er.source, entities);
        let target_type = entity_type_for(&er.target, entities);

        // Ticker/person/org analyzed_by ward/project — correct direction.
        let source_is_subject = matches!(
            source_type.as_deref(),
            Some("organization") | Some("person") | Some("concept")
        );
        let target_is_analyzer = matches!(target_type.as_deref(), Some("project") | Some("ward"));

        // If inverted (analyzer is source), swap.
        let source_is_analyzer = matches!(source_type.as_deref(), Some("project") | Some("ward"));
        let target_is_subject = matches!(
            target_type.as_deref(),
            Some("organization") | Some("person") | Some("concept")
        );

        if !source_is_subject && !target_is_analyzer && source_is_analyzer && target_is_subject {
            return Some(ExtractedRelationship {
                source: er.target.clone(),
                target: er.source.clone(),
                relationship_type: "analyzed_by".to_string(),
            });
        }
    }

    // Default: keep as-is, normalize type name.
    Some(ExtractedRelationship {
        source: er.source.clone(),
        target: er.target.clone(),
        relationship_type: rel.to_string(),
    })
}

/// Heuristic: does the name look like an agent? (e.g. "planner-agent", "code-agent")
fn is_agent_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with("-agent") || lower.ends_with("_agent") || lower == "agent"
}

/// Look up the entity type for a name from the extracted entities list.
fn entity_type_for(name: &str, entities: &[ExtractedEntity]) -> Option<String> {
    let lower = name.to_lowercase();
    entities
        .iter()
        .find(|e| e.name.to_lowercase() == lower)
        .map(|e| e.entity_type.to_lowercase())
}

/// Derive a sanitized task type from a task summary for use as a fact key.
///
/// Takes the first few words, lowercases them, replaces spaces and dots with
/// underscores, and caps length to keep the key concise.
fn sanitize_task_type(task_summary: &str) -> String {
    task_summary
        .to_lowercase()
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join("_")
        .replace('.', "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .take(40)
        .collect()
}

/// Session metrics used to inform the LLM about procedure extraction gating.
struct SessionMetrics {
    delegations: usize,
    tool_actions: usize,
    distinct_agents: usize,
}

/// Track distinct `*-agent` tokens mentioned on a line. Extracted to keep
/// `compute_session_metrics` under the cognitive-complexity threshold.
fn extract_agent_mentions(line: &str, agents: &mut std::collections::HashSet<String>) {
    for word in line.split_whitespace() {
        if !(word.ends_with("-agent") || word.ends_with("-agent,")) {
            continue;
        }
        let clean = word.trim_end_matches(',').trim_matches('"');
        if clean.len() < 40 {
            agents.insert(clean.to_string());
        }
    }
}

/// Compute basic metrics from a compiled transcript to help the LLM decide
/// whether a procedure should be extracted.
fn compute_session_metrics(transcript: &str) -> SessionMetrics {
    let mut delegations = 0usize;
    let mut tool_actions = 0usize;
    let mut agents: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in transcript.lines() {
        let lower = line.to_lowercase();
        if lower.contains("delegate_to_agent") || line.contains("## From ") {
            delegations += 1;
        }
        if line.contains("[called:") {
            tool_actions += 1;
        }
        extract_agent_mentions(line, &mut agents);
    }

    SessionMetrics {
        delegations,
        tool_actions,
        distinct_agents: agents.len(),
    }
}

/// Build a compact transcript from session messages.
fn build_transcript(messages: &[zbot_conversation::Message]) -> String {
    let mut parts = Vec::with_capacity(messages.len());

    for msg in messages {
        let role = match msg.role.as_str() {
            "user" => "USER",
            "assistant" => "ASSISTANT",
            "system" => continue, // Skip system messages — they're instructions, not conversation
            "tool" => "TOOL_RESULT",
            _ => &msg.role,
        };

        // For tool results, extract the meaningful content, not raw JSON
        let content = if msg.role == "tool" {
            summarize_tool_result(&msg.content)
        } else if msg.content.len() > 1000 {
            format!(
                "{}... [truncated, {} chars total]",
                agent_primitives::truncate_str(&msg.content, 1000),
                msg.content.len()
            )
        } else {
            msg.content.clone()
        };

        // For assistant messages with tool calls, show what tools were called
        let tool_info = if let Some(tc) = &msg.tool_calls {
            match serde_json::from_str::<Vec<serde_json::Value>>(tc) {
                Ok(calls) => {
                    let names: Vec<String> = calls
                        .iter()
                        .filter_map(|c| {
                            c.get("tool_name")
                                .or(c.get("name"))
                                .and_then(|n| n.as_str())
                                .map(String::from)
                        })
                        .collect();
                    if names.is_empty() {
                        String::new()
                    } else {
                        format!(" [called: {}]", names.join(", "))
                    }
                }
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        // Skip empty content (sometimes tool call messages have "[tool calls]" as placeholder)
        if content.is_empty() || content == "[tool calls]" {
            if !tool_info.is_empty() {
                parts.push(format!("ASSISTANT:{}", tool_info));
            }
            continue;
        }

        parts.push(format!("{}: {}{}", role, content, tool_info));
    }

    parts.join("\n\n")
}

/// Summarize a tool result for the distillation transcript.
/// Extracts meaningful content from JSON tool responses, reducing noise.
fn summarize_tool_result(content: &str) -> String {
    // Try to parse as JSON and extract key fields
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(obj) = val.as_object() {
            // Common tool result patterns
            if let Some(stdout) = obj.get("stdout").and_then(|v| v.as_str()) {
                let exit_code = obj.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
                let stderr = obj.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
                let mut result = format!("[exit_code: {}]", exit_code);
                if !stdout.trim().is_empty() {
                    let truncated = if stdout.len() > 500 {
                        &stdout[..500]
                    } else {
                        stdout
                    };
                    result.push_str(&format!(" {}", truncated.trim()));
                }
                if !stderr.trim().is_empty() && exit_code != 0 {
                    let truncated = if stderr.len() > 300 {
                        &stderr[..300]
                    } else {
                        stderr
                    };
                    result.push_str(&format!(" STDERR: {}", truncated.trim()));
                }
                return result;
            }
            // Delegation result
            if let Some(message) = obj.get("message").and_then(|v| v.as_str()) {
                return format!(
                    "[delegation result] {}",
                    if message.len() > 500 {
                        &message[..500]
                    } else {
                        message
                    }
                );
            }
            // Ward change
            if obj.get("__ward_changed__").is_some() {
                let action = obj
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("changed");
                return format!("[ward {}]", action);
            }
        }
    }
    // Fallback: truncate raw content
    if content.len() > 500 {
        format!(
            "{}... [truncated]",
            agent_primitives::truncate_str(content, 500)
        )
    } else {
        content.to_string()
    }
}

/// Parse the full distillation response (facts + entities + relationships).
///
/// The LLM might return:
/// - A JSON object: `{"facts": [...], "entities": [...], "relationships": [...]}`
/// - Just a JSON array of facts (backward compat): `[{...}, ...]`
/// - JSON wrapped in markdown code block
/// - JSON with surrounding explanation text
///
/// Returns Err if the response cannot be parsed at all — this is a real failure,
/// not "nothing worth remembering".
fn parse_distillation_response(content: &str) -> Result<DistillationResponse, String> {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        return Err("LLM returned empty response".to_string());
    }

    // Try parsing as full distillation response (object with facts/entities/relationships)
    if let Ok(resp) = serde_json::from_str::<DistillationResponse>(trimmed) {
        return Ok(resp);
    }

    // Try parsing as just a facts array (backward compat)
    if let Ok(facts) = serde_json::from_str::<Vec<ExtractedFact>>(trimmed) {
        return Ok(DistillationResponse {
            facts,
            entities: Vec::new(),
            relationships: Vec::new(),
            episode: None,
            procedure: None,
        });
    }

    // Try extracting JSON from markdown code blocks or surrounding text
    let json_str = extract_json_from_content(trimmed);

    if let Ok(resp) = serde_json::from_str::<DistillationResponse>(&json_str) {
        return Ok(resp);
    }

    if let Ok(facts) = serde_json::from_str::<Vec<ExtractedFact>>(&json_str) {
        return Ok(DistillationResponse {
            facts,
            entities: Vec::new(),
            relationships: Vec::new(),
            episode: None,
            procedure: None,
        });
    }

    // Try a lenient parse — maybe the LLM returned valid JSON but with extra/different field names
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
        return parse_distillation_from_value(&val);
    }

    // All parsing failed — this is a real error, not "nothing to extract"
    let preview = if trimmed.len() > 500 {
        &trimmed[..500]
    } else {
        trimmed
    };
    Err(format!(
        "Failed to parse distillation response. Preview: {}",
        preview
    ))
}

/// Try to extract a DistillationResponse from an arbitrary JSON Value.
/// Handles cases where the LLM uses slightly different field names or structures.
fn parse_distillation_from_value(val: &serde_json::Value) -> Result<DistillationResponse, String> {
    let obj = val.as_object().ok_or("Response is not a JSON object")?;

    let facts: Vec<ExtractedFact> = obj
        .get("facts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let entities: Vec<ExtractedEntity> = obj
        .get("entities")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let relationships: Vec<ExtractedRelationship> = obj
        .get("relationships")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let episode: Option<ExtractedEpisode> = obj
        .get("episode")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let procedure: Option<ExtractedProcedure> = obj
        .get("procedure")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    if facts.is_empty() && entities.is_empty() && episode.is_none() {
        // The JSON parsed but all arrays were empty or had incompatible fields
        // Log what was actually in the JSON to help debug
        let keys: Vec<&String> = obj.keys().collect();
        tracing::warn!(
            keys = ?keys,
            "Parsed JSON but extracted nothing — check if LLM used unexpected field names"
        );
    }

    Ok(DistillationResponse {
        facts,
        entities,
        relationships,
        episode,
        procedure,
    })
}

/// Extract JSON content from text that may contain markdown code blocks.
fn extract_json_from_content(content: &str) -> String {
    // 1. Try markdown code blocks first: ```json ... ``` or ``` ... ```
    let code_block_patterns = ["```json\n", "```json\r\n", "```JSON\n", "```\n", "```\r\n"];
    for pattern in &code_block_patterns {
        if let Some(start) = content.find(pattern) {
            let json_start = start + pattern.len();
            if let Some(end) = content[json_start..].find("```") {
                let extracted = content[json_start..json_start + end].trim();
                if !extracted.is_empty() {
                    return extracted.to_string();
                }
            }
        }
    }

    // 2. Try object brackets (full distillation response — most common expected format)
    if let Some(start) = content.find('{') {
        if let Some(end) = content.rfind('}') {
            if end > start {
                return content[start..=end].to_string();
            }
        }
    }

    // 3. Try array brackets (facts-only backward compat)
    if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') {
            if end > start {
                return content[start..=end].to_string();
            }
        }
    }

    content.to_string()
}

/// Load the embedded default distillation prompt from `gateway/templates/`.
///
/// Can be overridden by creating `config/distillation_prompt.md` in the vault.
fn default_distillation_prompt() -> String {
    let bytes = gateway_templates::Templates::get("distillation_prompt.md")
        .expect("distillation_prompt.md must be embedded in gateway/templates/")
        .data;
    String::from_utf8_lossy(&bytes).into_owned()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    // resolve_relationship_endpoint — FK survival policy
    //
    // Regression for the "India → Oil" failure: relationships that name
    // an entity only in the relationships block (not in the entities
    // array) used to be persisted with the literal name as the id, which
    // violated the FK on `kg_relationships`. The helper must: prefer an
    // existing entity_map entry, then look up by name in the graph, then
    // auto-create a stub entity so the edge survives.
    // ------------------------------------------------------------------------
    mod resolve_endpoint {
        use super::*;
        use gateway_services::VaultPaths;
        use knowledge_graph::{types::ExtractedKnowledge, Entity, EntityType};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tempfile::tempdir;
        use zbot_stores_sqlite::kg::storage::GraphStorage;
        use zbot_stores_sqlite::KnowledgeDatabase;

        fn fresh_graph() -> GraphStorage {
            let dir = tempdir().unwrap();
            let paths = Arc::new(VaultPaths::new(dir.keep()));
            std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
            let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
            GraphStorage::new(db).unwrap()
        }

        #[test]
        fn returns_entity_map_hit_without_touching_graph() {
            let graph = fresh_graph();
            let mut map = HashMap::new();
            map.insert("India".to_string(), "entity-india-id-42".to_string());

            let id = resolve_relationship_endpoint(&graph, "agent-x", "India", &mut map);
            assert_eq!(id.as_deref(), Some("entity-india-id-42"));
            // No new map key — hit was a read.
            assert_eq!(map.len(), 1);
        }

        #[test]
        fn resolves_via_graph_find_when_not_in_map() {
            let graph = fresh_graph();
            // Seed the graph with an entity from a "prior session".
            let existing = Entity::new(
                "agent-x".to_string(),
                EntityType::Location,
                "India".to_string(),
            );
            let existing_id = existing.id.clone();
            graph
                .store_knowledge(
                    "agent-x",
                    ExtractedKnowledge {
                        entities: vec![existing],
                        relationships: vec![],
                    },
                )
                .unwrap();

            let mut map = HashMap::new();
            let id = resolve_relationship_endpoint(&graph, "agent-x", "India", &mut map);
            assert_eq!(
                id.as_deref(),
                Some(existing_id.as_str()),
                "must reuse the pre-existing entity id"
            );
            // Cached into the map so subsequent endpoints in the same turn
            // don't re-query the DB for the same name.
            assert_eq!(map.get("India"), Some(&existing_id));
        }

        #[test]
        fn auto_creates_stub_when_name_is_unknown() {
            let graph = fresh_graph();
            let mut map = HashMap::new();

            let id = resolve_relationship_endpoint(&graph, "agent-x", "Oil", &mut map);
            let id = id.expect("stub should be persisted and resolved");
            assert!(!id.is_empty());
            // The stub is persisted — find_entity_by_name must now return it.
            let looked_up = graph.find_entity_by_name("agent-x", "Oil").unwrap();
            assert_eq!(looked_up.as_deref(), Some(id.as_str()));
            // Cached in the map too.
            assert_eq!(map.get("Oil"), Some(&id));
        }
    }

    #[test]
    fn test_parse_facts_from_json_array() {
        // Backward compat: plain array of facts
        let json = r#"[{"category": "user", "key": "user.preferred_language", "content": "User prefers Rust", "confidence": 0.9}]"#;
        let resp = parse_distillation_response(json).unwrap();
        assert_eq!(resp.facts.len(), 1);
        assert_eq!(resp.facts[0].key, "user.preferred_language");
        assert_eq!(resp.facts[0].confidence, 0.9);
    }

    #[test]
    fn test_parse_full_response() {
        let json = r#"{"facts": [{"category": "domain", "key": "domain.zbot.db_engine", "content": "Using SQLite", "confidence": 0.85}], "entities": [{"name": "SQLite", "type": "tool", "properties": {"usage": "embedded database"}}], "relationships": [{"source": "AgentZero", "target": "SQLite", "type": "uses"}]}"#;
        let resp = parse_distillation_response(json).unwrap();
        assert_eq!(resp.facts.len(), 1);
        assert_eq!(resp.entities.len(), 1);
        assert_eq!(resp.entities[0].name, "SQLite");
        assert_eq!(resp.relationships.len(), 1);
    }

    #[test]
    fn test_parse_facts_from_markdown() {
        let md = "```json\n[{\"category\": \"domain\", \"key\": \"domain.zbot.db_engine\", \"content\": \"Using SQLite\", \"confidence\": 0.85}]\n```";
        let resp = parse_distillation_response(md).unwrap();
        assert_eq!(resp.facts.len(), 1);
        assert_eq!(resp.facts[0].key, "domain.zbot.db_engine");
    }

    #[test]
    fn test_parse_facts_empty_array() {
        let resp = parse_distillation_response("[]").unwrap();
        assert!(resp.facts.is_empty());
    }

    #[test]
    fn test_parse_facts_unparseable() {
        // Unparseable text should now return Err, not Ok(empty)
        let result = parse_distillation_response("No facts to extract from this session.");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_facts_with_surrounding_text() {
        // JSON embedded in surrounding text should still be extracted
        let text = "Here are the extracted facts:\n{\"facts\": [{\"category\": \"pattern\", \"key\": \"pattern.workflow.test_before_commit\", \"content\": \"Always run tests before committing\", \"confidence\": 0.8}], \"entities\": [], \"relationships\": []}\nDone.";
        let resp = parse_distillation_response(text).unwrap();
        assert_eq!(resp.facts.len(), 1);
        assert_eq!(resp.facts[0].category, "pattern");
    }

    #[test]
    fn test_build_transcript_truncates() {
        let long_content = "x".repeat(2000);
        let messages = vec![zbot_conversation::Message {
            id: "msg-1".to_string(),
            execution_id: Some("exec-1".to_string()),
            session_id: "sess-1".to_string(),
            role: "user".to_string(),
            content: long_content,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            token_count: 500,
            tool_calls: None,
            tool_call_id: None,
            seq: 1,
        }];

        let transcript = build_transcript(&messages);
        assert!(transcript.contains("truncated"));
        assert!(transcript.len() < 3000); // Truncated at 1000 chars + prefix
    }

    #[test]
    fn test_default_confidence() {
        let json = r#"[{"category": "domain", "key": "domain.zbot.project_name", "content": "Project is called AgentZero"}]"#;
        let resp = parse_distillation_response(json).unwrap();
        assert_eq!(resp.facts[0].confidence, 0.8);
    }

    #[test]
    fn test_parse_response_with_episode() {
        let json = r#"{
            "facts": [{"category": "domain", "key": "domain.test", "content": "Test fact", "confidence": 0.9}],
            "entities": [],
            "relationships": [],
            "episode": {
                "task_summary": "User asked to analyze portfolio data",
                "outcome": "success",
                "strategy_used": "delegated to data-analyst for technicals",
                "key_learnings": "CSV parsing worked well with pandas"
            }
        }"#;
        let resp = parse_distillation_response(json).unwrap();
        assert_eq!(resp.facts.len(), 1);
        let ep = resp.episode.unwrap();
        assert_eq!(ep.outcome, "success");
        assert_eq!(ep.task_summary, "User asked to analyze portfolio data");
        assert_eq!(
            ep.strategy_used.as_deref(),
            Some("delegated to data-analyst for technicals")
        );
        assert_eq!(
            ep.key_learnings.as_deref(),
            Some("CSV parsing worked well with pandas")
        );
    }

    #[test]
    fn test_parse_response_without_episode() {
        let json = r#"{"facts": [], "entities": [], "relationships": []}"#;
        let resp = parse_distillation_response(json).unwrap();
        assert!(resp.episode.is_none());
    }

    #[test]
    fn test_parse_response_episode_partial_fields() {
        let json = r#"{
            "facts": [],
            "entities": [],
            "relationships": [],
            "episode": {
                "task_summary": "Quick question about Rust",
                "outcome": "partial"
            }
        }"#;
        let resp = parse_distillation_response(json).unwrap();
        let ep = resp.episode.unwrap();
        assert_eq!(ep.outcome, "partial");
        assert!(ep.strategy_used.is_none());
        assert!(ep.key_learnings.is_none());
    }

    #[test]
    fn test_sanitize_task_type_basic() {
        assert_eq!(
            sanitize_task_type("Analyze portfolio data"),
            "analyze_portfolio_data"
        );
    }

    #[test]
    fn test_sanitize_task_type_long_summary() {
        assert_eq!(
            sanitize_task_type(
                "User asked the agent to analyze their entire stock portfolio and generate a report"
            ),
            "user_asked_the_agent"
        );
    }

    #[test]
    fn test_sanitize_task_type_with_dots() {
        assert_eq!(
            sanitize_task_type("Fix config.toml parsing"),
            "fix_config_toml_parsing"
        );
    }

    #[test]
    fn test_sanitize_task_type_special_chars() {
        assert_eq!(
            sanitize_task_type("Build & deploy (v2)"),
            "build__deploy_v2"
        );
    }

    #[test]
    fn test_sanitize_task_type_empty() {
        assert_eq!(sanitize_task_type(""), "");
    }

    // ========================================================================
    // Relationship canonicalization tests
    // ========================================================================

    fn make_entity(name: &str, entity_type: &str) -> ExtractedEntity {
        ExtractedEntity {
            name: name.to_string(),
            entity_type: entity_type.to_string(),
            properties: std::collections::HashMap::new(),
        }
    }

    fn make_rel(source: &str, rel: &str, target: &str) -> ExtractedRelationship {
        ExtractedRelationship {
            source: source.to_string(),
            target: target.to_string(),
            relationship_type: rel.to_string(),
        }
    }

    #[test]
    fn canonicalize_drops_agent_analyzed_by_ticker() {
        // The LLM sometimes emits `planner-agent analyzed_by PTON` which is
        // nonsense. These should be dropped entirely.
        let entities = vec![
            make_entity("planner-agent", "tool"),
            make_entity("PTON", "organization"),
        ];
        let rel = make_rel("planner-agent", "analyzed_by", "PTON");
        assert!(canonicalize_relationship(&rel, &entities).is_none());
    }

    #[test]
    fn canonicalize_inverts_used_by_to_uses() {
        // `A used_by B` should become `B uses A`.
        let entities = vec![
            make_entity("code-agent", "tool"),
            make_entity("portfolio-analysis", "project"),
        ];
        let rel = make_rel("code-agent", "used_by", "portfolio-analysis");
        let result = canonicalize_relationship(&rel, &entities).unwrap();
        assert_eq!(result.source, "portfolio-analysis");
        assert_eq!(result.target, "code-agent");
        assert_eq!(result.relationship_type, "uses");
    }

    #[test]
    fn canonicalize_inverts_usedfor_to_uses() {
        // `A usedfor B` → `B uses A` (same rule as used_by).
        let entities = vec![
            make_entity("planner-agent", "tool"),
            make_entity("portfolio-analysis", "project"),
        ];
        let rel = make_rel("planner-agent", "usedfor", "portfolio-analysis");
        let result = canonicalize_relationship(&rel, &entities).unwrap();
        assert_eq!(result.source, "portfolio-analysis");
        assert_eq!(result.target, "planner-agent");
        assert_eq!(result.relationship_type, "uses");
    }

    #[test]
    fn canonicalize_inverts_created_by_to_created() {
        // `A created_by B` → `B created A`.
        let entities = vec![
            make_entity("financial_analysis.py", "file"),
            make_entity("code-agent", "tool"),
        ];
        let rel = make_rel("financial_analysis.py", "created_by", "code-agent");
        let result = canonicalize_relationship(&rel, &entities).unwrap();
        assert_eq!(result.source, "code-agent");
        assert_eq!(result.target, "financial_analysis.py");
        assert_eq!(result.relationship_type, "created");
    }

    #[test]
    fn canonicalize_preserves_correct_direction() {
        // `PTON analyzed_by portfolio-analysis` is correct — keep as is.
        let entities = vec![
            make_entity("PTON", "organization"),
            make_entity("portfolio-analysis", "project"),
        ];
        let rel = make_rel("PTON", "analyzed_by", "portfolio-analysis");
        let result = canonicalize_relationship(&rel, &entities).unwrap();
        assert_eq!(result.source, "PTON");
        assert_eq!(result.target, "portfolio-analysis");
        assert_eq!(result.relationship_type, "analyzed_by");
    }

    #[test]
    fn canonicalize_swaps_inverted_analyzed_by() {
        // `portfolio-analysis analyzed_by AAPL` is inverted — swap to
        // `AAPL analyzed_by portfolio-analysis`.
        let entities = vec![
            make_entity("portfolio-analysis", "project"),
            make_entity("AAPL", "organization"),
        ];
        let rel = make_rel("portfolio-analysis", "analyzed_by", "AAPL");
        let result = canonicalize_relationship(&rel, &entities).unwrap();
        assert_eq!(result.source, "AAPL");
        assert_eq!(result.target, "portfolio-analysis");
    }

    #[test]
    fn canonicalize_keeps_generic_relationships() {
        // Relationships without special rules pass through unchanged.
        let entities = vec![
            make_entity("yfinance", "tool"),
            make_entity("pandas", "tool"),
        ];
        let rel = make_rel("yfinance", "uses", "pandas");
        let result = canonicalize_relationship(&rel, &entities).unwrap();
        assert_eq!(result.source, "yfinance");
        assert_eq!(result.target, "pandas");
        assert_eq!(result.relationship_type, "uses");
    }

    #[test]
    fn is_agent_name_detects_agent_suffixes() {
        assert!(is_agent_name("planner-agent"));
        assert!(is_agent_name("code-agent"));
        assert!(is_agent_name("research_agent"));
        assert!(is_agent_name("Planner-Agent")); // case insensitive
        assert!(!is_agent_name("portfolio-analysis"));
        assert!(!is_agent_name("yfinance"));
    }

    // ------------------------------------------------------------------------
    // Task 3: verify procedure upsert path populates the embedding.
    //
    // Regression for the latent gap where `distillation.rs` called
    // `upsert_procedure(v, None)`. With Tasks 1+2 done on the
    // PatternExtractor side, this is the second write path that needed
    // the same fix — procedures from session distillation must arrive
    // at the store with an embedding so the SQLite `procedures_index`
    // vec0 table gets populated.
    // ------------------------------------------------------------------------
    mod procedure_upsert {
        use super::*;

        use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
        use async_trait::async_trait;
        use gateway_services::{ProviderService, VaultPaths};
        use serde_json::Value;
        use std::sync::Arc;
        use std::sync::Mutex;
        use tempfile::tempdir;
        use zbot_stores_traits::ProcedureStore;

        /// `ProcedureStore` that captures every `upsert_procedure` call so
        /// the test can assert on the inserted shape + embedding.
        #[derive(Default)]
        struct CapturingProcedureStore {
            upserts: Mutex<Vec<(Value, Option<Vec<f32>>)>>,
        }

        #[async_trait]
        impl ProcedureStore for CapturingProcedureStore {
            async fn upsert_procedure(
                &self,
                procedure: Value,
                embedding: Option<Vec<f32>>,
            ) -> Result<(), String> {
                self.upserts.lock().unwrap().push((procedure, embedding));
                Ok(())
            }
        }

        /// Embedding client that always returns a fixed-length vector.
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

        fn setup_distiller(
            store: Arc<CapturingProcedureStore>,
            embed: Arc<FixedEmbeddingClient>,
        ) -> SessionDistiller {
            let dir = tempdir().unwrap();
            let paths = Arc::new(VaultPaths::new(dir.keep()));
            std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
            let pool =
                zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap();
            let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
            let session_meta = Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool));
            let provider_service = Arc::new(ProviderService::new(paths.clone()));

            let mut distiller = SessionDistiller::new(
                provider_service,
                Some(embed as Arc<dyn EmbeddingClient>),
                messages,
                session_meta,
                paths,
                None,
            );
            distiller.set_procedure_store(store as Arc<dyn ProcedureStore>);
            distiller
        }

        fn sample_procedure() -> ExtractedProcedure {
            ExtractedProcedure {
                name: "expected_name".to_string(),
                description: "investigate the thing then summarise it".to_string(),
                steps: vec![ProcedureStep {
                    action: "do_something".to_string(),
                    agent: None,
                    task_template: None,
                    note: None,
                }],
                parameters: None,
                trigger_pattern: Some("when X happens".to_string()),
            }
        }

        #[tokio::test]
        async fn distillation_writes_procedure_with_embedding() {
            let store = Arc::new(CapturingProcedureStore::default());
            let embed = Arc::new(FixedEmbeddingClient {
                vec: vec![0.25_f32; 384],
            });
            let distiller = setup_distiller(store.clone(), embed);

            let procedure = sample_procedure();
            distiller
                .process_procedure_upsert("agent-x", Some("ward-y".to_string()), &procedure)
                .await;

            let upserts = store.upserts.lock().unwrap();
            assert_eq!(upserts.len(), 1, "exactly one upsert expected");
            let (value, embedding) = &upserts[0];
            assert_eq!(value["name"].as_str(), Some("expected_name"));
            assert!(
                embedding.is_some(),
                "procedure was upserted without embedding"
            );
            assert_eq!(embedding.as_ref().unwrap().len(), 384);
        }

        #[tokio::test]
        async fn distillation_procedure_upsert_handles_missing_embedding_client() {
            // Same path but no embedding client wired — confirms graceful
            // degradation: the upsert still happens, embedding is None.
            let store = Arc::new(CapturingProcedureStore::default());
            let dir = tempdir().unwrap();
            let paths = Arc::new(VaultPaths::new(dir.keep()));
            std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
            let pool =
                zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap();
            let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
            let session_meta = Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool));
            let provider_service = Arc::new(ProviderService::new(paths.clone()));

            let mut distiller =
                SessionDistiller::new(provider_service, None, messages, session_meta, paths, None);
            distiller.set_procedure_store(store.clone() as Arc<dyn ProcedureStore>);

            let procedure = sample_procedure();
            distiller
                .process_procedure_upsert("agent-x", None, &procedure)
                .await;

            let upserts = store.upserts.lock().unwrap();
            assert_eq!(upserts.len(), 1);
            assert!(upserts[0].1.is_none());
        }
    }
}
