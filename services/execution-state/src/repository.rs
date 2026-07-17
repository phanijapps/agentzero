//! # State Repository
//!
//! Database operations for sessions and agent executions.

use crate::types::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// DATABASE PROVIDER TRAIT
// ============================================================================

/// Trait for database connection access.
pub trait StateDbProvider: Send + Sync {
    fn with_connection<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce(&Connection) -> Result<R, rusqlite::Error>;
}

// ============================================================================
// REPOSITORY
// ============================================================================

/// Repository for session and execution operations.
pub struct StateRepository<D: StateDbProvider> {
    db: Arc<D>,
}

impl<D: StateDbProvider> StateRepository<D> {
    pub fn new(db: Arc<D>) -> Self {
        Self { db }
    }

    // =========================================================================
    // SESSION - CREATE
    // =========================================================================

    /// Create a new session.
    pub fn create_session(&self, session: &Session) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sessions (
                    id, status, source, root_agent_id, title,
                    created_at, started_at, completed_at,
                    total_tokens_in, total_tokens_out, metadata,
                    pending_delegations, continuation_needed, ward_id,
                    parent_session_id, thread_id, connector_id, respond_to, mode
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    session.id,
                    session.status.as_str(),
                    session.source.as_str(),
                    session.root_agent_id,
                    session.title,
                    session.created_at,
                    session.started_at,
                    session.completed_at,
                    session.total_tokens_in as i64,
                    session.total_tokens_out as i64,
                    session.metadata.as_ref().and_then(|m| serde_json::to_string(m).ok()),
                    session.pending_delegations as i64,
                    session.continuation_needed as i64,
                    session.ward_id,
                    session.parent_session_id,
                    session.thread_id,
                    session.connector_id,
                    session.respond_to.as_ref().and_then(|r| serde_json::to_string(r).ok()),
                    session.mode,
                ],
            )?;
            Ok(())
        })
    }

    // =========================================================================
    // SESSION - READ
    // =========================================================================

    /// Get a session by ID.
    pub fn get_session(&self, id: &str) -> Result<Option<Session>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, status, source, root_agent_id, title,
                        created_at, started_at, completed_at,
                        total_tokens_in, total_tokens_out, metadata,
                        pending_delegations, continuation_needed, ward_id,
                        parent_session_id, thread_id, connector_id, respond_to, mode
                 FROM sessions WHERE id = ?",
            )?;

            let session = stmt
                .query_row(params![id], |row| Self::row_to_session(row))
                .optional()?;

            Ok(session)
        })
    }

    /// List sessions with filtering.
    pub fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<Session>, String> {
        self.db.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT id, status, source, root_agent_id, title,
                        created_at, started_at, completed_at,
                        total_tokens_in, total_tokens_out, metadata,
                        pending_delegations, continuation_needed, ward_id,
                        parent_session_id, thread_id, connector_id, respond_to, mode
                 FROM sessions WHERE parent_session_id IS NULL",
            );

            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            if let Some(status) = &filter.status {
                sql.push_str(" AND status = ?");
                params_vec.push(Box::new(status.as_str().to_string()));
            }

            if let Some(root_agent_id) = &filter.root_agent_id {
                sql.push_str(" AND root_agent_id = ?");
                params_vec.push(Box::new(root_agent_id.clone()));
            }

            if let Some(from_time) = &filter.from_time {
                sql.push_str(" AND created_at >= ?");
                params_vec.push(Box::new(from_time.clone()));
            }

            if let Some(to_time) = &filter.to_time {
                sql.push_str(" AND created_at <= ?");
                params_vec.push(Box::new(to_time.clone()));
            }

            if let Some(thread_id) = &filter.thread_id {
                sql.push_str(" AND thread_id = ?");
                params_vec.push(Box::new(thread_id.clone()));
            }

            sql.push_str(" ORDER BY created_at DESC");

            if let Some(limit) = filter.limit {
                sql.push_str(&format!(" LIMIT {}", limit));
            } else {
                sql.push_str(" LIMIT 100");
            }

            if let Some(offset) = filter.offset {
                sql.push_str(&format!(" OFFSET {}", offset));
            }

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&sql)?;
            let sessions = stmt
                .query_map(params_refs.as_slice(), |row| Self::row_to_session(row))?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(sessions)
        })
    }

    /// Get session with all its executions.
    pub fn get_session_with_executions(
        &self,
        id: &str,
    ) -> Result<Option<SessionWithExecutions>, String> {
        let session = self.get_session(id)?;

        match session {
            Some(session) => {
                let executions = self.list_executions(&ExecutionFilter {
                    session_id: Some(id.to_string()),
                    ..Default::default()
                })?;

                let subagent_count = executions
                    .iter()
                    .filter(|e| e.delegation_type != DelegationType::Root)
                    .count() as u32;

                Ok(Some(SessionWithExecutions {
                    session,
                    executions,
                    subagent_count,
                }))
            }
            None => Ok(None),
        }
    }

    /// List sessions with their executions (for dashboard).
    pub fn list_sessions_with_executions(
        &self,
        filter: &SessionFilter,
    ) -> Result<Vec<SessionWithExecutions>, String> {
        let sessions = self.list_sessions(filter)?;

        let mut result = Vec::with_capacity(sessions.len());
        for session in sessions {
            let executions = self.list_executions(&ExecutionFilter {
                session_id: Some(session.id.clone()),
                ..Default::default()
            })?;

            let subagent_count = executions
                .iter()
                .filter(|e| e.delegation_type != DelegationType::Root)
                .count() as u32;

            result.push(SessionWithExecutions {
                session,
                executions,
                subagent_count,
            });
        }

        Ok(result)
    }

    /// List bounded Mission Control summary rows without loading logs/messages.
    pub fn list_mission_control_summaries(
        &self,
        filter: &MissionControlFilter,
    ) -> Result<Vec<MissionControlSessionSummary>, String> {
        const DEFAULT_LIMIT: u32 = 50;
        const MAX_LIMIT: u32 = 200;

        let limit = filter.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let sessions = self.list_sessions(&SessionFilter {
            limit: Some(limit),
            offset: filter.offset,
            ..Default::default()
        })?;

        if sessions.is_empty() {
            return Ok(Vec::new());
        }

        let session_ids: Vec<String> = sessions.iter().map(|s| s.id.clone()).collect();
        let executions_by_session = self.list_executions_for_sessions(&session_ids)?;

        let mut summaries = Vec::with_capacity(sessions.len());
        for session in sessions {
            let executions = executions_by_session
                .get(&session.id)
                .cloned()
                .unwrap_or_default();

            let root_execution = executions
                .iter()
                .find(|e| e.delegation_type == DelegationType::Root)
                .or_else(|| executions.iter().find(|e| e.parent_execution_id.is_none()));

            let root_execution_id = root_execution
                .map(|e| e.id.clone())
                .unwrap_or_else(|| session.id.clone());

            let subagent_count = executions
                .iter()
                .filter(|e| e.delegation_type != DelegationType::Root)
                .count() as u32;

            summaries.push(MissionControlSessionSummary {
                conversation_id: session.id,
                root_execution_id,
                status: session.status,
                source: session.source,
                root_agent_id: session.root_agent_id,
                title: session.title,
                created_at: session.created_at,
                started_at: session.started_at,
                completed_at: session.completed_at,
                total_tokens_in: session.total_tokens_in,
                total_tokens_out: session.total_tokens_out,
                subagent_count,
                mode: session.mode,
            });
        }

        Ok(summaries)
    }

    /// Get per-execution token slices for one selected Mission Control session.
    pub fn get_mission_control_session_tokens(
        &self,
        session_id: &str,
    ) -> Result<Option<MissionControlSessionTokens>, String> {
        let Some(session) = self.get_session(session_id)? else {
            return Ok(None);
        };

        let executions = self.list_executions(&ExecutionFilter {
            session_id: Some(session.id.clone()),
            ..Default::default()
        })?;

        let root_execution = executions
            .iter()
            .find(|e| e.delegation_type == DelegationType::Root)
            .or_else(|| executions.iter().find(|e| e.parent_execution_id.is_none()));

        let root_execution_id = root_execution
            .map(|e| e.id.clone())
            .unwrap_or_else(|| session.id.clone());

        let execution_summaries = executions
            .into_iter()
            .map(|e| MissionControlExecutionSummary {
                execution_id: e.id,
                agent_id: e.agent_id,
                delegation_type: e.delegation_type,
                tokens_in: e.tokens_in,
                tokens_out: e.tokens_out,
                child_session_id: e.child_session_id,
            })
            .collect();
        let current_plan = self.get_session_plan(session_id)?;

        Ok(Some(MissionControlSessionTokens {
            conversation_id: session.id,
            root_execution_id,
            total_tokens_in: session.total_tokens_in,
            total_tokens_out: session.total_tokens_out,
            executions: execution_summaries,
            current_plan,
        }))
    }

    /// Validate and persist the latest operational plan for one session.
    ///
    /// Model-owned JSON is validated before opening a database connection. A
    /// database transaction then verifies trusted execution ownership, allocates
    /// the next durable session sequence, and writes the snapshot together.
    pub fn save_session_plan(
        &self,
        session_id: &str,
        execution_id: &str,
        plan: serde_json::Value,
        explanation: Option<String>,
        source_event_timestamp: u64,
    ) -> Result<SessionPlanSaveOutcome, String> {
        let input = match SessionPlanInput::from_update(plan, explanation) {
            Ok(input) => input,
            Err(reason) => return Ok(SessionPlanSaveOutcome::Rejected(reason)),
        };
        let plan_json = serde_json::to_string(&input.plan)
            .map_err(|_| "failed to serialize validated session plan".to_owned())?;
        let source_event_timestamp = i64::try_from(source_event_timestamp)
            .map_err(|_| "source event timestamp exceeds SQLite integer range".to_owned())?;

        self.db.with_connection(|conn| {
            // An immediate transaction serializes concurrent per-session
            // sequence allocation before either writer observes the counter.
            let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
            let execution_belongs_to_session: bool = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM agent_executions
                    WHERE id = ?1 AND session_id = ?2
                )",
                params![execution_id, session_id],
                |row| row.get::<_, i64>(0).map(|count| count != 0),
            )?;
            if !execution_belongs_to_session {
                tx.rollback()?;
                return Ok(SessionPlanSaveOutcome::Rejected(
                    SessionPlanRejection::ExecutionSessionMismatch,
                ));
            }

            tx.execute(
                "INSERT INTO session_plan_counters (session_id, last_issued_sequence)
                 VALUES (?1, 0)
                 ON CONFLICT(session_id) DO NOTHING",
                params![session_id],
            )?;
            tx.execute(
                "UPDATE session_plan_counters
                 SET last_issued_sequence = last_issued_sequence + 1
                 WHERE session_id = ?1",
                params![session_id],
            )?;
            let sequence: i64 = tx.query_row(
                "SELECT last_issued_sequence
                 FROM session_plan_counters
                 WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            let updated_at = chrono::Utc::now().to_rfc3339();
            let rows_written = tx.execute(
                "INSERT INTO session_plans (
                    session_id, execution_id, plan_json, explanation,
                    source_event_timestamp, source_event_sequence, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(session_id) DO UPDATE SET
                    execution_id = excluded.execution_id,
                    plan_json = excluded.plan_json,
                    explanation = excluded.explanation,
                    source_event_timestamp = excluded.source_event_timestamp,
                    source_event_sequence = excluded.source_event_sequence,
                    updated_at = excluded.updated_at
                 WHERE excluded.source_event_sequence > session_plans.source_event_sequence",
                params![
                    session_id,
                    execution_id,
                    plan_json,
                    input.explanation,
                    source_event_timestamp,
                    sequence,
                    updated_at,
                ],
            )?;
            if rows_written == 0 {
                tx.rollback()?;
                return Ok(SessionPlanSaveOutcome::Rejected(
                    SessionPlanRejection::StaleSequence,
                ));
            }
            tx.commit()?;

            Ok(SessionPlanSaveOutcome::Accepted(SessionPlanSnapshot {
                execution_id: execution_id.to_owned(),
                explanation: input.explanation,
                plan: input.plan,
                updated_at,
                source_event_timestamp: u64::try_from(source_event_timestamp)
                    .expect("source timestamp is validated before persistence"),
                source_event_sequence: u64::try_from(sequence)
                    .expect("sequence is non-negative and starts at one"),
            }))
        })
    }

    /// Mark a session's remaining actionable plan steps as completed after a
    /// trusted terminal execution. This is deliberately separate from
    /// `save_session_plan`: it never accepts model-owned JSON and preserves
    /// failed steps as an honest record of the run.
    pub fn complete_session_plan(
        &self,
        session_id: &str,
        execution_id: &str,
    ) -> Result<Option<SessionPlanSnapshot>, String> {
        let source_event_timestamp = u64::try_from(chrono::Utc::now().timestamp_millis())
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?;
        let source_event_timestamp_i64 = i64::try_from(source_event_timestamp)
            .map_err(|_| "source event timestamp exceeds SQLite integer range".to_owned())?;

        self.db.with_connection(|conn| {
            let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
            let execution_belongs_to_session: bool = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM agent_executions
                    WHERE id = ?1 AND session_id = ?2
                )",
                params![execution_id, session_id],
                |row| row.get::<_, i64>(0).map(|count| count != 0),
            )?;
            if !execution_belongs_to_session {
                tx.rollback()?;
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }

            // A new user message can reactivate a session while the preceding
            // turn is unwinding. Never let that prior terminal transition
            // overwrite the active session's newer plan.
            let session_is_completed: bool = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sessions
                    WHERE id = ?1 AND status = 'completed'
                )",
                params![session_id],
                |row| row.get::<_, i64>(0).map(|count| count != 0),
            )?;
            if !session_is_completed {
                tx.rollback()?;
                return Ok(None);
            }

            let snapshot = tx
                .query_row(
                    "SELECT execution_id, plan_json, explanation, source_event_timestamp,
                            source_event_sequence, updated_at
                     FROM session_plans
                     WHERE session_id = ?1",
                    params![session_id],
                    Self::row_to_session_plan,
                )
                .optional()?;
            let Some(mut snapshot) = snapshot else {
                tx.commit()?;
                return Ok(None);
            };

            let mut changed = false;
            for step in &mut snapshot.plan {
                if matches!(
                    step.status,
                    SessionPlanStepStatus::Pending | SessionPlanStepStatus::InProgress
                ) {
                    step.status = SessionPlanStepStatus::Completed;
                    changed = true;
                }
            }
            if !changed {
                tx.commit()?;
                return Ok(None);
            }

            tx.execute(
                "INSERT INTO session_plan_counters (session_id, last_issued_sequence)
                 VALUES (?1, 0)
                 ON CONFLICT(session_id) DO NOTHING",
                params![session_id],
            )?;
            tx.execute(
                "UPDATE session_plan_counters
                 SET last_issued_sequence = last_issued_sequence + 1
                 WHERE session_id = ?1",
                params![session_id],
            )?;
            let sequence: i64 = tx.query_row(
                "SELECT last_issued_sequence
                 FROM session_plan_counters
                 WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            let updated_at = chrono::Utc::now().to_rfc3339();
            let plan_json = serde_json::to_string(&snapshot.plan)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            tx.execute(
                "UPDATE session_plans
                 SET execution_id = ?2,
                     plan_json = ?3,
                     source_event_timestamp = ?4,
                     source_event_sequence = ?5,
                     updated_at = ?6
                 WHERE session_id = ?1",
                params![
                    session_id,
                    execution_id,
                    plan_json,
                    source_event_timestamp_i64,
                    sequence,
                    updated_at,
                ],
            )?;
            tx.commit()?;

            snapshot.execution_id = execution_id.to_owned();
            snapshot.updated_at = updated_at;
            snapshot.source_event_timestamp = source_event_timestamp;
            snapshot.source_event_sequence =
                u64::try_from(sequence).expect("sequence is non-negative and starts at one");
            Ok(Some(snapshot))
        })
    }

    /// Return the current plan for one session, if one has been accepted.
    fn get_session_plan(&self, session_id: &str) -> Result<Option<SessionPlanSnapshot>, String> {
        self.db.with_connection(|conn| {
            let mut statement = conn.prepare(
                "SELECT execution_id, plan_json, explanation, source_event_timestamp,
                        source_event_sequence, updated_at
                 FROM session_plans
                 WHERE session_id = ?1",
            )?;
            statement
                .query_row(params![session_id], Self::row_to_session_plan)
                .optional()
        })
    }

    fn list_executions_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<HashMap<String, Vec<AgentExecution>>, String> {
        if session_ids.is_empty() {
            return Ok(HashMap::new());
        }

        self.db.with_connection(|conn| {
            let placeholders = std::iter::repeat_n("?", session_ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, session_id, agent_id, parent_execution_id,
                        delegation_type, task, status,
                        started_at, completed_at,
                        tokens_in, tokens_out, checkpoint, error, log_path,
                        child_session_id
                 FROM agent_executions
                 WHERE session_id IN ({placeholders})
                 ORDER BY session_id ASC,
                          CASE delegation_type WHEN 'root' THEN 0 ELSE 1 END,
                          started_at ASC"
            );

            let params_refs: Vec<&dyn rusqlite::ToSql> = session_ids
                .iter()
                .map(|id| id as &dyn rusqlite::ToSql)
                .collect();
            let mut stmt = conn.prepare(&sql)?;
            let executions = stmt
                .query_map(params_refs.as_slice(), |row| Self::row_to_execution(row))?
                .collect::<Result<Vec<_>, _>>()?;

            let mut by_session: HashMap<String, Vec<AgentExecution>> = HashMap::new();
            for execution in executions {
                by_session
                    .entry(execution.session_id.clone())
                    .or_default()
                    .push(execution);
            }
            Ok(by_session)
        })
    }

    // =========================================================================
    // SESSION - UPDATE
    // =========================================================================

    /// Update session status.
    /// When session becomes terminal (crashed/completed/cancelled), also updates
    /// all running/queued executions to match.
    pub fn update_session_status(&self, id: &str, status: SessionStatus) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();

        self.db.with_connection(|conn| {
            if status == SessionStatus::Running {
                conn.execute(
                    "UPDATE sessions
                     SET status = ?1, started_at = COALESCE(started_at, ?2)
                     WHERE id = ?3",
                    params![status.as_str(), now, id],
                )?;
            } else if status.is_terminal() {
                conn.execute(
                    "UPDATE sessions SET status = ?1, completed_at = ?2 WHERE id = ?3",
                    params![status.as_str(), now, id],
                )?;

                // Cascade status to running/queued executions
                // - If session crashed, mark running executions as crashed
                // - If session completed, mark running executions as completed
                let exec_status = match status {
                    SessionStatus::Crashed => "crashed",
                    SessionStatus::Completed => "completed",
                    _ => "completed", // fallback for any other terminal state
                };
                conn.execute(
                    "UPDATE agent_executions
                     SET status = ?1, completed_at = ?2
                     WHERE session_id = ?3 AND status IN ('running', 'queued')",
                    params![exec_status, now, id],
                )?;
            } else {
                conn.execute(
                    "UPDATE sessions SET status = ?1 WHERE id = ?2",
                    params![status.as_str(), id],
                )?;
            }
            Ok(())
        })
    }

    /// Update session token totals.
    pub fn update_session_tokens(&self, id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET
                    total_tokens_in = (SELECT COALESCE(SUM(tokens_in), 0) FROM agent_executions WHERE session_id = ?1),
                    total_tokens_out = (SELECT COALESCE(SUM(tokens_out), 0) FROM agent_executions WHERE session_id = ?1)
                 WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }

    /// Update session title.
    pub fn update_session_title(&self, id: &str, title: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET title = ?1 WHERE id = ?2",
                params![title, id],
            )?;
            Ok(())
        })
    }

    /// Update session ward (active project directory).
    pub fn update_session_ward(&self, id: &str, ward_id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET ward_id = ?1 WHERE id = ?2",
                params![ward_id, id],
            )?;
            Ok(())
        })
    }

    /// Atomically bind a ward only when the session has not entered one yet.
    ///
    /// Unlike [`Self::update_session_ward`], this is safe for concurrent
    /// bootstrap calls: a later caller receives the first caller's effective
    /// ward instead of replacing it.
    pub fn claim_session_ward_if_unset(
        &self,
        id: &str,
        ward_id: &str,
    ) -> Result<SessionWardClaim, String> {
        self.db.with_connection(|conn| {
            let claimed = conn.execute(
                "UPDATE sessions SET ward_id = ?1 WHERE id = ?2 AND ward_id IS NULL",
                params![ward_id, id],
            )?;
            if claimed == 1 {
                return Ok(SessionWardClaim::Claimed(ward_id.to_string()));
            }

            let existing = conn
                .query_row(
                    "SELECT ward_id FROM sessions WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten()
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            Ok(SessionWardClaim::Existing(existing))
        })
    }

    /// Update session routing fields (thread_id, connector_id, respond_to).
    pub fn update_session_routing(
        &self,
        id: &str,
        thread_id: Option<&str>,
        connector_id: Option<&str>,
        respond_to: Option<&Vec<String>>,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET thread_id = ?1, connector_id = ?2, respond_to = ?3 WHERE id = ?4",
                params![
                    thread_id,
                    connector_id,
                    respond_to.and_then(|r| serde_json::to_string(r).ok()),
                    id,
                ],
            )?;
            Ok(())
        })
    }

    /// Set the execution mode on a session record.
    pub fn set_session_mode(&self, session_id: &str, mode: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET mode = ?1 WHERE id = ?2",
                params![mode, session_id],
            )?;
            Ok(())
        })
    }

    // =========================================================================
    // SESSION - DELEGATION TRACKING
    // =========================================================================

    /// Increment pending delegations count.
    pub fn increment_pending_delegations(&self, session_id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET pending_delegations = pending_delegations + 1 WHERE id = ?1",
                params![session_id],
            )?;
            Ok(())
        })
    }

    /// Decrement pending delegations count, returns new count.
    pub fn decrement_pending_delegations(&self, session_id: &str) -> Result<u32, String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET pending_delegations = MAX(0, pending_delegations - 1) WHERE id = ?1",
                params![session_id],
            )?;
            let count: i64 = conn.query_row(
                "SELECT pending_delegations FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(count as u32)
        })
    }

    /// Reset both delegation bookkeeping fields to zero/false.
    ///
    /// Called when in-flight delegations are abandoned (user-initiated
    /// stop, session reactivation): the pending counts and the
    /// `continuation_needed` flag from the prior turn are stale because
    /// the executions they referred to have been cancelled.
    pub fn reset_delegation_tracking(&self, session_id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET pending_delegations = 0, continuation_needed = 0 WHERE id = ?1",
                params![session_id],
            )?;
            Ok(())
        })
    }

    /// Set continuation_needed flag.
    pub fn set_continuation_needed(&self, session_id: &str, needed: bool) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET continuation_needed = ?1 WHERE id = ?2",
                params![needed as i32, session_id],
            )?;
            Ok(())
        })
    }

    // =========================================================================
    // SESSION - DELETE
    // =========================================================================

    /// Delete a session (cascades to executions and messages).
    pub fn delete_session(&self, id: &str) -> Result<bool, String> {
        self.db.with_connection(|conn| {
            let count = conn.execute("DELETE FROM sessions WHERE id = ?", params![id])?;
            Ok(count > 0)
        })
    }

    /// Delete a session and all per-session rows in a single transaction.
    ///
    /// **Deletes**: `sessions`, `messages`, `agent_executions`, `execution_logs`,
    /// `artifacts` (DB pointers only — files on disk in ward dirs are kept),
    /// `distillation_runs`, `bridge_outbox`, `recall_log`.
    ///
    /// **Preserves**: `memory_facts`, `memory_facts_index` (vec0 embeddings),
    /// knowledge graph tables (`kg_entities`, `kg_relationships`, etc.), and
    /// all files on disk under `~/Documents/zbot/wards/...`. Users rely on
    /// cross-session memory surviving a single-session cleanup.
    ///
    /// All child tables use `session_id` as their FK column. `execution_logs`
    /// also carries a `parent_session_id` column for parent-session linkage,
    /// but `session_id` is the owner column so we only match on that. The
    /// `agent_executions.child_session_id` column is NOT matched here — the
    /// caller is expected to invoke this method per session (child sessions
    /// are separate calls made by the UI).
    ///
    /// Runs every `DELETE` inside one transaction via `unchecked_transaction()`
    /// (the `with_connection` trait hands us `&Connection`, not `&mut`). Any
    /// error aborts the transaction and rolls back cleanly.
    ///
    /// Returns the total number of rows deleted across all cascaded tables
    /// (informational; primarily for logging and tests). Deleting a non-existent
    /// session returns `Ok(0)` — callers that need a presence check should
    /// query first.
    pub fn delete_session_cascade(&self, session_id: &str) -> Result<usize, String> {
        self.db.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut total = 0usize;
            // Child tables keyed by session_id. Order doesn't matter inside a
            // transaction; we still delete child rows before the sessions row
            // so FK checks (if enabled) stay happy step-by-step.
            total += tx.execute(
                "DELETE FROM messages WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute(
                "DELETE FROM agent_executions WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute(
                "DELETE FROM execution_logs WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute(
                "DELETE FROM artifacts WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute(
                "DELETE FROM distillation_runs WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute(
                "DELETE FROM bridge_outbox WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute(
                "DELETE FROM recall_log WHERE session_id = ?",
                params![session_id],
            )?;
            total += tx.execute("DELETE FROM sessions WHERE id = ?", params![session_id])?;
            // Intentionally NOT touched: memory_facts, memory_facts_index,
            // kg_entities, kg_relationships, kg_causal_edges. Those are
            // cross-session state the user expects to survive this delete.
            tx.commit()?;
            Ok(total)
        })
    }

    /// Delete a session **and all of its descendant subagent sessions**, plus
    /// all per-session conversation rows for every session in the tree.
    ///
    /// Walks `sessions.parent_session_id` recursively to collect the full
    /// subtree rooted at `root_session_id`, then runs the same per-table
    /// `DELETE` set as [`Self::delete_session_cascade`] but matched against
    /// the entire descendant set. Memory tables (`memory_facts`, knowledge
    /// graph) are preserved by the same invariant.
    ///
    /// Idempotent for orphans: even when no `sessions` row exists for
    /// `root_session_id` (e.g. legacy rows that only ever made it into
    /// `execution_logs.conversation_id`), the literal id is still placed
    /// into the descendant set so per-table cleanup runs and the row
    /// disappears from the UI's `/api/logs/sessions` view.
    ///
    /// `execution_logs` is matched against **both** its `session_id` (which
    /// in production holds the `exec-…` execution id) and `conversation_id`
    /// (which holds the `sess-…` session id the rest of the system uses).
    /// Without the second column, orphan sessions never get cleaned and the
    /// historical cascade silently no-opped on this table.
    ///
    /// Returns the total number of rows deleted across all tables.
    pub fn delete_session_recursive_cascade(&self, root_session_id: &str) -> Result<usize, String> {
        self.db.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;

            // Materialize the descendant set into a temp table once, then
            // reuse it across every per-table delete. Avoids re-walking the
            // recursive CTE eight times and sidesteps the 999-bound-param
            // ceiling on `IN (?, ?, ...)`.
            tx.execute("DROP TABLE IF EXISTS _delete_session_set", [])?;
            tx.execute(
                "CREATE TEMP TABLE _delete_session_set (id TEXT PRIMARY KEY)",
                [],
            )?;
            // Always seed with the literal root id (whether or not it has a
            // `sessions` row) so orphan log entries with a matching
            // conversation_id still get cleaned. Recursive walk via
            // `sessions.parent_session_id` appends real subagent descendants.
            tx.execute(
                "WITH RECURSIVE descendants(id) AS (
                    SELECT ? AS id
                    UNION
                    SELECT s.id FROM sessions s
                    INNER JOIN descendants d ON s.parent_session_id = d.id
                 )
                 INSERT INTO _delete_session_set(id) SELECT id FROM descendants",
                params![root_session_id],
            )?;
            // Continuation turns (see runner::invoke_continuation) emit log
            // rows under a derived conversation_id of the form
            // `<sess>-cont-<8hex>`. They carry no `sessions` row of their
            // own — they're log artifacts of an existing session. Pull every
            // such derived id (rooted at any session in the descendant set)
            // into the delete set so a single cascade clears the whole
            // continuation history, not just one turn.
            tx.execute(
                "INSERT OR IGNORE INTO _delete_session_set(id) \
                 SELECT DISTINCT el.conversation_id \
                 FROM execution_logs el \
                 JOIN _delete_session_set d \
                   ON el.conversation_id LIKE d.id || '-cont-%'",
                [],
            )?;

            let mut total = 0usize;
            // Tables keyed exclusively by `session_id`. Order matters only
            // when FK enforcement is on: child rows first, sessions row last.
            for table in [
                "messages",
                "agent_executions",
                "artifacts",
                "distillation_runs",
                "bridge_outbox",
                "recall_log",
            ] {
                let sql = format!(
                    "DELETE FROM {} WHERE session_id IN (SELECT id FROM _delete_session_set)",
                    table
                );
                total += tx.execute(&sql, [])?;
            }
            // execution_logs uses both columns: `session_id` carries the
            // execution id, `conversation_id` carries the actual session id.
            // Match either so legacy orphan logs (only conversation_id set)
            // are cleaned alongside current rows.
            total += tx.execute(
                "DELETE FROM execution_logs \
                 WHERE session_id IN (SELECT id FROM _delete_session_set) \
                    OR conversation_id IN (SELECT id FROM _delete_session_set)",
                [],
            )?;
            total += tx.execute(
                "DELETE FROM sessions WHERE id IN (SELECT id FROM _delete_session_set)",
                [],
            )?;

            tx.execute("DROP TABLE _delete_session_set", [])?;
            tx.commit()?;
            Ok(total)
        })
    }

    /// Delete sessions older than a given timestamp.
    /// Returns the number of deleted sessions.
    pub fn delete_old_sessions(&self, older_than: &str) -> Result<u64, String> {
        self.db.with_connection(|conn| {
            // Use created_at since started_at can be NULL
            let count = conn.execute(
                "DELETE FROM sessions WHERE created_at < ?",
                params![older_than],
            )?;
            Ok(count as u64)
        })
    }

    // =========================================================================
    // EXECUTION - CREATE
    // =========================================================================

    /// Create a new agent execution.
    pub fn create_execution(&self, execution: &AgentExecution) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_executions (
                    id, session_id, agent_id, parent_execution_id,
                    delegation_type, task, status,
                    started_at, completed_at,
                    tokens_in, tokens_out, checkpoint, error, log_path,
                    child_session_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    execution.id,
                    execution.session_id,
                    execution.agent_id,
                    execution.parent_execution_id,
                    execution.delegation_type.as_str(),
                    execution.task,
                    execution.status.as_str(),
                    execution.started_at,
                    execution.completed_at,
                    execution.tokens_in as i64,
                    execution.tokens_out as i64,
                    execution
                        .checkpoint
                        .as_ref()
                        .and_then(|c| serde_json::to_string(c).ok()),
                    execution.error,
                    execution.log_path,
                    execution.child_session_id,
                ],
            )?;
            Ok(())
        })
    }

    // =========================================================================
    // EXECUTION - READ
    // =========================================================================

    /// Get an execution by ID.
    pub fn get_execution(&self, id: &str) -> Result<Option<AgentExecution>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, agent_id, parent_execution_id,
                        delegation_type, task, status,
                        started_at, completed_at,
                        tokens_in, tokens_out, checkpoint, error, log_path,
                        child_session_id
                 FROM agent_executions WHERE id = ?",
            )?;

            let execution = stmt
                .query_row(params![id], |row| Self::row_to_execution(row))
                .optional()?;

            Ok(execution)
        })
    }

    /// List executions with filtering.
    pub fn list_executions(&self, filter: &ExecutionFilter) -> Result<Vec<AgentExecution>, String> {
        self.db.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT id, session_id, agent_id, parent_execution_id,
                        delegation_type, task, status,
                        started_at, completed_at,
                        tokens_in, tokens_out, checkpoint, error, log_path,
                        child_session_id
                 FROM agent_executions WHERE 1=1",
            );

            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            if let Some(session_id) = &filter.session_id {
                sql.push_str(" AND session_id = ?");
                params_vec.push(Box::new(session_id.clone()));
            }

            if let Some(agent_id) = &filter.agent_id {
                sql.push_str(" AND agent_id = ?");
                params_vec.push(Box::new(agent_id.clone()));
            }

            if let Some(status) = &filter.status {
                sql.push_str(" AND status = ?");
                params_vec.push(Box::new(status.as_str().to_string()));
            }

            if let Some(parent_id) = &filter.parent_execution_id {
                sql.push_str(" AND parent_execution_id = ?");
                params_vec.push(Box::new(parent_id.clone()));
            }

            sql.push_str(" ORDER BY started_at ASC");

            if let Some(limit) = filter.limit {
                sql.push_str(&format!(" LIMIT {}", limit));
            }

            if let Some(offset) = filter.offset {
                sql.push_str(&format!(" OFFSET {}", offset));
            }

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&sql)?;
            let executions = stmt
                .query_map(params_refs.as_slice(), |row| Self::row_to_execution(row))?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(executions)
        })
    }

    /// Get child executions for a parent.
    pub fn get_child_executions(
        &self,
        parent_execution_id: &str,
    ) -> Result<Vec<AgentExecution>, String> {
        self.list_executions(&ExecutionFilter {
            parent_execution_id: Some(parent_execution_id.to_string()),
            ..Default::default()
        })
    }

    /// Get root execution for a session.
    pub fn get_root_execution(&self, session_id: &str) -> Result<Option<AgentExecution>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, agent_id, parent_execution_id,
                        delegation_type, task, status,
                        started_at, completed_at,
                        tokens_in, tokens_out, checkpoint, error, log_path,
                        child_session_id
                 FROM agent_executions
                 WHERE session_id = ? AND delegation_type = 'root'",
            )?;

            let execution = stmt
                .query_row(params![session_id], |row| Self::row_to_execution(row))
                .optional()?;

            Ok(execution)
        })
    }

    // =========================================================================
    // EXECUTION - UPDATE
    // =========================================================================

    /// Update execution status.
    pub fn update_execution_status(&self, id: &str, status: ExecutionStatus) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();

        self.db.with_connection(|conn| {
            if status == ExecutionStatus::Running {
                conn.execute(
                    "UPDATE agent_executions
                     SET status = ?1, started_at = COALESCE(started_at, ?2)
                     WHERE id = ?3",
                    params![status.as_str(), now, id],
                )?;
            } else if status.is_terminal() {
                conn.execute(
                    "UPDATE agent_executions SET status = ?1, completed_at = ?2 WHERE id = ?3",
                    params![status.as_str(), now, id],
                )?;
            } else {
                conn.execute(
                    "UPDATE agent_executions SET status = ?1 WHERE id = ?2",
                    params![status.as_str(), id],
                )?;
            }
            Ok(())
        })
    }

    /// Cancel a single execution by marking it as cancelled with a completed_at timestamp.
    pub fn cancel_execution(&self, execution_id: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE agent_executions SET status = 'cancelled', completed_at = ?1 WHERE id = ?2",
                params![chrono::Utc::now().to_rfc3339(), execution_id],
            )?;
            Ok(())
        })
    }

    /// Update execution tokens.
    pub fn update_execution_tokens(
        &self,
        id: &str,
        tokens_in: u64,
        tokens_out: u64,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE agent_executions SET tokens_in = ?1, tokens_out = ?2 WHERE id = ?3",
                params![tokens_in as i64, tokens_out as i64, id],
            )?;
            tx.execute(
                "UPDATE sessions
                 SET
                    total_tokens_in = (
                        SELECT COALESCE(SUM(tokens_in), 0)
                        FROM agent_executions
                        WHERE session_id = (
                            SELECT session_id FROM agent_executions WHERE id = ?1
                        )
                    ),
                    total_tokens_out = (
                        SELECT COALESCE(SUM(tokens_out), 0)
                        FROM agent_executions
                        WHERE session_id = (
                            SELECT session_id FROM agent_executions WHERE id = ?1
                        )
                    )
                 WHERE id = (
                    SELECT session_id FROM agent_executions WHERE id = ?1
                 )",
                params![id],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Save execution checkpoint.
    pub fn save_execution_checkpoint(
        &self,
        id: &str,
        checkpoint: &Checkpoint,
    ) -> Result<(), String> {
        let json = serde_json::to_string(checkpoint)
            .map_err(|e| format!("Failed to serialize checkpoint: {}", e))?;

        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE agent_executions SET checkpoint = ?1 WHERE id = ?2",
                params![json, id],
            )?;
            Ok(())
        })
    }

    /// Set child_session_id on an execution (for smart resume).
    pub fn set_child_session_id(
        &self,
        execution_id: &str,
        child_session_id: &str,
    ) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE agent_executions SET child_session_id = ?1 WHERE id = ?2",
                params![child_session_id, execution_id],
            )?;
            Ok(())
        })
    }

    /// Find the most recently crashed subagent execution for a session.
    /// Returns None if only the root execution crashed or no crashes exist.
    pub fn get_last_crashed_subagent(
        &self,
        session_id: &str,
    ) -> Result<Option<AgentExecution>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, agent_id, parent_execution_id,
                        delegation_type, task, status,
                        started_at, completed_at,
                        tokens_in, tokens_out, checkpoint, error, log_path, child_session_id
                 FROM agent_executions
                 WHERE session_id = ? AND status = 'crashed' AND parent_execution_id IS NOT NULL
                 ORDER BY started_at DESC
                 LIMIT 1",
            )?;

            let execution = stmt
                .query_row(params![session_id], |row| Self::row_to_execution(row))
                .optional()?;

            Ok(execution)
        })
    }

    /// Set execution error.
    pub fn set_execution_error(&self, id: &str, error: &str) -> Result<(), String> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE agent_executions SET error = ?1 WHERE id = ?2",
                params![error, id],
            )?;
            Ok(())
        })
    }

    // =========================================================================
    // STATS - DASHBOARD
    // =========================================================================

    /// Get dashboard stats (pre-computed, ready to display).
    pub fn get_dashboard_stats(&self) -> Result<DashboardStats, String> {
        self.db.with_connection(|conn| {
            // =====================================================================
            // SESSION COUNTS BY STATUS
            // =====================================================================
            let mut stmt = conn.prepare(
                "SELECT status, COUNT(*) FROM sessions WHERE parent_session_id IS NULL GROUP BY status",
            )?;

            let mut sessions_queued = 0u64;
            let mut sessions_running = 0u64;
            let mut sessions_paused = 0u64;
            let mut sessions_completed = 0u64;
            let mut sessions_crashed = 0u64;

            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?;

            for row in rows {
                let (status, count) = row?;
                match status.as_str() {
                    "queued" => sessions_queued = count,
                    "running" => sessions_running = count,
                    "paused" => sessions_paused = count,
                    "completed" => sessions_completed = count,
                    "crashed" => sessions_crashed = count,
                    _ => {}
                }
            }

            // =====================================================================
            // EXECUTION COUNTS BY STATUS
            // Only count executions from sessions that are still running/queued
            // (executions from crashed/completed sessions are not truly "running")
            // =====================================================================
            let mut stmt = conn.prepare(
                "SELECT e.status, COUNT(*)
                 FROM agent_executions e
                 JOIN sessions s ON e.session_id = s.id
                 WHERE s.status IN ('running', 'queued')
                 GROUP BY e.status",
            )?;

            let mut executions_queued = 0u64;
            let mut executions_running = 0u64;
            let mut executions_completed = 0u64;
            let mut executions_crashed = 0u64;
            let mut executions_cancelled = 0u64;

            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?;

            for row in rows {
                let (status, count) = row?;
                match status.as_str() {
                    "queued" => executions_queued = count,
                    "running" => executions_running = count,
                    "completed" => executions_completed = count,
                    "crashed" => executions_crashed = count,
                    "cancelled" => executions_cancelled = count,
                    _ => {}
                }
            }

            // =====================================================================
            // TODAY'S STATS
            // =====================================================================
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let mut stmt = conn.prepare(
                "SELECT COUNT(*), COALESCE(SUM(total_tokens_in + total_tokens_out), 0)
                 FROM sessions WHERE parent_session_id IS NULL AND DATE(created_at) = ?",
            )?;

            let (today_sessions, today_tokens) = stmt.query_row(params![today], |row| {
                Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64))
            })?;

            // =====================================================================
            // SESSIONS BY SOURCE
            // =====================================================================
            let mut stmt = conn.prepare(
                "SELECT source, COUNT(*) FROM sessions WHERE parent_session_id IS NULL GROUP BY source",
            )?;

            let mut sessions_by_source = std::collections::HashMap::new();
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?;

            for row in rows {
                let (source, count) = row?;
                sessions_by_source.insert(source, count);
            }

            Ok(DashboardStats {
                sessions_queued,
                sessions_running,
                sessions_paused,
                sessions_completed,
                sessions_crashed,
                executions_queued,
                executions_running,
                executions_completed,
                executions_crashed,
                executions_cancelled,
                today_sessions,
                today_tokens,
                sessions_by_source,
            })
        })
    }

    // =========================================================================
    // SESSION MESSAGES
    // =========================================================================

    /// Get messages for a session with scope filtering.
    ///
    /// Joins messages with executions to include agent_id and delegation_type.
    ///
    /// Scopes:
    /// - `all`: All messages from all executions
    /// - `root`: Only messages from root executions
    /// - `execution`: Messages from a specific execution
    /// - `delegates`: Only messages from delegated executions
    pub fn get_session_messages(
        &self,
        session_id: &str,
        scope: &str,
        execution_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<crate::handlers::SessionMessage>, String> {
        self.db.with_connection(|conn| {
            // Base query joining messages with executions
            let mut sql = String::from(
                "SELECT m.id, m.execution_id, e.agent_id, e.delegation_type,
                        m.role, m.content, m.created_at, m.tool_calls, m.tool_call_id
                 FROM messages m
                 JOIN agent_executions e ON m.execution_id = e.id
                 WHERE e.session_id = ?1",
            );

            // Apply scope filter
            match scope {
                "root" => sql.push_str(" AND e.delegation_type = 'root'"),
                "execution" => sql.push_str(" AND e.id = ?2"),
                "delegates" => sql.push_str(" AND e.delegation_type != 'root'"),
                // "all" - no additional filter
                _ => {}
            }

            // Apply agent_id filter if provided
            if agent_id.is_some() {
                if scope == "execution" {
                    sql.push_str(" AND e.agent_id = ?3");
                } else {
                    sql.push_str(" AND e.agent_id = ?2");
                }
            }

            // Order by execution start time, then message created_at
            sql.push_str(" ORDER BY e.started_at ASC, m.created_at ASC");

            let mut stmt = conn.prepare(&sql)?;

            // Build params based on scope and filters
            let messages: Vec<crate::handlers::SessionMessage> =
                match (scope, execution_id, agent_id) {
                    ("execution", Some(exec_id), Some(agent)) => stmt
                        .query_map(
                            params![session_id, exec_id, agent],
                            Self::row_to_session_message,
                        )?
                        .filter_map(|r| r.ok())
                        .collect(),
                    ("execution", Some(exec_id), None) => stmt
                        .query_map(params![session_id, exec_id], Self::row_to_session_message)?
                        .filter_map(|r| r.ok())
                        .collect(),
                    (_, _, Some(agent)) => stmt
                        .query_map(params![session_id, agent], Self::row_to_session_message)?
                        .filter_map(|r| r.ok())
                        .collect(),
                    _ => stmt
                        .query_map(params![session_id], Self::row_to_session_message)?
                        .filter_map(|r| r.ok())
                        .collect(),
                };

            Ok(messages)
        })
    }

    // =========================================================================
    // HELPERS
    // =========================================================================

    fn row_to_session(row: &rusqlite::Row) -> Result<Session, rusqlite::Error> {
        let status_str: String = row.get(1)?;
        let source_str: String = row.get(2)?;
        let metadata_json: Option<String> = row.get(10)?;
        let respond_to_json: Option<String> = row.get(17).ok().flatten();

        Ok(Session {
            id: row.get(0)?,
            status: status_str.parse().unwrap_or(SessionStatus::Running),
            source: source_str.parse().unwrap_or(TriggerSource::Web),
            root_agent_id: row.get(3)?,
            title: row.get(4)?,
            created_at: row.get(5)?,
            started_at: row.get(6)?,
            completed_at: row.get(7)?,
            total_tokens_in: row.get::<_, i64>(8)? as u64,
            total_tokens_out: row.get::<_, i64>(9)? as u64,
            metadata: metadata_json.and_then(|s| serde_json::from_str(&s).ok()),
            pending_delegations: row.get::<_, i64>(11).unwrap_or(0) as u32,
            continuation_needed: row.get::<_, i64>(12).unwrap_or(0) != 0,
            ward_id: row.get(13).ok().flatten(),
            parent_session_id: row.get(14).ok().flatten(),
            thread_id: row.get(15).ok().flatten(),
            connector_id: row.get(16).ok().flatten(),
            respond_to: respond_to_json.and_then(|s| serde_json::from_str(&s).ok()),
            mode: row.get(18).ok().flatten(),
        })
    }

    fn row_to_execution(row: &rusqlite::Row) -> Result<AgentExecution, rusqlite::Error> {
        let delegation_type_str: String = row.get(4)?;
        let status_str: String = row.get(6)?;
        let checkpoint_json: Option<String> = row.get(11)?;

        Ok(AgentExecution {
            id: row.get(0)?,
            session_id: row.get(1)?,
            agent_id: row.get(2)?,
            parent_execution_id: row.get(3)?,
            delegation_type: delegation_type_str.parse().unwrap_or(DelegationType::Root),
            task: row.get(5)?,
            status: status_str.parse().unwrap_or(ExecutionStatus::Queued),
            started_at: row.get(7)?,
            completed_at: row.get(8)?,
            tokens_in: row.get::<_, i64>(9)? as u64,
            tokens_out: row.get::<_, i64>(10)? as u64,
            checkpoint: checkpoint_json.and_then(|s| serde_json::from_str(&s).ok()),
            error: row.get(12)?,
            log_path: row.get(13)?,
            child_session_id: row.get(14)?,
        })
    }

    // =========================================================================
    // ARTIFACTS
    // =========================================================================

    /// Maximum user-facing goal artifacts retained in one session.
    pub const MAX_GOAL_ARTIFACTS_PER_SESSION: i64 = 24;

    /// Insert a new artifact record. Goal-artifact admission is an atomic
    /// insert-with-count predicate, so concurrent executions cannot exceed the
    /// per-session deliverable budget.
    pub fn create_artifact(&self, artifact: &Artifact) -> Result<(), String> {
        self.db.with_connection(|conn| {
            let inserted = conn.execute(
                "INSERT INTO artifacts (
                    id, session_id, ward_id, execution_id, agent_id,
                    file_path, file_name, file_type, file_size, label,
                    is_goal_artifact, created_at
                )
                SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12
                WHERE ?11 = 0 OR (
                    SELECT COUNT(*) FROM artifacts
                    WHERE session_id = ?2 AND is_goal_artifact = 1
                ) < ?13",
                params![
                    artifact.id,
                    artifact.session_id,
                    artifact.ward_id,
                    artifact.execution_id,
                    artifact.agent_id,
                    artifact.file_path,
                    artifact.file_name,
                    artifact.file_type,
                    artifact.file_size,
                    artifact.label,
                    artifact.is_goal_artifact,
                    artifact.created_at,
                    Self::MAX_GOAL_ARTIFACTS_PER_SESSION,
                ],
            )?;
            if inserted == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
    }

    /// List all artifacts for a session, ordered by creation time.
    pub fn list_artifacts_by_session(&self, session_id: &str) -> Result<Vec<Artifact>, String> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, ward_id, execution_id, agent_id,
                        file_path, file_name, file_type, file_size, label,
                        is_goal_artifact, created_at
                 FROM artifacts
                 WHERE session_id = ?1
                 ORDER BY created_at",
            )?;
            let rows = stmt
                .query_map(params![session_id], |row| {
                    Ok(Artifact {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        ward_id: row.get(2)?,
                        execution_id: row.get(3)?,
                        agent_id: row.get(4)?,
                        file_path: row.get(5)?,
                        file_name: row.get(6)?,
                        file_type: row.get(7)?,
                        file_size: row.get(8)?,
                        label: row.get(9)?,
                        is_goal_artifact: row.get(10)?,
                        created_at: row.get(11)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// List the bounded, explicitly designated goal artifacts for a session.
    /// This is intentionally separate from the full manifest so callers must
    /// opt into the user-facing deliverables projection.
    pub fn list_goal_artifacts_by_session(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<Artifact>, String> {
        let limit = limit.clamp(1, Self::MAX_GOAL_ARTIFACTS_PER_SESSION as u32);
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, ward_id, execution_id, agent_id,
                        file_path, file_name, file_type, file_size, label,
                        is_goal_artifact, created_at
                 FROM artifacts
                 WHERE session_id = ?1 AND is_goal_artifact = 1
                 ORDER BY created_at
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![session_id, limit], |row| {
                    Ok(Artifact {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        ward_id: row.get(2)?,
                        execution_id: row.get(3)?,
                        agent_id: row.get(4)?,
                        file_path: row.get(5)?,
                        file_name: row.get(6)?,
                        file_type: row.get(7)?,
                        file_size: row.get(8)?,
                        label: row.get(9)?,
                        is_goal_artifact: row.get(10)?,
                        created_at: row.get(11)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Get a single artifact by ID.
    pub fn get_artifact(&self, artifact_id: &str) -> Result<Option<Artifact>, String> {
        self.db.with_connection(|conn| {
            let result = conn
                .query_row(
                    "SELECT id, session_id, ward_id, execution_id, agent_id,
                            file_path, file_name, file_type, file_size, label,
                            is_goal_artifact, created_at
                     FROM artifacts
                     WHERE id = ?1",
                    params![artifact_id],
                    |row| {
                        Ok(Artifact {
                            id: row.get(0)?,
                            session_id: row.get(1)?,
                            ward_id: row.get(2)?,
                            execution_id: row.get(3)?,
                            agent_id: row.get(4)?,
                            file_path: row.get(5)?,
                            file_name: row.get(6)?,
                            file_type: row.get(7)?,
                            file_size: row.get(8)?,
                            label: row.get(9)?,
                            is_goal_artifact: row.get(10)?,
                            created_at: row.get(11)?,
                        })
                    },
                )
                .optional()?;
            Ok(result)
        })
    }

    fn row_to_session_message(
        row: &rusqlite::Row,
    ) -> Result<crate::handlers::SessionMessage, rusqlite::Error> {
        let tool_calls_json: Option<String> = row.get(7)?;
        let tool_call_id: Option<String> = row.get(8)?;

        Ok(crate::handlers::SessionMessage {
            id: row.get(0)?,
            execution_id: row.get(1)?,
            agent_id: row.get(2)?,
            delegation_type: row.get(3)?,
            role: row.get(4)?,
            content: row.get(5)?,
            created_at: row.get(6)?,
            tool_calls: tool_calls_json.and_then(|s| serde_json::from_str(&s).ok()),
            tool_call_id,
            tool_results: None,
        })
    }

    fn row_to_session_plan(row: &rusqlite::Row) -> Result<SessionPlanSnapshot, rusqlite::Error> {
        use rusqlite::types::Type;

        let plan_json: String = row.get(1)?;
        let plan = serde_json::from_str(&plan_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(1, Type::Text, Box::new(error))
        })?;
        let source_event_timestamp: i64 = row.get(3)?;
        let source_event_sequence: i64 = row.get(4)?;
        Ok(SessionPlanSnapshot {
            execution_id: row.get(0)?,
            plan,
            explanation: row.get(2)?,
            source_event_timestamp: u64::try_from(source_event_timestamp).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(3, Type::Integer, Box::new(error))
            })?,
            source_event_sequence: u64::try_from(source_event_sequence).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Integer, Box::new(error))
            })?,
            updated_at: row.get(5)?,
        })
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::{Barrier, Mutex};

    /// Test database provider using in-memory SQLite.
    struct TestDbProvider {
        conn: Mutex<Connection>,
    }

    impl TestDbProvider {
        fn new() -> Self {
            let conn = Connection::open_in_memory().expect("Failed to create in-memory database");

            // Create tables matching the actual schema
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    status TEXT NOT NULL DEFAULT 'queued',
                    source TEXT NOT NULL DEFAULT 'web',
                    root_agent_id TEXT NOT NULL,
                    title TEXT,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    completed_at TEXT,
                    total_tokens_in INTEGER NOT NULL DEFAULT 0,
                    total_tokens_out INTEGER NOT NULL DEFAULT 0,
                    metadata TEXT,
                    pending_delegations INTEGER DEFAULT 0,
                    continuation_needed INTEGER DEFAULT 0,
                    ward_id TEXT,
                    parent_session_id TEXT,
                    thread_id TEXT,
                    connector_id TEXT,
                    respond_to TEXT,
                    mode TEXT
                );

                CREATE TABLE IF NOT EXISTS agent_executions (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    parent_execution_id TEXT,
                    delegation_type TEXT NOT NULL DEFAULT 'root',
                    task TEXT,
                    status TEXT NOT NULL DEFAULT 'queued',
                    started_at TEXT,
                    completed_at TEXT,
                    tokens_in INTEGER NOT NULL DEFAULT 0,
                    tokens_out INTEGER NOT NULL DEFAULT 0,
                    checkpoint TEXT,
                    error TEXT,
                    log_path TEXT,
                    child_session_id TEXT,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS session_plan_counters (
                    session_id TEXT PRIMARY KEY,
                    last_issued_sequence INTEGER NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS session_plans (
                    session_id TEXT PRIMARY KEY,
                    execution_id TEXT NOT NULL,
                    plan_json TEXT NOT NULL,
                    explanation TEXT,
                    source_event_timestamp INTEGER NOT NULL,
                    source_event_sequence INTEGER NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS messages (
                    id TEXT PRIMARY KEY,
                    execution_id TEXT,
                    session_id TEXT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    token_count INTEGER DEFAULT 0,
                    tool_calls TEXT,
                    tool_results TEXT,
                    tool_call_id TEXT,
                    FOREIGN KEY (execution_id) REFERENCES agent_executions(id),
                    FOREIGN KEY (session_id) REFERENCES sessions(id)
                );

                CREATE INDEX IF NOT EXISTS idx_agent_executions_session ON agent_executions(session_id);
                CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
                CREATE INDEX IF NOT EXISTS idx_messages_execution ON messages(execution_id);
                CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);

                CREATE TABLE IF NOT EXISTS artifacts (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    ward_id TEXT,
                    execution_id TEXT,
                    agent_id TEXT,
                    file_path TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    file_type TEXT,
                    file_size INTEGER,
                    label TEXT,
                    is_goal_artifact INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                "#,
            )
            .expect("Failed to create tables");
            conn.execute("PRAGMA foreign_keys = ON", [])
                .expect("enable foreign keys");

            Self {
                conn: Mutex::new(conn),
            }
        }
    }

    impl StateDbProvider for TestDbProvider {
        fn with_connection<F, R>(&self, f: F) -> Result<R, String>
        where
            F: FnOnce(&Connection) -> Result<R, rusqlite::Error>,
        {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            f(&conn).map_err(|e| e.to_string())
        }
    }

    fn setup_repo() -> StateRepository<TestDbProvider> {
        let db = Arc::new(TestDbProvider::new());
        StateRepository::new(db)
    }

    // ========================================================================
    // Session Tests
    // ========================================================================

    #[test]
    fn create_session_success() {
        let repo = setup_repo();
        let session = Session::new("root-agent");

        let result = repo.create_session(&session);
        assert!(
            result.is_ok(),
            "Failed to create session: {:?}",
            result.err()
        );
    }

    #[test]
    fn get_session_success() {
        let repo = setup_repo();
        let session = Session::new_with_source("test-agent", TriggerSource::Cli);

        repo.create_session(&session).unwrap();

        let retrieved = repo.get_session(&session.id).unwrap();
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, session.id);
        assert_eq!(retrieved.status, SessionStatus::Running);
        assert_eq!(retrieved.source, TriggerSource::Cli);
        assert_eq!(retrieved.root_agent_id, "test-agent");
    }

    #[test]
    fn get_session_not_found() {
        let repo = setup_repo();

        let result = repo.get_session("nonexistent-session");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn list_sessions_empty() {
        let repo = setup_repo();
        let filter = SessionFilter::default();

        let sessions = repo.list_sessions(&filter).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn list_sessions_with_data() {
        let repo = setup_repo();

        let s1 = Session::new("agent1");
        let s2 = Session::new("agent2");
        let s3 = Session::new_with_source("agent3", TriggerSource::Cron);

        repo.create_session(&s1).unwrap();
        repo.create_session(&s2).unwrap();
        repo.create_session(&s3).unwrap();

        let filter = SessionFilter::default();
        let sessions = repo.list_sessions(&filter).unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[test]
    fn list_sessions_with_limit() {
        let repo = setup_repo();

        for i in 0..5 {
            let session = Session::new(format!("agent{}", i));
            repo.create_session(&session).unwrap();
        }

        let filter = SessionFilter {
            limit: Some(2),
            ..Default::default()
        };
        let sessions = repo.list_sessions(&filter).unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn mission_control_summaries_default_to_fifty_rows() {
        let repo = setup_repo();

        for i in 0..55 {
            let session = Session::new(format!("agent{}", i));
            repo.create_session(&session).unwrap();
            repo.create_execution(&AgentExecution::new_root(
                &session.id,
                &session.root_agent_id,
            ))
            .unwrap();
        }

        let summaries = repo
            .list_mission_control_summaries(&MissionControlFilter::default())
            .unwrap();

        assert_eq!(summaries.len(), 50);
    }

    #[test]
    fn mission_control_summaries_clamp_excessive_limit() {
        let repo = setup_repo();

        for i in 0..205 {
            let session = Session::new(format!("agent{}", i));
            repo.create_session(&session).unwrap();
            repo.create_execution(&AgentExecution::new_root(
                &session.id,
                &session.root_agent_id,
            ))
            .unwrap();
        }

        let summaries = repo
            .list_mission_control_summaries(&MissionControlFilter {
                limit: Some(500),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(summaries.len(), 200);
    }

    #[test]
    fn mission_control_summaries_are_list_only_rows() {
        let repo = setup_repo();
        let mut session = Session::new("root-agent");
        session.total_tokens_in = 100;
        session.total_tokens_out = 25;
        repo.create_session(&session).unwrap();

        let mut root = AgentExecution::new_root(&session.id, "root-agent");
        root.id = "exec-root".to_string();
        root.tokens_in = 70;
        root.tokens_out = 20;
        repo.create_execution(&root).unwrap();

        let mut child = AgentExecution::new_delegated(
            &session.id,
            "analyst-agent",
            &root.id,
            DelegationType::Sequential,
            "analyze",
        );
        child.id = "exec-child".to_string();
        child.tokens_in = 30;
        child.tokens_out = 5;
        repo.create_execution(&child).unwrap();

        let summaries = repo
            .list_mission_control_summaries(&MissionControlFilter::default())
            .unwrap();

        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.conversation_id, session.id);
        assert_eq!(summary.root_execution_id, "exec-root");
        assert_eq!(summary.total_tokens_in, 100);
        assert_eq!(summary.total_tokens_out, 25);
        assert_eq!(summary.subagent_count, 1);

        let json = serde_json::to_value(summary).unwrap();
        assert!(json.get("executions").is_none());
        assert!(json.get("child_execution_ids").is_none());
    }

    #[test]
    fn mission_control_session_tokens_include_execution_slices() {
        let repo = setup_repo();
        let mut session = Session::new("root-agent");
        session.total_tokens_in = 100;
        session.total_tokens_out = 25;
        repo.create_session(&session).unwrap();

        let mut root = AgentExecution::new_root(&session.id, "root-agent");
        root.id = "exec-root".to_string();
        root.tokens_in = 70;
        root.tokens_out = 20;
        repo.create_execution(&root).unwrap();

        let mut child = AgentExecution::new_delegated(
            &session.id,
            "analyst-agent",
            &root.id,
            DelegationType::Sequential,
            "analyze",
        );
        child.id = "exec-child".to_string();
        child.tokens_in = 30;
        child.tokens_out = 5;
        repo.create_execution(&child).unwrap();

        let tokens = repo
            .get_mission_control_session_tokens(&session.id)
            .unwrap()
            .unwrap();

        assert_eq!(tokens.conversation_id, session.id);
        assert_eq!(tokens.root_execution_id, "exec-root");
        assert_eq!(tokens.total_tokens_in, 100);
        assert_eq!(tokens.total_tokens_out, 25);
        assert_eq!(tokens.executions.len(), 2);
        assert_eq!(tokens.executions[0].execution_id, "exec-root");
        assert_eq!(tokens.executions[1].execution_id, "exec-child");
        assert!(tokens.current_plan.is_none());
    }

    #[test]
    fn session_plan_is_saved_and_projected_only_for_the_selected_session() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&root).unwrap();

        let saved = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{
                    "step": "Inspect the deployment configuration",
                    "status": "in_progress"
                }]),
                Some("Start with the deployed settings".to_owned()),
                100,
            )
            .unwrap();

        let plan = saved.accepted().expect("valid plan should be accepted");
        assert_eq!(plan.source_event_sequence, 1);
        assert_eq!(plan.plan[0].status, SessionPlanStepStatus::InProgress);

        let detail = repo
            .get_mission_control_session_tokens(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.current_plan, Some(plan));
    }

    #[test]
    fn terminal_completion_marks_actionable_steps_complete_and_keeps_failures() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&root).unwrap();
        repo.save_session_plan(
            &session.id,
            &root.id,
            serde_json::json!([
                {"step": "Gather evidence", "status": "completed"},
                {"step": "Analyze evidence", "status": "in_progress"},
                {"step": "Write conclusion", "status": "pending"},
                {"step": "Unavailable source", "status": "failed"}
            ]),
            Some("Preserve this explanation".to_owned()),
            100,
        )
        .unwrap();

        // The method is lifecycle-only: an active session must retain the
        // model's current progress rather than being force-completed.
        assert!(repo
            .complete_session_plan(&session.id, &root.id)
            .unwrap()
            .is_none());
        assert_eq!(
            repo.get_mission_control_session_tokens(&session.id)
                .unwrap()
                .unwrap()
                .current_plan
                .expect("active plan")
                .plan[1]
                .status,
            SessionPlanStepStatus::InProgress
        );
        repo.update_execution_status(&root.id, ExecutionStatus::Completed)
            .unwrap();
        repo.update_session_status(&session.id, SessionStatus::Completed)
            .unwrap();

        let completed = repo
            .complete_session_plan(&session.id, &root.id)
            .unwrap()
            .expect("actionable steps should change");
        assert_eq!(
            completed
                .plan
                .iter()
                .map(|step| step.status)
                .collect::<Vec<_>>(),
            vec![
                SessionPlanStepStatus::Completed,
                SessionPlanStepStatus::Completed,
                SessionPlanStepStatus::Completed,
                SessionPlanStepStatus::Failed,
            ]
        );
        assert_eq!(
            completed.explanation.as_deref(),
            Some("Preserve this explanation")
        );
        assert_eq!(completed.source_event_sequence, 2);
        assert_eq!(
            repo.get_mission_control_session_tokens(&session.id)
                .unwrap()
                .unwrap()
                .current_plan,
            Some(completed)
        );
        assert!(repo
            .complete_session_plan(&session.id, &root.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejected_plan_preserves_the_prior_snapshot_and_session_delete_cascades() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&root).unwrap();

        let first = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "Capture baseline", "status": "pending"}]),
                None,
                100,
            )
            .unwrap()
            .accepted()
            .unwrap();
        let rejected = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{
                    "step": "DROP TABLE sessions;",
                    "status": "not_a_status",
                    "unknown": true
                }]),
                None,
                101,
            )
            .unwrap();
        assert!(rejected.accepted().is_none());

        let detail = repo
            .get_mission_control_session_tokens(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.current_plan, Some(first));

        let next = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "Review baseline", "status": "completed"}]),
                None,
                102,
            )
            .unwrap()
            .accepted()
            .unwrap();
        assert_eq!(
            next.source_event_sequence, 2,
            "rejected input must not consume a sequence"
        );

        repo.delete_session(&session.id).unwrap();
        let plans_remaining = repo
            .db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM session_plans WHERE session_id = ?1",
                    params![session.id],
                    |row| row.get::<_, i64>(0),
                )
            })
            .unwrap();
        assert_eq!(plans_remaining, 0);
    }

    #[test]
    fn foreign_execution_cannot_replace_a_session_plan() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        let foreign_session = Session::new("foreign-agent");
        repo.create_session(&session).unwrap();
        repo.create_session(&foreign_session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        let foreign_root = AgentExecution::new_root(&foreign_session.id, "foreign-agent");
        repo.create_execution(&root).unwrap();
        repo.create_execution(&foreign_root).unwrap();

        let original = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "Keep this plan", "status": "pending"}]),
                None,
                100,
            )
            .unwrap()
            .accepted()
            .unwrap();

        let outcome = repo
            .save_session_plan(
                &session.id,
                &foreign_root.id,
                serde_json::json!([{"step": "Must not replace", "status": "completed"}]),
                None,
                101,
            )
            .unwrap();

        assert_eq!(
            outcome,
            SessionPlanSaveOutcome::Rejected(SessionPlanRejection::ExecutionSessionMismatch)
        );
        let detail = repo
            .get_mission_control_session_tokens(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.current_plan, Some(original));
    }

    #[test]
    fn session_plan_sequence_continues_after_repository_restart() {
        let db = Arc::new(TestDbProvider::new());
        let first_repo = StateRepository::new(db.clone());
        let session = Session::new("root-agent");
        first_repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        first_repo.create_execution(&root).unwrap();

        let first = first_repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "First update", "status": "pending"}]),
                None,
                1,
            )
            .unwrap()
            .accepted()
            .unwrap();
        drop(first_repo);

        let restarted_repo = StateRepository::new(db);
        let second = restarted_repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "Recovered update", "status": "in_progress"}]),
                None,
                0,
            )
            .unwrap()
            .accepted()
            .unwrap();

        assert_eq!(first.source_event_sequence, 1);
        assert_eq!(second.source_event_sequence, 2);
        assert_eq!(second.plan[0].step, "Recovered update");
    }

    #[test]
    fn equal_timestamps_use_acceptance_sequence_and_replace_the_snapshot() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&root).unwrap();

        let first = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "First", "status": "pending"}]),
                None,
                42,
            )
            .unwrap()
            .accepted()
            .unwrap();
        let second = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "Second", "status": "completed"}]),
                None,
                42,
            )
            .unwrap()
            .accepted()
            .unwrap();

        assert_eq!(
            (first.source_event_sequence, second.source_event_sequence),
            (1, 2)
        );
        let detail = repo
            .get_mission_control_session_tokens(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.current_plan.unwrap().plan[0].step, "Second");
    }

    #[test]
    fn concurrent_plan_acceptance_allocates_each_session_sequence_once() {
        let repo = Arc::new(setup_repo());
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&root).unwrap();

        const WRITERS: usize = 4;
        let barrier = Arc::new(Barrier::new(WRITERS));
        let mut writers = Vec::new();
        for index in 0..WRITERS {
            let repo = repo.clone();
            let barrier = barrier.clone();
            let session_id = session.id.clone();
            let execution_id = root.id.clone();
            writers.push(std::thread::spawn(move || {
                barrier.wait();
                repo.save_session_plan(
                    &session_id,
                    &execution_id,
                    serde_json::json!([{
                        "step": format!("Concurrent step {index}"),
                        "status": "pending"
                    }]),
                    None,
                    1,
                )
                .expect("save plan")
                .accepted()
                .expect("accepted plan")
                .source_event_sequence
            }));
        }

        let mut sequences: Vec<u64> = writers
            .into_iter()
            .map(|writer| writer.join().expect("writer join"))
            .collect();
        sequences.sort_unstable();
        assert_eq!(sequences, vec![1, 2, 3, 4]);
    }

    #[test]
    fn lower_sequence_at_repository_boundary_preserves_the_existing_plan() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();
        let root = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&root).unwrap();
        repo.db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO session_plan_counters (session_id, last_issued_sequence)
                     VALUES (?1, 4)",
                    params![session.id],
                )?;
                conn.execute(
                    "INSERT INTO session_plans (
                        session_id, execution_id, plan_json, explanation,
                        source_event_timestamp, source_event_sequence, updated_at
                     ) VALUES (?1, ?2, ?3, NULL, 10, 5, ?4)",
                    params![
                        session.id,
                        root.id,
                        r#"[{"step":"Existing plan","status":"pending"}]"#,
                        "2026-07-14T00:00:00Z",
                    ],
                )?;
                Ok(())
            })
            .unwrap();

        let outcome = repo
            .save_session_plan(
                &session.id,
                &root.id,
                serde_json::json!([{"step": "Must not replace", "status": "completed"}]),
                None,
                11,
            )
            .unwrap();
        assert_eq!(
            outcome,
            SessionPlanSaveOutcome::Rejected(SessionPlanRejection::StaleSequence)
        );

        let detail = repo
            .get_mission_control_session_tokens(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.current_plan.unwrap().plan[0].step, "Existing plan");
    }

    #[test]
    fn update_session_status() {
        let repo = setup_repo();
        let session = Session::new("agent");

        repo.create_session(&session).unwrap();
        repo.update_session_status(&session.id, SessionStatus::Completed)
            .unwrap();

        let updated = repo.get_session(&session.id).unwrap().unwrap();
        assert_eq!(updated.status, SessionStatus::Completed);
        assert!(updated.completed_at.is_some());
    }

    #[test]
    fn update_session_tokens() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        // Create an execution with tokens
        let mut exec = AgentExecution::new_root(&session.id, "agent");
        exec.tokens_in = 100;
        exec.tokens_out = 50;
        repo.create_execution(&exec).unwrap();

        // Update session tokens from execution totals
        repo.update_session_tokens(&session.id).unwrap();

        let updated = repo.get_session(&session.id).unwrap().unwrap();
        assert_eq!(updated.total_tokens_in, 100);
        assert_eq!(updated.total_tokens_out, 50);
    }

    #[test]
    fn delete_session() {
        let repo = setup_repo();
        let session = Session::new("agent");

        repo.create_session(&session).unwrap();
        assert!(repo.get_session(&session.id).unwrap().is_some());

        repo.delete_session(&session.id).unwrap();
        assert!(repo.get_session(&session.id).unwrap().is_none());
    }

    // ========================================================================
    // Cascade Delete Tests (R18)
    //
    // The base TestDbProvider only creates sessions/agent_executions/messages.
    // `delete_session_cascade` touches seven child tables plus a memory_facts
    // table that MUST survive — so these tests use a richer in-memory schema.
    // ========================================================================

    /// In-memory DB provider with every table `delete_session_cascade` reads
    /// or asserts on. Deliberately mirrors the production column names from
    /// the conversation schema so the real cascade SQL runs.
    struct CascadeDbProvider {
        conn: Mutex<Connection>,
    }

    impl CascadeDbProvider {
        fn new() -> Self {
            let conn = Connection::open_in_memory().expect("in-memory db");
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    status TEXT NOT NULL DEFAULT 'running',
                    source TEXT NOT NULL DEFAULT 'web',
                    root_agent_id TEXT NOT NULL,
                    title TEXT,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    completed_at TEXT,
                    total_tokens_in INTEGER NOT NULL DEFAULT 0,
                    total_tokens_out INTEGER NOT NULL DEFAULT 0,
                    metadata TEXT,
                    pending_delegations INTEGER DEFAULT 0,
                    continuation_needed INTEGER DEFAULT 0,
                    ward_id TEXT,
                    parent_session_id TEXT,
                    thread_id TEXT,
                    connector_id TEXT,
                    respond_to TEXT,
                    mode TEXT
                );

                CREATE TABLE IF NOT EXISTS agent_executions (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    parent_execution_id TEXT,
                    delegation_type TEXT NOT NULL DEFAULT 'root',
                    task TEXT,
                    status TEXT NOT NULL DEFAULT 'queued',
                    started_at TEXT,
                    completed_at TEXT,
                    tokens_in INTEGER NOT NULL DEFAULT 0,
                    tokens_out INTEGER NOT NULL DEFAULT 0,
                    checkpoint TEXT,
                    error TEXT,
                    log_path TEXT,
                    child_session_id TEXT
                );

                CREATE TABLE IF NOT EXISTS messages (
                    id TEXT PRIMARY KEY,
                    execution_id TEXT,
                    session_id TEXT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    token_count INTEGER DEFAULT 0,
                    tool_calls TEXT,
                    tool_results TEXT,
                    tool_call_id TEXT
                );

                CREATE TABLE IF NOT EXISTS execution_logs (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    conversation_id TEXT,
                    agent_id TEXT NOT NULL,
                    parent_session_id TEXT,
                    timestamp TEXT NOT NULL,
                    level TEXT NOT NULL,
                    category TEXT NOT NULL,
                    message TEXT NOT NULL,
                    metadata TEXT,
                    duration_ms INTEGER
                );

                CREATE TABLE IF NOT EXISTS artifacts (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    ward_id TEXT,
                    execution_id TEXT,
                    agent_id TEXT,
                    file_path TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    file_type TEXT,
                    file_size INTEGER,
                    label TEXT,
                    is_goal_artifact INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS distillation_runs (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL UNIQUE,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS bridge_outbox (
                    id TEXT PRIMARY KEY,
                    adapter_id TEXT NOT NULL,
                    capability TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending',
                    session_id TEXT,
                    thread_id TEXT,
                    agent_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS recall_log (
                    session_id TEXT NOT NULL,
                    fact_key TEXT NOT NULL,
                    recalled_at TEXT NOT NULL,
                    PRIMARY KEY (session_id, fact_key)
                );

                CREATE TABLE IF NOT EXISTS memory_facts (
                    id TEXT PRIMARY KEY,
                    session_id TEXT,
                    agent_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    category TEXT NOT NULL,
                    key TEXT NOT NULL,
                    content TEXT NOT NULL,
                    ward_id TEXT NOT NULL DEFAULT '__global__',
                    created_at TEXT NOT NULL
                );

                -- Knowledge graph tables — presence only, to prove the
                -- cascade doesn't reference or touch them.
                CREATE TABLE IF NOT EXISTS kg_entities (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS kg_relationships (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    source_entity_id TEXT NOT NULL,
                    target_entity_id TEXT NOT NULL,
                    relationship TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                "#,
            )
            .expect("create cascade schema");
            Self {
                conn: Mutex::new(conn),
            }
        }
    }

    impl StateDbProvider for CascadeDbProvider {
        fn with_connection<F, R>(&self, f: F) -> Result<R, String>
        where
            F: FnOnce(&Connection) -> Result<R, rusqlite::Error>,
        {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            f(&conn).map_err(|e| e.to_string())
        }
    }

    fn cascade_repo() -> (Arc<CascadeDbProvider>, StateRepository<CascadeDbProvider>) {
        let db = Arc::new(CascadeDbProvider::new());
        let repo = StateRepository::new(db.clone());
        (db, repo)
    }

    fn count_rows(db: &CascadeDbProvider, table: &str, col: &str, val: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {} WHERE {} = ?", table, col);
        db.with_connection(|conn| conn.query_row(&sql, params![val], |row| row.get::<_, i64>(0)))
            .unwrap()
    }

    fn seed_cascade_fixture(
        db: &CascadeDbProvider,
        repo: &StateRepository<CascadeDbProvider>,
        sid: &str,
    ) {
        // 1 session (via the real repository so all columns align).
        let mut session = Session::new_with_source("root", TriggerSource::Web);
        session.id = sid.to_string();
        repo.create_session(&session).unwrap();

        // Child rows have ids prefixed with the session id so multiple calls
        // to this helper within one test don't collide on PRIMARY KEY.
        let exec_id = format!("exec-{}", sid);
        let msg_id = format!("msg-{}", sid);
        let log_id = format!("log-{}", sid);
        let art_id = format!("art-{}", sid);
        let dr_id = format!("dr-{}", sid);
        let ob_id = format!("ob-{}", sid);

        // 1 execution + 1 message + 1 execution_log + 1 artifact
        // + 1 distillation_run + 1 bridge_outbox + 1 recall_log.
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_executions (id, session_id, agent_id, delegation_type, status) \
                 VALUES (?1, ?2, ?3, 'root', 'running')",
                params![exec_id, sid, "root"],
            )?;
            conn.execute(
                "INSERT INTO messages (id, execution_id, session_id, role, content, created_at) \
                 VALUES (?1, ?2, ?3, 'user', 'hello', '2026-04-19T00:00:00Z')",
                params![msg_id, exec_id, sid],
            )?;
            conn.execute(
                "INSERT INTO execution_logs (id, session_id, agent_id, timestamp, level, category, message) \
                 VALUES (?1, ?2, 'root', '2026-04-19T00:00:00Z', 'info', 'test', 'log')",
                params![log_id, sid],
            )?;
            conn.execute(
                "INSERT INTO artifacts (id, session_id, file_path, file_name, created_at) \
                 VALUES (?1, ?2, '/tmp/note.md', 'note.md', '2026-04-19T00:00:00Z')",
                params![art_id, sid],
            )?;
            conn.execute(
                "INSERT INTO distillation_runs (id, session_id, status, created_at) \
                 VALUES (?1, ?2, 'completed', '2026-04-19T00:00:00Z')",
                params![dr_id, sid],
            )?;
            conn.execute(
                "INSERT INTO bridge_outbox (id, adapter_id, capability, payload, status, session_id, created_at) \
                 VALUES (?1, 'adp', 'cap', '{}', 'pending', ?2, '2026-04-19T00:00:00Z')",
                params![ob_id, sid],
            )?;
            conn.execute(
                "INSERT INTO recall_log (session_id, fact_key, recalled_at) VALUES (?1, 'k', '2026-04-19T00:00:00Z')",
                params![sid],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn seed_memory_fact(db: &CascadeDbProvider, id: &str, content: &str) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO memory_facts (id, agent_id, scope, category, key, content, created_at) \
                 VALUES (?1, 'root', 'agent', 'note', 'k', ?2, '2026-04-19T00:00:00Z')",
                params![id, content],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn seed_kg_entity(db: &CascadeDbProvider, id: &str, name: &str) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_entities (id, agent_id, name, created_at) \
                 VALUES (?1, 'root', ?2, '2026-04-19T00:00:00Z')",
                params![id, name],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn seed_kg_relationship(db: &CascadeDbProvider, id: &str, src: &str, dst: &str) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_relationships (id, agent_id, source_entity_id, target_entity_id, relationship, created_at) \
                 VALUES (?1, 'root', ?2, ?3, 'related_to', '2026-04-19T00:00:00Z')",
                params![id, src, dst],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_session_cascade_removes_per_session_rows() {
        let (db, repo) = cascade_repo();
        let sid = "sess-cascade-1";
        seed_cascade_fixture(&db, &repo, sid);

        // Sanity: everything we seeded is there.
        assert_eq!(count_rows(&db, "sessions", "id", sid), 1);
        assert_eq!(count_rows(&db, "messages", "session_id", sid), 1);
        assert_eq!(count_rows(&db, "agent_executions", "session_id", sid), 1);
        assert_eq!(count_rows(&db, "execution_logs", "session_id", sid), 1);
        assert_eq!(count_rows(&db, "artifacts", "session_id", sid), 1);
        assert_eq!(count_rows(&db, "distillation_runs", "session_id", sid), 1);
        assert_eq!(count_rows(&db, "bridge_outbox", "session_id", sid), 1);
        assert_eq!(count_rows(&db, "recall_log", "session_id", sid), 1);

        let deleted = repo.delete_session_cascade(sid).unwrap();
        // 1 session + 1 per each of the 7 child tables = 8.
        assert_eq!(deleted, 8, "expected 8 rows deleted, got {deleted}");

        // Every per-session table is empty for this id.
        assert_eq!(count_rows(&db, "sessions", "id", sid), 0);
        assert_eq!(count_rows(&db, "messages", "session_id", sid), 0);
        assert_eq!(count_rows(&db, "agent_executions", "session_id", sid), 0);
        assert_eq!(count_rows(&db, "execution_logs", "session_id", sid), 0);
        assert_eq!(count_rows(&db, "artifacts", "session_id", sid), 0);
        assert_eq!(count_rows(&db, "distillation_runs", "session_id", sid), 0);
        assert_eq!(count_rows(&db, "bridge_outbox", "session_id", sid), 0);
        assert_eq!(count_rows(&db, "recall_log", "session_id", sid), 0);

        assert!(repo.get_session(sid).unwrap().is_none());
    }

    #[test]
    fn delete_session_cascade_preserves_memory_facts() {
        let (db, repo) = cascade_repo();
        let sid = "sess-cascade-2";
        seed_cascade_fixture(&db, &repo, sid);
        seed_memory_fact(&db, "fact-1", "persistent fact");
        seed_memory_fact(&db, "fact-2", "another fact");

        repo.delete_session_cascade(sid).unwrap();

        // Memory facts survive — cross-session state users rely on.
        assert_eq!(
            count_rows(&db, "memory_facts", "content", "persistent fact"),
            1,
            "memory_facts was touched by the cascade — that's the invariant violation we're guarding against"
        );
        assert_eq!(
            count_rows(&db, "memory_facts", "content", "another fact"),
            1
        );
    }

    #[test]
    fn delete_session_cascade_preserves_knowledge_graph() {
        let (db, repo) = cascade_repo();
        let sid = "sess-cascade-3";
        seed_cascade_fixture(&db, &repo, sid);
        seed_kg_entity(&db, "ent-1", "FooCorp");
        seed_kg_entity(&db, "ent-2", "BarCorp");
        seed_kg_relationship(&db, "rel-1", "ent-1", "ent-2");

        repo.delete_session_cascade(sid).unwrap();

        assert_eq!(count_rows(&db, "kg_entities", "id", "ent-1"), 1);
        assert_eq!(count_rows(&db, "kg_entities", "id", "ent-2"), 1);
        assert_eq!(count_rows(&db, "kg_relationships", "id", "rel-1"), 1);
    }

    #[test]
    fn delete_session_cascade_on_missing_session_returns_ok_zero() {
        let (_db, repo) = cascade_repo();
        let rows = repo.delete_session_cascade("sess-does-not-exist").unwrap();
        assert_eq!(rows, 0, "missing session should be a no-op, not an error");
    }

    #[test]
    fn delete_session_cascade_only_deletes_target_session() {
        // Two sessions side-by-side; deleting one must not touch the other.
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-keep");
        seed_cascade_fixture(&db, &repo, "sess-kill");

        let deleted = repo.delete_session_cascade("sess-kill").unwrap();
        assert_eq!(deleted, 8);

        // Survivor is fully intact.
        assert_eq!(count_rows(&db, "sessions", "id", "sess-keep"), 1);
        assert_eq!(count_rows(&db, "messages", "session_id", "sess-keep"), 1);
        assert_eq!(
            count_rows(&db, "agent_executions", "session_id", "sess-keep"),
            1
        );
        assert_eq!(
            count_rows(&db, "execution_logs", "session_id", "sess-keep"),
            1
        );
        assert_eq!(count_rows(&db, "artifacts", "session_id", "sess-keep"), 1);
        assert_eq!(
            count_rows(&db, "distillation_runs", "session_id", "sess-keep"),
            1
        );
        assert_eq!(
            count_rows(&db, "bridge_outbox", "session_id", "sess-keep"),
            1
        );
        assert_eq!(count_rows(&db, "recall_log", "session_id", "sess-keep"), 1);

        // Target is fully gone.
        assert_eq!(count_rows(&db, "sessions", "id", "sess-kill"), 0);
        assert_eq!(count_rows(&db, "messages", "session_id", "sess-kill"), 0);
    }

    // ========================================================================
    // delete_session_recursive_cascade Tests
    //
    // Recursive cascade walks `sessions.parent_session_id` to wipe the entire
    // subtree spawned by a root chat / research session. Memory tables must
    // still survive — same invariant as the non-recursive cascade.
    // ========================================================================

    /// Seed a session that has a parent — used to build subagent trees.
    fn seed_child_session(
        db: &CascadeDbProvider,
        repo: &StateRepository<CascadeDbProvider>,
        sid: &str,
        parent_sid: &str,
    ) {
        let mut session = Session::new_with_source("subagent", TriggerSource::Web);
        session.id = sid.to_string();
        session.parent_session_id = Some(parent_sid.to_string());
        repo.create_session(&session).unwrap();

        let exec_id = format!("exec-{}", sid);
        let msg_id = format!("msg-{}", sid);
        let log_id = format!("log-{}", sid);
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_executions (id, session_id, agent_id, delegation_type, status) \
                 VALUES (?1, ?2, 'subagent', 'delegated', 'running')",
                params![exec_id, sid],
            )?;
            conn.execute(
                "INSERT INTO messages (id, execution_id, session_id, role, content, created_at) \
                 VALUES (?1, ?2, ?3, 'assistant', 'subagent reply', '2026-04-19T00:00:00Z')",
                params![msg_id, exec_id, sid],
            )?;
            conn.execute(
                "INSERT INTO execution_logs (id, session_id, agent_id, parent_session_id, timestamp, level, category, message) \
                 VALUES (?1, ?2, 'subagent', ?3, '2026-04-19T00:00:00Z', 'info', 'test', 'sublog')",
                params![log_id, sid, parent_sid],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_recursive_cascade_wipes_full_subagent_tree() {
        // Root → 2 children, one of which has a grandchild. Deleting the
        // root must take all four sessions plus their per-session rows.
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-root");
        seed_child_session(&db, &repo, "sess-child-a", "sess-root");
        seed_child_session(&db, &repo, "sess-child-b", "sess-root");
        seed_child_session(&db, &repo, "sess-grandchild", "sess-child-a");

        let deleted = repo.delete_session_recursive_cascade("sess-root").unwrap();
        // Root: 1 session + 7 child rows = 8.
        // Each of 3 descendants: 1 session + 1 exec + 1 msg + 1 log = 4.
        // Total = 8 + 3*4 = 20.
        assert_eq!(deleted, 20, "expected 20 rows deleted, got {deleted}");

        for sid in [
            "sess-root",
            "sess-child-a",
            "sess-child-b",
            "sess-grandchild",
        ] {
            assert_eq!(
                count_rows(&db, "sessions", "id", sid),
                0,
                "session {sid} should be gone"
            );
            assert_eq!(
                count_rows(&db, "messages", "session_id", sid),
                0,
                "messages for {sid} should be gone"
            );
            assert_eq!(
                count_rows(&db, "agent_executions", "session_id", sid),
                0,
                "agent_executions for {sid} should be gone"
            );
            assert_eq!(
                count_rows(&db, "execution_logs", "session_id", sid),
                0,
                "execution_logs for {sid} should be gone"
            );
        }
    }

    #[test]
    fn delete_recursive_cascade_preserves_memory_and_kg() {
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-root");
        seed_child_session(&db, &repo, "sess-child", "sess-root");
        seed_memory_fact(&db, "fact-survives", "important note");
        seed_kg_entity(&db, "ent-survives", "FooCorp");
        seed_kg_relationship(&db, "rel-survives", "ent-survives", "ent-survives");

        repo.delete_session_recursive_cascade("sess-root").unwrap();

        assert_eq!(
            count_rows(&db, "memory_facts", "id", "fact-survives"),
            1,
            "memory facts must outlive a recursive subtree delete"
        );
        assert_eq!(count_rows(&db, "kg_entities", "id", "ent-survives"), 1);
        assert_eq!(count_rows(&db, "kg_relationships", "id", "rel-survives"), 1);
    }

    #[test]
    fn delete_recursive_cascade_does_not_touch_unrelated_trees() {
        // Two parallel trees. Deleting one tree must leave the other intact.
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-keep-root");
        seed_child_session(&db, &repo, "sess-keep-child", "sess-keep-root");
        seed_cascade_fixture(&db, &repo, "sess-kill-root");
        seed_child_session(&db, &repo, "sess-kill-child", "sess-kill-root");

        repo.delete_session_recursive_cascade("sess-kill-root")
            .unwrap();

        assert_eq!(count_rows(&db, "sessions", "id", "sess-keep-root"), 1);
        assert_eq!(count_rows(&db, "sessions", "id", "sess-keep-child"), 1);
        assert_eq!(
            count_rows(&db, "messages", "session_id", "sess-keep-child"),
            1
        );
        assert_eq!(count_rows(&db, "sessions", "id", "sess-kill-root"), 0);
        assert_eq!(count_rows(&db, "sessions", "id", "sess-kill-child"), 0);
    }

    #[test]
    fn delete_recursive_cascade_leaf_session_only_touches_leaf() {
        // Calling recursive cascade with a leaf id (no children) must behave
        // exactly like the non-recursive cascade — siblings survive.
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-root");
        seed_child_session(&db, &repo, "sess-leaf-a", "sess-root");
        seed_child_session(&db, &repo, "sess-leaf-b", "sess-root");

        repo.delete_session_recursive_cascade("sess-leaf-a")
            .unwrap();

        assert_eq!(count_rows(&db, "sessions", "id", "sess-leaf-a"), 0);
        assert_eq!(count_rows(&db, "sessions", "id", "sess-leaf-b"), 1);
        assert_eq!(count_rows(&db, "sessions", "id", "sess-root"), 1);
    }

    #[test]
    fn delete_recursive_cascade_missing_root_is_no_op() {
        let (_db, repo) = cascade_repo();
        let rows = repo
            .delete_session_recursive_cascade("sess-does-not-exist")
            .unwrap();
        assert_eq!(rows, 0);
    }

    /// Production shape: api-logs writes `execution_logs.session_id = exec-…`
    /// and `conversation_id = sess-…`. Used to seed orphan rows that exist
    /// only in the logs table (no sessions row).
    fn seed_log_only_orphan(db: &CascadeDbProvider, conversation_id: &str, rows: usize) {
        db.with_connection(|conn| {
            for i in 0..rows {
                let log_id = format!("log-orphan-{}-{}", conversation_id, i);
                let exec_id = format!("exec-orphan-{}-{}", conversation_id, i);
                conn.execute(
                    "INSERT INTO execution_logs \
                     (id, session_id, conversation_id, agent_id, timestamp, level, category, message) \
                     VALUES (?1, ?2, ?3, 'root', '2026-04-19T00:00:00Z', 'info', 'test', 'orphan')",
                    params![log_id, exec_id, conversation_id],
                )?;
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_recursive_cascade_cleans_orphan_logs_with_no_sessions_row() {
        // Reproduces the field bug where a session_id surfaced by
        // /api/logs/sessions had rows only in execution_logs.conversation_id —
        // the previous cascade silently no-opped because it matched the wrong
        // column and the endpoint 404-ed before cascade ran.
        let (db, repo) = cascade_repo();
        seed_log_only_orphan(&db, "sess-orphan-only", 7);
        // Sanity: nothing in sessions, exactly 7 orphan log rows.
        assert_eq!(count_rows(&db, "sessions", "id", "sess-orphan-only"), 0);
        assert_eq!(
            count_rows(&db, "execution_logs", "conversation_id", "sess-orphan-only"),
            7
        );

        let deleted = repo
            .delete_session_recursive_cascade("sess-orphan-only")
            .unwrap();
        assert_eq!(deleted, 7, "all 7 orphan log rows must be removed");
        assert_eq!(
            count_rows(&db, "execution_logs", "conversation_id", "sess-orphan-only"),
            0
        );
    }

    #[test]
    fn delete_recursive_cascade_cleans_logs_in_production_shape() {
        // Real session has sessions row + per-session rows + execution_logs
        // emitted in production shape (session_id=exec-X, conversation_id=sess-X).
        // The historical cascade missed these log rows entirely.
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-prod");
        // The fixture seeds one log via session_id; layer on production-shape
        // rows via conversation_id to prove the OR match works.
        seed_log_only_orphan(&db, "sess-prod", 4);
        assert_eq!(
            count_rows(&db, "execution_logs", "conversation_id", "sess-prod"),
            4
        );

        repo.delete_session_recursive_cascade("sess-prod").unwrap();

        assert_eq!(count_rows(&db, "sessions", "id", "sess-prod"), 0);
        assert_eq!(
            count_rows(&db, "execution_logs", "conversation_id", "sess-prod"),
            0,
            "production-shape log rows (matched via conversation_id) must be deleted"
        );
        assert_eq!(
            count_rows(&db, "execution_logs", "session_id", "sess-prod"),
            0,
            "fixture log row (matched via session_id) must also be deleted"
        );
    }

    /// Seed continuation log rows for a stem session in production shape:
    /// session_id=exec-X (reused root execution), conversation_id=stem-cont-XXX.
    /// This is what `runner::invoke_continuation` writes per turn.
    fn seed_continuation_logs(
        db: &CascadeDbProvider,
        stem: &str,
        exec_id: &str,
        suffixes: &[&str],
    ) {
        db.with_connection(|conn| {
            for sfx in suffixes {
                let log_id = format!("log-cont-{}-{}", stem, sfx);
                let conv_id = format!("{}-cont-{}", stem, sfx);
                conn.execute(
                    "INSERT INTO execution_logs \
                     (id, session_id, conversation_id, agent_id, timestamp, level, category, message) \
                     VALUES (?1, ?2, ?3, 'root', '2026-04-19T00:00:00Z', 'info', 'session', 'cont start')",
                    params![log_id, exec_id, conv_id],
                )?;
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_recursive_cascade_clears_all_continuation_logs() {
        // A session emits N continuation turns; every -cont-XXX log row must
        // be removed by a single cascade keyed on the canonical session id.
        // This is the regression the user surfaced: deleting the canonical
        // session left N-1 phantom continuations in execution_logs that
        // surfaced as new sidebar rows on the next refresh.
        let (db, repo) = cascade_repo();
        let stem = "sess-with-cont";
        seed_cascade_fixture(&db, &repo, stem);
        seed_continuation_logs(
            &db,
            stem,
            "exec-shared",
            &["aaaaaaaa", "bbbbbbbb", "cccccccc", "dddddddd"],
        );
        // 4 continuation log rows under conversation_id LIKE 'sess-with-cont-cont-%'
        let cont_rows_before: i64 = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM execution_logs \
                     WHERE conversation_id LIKE 'sess-with-cont-cont-%'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert_eq!(cont_rows_before, 4);

        repo.delete_session_recursive_cascade(stem).unwrap();

        let cont_rows_after: i64 = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM execution_logs \
                     WHERE conversation_id LIKE 'sess-with-cont-cont-%'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert_eq!(
            cont_rows_after, 0,
            "every continuation log row must be removed by a single cascade on the stem"
        );
        assert_eq!(count_rows(&db, "sessions", "id", stem), 0);
    }

    #[test]
    fn delete_recursive_cascade_continuation_does_not_match_other_stems() {
        // `LIKE 'rootid-cont-%'` must be anchored to the root id, not match
        // some other session whose id happens to share a prefix.
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-keep");
        seed_continuation_logs(&db, "sess-keep", "exec-keep", &["11111111"]);
        seed_cascade_fixture(&db, &repo, "sess-kill");
        seed_continuation_logs(&db, "sess-kill", "exec-kill", &["22222222"]);

        repo.delete_session_recursive_cascade("sess-kill").unwrap();

        let kept_cont: i64 = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM execution_logs \
                     WHERE conversation_id = 'sess-keep-cont-11111111'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert_eq!(kept_cont, 1, "sibling stem's continuations must survive");
        let killed_cont: i64 = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM execution_logs \
                     WHERE conversation_id = 'sess-kill-cont-22222222'",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert_eq!(killed_cont, 0);
    }

    #[test]
    fn delete_recursive_cascade_runs_twice_safely() {
        // Temp table cleanup must allow back-to-back calls on the same
        // connection (regression guard for `_delete_session_set` collisions).
        let (db, repo) = cascade_repo();
        seed_cascade_fixture(&db, &repo, "sess-a");
        seed_cascade_fixture(&db, &repo, "sess-b");

        repo.delete_session_recursive_cascade("sess-a").unwrap();
        repo.delete_session_recursive_cascade("sess-b").unwrap();

        assert_eq!(count_rows(&db, "sessions", "id", "sess-a"), 0);
        assert_eq!(count_rows(&db, "sessions", "id", "sess-b"), 0);
    }

    // ========================================================================
    // Execution Tests
    // ========================================================================

    #[test]
    fn create_execution_success() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec = AgentExecution::new_root(&session.id, "root-agent");
        let result = repo.create_execution(&exec);

        assert!(
            result.is_ok(),
            "Failed to create execution: {:?}",
            result.err()
        );
    }

    #[test]
    fn get_execution_success() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec = AgentExecution::new_root(&session.id, "root-agent");
        repo.create_execution(&exec).unwrap();

        let retrieved = repo.get_execution(&exec.id).unwrap();
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, exec.id);
        assert_eq!(retrieved.session_id, session.id);
        assert_eq!(retrieved.agent_id, "root-agent");
        assert_eq!(retrieved.delegation_type, DelegationType::Root);
    }

    #[test]
    fn get_execution_not_found() {
        let repo = setup_repo();

        let result = repo.get_execution("nonexistent");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn create_delegated_execution() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let root_exec = AgentExecution::new_root(&session.id, "root");
        repo.create_execution(&root_exec).unwrap();

        let sub_exec = AgentExecution::new_delegated(
            &session.id,
            "researcher",
            &root_exec.id,
            DelegationType::Sequential,
            "Research AI topics",
        );
        repo.create_execution(&sub_exec).unwrap();

        let retrieved = repo.get_execution(&sub_exec.id).unwrap().unwrap();
        assert_eq!(retrieved.parent_execution_id, Some(root_exec.id.clone()));
        assert_eq!(retrieved.delegation_type, DelegationType::Sequential);
        assert_eq!(retrieved.task, Some("Research AI topics".to_string()));
    }

    #[test]
    fn list_executions_by_session() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec1 = AgentExecution::new_root(&session.id, "agent1");
        let exec2 = AgentExecution::new_delegated(
            &session.id,
            "agent2",
            &exec1.id,
            DelegationType::Sequential,
            "task",
        );

        repo.create_execution(&exec1).unwrap();
        repo.create_execution(&exec2).unwrap();

        let filter = ExecutionFilter {
            session_id: Some(session.id.clone()),
            ..Default::default()
        };

        let executions = repo.list_executions(&filter).unwrap();
        assert_eq!(executions.len(), 2);
    }

    #[test]
    fn get_child_executions() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let root = AgentExecution::new_root(&session.id, "root");
        repo.create_execution(&root).unwrap();

        let child1 = AgentExecution::new_delegated(
            &session.id,
            "child1",
            &root.id,
            DelegationType::Sequential,
            "task1",
        );
        let child2 = AgentExecution::new_delegated(
            &session.id,
            "child2",
            &root.id,
            DelegationType::Parallel,
            "task2",
        );

        repo.create_execution(&child1).unwrap();
        repo.create_execution(&child2).unwrap();

        let children = repo.get_child_executions(&root.id).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn update_execution_status() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec = AgentExecution::new_root(&session.id, "agent");
        repo.create_execution(&exec).unwrap();

        repo.update_execution_status(&exec.id, ExecutionStatus::Running)
            .unwrap();
        let updated = repo.get_execution(&exec.id).unwrap().unwrap();
        assert_eq!(updated.status, ExecutionStatus::Running);
        assert!(updated.started_at.is_some());

        repo.update_execution_status(&exec.id, ExecutionStatus::Completed)
            .unwrap();
        let updated = repo.get_execution(&exec.id).unwrap().unwrap();
        assert_eq!(updated.status, ExecutionStatus::Completed);
        assert!(updated.completed_at.is_some());
    }

    #[test]
    fn update_execution_tokens() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec = AgentExecution::new_root(&session.id, "agent");
        repo.create_execution(&exec).unwrap();

        repo.update_execution_tokens(&exec.id, 200, 100).unwrap();

        let updated = repo.get_execution(&exec.id).unwrap().unwrap();
        assert_eq!(updated.tokens_in, 200);
        assert_eq!(updated.tokens_out, 100);
    }

    #[test]
    fn update_execution_tokens_refreshes_session_totals() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let mut root = AgentExecution::new_root(&session.id, "agent");
        root.id = "exec-root".to_string();
        root.tokens_in = 10;
        root.tokens_out = 5;
        repo.create_execution(&root).unwrap();

        let mut child = AgentExecution::new_delegated(
            &session.id,
            "research-agent",
            &root.id,
            DelegationType::Sequential,
            "research",
        );
        child.id = "exec-child".to_string();
        child.tokens_in = 20;
        child.tokens_out = 7;
        repo.create_execution(&child).unwrap();

        repo.update_session_tokens(&session.id).unwrap();
        let before = repo.get_session(&session.id).unwrap().unwrap();
        assert_eq!(before.total_tokens_in, 30);
        assert_eq!(before.total_tokens_out, 12);

        repo.update_execution_tokens(&root.id, 100, 50).unwrap();

        let after = repo.get_session(&session.id).unwrap().unwrap();
        assert_eq!(after.total_tokens_in, 120);
        assert_eq!(after.total_tokens_out, 57);
    }

    #[test]
    fn set_execution_error() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec = AgentExecution::new_root(&session.id, "agent");
        repo.create_execution(&exec).unwrap();

        repo.set_execution_error(&exec.id, "Something went wrong")
            .unwrap();

        let updated = repo.get_execution(&exec.id).unwrap().unwrap();
        assert_eq!(updated.error, Some("Something went wrong".to_string()));
        // Note: set_execution_error only sets the error message, not the status
        // Status should be updated separately via update_execution_status
    }

    // ========================================================================
    // Session with Executions Tests
    // ========================================================================

    #[test]
    fn get_session_with_executions() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let root = AgentExecution::new_root(&session.id, "root");
        let sub = AgentExecution::new_delegated(
            &session.id,
            "sub",
            &root.id,
            DelegationType::Sequential,
            "task",
        );

        repo.create_execution(&root).unwrap();
        repo.create_execution(&sub).unwrap();

        let result = repo.get_session_with_executions(&session.id).unwrap();
        assert!(result.is_some());

        let swe = result.unwrap();
        assert_eq!(swe.session.id, session.id);
        assert_eq!(swe.executions.len(), 2);
        assert_eq!(swe.subagent_count, 1); // One subagent
    }

    #[test]
    fn list_sessions_with_executions() {
        let repo = setup_repo();

        let s1 = Session::new("agent1");
        let s2 = Session::new("agent2");

        repo.create_session(&s1).unwrap();
        repo.create_session(&s2).unwrap();

        let e1 = AgentExecution::new_root(&s1.id, "agent1");
        let e2 = AgentExecution::new_root(&s2.id, "agent2");

        repo.create_execution(&e1).unwrap();
        repo.create_execution(&e2).unwrap();

        let filter = SessionFilter::default();
        let sessions = repo.list_sessions_with_executions(&filter).unwrap();
        assert_eq!(sessions.len(), 2);

        for swe in sessions {
            assert!(!swe.executions.is_empty());
        }
    }

    // ========================================================================
    // Dashboard Stats Tests
    // ========================================================================

    #[test]
    fn get_dashboard_stats_empty() {
        let repo = setup_repo();

        let stats = repo.get_dashboard_stats().unwrap();

        assert_eq!(stats.sessions_running, 0);
        assert_eq!(stats.sessions_queued, 0);
        assert_eq!(stats.sessions_completed, 0);
        assert_eq!(stats.executions_running, 0);
    }

    #[test]
    fn get_dashboard_stats_with_data() {
        let repo = setup_repo();

        // Create sessions with different statuses and sources
        let s1 = Session::new_with_source("agent1", TriggerSource::Web);
        let s2 = Session::new_with_source("agent2", TriggerSource::Web);
        let s3 = Session::new_with_source("agent3", TriggerSource::Cli);
        let s4 = Session::new_queued("agent4", TriggerSource::Cron);

        repo.create_session(&s1).unwrap();
        repo.create_session(&s2).unwrap();
        repo.create_session(&s3).unwrap();
        repo.create_session(&s4).unwrap();

        // Complete one session
        repo.update_session_status(&s2.id, SessionStatus::Completed)
            .unwrap();

        // Create executions
        let e1 = AgentExecution::new_root(&s1.id, "agent1");
        let e3 = AgentExecution::new_root(&s3.id, "agent3");

        repo.create_execution(&e1).unwrap();
        repo.create_execution(&e3).unwrap();

        // Start one execution
        repo.update_execution_status(&e1.id, ExecutionStatus::Running)
            .unwrap();

        let stats = repo.get_dashboard_stats().unwrap();

        assert_eq!(stats.sessions_running, 2); // s1, s3 (s2 completed, s4 queued)
        assert_eq!(stats.sessions_queued, 1); // s4
        assert_eq!(stats.sessions_completed, 1); // s2
        assert_eq!(stats.executions_running, 1); // e1
        assert_eq!(stats.executions_queued, 1); // e3 (not started yet)

        // Check by source
        assert_eq!(*stats.sessions_by_source.get("web").unwrap_or(&0), 2);
        assert_eq!(*stats.sessions_by_source.get("cli").unwrap_or(&0), 1);
        assert_eq!(*stats.sessions_by_source.get("cron").unwrap_or(&0), 1);
    }

    // ========================================================================
    // Checkpoint Tests
    // ========================================================================

    #[test]
    fn save_execution_checkpoint() {
        let repo = setup_repo();
        let session = Session::new("agent");
        repo.create_session(&session).unwrap();

        let exec = AgentExecution::new_root(&session.id, "agent");
        repo.create_execution(&exec).unwrap();

        let checkpoint = Checkpoint::new(5, "msg-last");
        repo.save_execution_checkpoint(&exec.id, &checkpoint)
            .unwrap();

        let updated = repo.get_execution(&exec.id).unwrap().unwrap();
        assert!(updated.checkpoint.is_some());

        let saved_checkpoint = updated.checkpoint.unwrap();
        assert_eq!(saved_checkpoint.llm_turn, 5);
        assert_eq!(saved_checkpoint.last_message_id, "msg-last");
    }

    // ========================================================================
    // Session Messages Tests
    // ========================================================================

    /// Helper to add a message to an execution
    fn add_message(
        repo: &StateRepository<TestDbProvider>,
        execution_id: &str,
        role: &str,
        content: &str,
    ) {
        repo.db.with_connection(|conn| {
            let id = format!("msg-{}", uuid::Uuid::new_v4());
            let created_at = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO messages (id, execution_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, execution_id, role, content, created_at],
            )?;
            Ok(())
        }).unwrap();
    }

    #[test]
    fn get_session_messages_all_scope() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();

        // Create root execution
        let mut root_exec = AgentExecution::new_root(&session.id, "claude");
        root_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&root_exec).unwrap();

        // Create subagent execution
        let mut sub_exec = AgentExecution::new_delegated(
            &session.id,
            "research-agent",
            &root_exec.id,
            DelegationType::Sequential,
            "Research AI",
        );
        sub_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub_exec).unwrap();

        // Add messages to both
        add_message(&repo, &root_exec.id, "user", "Hello root");
        add_message(&repo, &root_exec.id, "assistant", "Root response");
        add_message(&repo, &sub_exec.id, "user", "Research task");
        add_message(&repo, &sub_exec.id, "assistant", "Research result");

        // Get all messages
        let messages = repo
            .get_session_messages(&session.id, "all", None, None)
            .unwrap();
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn get_session_messages_root_scope() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();

        // Create root execution
        let mut root_exec = AgentExecution::new_root(&session.id, "claude");
        root_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&root_exec).unwrap();

        // Create subagent execution
        let mut sub_exec = AgentExecution::new_delegated(
            &session.id,
            "research-agent",
            &root_exec.id,
            DelegationType::Sequential,
            "Research AI",
        );
        sub_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub_exec).unwrap();

        // Add messages to both
        add_message(&repo, &root_exec.id, "user", "Hello root");
        add_message(&repo, &root_exec.id, "assistant", "Root response");
        add_message(&repo, &sub_exec.id, "user", "Research task");
        add_message(&repo, &sub_exec.id, "assistant", "Research result");

        // Get only root messages
        let messages = repo
            .get_session_messages(&session.id, "root", None, None)
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.delegation_type == "root"));
    }

    #[test]
    fn get_session_messages_execution_scope() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();

        // Create root execution
        let mut root_exec = AgentExecution::new_root(&session.id, "claude");
        root_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&root_exec).unwrap();

        // Create subagent execution
        let mut sub_exec = AgentExecution::new_delegated(
            &session.id,
            "research-agent",
            &root_exec.id,
            DelegationType::Sequential,
            "Research AI",
        );
        sub_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub_exec).unwrap();

        // Add messages
        add_message(&repo, &root_exec.id, "user", "Hello root");
        add_message(&repo, &sub_exec.id, "user", "Research task");
        add_message(&repo, &sub_exec.id, "assistant", "Research result");

        // Get only specific execution messages
        let messages = repo
            .get_session_messages(&session.id, "execution", Some(&sub_exec.id), None)
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.execution_id == sub_exec.id));
    }

    #[test]
    fn get_session_messages_delegates_scope() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();

        // Create root execution
        let mut root_exec = AgentExecution::new_root(&session.id, "claude");
        root_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&root_exec).unwrap();

        // Create two subagent executions
        let mut sub1 = AgentExecution::new_delegated(
            &session.id,
            "research-agent",
            &root_exec.id,
            DelegationType::Sequential,
            "Research",
        );
        sub1.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub1).unwrap();

        let mut sub2 = AgentExecution::new_delegated(
            &session.id,
            "writer-agent",
            &root_exec.id,
            DelegationType::Parallel,
            "Write",
        );
        sub2.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub2).unwrap();

        // Add messages
        add_message(&repo, &root_exec.id, "user", "Root message");
        add_message(&repo, &sub1.id, "assistant", "Research result");
        add_message(&repo, &sub2.id, "assistant", "Writing result");

        // Get only delegate messages
        let messages = repo
            .get_session_messages(&session.id, "delegates", None, None)
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.delegation_type != "root"));
    }

    #[test]
    fn get_session_messages_agent_filter() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).unwrap();

        // Create root execution
        let mut root_exec = AgentExecution::new_root(&session.id, "claude");
        root_exec.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&root_exec).unwrap();

        // Create two different agent executions
        let mut sub1 = AgentExecution::new_delegated(
            &session.id,
            "research-agent",
            &root_exec.id,
            DelegationType::Sequential,
            "Research 1",
        );
        sub1.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub1).unwrap();

        let mut sub2 = AgentExecution::new_delegated(
            &session.id,
            "writer-agent",
            &root_exec.id,
            DelegationType::Sequential,
            "Write",
        );
        sub2.started_at = Some(chrono::Utc::now().to_rfc3339());
        repo.create_execution(&sub2).unwrap();

        // Add messages
        add_message(&repo, &root_exec.id, "assistant", "Claude response");
        add_message(&repo, &sub1.id, "assistant", "Research result");
        add_message(&repo, &sub2.id, "assistant", "Writing result");

        // Filter by agent_id
        let messages = repo
            .get_session_messages(&session.id, "all", None, Some("research-agent"))
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent_id, "research-agent");
    }

    #[test]
    fn goal_artifacts_are_opt_in_bounded_and_queryable() {
        let repo = setup_repo();
        let session = Session::new("root-agent");
        repo.create_session(&session).expect("create session");

        let unmarked = Artifact::new(&session.id, "/ward/working-plan.md", "working-plan.md");
        repo.create_artifact(&unmarked)
            .expect("store unmarked artifact");

        for index in 0..StateRepository::<TestDbProvider>::MAX_GOAL_ARTIFACTS_PER_SESSION {
            let mut artifact = Artifact::new(
                &session.id,
                format!("/ward/result-{index}.json"),
                format!("result-{index}.json"),
            );
            artifact.is_goal_artifact = true;
            repo.create_artifact(&artifact)
                .expect("store goal artifact within limit");
        }

        let mut over_limit =
            Artifact::new(&session.id, "/ward/one-too-many.json", "one-too-many.json");
        over_limit.is_goal_artifact = true;
        assert!(repo.create_artifact(&over_limit).is_err());

        let all = repo
            .list_artifacts_by_session(&session.id)
            .expect("list all");
        assert_eq!(all.len(), 25);
        assert!(!all[0].is_goal_artifact);

        let goals = repo
            .list_goal_artifacts_by_session(&session.id, 24)
            .expect("list goals");
        assert_eq!(goals.len(), 24);
        assert!(goals.iter().all(|artifact| artifact.is_goal_artifact));
    }
}
