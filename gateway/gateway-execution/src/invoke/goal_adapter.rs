//! # Goal Adapter
//!
//! Bridges [`GoalStore`] to [`agent_tools::GoalAccess`] so the `goal`
//! tool can create/update/list agent goals through the trait surface.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use agent_tools::{GoalAccess, GoalSummary};
use zbot_stores_domain::Goal;
use zbot_stores_traits::GoalStore;

/// Adapter that implements [`GoalAccess`] by delegating to a [`GoalStore`].
pub struct GoalAdapter {
    store: Arc<dyn GoalStore>,
}

impl GoalAdapter {
    pub fn new(store: Arc<dyn GoalStore>) -> Self {
        Self { store }
    }
}

fn to_summary(g: Goal) -> GoalSummary {
    GoalSummary {
        id: g.id,
        title: g.title,
        description: g.description,
        state: g.state,
        slots: g.slots,
        filled_slots: g.filled_slots,
    }
}

fn value_to_summary(v: Value) -> Result<GoalSummary, String> {
    let goal: Goal = serde_json::from_value(v).map_err(|e| format!("decode Goal: {e}"))?;
    Ok(to_summary(goal))
}

#[async_trait]
impl GoalAccess for GoalAdapter {
    async fn create(
        &self,
        agent_id: &str,
        title: &str,
        description: Option<&str>,
        slots_json: Option<&str>,
    ) -> std::result::Result<GoalSummary, String> {
        let now = chrono::Utc::now().to_rfc3339();
        let goal = Goal {
            id: format!("goal-{}", uuid::Uuid::new_v4()),
            agent_id: agent_id.to_string(),
            ward_id: None,
            title: title.to_string(),
            description: description.map(String::from),
            state: "active".to_string(),
            parent_goal_id: None,
            slots: slots_json.map(String::from),
            filled_slots: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        };
        let payload = serde_json::to_value(&goal).map_err(|e| format!("encode Goal: {e}"))?;
        self.store.create_goal(payload).await?;
        Ok(to_summary(goal))
    }

    async fn update_state(
        &self,
        goal_id: &str,
        new_state: &str,
    ) -> std::result::Result<(), String> {
        self.store.update_goal_state(goal_id, new_state).await
    }

    async fn update_filled_slots(
        &self,
        goal_id: &str,
        filled_slots_json: &str,
    ) -> std::result::Result<(), String> {
        self.store
            .update_goal_filled_slots(goal_id, filled_slots_json)
            .await
    }

    async fn list_active(&self, agent_id: &str) -> std::result::Result<Vec<GoalSummary>, String> {
        let rows = self.store.list_active_goals(agent_id).await?;
        rows.into_iter().map(value_to_summary).collect()
    }

    async fn get(&self, goal_id: &str) -> std::result::Result<Option<GoalSummary>, String> {
        match self.store.get_goal(goal_id).await? {
            Some(v) => Ok(Some(value_to_summary(v)?)),
            None => Ok(None),
        }
    }
}
