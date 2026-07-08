//! `MessageStore` — append-only conversational message log (LLM-facing).
//!
//! `seq` is assigned atomically inside the INSERT (server-side subquery), so
//! concurrent appends cannot collide on `seq` (no `next_seq`-then-`append`
//! TOCTOU). SQLite WAL serializes writers; `busy_timeout` handles contention.

use crate::domain::Message;
use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub trait MessageStore: Send + Sync {
    /// Append a message. `msg.seq` is ignored — `seq` is assigned atomically
    /// server-side.
    fn append(&self, msg: &Message) -> Result<()>;

    /// Replay messages for a session, ordered by `seq`. If `after_seq` is set,
    /// return only rows with `seq > after_seq` (cursor pagination).
    fn replay(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Message>>;

    /// The ordered sequence of tool names invoked in a session's assistant
    /// turns (parses `tool_calls` blobs). Mirrors the legacy
    /// `ConversationStore::tool_sequence_for_session`.
    fn tool_sequence_for_session(&self, session_id: &str) -> Result<Vec<String>>;
}

pub struct SqliteMessageStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteMessageStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }
}

impl MessageStore for SqliteMessageStore {
    fn append(&self, msg: &Message) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO messages
                (id, execution_id, session_id, role, content, created_at,
                 token_count, tool_calls, tool_call_id, seq)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?,
                     (SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?))",
            rusqlite::params![
                msg.id,
                msg.execution_id,
                msg.session_id,
                msg.role,
                msg.content,
                msg.created_at,
                msg.token_count,
                msg.tool_calls,
                msg.tool_call_id,
                msg.session_id, // binds the subquery's WHERE
            ],
        )?;
        Ok(())
    }

    fn replay(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let conn = self.pool.get()?;
        let mut sql = String::from(
            "SELECT id, execution_id, session_id, role, content, created_at, token_count,
                    tool_calls, tool_call_id, seq
             FROM messages WHERE session_id = ?",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(session_id.to_string())];
        if let Some(s) = after_seq {
            sql.push_str(" AND seq > ?");
            params.push(Box::new(s));
        }
        sql.push_str(" ORDER BY seq ASC LIMIT ?");
        params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |r| {
            Ok(Message {
                id: r.get(0)?,
                execution_id: r.get(1)?,
                session_id: r.get(2)?,
                role: r.get(3)?,
                content: r.get(4)?,
                created_at: r.get(5)?,
                token_count: r.get(6)?,
                tool_calls: r.get(7)?,
                tool_call_id: r.get(8)?,
                seq: r.get(9)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    fn tool_sequence_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT tool_calls FROM messages
             WHERE session_id = ? AND role = 'assistant' AND tool_calls IS NOT NULL
             ORDER BY seq ASC",
        )?;
        let blobs: Vec<String> = stmt
            .query_map([session_id], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        let mut seq = Vec::new();
        for blob in blobs {
            extend_tool_names(&blob, &mut seq);
        }
        Ok(seq)
    }
}

/// Parse the stored `tool_calls` format `[{"tool_name": "...", ...}, ...]` and
/// append tool names in order.
fn extend_tool_names(blob: &str, out: &mut Vec<String>) {
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(blob) else {
        return;
    };
    for v in arr {
        if let Some(name) = v.get("tool_name").and_then(|n| n.as_str()) {
            out.push(name.to_string());
        }
    }
}
