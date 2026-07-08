//! Session handoff — types, the production `LlmHandoffWriter`, and the
//! `HandoffWriter` engine struct that persists summarized sessions to the
//! memory fact store.
//!
//! Lives in `gateway-execution` (not `gateway-memory`) because the prompt
//! formatting, summarization template, and `## Last Session` injection block
//! all bake in zbot-specific chat-prompt conventions and tool references —
//! they are application-layer concerns, not generic memory primitives.
//!
//! The engine takes `Arc<dyn zbot_conversation::MessageStore>` for
//! consistency with the broader execution layer. Rich-type conversion
//! (POD `Message` → `agent_runtime::ChatMessage`) lives here as
//! `messages_to_chat_format`.

use std::sync::Arc;

use agent_runtime::ChatMessage;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gateway_memory::{LlmClientConfig, MemoryLlmFactory};
use serde::{Deserialize, Serialize};
use zbot_stores_domain::{RouteHint, RouteSourceKind};

pub const HANDOFF_MAX_AGE_DAYS: i64 = 7;

pub const HANDOFF_AGENT_SENTINEL: &str = "__handoff__";
pub const HANDOFF_CATEGORY: &str = "handoff";
pub const HANDOFF_SCOPE: &str = "global";
pub const HANDOFF_WARD: &str = "__global__";
pub const HANDOFF_CTX_OWNER: &str = "root";

/// Stored JSON shape for each `handoff.*` fact in the DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffEntry {
    pub summary: String,
    pub session_id: String,
    pub completed_at: String,
    pub ward_id: String,
    pub intent_key: String,
    pub goal_count: u32,
    pub open_task_count: u32,
    pub correction_count: u32,
    pub turns: u32,
    #[serde(default)]
    pub route_hints: Vec<RouteHint>,
}

/// Input passed to the LLM for summarisation.
#[derive(Debug, Clone)]
pub struct HandoffInput {
    pub messages: Vec<ChatMessage>,
    pub ward_id: String,
    pub route_hints: Vec<RouteHint>,
}

/// Mockable LLM interface for generating 3-5 sentence handoff summaries.
#[async_trait]
pub trait HandoffLlm: Send + Sync {
    async fn summarize(&self, input: &HandoffInput) -> Result<String, String>;
}

/// Returns false if the entry is older than `HANDOFF_MAX_AGE_DAYS` or unparseable.
pub fn should_inject(entry: &HandoffEntry) -> bool {
    let Ok(completed_at) = entry.completed_at.parse::<DateTime<Utc>>() else {
        return false;
    };
    (Utc::now() - completed_at).num_days() <= HANDOFF_MAX_AGE_DAYS
}

/// Reads `handoff.latest` from the fact store.
/// Returns `None` if absent, unparseable, older than `HANDOFF_MAX_AGE_DAYS`,
/// or if `current_ward` is `Some` and doesn't match the entry's ward.
pub async fn read_handoff_block(
    fact_store: &Arc<dyn zbot_stores::MemoryFactStore>,
    current_ward: Option<&str>,
) -> Option<String> {
    let mut ctx_wards = Vec::new();
    if let Some(ward) = current_ward {
        ctx_wards.push(ward);
    }
    if !ctx_wards.contains(&HANDOFF_WARD) {
        ctx_wards.push(HANDOFF_WARD);
    }

    let mut entry_json = None;
    for ward in ctx_wards {
        entry_json = fact_store
            .get_ctx_fact(ward, "handoff.latest")
            .await
            .ok()
            .flatten()
            .and_then(|v| {
                v.get("content")
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
            });
        if entry_json.is_some() {
            break;
        }
    }

    let entry_json = if let Some(ctx) = entry_json {
        ctx
    } else {
        let fact = fact_store
            .get_fact_by_key(
                HANDOFF_AGENT_SENTINEL,
                HANDOFF_SCOPE,
                HANDOFF_WARD,
                "handoff.latest",
            )
            .await
            .ok()??;
        fact.content
    };
    let entry: HandoffEntry = serde_json::from_str(&entry_json).ok()?;
    let completed_at = entry.completed_at.parse::<DateTime<Utc>>().ok()?;
    if (Utc::now() - completed_at).num_days() > HANDOFF_MAX_AGE_DAYS {
        return None;
    }
    // Ward filter: skip if the new session is a different ward (None = accept any)
    if let Some(cw) = current_ward {
        if !entry.ward_id.is_empty() && entry.ward_id != cw {
            return None;
        }
    }
    let date_str = completed_at.format("%Y-%m-%d").to_string();
    let pointer_block = format_route_pointer_block(&entry.route_hints);
    Some(format!(
        "## Last Session  ({date} · ward: {ward} · {turns} turns)\n\
         {summary}\n\n\
         {pointers}\
         Corrections active: {corrections} · Goals: {goals}\n\
         Full context: memory(action=get_fact, key=ctx.{sid}.handoff)\n\
         Last intent:  memory(action=get_fact, key={intent_key})",
        date = date_str,
        ward = entry.ward_id,
        turns = entry.turns,
        summary = entry.summary,
        pointers = pointer_block,
        corrections = entry.correction_count,
        goals = entry.goal_count,
        sid = entry.session_id,
        intent_key = entry.intent_key,
    ))
}

// ============================================================================
// LLM-backed implementation
// ============================================================================

/// Production `HandoffLlm` wired to the injected `MemoryLlmFactory`.
pub struct LlmHandoffWriter {
    factory: Arc<dyn MemoryLlmFactory>,
}

impl LlmHandoffWriter {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self { factory }
    }
}

/// Formats conversation messages for the LLM summary prompt.
/// Includes user, assistant, and tool messages (up to 40).
/// Assistant messages with tool calls are annotated with `[called: name1, name2]`.
pub fn format_conversation_for_summary(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant" || m.role == "tool")
        .take(40)
        .map(|m| {
            if m.role == "assistant" {
                if let Some(calls) = &m.tool_calls {
                    let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
                    if !names.is_empty() {
                        let text = m.text_content();
                        return if text.is_empty() {
                            format!("assistant [called: {}]", names.join(", "))
                        } else {
                            format!("assistant [called: {}]: {}", names.join(", "), text)
                        };
                    }
                }
            }
            format!("{}: {}", m.role, m.text_content())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait]
impl HandoffLlm for LlmHandoffWriter {
    async fn summarize(&self, input: &HandoffInput) -> Result<String, String> {
        let client = self
            .factory
            .build_client(LlmClientConfig::new(0.2, 256))
            .await?;
        let conversation = format_conversation_for_summary(&input.messages);
        let pointers = format_route_pointer_block(&input.route_hints);
        let prompt = format!(
            "Summarize this conversation in 3-5 sentences. Cover:\n\
             - What was accomplished\n\
             - What was left incomplete or in progress\n\
             - What the user seemed most focused on or interested in next\n\n\
             Be specific. Do not use filler phrases like 'the user and assistant discussed'.\n\
             Use past tense. Write for an agent reading this at the start of the NEXT session.\n\
             Preserve the meaning of any pointer lines, but do not restate every pointer in prose.\n\n\
             Ward: {ward}\n\n\
             {pointers}\
             Conversation:\n{conversation}",
            ward = input.ward_id,
            pointers = pointers,
            conversation = conversation,
        );
        let messages = vec![
            ChatMessage::system(
                "You are a concise session summarizer. Return only the summary text, no JSON."
                    .to_string(),
            ),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        Ok(response.content)
    }
}

// ============================================================================
// HandoffWriter engine
// ============================================================================

/// Writes session handoff to the memory fact store.
///
pub struct HandoffWriter {
    llm: Arc<dyn HandoffLlm>,
    fact_store: Arc<dyn zbot_stores::MemoryFactStore>,
    messages: Arc<dyn zbot_conversation::MessageStore>,
}

impl HandoffWriter {
    pub fn new(
        llm: Arc<dyn HandoffLlm>,
        fact_store: Arc<dyn zbot_stores::MemoryFactStore>,
        messages: Arc<dyn zbot_conversation::MessageStore>,
    ) -> Self {
        Self {
            llm,
            fact_store,
            messages,
        }
    }

    /// Fire-and-forget entry point: loads last 50 messages then calls
    /// `write_with_messages`. All errors are logged at warn and swallowed.
    pub async fn write(&self, session_id: &str, agent_id: &str, ward_id: &str) {
        let messages_raw = match self.messages.replay(session_id, None, 50) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(session_id, "handoff: failed to load messages: {e}");
                return;
            }
        };
        let messages = messages_to_chat_format(&messages_raw);
        if let Err(e) = self
            .write_with_messages(session_id, agent_id, ward_id, messages)
            .await
        {
            tracing::warn!(session_id, "handoff: write failed: {e}");
        }
    }

    /// Testable core: accepts pre-loaded messages; returns Err on LLM or store failure.
    pub async fn write_with_messages(
        &self,
        session_id: &str,
        agent_id: &str,
        ward_id: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<(), String> {
        let turns = messages.iter().filter(|m| m.role == "user").count() as u32;
        let correction_count = self
            .fact_store
            .get_facts_by_category(agent_id, "correction", 200)
            .await
            .unwrap_or_default()
            .len() as u32;
        let input = HandoffInput {
            route_hints: collect_route_hints(session_id, ward_id, &messages),
            messages,
            ward_id: ward_id.to_string(),
        };
        let summary = self.llm.summarize(&input).await?;
        let entry = HandoffEntry {
            summary,
            session_id: session_id.to_string(),
            completed_at: Utc::now().to_rfc3339(),
            ward_id: ward_id.to_string(),
            intent_key: format!("ctx.{session_id}.intent"),
            goal_count: 0,
            open_task_count: 0,
            correction_count,
            turns,
            route_hints: input.route_hints,
        };
        self.persist(session_id, &entry).await
    }

    async fn persist(&self, session_id: &str, entry: &HandoffEntry) -> Result<(), String> {
        let json = serde_json::to_string(entry).map_err(|e| format!("serialize entry: {e}"))?;
        let ctx_ward = if entry.ward_id.trim().is_empty() {
            HANDOFF_WARD
        } else {
            entry.ward_id.as_str()
        };
        self.fact_store
            .save_ctx_fact(
                session_id,
                ctx_ward,
                "handoff.latest",
                &json,
                HANDOFF_CTX_OWNER,
                true,
            )
            .await
            .map_err(|e| format!("save handoff.latest: {e}"))?;
        self.fact_store
            .save_ctx_fact(
                session_id,
                ctx_ward,
                &format!("handoff.{session_id}"),
                &json,
                HANDOFF_CTX_OWNER,
                true,
            )
            .await
            .map_err(|e| format!("save handoff.{session_id}: {e}"))?;
        self.fact_store
            .save_ctx_fact(
                session_id,
                ctx_ward,
                &format!("ctx.{session_id}.handoff"),
                &json,
                HANDOFF_CTX_OWNER,
                true,
            )
            .await
            .map_err(|e| format!("save ctx.{session_id}.handoff: {e}"))?;
        if ctx_ward != HANDOFF_WARD {
            self.fact_store
                .save_ctx_fact(
                    session_id,
                    HANDOFF_WARD,
                    "handoff.latest",
                    &json,
                    HANDOFF_CTX_OWNER,
                    true,
                )
                .await
                .map_err(|e| format!("save global handoff.latest: {e}"))?;
        }
        tracing::info!(
            session_id,
            handoff_ctx_saved = true,
            ward = %ctx_ward,
            "handoff: saved full payload through ctx storage"
        );
        Ok(())
    }
}

fn format_route_pointer_block(route_hints: &[RouteHint]) -> String {
    if route_hints.is_empty() {
        return String::new();
    }
    let mut out = String::from("Pointers:\n");
    for hint in route_hints.iter().take(8) {
        let kind = serde_json::to_value(&hint.source_kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "source".to_string());
        out.push_str(&format!("- {kind} in ward `{}`", hint.ward_id));
        if let Some(path) = hint.source_path.as_deref() {
            out.push_str(&format!(" at `{path}`"));
        }
        if let Some(exec) = hint.execution_id.as_deref() {
            out.push_str(&format!(" (execution `{exec}`)"));
        } else if let Some(session) = hint.session_id.as_deref() {
            out.push_str(&format!(" (session `{session}`)"));
        }
        out.push('\n');
    }
    out.push('\n');
    out
}

fn collect_route_hints(
    session_id: &str,
    fallback_ward: &str,
    messages: &[ChatMessage],
) -> Vec<RouteHint> {
    let mut hints = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for message in messages {
        if let Some(calls) = message.tool_calls.as_ref() {
            for call in calls {
                collect_tool_call_route_hints(
                    session_id,
                    fallback_ward,
                    call,
                    &mut hints,
                    &mut seen,
                );
            }
        }
        collect_json_text_route_hints(
            session_id,
            fallback_ward,
            &message.text_content(),
            &mut hints,
            &mut seen,
        );
    }

    hints
}

fn collect_tool_call_route_hints(
    session_id: &str,
    fallback_ward: &str,
    call: &agent_runtime::types::ToolCall,
    hints: &mut Vec<RouteHint>,
    seen: &mut std::collections::HashSet<String>,
) {
    if call.name == "delegate_to_agent" {
        if let Some(agent_id) = call.arguments.get("agent_id").and_then(|v| v.as_str()) {
            if let Some(ward) = agent_id.strip_prefix("ward:") {
                push_route_hint(
                    RouteHint::new(ward.to_string(), RouteSourceKind::Episode)
                        .with_session_id(Some(session_id.to_string())),
                    hints,
                    seen,
                );
            }
        }
    }

    if call.name == "respond" {
        collect_artifact_paths_from_value(session_id, fallback_ward, &call.arguments, hints, seen);
    }
}

fn collect_json_text_route_hints(
    session_id: &str,
    fallback_ward: &str,
    text: &str,
    hints: &mut Vec<RouteHint>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return;
    };
    collect_route_hints_from_value(session_id, fallback_ward, &value, hints, seen);
}

fn collect_route_hints_from_value(
    session_id: &str,
    fallback_ward: &str,
    value: &serde_json::Value,
    hints: &mut Vec<RouteHint>,
    seen: &mut std::collections::HashSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(agent_id) = map.get("agent_id").and_then(|v| v.as_str()) {
                if let Some(ward) = agent_id.strip_prefix("ward:") {
                    let execution_id = map
                        .get("execution_id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    push_route_hint(
                        RouteHint::new(ward.to_string(), RouteSourceKind::Episode)
                            .with_session_id(Some(session_id.to_string()))
                            .with_execution_id(execution_id),
                        hints,
                        seen,
                    );
                }
            }
            collect_artifact_paths_from_value(session_id, fallback_ward, value, hints, seen);
            for child in map.values() {
                collect_route_hints_from_value(session_id, fallback_ward, child, hints, seen);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_route_hints_from_value(session_id, fallback_ward, child, hints, seen);
            }
        }
        _ => {}
    }
}

fn collect_artifact_paths_from_value(
    session_id: &str,
    fallback_ward: &str,
    value: &serde_json::Value,
    hints: &mut Vec<RouteHint>,
    seen: &mut std::collections::HashSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            let path = map
                .get("path")
                .or_else(|| map.get("file_path"))
                .and_then(|v| v.as_str())
                .and_then(|p| safe_ward_relative_path(p, fallback_ward));
            if let Some(path) = path {
                push_route_hint(
                    RouteHint::new(fallback_ward.to_string(), RouteSourceKind::Artifact)
                        .with_session_id(Some(session_id.to_string()))
                        .with_source_path(Some(path)),
                    hints,
                    seen,
                );
            }
            for child in map.values() {
                collect_artifact_paths_from_value(session_id, fallback_ward, child, hints, seen);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_artifact_paths_from_value(session_id, fallback_ward, child, hints, seen);
            }
        }
        _ => {}
    }
}

fn safe_ward_relative_path(path: &str, ward: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.starts_with('/') {
        return Some(trimmed.to_string());
    }
    let marker = format!("/wards/{ward}/");
    trimmed
        .find(&marker)
        .map(|idx| trimmed[(idx + marker.len())..].to_string())
}

fn push_route_hint(
    hint: RouteHint,
    hints: &mut Vec<RouteHint>,
    seen: &mut std::collections::HashSet<String>,
) {
    let key = serde_json::to_string(&hint).unwrap_or_default();
    if seen.insert(key) {
        hints.push(hint);
    }
}

// ============================================================================
// Message → ChatMessage conversion
// ============================================================================

/// Convert POD `Message` rows into `agent_runtime::ChatMessage` for LLM use.
///
/// Parses `tool_calls` JSON on assistant messages from the
/// stored format `[{"tool_id", "tool_name", "args", ...}]` into the LLM's
/// `ToolCall { id, name, arguments }` shape.
pub fn messages_to_chat_format(messages: &[zbot_conversation::Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| {
            let tool_calls = if m.role == "assistant" {
                m.tool_calls.as_deref().and_then(parse_tool_calls_json)
            } else {
                None
            };
            ChatMessage {
                role: m.role.clone(),
                content: vec![agent_primitives::types::Part::Text {
                    text: m.content.clone(),
                }],
                tool_calls,
                tool_call_id: m.tool_call_id.clone(),
                is_summary: false,
            }
        })
        .collect()
}

fn parse_tool_calls_json(json_str: &str) -> Option<Vec<agent_runtime::types::ToolCall>> {
    let stored: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
    let tool_calls: Vec<agent_runtime::types::ToolCall> = stored
        .into_iter()
        .filter_map(|v| {
            let tool_id = v.get("tool_id")?.as_str()?.to_string();
            let tool_name = v.get("tool_name")?.as_str()?.to_string();
            let args = v.get("args")?.clone();
            Some(agent_runtime::types::ToolCall::new(
                tool_id, tool_name, args,
            ))
        })
        .collect();
    if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use zbot_stores_domain::MemoryFact;

    // ---- Mock MemoryFactStore ----

    #[derive(Default)]
    struct MockFactStore {
        facts: Mutex<HashMap<String, String>>,
        ctx_facts: Mutex<HashMap<String, String>>,
        save_fact_calls: Mutex<u32>,
        corrections: Mutex<Vec<String>>,
    }

    impl MockFactStore {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn get(&self, key: &str) -> Option<String> {
            self.ctx_facts
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .or_else(|| self.facts.lock().unwrap().get(key).cloned())
        }
        fn contains(&self, key: &str) -> bool {
            self.ctx_facts.lock().unwrap().contains_key(key)
                || self.facts.lock().unwrap().contains_key(key)
        }
        fn save_fact_call_count(&self) -> u32 {
            *self.save_fact_calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl zbot_stores::MemoryFactStore for MockFactStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            key: &str,
            content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> Result<serde_json::Value, String> {
            *self.save_fact_calls.lock().unwrap() += 1;
            self.facts
                .lock()
                .unwrap()
                .insert(key.to_string(), content.to_string());
            Ok(serde_json::json!({"ok": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!([]))
        }

        async fn get_fact_by_key(
            &self,
            _agent_id: &str,
            _scope: &str,
            _ward_id: &str,
            key: &str,
        ) -> Result<Option<MemoryFact>, String> {
            let content = self.facts.lock().unwrap().get(key).cloned();
            Ok(content.map(|c| MemoryFact {
                id: "mock".to_string(),
                session_id: None,
                agent_id: HANDOFF_AGENT_SENTINEL.to_string(),
                scope: HANDOFF_SCOPE.to_string(),
                category: HANDOFF_CATEGORY.to_string(),
                key: key.to_string(),
                content: c,
                confidence: 1.0,
                mention_count: 1,
                source_summary: None,
                embedding: None,
                ward_id: HANDOFF_WARD.to_string(),
                contradicted_by: None,
                created_at: String::new(),
                updated_at: String::new(),
                expires_at: None,
                valid_from: None,
                valid_until: None,
                superseded_by: None,
                pinned: false,
                epistemic_class: None,
                source_episode_id: None,
                source_ref: None,
            }))
        }

        async fn get_ctx_fact(
            &self,
            _ward_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, String> {
            Ok(self.ctx_facts.lock().unwrap().get(key).map(|content| {
                serde_json::json!({
                    "found": true,
                    "key": key,
                    "content": content,
                    "owner": HANDOFF_CTX_OWNER,
                    "session_id": "mock-session",
                    "pinned": true,
                })
            }))
        }

        async fn save_ctx_fact(
            &self,
            _session_id: &str,
            _ward_id: &str,
            key: &str,
            content: &str,
            _owner: &str,
            _pinned: bool,
        ) -> Result<serde_json::Value, String> {
            self.ctx_facts
                .lock()
                .unwrap()
                .insert(key.to_string(), content.to_string());
            Ok(serde_json::json!({"ok": true, "action": "save_ctx_fact"}))
        }

        async fn get_facts_by_category(
            &self,
            _agent_id: &str,
            category: &str,
            _limit: usize,
        ) -> Result<Vec<MemoryFact>, String> {
            if category != "correction" {
                return Ok(Vec::new());
            }
            let items = self.corrections.lock().unwrap().clone();
            Ok(items
                .into_iter()
                .enumerate()
                .map(|(i, content)| MemoryFact {
                    id: format!("corr-{i}"),
                    session_id: None,
                    agent_id: String::new(),
                    scope: String::new(),
                    ward_id: String::new(),
                    category: "correction".to_string(),
                    key: format!("correction.{i}"),
                    content,
                    confidence: 1.0,
                    mention_count: 1,
                    source_summary: None,
                    embedding: None,
                    contradicted_by: None,
                    created_at: String::new(),
                    updated_at: String::new(),
                    expires_at: None,
                    valid_from: None,
                    valid_until: None,
                    superseded_by: None,
                    pinned: false,
                    epistemic_class: None,
                    source_episode_id: None,
                    source_ref: None,
                })
                .collect())
        }
    }

    // ---- Mock LLM ----

    struct MockLlm {
        response: Mutex<Result<String, String>>,
        calls: Mutex<u32>,
    }

    impl MockLlm {
        fn ok(summary: &str) -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(Ok(summary.to_string())),
                calls: Mutex::new(0),
            })
        }
        fn err() -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(Err("mock LLM error".to_string())),
                calls: Mutex::new(0),
            })
        }
        fn call_count(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl HandoffLlm for MockLlm {
        async fn summarize(&self, _input: &HandoffInput) -> Result<String, String> {
            *self.calls.lock().unwrap() += 1;
            self.response.lock().unwrap().clone()
        }
    }

    // ---- Mock MessageStore ----
    // Minimal impl — `HandoffWriter::write_with_messages` doesn't read from
    // the store (caller pre-loads messages), so these methods are not
    // exercised. Kept here so the writer can be constructed in tests.

    #[derive(Default)]
    struct MockConvStore;

    impl zbot_conversation::MessageStore for MockConvStore {
        fn append(&self, _msg: &zbot_conversation::Message) -> anyhow::Result<()> {
            Ok(())
        }

        fn replay(
            &self,
            _session_id: &str,
            _after_seq: Option<i64>,
            _limit: usize,
        ) -> anyhow::Result<Vec<zbot_conversation::Message>> {
            Ok(Vec::new())
        }

        fn tool_sequence_for_session(&self, _session_id: &str) -> anyhow::Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    fn make_writer(
        llm: Arc<dyn HandoffLlm>,
        store: Arc<dyn zbot_stores::MemoryFactStore>,
    ) -> HandoffWriter {
        HandoffWriter::new(llm, store, Arc::new(MockConvStore))
    }

    fn sample_messages(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    ChatMessage::user(format!("user message {i}"))
                } else {
                    ChatMessage::assistant(format!("assistant reply {i}"))
                }
            })
            .collect()
    }

    // ---- Test 4: stale_handoff_excluded ----

    #[test]
    fn stale_handoff_excluded() {
        let stale = HandoffEntry {
            summary: "old summary".to_string(),
            session_id: "sess-old".to_string(),
            completed_at: (Utc::now() - chrono::Duration::days(8)).to_rfc3339(),
            ward_id: "ward".to_string(),
            intent_key: "ctx.sess-old.intent".to_string(),
            goal_count: 0,
            open_task_count: 0,
            correction_count: 0,
            turns: 5,
            route_hints: Vec::new(),
        };
        assert!(
            !should_inject(&stale),
            "8-day-old handoff should be excluded"
        );

        let fresh = HandoffEntry {
            completed_at: (Utc::now() - chrono::Duration::days(6)).to_rfc3339(),
            ..stale.clone()
        };
        assert!(
            should_inject(&fresh),
            "6-day-old handoff should be injected"
        );
    }

    // ---- Test 5: read_handoff_block returns formatted block ----

    #[tokio::test]
    async fn read_handoff_block_returns_formatted_block() {
        let store = MockFactStore::new();
        let entry = HandoffEntry {
            summary: "User explored memory. Spec written.".to_string(),
            session_id: "sess-abc".to_string(),
            completed_at: Utc::now().to_rfc3339(),
            ward_id: "memory-ward".to_string(),
            intent_key: "ctx.sess-abc.intent".to_string(),
            goal_count: 2,
            open_task_count: 0,
            correction_count: 3,
            turns: 10,
            route_hints: vec![RouteHint::new("memory-ward", RouteSourceKind::Artifact)
                .with_source_path(Some("output/report.html".to_string()))
                .with_session_id(Some("sess-abc".to_string()))],
        };
        store.facts.lock().unwrap().insert(
            "handoff.latest".to_string(),
            serde_json::to_string(&entry).unwrap(),
        );

        let store: Arc<dyn zbot_stores::MemoryFactStore> = store;
        let block = read_handoff_block(&store, None)
            .await
            .expect("should return a block");
        assert!(block.contains("## Last Session"));
        assert!(block.contains("User explored memory"));
        assert!(block.contains("memory-ward"));
        assert!(block.contains("Corrections active: 3"));
        assert!(block.contains("Goals: 2"));
        assert!(block.contains("ctx.sess-abc.handoff"));
        assert!(block.contains("Pointers:"));
        assert!(block.contains("output/report.html"));
    }

    // ---- Test 6: read_handoff_block returns None for stale entry ----

    #[tokio::test]
    async fn read_handoff_block_returns_none_when_stale() {
        let store = MockFactStore::new();
        let entry = HandoffEntry {
            summary: "old".to_string(),
            session_id: "sess-old".to_string(),
            completed_at: (Utc::now() - chrono::Duration::days(10)).to_rfc3339(),
            ward_id: "ward".to_string(),
            intent_key: "ctx.sess-old.intent".to_string(),
            goal_count: 0,
            open_task_count: 0,
            correction_count: 0,
            turns: 5,
            route_hints: Vec::new(),
        };
        store.facts.lock().unwrap().insert(
            "handoff.latest".to_string(),
            serde_json::to_string(&entry).unwrap(),
        );

        let store: Arc<dyn zbot_stores::MemoryFactStore> = store;
        assert!(read_handoff_block(&store, None).await.is_none());
    }

    // ---- Test 7: read_handoff_block returns None when absent ----

    #[tokio::test]
    async fn read_handoff_block_returns_none_when_absent() {
        let store: Arc<dyn zbot_stores::MemoryFactStore> = MockFactStore::new();
        assert!(read_handoff_block(&store, None).await.is_none());
    }

    // ---- Test 8: ward filter ----

    #[tokio::test]
    async fn read_handoff_block_returns_none_when_ward_differs() {
        let store = MockFactStore::new();
        let entry = HandoffEntry {
            summary: "Done something.".to_string(),
            session_id: "sess-1".to_string(),
            completed_at: Utc::now().to_rfc3339(),
            ward_id: "coding-ward".to_string(),
            intent_key: "ctx.sess-1.intent".to_string(),
            goal_count: 0,
            open_task_count: 0,
            correction_count: 0,
            turns: 3,
            route_hints: Vec::new(),
        };
        store.facts.lock().unwrap().insert(
            "handoff.latest".to_string(),
            serde_json::to_string(&entry).unwrap(),
        );
        let store: Arc<dyn zbot_stores::MemoryFactStore> = store;
        assert!(read_handoff_block(&store, Some("research-ward"))
            .await
            .is_none());
        assert!(read_handoff_block(&store, Some("coding-ward"))
            .await
            .is_some());
        assert!(read_handoff_block(&store, None).await.is_some());
    }

    // ---- format_conversation_for_summary covers tool-call annotations ----

    #[test]
    fn format_conversation_includes_tool_annotations() {
        use agent_runtime::ToolCall;

        let mut assistant_msg = ChatMessage::assistant("Let me check.".to_string());
        assistant_msg.tool_calls = Some(vec![ToolCall {
            id: "tc-1".to_string(),
            name: "memory".to_string(),
            arguments: serde_json::json!({"action": "get_fact"}),
        }]);

        let messages = vec![
            ChatMessage::user("What do you know?".to_string()),
            assistant_msg,
            ChatMessage::system("memory result".to_string()), // system filtered out
        ];

        let text = format_conversation_for_summary(&messages);
        assert!(
            text.contains("[called: memory]"),
            "tool name missing: {text}"
        );
        assert!(
            !text.contains("memory result"),
            "system message should be excluded: {text}"
        );
        assert!(
            text.contains("What do you know"),
            "user message missing: {text}"
        );
    }

    // ---- HandoffWriter tests (moved from gateway-execution) ----

    // ---- Test 1: generates_summary_from_messages ----

    #[tokio::test]
    async fn generates_summary_from_messages() {
        let llm = MockLlm::ok(
            "User explored memory limits. Wrote reflective spec. Left impl incomplete.",
        );
        let store = MockFactStore::new();
        let writer = make_writer(llm.clone(), store.clone());

        let result = writer
            .write_with_messages("sess-abc", "agent-root", "test-ward", sample_messages(6))
            .await;

        assert!(result.is_ok(), "write_with_messages failed: {result:?}");
        assert_eq!(llm.call_count(), 1, "LLM should be called exactly once");

        let raw = store.get("handoff.latest").expect("handoff.latest missing");
        let entry: HandoffEntry = serde_json::from_str(&raw).unwrap();
        assert!(!entry.summary.is_empty(), "summary should be non-empty");
        assert_eq!(entry.session_id, "sess-abc");
        assert_eq!(entry.ward_id, "test-ward");
        assert_eq!(entry.turns, 3, "3 user messages in sample_messages(6)");
    }

    // ---- Test 2: writes_latest_and_archived_keys ----

    #[tokio::test]
    async fn writes_latest_and_archived_keys() {
        let store = MockFactStore::new();
        let writer = make_writer(MockLlm::ok("Session summary here."), store.clone());

        writer
            .write_with_messages("sess-xyz", "agent-root", "my-ward", sample_messages(4))
            .await
            .unwrap();

        assert!(store.contains("handoff.latest"), "handoff.latest missing");
        assert!(
            store.contains("handoff.sess-xyz"),
            "handoff.sess-xyz missing"
        );
        assert!(
            store.contains("ctx.sess-xyz.handoff"),
            "ctx.sess-xyz.handoff missing"
        );

        let latest: HandoffEntry =
            serde_json::from_str(&store.get("handoff.latest").unwrap()).unwrap();
        let archived: HandoffEntry =
            serde_json::from_str(&store.get("handoff.sess-xyz").unwrap()).unwrap();
        assert_eq!(latest.summary, archived.summary);
        assert_eq!(
            store.save_fact_call_count(),
            0,
            "full handoff JSON must not be written as a semantic fact"
        );
    }

    #[tokio::test]
    async fn writes_oversized_handoff_payload_through_ctx_storage() {
        let store = MockFactStore::new();
        let writer = make_writer(MockLlm::ok(&"long summary ".repeat(120)), store.clone());

        writer
            .write_with_messages("sess-long", "agent-root", "my-ward", sample_messages(4))
            .await
            .unwrap();

        let latest = store.get("handoff.latest").expect("handoff.latest missing");
        assert!(
            latest.chars().count() > 800,
            "test must exercise payloads over the normal semantic fact cap"
        );
        assert!(store.contains("handoff.sess-long"));
        assert!(store.contains("ctx.sess-long.handoff"));
        assert_eq!(
            store.save_fact_call_count(),
            0,
            "oversized handoff payload must avoid save_fact"
        );
    }

    // ---- Test 3: failure_is_silent ----

    #[tokio::test]
    async fn failure_is_silent() {
        let store = MockFactStore::new();
        let writer = make_writer(MockLlm::err(), store.clone());

        let result = writer
            .write_with_messages("sess-fail", "agent-root", "ward", sample_messages(2))
            .await;

        assert!(result.is_err(), "expected Err when LLM fails");
        assert!(
            !store.contains("handoff.latest"),
            "no fact should be written on LLM failure"
        );
    }

    // ---- Test 9: real correction_count ----

    #[tokio::test]
    async fn correction_count_reflects_real_count() {
        let store = MockFactStore::new();
        store
            .corrections
            .lock()
            .unwrap()
            .push("Use write_file not shell".to_string());
        store
            .corrections
            .lock()
            .unwrap()
            .push("Never CPU offload".to_string());

        let writer = make_writer(MockLlm::ok("Summary."), store.clone());
        writer
            .write_with_messages("sess-c", "agent-root", "test-ward", sample_messages(4))
            .await
            .unwrap();

        let raw = store.get("handoff.latest").unwrap();
        let entry: HandoffEntry = serde_json::from_str(&raw).unwrap();
        assert_eq!(entry.correction_count, 2);
    }

    #[tokio::test]
    async fn write_with_messages_preserves_ward_execution_and_artifact_pointers() {
        use agent_runtime::ToolCall;

        let store = MockFactStore::new();
        let writer = make_writer(MockLlm::ok("Summary."), store.clone());

        let mut delegate_call = ChatMessage::assistant("Delegating.".to_string());
        delegate_call.tool_calls = Some(vec![ToolCall {
            id: "call-1".to_string(),
            name: "delegate_to_agent".to_string(),
            arguments: serde_json::json!({
                "agent_id": "ward:financial-analysis",
                "task": "analyze AMD",
                "wait_for_result": true
            }),
        }]);
        let delegate_result = ChatMessage::tool_result(
            "call-1".to_string(),
            serde_json::json!({
                "agent_id": "ward:financial-analysis",
                "execution_id": "exec-ward-1",
                "status": "delegated"
            })
            .to_string(),
        );
        let mut respond_call = ChatMessage::assistant("Responding.".to_string());
        respond_call.tool_calls = Some(vec![ToolCall {
            id: "call-2".to_string(),
            name: "respond".to_string(),
            arguments: serde_json::json!({
                "artifacts": [
                    {"label": "Report", "path": "output/amd-peer-valuation.html"}
                ],
                "message": "done"
            }),
        }]);

        writer
            .write_with_messages(
                "sess-route",
                "root",
                "financial-analysis",
                vec![
                    ChatMessage::user("analyze AMD".to_string()),
                    delegate_call,
                    delegate_result,
                    respond_call,
                ],
            )
            .await
            .unwrap();

        let raw = store.get("handoff.latest").expect("handoff.latest missing");
        let entry: HandoffEntry = serde_json::from_str(&raw).unwrap();
        assert!(entry.route_hints.iter().any(|hint| {
            hint.ward_id == "financial-analysis"
                && hint.source_kind == RouteSourceKind::Episode
                && hint.execution_id.as_deref() == Some("exec-ward-1")
        }));
        assert!(entry.route_hints.iter().any(|hint| {
            hint.ward_id == "financial-analysis"
                && hint.source_kind == RouteSourceKind::Artifact
                && hint.source_path.as_deref() == Some("output/amd-peer-valuation.html")
        }));
    }

    // ---- messages_to_chat_format conversion ----

    #[test]
    fn messages_to_chat_format_parses_tool_calls() {
        let stored_tc = serde_json::json!([{
            "tool_id": "tc-1",
            "tool_name": "memory",
            "args": {"action": "get_fact"},
        }])
        .to_string();
        let msgs = vec![
            zbot_conversation::Message {
                id: "m1".into(),
                execution_id: None,
                session_id: "s".into(),
                role: "user".into(),
                content: "hi".into(),
                created_at: "now".into(),
                token_count: 1,
                tool_calls: None,
                tool_call_id: None,
                seq: 1,
            },
            zbot_conversation::Message {
                id: "m2".into(),
                execution_id: None,
                session_id: "s".into(),
                role: "assistant".into(),
                content: "ok".into(),
                created_at: "now".into(),
                token_count: 1,
                tool_calls: Some(stored_tc),
                tool_call_id: None,
                seq: 2,
            },
        ];
        let out = messages_to_chat_format(&msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "user");
        assert!(out[0].tool_calls.is_none());
        assert_eq!(out[1].role, "assistant");
        let tcs = out[1].tool_calls.as_ref().expect("tool_calls parsed");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "memory");
        assert_eq!(tcs[0].id, "tc-1");
    }
}
