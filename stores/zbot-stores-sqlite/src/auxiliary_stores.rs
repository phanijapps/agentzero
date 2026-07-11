// ============================================================================
// AUXILIARY STORE IMPLS
// SQLite-backed impls of GoalStore, RecallLogStore, DistillationStore.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use zbot_stores_domain::Goal;
use zbot_stores_traits::{GoalStore, RecallLogStore};

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
