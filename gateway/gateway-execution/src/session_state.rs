//! # Session State Builder
//!
//! Assembles a structured `SessionState` snapshot for Mission Control. The
//! HTTP handler at `/api/logs/sessions/:id` calls
//! `SessionStateBuilder::build(session_id)` and serialises the result to JSON.
//!
//! **Slice 3 (T11 redo):** tool-payload-derived fields (`response`, `plan`,
//! `recalled_facts`) read from the **messages** table via `MessageStore::replay`
//! — the durable, full-fidelity source. `ward` and `title` prefer the `sessions`
//! row (persisted by `WardChanged`/`SessionTitleChanged` handlers). Non-tool
//! metadata (`intent`, `model`, delegation `task`) stay on `execution_logs`
//! (unaffected by the slim). The previous attempt read a `context_state`
//! checkpoint snapshot that was under-sourced for delegation sessions — this
//! reroute reads messages directly so delegation-based sessions render
//! correctly (their plan arrives in a `role=system` delegation-result message,
//! not an `update_plan` tool call).

use std::sync::Arc;

use api_logs::{ExecutionLog, LogCategory, LogService, SessionStatus};
use execution_state::StateService;
use serde::Serialize;
use zbot_conversation::{Message, MessageStore};
use zbot_stores_sqlite::{ConversationRepository, DatabaseManager};

// ============================================================================
// TYPES
// ============================================================================

/// Top-level session state returned by the API.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// Session metadata (id, title, status, timing, tokens).
    pub session: SessionMeta,
    /// The user message that triggered this execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    /// Current execution phase.
    pub phase: SessionPhase,
    /// Final response text (from `respond` tool or last assistant message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    /// Intent analysis metadata (the JSON blob from the intent log).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_analysis: Option<serde_json::Value>,
    /// Ward that was selected for this execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ward: Option<WardInfo>,
    /// Facts recalled from memory.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recalled_facts: Vec<String>,
    /// Plan steps (from the latest `update_plan` tool call).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<PlanStep>,
    /// Subagent executions spawned by delegation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentState>,
    /// Whether the session is still running.
    pub is_live: bool,
}

/// Compact session metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    pub token_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Execution phase — a coarse state-machine label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionPhase {
    Intent,
    Planning,
    Executing,
    Responding,
    Completed,
    Error,
}

/// Information about the ward selected for this execution.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WardInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// A single step in the agent's plan.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// State of a delegated subagent execution.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentState {
    pub agent_id: String,
    pub execution_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    pub token_count: i32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallEntry>,
}

/// A single tool call within a subagent execution.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEntry {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Tool input (args), sourced from the assistant `tool_calls` JSON (messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Tool output (result), sourced from the matching `role=tool` message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

// ============================================================================
// BUILDER
// ============================================================================

/// Tools that are considered "internal" and do not count as real execution.
const INTERNAL_TOOLS: &[&str] = &["analyze_intent", "update_plan", "set_session_title"];

/// Assembles a [`SessionState`] from the logs database, the messages store,
/// and the sessions table.
///
/// **Field sourcing (Slice 3 redo):**
/// - `intent`, `model` ← `execution_logs` (un-slimmed Intent/model metadata).
/// - `subagents` ← `log_service` child-session enumeration (unchanged).
/// - `response`, `plan`, `recalled_facts` ← `MessageStore::replay` (durable,
///   shape-agnostic; handles delegation `system` messages too).
/// - `ward`, `title` ← `state_service.get_session()` (persisted by
///   `WardChanged`/`SessionTitleChanged` handlers), fallback to intent.
/// - `user_message`, `token_count` ← `ConversationRepository` (unchanged —
///   T13's concern; the new MessageStore reads the same table).
pub struct SessionStateBuilder {
    log_service: Arc<LogService<DatabaseManager>>,
    conversations: Arc<ConversationRepository>,
    messages: Arc<dyn MessageStore>,
    state_service: Arc<StateService<DatabaseManager>>,
}

impl SessionStateBuilder {
    /// Create a new builder.
    pub fn new(
        log_service: Arc<LogService<DatabaseManager>>,
        conversations: Arc<ConversationRepository>,
        messages: Arc<dyn MessageStore>,
        state_service: Arc<StateService<DatabaseManager>>,
    ) -> Self {
        Self {
            log_service,
            conversations,
            messages,
            state_service,
        }
    }

    /// Build a complete [`SessionState`] for the given session.
    ///
    /// Returns `None` when the session does not exist in the logs database.
    pub fn build(&self, session_id: &str) -> Result<Option<SessionState>, String> {
        let detail = match self.log_service.get_session_detail(session_id)? {
            Some(d) => d,
            None => return Ok(None),
        };

        let session = &detail.session;
        let logs = &detail.logs;

        // Messages are keyed by conversation_id (sess-xxx); session.session_id is
        // the execution id (exec-xxx) that execution_logs use. Replay by
        // conversation_id so root messages resolve. Child-session messages are
        // replayed lazily for response fallback only.
        let root_messages = self
            .messages
            .replay(&session.conversation_id, None, 10_000)
            .unwrap_or_default();

        // user_message and token_count still come from ConversationRepository
        // (T13's concern — same table, different reader). Kept here to avoid
        // touching T13 in this slice.
        let user_message = self.extract_user_message(&session.session_id);
        let intent_analysis = Self::extract_intent(logs);

        // ward / title: prefer the sessions row (WardChanged /
        // SessionTitleChanged handlers), fallback to legacy log scan, then
        // intent.
        let session_row = self.state_service.get_session(&session.conversation_id).ok().flatten();
        let ward = session_row
            .as_ref()
            .and_then(|s| s.ward_id.clone())
            .map(|name| WardInfo { name, content: None })
            .or_else(|| Self::extract_ward(logs, intent_analysis.as_ref()));
        let title = session_row
            .as_ref()
            .and_then(|s| s.title.clone())
            .or_else(|| Self::extract_title(logs, intent_analysis.as_ref()));

        // Tool-payload-derived fields: read messages (Slice 3 redo).
        let plan = extract_plan_from_messages(&root_messages);
        let recalled_facts = extract_recalled_facts_from_messages(&root_messages);
        let response = extract_response_from_messages(&root_messages)
            .or_else(|| self.response_from_child_messages(&session.child_session_ids))
            .or_else(|| Self::response_from_logs_fallback(logs, &session.session_id));

        let subagents = self.build_subagents(&session.child_session_ids);
        let phase = Self::derive_phase(&session.status, logs, response.as_ref());
        let model = Self::extract_model(logs);

        Ok(Some(SessionState {
            session: SessionMeta {
                id: session.session_id.clone(),
                title,
                status: session.status.as_str().to_string(),
                started_at: session.started_at.clone(),
                duration_ms: session.duration_ms,
                // LogSession.token_count is often 0 — sum from messages table instead
                token_count: self
                    .sum_token_count(&session.session_id, &session.child_session_ids)
                    .unwrap_or(session.token_count),
                model,
            },
            user_message,
            phase,
            response,
            intent_analysis,
            ward,
            recalled_facts,
            // If session is completed, mark all plan steps as done
            plan: if matches!(phase, SessionPhase::Completed) {
                plan.into_iter()
                    .map(|mut s| {
                        s.status = Some("completed".to_string());
                        s
                    })
                    .collect()
            } else {
                plan
            },
            subagents,
            is_live: session.status == SessionStatus::Running,
        }))
    }

    // ========================================================================
    // EXTRACTION HELPERS
    // ========================================================================

    /// Title-length cap for the intent-analysis fallback. Keeps UI rows short
    /// without depending on the retired model-visible title tool.
    const INTENT_TITLE_MAX_CHARS: usize = 80;

    /// Extract session title. Replays legacy `set_session_title` tool-call logs
    /// from old conversation DBs, then falls back to intent analysis so new
    /// sessions still land with a meaningful title instead of null.
    fn extract_title(logs: &[ExecutionLog], intent: Option<&serde_json::Value>) -> Option<String> {
        for log in logs {
            if log.category == LogCategory::ToolCall {
                if let Some(meta) = &log.metadata {
                    let tool = meta.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                    if tool == "set_session_title" {
                        if let Some(title) = meta
                            .get("args")
                            .and_then(|a| a.get("title").or_else(|| a.get("name")))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                        {
                            return Some(title);
                        }
                    }
                }
            }
        }
        Self::title_from_intent(intent)
    }

    fn title_from_intent(intent: Option<&serde_json::Value>) -> Option<String> {
        let primary = intent?
            .get("primary_intent")
            .and_then(|v| v.as_str())?
            .trim();
        if primary.is_empty() {
            return None;
        }
        let truncated: String = primary.chars().take(Self::INTENT_TITLE_MAX_CHARS).collect();
        if truncated.len() < primary.len() {
            Some(format!(
                "{}…",
                truncated.trim_end_matches(|c: char| c.is_whitespace() || c == ',')
            ))
        } else {
            Some(truncated)
        }
    }

    /// Sum token counts from the messages table for this execution.
    fn sum_token_count(&self, execution_id: &str, child_session_ids: &[String]) -> Option<i32> {
        let mut total: i32 = 0;
        // Root session tokens
        if let Ok(messages) = self.conversations.get_messages(execution_id) {
            total += messages.iter().map(|m| m.token_count).sum::<i32>();
        }
        // Child session tokens
        for child_id in child_session_ids {
            if let Ok(messages) = self.conversations.get_messages(child_id) {
                total += messages.iter().map(|m| m.token_count).sum::<i32>();
            }
        }
        if total > 0 {
            Some(total)
        } else {
            None
        }
    }

    /// Extract the first user message from the conversation messages table.
    fn extract_user_message(&self, conversation_id: &str) -> Option<String> {
        let messages = self.conversations.get_messages(conversation_id).ok()?;
        messages
            .into_iter()
            .find(|m| m.role == "user")
            .map(|m| m.content)
    }

    /// Extract intent analysis metadata from the first Intent-category log.
    fn extract_intent(logs: &[ExecutionLog]) -> Option<serde_json::Value> {
        logs.iter()
            .find(|l| l.category == LogCategory::Intent)
            .and_then(|l| l.metadata.clone())
    }

    /// Extract ward info. Primary source is `sessions.ward_id` (handled by the
    /// caller via `state_service`); this fallback tries the intent-analysis
    /// `ward_recommendation` block. The legacy ward-tool-call scan is gone
    /// (its `args` are slimmed out of `execution_logs.metadata` — full args
    /// now live only in `messages.tool_calls`, which the caller does not
    /// thread into this helper).
    fn extract_ward(logs: &[ExecutionLog], intent: Option<&serde_json::Value>) -> Option<WardInfo> {
        // Legacy fallback: extract from intent analysis metadata. The Intent
        // log is un-slimmed and still carries ward_recommendation.
        if let Some(intent_val) = intent {
            if let Some(ward_name) = intent_val
                .get("ward_recommendation")
                .and_then(|wr| wr.get("ward_name"))
                .or_else(|| intent_val.get("ward"))
                .and_then(|v| v.as_str())
            {
                return Some(WardInfo {
                    name: ward_name.to_string(),
                    content: None,
                });
            }
        }

        // Allow other callers (tests, ad-hoc) to surface a ward from a
        // model-emitted metadata blob if present. This is best-effort and
        // does not depend on the slimmed tool-call args.
        for log in logs {
            if let Some(meta) = log.metadata.as_ref() {
                if let Some(ward_name) = meta.get("ward").and_then(|v| v.as_str()) {
                    return Some(WardInfo {
                        name: ward_name.to_string(),
                        content: None,
                    });
                }
            }
        }

        None
    }

    /// Extract the model from the first log entry that carries model metadata.
    fn extract_model(logs: &[ExecutionLog]) -> Option<String> {
        for log in logs {
            if let Some(meta) = &log.metadata {
                if let Some(model) = meta.get("model").and_then(|v| v.as_str()) {
                    return Some(model.to_string());
                }
            }
        }
        None
    }

    /// Scan child-session messages for a non-empty final assistant response.
    /// Used when the root session has no final assistant message — a subagent
    /// may have produced the user-facing response (e.g. via `respond`).
    fn response_from_child_messages(&self, child_session_ids: &[String]) -> Option<String> {
        for child_id in child_session_ids {
            let msgs = self
                .messages
                .replay(child_id, None, 10_000)
                .unwrap_or_default();
            if let Some(resp) = extract_response_from_messages(&msgs) {
                return Some(resp);
            }
        }
        None
    }

    /// Logs-only fallback for `response`: the slimmed `execution_logs` no
    /// longer carry tool `args`, so this scans only for Response-category
    /// logs (a separate, un-slimmed channel) and the assistant `respond`
    /// tool_call indicator (which tells us the respond tool was invoked, but
    /// not its text — the text now lives only in `messages`).
    fn response_from_logs_fallback(logs: &[ExecutionLog], _execution_id: &str) -> Option<String> {
        for log in logs.iter().rev() {
            if log.category == LogCategory::Response && !log.message.is_empty() {
                return Some(log.message.clone());
            }
        }
        None
    }

    /// Build subagent state for each child session.
    fn build_subagents(&self, child_session_ids: &[String]) -> Vec<SubagentState> {
        let mut subagents = Vec::new();

        for child_id in child_session_ids {
            let detail = match self.log_service.get_session_detail(child_id) {
                Ok(Some(d)) => d,
                _ => continue,
            };

            let child_session = &detail.session;
            let child_logs = &detail.logs;
            // Child messages (keyed by conversation_id / sess-*) for tool-call input/output.
            let child_messages = self
                .messages
                .replay(&child_session.conversation_id, None, 10_000)
                .unwrap_or_default();

            // Extract the delegation task: check parent's delegation logs (metadata.task),
            // then child's delegation logs, then fall back to agent_executions.task via child session
            let task = self.extract_delegation_task(child_id, &child_session.agent_id, child_logs);

            // Build tool call entries for this subagent (input/output from messages)
            let tool_calls = Self::build_tool_calls(child_logs, &child_messages);

            subagents.push(SubagentState {
                agent_id: child_session.agent_id.clone(),
                execution_id: child_session.session_id.clone(),
                task,
                status: child_session.status.as_str().to_string(),
                duration_ms: child_session.duration_ms,
                token_count: child_session.token_count,
                tool_calls,
            });
        }

        subagents
    }

    /// Extract the delegation task for a child session.
    /// Checks parent's delegation logs (metadata.task matching child_agent),
    /// then child's own delegation/session logs.
    fn extract_delegation_task(
        &self,
        _child_session_id: &str,
        child_agent_id: &str,
        child_logs: &[ExecutionLog],
    ) -> Option<String> {
        // Try to find the parent session and its delegation logs
        // The child's logs contain parent_session_id references
        if let Some(parent_sid) = child_logs
            .iter()
            .find_map(|l| l.parent_session_id.as_deref())
        {
            if let Ok(Some(parent_detail)) = self.log_service.get_session_detail(parent_sid) {
                // Find delegation log in parent matching this child agent
                for log in &parent_detail.logs {
                    if log.category == LogCategory::Delegation {
                        if let Some(meta) = &log.metadata {
                            let agent = meta
                                .get("child_agent")
                                .or_else(|| meta.get("child_agent_id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if agent == child_agent_id {
                                // Check metadata.task first, then message
                                if let Some(task) = meta.get("task").and_then(|v| v.as_str()) {
                                    return Some(task.chars().take(200).collect());
                                }
                                if !log.message.is_empty() && log.message != "Session started" {
                                    return Some(log.message.chars().take(200).collect());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fallback: first non-"Session started" delegation or session log in child
        child_logs
            .iter()
            .find(|l| {
                (l.category == LogCategory::Delegation || l.category == LogCategory::Session)
                    && !l.message.is_empty()
                    && l.message != "Session started"
                    && l.message != "Execution completed successfully"
            })
            .map(|l| l.message.chars().take(200).collect())
    }

    /// Build tool call entries. Input (args) comes from the assistant `tool_calls`
    /// JSON in messages; output (result) from the matching `role=tool` message
    /// (linked by `tool_call_id`); `duration_ms` from tool_result logs.
    fn build_tool_calls(logs: &[ExecutionLog], messages: &[Message]) -> Vec<ToolCallEntry> {
        use std::collections::HashMap;

        // duration_ms per tool_id, from tool_result logs.
        let duration: HashMap<&str, i64> = logs
            .iter()
            .filter(|l| l.category == LogCategory::ToolResult)
            .filter_map(|l| {
                let id = l.metadata.as_ref()?.get("tool_id")?.as_str()?;
                l.duration_ms.map(|d| (id, d))
            })
            .collect();

        // output (result) per tool_call_id, from role=tool messages.
        let outputs: HashMap<&str, &str> = messages
            .iter()
            .filter(|m| m.role == "tool")
            .filter_map(|m| m.tool_call_id.as_deref().map(|id| (id, m.content.as_str())))
            .collect();

        let mut entries = Vec::new();
        for m in messages.iter().filter(|m| m.role == "assistant") {
            let Some(tc_json) = m.tool_calls.as_deref() else {
                continue;
            };
            let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(tc_json) else {
                continue;
            };
            for tc in arr {
                let tool_name = tc
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                if INTERNAL_TOOLS.contains(&tool_name.as_str()) {
                    continue;
                }
                let tool_id = tc.get("tool_id").and_then(|v| v.as_str());
                let input = tc.get("args").map(|a| a.to_string());
                let output = tool_id.and_then(|id| outputs.get(id)).map(|s| s.to_string());
                let duration_ms = tool_id.and_then(|id| duration.get(id).copied());
                entries.push(ToolCallEntry {
                    tool_name,
                    status: Some("completed".to_string()),
                    duration_ms,
                    summary: None,
                    input,
                    output,
                });
            }
        }

        entries
    }

    // ========================================================================
    // PHASE DERIVATION
    // ========================================================================

    /// Derive the current execution phase from status and logs.
    ///
    /// Logic:
    /// - `completed` / `stopped` → `Completed`
    /// - `error` → `Error`
    /// - has `respond` tool call or assistant message → `Responding`
    /// - has delegation or non-internal tool calls → `Executing`
    /// - has `update_plan` tool call → `Planning`
    /// - otherwise → `Intent`
    fn derive_phase(
        status: &SessionStatus,
        logs: &[ExecutionLog],
        response: Option<&String>,
    ) -> SessionPhase {
        // Terminal states
        match status {
            SessionStatus::Completed | SessionStatus::Stopped => return SessionPhase::Completed,
            SessionStatus::Error => return SessionPhase::Error,
            _ => {}
        }

        // Has respond tool or response content → Responding
        let has_respond_tool = logs.iter().any(|l| {
            l.category == LogCategory::ToolCall
                && l.metadata
                    .as_ref()
                    .and_then(|m| m.get("tool_name"))
                    .and_then(|v| v.as_str())
                    == Some("respond")
        });

        if has_respond_tool || response.is_some() {
            return SessionPhase::Responding;
        }

        // Has delegation or non-internal tool calls → Executing
        let has_delegation = logs.iter().any(|l| l.category == LogCategory::Delegation);

        let has_external_tool = logs.iter().any(|l| {
            l.category == LogCategory::ToolCall
                && l.metadata
                    .as_ref()
                    .and_then(|m| m.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .map(|name| !INTERNAL_TOOLS.contains(&name))
                    .unwrap_or(false)
        });

        if has_delegation || has_external_tool {
            return SessionPhase::Executing;
        }

        // Has update_plan → Planning
        let has_plan = logs.iter().any(|l| {
            l.category == LogCategory::ToolCall
                && l.metadata
                    .as_ref()
                    .and_then(|m| m.get("tool_name"))
                    .and_then(|v| v.as_str())
                    == Some("update_plan")
        });

        if has_plan {
            return SessionPhase::Planning;
        }

        // Default
        SessionPhase::Intent
    }
}

// ============================================================================
// MESSAGES-BASED EXTRACTION (tool-payload fields — Slice 3 redo)
// ============================================================================
//
// These free functions read the **messages** table (the durable, full-fidelity
// source) instead of the slimmed `execution_logs.metadata`. They are robust
// to delegation-based sessions — `extract_plan_from_messages` parses plan
// steps from BOTH `update_plan` tool_calls AND delegation `system` messages
// (real sessions rarely emit `update_plan`; the planner-agent's plan arrives
// as a `role=system` message whose content embeds the plan markdown).

/// Extract plan steps from the messages of a session.
///
/// Source priority (last-wins, mirroring the legacy log-based scan):
/// 1. The **last** assistant `update_plan` tool_call — its `args.steps` JSON.
/// 2. Delegation `system` messages — a delegation's plan arrives as a
///    `role=system` message; we parse plan-like lines from the embedded
///    content (e.g. the continuation prompt at `core.rs:319` carries the
///    full plan markdown, and the `## From Planner Agent` callback carries
///    the planner's response).
///
/// Returns `Vec::new()` when no plan-shaped content was found (e.g. a fresh
/// session still in the Intent phase).
pub fn extract_plan_from_messages(messages: &[Message]) -> Vec<PlanStep> {
    let mut steps: Vec<PlanStep> = Vec::new();

    // (1) update_plan tool_calls in assistant messages — full args live here
    // after the slim (the slim only affects execution_logs.metadata).
    for msg in messages.iter().rev() {
        if msg.role != "assistant" {
            continue;
        }
        let Some(tc_json) = msg.tool_calls.as_deref() else {
            continue;
        };
        let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(tc_json) else {
            continue;
        };
        for call in calls.iter().rev() {
            let tool_name = call
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if tool_name == "update_plan" {
                let args = call.get("args").unwrap_or(&serde_json::Value::Null);
                let parsed = parse_plan_steps_from_json(args);
                if !parsed.is_empty() {
                    return parsed;
                }
            }
        }
    }

    // (2) delegation `system` messages. Real sessions are delegation-based:
    // the plan arrives as the content of a system message. We scan every
    // system message for plan-shaped lines and merge them. Two known shapes:
    //   - "## From Planner Agent\n\n{response}\n\n---..." (callback format)
    //   - "[DELEGATION COMPLETED. YOUR PLAN IS BELOW.\n...\n\n{plan_md}]" (continuation)
    if steps.is_empty() {
        for msg in messages {
            if msg.role != "system" {
                continue;
            }
            let parsed = parse_plan_from_system_content(&msg.content);
            if !parsed.is_empty() {
                steps = parsed;
                break;
            }
        }
    }

    steps
}

/// Parse plan steps from a JSON `args` blob (`{steps: [...]}` or
/// `{plan: [...]}`).
fn parse_plan_steps_from_json(args: &serde_json::Value) -> Vec<PlanStep> {
    let steps_val = args
        .get("steps")
        .or_else(|| args.get("plan"))
        .and_then(|v| v.as_array());

    let Some(arr) = steps_val else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|v| {
            if let Some(s) = v.as_str() {
                Some(PlanStep {
                    text: s.to_string(),
                    status: None,
                })
            } else if let Some(obj) = v.as_object() {
                let text = obj
                    .get("text")
                    .or_else(|| obj.get("step"))
                    .or_else(|| obj.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(unknown)")
                    .to_string();
                let status = obj
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(PlanStep { text, status })
            } else {
                None
            }
        })
        .collect()
}

/// Parse plan-like content from a delegation `system` message.
///
/// Heuristics (best-effort; the system message carries free-form markdown):
/// - Strip the `## From X` / `[DELEGATION COMPLETED...]` envelope.
/// - Collect non-empty lines that look like plan steps — numbered
///   (`1.`, `Step 1:`, `## Step 1`) or bulleted (`-`, `*`).
/// - If no list-like lines are found AND the content is non-trivial, emit the
///   trimmed content as a single step (so Mission Control renders *something*
///   for a planner delegation rather than a blank plan).
///
/// Slices on common planner outputs: `plan.md` headers (`## Steps`,
/// `## Step N`), numbered lists, and the inline continuation envelope.
fn parse_plan_from_system_content(content: &str) -> Vec<PlanStep> {
    let mut steps: Vec<PlanStep> = Vec::new();

    // Strip the leading envelope: either "[DELEGATION COMPLETED..." or
    // "## From Agent-Name". We take the text after the first blank line.
    let body = strip_delegation_envelope(content);

    // Try to find a Steps section first (plan.md's `## Steps` heading).
    let mut in_steps_section = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Detect the start of the Steps section.
        if trimmed.eq_ignore_ascii_case("## Steps")
            || trimmed.eq_ignore_ascii_case("## Step Outline")
            || trimmed.eq_ignore_ascii_case("# Steps")
        {
            in_steps_section = true;
            continue;
        }
        // Another markdown heading ends the Steps section.
        if in_steps_section && trimmed.starts_with('#') {
            in_steps_section = false;
            // fall through — this heading may still be a plan step elsewhere
        }
        if let Some(step) = step_from_line(trimmed) {
            steps.push(step);
        }
    }

    // Fallback: if nothing list-like surfaced but the content is non-trivial,
    // emit the body (truncated) as a single plan step. This guarantees a
    // delegation-based session shows *something* in the Mission Control plan
    // pane rather than a blank.
    if steps.is_empty() {
        let trimmed = body.trim();
        if trimmed.len() > 3 {
            let single = if trimmed.len() > 200 {
                format!("{}…", trimmed.chars().take(200).collect::<String>())
            } else {
                trimmed.to_string()
            };
            steps.push(PlanStep {
                text: single,
                status: None,
            });
        }
    }

    steps
}

/// Strip the `## From X\n\n` prefix and the trailing
/// `\n\n---\n_Conversation: ..._` footer from a delegation-result system
/// message, returning the body. Also strips the leading
/// `[DELEGATION COMPLETED. YOUR PLAN IS BELOW....]` envelope.
fn strip_delegation_envelope(content: &str) -> String {
    let mut s = content.trim().to_string();

    // Strip leading "[DELEGATION COMPLETED...]" envelope (up to and including
    // the first `]`).
    if s.starts_with('[') {
        if let Some(end) = s.find(']') {
            s = s[end + 1..].trim_start().to_string();
        }
    }

    // Strip leading "## From Agent-Name" header line + blank line.
    if s.starts_with("## From ") {
        if let Some(newline) = s.find('\n') {
            s = s[newline + 1..].trim_start().to_string();
        }
    }

    // Strip trailing "\n\n---\n_Conversation: ..._" footer.
    if let Some(idx) = s.find("\n---\n") {
        s = s[..idx].trim_end().to_string();
    }

    s
}

/// Recognise a plan-step-shaped line. Supports:
/// - `1. Text` / `1) Text`
/// - `- Text` / `* Text`
/// - `Step 1: Text` / `## Step 1: Text`
fn step_from_line(line: &str) -> Option<PlanStep> {
    // Numbered list: "1. ...", "1) ...", "Step 1: ...", "## Step 1: ..."
    let numbered = line
        .trim_start_matches('#')
        .trim_start()
        .strip_prefix("Step ")
        .and_then(|s| {
            let after = s.trim_start_matches(|c: char| c.is_ascii_digit());
            after
                .strip_prefix(':')
                .or_else(|| after.strip_prefix('.'))
                .or_else(|| after.strip_prefix(')'))
                .map(|rest| rest.trim_start())
                .filter(|rest| !rest.is_empty())
        });
    if let Some(text) = numbered {
        return Some(PlanStep {
            text: text.to_string(),
            status: None,
        });
    }

    // Plain "1. ..." / "1) ..." without the "Step" prefix
    let starts_with_digit = line
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false);
    if starts_with_digit {
        let after_digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
        let rest = after_digits
            .strip_prefix(". ")
            .or_else(|| after_digits.strip_prefix(") "))
            .or_else(|| after_digits.strip_prefix("."))
            .or_else(|| after_digits.strip_prefix(")"));
        if let Some(rest) = rest {
            let rest = rest.trim();
            if !rest.is_empty() {
                return Some(PlanStep {
                    text: rest.to_string(),
                    status: None,
                });
            }
        }
    }

    // Bulleted: "- ..." / "* ..."
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        let rest = rest.trim();
        if !rest.is_empty() {
            return Some(PlanStep {
                text: rest.to_string(),
                status: None,
            });
        }
    }

    None
}

/// Extract recalled facts from `role=tool` result messages whose matching
/// assistant tool_call invoked a memory/recall tool.
///
/// Source: for each assistant message with `tool_calls`, scan its tool_calls
/// for memory/recall tool names; for each, look up the matching `role=tool`
/// message by `tool_call_id` and collect non-empty content lines as facts.
pub fn extract_recalled_facts_from_messages(messages: &[Message]) -> Vec<String> {
    let mut facts = Vec::new();
    for msg in messages {
        if msg.role != "assistant" {
            continue;
        }
        let Some(tc_json) = msg.tool_calls.as_deref() else {
            continue;
        };
        let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(tc_json) else {
            continue;
        };
        for call in &calls {
            let tool_name = call
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !(tool_name.contains("memory") || tool_name.contains("recall")) {
                continue;
            }
            let Some(tool_id) = call.get("tool_id").and_then(|v| v.as_str()) else {
                continue;
            };
            // Find the matching tool-result message by tool_call_id.
            for m in messages {
                if m.role == "tool" && m.tool_call_id.as_deref() == Some(tool_id) {
                    extract_fact_lines(&m.content, &mut facts);
                }
            }
        }
    }
    facts
}

/// Parse fact lines from a tool-result string. Each non-empty trimmed line
/// becomes one fact entry (mirrors the legacy log-based extractor).
fn extract_fact_lines(text: &str, facts: &mut Vec<String>) {
    // Try JSON first — a memory tool may return a JSON array/objects.
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
        match &val {
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            facts.push(trimmed.to_string());
                        }
                    } else if let Some(obj) = item.as_object() {
                        if let Some(fact) = obj
                            .get("fact")
                            .or_else(|| obj.get("text"))
                            .or_else(|| obj.get("content"))
                            .and_then(|v| v.as_str())
                        {
                            let trimmed = fact.trim();
                            if !trimmed.is_empty() {
                                facts.push(trimmed.to_string());
                            }
                        }
                    }
                }
                return;
            }
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    facts.push(trimmed.to_string());
                }
                return;
            }
            _ => {}
        }
    }

    // Plain-text fallback: one fact per non-empty line.
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            facts.push(trimmed.to_string());
        }
    }
}

/// Extract the agent's final response text from messages.
///
/// Walks messages in reverse to find the **last assistant message** with
/// non-empty content that isn't a tool-call-only marker (`[tool calls]`).
/// For completed sessions this is the user-facing response. For sessions
/// still running with no final assistant turn yet, returns `None`
/// (acceptable — the UI re-loads on update).
pub fn extract_response_from_messages(messages: &[Message]) -> Option<String> {
    for msg in messages.iter().rev() {
        if msg.role != "assistant" {
            continue;
        }
        let content = msg.content.trim();
        if content.is_empty() {
            continue;
        }
        // Skip tool-call-only assistant turns.
        if content == "[tool calls]" || content.starts_with("[tool") {
            continue;
        }
        return Some(msg.content.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_from_system_content_extracts_steps_section() {
        let content = "## From Planner Agent\n\n## Steps\n1. Gather data\n2. Build feature\n3. Ship\n\n---\n_Conversation: `x`_";
        let steps = parse_plan_from_system_content(content);
        assert_eq!(steps.len(), 3);
        assert!(steps[0].text.contains("Gather data"));
        assert!(steps[2].text.contains("Ship"));
    }

    #[test]
    fn parse_plan_from_system_content_fallback_single_step() {
        // No list-shaped content — the planner returned prose.
        let content =
            "## From Research Agent\n\nI looked into it and the answer is 42.\n\n---\n_Conv_";
        let steps = parse_plan_from_system_content(content);
        assert_eq!(steps.len(), 1);
        assert!(steps[0].text.contains("answer is 42"));
    }

    #[test]
    fn parse_plan_from_system_content_strips_delegation_completed_envelope() {
        let content = "[DELEGATION COMPLETED. YOUR PLAN IS BELOW.\nReview it.\n]\n\n## Steps\n- One\n- Two";
        let steps = parse_plan_from_system_content(content);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].text, "One");
        assert_eq!(steps[1].text, "Two");
    }

    #[test]
    fn extract_response_skips_tool_call_markers() {
        let msgs = vec![
            Message {
                id: "msg-1".into(),
                execution_id: None,
                session_id: "s".into(),
                role: "assistant".into(),
                content: "[tool calls]".into(),
                created_at: "".into(),
                token_count: 0,
                tool_calls: None,
                tool_call_id: None,
                seq: 1,
            },
            Message {
                id: "msg-2".into(),
                execution_id: None,
                session_id: "s".into(),
                role: "assistant".into(),
                content: "Final answer".into(),
                created_at: "".into(),
                token_count: 0,
                tool_calls: None,
                tool_call_id: None,
                seq: 2,
            },
        ];
        assert_eq!(
            extract_response_from_messages(&msgs).as_deref(),
            Some("Final answer")
        );
    }

    #[test]
    fn extract_plan_from_messages_uses_update_plan_tool_call() {
        let tool_calls = r#"[{"tool_id":"t1","tool_name":"update_plan","args":{"steps":[{"text":"A"},{"text":"B"}]}}]"#;
        let msgs = vec![Message {
            id: "msg-1".into(),
            execution_id: None,
            session_id: "s".into(),
            role: "assistant".into(),
            content: "[tool calls]".into(),
            created_at: "".into(),
            token_count: 0,
            tool_calls: Some(tool_calls.into()),
            tool_call_id: None,
            seq: 1,
        }];
        let plan = extract_plan_from_messages(&msgs);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].text, "A");
    }

    #[test]
    fn extract_recalled_facts_handles_json_array_tool_result() {
        let tool_calls = r#"[{"tool_id":"t1","tool_name":"memory_recall","args":{}}]"#;
        let msgs = vec![
            Message {
                id: "msg-1".into(),
                execution_id: None,
                session_id: "s".into(),
                role: "assistant".into(),
                content: "[tool calls]".into(),
                created_at: "".into(),
                token_count: 0,
                tool_calls: Some(tool_calls.into()),
                tool_call_id: None,
                seq: 1,
            },
            Message {
                id: "msg-2".into(),
                execution_id: None,
                session_id: "s".into(),
                role: "tool".into(),
                content: r#"["fact one","fact two"]"#.into(),
                created_at: "".into(),
                token_count: 0,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
                seq: 2,
            },
        ];
        let facts = extract_recalled_facts_from_messages(&msgs);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0], "fact one");
    }
}
