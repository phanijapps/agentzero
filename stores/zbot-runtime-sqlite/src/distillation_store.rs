use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use zbot_stores_traits::DistillationStore;

use crate::{DistillationRepository, DistillationRun};

/// Trait adapter for operational distillation-run records in conversations.db.
pub struct GatewayDistillationStore {
    repo: Arc<DistillationRepository>,
}

impl GatewayDistillationStore {
    pub fn new(repo: Arc<DistillationRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl DistillationStore for GatewayDistillationStore {
    async fn insert_run(&self, run: Value) -> Result<(), String> {
        let typed: DistillationRun =
            serde_json::from_value(run).map_err(|e| format!("decode DistillationRun: {e}"))?;
        self.repo.insert(&typed)
    }

    async fn get_run_by_session(&self, session_id: &str) -> Result<Option<Value>, String> {
        self.repo
            .get_by_session_id(session_id)?
            .map(|run| serde_json::to_value(run).map_err(|e| e.to_string()))
            .transpose()
    }

    async fn update_retry(&self, session_id: &str) -> Result<(), String> {
        self.repo.update_retry(session_id, "retry", 1, None)
    }

    async fn update_success(
        &self,
        session_id: &str,
        _summary: Option<String>,
    ) -> Result<(), String> {
        self.repo.update_success(session_id, 0, 0, 0, false, 0)
    }

    async fn record_distillation_pending(
        &self,
        session_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.repo.insert(&DistillationRun {
            id: format!("dr-{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_owned(),
            status: status.to_owned(),
            error: error.map(str::to_owned),
            created_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        })
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
        self.repo.update_success(
            session_id,
            facts,
            entities,
            relationships,
            episode_created,
            duration_ms,
        )
    }

    async fn record_distillation_failure(
        &self,
        session_id: &str,
        status: &str,
        retry_count: i32,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.repo
            .update_retry(session_id, status, retry_count, error)
    }
}
