//! Backend-agnostic domain types for the conversation store.

use serde::{Deserialize, Serialize};

/// A single conversational message. Append-only — the write path never
/// `UPDATE`s or `DELETE`s a message row. The legacy `tool_results` column is
/// intentionally absent (dropped at the v1 schema); the wire DTO keeps a
/// `tool_results` field mapped to `None` for API-contract stability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// `msg-<uuid>` (wire shape preserved from the legacy repository).
    pub id: String,
    pub execution_id: Option<String>,
    pub session_id: String,
    /// user | assistant | tool | system
    pub role: String,
    pub content: String,
    /// RFC3339 timestamp.
    pub created_at: String,
    pub token_count: i64,
    /// JSON array of tool calls, assistant turns only.
    pub tool_calls: Option<String>,
    /// Links a `role=tool` row back to its assistant tool call.
    pub tool_call_id: Option<String>,
    /// Per-session monotonic counter; assigned atomically inside the INSERT.
    pub seq: i64,
}

/// Versioned state checkpoint. Promotes the legacy `Checkpoint` struct
/// (`services/execution-state/src/types.rs:667`) into a first-class versioned
/// row. `context_state` is a typed JSON snapshot written at each turn boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub execution_id: String,
    pub session_id: String,
    pub llm_turn: u32,
    pub last_message_id: String,
    /// JSON array of in-flight tool calls.
    pub pending_tool_calls: Option<String>,
    /// JSON snapshot: {intent, ward, plan, recalled_facts, response, title,
    /// model, subagents}. Written at the turn boundary.
    pub context_state: Option<String>,
    /// JSON array of active child execution ids.
    pub child_executions: Option<String>,
    pub schema_version: i64,
    pub created_at: String,
}
