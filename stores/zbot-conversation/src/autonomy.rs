//! Durable cross-session decision threads and their audit history.

use anyhow::{bail, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OptionalExtension};

use crate::domain::{
    AutonomyApprovalPolicy, AutonomyEvidence, AutonomyItem, AutonomyRun, AutonomyState,
    LedgerResumePacket,
};

pub trait AutonomyStore: Send + Sync {
    fn create(&self, item: &AutonomyItem, evidence: &[AutonomyEvidence]) -> Result<()>;
    fn get(&self, item_id: &str) -> Result<Option<AutonomyItem>>;
    fn list_open(&self, limit: usize) -> Result<Vec<AutonomyItem>>;
    fn evidence(&self, item_id: &str) -> Result<Vec<AutonomyEvidence>>;
    fn runs(&self, item_id: &str) -> Result<Vec<AutonomyRun>>;
    /// Atomically verifies that an item is approved, builds its bounded packet,
    /// and records the user-requested resume attempt before execution starts.
    fn prepare_resume(&self, item_id: &str) -> Result<LedgerResumePacket>;
    fn transition(
        &self,
        item_id: &str,
        next: AutonomyState,
        outcome: Option<&str>,
    ) -> Result<AutonomyItem>;
}

pub struct SqliteAutonomyStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteAutonomyStore {
    #[must_use]
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }
}

impl AutonomyStore for SqliteAutonomyStore {
    fn create(&self, item: &AutonomyItem, evidence: &[AutonomyEvidence]) -> Result<()> {
        if item.state != AutonomyState::Proposed {
            bail!("new autonomy items must start proposed");
        }
        if evidence.iter().any(|entry| entry.item_id != item.id) {
            bail!("autonomy evidence must belong to its item");
        }
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO autonomy_items
                (id, title, objective, next_action, state, approval_policy,
                 source_session_id, dedupe_key, created_at, updated_at, completed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                item.id,
                item.title,
                item.objective,
                item.next_action,
                item.state.as_str(),
                item.approval_policy.as_str(),
                item.source_session_id,
                item.dedupe_key,
                item.created_at,
                item.updated_at,
                item.completed_at,
            ],
        )?;
        for entry in evidence {
            tx.execute(
                "INSERT INTO autonomy_evidence (id, item_id, kind, reference_id, label, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    entry.id,
                    entry.item_id,
                    entry.kind,
                    entry.reference_id,
                    entry.label,
                    entry.created_at
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO autonomy_runs (id, item_id, kind, to_state, created_at)
             VALUES (?, ?, 'created', ?, ?)",
            params![
                format!("run-{}", uuid::Uuid::now_v7()),
                item.id,
                item.state.as_str(),
                item.created_at
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn get(&self, item_id: &str) -> Result<Option<AutonomyItem>> {
        let conn = self.pool.get()?;
        load_item(&conn, item_id)
    }

    fn list_open(&self, limit: usize) -> Result<Vec<AutonomyItem>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, objective, next_action, state, approval_policy, source_session_id,
                    dedupe_key, created_at, updated_at, completed_at
             FROM autonomy_items WHERE state != 'complete'
             ORDER BY updated_at DESC LIMIT ?",
        )?;
        let rows = stmt.query_map([limit as i64], row_to_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn evidence(&self, item_id: &str) -> Result<Vec<AutonomyEvidence>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, item_id, kind, reference_id, label, created_at
             FROM autonomy_evidence WHERE item_id = ? ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([item_id], |row| {
            Ok(AutonomyEvidence {
                id: row.get(0)?,
                item_id: row.get(1)?,
                kind: row.get(2)?,
                reference_id: row.get(3)?,
                label: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn runs(&self, item_id: &str) -> Result<Vec<AutonomyRun>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, item_id, kind, from_state, to_state, outcome, created_at
             FROM autonomy_runs WHERE item_id = ? ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([item_id], row_to_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn prepare_resume(&self, item_id: &str) -> Result<LedgerResumePacket> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let item =
            load_item(&tx, item_id)?.ok_or_else(|| anyhow::anyhow!("autonomy item not found"))?;
        let evidence = load_evidence(&tx, item_id)?;
        let packet = LedgerResumePacket::from_approved_item(&item, &evidence)
            .map_err(|error| anyhow::anyhow!(error))?;
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO autonomy_runs (id, item_id, kind, from_state, to_state, outcome, created_at)
             VALUES (?, ?, 'resume_requested', ?, ?, 'user_requested', ?)",
            params![
                format!("run-{}", uuid::Uuid::now_v7()),
                item_id,
                AutonomyState::Approved.as_str(),
                AutonomyState::Approved.as_str(),
                now,
            ],
        )?;
        tx.commit()?;
        Ok(packet)
    }

    fn transition(
        &self,
        item_id: &str,
        next: AutonomyState,
        outcome: Option<&str>,
    ) -> Result<AutonomyItem> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let current =
            load_item(&tx, item_id)?.ok_or_else(|| anyhow::anyhow!("autonomy item not found"))?;
        if !current.state.can_transition_to(next) {
            bail!("invalid autonomy transition: {} -> {}", current.state, next);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let completed_at = if next == AutonomyState::Complete {
            Some(now.clone())
        } else {
            None
        };
        tx.execute(
            "UPDATE autonomy_items SET state = ?, updated_at = ?, completed_at = ? WHERE id = ?",
            params![next.as_str(), now, completed_at, item_id],
        )?;
        tx.execute(
            "INSERT INTO autonomy_runs (id, item_id, kind, from_state, to_state, outcome, created_at)
             VALUES (?, ?, 'transition', ?, ?, ?, ?)",
            params![format!("run-{}", uuid::Uuid::now_v7()), item_id, current.state.as_str(), next.as_str(), outcome, now],
        )?;
        let updated = load_item(&tx, item_id)?.expect("item exists after update");
        tx.commit()?;
        Ok(updated)
    }
}

fn load_item(conn: &rusqlite::Connection, item_id: &str) -> Result<Option<AutonomyItem>> {
    conn.query_row(
        "SELECT id, title, objective, next_action, state, approval_policy, source_session_id,
                dedupe_key, created_at, updated_at, completed_at
         FROM autonomy_items WHERE id = ?",
        [item_id],
        row_to_item,
    )
    .optional()
    .map_err(Into::into)
}

fn load_evidence(conn: &rusqlite::Connection, item_id: &str) -> Result<Vec<AutonomyEvidence>> {
    let mut stmt = conn.prepare(
        "SELECT id, item_id, kind, reference_id, label, created_at
         FROM autonomy_evidence WHERE item_id = ? ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([item_id], |row| {
        Ok(AutonomyEvidence {
            id: row.get(0)?,
            item_id: row.get(1)?,
            kind: row.get(2)?,
            reference_id: row.get(3)?,
            label: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutonomyItem> {
    let state: String = row.get(4)?;
    let policy: String = row.get(5)?;
    Ok(AutonomyItem {
        id: row.get(0)?,
        title: row.get(1)?,
        objective: row.get(2)?,
        next_action: row.get(3)?,
        state: AutonomyState::parse(&state).ok_or(rusqlite::Error::InvalidQuery)?,
        approval_policy: AutonomyApprovalPolicy::parse(&policy)
            .ok_or(rusqlite::Error::InvalidQuery)?,
        source_session_id: row.get(6)?,
        dedupe_key: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        completed_at: row.get(10)?,
    })
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutonomyRun> {
    let from: Option<String> = row.get(3)?;
    let to: Option<String> = row.get(4)?;
    Ok(AutonomyRun {
        id: row.get(0)?,
        item_id: row.get(1)?,
        kind: row.get(2)?,
        from_state: from.as_deref().and_then(AutonomyState::parse),
        to_state: to.as_deref().and_then(AutonomyState::parse),
        outcome: row.get(5)?,
        created_at: row.get(6)?,
    })
}
