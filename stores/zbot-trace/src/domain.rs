//! Trace domain types.
//!
//! `SlimLog` is the payload-free row the live `/api/logs` UI reads (columns
//! unchanged from the legacy `execution_logs` table; only the *writers* stop
//! emitting payload blobs into `metadata`). `TraceEvent` is the full-fidelity
//! OTel-GenAI-shaped record streamed to `traces/<session_id>.jsonl.gz`.

use serde::{Deserialize, Serialize};

/// One payload-free `execution_logs` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlimLog {
    pub id: String,
    pub session_id: String,
    pub conversation_id: Option<String>,
    pub agent_id: String,
    pub parent_session_id: Option<String>,
    pub timestamp: String,
    /// info | warn | error
    pub level: String,
    /// session | token | tool_call | tool_result | thinking | delegation |
    /// system | error | response | intent
    pub category: String,
    /// Short human-readable string.
    pub message: String,
    /// Display scalars ONLY — `{tool_name}`, `{tool_id}`, `{error}`,
    /// `{blocked_by_hook}`. Never tool args/result payloads.
    pub metadata: Option<String>,
    pub duration_ms: Option<i64>,
}

/// A full-fidelity trace event, written to `.jsonl.gz`. Attribute names
/// follow OpenTelemetry GenAI semantic conventions for portability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub trace_id: String,
    pub span_id: String,
    pub session_id: String,
    pub execution_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub timestamp: String,
    pub level: String,
    pub category: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// OTel `tool.name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// OTel `gen_ai.tool.call.input` / `.output` — full args/results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// OTel `gen_ai.usage.*`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    /// OTel `gen_ai.request.model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}
