//! Adapter-owned sidecar stores for AgentZero-specific records.
//!
//! These records are product/runtime state rather than generic Engram memory
//! framework concepts. Keeping them here preserves the existing store-trait
//! contracts without pushing zbot-only schema into Engram.

use agent_primitives::vec_math::cosine_f64;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use engram_domain::{
    Actor, ActorKind, AllowedUse, DeleteMode, MemoryContent, MemoryContentFormat, MemoryId,
    MemoryKind, MemoryRecord, MemoryStatus, Policy, Provenance, Retention, Visibility,
};
use engram_memory::MemoryService;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;
use zbot_stores_traits::{
    CompactionRunSummary, CompactionStore, DistillationStore, EmbeddingQueryIdentity, EpisodeStats,
    EpisodeStore, GoalStore, KgEpisodeStatusCounts, KgEpisodeStore, OutboxStore,
    PatternProcedureInsert, Procedure, ProcedureStats, ProcedureStore, ProcedureSummary,
    RecallLogStore, SessionEpisode, SuccessfulEpisode,
};

use crate::{
    bootstrap::EngramProvider,
    capabilities::AdapterFeature,
    config::{AdapterConfig, ProviderMode},
    error::{AdapterError, AdapterResult},
    governance::{select_and_persist_governance_metadata, GovernancePolicy, GovernanceScope},
    scope::ScopeMapper,
};

const SIDECAR_COMPONENT: &str = "zbot_sidecars";

/// Sidecar-backed implementation of zbot-only store traits.
#[derive(Clone)]
pub struct EngramSidecarStores {
    connection: Arc<Mutex<Connection>>,
    embedding_identity: EmbeddingQueryIdentity,
    /// Canonical semantic mirror. The SQLite sidecar remains only for zbot's
    /// product-specific query and lifecycle DTOs.
    memory: Arc<dyn MemoryService>,
    mapper: ScopeMapper,
    governance: GovernancePolicy,
}

impl EngramSidecarStores {
    /// Open sidecar storage under the configured confined Engram directory.
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "sidecars",
                reason: "provider mode is not engram".to_string(),
            });
        }
        let provider = EngramProvider::open(config.clone())?;
        Self::from_provider(config, &provider)
    }

    /// Build sidecars from the shared provider used by the composition root.
    /// This avoids creating a second provider while still giving semantic
    /// sidecar producers the canonical Engram memory service.
    pub fn from_provider(config: AdapterConfig, provider: &EngramProvider) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "sidecars",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        provider.require_feature(AdapterFeature::MemoryFacts)?;
        let memory = provider.memory()?;
        let mapper = config.scope_mapper()?;
        Self::open_path(
            &config.compatibility_store_path("zbot-sidecars.sqlite")?,
            embedding_identity_from_config(&config),
            memory,
            mapper,
            config.governance.clone(),
        )
    }

    fn open_path(
        path: &Path,
        embedding_identity: EmbeddingQueryIdentity,
        memory: Arc<dyn MemoryService>,
        mapper: ScopeMapper,
        governance: GovernancePolicy,
    ) -> AdapterResult<Self> {
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

                CREATE TABLE IF NOT EXISTS procedures (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    ward_id TEXT,
                    name TEXT NOT NULL,
                    success_count INTEGER NOT NULL,
                    failure_count INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    record_json TEXT NOT NULL,
                    embedding_json TEXT,
                    embedding_identity_json TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_procedures_ward
                    ON procedures(ward_id, updated_at);
                CREATE INDEX IF NOT EXISTS idx_sidecar_procedures_agent_name
                    ON procedures(agent_id, name, success_count, created_at);

                CREATE TABLE IF NOT EXISTS episodes (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    ward_id TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    task_summary TEXT NOT NULL,
                    key_learnings TEXT,
                    created_at TEXT NOT NULL,
                    record_json TEXT NOT NULL,
                    embedding_json TEXT,
                    embedding_identity_json TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_episodes_ward
                    ON episodes(ward_id, created_at);
                CREATE INDEX IF NOT EXISTS idx_sidecar_episodes_agent
                    ON episodes(agent_id, created_at);
                CREATE INDEX IF NOT EXISTS idx_sidecar_episodes_session
                    ON episodes(session_id);

                CREATE TABLE IF NOT EXISTS kg_episodes (
                    id TEXT PRIMARY KEY,
                    source_type TEXT NOT NULL,
                    source_ref TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    session_id TEXT,
                    agent_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    retry_count INTEGER NOT NULL,
                    error TEXT,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    completed_at TEXT,
                    payload TEXT,
                    UNIQUE(source_type, content_hash)
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_kg_status
                    ON kg_episodes(status, created_at);
                CREATE INDEX IF NOT EXISTS idx_sidecar_kg_source
                    ON kg_episodes(source_ref, status);

                CREATE TABLE IF NOT EXISTS goals (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    state TEXT NOT NULL,
                    record_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_goals_agent_state
                    ON goals(agent_id, state);

                CREATE TABLE IF NOT EXISTS recall_log (
                    session_id TEXT NOT NULL,
                    fact_key TEXT NOT NULL,
                    recalled_at TEXT NOT NULL,
                    PRIMARY KEY(session_id, fact_key)
                );

                CREATE TABLE IF NOT EXISTS distillation_runs (
                    session_id TEXT PRIMARY KEY,
                    status TEXT NOT NULL,
                    retry_count INTEGER NOT NULL,
                    record_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_distillation_status
                    ON distillation_runs(status);

                CREATE TABLE IF NOT EXISTS compaction_audit (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_compaction_run
                    ON compaction_audit(run_id, created_at);

                CREATE TABLE IF NOT EXISTS outbox (
                    id TEXT PRIMARY KEY,
                    adapter_id TEXT NOT NULL,
                    capability TEXT NOT NULL,
                    status TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    session_id TEXT,
                    thread_id TEXT,
                    agent_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_sidecar_outbox_adapter_status
                    ON outbox(adapter_id, status);
                "#,
            )
            .map_err(|error| AdapterError::Bootstrap {
                component: SIDECAR_COMPONENT,
                reason: error.to_string(),
            })?;
        ensure_optional_column(&connection, "procedures", "embedding_identity_json", "TEXT")
            .map_err(|error| AdapterError::Bootstrap {
                component: SIDECAR_COMPONENT,
                reason: error.to_string(),
            })?;
        ensure_optional_column(&connection, "episodes", "embedding_identity_json", "TEXT")
            .map_err(|error| AdapterError::Bootstrap {
                component: SIDECAR_COMPONENT,
                reason: error.to_string(),
            })?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            embedding_identity,
            memory,
            mapper,
            governance,
        })
    }

    async fn mirror_procedure(&self, procedure: &Procedure) -> Result<(), String> {
        let summary = procedure
            .trigger_pattern
            .as_ref()
            .map(|trigger| format!("trigger: {trigger}"));
        let content = if procedure.description.trim().is_empty() {
            procedure.name.clone()
        } else {
            procedure.description.clone()
        };
        let record = self.governed_semantic_memory_record(
            &format!("zbot-sidecar:procedure:{}", procedure.id),
            MemoryKind::Procedure,
            content,
            summary,
            procedure.ward_id.as_deref().unwrap_or("__global__"),
            None,
            &procedure.agent_id,
            &format!("procedure:{}", procedure.id),
            &procedure.created_at,
            Some(&procedure.updated_at),
            "procedure",
        )?;
        self.memory
            .put_memory(record)
            .await
            .map(|_| ())
            .map_err(|_| "canonical procedure mirror failed".to_string())
    }

    async fn mirror_episode(&self, episode: &SessionEpisode) -> Result<(), String> {
        let record = self.governed_semantic_memory_record(
            &format!("zbot-sidecar:episode:{}", episode.id),
            MemoryKind::Episode,
            episode.task_summary.clone(),
            episode.key_learnings.clone(),
            &episode.ward_id,
            Some(&episode.session_id),
            &episode.agent_id,
            &format!("episode:{}", episode.id),
            &episode.created_at,
            None,
            "episode",
        )?;
        self.memory
            .put_memory(record)
            .await
            .map(|_| ())
            .map_err(|_| "canonical episode mirror failed".to_string())
    }

    async fn mirror_evidence_payload(&self, payload: &str) -> Result<(), String> {
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            // Normal ingestion chunks are arbitrary text. Only the structured
            // evidence-intake envelope is a durable semantic producer here.
            return Ok(());
        };
        let Some(evidence_id) = json_string(&value, "evidence_id") else {
            return Ok(());
        };
        let Some(agent_id) = json_string(&value, "agent_id") else {
            return Ok(());
        };
        let Some(source_id) = json_string(&value, "source_id") else {
            return Ok(());
        };
        let source_type =
            json_string(&value, "source_type").unwrap_or_else(|| "evidence_intake".to_string());
        let session_id = json_string(&value, "session_id");
        let ward_id = json_string(&value, "ward_id").unwrap_or_else(|| "__global__".to_string());
        let created_at = Utc::now().to_rfc3339();
        let record = self.governed_semantic_memory_record(
            &format!("zbot-sidecar:evidence:{evidence_id}"),
            MemoryKind::Artifact,
            format!("Evidence intake from {source_type}"),
            Some(format!("source: {source_id}")),
            &ward_id,
            session_id.as_deref(),
            &agent_id,
            &source_id,
            &created_at,
            None,
            "evidence",
        )?;
        self.memory
            .put_memory(record)
            .await
            .map(|_| ())
            .map_err(|_| "canonical evidence mirror failed".to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn governed_semantic_memory_record(
        &self,
        id: &str,
        kind: MemoryKind,
        content: String,
        summary: Option<String>,
        ward_id: &str,
        session_id: Option<&str>,
        agent_id: &str,
        source_id: &str,
        created_at: &str,
        updated_at: Option<&str>,
        record_kind: &str,
    ) -> Result<MemoryRecord, String> {
        let created_at = parse_sidecar_timestamp(created_at)?;
        let updated_at = updated_at.map(parse_sidecar_timestamp).transpose()?;
        let scope = self
            .mapper
            .memory_fact_scope(ward_id, session_id)
            .map_err(AdapterError::into_trait_error)?;
        let mut metadata = BTreeMap::new();
        let selection = select_and_persist_governance_metadata(
            &self.governance,
            GovernanceScope {
                ward_id: Some(ward_id),
                session_id,
                source_id: Some(source_id),
                ..GovernanceScope::default()
            },
            &mut metadata,
        );
        let concept_ids = selection
            .taxonomy_scheme_ids
            .iter()
            // These records are canonical Engram memory records. `memory` is
            // the stable starter-SKOS classification shared by facts and
            // sidecar mirrors; the more specific record kind remains explicit
            // alongside it without inventing a concept in user taxonomies.
            .map(|scheme_id| format!("{scheme_id}:concept:memory"))
            .collect::<Vec<_>>();
        if !concept_ids.is_empty() {
            metadata.insert(
                "governanceTaxonomyConceptIds".to_string(),
                json!(concept_ids),
            );
        }
        metadata.insert("governanceRecordKind".to_string(), json!(record_kind));
        metadata.insert("zbotSidecarSourceId".to_string(), json!(source_id));

        Ok(MemoryRecord {
            id: MemoryId::from(id),
            kind,
            content: MemoryContent {
                text: content,
                summary,
                entities: Vec::new(),
                language: None,
                format: Some(MemoryContentFormat::Text),
                structured: None,
                hash: None,
            },
            scope,
            provenance: Provenance {
                source: "agentzero.sidecar_adapter".to_string(),
                actor: Actor {
                    id: agent_id.into(),
                    kind: ActorKind::Agent,
                    display_name: None,
                    metadata: None,
                },
                observed_at: created_at,
                evidence: Vec::new(),
                derivations: Vec::new(),
                confidence: Some(1.0),
                method: Some("agentzero.sidecar_semantic_mirror".to_string()),
            },
            policy: Policy {
                visibility: Visibility::Workspace,
                retention: Retention::Durable,
                sensitivity: None,
                allowed_uses: vec![AllowedUse::Retrieval, AllowedUse::Consolidation],
                expires_at: None,
                delete_mode: Some(DeleteMode::Archive),
            },
            status: MemoryStatus::Active,
            links: Vec::new(),
            assertions: Vec::new(),
            created_at,
            updated_at,
            metadata: Some(metadata),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "adapter storage `zbot_sidecars` failed: lock poisoned".to_string())
    }

    fn procedure_by_id(&self, id: &str) -> Result<Option<Procedure>, String> {
        self.connection()?
            .query_row(
                "SELECT record_json FROM procedures WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|json| serde_json::from_str::<Procedure>(&json))
            .transpose()
            .map_err(|error| format!("decode Procedure: {error}"))
    }

    fn upsert_procedure_record(
        &self,
        procedure: &Procedure,
        embedding: Option<&[f32]>,
    ) -> Result<(), String> {
        let record_json = serde_json::to_string(procedure).map_err(|error| error.to_string())?;
        let embedding_json = embedding
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let embedding_identity_json = embedding.map(|_| encode_identity(&self.embedding_identity));
        self.connection()?
            .execute(
                r#"
                INSERT INTO procedures
                    (id, agent_id, ward_id, name, success_count, failure_count,
                     created_at, updated_at, record_json, embedding_json, embedding_identity_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(id) DO UPDATE SET
                    agent_id = excluded.agent_id,
                    ward_id = excluded.ward_id,
                    name = excluded.name,
                    success_count = excluded.success_count,
                    failure_count = excluded.failure_count,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at,
                    record_json = excluded.record_json,
                    embedding_json = COALESCE(excluded.embedding_json, procedures.embedding_json),
                    embedding_identity_json = COALESCE(excluded.embedding_identity_json, procedures.embedding_identity_json)
                "#,
                params![
                    procedure.id.as_str(),
                    procedure.agent_id.as_str(),
                    procedure.ward_id.as_deref(),
                    procedure.name.as_str(),
                    procedure.success_count,
                    procedure.failure_count,
                    procedure.created_at.as_str(),
                    procedure.updated_at.as_str(),
                    record_json,
                    embedding_json,
                    embedding_identity_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn row_jsons(&self, sql: &str, values: Vec<SqlValue>) -> Result<Vec<Value>, String> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(sql).map_err(storage_error)?;
        let mut rows = statement
            .query(params_from_iter(values))
            .map_err(storage_error)?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(storage_error)? {
            let json: String = row.get(0).map_err(storage_error)?;
            values.push(serde_json::from_str(&json).map_err(|error| error.to_string())?);
        }
        Ok(values)
    }

    fn kg_episode_value(&self, id: &str) -> Result<Option<Value>, String> {
        self.connection()?
            .query_row(
                "SELECT id, source_type, source_ref, content_hash, session_id, agent_id,
                        status, retry_count, error, created_at, started_at, completed_at
                 FROM kg_episodes WHERE id = ?1",
                params![id],
                kg_episode_from_row,
            )
            .optional()
            .map_err(storage_error)
    }

    fn insert_compaction(
        &self,
        run_id: &str,
        kind: &str,
        payload: Value,
    ) -> Result<String, String> {
        let id = format!("cmp-{}", Uuid::new_v4());
        self.connection()?
            .execute(
                "INSERT INTO compaction_audit (id, run_id, kind, payload_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, run_id, kind, payload.to_string(), now()],
            )
            .map_err(storage_error)?;
        Ok(id)
    }
}

#[async_trait]
impl ProcedureStore for EngramSidecarStores {
    async fn list_by_ward(&self, ward_id: &str, limit: usize) -> Result<Vec<Value>, String> {
        self.row_jsons(
            "SELECT record_json FROM procedures
             WHERE ward_id = ?1
             ORDER BY updated_at DESC, id ASC
             LIMIT ?2",
            vec![
                SqlValue::Text(ward_id.to_string()),
                SqlValue::Integer(limit as i64),
            ],
        )
    }

    async fn upsert_procedure(
        &self,
        mut procedure: Procedure,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        procedure.embedding = None;
        self.mirror_procedure(&procedure).await?;
        self.upsert_procedure_record(&procedure, embedding.as_deref())
    }

    async fn search_procedures_by_similarity(
        &self,
        embedding: &[f32],
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        self.search_procedures_by_similarity_with_identity(
            embedding, None, agent_id, ward_id, limit,
        )
        .await
    }

    async fn search_procedures_by_similarity_with_identity(
        &self,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        if !identity_compatible(&self.embedding_identity, query_identity, embedding.len()) {
            return Ok(Vec::new());
        }
        let mut clauses = vec!["agent_id = ?1".to_string()];
        let mut values = vec![SqlValue::Text(agent_id.to_string())];
        if let Some(ward_id) = ward_id {
            values.push(SqlValue::Text(ward_id.to_string()));
            clauses.push(format!("ward_id = ?{}", values.len()));
        }
        let sql = format!(
            "SELECT record_json, embedding_json, embedding_identity_json FROM procedures WHERE {}",
            clauses.join(" AND ")
        );
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql).map_err(storage_error)?;
        let mut rows = statement
            .query(params_from_iter(values))
            .map_err(storage_error)?;
        let mut scored = Vec::new();
        while let Some(row) = rows.next().map_err(storage_error)? {
            let record_json: String = row.get(0).map_err(storage_error)?;
            let embedding_json: Option<String> = row.get(1).map_err(storage_error)?;
            let identity_json: Option<String> = row.get(2).map_err(storage_error)?;
            let Some(stored) = decode_embedding(embedding_json)? else {
                continue;
            };
            if !stored_identity_compatible(&self.embedding_identity, identity_json, stored.len())? {
                continue;
            }
            let score = cosine_f64(embedding, &stored);
            if score > 0.0 {
                scored.push((
                    serde_json::from_str::<Value>(&record_json).map_err(|e| e.to_string())?,
                    score,
                ));
            }
        }
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored
            .into_iter()
            .map(|(procedure, score)| json!({ "procedure": procedure, "score": score }))
            .collect())
    }

    async fn increment_success(
        &self,
        id: &str,
        duration_ms: Option<i64>,
        token_cost: Option<i64>,
    ) -> Result<(), String> {
        let Some(mut procedure) = self.procedure_by_id(id)? else {
            return Ok(());
        };
        procedure.success_count += 1;
        procedure.avg_duration_ms = duration_ms.or(procedure.avg_duration_ms);
        procedure.avg_token_cost = token_cost.or(procedure.avg_token_cost);
        procedure.last_used = Some(now());
        procedure.updated_at = now();
        self.mirror_procedure(&procedure).await?;
        self.upsert_procedure_record(&procedure, None)
    }

    async fn increment_failure(&self, id: &str) -> Result<(), String> {
        let Some(mut procedure) = self.procedure_by_id(id)? else {
            return Ok(());
        };
        procedure.failure_count += 1;
        procedure.updated_at = now();
        self.mirror_procedure(&procedure).await?;
        self.upsert_procedure_record(&procedure, None)
    }

    async fn procedure_stats(&self) -> Result<ProcedureStats, String> {
        let total = self
            .connection()?
            .query_row("SELECT COUNT(*) FROM procedures", [], |row| row.get(0))
            .map_err(storage_error)?;
        Ok(ProcedureStats { total })
    }

    async fn get_procedure_summary_by_name(
        &self,
        agent_id: &str,
        name: &str,
    ) -> Result<Option<ProcedureSummary>, String> {
        self.connection()?
            .query_row(
                "SELECT id, name, success_count FROM procedures
                 WHERE agent_id = ?1 AND name = ?2
                 ORDER BY success_count DESC, created_at DESC, id ASC
                 LIMIT 1",
                params![agent_id, name],
                |row| {
                    Ok(ProcedureSummary {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        success_count: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(storage_error)
    }

    async fn get_procedure_by_name(
        &self,
        agent_id: &str,
        name: &str,
    ) -> Result<Option<Procedure>, String> {
        self.connection()?
            .query_row(
                "SELECT record_json FROM procedures
                 WHERE agent_id = ?1 AND name = ?2
                 ORDER BY success_count DESC, created_at DESC, id ASC
                 LIMIT 1",
                params![agent_id, name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|json| serde_json::from_str::<Procedure>(&json))
            .transpose()
            .map_err(|error| format!("decode Procedure: {error}"))
    }

    async fn dedupe_procedures_by_name(&self) -> Result<usize, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT agent_id, name FROM procedures
                 GROUP BY agent_id, name HAVING COUNT(*) > 1",
            )
            .map_err(storage_error)?;
        let groups = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        drop(statement);

        let mut deleted = 0;
        for (agent_id, name) in groups {
            let keep: String = connection
                .query_row(
                    "SELECT id FROM procedures
                     WHERE agent_id = ?1 AND name = ?2
                     ORDER BY success_count DESC, created_at DESC, id ASC
                     LIMIT 1",
                    params![agent_id, name],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            deleted += connection
                .execute(
                    "DELETE FROM procedures
                     WHERE agent_id = ?1 AND name = ?2 AND id <> ?3",
                    params![agent_id, name, keep],
                )
                .map_err(storage_error)?;
        }
        Ok(deleted)
    }

    async fn insert_pattern_procedure(
        &self,
        req: PatternProcedureInsert,
    ) -> Result<String, String> {
        let id = format!("proc-{}", Uuid::new_v4());
        let timestamp = now();
        let procedure = Procedure {
            id: id.clone(),
            agent_id: req.agent_id,
            ward_id: req.ward_id,
            name: req.name,
            description: req.description,
            trigger_pattern: req.trigger_pattern,
            steps: req.steps_json,
            parameters: req.parameters_json,
            success_count: req.success_count,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        self.mirror_procedure(&procedure).await?;
        self.upsert_procedure_record(&procedure, req.embedding.as_deref())?;
        Ok(id)
    }
}

#[async_trait]
impl EpisodeStore for EngramSidecarStores {
    async fn list_by_ward(&self, ward_id: &str, limit: usize) -> Result<Vec<Value>, String> {
        self.row_jsons(
            "SELECT record_json FROM episodes
             WHERE ward_id = ?1
             ORDER BY created_at DESC, id ASC
             LIMIT ?2",
            vec![
                SqlValue::Text(ward_id.to_string()),
                SqlValue::Integer(limit as i64),
            ],
        )
    }

    async fn insert_episode(
        &self,
        mut episode: SessionEpisode,
        embedding: Option<Vec<f32>>,
    ) -> Result<String, String> {
        if episode.id.is_empty() {
            episode.id = format!("ep-{}", Uuid::new_v4());
        }
        let id = episode.id.clone();
        episode.embedding = None;
        self.mirror_episode(&episode).await?;
        let record_json = serde_json::to_string(&episode).map_err(|error| error.to_string())?;
        let embedding_json = embedding
            .as_deref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let embedding_identity_json = embedding
            .as_ref()
            .map(|_| encode_identity(&self.embedding_identity));
        self.connection()?
            .execute(
                r#"
                INSERT INTO episodes
                    (id, session_id, agent_id, ward_id, outcome, task_summary,
                     key_learnings, created_at, record_json, embedding_json, embedding_identity_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(id) DO UPDATE SET
                    session_id = excluded.session_id,
                    agent_id = excluded.agent_id,
                    ward_id = excluded.ward_id,
                    outcome = excluded.outcome,
                    task_summary = excluded.task_summary,
                    key_learnings = excluded.key_learnings,
                    created_at = excluded.created_at,
                    record_json = excluded.record_json,
                    embedding_json = COALESCE(excluded.embedding_json, episodes.embedding_json),
                    embedding_identity_json = COALESCE(excluded.embedding_identity_json, episodes.embedding_identity_json)
                "#,
                params![
                    episode.id.as_str(),
                    episode.session_id.as_str(),
                    episode.agent_id.as_str(),
                    episode.ward_id.as_str(),
                    episode.outcome.as_str(),
                    episode.task_summary.as_str(),
                    episode.key_learnings.as_deref(),
                    episode.created_at.as_str(),
                    record_json,
                    embedding_json,
                    embedding_identity_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(id)
    }

    async fn search_episodes_by_similarity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        self.search_episodes_by_similarity_with_identity(
            agent_id, embedding, None, threshold, limit,
        )
        .await
    }

    async fn search_episodes_by_similarity_with_identity(
        &self,
        agent_id: &str,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        if !identity_compatible(&self.embedding_identity, query_identity, embedding.len()) {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT record_json, embedding_json, embedding_identity_json FROM episodes WHERE agent_id = ?1",
            )
            .map_err(storage_error)?;
        let mut rows = statement.query(params![agent_id]).map_err(storage_error)?;
        let mut scored = Vec::new();
        while let Some(row) = rows.next().map_err(storage_error)? {
            let record_json: String = row.get(0).map_err(storage_error)?;
            let embedding_json: Option<String> = row.get(1).map_err(storage_error)?;
            let identity_json: Option<String> = row.get(2).map_err(storage_error)?;
            let Some(stored) = decode_embedding(embedding_json)? else {
                continue;
            };
            if !stored_identity_compatible(&self.embedding_identity, identity_json, stored.len())? {
                continue;
            }
            let score = cosine_f64(embedding, &stored);
            if score >= f64::from(threshold) {
                scored.push((
                    serde_json::from_str::<Value>(&record_json).map_err(|e| e.to_string())?,
                    score,
                ));
            }
        }
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored
            .into_iter()
            .map(|(episode, score)| json!({ "episode": episode, "score": score }))
            .collect())
    }

    async fn keyword_search_episodes(
        &self,
        query: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        let pattern = format!("%{}%", query);
        let (sql, values) = if let Some(ward_id) = ward_id {
            (
                "SELECT record_json FROM episodes
                 WHERE ward_id = ?1 AND (task_summary LIKE ?2 OR key_learnings LIKE ?2)
                 ORDER BY created_at DESC LIMIT ?3",
                vec![
                    SqlValue::Text(ward_id.to_string()),
                    SqlValue::Text(pattern),
                    SqlValue::Integer(limit as i64),
                ],
            )
        } else {
            (
                "SELECT record_json FROM episodes
                 WHERE task_summary LIKE ?1 OR key_learnings LIKE ?1
                 ORDER BY created_at DESC LIMIT ?2",
                vec![SqlValue::Text(pattern), SqlValue::Integer(limit as i64)],
            )
        };
        self.row_jsons(sql, values)?
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
            .collect()
    }

    async fn fetch_recent_successful_by_ward(
        &self,
        ward_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        self.row_jsons(
            "SELECT record_json FROM episodes
             WHERE ward_id = ?1 AND outcome = 'success'
             ORDER BY created_at DESC, id ASC LIMIT ?2",
            vec![
                SqlValue::Text(ward_id.to_string()),
                SqlValue::Integer(limit as i64),
            ],
        )?
        .into_iter()
        .map(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
        .collect()
    }

    async fn fetch_recent_failed_by_ward(
        &self,
        ward_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEpisode>, String> {
        self.row_jsons(
            "SELECT record_json FROM episodes
             WHERE ward_id = ?1 AND outcome = 'failed'
               AND key_learnings IS NOT NULL AND key_learnings != ''
             ORDER BY created_at DESC, id ASC LIMIT ?2",
            vec![
                SqlValue::Text(ward_id.to_string()),
                SqlValue::Integer(limit as i64),
            ],
        )?
        .into_iter()
        .map(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
        .collect()
    }

    async fn episode_stats(&self) -> Result<EpisodeStats, String> {
        let total = self
            .connection()?
            .query_row("SELECT COUNT(*) FROM episodes", [], |row| row.get(0))
            .map_err(storage_error)?;
        Ok(EpisodeStats { total })
    }

    async fn list_successful_episodes_with_embedding(
        &self,
        lookback_days: i64,
        limit: usize,
    ) -> Result<Vec<SuccessfulEpisode>, String> {
        let cutoff = (Utc::now() - Duration::days(lookback_days)).to_rfc3339();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, agent_id, task_summary, embedding_json, embedding_identity_json
                 FROM episodes
                 WHERE outcome = 'success' AND created_at >= ?1
                 ORDER BY created_at DESC, id ASC LIMIT ?2",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![cutoff, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(storage_error)?;
        rows.map(|row| {
            let (id, session_id, agent_id, task_summary, embedding_json, identity_json) =
                row.map_err(storage_error)?;
            let embedding = decode_embedding(embedding_json)?;
            let embedding = match embedding {
                Some(vector)
                    if stored_identity_compatible(
                        &self.embedding_identity,
                        identity_json,
                        vector.len(),
                    )? =>
                {
                    Some(vector)
                }
                _ => None,
            };
            Ok(SuccessfulEpisode {
                id,
                session_id,
                agent_id,
                task_summary,
                embedding,
            })
        })
        .collect()
    }

    async fn task_summaries_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<String>, String> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=session_ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT task_summary FROM episodes WHERE session_id IN ({placeholders})");
        let values = session_ids
            .iter()
            .map(|id| SqlValue::Text(id.clone()))
            .collect::<Vec<_>>();
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql).map_err(storage_error)?;
        let rows = statement
            .query_map(params_from_iter(values), |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }
}

#[async_trait]
impl KgEpisodeStore for EngramSidecarStores {
    async fn get_episode(&self, id: &str) -> Result<Option<Value>, String> {
        self.kg_episode_value(id)
    }

    async fn get_by_content_hash(
        &self,
        source_type: &str,
        content_hash: &str,
    ) -> Result<Option<Value>, String> {
        let id = self
            .connection()?
            .query_row(
                "SELECT id FROM kg_episodes WHERE source_type = ?1 AND content_hash = ?2",
                params![source_type, content_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        id.map(|id| self.kg_episode_value(&id))
            .transpose()
            .map(Option::flatten)
    }

    async fn list_by_session(&self, session_id: &str) -> Result<Vec<Value>, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, source_type, source_ref, content_hash, session_id, agent_id,
                        status, retry_count, error, created_at, started_at, completed_at
                 FROM kg_episodes WHERE session_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![session_id], kg_episode_from_row)
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    async fn status_counts_for_source(
        &self,
        source_ref_prefix: &str,
    ) -> Result<KgEpisodeStatusCounts, String> {
        let connection = self.connection()?;
        status_counts(
            &connection,
            "source_ref LIKE ?1",
            &[SqlValue::Text(format!("{source_ref_prefix}%"))],
        )
    }

    async fn count_pending_global(&self) -> Result<u64, String> {
        self.connection()?
            .query_row(
                "SELECT COUNT(*) FROM kg_episodes WHERE status = 'pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count as u64)
            .map_err(storage_error)
    }

    async fn count_pending_for_source(&self, source_ref_prefix: &str) -> Result<u64, String> {
        self.connection()?
            .query_row(
                "SELECT COUNT(*) FROM kg_episodes
                 WHERE status = 'pending' AND source_ref LIKE ?1",
                params![format!("{source_ref_prefix}%")],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count as u64)
            .map_err(storage_error)
    }

    async fn upsert_pending(
        &self,
        source_type: &str,
        source_ref: &str,
        content_hash: &str,
        session_id: Option<&str>,
        agent_id: &str,
    ) -> Result<String, String> {
        if let Some(existing) = self
            .connection()?
            .query_row(
                "SELECT id FROM kg_episodes WHERE source_type = ?1 AND content_hash = ?2",
                params![source_type, content_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
        {
            return Ok(existing);
        }

        let id = format!("kg-{}", Uuid::new_v4());
        self.connection()?
            .execute(
                "INSERT INTO kg_episodes
                    (id, source_type, source_ref, content_hash, session_id, agent_id,
                     status, retry_count, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7)",
                params![
                    id,
                    source_type,
                    source_ref,
                    content_hash,
                    session_id,
                    agent_id,
                    now()
                ],
            )
            .map_err(storage_error)?;
        Ok(id)
    }

    async fn claim_next_pending(&self) -> Result<Option<Value>, String> {
        let id = self
            .connection()?
            .query_row(
                "SELECT id FROM kg_episodes
                 WHERE status = 'pending'
                 ORDER BY created_at ASC, id ASC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        let Some(id) = id else {
            return Ok(None);
        };
        self.connection()?
            .execute(
                "UPDATE kg_episodes SET status = 'running', started_at = ?1 WHERE id = ?2",
                params![now(), id],
            )
            .map_err(storage_error)?;
        self.kg_episode_value(&id)
    }

    async fn mark_done(&self, id: &str) -> Result<(), String> {
        self.connection()?
            .execute(
                "UPDATE kg_episodes SET status = 'done', completed_at = ?1, error = NULL WHERE id = ?2",
                params![now(), id],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    async fn mark_failed(&self, id: &str, error: &str) -> Result<(), String> {
        self.connection()?
            .execute(
                "UPDATE kg_episodes SET status = 'failed', completed_at = ?1, error = ?2 WHERE id = ?3",
                params![now(), error, id],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    async fn retry_if_eligible(&self, id: &str, max_retries: u32) -> Result<bool, String> {
        let retry_count = self
            .connection()?
            .query_row(
                "SELECT retry_count FROM kg_episodes WHERE id = ?1 AND status = 'failed'",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(storage_error)?;
        let Some(retry_count) = retry_count else {
            return Ok(false);
        };
        if retry_count >= i64::from(max_retries) {
            return Ok(false);
        }
        self.connection()?
            .execute(
                "UPDATE kg_episodes
                 SET status = 'pending', retry_count = retry_count + 1,
                     error = NULL, started_at = NULL, completed_at = NULL
                 WHERE id = ?1",
                params![id],
            )
            .map_err(storage_error)?;
        Ok(true)
    }

    async fn set_payload(&self, id: &str, text: &str) -> Result<(), String> {
        self.mirror_evidence_payload(text).await?;
        self.connection()?
            .execute(
                "UPDATE kg_episodes SET payload = ?1 WHERE id = ?2",
                params![text, id],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    async fn get_payload(&self, id: &str) -> Result<Option<String>, String> {
        self.connection()?
            .query_row(
                "SELECT payload FROM kg_episodes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)
            .map(Option::flatten)
    }
}

#[async_trait]
impl GoalStore for EngramSidecarStores {
    async fn get_goal(&self, goal_id: &str) -> Result<Option<Value>, String> {
        self.connection()?
            .query_row(
                "SELECT record_json FROM goals WHERE id = ?1",
                params![goal_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|json| serde_json::from_str::<Value>(&json))
            .transpose()
            .map_err(|error| error.to_string())
    }

    async fn list_active_goals(&self, agent_id: &str) -> Result<Vec<Value>, String> {
        self.row_jsons(
            "SELECT record_json FROM goals
             WHERE agent_id = ?1 AND state = 'active'
             ORDER BY updated_at DESC, id ASC",
            vec![SqlValue::Text(agent_id.to_string())],
        )
    }

    async fn create_goal(&self, mut goal: Value) -> Result<String, String> {
        let id = json_string(&goal, "id").unwrap_or_else(|| format!("goal-{}", Uuid::new_v4()));
        let agent_id = json_string(&goal, "agent_id")
            .or_else(|| json_string(&goal, "agentId"))
            .unwrap_or_else(|| "__global__".to_string());
        let state = json_string(&goal, "state").unwrap_or_else(|| "active".to_string());
        set_json_string(&mut goal, "id", &id);
        set_json_string(&mut goal, "agent_id", &agent_id);
        set_json_string(&mut goal, "state", &state);
        self.connection()?
            .execute(
                "INSERT INTO goals (id, agent_id, state, record_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    agent_id = excluded.agent_id,
                    state = excluded.state,
                    record_json = excluded.record_json,
                    updated_at = excluded.updated_at",
                params![id, agent_id, state, goal.to_string(), now()],
            )
            .map_err(storage_error)?;
        Ok(id)
    }

    async fn update_goal_state(&self, goal_id: &str, new_state: &str) -> Result<(), String> {
        let Some(mut goal) = self.get_goal(goal_id).await? else {
            return Ok(());
        };
        set_json_string(&mut goal, "state", new_state);
        self.connection()?
            .execute(
                "UPDATE goals SET state = ?1, record_json = ?2, updated_at = ?3 WHERE id = ?4",
                params![new_state, goal.to_string(), now(), goal_id],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    async fn update_goal_filled_slots(
        &self,
        goal_id: &str,
        filled_slots_json: &str,
    ) -> Result<(), String> {
        let Some(mut goal) = self.get_goal(goal_id).await? else {
            return Ok(());
        };
        let slots = serde_json::from_str::<Value>(filled_slots_json)
            .unwrap_or_else(|_| Value::String(filled_slots_json.to_string()));
        if let Some(object) = goal.as_object_mut() {
            object.insert("filled_slots".to_string(), slots);
        }
        self.connection()?
            .execute(
                "UPDATE goals SET record_json = ?1, updated_at = ?2 WHERE id = ?3",
                params![goal.to_string(), now(), goal_id],
            )
            .map_err(storage_error)?;
        Ok(())
    }
}

#[async_trait]
impl RecallLogStore for EngramSidecarStores {
    async fn log_recall(&self, session_id: &str, fact_key: &str) -> Result<(), String> {
        self.connection()?
            .execute(
                "INSERT INTO recall_log (session_id, fact_key, recalled_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(session_id, fact_key) DO UPDATE SET recalled_at = excluded.recalled_at",
                params![session_id, fact_key, now()],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    async fn get_keys_for_session(&self, session_id: &str) -> Result<Vec<String>, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT fact_key FROM recall_log WHERE session_id = ?1 ORDER BY fact_key ASC")
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    async fn get_keys_for_sessions(&self, session_ids: &[String]) -> Result<Vec<String>, String> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=session_ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT fact_key FROM recall_log WHERE session_id IN ({placeholders}) ORDER BY fact_key ASC"
        );
        let values = session_ids
            .iter()
            .map(|id| SqlValue::Text(id.clone()))
            .collect::<Vec<_>>();
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql).map_err(storage_error)?;
        let rows = statement
            .query_map(params_from_iter(values), |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }
}

#[async_trait]
impl DistillationStore for EngramSidecarStores {
    async fn insert_run(&self, run: Value) -> Result<(), String> {
        let session_id = json_string(&run, "session_id")
            .or_else(|| json_string(&run, "sessionId"))
            .ok_or_else(|| "distillation run missing session_id".to_string())?;
        let status = json_string(&run, "status").unwrap_or_else(|| "pending".to_string());
        let retry_count = run
            .get("retry_count")
            .or_else(|| run.get("retryCount"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        self.connection()?
            .execute(
                "INSERT INTO distillation_runs (session_id, status, retry_count, record_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(session_id) DO UPDATE SET
                    status = excluded.status,
                    retry_count = excluded.retry_count,
                    record_json = excluded.record_json,
                    updated_at = excluded.updated_at",
                params![session_id, status, retry_count, run.to_string(), now()],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    async fn get_run_by_session(&self, session_id: &str) -> Result<Option<Value>, String> {
        self.connection()?
            .query_row(
                "SELECT record_json FROM distillation_runs WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|json| serde_json::from_str::<Value>(&json))
            .transpose()
            .map_err(|error| error.to_string())
    }

    async fn update_retry(&self, session_id: &str) -> Result<(), String> {
        let mut run = self.get_run_by_session(session_id).await?.unwrap_or_else(
            || json!({ "session_id": session_id, "status": "failed", "retry_count": 0 }),
        );
        let retry_count = run.get("retry_count").and_then(Value::as_i64).unwrap_or(0) + 1;
        set_json_i64(&mut run, "retry_count", retry_count);
        set_json_string(&mut run, "status", "failed");
        self.insert_run(run).await
    }

    async fn update_success(
        &self,
        session_id: &str,
        summary: Option<String>,
    ) -> Result<(), String> {
        let mut run = self
            .get_run_by_session(session_id)
            .await?
            .unwrap_or_else(|| json!({ "session_id": session_id }));
        set_json_string(&mut run, "status", "success");
        if let Some(summary) = summary {
            set_json_string(&mut run, "summary", &summary);
        }
        self.insert_run(run).await
    }

    async fn record_distillation_pending(
        &self,
        session_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let mut run = json!({
            "session_id": session_id,
            "status": status,
            "retry_count": 0 });
        if let Some(error) = error {
            set_json_string(&mut run, "error", error);
        }
        self.insert_run(run).await
    }

    async fn record_distillation_success(
        &self,
        session_id: &str,
        facts: i32,
        entities: i32,
        relationships: i32,
        episode_created: bool,
        duration_ms: i64,
    ) -> Result<(), String> {
        self.insert_run(json!({
            "session_id": session_id,
            "status": "success",
            "retry_count": 0,
            "facts_extracted": facts,
            "entities_extracted": entities,
            "relationships_extracted": relationships,
            "episode_created": episode_created,
            "duration_ms": duration_ms }))
            .await
    }

    async fn record_distillation_failure(
        &self,
        session_id: &str,
        status: &str,
        retry_count: i32,
        error: Option<&str>,
    ) -> Result<(), String> {
        let mut run = json!({
            "session_id": session_id,
            "status": status,
            "retry_count": retry_count });
        if let Some(error) = error {
            set_json_string(&mut run, "error", error);
        }
        self.insert_run(run).await
    }
}

#[async_trait]
impl CompactionStore for EngramSidecarStores {
    async fn record_merge(
        &self,
        run_id: &str,
        loser_entity_id: &str,
        winner_entity_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.insert_compaction(
            run_id,
            "merge",
            json!({
                "loser_entity_id": loser_entity_id,
                "winner_entity_id": winner_entity_id,
                "reason": reason }),
        )
    }

    async fn record_synthesis(
        &self,
        run_id: &str,
        fact_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.insert_compaction(
            run_id,
            "synthesis",
            json!({ "fact_id": fact_id, "reason": reason }),
        )
    }

    async fn record_pattern(
        &self,
        run_id: &str,
        procedure_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.insert_compaction(
            run_id,
            "pattern",
            json!({ "procedure_id": procedure_id, "reason": reason }),
        )
    }

    async fn record_prune(
        &self,
        run_id: &str,
        entity_id: Option<&str>,
        relationship_id: Option<&str>,
        reason: &str,
    ) -> Result<String, String> {
        self.insert_compaction(
            run_id,
            "prune",
            json!({
                "entity_id": entity_id,
                "relationship_id": relationship_id,
                "reason": reason }),
        )
    }

    async fn record_archival(
        &self,
        run_id: &str,
        entity_id: &str,
        reason: &str,
    ) -> Result<String, String> {
        self.insert_compaction(
            run_id,
            "archival",
            json!({ "entity_id": entity_id, "reason": reason }),
        )
    }

    async fn latest_run_summary(&self) -> Result<Option<CompactionRunSummary>, String> {
        let latest = self
            .connection()?
            .query_row(
                "SELECT run_id, MAX(created_at) FROM compaction_audit GROUP BY run_id
                 ORDER BY MAX(created_at) DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(storage_error)?;
        let Some((run_id, latest_at)) = latest else {
            return Ok(None);
        };
        let (merges, prunes) = self
            .connection()?
            .query_row(
                "SELECT
                    SUM(CASE WHEN kind = 'merge' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN kind = 'prune' THEN 1 ELSE 0 END)
                 FROM compaction_audit WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .map_err(storage_error)?;
        Ok(Some(CompactionRunSummary {
            run_id,
            latest_at,
            merges: merges.unwrap_or(0) as u64,
            prunes: prunes.unwrap_or(0) as u64,
        }))
    }
}

impl OutboxStore for EngramSidecarStores {
    fn insert_item(
        &self,
        adapter_id: &str,
        capability: &str,
        payload: &Value,
        session_id: Option<&str>,
        thread_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<String, String> {
        let id = format!("obx-{}", Uuid::new_v4());
        let timestamp = now();
        self.connection()?
            .execute(
                "INSERT INTO outbox
                    (id, adapter_id, capability, status, payload_json,
                     session_id, thread_id, agent_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    id,
                    adapter_id,
                    capability,
                    payload.to_string(),
                    session_id,
                    thread_id,
                    agent_id,
                    timestamp,
                ],
            )
            .map_err(storage_error)?;
        Ok(id)
    }

    fn mark_inflight(&self, id: &str) -> Result<(), String> {
        self.connection()?
            .execute(
                "UPDATE outbox SET status = 'inflight', updated_at = ?1 WHERE id = ?2",
                params![now(), id],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn mark_sent(&self, id: &str) -> Result<(), String> {
        self.connection()?
            .execute(
                "UPDATE outbox SET status = 'sent', updated_at = ?1 WHERE id = ?2",
                params![now(), id],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn reset_inflight(&self, adapter_id: &str) -> Result<usize, String> {
        self.connection()?
            .execute(
                "UPDATE outbox SET status = 'pending', updated_at = ?1
                 WHERE adapter_id = ?2 AND status = 'inflight'",
                params![now(), adapter_id],
            )
            .map_err(storage_error)
    }
}

fn kg_episode_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "source_type": row.get::<_, String>(1)?,
        "source_ref": row.get::<_, String>(2)?,
        "content_hash": row.get::<_, String>(3)?,
        "session_id": row.get::<_, Option<String>>(4)?,
        "agent_id": row.get::<_, String>(5)?,
        "status": row.get::<_, String>(6)?,
        "retry_count": row.get::<_, i64>(7)?,
        "error": row.get::<_, Option<String>>(8)?,
        "created_at": row.get::<_, String>(9)?,
        "started_at": row.get::<_, Option<String>>(10)?,
        "completed_at": row.get::<_, Option<String>>(11)? }))
}

fn status_counts(
    connection: &Connection,
    predicate: &str,
    values: &[SqlValue],
) -> Result<KgEpisodeStatusCounts, String> {
    let sql = format!("SELECT status, COUNT(*) FROM kg_episodes WHERE {predicate} GROUP BY status");
    let mut statement = connection.prepare(&sql).map_err(storage_error)?;
    let rows = statement
        .query_map(params_from_iter(values.iter().cloned()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(storage_error)?;
    let mut counts = KgEpisodeStatusCounts::default();
    for row in rows {
        let (status, count) = row.map_err(storage_error)?;
        match status.as_str() {
            "pending" => counts.pending = count as u64,
            "running" => counts.running = count as u64,
            "done" => counts.done = count as u64,
            "failed" => counts.failed = count as u64,
            _ => {}
        }
    }
    Ok(counts)
}

fn json_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn set_json_string(value: &mut Value, key: &str, content: &str) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_string(), Value::String(content.to_string()));
    }
}

fn set_json_i64(value: &mut Value, key: &str, content: i64) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_string(), Value::Number(content.into()));
    }
}

fn decode_embedding(value: Option<String>) -> Result<Option<Vec<f32>>, String> {
    value
        .map(|json| serde_json::from_str::<Vec<f32>>(&json))
        .transpose()
        .map_err(|error| format!("decode embedding: {error}"))
}

fn ensure_optional_column(
    connection: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if names.iter().any(|name| name == column) {
        return Ok(());
    }
    connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
        [],
    )?;
    Ok(())
}

fn embedding_identity_from_config(config: &AdapterConfig) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: config.embedding_provider.provider_type.clone(),
        model: config.embedding_provider.model.clone(),
        dimensions: config.embedding_provider.dimensions,
        prompt_profile: config.embedding_provider.prompt_profile.clone(),
        normalization: config.embedding_provider.normalization.clone(),
    }
}

fn encode_identity(identity: &EmbeddingQueryIdentity) -> String {
    json!({
        "providerType": identity.provider_type.clone(),
        "model": identity.model.clone(),
        "dimensions": identity.dimensions,
        "promptProfile": identity.prompt_profile.clone(),
        "normalization": identity.normalization.clone() })
    .to_string()
}

fn decode_identity(value: Option<String>) -> Result<Option<EmbeddingQueryIdentity>, String> {
    let Some(json) = value else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&json)
        .map_err(|error| format!("decode embedding identity: {error}"))?;
    let dimensions = value
        .get("dimensions")
        .and_then(Value::as_u64)
        .ok_or_else(|| "decode embedding identity: missing dimensions".to_string())?
        as u32;
    Ok(Some(EmbeddingQueryIdentity {
        provider_type: value
            .get("providerType")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        dimensions,
        prompt_profile: value
            .get("promptProfile")
            .and_then(Value::as_str)
            .unwrap_or("query")
            .to_string(),
        normalization: value
            .get("normalization")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }))
}

fn identity_compatible(
    expected: &EmbeddingQueryIdentity,
    actual: Option<&EmbeddingQueryIdentity>,
    vector_dimensions: usize,
) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    expected.provider_type == actual.provider_type
        && expected.model == actual.model
        && expected.dimensions == actual.dimensions
        && expected.dimensions as usize == vector_dimensions
        && expected.prompt_profile == actual.prompt_profile
        && expected.normalization == actual.normalization
}

fn stored_identity_compatible(
    expected: &EmbeddingQueryIdentity,
    identity_json: Option<String>,
    vector_dimensions: usize,
) -> Result<bool, String> {
    let actual = decode_identity(identity_json)?;
    Ok(identity_compatible(
        expected,
        actual.as_ref(),
        vector_dimensions,
    ))
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn parse_sidecar_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| "sidecar semantic record has an invalid timestamp".to_string())
}

fn storage_error(error: rusqlite::Error) -> String {
    AdapterError::Storage {
        component: SIDECAR_COMPONENT,
        reason: error.to_string(),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GovernanceSelection, ZBOT_BASE_ONTOLOGY_ID, ZBOT_GENERAL_SCHEME_ID};

    fn governed_config(root: &tempfile::TempDir) -> AdapterConfig {
        let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
        config.embedding_provider.dimensions = 2;
        config.governance.default_selection = GovernanceSelection {
            ontology_ids: vec![ZBOT_BASE_ONTOLOGY_ID.to_string()],
            taxonomy_scheme_ids: vec![ZBOT_GENERAL_SCHEME_ID.to_string()],
        };
        config
    }

    fn procedure() -> Procedure {
        Procedure {
            id: "proc-governed".to_string(),
            agent_id: "agent-a".to_string(),
            ward_id: Some("ward-a".to_string()),
            name: "build".to_string(),
            description: "Build and verify the application.".to_string(),
            trigger_pattern: None,
            steps: "[]".to_string(),
            parameters: None,
            success_count: 1,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
            updated_at: "2026-07-13T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn semantic_sidecars_are_mirrored_with_governance_metadata() {
        let root = tempfile::tempdir().expect("root");
        let store = EngramSidecarStores::open(governed_config(&root)).expect("store");
        let procedure = procedure();

        ProcedureStore::upsert_procedure(&store, procedure, None)
            .await
            .expect("procedure write");

        let scope = store
            .mapper
            .memory_fact_scope("ward-a", None)
            .expect("procedure scope");
        let procedure_record = store
            .memory
            .get_memory(
                &MemoryId::from("zbot-sidecar:procedure:proc-governed"),
                &scope,
            )
            .await
            .expect("procedure lookup")
            .expect("canonical procedure");
        let metadata = procedure_record.metadata.expect("procedure metadata");
        assert_eq!(
            metadata.get("governanceOntologyIds"),
            Some(&json!([ZBOT_BASE_ONTOLOGY_ID]))
        );
        assert_eq!(
            metadata.get("governanceTaxonomySchemeIds"),
            Some(&json!([ZBOT_GENERAL_SCHEME_ID]))
        );
        assert_eq!(
            metadata.get("governanceTaxonomyConceptIds"),
            Some(&json!([format!("{ZBOT_GENERAL_SCHEME_ID}:concept:memory")]))
        );
        assert_eq!(
            metadata.get("governanceRecordKind"),
            Some(&json!("procedure"))
        );

        let episode = SessionEpisode {
            id: "episode-governed".to_string(),
            session_id: "sess-a".to_string(),
            agent_id: "agent-a".to_string(),
            ward_id: "ward-a".to_string(),
            task_summary: "Implemented the governed sidecar mirror.".to_string(),
            outcome: "success".to_string(),
            strategy_used: None,
            key_learnings: Some("Use Engram for canonical semantics.".to_string()),
            token_cost: None,
            embedding: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
        };
        EpisodeStore::insert_episode(&store, episode, None)
            .await
            .expect("episode write");
        let episode_scope = store
            .mapper
            .memory_fact_scope("ward-a", Some("sess-a"))
            .expect("episode scope");
        let episode_record = store
            .memory
            .get_memory(
                &MemoryId::from("zbot-sidecar:episode:episode-governed"),
                &episode_scope,
            )
            .await
            .expect("episode lookup")
            .expect("canonical episode");
        assert_eq!(episode_record.kind, MemoryKind::Episode);
        assert_eq!(
            episode_record
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("governanceRecordKind")),
            Some(&json!("episode"))
        );
        assert_eq!(
            episode_record
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("governanceTaxonomyConceptIds")),
            Some(&json!([format!("{ZBOT_GENERAL_SCHEME_ID}:concept:memory")]))
        );

        let evidence_id = KgEpisodeStore::upsert_pending(
            &store,
            "evidence_intake",
            "source-a",
            "content-hash-a",
            Some("sess-a"),
            "agent-a",
        )
        .await
        .expect("evidence pending");
        KgEpisodeStore::set_payload(
            &store,
            &evidence_id,
            &json!({
                "evidence_id": "evidence-governed",
                "agent_id": "agent-a",
                "source_id": "source-a",
                "source_type": "connector",
                "session_id": "sess-a",
                "ward_id": "ward-a" })
            .to_string(),
        )
        .await
        .expect("evidence payload");
        let evidence_record = store
            .memory
            .get_memory(
                &MemoryId::from("zbot-sidecar:evidence:evidence-governed"),
                &episode_scope,
            )
            .await
            .expect("evidence lookup")
            .expect("canonical evidence");
        assert_eq!(evidence_record.kind, MemoryKind::Artifact);
        assert_eq!(
            evidence_record
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("governanceRecordKind")),
            Some(&json!("evidence"))
        );
        assert_eq!(
            evidence_record
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("governanceTaxonomyConceptIds")),
            Some(&json!([format!("{ZBOT_GENERAL_SCHEME_ID}:concept:memory")]))
        );
    }
}
