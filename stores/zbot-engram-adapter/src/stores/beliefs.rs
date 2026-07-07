//! `BeliefStore` and `BeliefContradictionStore` backed by Engram belief records.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use engram_belief::{BeliefQuery, BeliefQueryOrder, BeliefRepository};
use engram_domain::{BeliefId as EngramBeliefId, BeliefStatus, ContradictionId, Scope};
use rusqlite::{params, Connection, OptionalExtension};
use zbot_stores_traits::{
    Belief, BeliefContradiction, BeliefContradictionStore, BeliefStore, EmbeddingQueryIdentity,
    Resolution, ScoredBelief,
};

use crate::{
    bootstrap::EngramProvider,
    capabilities::AdapterFeature,
    config::{AdapterConfig, ProviderMode},
    error::{AdapterError, AdapterResult},
    mapping::belief::{
        belief_record_to_belief, belief_to_belief_record, contradiction_resolution_to_record,
        contradiction_to_record,
    },
    scope::ScopeMapper,
};

const SIDECAR_COMPONENT: &str = "belief_sidecar";

/// Engram-backed implementation of AgentZero belief and contradiction traits.
#[derive(Clone)]
pub struct EngramBeliefStore {
    beliefs: Arc<dyn BeliefRepository>,
    mapper: ScopeMapper,
    sidecar: BeliefSidecar,
}

impl EngramBeliefStore {
    /// Open an Engram-backed belief store for `ProviderMode::Engram`.
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "beliefs",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        let provider = EngramProvider::open(config.clone())?;
        Self::from_provider(config, &provider)
    }

    /// Build a belief store from an already-bootstrapped Engram provider.
    pub fn from_provider(config: AdapterConfig, provider: &EngramProvider) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "beliefs",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        provider.require_feature(AdapterFeature::Beliefs)?;
        provider.require_feature(AdapterFeature::Contradictions)?;
        let mapper = config.scope_mapper()?;
        let beliefs = provider.beliefs()?;
        let sidecar = BeliefSidecar::open(
            &config.compatibility_store_path("zbot-beliefs.sqlite")?,
            embedding_identity_from_config(&config),
        )?;

        Ok(Self {
            beliefs,
            mapper,
            sidecar,
        })
    }

    /// Probe Engram record-time behavior explicitly without changing zbot traits.
    pub async fn get_belief_recorded_at(
        &self,
        partition_id: &str,
        subject: &str,
        valid_at: DateTime<Utc>,
        recorded_at: DateTime<Utc>,
    ) -> Result<Option<Belief>, String> {
        let scope = self.scope_for_partition(partition_id)?;
        let query = BeliefQuery {
            scope,
            subject_key: Some(subject.to_string()),
            valid_at: Some(valid_at),
            recorded_at: Some(recorded_at),
            statuses: vec![BeliefStatus::Active],
            include_stale: false,
            order: BeliefQueryOrder::LatestRecordedFirst,
        };
        self.beliefs
            .get_belief(query)
            .await
            .map_err(|error| format!("unsupported: record_time_history: {error}"))?
            .map(|record| {
                let mut belief =
                    belief_record_to_belief(&record).map_err(AdapterError::into_trait_error)?;
                belief.embedding = self.sidecar.embedding_for_belief(&belief.id)?;
                Ok(belief)
            })
            .transpose()
    }

    fn scope_for_partition(&self, partition_id: &str) -> Result<Scope, String> {
        self.mapper
            .partition_scope(partition_id)
            .map_err(AdapterError::into_trait_error)
    }

    async fn write_belief(&self, belief: &Belief) -> Result<(), String> {
        let mut belief = belief.clone();
        if let Some(existing) =
            self.sidecar
                .find_by_key(&belief.partition_id, &belief.subject, belief.valid_from)?
        {
            belief.id = existing.belief.id;
            belief.created_at = existing.belief.created_at;
        }
        let embedding = belief.embedding.clone();
        let record = belief_to_belief_record(&belief, &self.mapper)
            .map_err(AdapterError::into_trait_error)?;
        self.beliefs
            .upsert_belief(record)
            .await
            .map_err(|error| error.to_string())?;
        belief.embedding = None;
        self.sidecar.store_belief(&belief, embedding.as_deref())
    }

    async fn update_belief_from_record(
        &self,
        record: engram_domain::Belief,
    ) -> Result<Belief, String> {
        let mut belief =
            belief_record_to_belief(&record).map_err(AdapterError::into_trait_error)?;
        let embedding = self.sidecar.embedding_for_belief(&belief.id)?;
        self.sidecar.store_belief(&belief, embedding.as_deref())?;
        belief.embedding = embedding;
        Ok(belief)
    }
}

#[async_trait]
impl BeliefStore for EngramBeliefStore {
    async fn get_belief(
        &self,
        partition_id: &str,
        subject: &str,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Option<Belief>, String> {
        self.sidecar
            .get_belief(partition_id, subject, as_of.unwrap_or_else(Utc::now))
    }

    async fn list_beliefs(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String> {
        self.sidecar.list_beliefs(partition_id, limit)
    }

    async fn upsert_belief(&self, belief: &Belief) -> Result<(), String> {
        self.write_belief(belief).await
    }

    async fn supersede_belief(
        &self,
        old_id: &str,
        new_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String> {
        let belief = self
            .sidecar
            .get_belief_by_id(old_id)?
            .ok_or_else(|| format!("belief not found: {old_id}"))?;
        let scope = self.scope_for_partition(&belief.partition_id)?;
        let record = self
            .beliefs
            .supersede_belief(
                &EngramBeliefId::from(old_id),
                &scope,
                EngramBeliefId::from(new_id),
                transition_time,
            )
            .await
            .map_err(|error| error.to_string())?;
        self.update_belief_from_record(record).await.map(|_| ())
    }

    async fn mark_stale(&self, belief_id: &str) -> Result<(), String> {
        let belief = self
            .sidecar
            .get_belief_by_id(belief_id)?
            .ok_or_else(|| format!("belief not found: {belief_id}"))?;
        let scope = self.scope_for_partition(&belief.partition_id)?;
        let record = self
            .beliefs
            .mark_stale(&EngramBeliefId::from(belief_id), &scope, Utc::now())
            .await
            .map_err(|error| error.to_string())?;
        self.update_belief_from_record(record).await.map(|_| ())
    }

    async fn retract_belief(
        &self,
        belief_id: &str,
        transition_time: DateTime<Utc>,
    ) -> Result<(), String> {
        let belief = self
            .sidecar
            .get_belief_by_id(belief_id)?
            .ok_or_else(|| format!("belief not found: {belief_id}"))?;
        let scope = self.scope_for_partition(&belief.partition_id)?;
        let record = self
            .beliefs
            .retract_belief(&EngramBeliefId::from(belief_id), &scope, transition_time)
            .await
            .map_err(|error| error.to_string())?;
        self.update_belief_from_record(record).await.map(|_| ())
    }

    async fn beliefs_referencing_fact(&self, fact_id: &str) -> Result<Vec<String>, String> {
        self.sidecar.beliefs_referencing_fact(fact_id)
    }

    async fn get_belief_by_id(&self, belief_id: &str) -> Result<Option<Belief>, String> {
        self.sidecar.get_belief_by_id(belief_id)
    }

    async fn list_stale(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String> {
        self.sidecar.list_stale(partition_id, limit)
    }

    async fn clear_stale(&self, belief_id: &str) -> Result<(), String> {
        let belief = self
            .sidecar
            .get_belief_by_id(belief_id)?
            .ok_or_else(|| format!("belief not found: {belief_id}"))?;
        let scope = self.scope_for_partition(&belief.partition_id)?;
        let record = self
            .beliefs
            .clear_stale(&EngramBeliefId::from(belief_id), &scope, Utc::now())
            .await
            .map_err(|error| error.to_string())?;
        self.update_belief_from_record(record).await.map(|_| ())
    }

    async fn search_beliefs(
        &self,
        partition_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<ScoredBelief>, String> {
        let _ = (partition_id, query_embedding, limit);
        Ok(Vec::new())
    }

    async fn search_beliefs_with_identity(
        &self,
        partition_id: &str,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        limit: usize,
    ) -> Result<Vec<ScoredBelief>, String> {
        self.sidecar
            .search_beliefs(partition_id, query_embedding, query_identity, limit)
    }
}

#[async_trait]
impl BeliefContradictionStore for EngramBeliefStore {
    async fn insert_contradiction(&self, c: &BeliefContradiction) -> Result<(), String> {
        let partition_id = self
            .sidecar
            .partition_for_belief_pair(&c.belief_a_id, &c.belief_b_id)?
            .ok_or_else(|| "cannot infer contradiction partition".to_string())?;
        let canonical = canonicalize_zbot_contradiction(c);
        let record = contradiction_to_record(&canonical, &partition_id, &self.mapper)
            .map_err(AdapterError::into_trait_error)?;
        let persisted = self
            .beliefs
            .put_contradiction(record)
            .await
            .map_err(|error| error.to_string())?;
        let persisted = crate::mapping::belief::contradiction_record_to_contradiction(&persisted)
            .map_err(AdapterError::into_trait_error)?;
        self.sidecar
            .store_contradiction(&persisted, &partition_id, true)
    }

    async fn for_belief(&self, belief_id: &str) -> Result<Vec<BeliefContradiction>, String> {
        self.sidecar.contradictions_for_belief(belief_id)
    }

    async fn list_recent(
        &self,
        partition_id: &str,
        limit: usize,
    ) -> Result<Vec<BeliefContradiction>, String> {
        self.sidecar.list_recent_contradictions(partition_id, limit)
    }

    async fn pair_exists(&self, belief_a_id: &str, belief_b_id: &str) -> Result<bool, String> {
        self.sidecar.pair_exists(belief_a_id, belief_b_id)
    }

    async fn resolve(&self, contradiction_id: &str, resolution: Resolution) -> Result<(), String> {
        let contradiction = self
            .sidecar
            .get_contradiction(contradiction_id)?
            .ok_or_else(|| format!("contradiction not found: {contradiction_id}"))?;
        let partition_id = self
            .sidecar
            .partition_for_belief_pair(&contradiction.belief_a_id, &contradiction.belief_b_id)?
            .ok_or_else(|| "cannot infer contradiction partition".to_string())?;
        let scope = self.scope_for_partition(&partition_id)?;
        let resolved_at = Utc::now();
        let record_resolution = contradiction_resolution_to_record(
            &resolution,
            &contradiction.belief_a_id,
            &contradiction.belief_b_id,
            resolved_at,
        );
        self.beliefs
            .resolve_contradiction(
                &ContradictionId::from(contradiction_id),
                &scope,
                record_resolution,
            )
            .await
            .map_err(|error| error.to_string())?;
        let mut updated = contradiction;
        updated.resolution = Some(resolution);
        updated.resolved_at = Some(resolved_at);
        self.sidecar
            .store_contradiction(&updated, &partition_id, false)
    }
}

#[derive(Clone)]
struct BeliefSidecar {
    connection: Arc<Mutex<Connection>>,
    embedding_identity: EmbeddingQueryIdentity,
}

#[derive(Debug, Clone)]
struct BeliefEntry {
    belief: Belief,
    embedding_identity: Option<EmbeddingQueryIdentity>,
}

impl BeliefSidecar {
    fn open(path: &Path, embedding_identity: EmbeddingQueryIdentity) -> AdapterResult<Self> {
        let connection = Connection::open(path).map_err(|error| AdapterError::Storage {
            component: SIDECAR_COMPONENT,
            reason: error.to_string(),
        })?;
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;
                CREATE TABLE IF NOT EXISTS zbot_beliefs (
                    id TEXT PRIMARY KEY,
                    partition_id TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    valid_from TEXT,
                    valid_until TEXT,
                    updated_at TEXT NOT NULL,
                    source_fact_ids_json TEXT NOT NULL,
                    stale INTEGER NOT NULL DEFAULT 0,
                    superseded_by TEXT,
                    belief_json TEXT NOT NULL,
                    embedding BLOB,
                    embedding_identity_json TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_beliefs_partition_subject
                    ON zbot_beliefs(partition_id, subject);
                CREATE INDEX IF NOT EXISTS idx_beliefs_partition_updated
                    ON zbot_beliefs(partition_id, updated_at);
                CREATE TABLE IF NOT EXISTS zbot_contradictions (
                    id TEXT PRIMARY KEY,
                    partition_id TEXT NOT NULL,
                    belief_a_id TEXT NOT NULL,
                    belief_b_id TEXT NOT NULL,
                    detected_at TEXT NOT NULL,
                    resolved_at TEXT,
                    contradiction_json TEXT NOT NULL,
                    UNIQUE(belief_a_id, belief_b_id)
                );
                CREATE INDEX IF NOT EXISTS idx_contradictions_belief_a
                    ON zbot_contradictions(belief_a_id);
                CREATE INDEX IF NOT EXISTS idx_contradictions_belief_b
                    ON zbot_contradictions(belief_b_id);
                CREATE INDEX IF NOT EXISTS idx_contradictions_partition_detected
                    ON zbot_contradictions(partition_id, detected_at);
                "#,
            )
            .map_err(|error| AdapterError::Storage {
                component: SIDECAR_COMPONENT,
                reason: error.to_string(),
            })?;
        ensure_optional_column(
            &connection,
            "zbot_beliefs",
            "embedding_identity_json",
            "TEXT",
        )
        .map_err(|error| AdapterError::Storage {
            component: SIDECAR_COMPONENT,
            reason: error.to_string(),
        })?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            embedding_identity,
        })
    }

    fn store_belief(&self, belief: &Belief, embedding: Option<&[u8]>) -> Result<(), String> {
        let mut stored = belief.clone();
        stored.embedding = None;
        let belief_json = serde_json::to_string(&stored).map_err(|error| error.to_string())?;
        let source_fact_ids_json =
            serde_json::to_string(&belief.source_fact_ids).map_err(|error| error.to_string())?;
        let valid_from = belief.valid_from.map(|timestamp| timestamp.to_rfc3339());
        let valid_until = belief.valid_until.map(|timestamp| timestamp.to_rfc3339());
        self.lock()?
            .execute(
                r#"
                INSERT INTO zbot_beliefs
                    (id, partition_id, subject, valid_from, valid_until, updated_at,
                    source_fact_ids_json, stale, superseded_by, belief_json, embedding,
                    embedding_identity_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT(id) DO UPDATE SET
                    partition_id = excluded.partition_id,
                    subject = excluded.subject,
                    valid_from = excluded.valid_from,
                    valid_until = excluded.valid_until,
                    updated_at = excluded.updated_at,
                    source_fact_ids_json = excluded.source_fact_ids_json,
                    stale = excluded.stale,
                    superseded_by = excluded.superseded_by,
                    belief_json = excluded.belief_json,
                    embedding = COALESCE(excluded.embedding, zbot_beliefs.embedding),
                    embedding_identity_json = COALESCE(excluded.embedding_identity_json, zbot_beliefs.embedding_identity_json)
                "#,
                params![
                    belief.id,
                    belief.partition_id,
                    belief.subject,
                    valid_from,
                    valid_until,
                    belief.updated_at.to_rfc3339(),
                    source_fact_ids_json,
                    i32::from(belief.stale),
                    belief.superseded_by,
                    belief_json,
                    embedding,
                    embedding.map(|_| encode_identity(&self.embedding_identity)),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn find_by_key(
        &self,
        partition_id: &str,
        subject: &str,
        valid_from: Option<DateTime<Utc>>,
    ) -> Result<Option<BeliefEntry>, String> {
        let valid_from = valid_from.map(|timestamp| timestamp.to_rfc3339());
        self.lock()?
            .query_row(
                "SELECT belief_json, embedding, embedding_identity_json FROM zbot_beliefs
                 WHERE partition_id = ?1 AND subject = ?2
                   AND ((valid_from IS NULL AND ?3 IS NULL) OR valid_from = ?3)
                 LIMIT 1",
                params![partition_id, subject, valid_from],
                decode_belief_entry,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn get_belief(
        &self,
        partition_id: &str,
        subject: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Option<Belief>, String> {
        let mut beliefs = self
            .load_all_beliefs()?
            .into_iter()
            .map(|entry| entry.belief)
            .filter(|belief| belief.partition_id == partition_id && belief.subject == subject)
            .filter(|belief| valid_at(belief, as_of))
            .collect::<Vec<_>>();
        beliefs.sort_by(|left, right| {
            right
                .valid_from
                .cmp(&left.valid_from)
                .then_with(|| right.created_at.cmp(&left.created_at))
        });
        Ok(beliefs.into_iter().next())
    }

    fn get_belief_by_id(&self, id: &str) -> Result<Option<Belief>, String> {
        self.lock()?
            .query_row(
                "SELECT belief_json, embedding, embedding_identity_json FROM zbot_beliefs WHERE id = ?1",
                params![id],
                decode_belief_entry,
            )
            .optional()
            .map(|entry| entry.map(|entry| entry.belief))
            .map_err(|error| error.to_string())
    }

    fn embedding_for_belief(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        self.lock()?
            .query_row(
                "SELECT embedding FROM zbot_beliefs WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()
            .map(|value| value.flatten())
            .map_err(|error| error.to_string())
    }

    fn list_beliefs(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String> {
        let mut beliefs = self
            .load_all_beliefs()?
            .into_iter()
            .map(|entry| entry.belief)
            .filter(|belief| belief.partition_id == partition_id)
            .collect::<Vec<_>>();
        beliefs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        beliefs.truncate(limit);
        Ok(beliefs)
    }

    fn list_stale(&self, partition_id: &str, limit: usize) -> Result<Vec<Belief>, String> {
        let mut beliefs = self
            .load_all_beliefs()?
            .into_iter()
            .map(|entry| entry.belief)
            .filter(|belief| belief.partition_id == partition_id && belief.stale)
            .collect::<Vec<_>>();
        beliefs.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
        beliefs.truncate(limit);
        Ok(beliefs)
    }

    fn beliefs_referencing_fact(&self, fact_id: &str) -> Result<Vec<String>, String> {
        let mut ids = self
            .load_all_beliefs()?
            .into_iter()
            .map(|entry| entry.belief)
            .filter(|belief| belief.valid_until.is_none())
            .filter(|belief| belief.source_fact_ids.iter().any(|id| id == fact_id))
            .map(|belief| belief.id)
            .collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }

    fn search_beliefs(
        &self,
        partition_id: &str,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        limit: usize,
    ) -> Result<Vec<ScoredBelief>, String> {
        if !identity_compatible(
            &self.embedding_identity,
            query_identity,
            query_embedding.len(),
        ) {
            return Ok(Vec::new());
        }
        let now = Utc::now();
        let mut scored = self
            .load_all_beliefs()?
            .into_iter()
            .filter(|entry| entry.belief.partition_id == partition_id)
            .filter(|entry| entry.belief.superseded_by.is_none() && valid_at(&entry.belief, now))
            .filter_map(|entry| {
                let embedding = decode_embedding_bytes(entry.belief.embedding.as_deref()?)?;
                if !identity_compatible(
                    &self.embedding_identity,
                    entry.embedding_identity.as_ref(),
                    embedding.len(),
                ) {
                    return None;
                }
                let score = cosine_similarity(query_embedding, &embedding);
                Some(ScoredBelief {
                    belief: entry.belief,
                    score,
                })
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    fn partition_for_belief_pair(&self, a: &str, b: &str) -> Result<Option<String>, String> {
        Ok(self
            .get_belief_by_id(a)?
            .or(self.get_belief_by_id(b)?)
            .map(|belief| belief.partition_id))
    }

    fn store_contradiction(
        &self,
        contradiction: &BeliefContradiction,
        partition_id: &str,
        insert_only: bool,
    ) -> Result<(), String> {
        let contradiction_json =
            serde_json::to_string(contradiction).map_err(|error| error.to_string())?;
        let sql = if insert_only {
            r#"
            INSERT INTO zbot_contradictions
                (id, partition_id, belief_a_id, belief_b_id, detected_at, resolved_at,
                 contradiction_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(belief_a_id, belief_b_id) DO NOTHING
            "#
        } else {
            r#"
            INSERT INTO zbot_contradictions
                (id, partition_id, belief_a_id, belief_b_id, detected_at, resolved_at,
                 contradiction_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                partition_id = excluded.partition_id,
                belief_a_id = excluded.belief_a_id,
                belief_b_id = excluded.belief_b_id,
                detected_at = excluded.detected_at,
                resolved_at = excluded.resolved_at,
                contradiction_json = excluded.contradiction_json
            "#
        };
        self.lock()?
            .execute(
                sql,
                params![
                    contradiction.id,
                    partition_id,
                    contradiction.belief_a_id,
                    contradiction.belief_b_id,
                    contradiction.detected_at.to_rfc3339(),
                    contradiction
                        .resolved_at
                        .map(|timestamp| timestamp.to_rfc3339()),
                    contradiction_json,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn get_contradiction(&self, id: &str) -> Result<Option<BeliefContradiction>, String> {
        self.lock()?
            .query_row(
                "SELECT contradiction_json FROM zbot_contradictions WHERE id = ?1",
                params![id],
                decode_contradiction,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn contradictions_for_belief(
        &self,
        belief_id: &str,
    ) -> Result<Vec<BeliefContradiction>, String> {
        let mut rows = self.load_all_contradictions()?;
        rows.retain(|row| row.belief_a_id == belief_id || row.belief_b_id == belief_id);
        rows.sort_by(|left, right| right.detected_at.cmp(&left.detected_at));
        Ok(rows)
    }

    fn list_recent_contradictions(
        &self,
        partition_id: &str,
        limit: usize,
    ) -> Result<Vec<BeliefContradiction>, String> {
        let mut rows = self
            .load_all_contradictions_with_partition()?
            .into_iter()
            .filter(|(partition, _)| partition == partition_id)
            .map(|(_, contradiction)| contradiction)
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| right.detected_at.cmp(&left.detected_at));
        rows.truncate(limit);
        Ok(rows)
    }

    fn pair_exists(&self, a: &str, b: &str) -> Result<bool, String> {
        let (a, b) = canonical_pair(a, b);
        self.lock()?
            .query_row(
                "SELECT 1 FROM zbot_contradictions WHERE belief_a_id = ?1 AND belief_b_id = ?2",
                params![a, b],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|value| value.is_some())
            .map_err(|error| error.to_string())
    }

    fn load_all_beliefs(&self) -> Result<Vec<BeliefEntry>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT belief_json, embedding, embedding_identity_json FROM zbot_beliefs ORDER BY updated_at DESC")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], decode_belief_entry)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    fn load_all_contradictions(&self) -> Result<Vec<BeliefContradiction>, String> {
        Ok(self
            .load_all_contradictions_with_partition()?
            .into_iter()
            .map(|(_, contradiction)| contradiction)
            .collect())
    }

    fn load_all_contradictions_with_partition(
        &self,
    ) -> Result<Vec<(String, BeliefContradiction)>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT partition_id, contradiction_json FROM zbot_contradictions ORDER BY detected_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let partition_id: String = row.get(0)?;
                let contradiction = decode_contradiction(row)?;
                Ok((partition_id, contradiction))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "belief sidecar connection lock poisoned".to_string())
    }
}

fn decode_belief_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<BeliefEntry> {
    let belief_json: String = row.get(0)?;
    let embedding: Option<Vec<u8>> = row.get(1)?;
    let embedding_identity_json: Option<String> = row.get(2)?;
    let mut belief = serde_json::from_str::<Belief>(&belief_json).map_err(json_sql_error(0))?;
    belief.embedding = embedding;
    let embedding_identity = embedding_identity_json
        .map(|json| decode_identity_json(&json).map_err(json_string_sql_error(2)))
        .transpose()?;
    Ok(BeliefEntry {
        belief,
        embedding_identity,
    })
}

fn decode_contradiction(row: &rusqlite::Row<'_>) -> rusqlite::Result<BeliefContradiction> {
    let contradiction_json: String = row.get(1).or_else(|_| row.get(0))?;
    serde_json::from_str::<BeliefContradiction>(&contradiction_json).map_err(json_sql_error(1))
}

fn json_sql_error(column: usize) -> impl FnOnce(serde_json::Error) -> rusqlite::Error {
    move |error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    }
}

fn json_string_sql_error(column: usize) -> impl FnOnce(String) -> rusqlite::Error {
    move |error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    }
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
    serde_json::json!({
        "providerType": identity.provider_type,
        "model": identity.model,
        "dimensions": identity.dimensions,
        "promptProfile": identity.prompt_profile,
        "normalization": identity.normalization,
    })
    .to_string()
}

fn decode_identity_json(json: &str) -> Result<EmbeddingQueryIdentity, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let dimensions = value
        .get("dimensions")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "missing embedding identity dimensions".to_string())?;
    Ok(EmbeddingQueryIdentity {
        provider_type: value
            .get("providerType")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        model: value
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        dimensions: dimensions as u32,
        prompt_profile: value
            .get("promptProfile")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("query")
            .to_string(),
        normalization: value
            .get("normalization")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    })
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

fn valid_at(belief: &Belief, at: DateTime<Utc>) -> bool {
    belief.valid_from.is_none_or(|valid_from| valid_from <= at)
        && belief
            .valid_until
            .is_none_or(|valid_until| valid_until > at)
}

fn decode_embedding_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let array: [u8; 4] = chunk.try_into().ok()?;
            Some(f32::from_le_bytes(array))
        })
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (dot, left_norm, right_norm) =
        a.iter()
            .zip(b)
            .fold((0.0_f64, 0.0_f64, 0.0_f64), |acc, (left, right)| {
                let left = f64::from(*left);
                let right = f64::from(*right);
                (
                    acc.0 + left * right,
                    acc.1 + left * left,
                    acc.2 + right * right,
                )
            });
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / left_norm.sqrt() / right_norm.sqrt()
}

fn canonicalize_zbot_contradiction(contradiction: &BeliefContradiction) -> BeliefContradiction {
    let (a, b) = canonical_pair(&contradiction.belief_a_id, &contradiction.belief_b_id);
    let mut canonical = contradiction.clone();
    canonical.belief_a_id = a.to_string();
    canonical.belief_b_id = b.to_string();
    canonical
}

fn canonical_pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}
