// ============================================================================
// GATEWAY PROCEDURE STORE
// SQLite-backed implementation of the ProcedureStore trait.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::params;
use serde_json::Value;
use zbot_stores_domain::Procedure;
use zbot_stores_traits::{
    PatternProcedureInsert, ProcedureStats, ProcedureStore, ProcedureSummary,
};

use crate::procedure_repository::ProcedureRepository;

pub struct GatewayProcedureStore {
    repo: Arc<ProcedureRepository>,
}

impl GatewayProcedureStore {
    pub fn new(repo: Arc<ProcedureRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl ProcedureStore for GatewayProcedureStore {
    async fn list_by_ward(&self, ward_id: &str, limit: usize) -> Result<Vec<Value>, String> {
        let rows = self.repo.list_by_ward(ward_id, limit)?;
        rows.into_iter()
            .map(|p| serde_json::to_value(p).map_err(|e| e.to_string()))
            .collect()
    }

    async fn upsert_procedure(
        &self,
        procedure: Value,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        let mut typed: Procedure =
            serde_json::from_value(procedure).map_err(|e| format!("decode Procedure: {e}"))?;
        if embedding.is_some() {
            typed.embedding = embedding;
        }
        self.repo.upsert_procedure(&typed)
    }

    async fn search_procedures_by_similarity(
        &self,
        embedding: &[f32],
        agent_id: &str,
        ward_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let scored = self
            .repo
            .search_by_similarity(embedding, agent_id, ward_id, limit)?;
        Ok(scored
            .into_iter()
            .map(|(p, score)| {
                serde_json::json!({
                    "procedure": p,
                    "score": score,
                })
            })
            .collect())
    }

    async fn increment_success(
        &self,
        id: &str,
        duration_ms: Option<i64>,
        token_cost: Option<i64>,
    ) -> Result<(), String> {
        self.repo.increment_success(id, duration_ms, token_cost)
    }

    async fn increment_failure(&self, id: &str) -> Result<(), String> {
        self.repo.increment_failure(id)
    }

    async fn dedupe_procedures_by_name(&self) -> Result<usize, String> {
        self.repo.dedupe_by_name()
    }

    async fn procedure_stats(&self) -> Result<ProcedureStats, String> {
        // ProcedureRepository doesn't expose a global count; defer to default.
        Ok(ProcedureStats::default())
    }

    // ---- Sleep-time pattern extraction (Phase D4) ----------------------

    async fn get_procedure_summary_by_name(
        &self,
        agent_id: &str,
        name: &str,
    ) -> Result<Option<ProcedureSummary>, String> {
        let agent_id = agent_id.to_string();
        let name = name.to_string();
        self.repo.db().with_connection(|conn| {
            let r = conn.query_row(
                "SELECT id, name, success_count
                 FROM procedures
                 WHERE agent_id = ?1 AND name = ?2
                 LIMIT 1",
                params![agent_id, name],
                |row| {
                    Ok(ProcedureSummary {
                        id: row.get::<_, String>(0)?,
                        name: row.get::<_, String>(1)?,
                        success_count: row.get::<_, i32>(2)?,
                    })
                },
            );
            match r {
                Ok(p) => Ok(Some(p)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    async fn get_procedure_by_name(
        &self,
        agent_id: &str,
        name: &str,
    ) -> Result<Option<Procedure>, String> {
        let agent_id = agent_id.to_string();
        let name = name.to_string();
        self.repo.db().with_connection(|conn| {
            let r = conn.query_row(
                "SELECT id, agent_id, ward_id, name, description, trigger_pattern,
                        steps, parameters, success_count, failure_count,
                        avg_duration_ms, avg_token_cost, last_used,
                        created_at, updated_at
                 FROM procedures
                 WHERE agent_id = ?1 AND name = ?2
                 LIMIT 1",
                params![agent_id, name],
                |row| {
                    Ok(Procedure {
                        id: row.get(0)?,
                        agent_id: row.get(1)?,
                        ward_id: row.get(2)?,
                        name: row.get(3)?,
                        description: row.get(4)?,
                        trigger_pattern: row.get(5)?,
                        steps: row.get(6)?,
                        parameters: row.get(7)?,
                        success_count: row.get(8)?,
                        failure_count: row.get(9)?,
                        avg_duration_ms: row.get(10)?,
                        avg_token_cost: row.get(11)?,
                        last_used: row.get(12)?,
                        embedding: None,
                        created_at: row.get(13)?,
                        updated_at: row.get(14)?,
                    })
                },
            );
            match r {
                Ok(p) => Ok(Some(p)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    async fn insert_pattern_procedure(
        &self,
        req: PatternProcedureInsert,
    ) -> Result<String, String> {
        let id = format!("proc-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
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
            embedding: req.embedding,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repo.upsert_procedure(&procedure)?;
        Ok(id)
    }
}
