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

/// Durable operational state for a user-approved decision thread. This is not
/// semantic memory: it records work zbot currently owes the user.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyState {
    Proposed,
    Approved,
    Blocked,
    Complete,
    Stale,
}

impl AutonomyState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Blocked => "blocked",
            Self::Complete => "complete",
            Self::Stale => "stale",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "proposed" => Some(Self::Proposed),
            "approved" => Some(Self::Approved),
            "blocked" => Some(Self::Blocked),
            "complete" => Some(Self::Complete),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Proposed,
                Self::Approved | Self::Complete | Self::Stale
            ) | (Self::Approved, Self::Blocked | Self::Complete | Self::Stale)
                | (Self::Blocked, Self::Approved | Self::Complete | Self::Stale)
                | (Self::Stale, Self::Approved | Self::Complete)
        )
    }
}

impl std::fmt::Display for AutonomyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether an item may be invoked without a new approval prompt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyApprovalPolicy {
    Manual,
    AskOnce,
    AutoReadonly,
}

impl AutonomyApprovalPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AskOnce => "ask_once",
            Self::AutoReadonly => "auto_readonly",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(Self::Manual),
            "ask_once" => Some(Self::AskOnce),
            "auto_readonly" => Some(Self::AutoReadonly),
            _ => None,
        }
    }
}

/// A compact cross-session decision or explicitly approved follow-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutonomyItem {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub next_action: String,
    pub state: AutonomyState,
    pub approval_policy: AutonomyApprovalPolicy,
    pub source_session_id: Option<String>,
    pub dedupe_key: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

/// Reference-only evidence. The ledger never copies a source transcript or
/// artifact payload into its own tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutonomyEvidence {
    pub id: String,
    pub item_id: String,
    pub kind: String,
    pub reference_id: String,
    pub label: Option<String>,
    pub created_at: String,
}

/// Auditable lifecycle transition or bounded execution attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutonomyRun {
    pub id: String,
    pub item_id: String,
    pub kind: String,
    pub from_state: Option<AutonomyState>,
    pub to_state: Option<AutonomyState>,
    pub outcome: Option<String>,
    pub created_at: String,
}
