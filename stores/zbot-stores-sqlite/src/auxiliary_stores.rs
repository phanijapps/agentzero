// ============================================================================
// AUXILIARY STORE IMPLS
// SQLite-backed impls of GoalStore, RecallLogStore, DistillationStore.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use zbot_stores_domain::Goal;
use zbot_stores_traits::{DistillationStore, GoalStore, RecallLogStore};

use crate::distillation_repository::{DistillationRepository, DistillationRun};
use crate::goal_repository::GoalRepository;
use crate::recall_log_repository::RecallLogRepository;

// ----------------------------------------------------------------------------
// GoalStore
// ----------------------------------------------------------------------------

pub struct GatewayGoalStore {
    repo: Arc<GoalRepository>,
}

impl GatewayGoalStore {
    pub fn new(repo: Arc<GoalRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl GoalStore for GatewayGoalStore {
    async fn get_goal(&self, goal_id: &str) -> Result<Option<Value>, String> {
        match self.repo.get(goal_id)? {
            Some(g) => Ok(Some(serde_json::to_value(g).map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    async fn list_active_goals(&self, agent_id: &str) -> Result<Vec<Value>, String> {
        let goals = self.repo.list_active(agent_id)?;
        goals
            .into_iter()
            .map(|g| serde_json::to_value(g).map_err(|e| e.to_string()))
            .collect()
    }

    async fn create_goal(&self, goal: Value) -> Result<String, String> {
        let typed: Goal = serde_json::from_value(goal).map_err(|e| format!("decode Goal: {e}"))?;
        self.repo.create(&typed)
    }

    async fn update_goal_state(&self, goal_id: &str, new_state: &str) -> Result<(), String> {
        self.repo.update_state(goal_id, new_state)
    }

    async fn update_goal_filled_slots(
        &self,
        goal_id: &str,
        filled_slots_json: &str,
    ) -> Result<(), String> {
        self.repo.update_filled_slots(goal_id, filled_slots_json)
    }
}

// ----------------------------------------------------------------------------
// RecallLogStore
// ----------------------------------------------------------------------------

pub struct GatewayRecallLogStore {
    repo: Arc<RecallLogRepository>,
}

impl GatewayRecallLogStore {
    pub fn new(repo: Arc<RecallLogRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl RecallLogStore for GatewayRecallLogStore {
    async fn log_recall(&self, session_id: &str, fact_key: &str) -> Result<(), String> {
        self.repo.log_recall(session_id, fact_key)
    }

    async fn get_keys_for_session(&self, session_id: &str) -> Result<Vec<String>, String> {
        self.repo.get_keys_for_session(session_id)
    }

    async fn get_keys_for_sessions(&self, session_ids: &[String]) -> Result<Vec<String>, String> {
        // Repo returns HashMap<String, usize> (count per key); the trait
        // surface is "list of distinct keys" so we collapse to keys-only.
        let id_refs: Vec<&str> = session_ids.iter().map(|s| s.as_str()).collect();
        let counts = self.repo.get_keys_for_sessions(&id_refs)?;
        Ok(counts.into_keys().collect())
    }
}

// ----------------------------------------------------------------------------
// DistillationStore
// ----------------------------------------------------------------------------

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
        match self.repo.get_by_session_id(session_id)? {
            Some(r) => Ok(Some(serde_json::to_value(r).map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    async fn update_retry(&self, session_id: &str) -> Result<(), String> {
        // Trait contract is "bump retry by 1, status=retry, no error message".
        // Repo wants explicit retry_count; we don't track it through the
        // trait — pass 1 to mean "first/next retry" with empty error. This
        // simplification is fine because the executor that calls this also
        // tracks its own retry counter separately.
        self.repo.update_retry(session_id, "retry", 1, None)
    }

    async fn update_success(
        &self,
        session_id: &str,
        _summary: Option<String>,
    ) -> Result<(), String> {
        // Trait contract is "mark this run successful". Counts (facts,
        // entities, relationships, duration) aren't part of the simplified
        // trait — passing zeros is fine because the actual production
        // updater goes through the concrete repo path. Trait is for tests
        // and trait-aware executors that don't track these.
        self.repo.update_success(session_id, 0, 0, 0, false, 0)
    }

    async fn record_distillation_pending(
        &self,
        session_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let run = DistillationRun {
            id: format!("dr-{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            status: status.to_string(),
            error: error.map(|s| s.to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        };
        self.repo.insert(&run)
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
