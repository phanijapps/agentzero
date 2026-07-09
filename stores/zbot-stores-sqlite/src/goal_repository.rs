//! CRUD over the `kg_goals` table. Goals are agent intents with lifecycle
//! state (active / blocked / satisfied / abandoned) and decomposition edges.

use crate::KnowledgeDatabase;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

// Goal moved to `zbot-stores-domain` (Phase D5).
pub use zbot_stores_domain::Goal;

/// Repository for `kg_goals` — provides standard CRUD and state-transition helpers.
pub struct GoalRepository {
    db: Arc<KnowledgeDatabase>,
}

impl GoalRepository {
    pub fn new(db: Arc<KnowledgeDatabase>) -> Self {
        Self { db }
    }

    /// Insert a new goal row. `id` must be unique; the caller generates it.
    pub fn create(&self, goal: &Goal) -> Result<String, String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_goals (
                    id, agent_id, ward_id, title, description, state,
                    parent_goal_id, slots, filled_slots,
                    created_at, updated_at, completed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    goal.id,
                    goal.agent_id,
                    goal.ward_id,
                    goal.title,
                    goal.description,
                    goal.state,
                    goal.parent_goal_id,
                    goal.slots,
                    goal.filled_slots,
                    goal.created_at,
                    goal.updated_at,
                    goal.completed_at,
                ],
            )?;
            Ok(goal.id.clone())
        })
    }

    /// Fetch a single goal by ID. Returns `None` if not found.
    pub fn get(&self, goal_id: &str) -> Result<Option<Goal>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, ward_id, title, description, state,
                        parent_goal_id, slots, filled_slots,
                        created_at, updated_at, completed_at
                 FROM kg_goals WHERE id = ?1",
            )?;
            stmt.query_row(params![goal_id], row_to_goal).optional()
        })
    }

    /// Transition a goal to a new state. Terminal states (`satisfied`,
    /// `abandoned`) stamp `completed_at`; non-terminal states leave the
    /// existing `completed_at` value untouched via `COALESCE`.
    pub fn update_state(&self, goal_id: &str, new_state: &str) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        let completed = if new_state == "satisfied" || new_state == "abandoned" {
            Some(now.clone())
        } else {
            None
        };
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE kg_goals
                 SET state = ?1, updated_at = ?2,
                     completed_at = COALESCE(?3, completed_at)
                 WHERE id = ?4",
                params![new_state, now, completed, goal_id],
            )?;
            Ok(())
        })
    }

    /// Persist a JSON snapshot of the filled slots for a goal.
    pub fn update_filled_slots(
        &self,
        goal_id: &str,
        filled_slots_json: &str,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE kg_goals SET filled_slots = ?1, updated_at = ?2 WHERE id = ?3",
                params![filled_slots_json, now, goal_id],
            )?;
            Ok(())
        })
    }

    /// List all goals for an agent that are currently in `active` state,
    /// newest first.
    pub fn list_active(&self, agent_id: &str) -> Result<Vec<Goal>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, ward_id, title, description, state,
                        parent_goal_id, slots, filled_slots,
                        created_at, updated_at, completed_at
                 FROM kg_goals
                 WHERE agent_id = ?1 AND state = 'active'
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map(params![agent_id], row_to_goal)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

fn row_to_goal(row: &rusqlite::Row) -> rusqlite::Result<Goal> {
    Ok(Goal {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        ward_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        state: row.get(5)?,
        parent_goal_id: row.get(6)?,
        slots: row.get(7)?,
        filled_slots: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeDatabase;
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, GoalRepository) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        (tmp, GoalRepository::new(db))
    }

    fn sample_goal(id: &str) -> Goal {
        let now = chrono::Utc::now().to_rfc3339();
        Goal {
            id: id.to_string(),
            agent_id: "root".to_string(),
            ward_id: None,
            title: "test goal".to_string(),
            description: Some("details".to_string()),
            state: "active".to_string(),
            parent_goal_id: None,
            slots: Some(r#"[{"name":"tickers","type":"list"}]"#.to_string()),
            filled_slots: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        }
    }

    #[test]
    fn create_and_get_roundtrip() {
        let (_tmp, repo) = setup();
        let goal = sample_goal("g1");
        repo.create(&goal).unwrap();
        let fetched = repo.get("g1").unwrap().expect("found");
        assert_eq!(fetched.title, "test goal");
        assert_eq!(fetched.state, "active");
    }

    #[test]
    fn get_missing_returns_none() {
        let (_tmp, repo) = setup();
        assert!(repo.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn update_state_marks_completed_at_on_terminal_states() {
        let (_tmp, repo) = setup();

        // Terminal: satisfied
        let goal = sample_goal("g2");
        repo.create(&goal).unwrap();
        repo.update_state("g2", "satisfied").unwrap();
        let fetched = repo.get("g2").unwrap().expect("found");
        assert_eq!(fetched.state, "satisfied");
        assert!(fetched.completed_at.is_some());

        // Terminal: abandoned
        let goal = sample_goal("g2b");
        repo.create(&goal).unwrap();
        repo.update_state("g2b", "abandoned").unwrap();
        let fetched = repo.get("g2b").unwrap().expect("found");
        assert_eq!(fetched.state, "abandoned");
        assert!(fetched.completed_at.is_some());

        // Non-terminal: blocked — completed_at must stay None
        let goal = sample_goal("g3");
        repo.create(&goal).unwrap();
        repo.update_state("g3", "blocked").unwrap();
        let fetched = repo.get("g3").unwrap().expect("found");
        assert_eq!(fetched.state, "blocked");
        assert!(fetched.completed_at.is_none());
    }

    #[test]
    fn list_active_filters_out_satisfied() {
        let (_tmp, repo) = setup();
        let mut goal_a = sample_goal("ga");
        goal_a.title = "alpha".to_string();
        let goal_b = sample_goal("gb");
        repo.create(&goal_a).unwrap();
        repo.create(&goal_b).unwrap();
        repo.update_state("gb", "satisfied").unwrap();

        let active = repo.list_active("root").unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "ga");
    }

    #[test]
    fn update_filled_slots() {
        let (_tmp, repo) = setup();
        let goal = sample_goal("g4");
        repo.create(&goal).unwrap();
        repo.update_filled_slots("g4", r#"{"tickers":["AAPL"]}"#)
            .unwrap();
        let fetched = repo.get("g4").unwrap().expect("found");
        assert_eq!(
            fetched.filled_slots.as_deref(),
            Some(r#"{"tickers":["AAPL"]}"#)
        );
    }
}
