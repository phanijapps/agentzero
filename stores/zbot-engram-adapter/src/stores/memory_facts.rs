//! `MemoryFactStore` implementation backed by Engram memory records.

use agent_primitives::vec_math::cosine_f64;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use agent_runtime::llm::embedding::EmbeddingClient;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use engram_domain::{MemoryId, MemoryStatus};
use engram_memory::MemoryService;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use zbot_stores_traits::{
    EmbeddingQueryIdentity, MemoryFact, MemoryFactStore, MemoryFactWriteRequest, SkillIndexRow,
    StrategyFactInsert, StrategyFactMatch,
};

use crate::{
    bootstrap::EngramProvider,
    config::{AdapterConfig, AdapterEmbeddingProviderConfig, EmbeddingMode, ProviderMode},
    error::{AdapterError, AdapterResult},
    mapping::memory::{memory_fact_to_record_with_governance, memory_record_to_fact},
    scope::ScopeMapper,
};

const SIDECAR_COMPONENT: &str = "memory_fact_sidecar";
const MAX_FACT_CONTENT_CHARS: usize = 800;
const CTX_AGENT_SENTINEL: &str = "__ctx__";
const CTX_SCOPE: &str = "session";
const CTX_CATEGORY: &str = "ctx";
const PRIMITIVE_AGENT_SENTINEL: &str = "__ward__";
const PRIMITIVE_SCOPE: &str = "global";
const PRIMITIVE_CATEGORY: &str = "primitive";
const RRF_K: f64 = 60.0;
const MIN_SEMANTIC_SCORE: f64 = 0.20;
const MAX_RECALL_QUERY_CHARS: usize = 500;
const MAX_SEARCH_LIMIT: usize = 50;

/// Engram-backed implementation of AgentZero's memory fact store trait.
///
/// Engram persists the canonical memory record. The sidecar preserves zbot's
/// existing JSON row shape, embedding bytes, and compatibility query indexes
/// until those query ports are available directly in Engram.
#[derive(Clone)]
pub struct EngramMemoryFactStore {
    memory: Arc<dyn MemoryService>,
    mapper: ScopeMapper,
    sidecar: MemoryFactSidecar,
    governance: crate::governance::GovernancePolicy,
    embedding_mode: EmbeddingMode,
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
}

impl EngramMemoryFactStore {
    /// Open an Engram-backed memory fact store for `ProviderMode::Engram`.
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "memory_facts",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        let provider = EngramProvider::open(config.clone())?;
        Self::from_provider(config, &provider)
    }

    /// Build a memory fact store from an already-bootstrapped Engram provider.
    pub fn from_provider(config: AdapterConfig, provider: &EngramProvider) -> AdapterResult<Self> {
        Self::from_provider_with_embedding_client(config, provider, None)
    }

    /// Build a memory fact store with a live embedding client for query recall.
    pub fn from_provider_with_embedding_client(
        config: AdapterConfig,
        provider: &EngramProvider,
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
    ) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "memory_facts",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        // Skip conformance gate if memory handle is already available
        // (it can pass even when the in-memory conformance check fails)
        let mapper = config.scope_mapper()?;
        let memory = provider.memory()?;
        let embedding_identity = embedding_client
            .as_ref()
            .map(|client| {
                EmbeddingIdentity::from_client(client.as_ref(), &config.embedding_provider)
            })
            .unwrap_or_else(|| EmbeddingIdentity::from(&config.embedding_provider));
        let sidecar = MemoryFactSidecar::open(
            &config.compatibility_store_path("zbot-memory-facts.sqlite")?,
            embedding_identity,
        )?;

        Ok(Self {
            memory,
            mapper,
            sidecar,
            governance: config.governance.clone(),
            embedding_mode: config.embedding_mode,
            embedding_client,
        })
    }

    async fn embed_query(&self, query: &str) -> (Option<Vec<f32>>, Option<&'static str>) {
        let Some(client) = self.embedding_client.as_ref() else {
            tracing::debug!(
                degraded_reason = "embedding_client_unavailable",
                "memory fact recall degraded to exact-only mode"
            );
            return (None, Some("embedding_client_unavailable"));
        };
        let before_identity = EmbeddingIdentity::from_client_with_fallback(
            client.as_ref(),
            &self.sidecar.embedding_identity,
        );
        if before_identity != self.sidecar.embedding_identity {
            tracing::warn!(
                degraded_reason = "embedding_identity_mismatch",
                expected_provider_type = %self.sidecar.embedding_identity.provider_type,
                expected_model = %self.sidecar.embedding_identity.model,
                expected_dimensions = self.sidecar.embedding_identity.dimensions,
                actual_provider_type = %client.provider_type(),
                actual_model = %client.model_name(),
                actual_dimensions = client.dimensions(),
                "memory fact recall degraded to exact-only mode"
            );
            return (None, Some("embedding_identity_mismatch"));
        }
        let mut embeddings = match client.embed(&[query]).await {
            Ok(embeddings) => embeddings,
            Err(error) => {
                tracing::warn!(
                    degraded_reason = "embedding_error",
                    embedding_model = %client.model_name(),
                    embedding_dimensions = client.dimensions(),
                    error = %error,
                    "memory fact recall degraded to exact-only mode"
                );
                return (None, Some("embedding_error"));
            }
        };
        let after_identity = EmbeddingIdentity::from_client_with_fallback(
            client.as_ref(),
            &self.sidecar.embedding_identity,
        );
        if after_identity != before_identity || after_identity != self.sidecar.embedding_identity {
            tracing::warn!(
                degraded_reason = "embedding_identity_changed_during_embed",
                expected_provider_type = %self.sidecar.embedding_identity.provider_type,
                expected_model = %self.sidecar.embedding_identity.model,
                expected_dimensions = self.sidecar.embedding_identity.dimensions,
                actual_provider_type = %after_identity.provider_type,
                actual_model = %after_identity.model,
                actual_dimensions = after_identity.dimensions,
                "memory fact recall degraded to exact-only mode"
            );
            return (None, Some("embedding_identity_changed_during_embed"));
        }
        let embedding = embeddings.pop().filter(|embedding| !embedding.is_empty());
        if embedding.is_none() {
            tracing::warn!(
                degraded_reason = "empty_query_embedding",
                embedding_model = %client.model_name(),
                embedding_dimensions = client.dimensions(),
                "memory fact recall degraded to exact-only mode"
            );
            return (None, Some("empty_query_embedding"));
        }
        if embedding
            .as_ref()
            .is_some_and(|embedding| embedding.len() != client.dimensions())
        {
            tracing::warn!(
                degraded_reason = "query_embedding_dimension_mismatch",
                embedding_model = %client.model_name(),
                expected_dimensions = client.dimensions(),
                actual_dimensions = embedding.as_ref().map(Vec::len).unwrap_or(0),
                "memory fact recall degraded to exact-only mode"
            );
            return (None, Some("query_embedding_dimension_mismatch"));
        }
        (embedding, None)
    }

    fn query_identity(&self) -> EmbeddingQueryIdentity {
        self.sidecar.embedding_identity.to_query_identity()
    }

    async fn upsert_fact_record(
        &self,
        mut fact: MemoryFact,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        if let Some(embedding) = embedding {
            self.ensure_embedding_write_identity(&embedding)?;
            fact.embedding = Some(embedding);
        }
        validate_fact_content(&fact.category, &fact.content)?;

        let record = memory_fact_to_record_with_governance(
            &fact,
            &self.mapper,
            self.embedding_mode,
            Some(&self.governance),
        )
        .map_err(AdapterError::into_trait_error)?;
        self.memory
            .put_memory(record)
            .await
            .map_err(|error| error.to_string())?;

        let embedding = fact.embedding.clone();
        let mut sidecar_fact = fact;
        sidecar_fact.embedding = None;
        self.sidecar
            .store_fact(&sidecar_fact, embedding.as_deref(), false)
    }

    fn ensure_embedding_write_identity(&self, embedding: &[f32]) -> Result<(), String> {
        let Some(client) = self.embedding_client.as_ref() else {
            return Ok(());
        };
        if embedding.len() as u32 != self.sidecar.embedding_identity.dimensions {
            return Err(
                "embedding dimension mismatch - reindex required before storing vectors"
                    .to_string(),
            );
        }
        if self
            .sidecar
            .embedding_identity
            .compatible_client(client.as_ref())
        {
            Ok(())
        } else {
            Err("embedding_identity_mismatch - reindex required before storing vectors".to_string())
        }
    }

    async fn update_status(&self, fact: &MemoryFact, status: MemoryStatus) -> Result<(), String> {
        let scope = self
            .mapper
            .memory_fact_scope(&fact.ward_id, fact.session_id.as_deref())
            .map_err(AdapterError::into_trait_error)?;
        self.memory
            .update_memory_status(&MemoryId::from(fact.id.as_str()), &scope, status)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn read_canonical_or_sidecar(&self, entry: SidecarEntry) -> Result<MemoryFact, String> {
        let scope = self
            .mapper
            .memory_fact_scope(&entry.fact.ward_id, entry.fact.session_id.as_deref())
            .map_err(AdapterError::into_trait_error)?;
        let maybe_record = self
            .memory
            .get_memory(&MemoryId::from(entry.fact.id.as_str()), &scope)
            .await
            .map_err(|error| error.to_string())?;

        match maybe_record {
            Some(record) => memory_record_to_fact(&record)
                .map_err(AdapterError::into_trait_error)
                .or(Ok(entry.fact)),
            None => Ok(entry.fact),
        }
    }
}

#[async_trait]
impl MemoryFactStore for EngramMemoryFactStore {
    async fn save_fact(
        &self,
        agent_id: &str,
        category: &str,
        key: &str,
        content: &str,
        confidence: f64,
        session_id: Option<&str>,
        valid_from: Option<DateTime<Utc>>,
    ) -> Result<Value, String> {
        self.save_fact_with_context(MemoryFactWriteRequest {
            agent_id: agent_id.to_string(),
            category: category.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            confidence,
            session_id: session_id.map(ToOwned::to_owned),
            ward_id: None,
            source_ref: None,
            valid_from,
        })
        .await
    }

    async fn save_fact_with_context(
        &self,
        request: MemoryFactWriteRequest,
    ) -> Result<Value, String> {
        validate_fact_content(&request.category, &request.content)?;

        let scope = default_scope_for_category(&request.category).to_string();
        let ward_id = request
            .ward_id
            .as_deref()
            .map(str::trim)
            .filter(|ward_id| !ward_id.is_empty())
            .unwrap_or("__global__");
        let now = Utc::now();
        let existing = self.sidecar.find_active_by_key_with_session(
            &request.agent_id,
            &scope,
            ward_id,
            &request.key,
            Some(&request.category),
            request.session_id.as_deref(),
        )?;

        let fact = if let Some(entry) = existing {
            let mut fact = entry.fact;
            fact.content = request.content.clone();
            fact.confidence = request.confidence;
            fact.mention_count = fact.mention_count.saturating_add(1);
            fact.session_id = request.session_id.clone();
            fact.ward_id = ward_id.to_string();
            fact.source_ref = request.source_ref.clone();
            fact.updated_at = now.to_rfc3339();
            fact.valid_from = Some(request.valid_from.unwrap_or(now).to_rfc3339());
            fact
        } else {
            MemoryFact {
                id: format!("fact-{}", Uuid::new_v4()),
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
                scope,
                category: request.category.clone(),
                key: request.key.clone(),
                content: request.content.clone(),
                confidence: request.confidence,
                mention_count: 1,
                source_summary: None,
                embedding: None,
                ward_id: ward_id.to_string(),
                contradicted_by: None,
                created_at: now.to_rfc3339(),
                updated_at: now.to_rfc3339(),
                expires_at: None,
                valid_from: Some(request.valid_from.unwrap_or(now).to_rfc3339()),
                valid_until: None,
                superseded_by: None,
                pinned: false,
                epistemic_class: Some("current".to_string()),
                source_episode_id: None,
                source_ref: request.source_ref.clone(),
            }
        };

        self.upsert_fact_record(fact, None).await?;

        Ok(json!({
            "success": true,
            "action": "save_fact",
            "key": request.key,
            "category": request.category,
            "confidence": request.confidence,
            "message": format!("Fact saved: [{}] {}", request.category, request.content),
        }))
    }

    async fn recall_facts(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Value, String> {
        let query = bounded_recall_query(query);
        let (query_embedding, degraded_reason) = self.embed_query(&query).await;
        let query_identity = query_embedding.as_ref().map(|_| self.query_identity());
        let rows = self
            .search_memory_facts_hybrid_with_identity(
                Some(agent_id),
                &query,
                "hybrid",
                limit,
                None,
                query_embedding.as_deref(),
                query_identity.as_ref(),
                None,
            )
            .await?;
        let rows = tag_degraded_rows(rows, degraded_reason);
        Ok(recall_value(&query, rows, degraded_reason))
    }

    async fn recall_facts_prioritized(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Value, String> {
        let query = bounded_recall_query(query);
        let (query_embedding, degraded_reason) = self.embed_query(&query).await;
        let query_identity = query_embedding.as_ref().map(|_| self.query_identity());
        let rows = self
            .search_memory_facts_hybrid_with_identity(
                Some(agent_id),
                &query,
                "hybrid",
                limit,
                None,
                query_embedding.as_deref(),
                query_identity.as_ref(),
                as_of,
            )
            .await?;
        let rows = tag_degraded_rows(rows, degraded_reason);
        Ok(recall_value(&query, rows, degraded_reason))
    }

    async fn recall_facts_prioritized_scoped(
        &self,
        agent_id: &str,
        query: &str,
        ward_id: Option<&str>,
        limit: usize,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Value, String> {
        let query = bounded_recall_query(query);
        let (query_embedding, degraded_reason) = self.embed_query(&query).await;
        let query_identity = query_embedding.as_ref().map(|_| self.query_identity());
        let rows = self
            .search_memory_facts_hybrid_with_identity(
                Some(agent_id),
                &query,
                "hybrid",
                limit,
                ward_id,
                query_embedding.as_deref(),
                query_identity.as_ref(),
                as_of,
            )
            .await?;
        let rows = tag_degraded_rows(rows, degraded_reason);
        Ok(recall_value(&query, rows, degraded_reason))
    }

    async fn save_ctx_fact(
        &self,
        session_id: &str,
        ward_id: &str,
        key: &str,
        content: &str,
        owner: &str,
        pinned: bool,
    ) -> Result<Value, String> {
        validate_fact_content(CTX_CATEGORY, content)?;

        let now = Utc::now().to_rfc3339();
        let mut fact = self
            .sidecar
            .find_active_by_key(
                CTX_AGENT_SENTINEL,
                CTX_SCOPE,
                ward_id,
                key,
                Some(CTX_CATEGORY),
            )?
            .map(|entry| entry.fact)
            .unwrap_or_else(|| MemoryFact {
                id: format!("fact-{}", Uuid::new_v4()),
                session_id: Some(session_id.to_string()),
                agent_id: CTX_AGENT_SENTINEL.to_string(),
                scope: CTX_SCOPE.to_string(),
                category: CTX_CATEGORY.to_string(),
                key: key.to_string(),
                content: String::new(),
                confidence: 1.0,
                mention_count: 0,
                source_summary: None,
                embedding: None,
                ward_id: ward_id.to_string(),
                contradicted_by: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                expires_at: None,
                valid_from: None,
                valid_until: None,
                superseded_by: None,
                pinned,
                epistemic_class: Some("current".to_string()),
                source_episode_id: None,
                source_ref: None,
            });

        fact.session_id = Some(session_id.to_string());
        fact.content = content.to_string();
        fact.source_summary = Some(format!("owner={owner}"));
        fact.pinned = pinned;
        fact.updated_at = now;
        fact.mention_count = fact.mention_count.saturating_add(1);

        self.upsert_fact_record(fact, None).await?;

        Ok(json!({
            "success": true,
            "action": "save_ctx_fact",
            "key": key,
            "owner": owner,
            "session_id": session_id,
        }))
    }

    async fn get_ctx_fact(&self, ward_id: &str, key: &str) -> Result<Option<Value>, String> {
        Ok(self
            .sidecar
            .find_active_by_key(
                CTX_AGENT_SENTINEL,
                CTX_SCOPE,
                ward_id,
                key,
                Some(CTX_CATEGORY),
            )?
            .map(|entry| {
                let fact = entry.fact;
                let owner = fact
                    .source_summary
                    .as_deref()
                    .and_then(|summary| summary.strip_prefix("owner="))
                    .unwrap_or("unknown")
                    .to_string();
                json!({
                    "found": true,
                    "key": fact.key,
                    "content": fact.content,
                    "owner": owner,
                    "session_id": fact.session_id,
                    "created_at": fact.created_at,
                    "updated_at": fact.updated_at,
                    "pinned": fact.pinned,
                })
            }))
    }

    async fn upsert_primitive(
        &self,
        ward_id: &str,
        key: &str,
        signature: &str,
        summary: &str,
    ) -> Result<Value, String> {
        let now = Utc::now().to_rfc3339();
        let content = primitive_content(signature, summary);
        let mut fact = self
            .sidecar
            .find_active_by_key(
                PRIMITIVE_AGENT_SENTINEL,
                PRIMITIVE_SCOPE,
                ward_id,
                key,
                Some(PRIMITIVE_CATEGORY),
            )?
            .map(|entry| entry.fact)
            .unwrap_or_else(|| MemoryFact {
                id: format!("fact-{}", Uuid::new_v4()),
                session_id: None,
                agent_id: PRIMITIVE_AGENT_SENTINEL.to_string(),
                scope: PRIMITIVE_SCOPE.to_string(),
                category: PRIMITIVE_CATEGORY.to_string(),
                key: key.to_string(),
                content: String::new(),
                confidence: 1.0,
                mention_count: 0,
                source_summary: None,
                embedding: None,
                ward_id: ward_id.to_string(),
                contradicted_by: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                expires_at: None,
                valid_from: None,
                valid_until: None,
                superseded_by: None,
                pinned: false,
                epistemic_class: Some("current".to_string()),
                source_episode_id: None,
                source_ref: None,
            });

        fact.content = content;
        fact.updated_at = now;
        fact.mention_count = fact.mention_count.saturating_add(1);
        self.upsert_fact_record(fact, None).await?;

        Ok(json!({ "success": true, "key": key, "ward_id": ward_id }))
    }

    async fn list_primitives(&self, ward_id: &str) -> Result<Value, String> {
        let primitives = self
            .sidecar
            .list_primitives_for_ward(ward_id)?
            .into_iter()
            .map(|entry| {
                let (signature, summary) = split_primitive_content(&entry.fact.content);
                json!({
                    "key": entry.fact.key,
                    "signature": signature,
                    "summary": summary,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "primitives": primitives }))
    }

    async fn list_primitives_for_ward(&self, ward_id: &str) -> Result<Vec<MemoryFact>, String> {
        Ok(self
            .sidecar
            .list_primitives_for_ward(ward_id)?
            .into_iter()
            .map(|entry| entry.fact)
            .collect())
    }

    async fn list_recent_state_handoffs(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        Ok(self
            .sidecar
            .list_recent_state_handoffs(session_id, limit)?
            .into_iter()
            .map(|entry| entry.fact)
            .collect())
    }

    async fn delete_facts_by_key(&self, category: &str, key: &str) -> Result<usize, String> {
        let entries = self.sidecar.list_by_category_key(category, key)?;
        let mut deleted = 0;
        for entry in entries {
            self.update_status(&entry.fact, MemoryStatus::Forgotten)
                .await?;
            if self.sidecar.delete(&entry.fact.id)? {
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    async fn list_skill_index(&self) -> Result<Vec<SkillIndexRow>, String> {
        self.sidecar.list_skill_index()
    }

    async fn upsert_skill_index(&self, row: SkillIndexRow) -> Result<(), String> {
        self.sidecar.upsert_skill_index(&row)
    }

    async fn delete_skill_index(&self, name: &str) -> Result<bool, String> {
        self.sidecar.delete_skill_index(name)
    }

    async fn count_all_facts(&self, agent_id: Option<&str>) -> Result<i64, String> {
        self.sidecar.count_active(agent_id)
    }

    async fn aggregate_stats(&self) -> Result<zbot_stores_traits::MemoryAggregateStats, String> {
        Ok(zbot_stores_traits::MemoryAggregateStats {
            facts: self.count_all_facts(None).await?,
            ..Default::default()
        })
    }

    async fn list_memory_facts(
        &self,
        agent_id: Option<&str>,
        category: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>, String> {
        self.sidecar
            .list_active(agent_id, category, scope, limit, offset)?
            .into_iter()
            .map(|entry| serde_json::to_value(entry.fact).map_err(|error| error.to_string()))
            .collect()
    }

    async fn list_memory_facts_typed(
        &self,
        agent_id: Option<&str>,
        category: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        Ok(self
            .sidecar
            .list_active(agent_id, category, scope, limit, offset)?
            .into_iter()
            .map(|entry| entry.fact)
            .collect())
    }

    async fn get_memory_fact_by_id(&self, fact_id: &str) -> Result<Option<Value>, String> {
        let Some(entry) = self.sidecar.get(fact_id)? else {
            return Ok(None);
        };
        let fact = self.read_canonical_or_sidecar(entry).await?;
        serde_json::to_value(fact)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    async fn delete_memory_fact(&self, fact_id: &str) -> Result<bool, String> {
        let Some(entry) = self.sidecar.get(fact_id)? else {
            return Ok(false);
        };
        self.update_status(&entry.fact, MemoryStatus::Forgotten)
            .await?;
        self.sidecar.delete(fact_id)
    }

    async fn upsert_typed_fact(
        &self,
        mut fact: MemoryFact,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        // The `embedding` parameter is the sole vector channel — the struct
        // field is `#[serde(skip)]` and was always dropped by the old Value
        // round-trip, so keep honoring only the explicit argument here.
        fact.embedding = embedding.clone();
        self.upsert_fact_record(fact, embedding).await
    }

    async fn supersede_fact(
        &self,
        old_id: &str,
        new_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String> {
        let Some(mut entry) = self.sidecar.get(old_id)? else {
            return Err(format!("memory fact not found: {old_id}"));
        };
        entry.fact.valid_until = Some(transition_time.to_rfc3339());
        entry.fact.superseded_by = Some(new_id.to_string());
        entry.fact.updated_at = Utc::now().to_rfc3339();

        let embedding = entry.embedding.clone();
        self.upsert_fact_record(entry.fact.clone(), embedding)
            .await?;
        self.update_status(&entry.fact, MemoryStatus::Archived)
            .await
    }

    async fn archive_fact(&self, fact_id: &str) -> Result<bool, String> {
        let Some(entry) = self.sidecar.get(fact_id)? else {
            return Ok(false);
        };
        self.update_status(&entry.fact, MemoryStatus::Archived)
            .await?;
        self.sidecar.archive(fact_id)
    }

    async fn search_memory_facts_hybrid(
        &self,
        agent_id: Option<&str>,
        query: &str,
        mode: &str,
        limit: usize,
        ward_id: Option<&str>,
        query_embedding: Option<&[f32]>,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Vec<Value>, String> {
        self.search_memory_facts_hybrid_with_identity(
            agent_id,
            query,
            mode,
            limit,
            ward_id,
            query_embedding,
            None,
            as_of,
        )
        .await
    }

    async fn search_memory_facts_hybrid_with_identity(
        &self,
        agent_id: Option<&str>,
        query: &str,
        mode: &str,
        limit: usize,
        ward_id: Option<&str>,
        query_embedding: Option<&[f32]>,
        query_identity: Option<&EmbeddingQueryIdentity>,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Vec<Value>, String> {
        self.sidecar
            .search(SearchRequest {
                agent_id,
                query,
                mode,
                limit,
                ward_id,
                query_embedding,
                query_identity,
                as_of,
            })?
            .into_iter()
            .map(|hit| {
                let mut value =
                    serde_json::to_value(hit.fact).map_err(|error| error.to_string())?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("score".to_string(), json!(hit.score));
                    object.insert("match_source".to_string(), Value::String(hit.match_source));
                    if let Some(reason) = hit.degraded_reason {
                        object.insert("degraded".to_string(), json!(true));
                        object.insert("degraded_reason".to_string(), json!(reason));
                    }
                }
                Ok(value)
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
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Vec<(MemoryFact, f64, String)>, String> {
        self.search_memory_facts_hybrid_typed_with_identity(
            agent_id,
            query,
            mode,
            limit,
            ward_id,
            query_embedding,
            None,
            as_of,
        )
        .await
    }

    async fn search_memory_facts_hybrid_typed_with_identity(
        &self,
        agent_id: Option<&str>,
        query: &str,
        mode: &str,
        limit: usize,
        ward_id: Option<&str>,
        query_embedding: Option<&[f32]>,
        query_identity: Option<&EmbeddingQueryIdentity>,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Vec<(MemoryFact, f64, String)>, String> {
        Ok(self
            .sidecar
            .search(SearchRequest {
                agent_id,
                query,
                mode,
                limit,
                ward_id,
                query_embedding,
                query_identity,
                as_of,
            })?
            .into_iter()
            .map(|hit| (hit.fact, hit.score, hit.match_source))
            .collect())
    }

    async fn find_strategy_fact_by_similarity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f32,
        scan_limit: usize,
    ) -> Result<Option<StrategyFactMatch>, String> {
        let _ = (agent_id, embedding, threshold, scan_limit);
        Ok(None)
    }

    async fn find_strategy_fact_by_similarity_with_identity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        threshold: f32,
        scan_limit: usize,
    ) -> Result<Option<StrategyFactMatch>, String> {
        if !self
            .sidecar
            .embedding_identity
            .compatible_query(embedding, query_identity)
        {
            return Ok(None);
        }
        for entry in
            self.sidecar
                .list_active(Some(agent_id), Some("strategy"), None, scan_limit, 0)?
        {
            let Some(stored) = entry.embedding.as_deref() else {
                continue;
            };
            if !embedding_compatible(&entry, &self.sidecar.embedding_identity, embedding) {
                continue;
            }
            if cosine_f64(embedding, stored) >= f64::from(threshold) {
                return Ok(Some(StrategyFactMatch {
                    fact_id: entry.fact.id,
                    source_episode_id: entry.fact.source_episode_id,
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
        let Some(mut entry) = self.sidecar.get(fact_id)? else {
            return Err(format!("memory fact not found: {fact_id}"));
        };
        entry.fact.mention_count = entry.fact.mention_count.saturating_add(1);
        entry.fact.source_episode_id = Some(merged_source_episode_id.to_string());
        entry.fact.updated_at = now_rfc3339.to_string();
        self.upsert_fact_record(entry.fact, entry.embedding).await
    }

    async fn insert_strategy_fact(&self, req: StrategyFactInsert) -> Result<String, String> {
        let id = format!("fact-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
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
        let embedding = fact.embedding.clone();
        self.upsert_fact_record(fact, embedding).await?;
        Ok(id)
    }

    async fn get_facts_by_category(
        &self,
        agent_id: &str,
        category: &str,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        Ok(self
            .sidecar
            .list_active(Some(agent_id), Some(category), None, limit, 0)?
            .into_iter()
            .map(|entry| entry.fact)
            .collect())
    }

    async fn get_high_confidence_facts(
        &self,
        agent_id: Option<&str>,
        threshold: f64,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        Ok(self
            .sidecar
            .load_all()?
            .into_iter()
            .filter(|entry| {
                !entry.archived
                    && agent_visible(agent_id, &entry.fact)
                    && entry.fact.confidence >= threshold
                    && fact_valid_at(&entry.fact, Utc::now())
            })
            .take(limit)
            .map(|entry| entry.fact)
            .collect())
    }

    async fn get_fact_by_key(
        &self,
        agent_id: &str,
        scope: &str,
        ward_id: &str,
        key: &str,
    ) -> Result<Option<MemoryFact>, String> {
        Ok(self
            .sidecar
            .find_active_by_key(agent_id, scope, ward_id, key, None)?
            .map(|entry| entry.fact))
    }

    async fn get_fact_embedding(&self, fact_id: &str) -> Result<Option<Vec<f32>>, String> {
        Ok(self.sidecar.get(fact_id)?.and_then(|entry| entry.embedding))
    }

    async fn get_cached_embedding(
        &self,
        content_hash: &str,
        model_name: &str,
    ) -> Result<Option<Vec<f32>>, String> {
        self.sidecar.get_cached_embedding(content_hash, model_name)
    }

    async fn cache_embedding(
        &self,
        content_hash: &str,
        model_name: &str,
        embedding: &[f32],
    ) -> Result<(), String> {
        self.sidecar
            .cache_embedding(content_hash, model_name, embedding)
    }

    async fn get_memory_facts(
        &self,
        agent_id: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryFact>, String> {
        Ok(self
            .sidecar
            .list_active(Some(agent_id), None, scope, limit, 0)?
            .into_iter()
            .map(|entry| entry.fact)
            .collect())
    }

    async fn list_contradicted_fact_episode_ids(
        &self,
        agent_id: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<String>, String> {
        self.sidecar
            .list_contradicted_fact_episode_ids(agent_id, since)
    }
}

#[derive(Clone)]
struct MemoryFactSidecar {
    connection: Arc<Mutex<Connection>>,
    embedding_identity: EmbeddingIdentity,
}

impl MemoryFactSidecar {
    fn open(path: &Path, embedding_identity: EmbeddingIdentity) -> AdapterResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| AdapterError::Storage {
                component: SIDECAR_COMPONENT,
                reason: "sidecar parent cannot be created".to_string(),
            })?;
        }

        let connection = Connection::open(path).map_err(|_| AdapterError::Bootstrap {
            component: SIDECAR_COMPONENT,
            reason: "sidecar construction failed".to_string(),
        })?;
        connection
            .execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = WAL;
                PRAGMA busy_timeout = 5000;

                CREATE TABLE IF NOT EXISTS memory_facts (
                    id TEXT PRIMARY KEY,
                    record_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    ward_id TEXT NOT NULL,
                    session_id TEXT,
                    category TEXT NOT NULL,
                    key TEXT NOT NULL,
                    content TEXT NOT NULL,
                    confidence REAL NOT NULL,
                    mention_count INTEGER NOT NULL,
                    valid_from TEXT,
                    valid_until TEXT,
                    updated_at TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0,
                    fact_json TEXT NOT NULL,
                    embedding_json TEXT,
                    embedding_identity_json TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_memory_facts_agent
                    ON memory_facts(agent_id, archived);
                CREATE INDEX IF NOT EXISTS idx_memory_facts_list
                    ON memory_facts(agent_id, category, scope, archived, updated_at);
                CREATE INDEX IF NOT EXISTS idx_memory_facts_key
                    ON memory_facts(agent_id, scope, ward_id, key, category, archived);
                CREATE INDEX IF NOT EXISTS idx_memory_facts_ward
                    ON memory_facts(ward_id, archived);

                CREATE TABLE IF NOT EXISTS skill_index_state (
                    name TEXT PRIMARY KEY,
                    source_root TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    mtime_unix INTEGER NOT NULL,
                    size_bytes INTEGER NOT NULL,
                    last_indexed_unix INTEGER NOT NULL,
                    format_version INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS embedding_cache (
                    content_hash TEXT NOT NULL,
                    model_name TEXT NOT NULL,
                    embedding_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY(content_hash, model_name)
                );
                "#,
            )
            .map_err(|_| AdapterError::Bootstrap {
                component: SIDECAR_COMPONENT,
                reason: "sidecar schema initialization failed".to_string(),
            })?;
        ensure_optional_column(
            &connection,
            "memory_facts",
            "embedding_identity_json",
            "TEXT",
        )
        .map_err(|_| AdapterError::Bootstrap {
            component: SIDECAR_COMPONENT,
            reason: "sidecar embedding identity migration failed".to_string(),
        })?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            embedding_identity,
        })
    }

    fn store_fact(
        &self,
        fact: &MemoryFact,
        embedding: Option<&[f32]>,
        archived: bool,
    ) -> Result<(), String> {
        let fact_json = serde_json::to_string(fact).map_err(|error| error.to_string())?;
        let embedding_json = embedding
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let embedding_identity_json = embedding
            .map(|_| serde_json::to_string(&self.embedding_identity))
            .transpose()
            .map_err(|error| error.to_string())?;
        let archived = i64::from(archived);

        self.connection()?
            .execute(
                r#"
            INSERT INTO memory_facts
                (id, record_id, agent_id, scope, ward_id, session_id, category, key, content,
                 confidence, mention_count, valid_from, valid_until, updated_at, archived,
                 fact_json, embedding_json, embedding_identity_json)
            VALUES
                (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                record_id = excluded.record_id,
                agent_id = excluded.agent_id,
                scope = excluded.scope,
                ward_id = excluded.ward_id,
                session_id = excluded.session_id,
                category = excluded.category,
                key = excluded.key,
                content = excluded.content,
                confidence = excluded.confidence,
                mention_count = excluded.mention_count,
                valid_from = excluded.valid_from,
                valid_until = excluded.valid_until,
                updated_at = excluded.updated_at,
                archived = excluded.archived,
                fact_json = excluded.fact_json,
                embedding_json = excluded.embedding_json,
                embedding_identity_json = excluded.embedding_identity_json
            "#,
                params![
                    fact.id.as_str(),
                    fact.agent_id.as_str(),
                    fact.scope.as_str(),
                    fact.ward_id.as_str(),
                    fact.session_id.as_deref(),
                    fact.category.as_str(),
                    fact.key.as_str(),
                    fact.content.as_str(),
                    fact.confidence,
                    fact.mention_count,
                    fact.valid_from.as_deref(),
                    fact.valid_until.as_deref(),
                    fact.updated_at.as_str(),
                    archived,
                    fact_json,
                    embedding_json,
                    embedding_identity_json,
                ],
            )
            .map_err(|error| storage_error(error).to_string())?;
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<SidecarEntry>, String> {
        let row = self
            .connection()?
            .query_row(
                "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts WHERE id = ?1",
                params![id],
                sidecar_entry_from_row,
            )
            .optional()
            .map_err(|error| storage_error(error).to_string())?;
        row.transpose()
    }

    fn delete(&self, id: &str) -> Result<bool, String> {
        let affected = self
            .connection()?
            .execute("DELETE FROM memory_facts WHERE id = ?1", params![id])
            .map_err(|error| storage_error(error).to_string())?;
        Ok(affected > 0)
    }

    fn archive(&self, id: &str) -> Result<bool, String> {
        let affected = self
            .connection()?
            .execute(
                "UPDATE memory_facts SET archived = 1 WHERE id = ?1",
                params![id],
            )
            .map_err(|error| storage_error(error).to_string())?;
        Ok(affected > 0)
    }

    fn count_active(&self, agent_id: Option<&str>) -> Result<i64, String> {
        match agent_id {
            Some(agent_id) => self
                .connection()?
                .query_row(
                    "SELECT COUNT(*) FROM memory_facts WHERE archived = 0 AND agent_id = ?1",
                    params![agent_id],
                    |row| row.get(0),
                )
                .map_err(|error| storage_error(error).to_string()),
            None => self
                .connection()?
                .query_row(
                    "SELECT COUNT(*) FROM memory_facts WHERE archived = 0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| storage_error(error).to_string()),
        }
    }

    fn list_active(
        &self,
        agent_id: Option<&str>,
        category: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SidecarEntry>, String> {
        let mut where_clauses = vec!["archived = 0".to_string()];
        let mut values = Vec::new();
        push_optional_clause(&mut where_clauses, &mut values, "agent_id", agent_id);
        push_optional_clause(&mut where_clauses, &mut values, "category", category);
        push_optional_clause(&mut where_clauses, &mut values, "scope", scope);
        values.push(SqlValue::Integer(limit as i64));
        values.push(SqlValue::Integer(offset as i64));

        let sql = format!(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE {} ORDER BY updated_at DESC, id ASC LIMIT ?{} OFFSET ?{}",
            where_clauses.join(" AND "),
            values.len() - 1,
            values.len()
        );
        self.query_entries(&sql, values)
    }

    fn find_active_by_key(
        &self,
        agent_id: &str,
        scope: &str,
        ward_id: &str,
        key: &str,
        category: Option<&str>,
    ) -> Result<Option<SidecarEntry>, String> {
        let mut where_clauses = vec![
            "archived = 0".to_string(),
            "agent_id = ?1".to_string(),
            "scope = ?2".to_string(),
            "ward_id = ?3".to_string(),
            "key = ?4".to_string(),
        ];
        let mut values = vec![
            SqlValue::Text(agent_id.to_string()),
            SqlValue::Text(scope.to_string()),
            SqlValue::Text(ward_id.to_string()),
            SqlValue::Text(key.to_string()),
        ];
        push_optional_clause(&mut where_clauses, &mut values, "category", category);
        let sql = format!(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE {} ORDER BY updated_at DESC LIMIT 1",
            where_clauses.join(" AND ")
        );
        Ok(self.query_entries(&sql, values)?.into_iter().next())
    }

    /// Exact active-fact lookup that treats an execution session as part of
    /// the write identity. Model-originated writes are session-scoped in
    /// Engram, so allowing the same key in another session to overwrite this
    /// row would silently cross that boundary.
    fn find_active_by_key_with_session(
        &self,
        agent_id: &str,
        scope: &str,
        ward_id: &str,
        key: &str,
        category: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Option<SidecarEntry>, String> {
        let mut where_clauses = vec![
            "archived = 0".to_string(),
            "agent_id = ?1".to_string(),
            "scope = ?2".to_string(),
            "ward_id = ?3".to_string(),
            "key = ?4".to_string(),
        ];
        let mut values = vec![
            SqlValue::Text(agent_id.to_string()),
            SqlValue::Text(scope.to_string()),
            SqlValue::Text(ward_id.to_string()),
            SqlValue::Text(key.to_string()),
        ];
        match session_id {
            Some(session_id) => {
                values.push(SqlValue::Text(session_id.to_string()));
                where_clauses.push(format!("session_id = ?{}", values.len()));
            }
            None => where_clauses.push("session_id IS NULL".to_string()),
        }
        push_optional_clause(&mut where_clauses, &mut values, "category", category);
        let sql = format!(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE {} ORDER BY updated_at DESC LIMIT 1",
            where_clauses.join(" AND ")
        );
        Ok(self.query_entries(&sql, values)?.into_iter().next())
    }

    fn list_by_category_key(&self, category: &str, key: &str) -> Result<Vec<SidecarEntry>, String> {
        self.query_entries(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE archived = 0 AND category = ?1 AND key = ?2 ORDER BY updated_at DESC, id ASC",
            vec![
                SqlValue::Text(category.to_string()),
                SqlValue::Text(key.to_string()),
            ],
        )
    }

    fn list_primitives_for_ward(&self, ward_id: &str) -> Result<Vec<SidecarEntry>, String> {
        self.query_entries(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE archived = 0
               AND agent_id = ?1
               AND scope = ?2
               AND ward_id = ?3
               AND category = ?4
             ORDER BY key ASC",
            vec![
                SqlValue::Text(PRIMITIVE_AGENT_SENTINEL.to_string()),
                SqlValue::Text(PRIMITIVE_SCOPE.to_string()),
                SqlValue::Text(ward_id.to_string()),
                SqlValue::Text(PRIMITIVE_CATEGORY.to_string()),
            ],
        )
    }

    fn list_recent_state_handoffs(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SidecarEntry>, String> {
        let pattern = format!("ctx.{session_id}.state.%");
        self.query_entries(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE archived = 0
               AND agent_id = ?1
               AND scope = ?2
               AND category = ?3
               AND session_id = ?4
               AND key LIKE ?5
             ORDER BY updated_at DESC, id ASC
             LIMIT ?6",
            vec![
                SqlValue::Text(CTX_AGENT_SENTINEL.to_string()),
                SqlValue::Text(CTX_SCOPE.to_string()),
                SqlValue::Text(CTX_CATEGORY.to_string()),
                SqlValue::Text(session_id.to_string()),
                SqlValue::Text(pattern),
                SqlValue::Integer(limit as i64),
            ],
        )
    }

    fn list_skill_index(&self) -> Result<Vec<SkillIndexRow>, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT name, source_root, file_path, mtime_unix, size_bytes,
                        last_indexed_unix, format_version
                 FROM skill_index_state
                 ORDER BY name ASC",
            )
            .map_err(|error| storage_error(error).to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(SkillIndexRow {
                    name: row.get(0)?,
                    source_root: row.get(1)?,
                    file_path: row.get(2)?,
                    mtime_unix: row.get(3)?,
                    size_bytes: row.get(4)?,
                    last_indexed_unix: row.get(5)?,
                    format_version: row.get(6)?,
                })
            })
            .map_err(|error| storage_error(error).to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error(error).to_string())
    }

    fn upsert_skill_index(&self, row: &SkillIndexRow) -> Result<(), String> {
        self.connection()?
            .execute(
                r#"
                INSERT INTO skill_index_state
                    (name, source_root, file_path, mtime_unix, size_bytes,
                     last_indexed_unix, format_version)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(name) DO UPDATE SET
                    source_root = excluded.source_root,
                    file_path = excluded.file_path,
                    mtime_unix = excluded.mtime_unix,
                    size_bytes = excluded.size_bytes,
                    last_indexed_unix = excluded.last_indexed_unix,
                    format_version = excluded.format_version
                "#,
                params![
                    row.name.as_str(),
                    row.source_root.as_str(),
                    row.file_path.as_str(),
                    row.mtime_unix,
                    row.size_bytes,
                    row.last_indexed_unix,
                    row.format_version,
                ],
            )
            .map_err(|error| storage_error(error).to_string())?;
        Ok(())
    }

    fn delete_skill_index(&self, name: &str) -> Result<bool, String> {
        let affected = self
            .connection()?
            .execute(
                "DELETE FROM skill_index_state WHERE name = ?1",
                params![name],
            )
            .map_err(|error| storage_error(error).to_string())?;
        Ok(affected > 0)
    }

    fn get_cached_embedding(
        &self,
        content_hash: &str,
        model_name: &str,
    ) -> Result<Option<Vec<f32>>, String> {
        let json = self
            .connection()?
            .query_row(
                "SELECT embedding_json FROM embedding_cache
                 WHERE content_hash = ?1 AND model_name = ?2",
                params![content_hash, model_name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| storage_error(error).to_string())?;
        json.map(|json| serde_json::from_str::<Vec<f32>>(&json))
            .transpose()
            .map_err(|error| format!("decode cached embedding: {error}"))
    }

    fn cache_embedding(
        &self,
        content_hash: &str,
        model_name: &str,
        embedding: &[f32],
    ) -> Result<(), String> {
        let embedding_json = serde_json::to_string(embedding).map_err(|error| error.to_string())?;
        self.connection()?
            .execute(
                r#"
                INSERT INTO embedding_cache
                    (content_hash, model_name, embedding_json, created_at)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(content_hash, model_name) DO UPDATE SET
                    embedding_json = excluded.embedding_json,
                    created_at = excluded.created_at
                "#,
                params![
                    content_hash,
                    model_name,
                    embedding_json,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|error| storage_error(error).to_string())?;
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<SidecarEntry>, String> {
        self.query_entries(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts ORDER BY updated_at DESC",
            Vec::new(),
        )
    }

    fn search(&self, request: SearchRequest<'_>) -> Result<Vec<SearchHit>, String> {
        let cutoff = request.as_of.unwrap_or_else(Utc::now);
        let query = bounded_recall_query(request.query);
        let tokens = search_tokens(&query);
        let limit = request.limit.clamp(1, MAX_SEARCH_LIMIT);
        let entries = self
            .search_candidates(request.agent_id, request.ward_id)?
            .into_iter()
            .filter(|entry| !entry.archived && fact_valid_at(&entry.fact, cutoff))
            .collect::<Vec<_>>();

        let mut hits = if request.mode == "hybrid" {
            match request.query_embedding {
                Some(query_embedding)
                    if self
                        .embedding_identity
                        .compatible_query(query_embedding, request.query_identity) =>
                {
                    rank_hybrid_entries(entries, &tokens, query_embedding, &self.embedding_identity)
                }
                Some(query_embedding) => {
                    let reason = self
                        .embedding_identity
                        .query_mismatch_reason(query_embedding, request.query_identity)
                        .unwrap_or("query_embedding_identity_mismatch");
                    self.log_exact_degraded(reason, Some(query_embedding.len()));
                    entries
                        .into_iter()
                        .filter_map(|entry| exact_degraded_hit(entry, &tokens, reason))
                        .collect()
                }
                None => {
                    self.log_exact_degraded("query_embedding_unavailable", None);
                    entries
                        .into_iter()
                        .filter_map(|entry| {
                            exact_degraded_hit(entry, &tokens, "query_embedding_unavailable")
                        })
                        .collect()
                }
            }
        } else {
            entries
                .into_iter()
                .filter_map(|entry| {
                    score_entry(
                        entry,
                        &tokens,
                        request.mode,
                        request.query_embedding,
                        request.query_identity,
                        &self.embedding_identity,
                    )
                })
                .collect::<Vec<_>>()
        };
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    fact_scope_rank(&right.fact, request.agent_id, request.ward_id).cmp(
                        &fact_scope_rank(&left.fact, request.agent_id, request.ward_id),
                    )
                })
                .then_with(|| right.fact.updated_at.cmp(&left.fact.updated_at))
                .then_with(|| left.fact.id.cmp(&right.fact.id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    fn search_candidates(
        &self,
        agent_id: Option<&str>,
        ward_id: Option<&str>,
    ) -> Result<Vec<SidecarEntry>, String> {
        let mut where_clauses = vec!["archived = 0".to_string(), "category != 'ctx'".to_string()];
        let mut values = Vec::new();

        if let Some(agent_id) = agent_id {
            values.push(SqlValue::Text(agent_id.to_string()));
            where_clauses.push(format!(
                "(agent_id = ?{} OR scope = 'global')",
                values.len()
            ));
        }

        if let Some(ward_id) = ward_id {
            values.push(SqlValue::Text(ward_id.to_string()));
            where_clauses.push(format!(
                "(ward_id = ?{} OR ward_id = '__global__')",
                values.len()
            ));
        }
        let sql = format!(
            "SELECT fact_json, embedding_json, archived, embedding_identity_json FROM memory_facts \
             WHERE {} ORDER BY updated_at DESC, id ASC",
            where_clauses.join(" AND ")
        );
        self.query_entries(&sql, values)
    }

    fn log_exact_degraded(&self, reason: &'static str, actual_dimensions: Option<usize>) {
        tracing::debug!(
            degraded_reason = reason,
            provider_type = %self.embedding_identity.provider_type,
            model = %self.embedding_identity.model,
            expected_dimensions = self.embedding_identity.dimensions,
            actual_dimensions = actual_dimensions,
            prompt_profile = %self.embedding_identity.prompt_profile,
            normalization = ?self.embedding_identity.normalization,
            "memory fact hybrid recall degraded to exact-only mode"
        );
    }

    fn list_contradicted_fact_episode_ids(
        &self,
        agent_id: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<String>, String> {
        let since = since.to_rfc3339();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT json_extract(fact_json, '$.source_episode_id')
                 FROM memory_facts
                 WHERE archived = 0
                   AND agent_id = ?1
                   AND json_extract(fact_json, '$.contradicted_by') IS NOT NULL
                   AND json_extract(fact_json, '$.source_episode_id') IS NOT NULL
                   AND updated_at > ?2
                 ORDER BY json_extract(fact_json, '$.source_episode_id') ASC",
            )
            .map_err(|error| storage_error(error).to_string())?;
        let rows = statement
            .query_map(params![agent_id, since], |row| row.get::<_, String>(0))
            .map_err(|error| storage_error(error).to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error(error).to_string())
    }

    fn query_entries(&self, sql: &str, values: Vec<SqlValue>) -> Result<Vec<SidecarEntry>, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(sql)
            .map_err(|error| storage_error(error).to_string())?;
        let mut rows = statement
            .query(params_from_iter(values))
            .map_err(|error| storage_error(error).to_string())?;

        let mut entries = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| storage_error(error).to_string())?
        {
            let entry =
                sidecar_entry_from_row(row).map_err(|error| storage_error(error).to_string())??;
            entries.push(entry);
        }
        Ok(entries)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "adapter storage `memory_fact_sidecar` failed: lock poisoned".to_string())
    }
}

fn ensure_optional_column(
    connection: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
        [],
    )?;
    Ok(())
}

#[derive(Clone)]
struct SidecarEntry {
    fact: MemoryFact,
    embedding: Option<Vec<f32>>,
    embedding_identity: Option<EmbeddingIdentity>,
    archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddingIdentity {
    provider_type: String,
    model: String,
    dimensions: u32,
    prompt_profile: String,
    normalization: Option<String>,
}

impl EmbeddingIdentity {
    fn compatible_query(
        &self,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
    ) -> bool {
        self.query_mismatch_reason(query_embedding, query_identity)
            .is_none()
    }

    fn query_mismatch_reason(
        &self,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
    ) -> Option<&'static str> {
        if self.dimensions as usize != query_embedding.len() {
            return Some("query_embedding_dimension_mismatch");
        }
        let Some(query_identity) = query_identity else {
            return Some("query_embedding_identity_missing");
        };
        if self.provider_type != query_identity.provider_type
            || self.model != query_identity.model
            || self.dimensions != query_identity.dimensions
            || self.prompt_profile != query_identity.prompt_profile
            || self.normalization != query_identity.normalization
        {
            return Some("query_embedding_identity_mismatch");
        }
        None
    }

    fn compatible_client(&self, client: &dyn EmbeddingClient) -> bool {
        self.provider_type == client.provider_type()
            && self.model == client.model_name()
            && self.dimensions as usize == client.dimensions()
            && self.prompt_profile == client.prompt_profile()
            && self.normalization == client.normalization()
    }

    fn from_client(
        client: &dyn EmbeddingClient,
        fallback: &AdapterEmbeddingProviderConfig,
    ) -> Self {
        Self {
            provider_type: client.provider_type(),
            model: client.model_name(),
            dimensions: client.dimensions() as u32,
            prompt_profile: client.prompt_profile(),
            normalization: client
                .normalization()
                .or_else(|| fallback.normalization.clone()),
        }
    }

    fn from_client_with_fallback(client: &dyn EmbeddingClient, fallback: &Self) -> Self {
        Self {
            provider_type: client.provider_type(),
            model: client.model_name(),
            dimensions: client.dimensions() as u32,
            prompt_profile: client.prompt_profile(),
            normalization: client
                .normalization()
                .or_else(|| fallback.normalization.clone()),
        }
    }

    fn to_query_identity(&self) -> EmbeddingQueryIdentity {
        EmbeddingQueryIdentity {
            provider_type: self.provider_type.clone(),
            model: self.model.clone(),
            dimensions: self.dimensions,
            prompt_profile: self.prompt_profile.clone(),
            normalization: self.normalization.clone(),
        }
    }
}

impl From<&AdapterEmbeddingProviderConfig> for EmbeddingIdentity {
    fn from(config: &AdapterEmbeddingProviderConfig) -> Self {
        Self {
            provider_type: config.provider_type.clone(),
            model: config.model.clone(),
            dimensions: config.dimensions,
            prompt_profile: config.prompt_profile.clone(),
            normalization: config.normalization.clone(),
        }
    }
}

struct SearchHit {
    fact: MemoryFact,
    score: f64,
    match_source: String,
    degraded_reason: Option<&'static str>,
}

fn fact_scope_rank(fact: &MemoryFact, agent_id: Option<&str>, ward_id: Option<&str>) -> u8 {
    let mut rank = 0u8;
    if agent_id.is_some_and(|agent_id| fact.agent_id == agent_id) {
        rank += 2;
    }
    if ward_id.is_some_and(|ward_id| fact.ward_id == ward_id) {
        rank += 1;
    }
    if fact.scope == "global" {
        rank = rank.saturating_sub(1);
    }
    rank
}

#[derive(Clone, Copy)]
struct SparseMatch {
    score: f64,
    high_specificity: bool,
}

#[derive(Default)]
struct HybridSignal {
    fact: Option<MemoryFact>,
    score: f64,
    semantic: bool,
    sparse: bool,
    high_specificity: bool,
    semantic_score: f64,
}

struct SearchRequest<'a> {
    agent_id: Option<&'a str>,
    query: &'a str,
    mode: &'a str,
    limit: usize,
    ward_id: Option<&'a str>,
    query_embedding: Option<&'a [f32]>,
    query_identity: Option<&'a EmbeddingQueryIdentity>,
    as_of: Option<DateTime<Utc>>,
}

fn sidecar_entry_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<SidecarEntry, String>> {
    let fact_json: String = row.get(0)?;
    let embedding_json: Option<String> = row.get(1)?;
    let archived: i64 = row.get(2)?;
    let embedding_identity_json: Option<String> = row.get(3)?;
    Ok(parse_sidecar_entry(
        fact_json,
        embedding_json,
        archived,
        embedding_identity_json,
    ))
}

fn parse_sidecar_entry(
    fact_json: String,
    embedding_json: Option<String>,
    archived: i64,
    embedding_identity_json: Option<String>,
) -> Result<SidecarEntry, String> {
    let mut fact: MemoryFact =
        serde_json::from_str(&fact_json).map_err(|error| format!("decode MemoryFact: {error}"))?;
    fact.embedding = None;
    let embedding = embedding_json
        .map(|json| serde_json::from_str::<Vec<f32>>(&json))
        .transpose()
        .map_err(|error| format!("decode fact embedding: {error}"))?;
    let embedding_identity = embedding_identity_json
        .map(|json| serde_json::from_str::<EmbeddingIdentity>(&json))
        .transpose()
        .map_err(|error| format!("decode embedding identity: {error}"))?;
    Ok(SidecarEntry {
        fact,
        embedding,
        embedding_identity,
        archived: archived != 0,
    })
}

fn bounded_recall_query(query: &str) -> String {
    let compact = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= MAX_RECALL_QUERY_CHARS {
        return compact;
    }
    compact.chars().take(MAX_RECALL_QUERY_CHARS).collect()
}

fn recall_value(query: &str, rows: Vec<Value>, degraded_reason: Option<&'static str>) -> Value {
    let count = rows.len();
    let recalled = rows.clone();
    let mut value = json!({
        "query": query,
        "results": rows,
        "recalled": recalled,
        "count": count,
        "degraded": degraded_reason.is_some(),
        "source": "memory_db",
    });
    if let (Some(reason), Some(object)) = (degraded_reason, value.as_object_mut()) {
        object.insert("reason".to_string(), json!(reason));
        object.insert("degraded_reason".to_string(), json!(reason));
    }
    value
}

fn tag_degraded_rows(rows: Vec<Value>, degraded_reason: Option<&'static str>) -> Vec<Value> {
    let Some(reason) = degraded_reason else {
        return rows;
    };
    rows.into_iter()
        .map(|mut row| {
            if let Some(object) = row.as_object_mut() {
                let match_source = object
                    .get("match_source")
                    .and_then(Value::as_str)
                    .unwrap_or("exact_degraded")
                    .to_string();
                object.insert("degraded".to_string(), json!(true));
                object.insert("degraded_reason".to_string(), json!(reason));
                object.insert("match_source".to_string(), json!(match_source));
            }
            row
        })
        .collect()
}

fn push_optional_clause(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        values.push(SqlValue::Text(value.to_string()));
        clauses.push(format!("{column} = ?{}", values.len()));
    }
}

fn score_entry(
    entry: SidecarEntry,
    tokens: &[String],
    mode: &str,
    query_embedding: Option<&[f32]>,
    query_identity: Option<&EmbeddingQueryIdentity>,
    expected_identity: &EmbeddingIdentity,
) -> Option<SearchHit> {
    match mode {
        "fts" => {
            let score = sparse_match(&entry.fact, tokens, true)?.score;
            Some(SearchHit {
                fact: entry.fact,
                score,
                match_source: "fts".to_string(),
                degraded_reason: None,
            })
        }
        "semantic" => {
            let query_embedding = query_embedding.filter(|embedding| {
                expected_identity.compatible_query(embedding, query_identity)
            })?;
            if !embedding_compatible(&entry, expected_identity, query_embedding) {
                return None;
            }
            let score = semantic_score(Some(query_embedding), entry.embedding.as_deref());
            (score > 0.0).then(|| SearchHit {
                fact: entry.fact,
                score,
                match_source: "vec".to_string(),
                degraded_reason: None,
            })
        }
        _ => match query_embedding {
            Some(query_embedding)
                if expected_identity.compatible_query(query_embedding, query_identity) =>
            {
                rank_hybrid_entries(vec![entry], tokens, query_embedding, expected_identity)
                    .into_iter()
                    .next()
            }
            None => exact_degraded_hit(entry, tokens, "query_embedding_unavailable"),
            Some(_) => exact_degraded_hit(entry, tokens, "query_embedding_dimension_mismatch"),
        },
    }
}

fn rank_hybrid_entries(
    entries: Vec<SidecarEntry>,
    tokens: &[String],
    query_embedding: &[f32],
    expected_identity: &EmbeddingIdentity,
) -> Vec<SearchHit> {
    let mut semantic_ranked = Vec::<(String, f64, MemoryFact)>::new();
    let mut sparse_ranked = Vec::<(String, SparseMatch, MemoryFact)>::new();

    for entry in entries {
        let fact = entry.fact.clone();
        let id = fact.id.clone();
        if embedding_compatible(&entry, expected_identity, query_embedding) {
            let semantic = semantic_score(Some(query_embedding), entry.embedding.as_deref());
            if semantic > 0.0 {
                semantic_ranked.push((id.clone(), semantic, fact.clone()));
            }
        }
        if let Some(sparse) = sparse_match(&fact, tokens, false) {
            sparse_ranked.push((id, sparse, fact));
        }
    }

    semantic_ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sparse_ranked.sort_by(|left, right| {
        right
            .1
            .score
            .partial_cmp(&left.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut fused = HashMap::<String, HybridSignal>::new();
    for (rank_zero, (id, score, fact)) in semantic_ranked.into_iter().enumerate() {
        let rank = (rank_zero as f64) + 1.0;
        let signal = fused.entry(id).or_default();
        signal.fact.get_or_insert(fact);
        signal.score += 1.0 / (RRF_K + rank);
        signal.semantic = true;
        signal.semantic_score = signal.semantic_score.max(score);
    }
    for (rank_zero, (id, sparse, fact)) in sparse_ranked.into_iter().enumerate() {
        let rank = (rank_zero as f64) + 1.0;
        let signal = fused.entry(id).or_default();
        signal.fact.get_or_insert(fact);
        signal.score += 1.0 / (RRF_K + rank);
        signal.sparse = true;
        signal.high_specificity |= sparse.high_specificity;
    }

    let mut hits = fused
        .into_values()
        .filter(|signal| signal.semantic_score >= MIN_SEMANTIC_SCORE || signal.high_specificity)
        .filter_map(|signal| {
            let fact = signal.fact?;
            Some(SearchHit {
                fact,
                score: normalize_rrf_score(signal.score),
                match_source: match (signal.semantic, signal.sparse) {
                    (true, true) => "hybrid",
                    (true, false) => "vec",
                    (false, true) => "fts",
                    (false, false) => return None,
                }
                .to_string(),
                degraded_reason: None,
            })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits
}

fn normalize_rrf_score(score: f64) -> f64 {
    // RRF scores are small (roughly 1 / (k + rank)). Multiplying by `k`
    // then clamping made every hit present in both sparse and semantic lists
    // exactly 1.0, destroying the ranking before unified recall could use it.
    // This monotonic transform keeps the adapter's [0, 1) score contract
    // without collapsing distinct fused scores.
    let scaled = (score * RRF_K).max(0.0);
    scaled / (1.0 + scaled)
}

fn embedding_compatible(
    entry: &SidecarEntry,
    expected_identity: &EmbeddingIdentity,
    query_embedding: &[f32],
) -> bool {
    entry.embedding_identity.as_ref() == Some(expected_identity)
        && entry
            .embedding
            .as_ref()
            .is_some_and(|embedding| embedding.len() == query_embedding.len())
}

fn exact_degraded_hit(
    entry: SidecarEntry,
    tokens: &[String],
    reason: &'static str,
) -> Option<SearchHit> {
    let sparse = exact_identifier_or_key_match(&entry.fact, tokens)?;
    Some(SearchHit {
        fact: entry.fact,
        score: sparse.score,
        match_source: "exact_degraded".to_string(),
        degraded_reason: Some(reason),
    })
}

fn search_tokens(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
        .map(|token| token.trim_matches(['.', '_', '-']).to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn sparse_match(
    fact: &MemoryFact,
    tokens: &[String],
    include_category: bool,
) -> Option<SparseMatch> {
    let haystack = searchable_text(fact, include_category);
    let mut score = 0.0;
    let mut high_specificity = false;

    for token in tokens {
        if token.len() < 3 || is_generic_token(token) {
            continue;
        }
        if haystack.contains(token.as_str()) {
            let weight = sparse_token_weight(token);
            score += weight;
            high_specificity |= is_high_specificity_token(fact, token);
        }
    }

    (score > 0.0).then_some(SparseMatch {
        score,
        high_specificity,
    })
}

fn exact_identifier_or_key_match(fact: &MemoryFact, tokens: &[String]) -> Option<SparseMatch> {
    let haystack = searchable_text(fact, false);
    let mut score = 0.0;
    let mut high_specificity = false;

    for token in tokens {
        if token.len() < 3 || is_generic_token(token) {
            continue;
        }
        if (is_identifier_token(token) || fact.key.eq_ignore_ascii_case(token))
            && haystack.contains(token.as_str())
        {
            score += sparse_token_weight(token);
            high_specificity = true;
        }
    }

    high_specificity.then_some(SparseMatch {
        score: score.max(1.0),
        high_specificity,
    })
}

fn searchable_text(fact: &MemoryFact, include_category: bool) -> String {
    if include_category {
        format!(
            "{} {} {} {}",
            fact.key,
            fact.category,
            fact.content,
            fact.source_summary.as_deref().unwrap_or_default()
        )
        .to_lowercase()
    } else {
        format!(
            "{} {} {}",
            fact.key,
            fact.content,
            fact.source_summary.as_deref().unwrap_or_default()
        )
        .to_lowercase()
    }
}

fn sparse_token_weight(token: &str) -> f64 {
    if is_identifier_token(token) {
        4.0
    } else if token.len() >= 10 {
        2.0
    } else {
        1.0
    }
}

fn is_high_specificity_token(fact: &MemoryFact, token: &str) -> bool {
    is_identifier_token(token) || key_segments(&fact.key).any(|segment| segment == token)
}

fn is_identifier_token(token: &str) -> bool {
    token.chars().any(|ch| ch.is_ascii_digit())
        || token.contains('.')
        || token.contains('_')
        || token.contains('-')
}

fn key_segments(key: &str) -> impl Iterator<Item = String> + '_ {
    key.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
        .flat_map(|part| part.split(['.', '_', '-']))
        .map(str::to_lowercase)
        .filter(|segment| !segment.is_empty() && !is_generic_token(segment))
}

fn is_generic_token(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "the"
            | "or"
            | "of"
            | "to"
            | "in"
            | "for"
            | "with"
            | "about"
            | "domain"
            | "research"
            | "analysis"
            | "methodology"
            | "review"
            | "academic"
            | "paper"
            | "critical"
            | "evaluation"
            | "task"
            | "knowledge"
            | "memory"
            | "context"
    )
}

fn semantic_score(query_embedding: Option<&[f32]>, fact_embedding: Option<&[f32]>) -> f64 {
    match (query_embedding, fact_embedding) {
        (Some(query), Some(fact)) => cosine_f64(query, fact),
        _ => 0.0,
    }
}

fn agent_visible(agent_id: Option<&str>, fact: &MemoryFact) -> bool {
    agent_id.is_none_or(|agent_id| fact.agent_id == agent_id || fact.scope == "global")
}

fn fact_valid_at(fact: &MemoryFact, cutoff: DateTime<Utc>) -> bool {
    if let Some(valid_from) = fact.valid_from.as_deref().and_then(parse_rfc3339) {
        if valid_from > cutoff {
            return false;
        }
    }
    if let Some(valid_until) = fact.valid_until.as_deref().and_then(parse_rfc3339) {
        if valid_until <= cutoff {
            return false;
        }
    }
    true
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .ok()
}

fn default_scope_for_category(category: &str) -> &'static str {
    match category {
        "correction" | "strategy" | "instruction" | "pattern" => "agent",
        _ => "global",
    }
}

fn primitive_content(signature: &str, summary: &str) -> String {
    if summary.is_empty() {
        signature.to_string()
    } else {
        format!("{signature}\n{summary}")
    }
}

fn split_primitive_content(content: &str) -> (String, String) {
    match content.split_once('\n') {
        Some((signature, summary)) => (signature.to_string(), summary.to_string()),
        None => (content.to_string(), String::new()),
    }
}

fn validate_fact_content(category: &str, content: &str) -> Result<(), String> {
    if matches!(category, "ctx" | "primitive") {
        return Ok(());
    }
    let len = content.chars().count();
    if len > MAX_FACT_CONTENT_CHARS {
        return Err(format!(
            "fact content too long: {len} chars (max {MAX_FACT_CONTENT_CHARS})"
        ));
    }
    Ok(())
}

fn storage_error(error: rusqlite::Error) -> AdapterError {
    AdapterError::Storage {
        component: SIDECAR_COMPONENT,
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AdapterErrorKind;

    #[test]
    fn hybrid_rrf_normalization_preserves_distinct_scores() {
        let top = normalize_rrf_score((1.0 / 61.0) + (1.0 / 61.0));
        let next = normalize_rrf_score((1.0 / 62.0) + (1.0 / 62.0));

        assert!(
            top > next,
            "hybrid normalization must preserve distinct RRF relevance"
        );
        assert!(top <= 1.0 && next >= 0.0);
    }

    #[test]
    fn provider_mode_error_keeps_stable_kind() {
        let err = match EngramMemoryFactStore::open(AdapterConfig::default()) {
            Ok(_) => panic!("wrong mode should not open a memory fact store"),
            Err(error) => error,
        };
        assert_eq!(err.kind(), AdapterErrorKind::UnsupportedFeature);
    }
}
