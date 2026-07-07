//! # Session State Builder
//!
//! Assembles a structured `SessionState` snapshot from execution logs and
//! conversation data. This is the backend half of the executor-steering API:
//! the HTTP handler calls `SessionStateBuilder::build(session_id)` and
//! serialises the result straight to JSON.

use std::sync::Arc;

use api_logs::{ExecutionLog, LogCategory, LogService, SessionStatus};
use serde::Serialize;
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
}

// ============================================================================
// BUILDER
// ============================================================================

/// Tools that are considered "internal" and do not count as real execution.
const INTERNAL_TOOLS: &[&str] = &["analyze_intent", "update_plan", "set_session_title"];

/// Assembles a [`SessionState`] from the logs database and conversation
/// messages table.
pub struct SessionStateBuilder {
    log_service: Arc<LogService<DatabaseManager>>,
    conversations: Arc<ConversationRepository>,
}

impl SessionStateBuilder {
    /// Create a new builder.
    pub fn new(
        log_service: Arc<LogService<DatabaseManager>>,
        conversations: Arc<ConversationRepository>,
    ) -> Self {
        Self {
            log_service,
            conversations,
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

        // Messages table uses execution_id (exec-xxx), not conversation_id (sess-xxx)
        let user_message = self.extract_user_message(&session.session_id);
        let intent_analysis = Self::extract_intent(logs);
        let ward = Self::extract_ward(logs, intent_analysis.as_ref());
        let recalled_facts = Self::extract_recalled_facts(logs);
        let plan = Self::extract_plan(logs);
        let subagents = self.build_subagents(&session.child_session_ids);
        let response = self.extract_response(logs, &session.session_id, &session.child_session_ids);
        let phase = Self::derive_phase(&session.status, logs, response.as_ref());

        let model = Self::extract_model(logs);

        Ok(Some(SessionState {
            session: SessionMeta {
                id: session.session_id.clone(),
                title: session
                    .title
                    .clone()
                    .or_else(|| Self::extract_title(logs, intent_analysis.as_ref())),
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

    /// Extract ward info from a ward tool call or from intent analysis metadata.
    fn extract_ward(logs: &[ExecutionLog], intent: Option<&serde_json::Value>) -> Option<WardInfo> {
        // First, try to find a ward from tool_call logs whose message mentions "ward"
        for log in logs {
            if log.category == LogCategory::ToolCall {
                if let Some(meta) = &log.metadata {
                    let tool_name = meta.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                    if tool_name.contains("ward") || tool_name == "load_ward" {
                        let name = meta
                            .get("args")
                            .and_then(|a| a.get("ward_name"))
                            .or_else(|| meta.get("args").and_then(|a| a.get("name")))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        return Some(WardInfo {
                            name,
                            content: None,
                        });
                    }
                }
            }
        }

        // Fallback: extract from intent analysis
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

        None
    }

    /// Extract recalled facts from memory tool_result logs.
    fn extract_recalled_facts(logs: &[ExecutionLog]) -> Vec<String> {
        let mut facts = Vec::new();
        for log in logs {
            if log.category == LogCategory::ToolResult {
                if let Some(meta) = &log.metadata {
                    let tool_name = meta.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                    if tool_name.contains("memory") || tool_name.contains("recall") {
                        // Try to extract facts from the result
                        if let Some(result) = meta.get("result").and_then(|v| v.as_str()) {
                            for line in result.lines() {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    facts.push(trimmed.to_string());
                                }
                            }
                        } else if let Some(result_arr) =
                            meta.get("result").and_then(|v| v.as_array())
                        {
                            for item in result_arr {
                                if let Some(s) = item.as_str() {
                                    facts.push(s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        facts
    }

    /// Extract plan steps from the latest `update_plan` tool call args.
    fn extract_plan(logs: &[ExecutionLog]) -> Vec<PlanStep> {
        // Find the *last* update_plan tool_call (the most recent plan)
        let plan_log = logs.iter().rev().find(|l| {
            l.category == LogCategory::ToolCall
                && l.metadata
                    .as_ref()
                    .and_then(|m| m.get("tool_name"))
                    .and_then(|v| v.as_str())
                    == Some("update_plan")
        });

        let Some(log) = plan_log else {
            return Vec::new();
        };

        let Some(meta) = &log.metadata else {
            return Vec::new();
        };

        // Try to extract steps from args
        let args = match meta.get("args") {
            Some(a) => a,
            None => meta,
        };

        // Steps might be in args.steps or args.plan
        let steps_val = args
            .get("steps")
            .or_else(|| args.get("plan"))
            .and_then(|v| v.as_array());

        match steps_val {
            Some(arr) => arr
                .iter()
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
                .collect(),
            None => Vec::new(),
        }
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

    /// Extract the agent response text.
    ///
    /// Prefers the `respond` tool call args, falls back to the last assistant
    /// message in the conversation.
    fn extract_response(
        &self,
        logs: &[ExecutionLog],
        execution_id: &str,
        child_session_ids: &[String],
    ) -> Option<String> {
        // Helper: find respond tool call in a set of logs
        let find_respond = |logs: &[ExecutionLog]| -> Option<String> {
            for log in logs.iter().rev() {
                if log.category == LogCategory::ToolCall {
                    if let Some(meta) = &log.metadata {
                        let tool_name =
                            meta.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                        if tool_name == "respond" {
                            if let Some(text) = meta
                                .get("args")
                                .and_then(|a| a.get("text").or_else(|| a.get("message")))
                                .and_then(|v| v.as_str())
                            {
                                return Some(text.to_string());
                            }
                        }
                    }
                }
            }
            None
        };

        // First: check root session logs
        if let Some(r) = find_respond(logs) {
            return Some(r);
        }

        // Second: check child session logs (subagent may have called respond)
        for child_id in child_session_ids {
            if let Ok(Some(detail)) = self.log_service.get_session_detail(child_id) {
                if let Some(r) = find_respond(&detail.logs) {
                    return Some(r);
                }
            }
        }

        // Third: look for a Response-category log
        for log in logs.iter().rev() {
            if log.category == LogCategory::Response && !log.message.is_empty() {
                return Some(log.message.clone());
            }
        }

        // Fallback: last assistant message from conversation (skip tool-call-only messages)
        if let Ok(messages) = self.conversations.get_messages(execution_id) {
            for msg in messages.iter().rev() {
                if msg.role == "assistant"
                    && !msg.content.is_empty()
                    && msg.content.trim() != "[tool calls]"
                    && !msg.content.starts_with("[tool")
                {
                    return Some(msg.content.clone());
                }
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

            // Extract the delegation task: check parent's delegation logs (metadata.task),
            // then child's delegation logs, then fall back to agent_executions.task via child session
            let task = self.extract_delegation_task(child_id, &child_session.agent_id, child_logs);

            // Build tool call entries for this subagent
            let tool_calls = Self::build_tool_calls(child_logs);

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

    /// Build tool call entries from a set of logs.
    fn build_tool_calls(logs: &[ExecutionLog]) -> Vec<ToolCallEntry> {
        let mut entries = Vec::new();

        for log in logs {
            if log.category == LogCategory::ToolCall {
                let tool_name = log
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Skip internal tools
                if INTERNAL_TOOLS.contains(&tool_name.as_str()) {
                    continue;
                }

                let summary = log
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("summary"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        if !log.message.is_empty() {
                            Some(log.message.clone())
                        } else {
                            None
                        }
                    });

                entries.push(ToolCallEntry {
                    tool_name,
                    status: Some("completed".to_string()),
                    duration_ms: log.duration_ms,
                    summary,
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
