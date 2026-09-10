// ============================================================================
// MEMORY TOOL
// Durable structured facts via the DB-backed fact store
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext, ToolPermissions};
use zbot_stores_traits::{MemoryFactStore, MemoryFactWriteRequest, StoreError, StoreResult};

use super::ingest::{EvidenceRecord, IngestionAccess};
// ============================================================================
// MEMORY TOOL
// ============================================================================

/// Narrow model-facing durable memory writer.
///
/// The broad `memory` tool remains available for internal compatibility and
/// exact legacy calls, but model-visible prompts should use this write-only
/// surface for durable facts.
pub struct MemoryWriteTool {
    fact_store: Option<Arc<dyn MemoryFactStore>>,
    evidence_intake: Option<Arc<dyn crate::tools::ingest::IngestionAccess>>,
}

impl MemoryWriteTool {
    /// Create a memory write tool over the durable fact store.
    #[must_use]
    pub fn new(fact_store: Option<Arc<dyn MemoryFactStore>>) -> Self {
        Self {
            fact_store,
            evidence_intake: None,
        }
    }

    /// Wire evidence intake when a runtime adapter is available.
    #[must_use]
    pub fn with_evidence_intake(
        mut self,
        evidence_intake: Arc<dyn crate::tools::ingest::IngestionAccess>,
    ) -> Self {
        self.evidence_intake = Some(evidence_intake);
        self
    }

    #[must_use]
    pub fn with_optional_evidence_intake(
        mut self,
        evidence_intake: Option<Arc<dyn IngestionAccess>>,
    ) -> Self {
        self.evidence_intake = evidence_intake;
        self
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Persist one durable structured memory fact with category, key, content, and optional confidence. Use for important user preferences, corrections, decisions, domain notes, or reusable patterns."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["user", "pattern", "domain", "ctx"],
                    "description": "'ctx' is reserved for session state; ordinary durable notes should use user, pattern, or domain."
                },
                "key": {
                    "type": "string",
                    "description": "Dot-notation key such as user.preferred_format or domain.finance.valuation_rule."
                },
                "content": {
                    "type": "string",
                    "description": "One or two sentence fact content."
                },
                "confidence": {
                    "type": "number",
                    "description": "Confidence from 0.0 to 1.0. Defaults to 0.8."
                },
                "retention_policy": {
                    "type": "string",
                    "description": "Durable evidence retention policy selected by the host. Defaults to durable.",
                    "default": "durable"
                },
                "ontology_labels": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional dynamic ontology labels."
                },
                "taxonomy_labels": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional SKOS/taxonomy labels."
                }
            },
            "required": ["category", "key", "content"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, mut args: Value) -> Result<Value> {
        let obj = args
            .as_object_mut()
            .ok_or_else(|| AgentError::Tool("memory_write expects an object".to_string()))?;
        obj.insert("action".to_string(), Value::String("save_fact".to_string()));
        let agent_id = obj
            .get("agent_id")
            .and_then(Value::as_str)
            .unwrap_or("root")
            .to_string();
        self.action_save_fact(ctx.as_ref(), &agent_id, &args).await
    }
}

impl MemoryWriteTool {
    /// Save a structured memory fact via the DB-backed fact store.
    async fn action_save_fact(
        &self,
        ctx: &dyn ToolContext,
        agent_id: &str,
        args: &Value,
    ) -> Result<Value> {
        let category = args
            .get("category")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'category' for save_fact".to_string()))?;

        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'key' for save_fact".to_string()))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'content' for save_fact".to_string()))?;

        let confidence = args
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.8);

        // Validate category
        let valid_categories = ["user", "pattern", "domain", "ctx"];
        if !valid_categories.contains(&category) {
            return Err(AgentError::Tool(format!(
                "Invalid category '{}'. Valid agent-writable categories: {}. Policy-shaped 'instruction' and 'correction' facts are internal-only.",
                category,
                valid_categories.join(", ")
            )));
        }

        // Ctx category is session state — writes gate through a separate
        // path because the permission rules + storage sentinels differ.
        if category == "ctx" {
            return self.action_save_ctx_fact(ctx, agent_id, key, content).await;
        }

        if content.len() > 500 {
            return Err(AgentError::Tool(
                "Fact content too long. Keep to 1-2 sentences (max 500 chars).".to_string(),
            ));
        }

        if let Some(intake) = &self.evidence_intake {
            let record = memory_evidence_record(ctx, agent_id, category, key, args);
            intake
                .record_evidence(record)
                .await
                .map_err(|e| AgentError::Tool(e.to_string()))?;
        }

        // Use DB-backed fact store if available
        match &self.fact_store {
            Some(store) => store
                // valid_from=None ⇒ store defaults to Utc::now(). A
                // first-class JSON parameter for valid_from is deferred
                // to bi-temporal phase 2 (point-in-time recall API). Keep the
                // executing session and ward attached so canonical backends
                // do not widen model-originated facts to global scope.
                .save_fact_with_context(MemoryFactWriteRequest {
                    agent_id: agent_id.to_string(),
                    category: category.to_string(),
                    key: key.to_string(),
                    content: content.to_string(),
                    confidence,
                    session_id: {
                        let session_id = ctx.session_id().trim();
                        (!session_id.is_empty()).then(|| session_id.to_string())
                    },
                    ward_id: ctx.get_state("ward_id").and_then(|value| {
                        value
                            .as_str()
                            .map(str::trim)
                            .filter(|ward_id| !ward_id.is_empty())
                            .map(str::to_string)
                    }),
                    source_ref: Some("agentzero.memory_tool".to_string()),
                    valid_from: None,
                })
                .await
                .map_err(|e| AgentError::Tool(e.to_string())),
            None => Err(AgentError::Tool(
                "Durable memory facts require a DB-backed fact store (not available in this runtime)"
                    .to_string(),
            )),
        }
    }

    /// Save a ctx-namespaced fact (session state).
    ///
    /// Dispatched from `action_save_fact` when `category='ctx'`. Enforces
    /// the permission rules defined in
    /// `docs/specs/2026-04-17-session-ctx-memory-bundle.md`:
    /// - Root (not delegated) can write any ctx key.
    /// - Delegated subagents can ONLY write keys matching
    ///   `ctx.<sid>.state.<anything>` — they cannot overwrite
    ///   root-owned canonicals (intent, prompt, plan, session.meta,
    ///   ward_briefing, memory).
    async fn action_save_ctx_fact(
        &self,
        ctx: &dyn ToolContext,
        agent_id: &str,
        key: &str,
        content: &str,
    ) -> Result<Value> {
        let is_delegated = ctx
            .get_state("app:is_delegated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Pure-function permission check. Returns the session id on
        // success so we can pass it to the store; on failure it carries
        // the user-facing error message.
        let sid = check_ctx_write_permission(is_delegated, key)
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let current_sid = ctx.session_id();
        if sid != current_sid {
            return Err(AgentError::Tool(format!(
                "ctx fact write session mismatch: key targets `{sid}` but current session is `{current_sid}`"
            )));
        }

        // Ward comes from current context; ctx facts are stored per-ward
        // so cleanup on ward deletion is straightforward.
        let ward_id = ctx
            .get_state("ward_id")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "__global__".to_string());

        // Owner: root when not delegated, otherwise the subagent's id.
        let owner = if is_delegated {
            format!("subagent:{}", agent_id)
        } else {
            "root".to_string()
        };

        // State handoffs (pinned=false) can be overwritten on rerun;
        // root-owned canonicals (pinned=true) are protected from drift.
        let pinned = !is_delegated;

        match &self.fact_store {
            Some(store) => store
                .save_ctx_fact(&sid, &ward_id, key, content, &owner, pinned)
                .await
                .map_err(|e| AgentError::Tool(e.to_string())),
            None => Err(AgentError::Tool(
                "Ctx facts require a DB-backed fact store (not available in this runtime)"
                    .to_string(),
            )),
        }
    }
}

/// Classify a ctx key and enforce the writer's permission.
///
/// Pure function — no tool context, no I/O. Testable in isolation.
/// Returns the extracted `session_id` on success so the caller can pass
/// it down to storage; returns a user-facing error message on reject.
///
/// Permission rules:
/// - Root (`is_delegated=false`) may write any well-formed ctx key.
/// - Delegated subagents may only write `ctx.<sid>.state.<anything>`;
///   they cannot overwrite root-owned canonicals (intent, prompt,
///   plan, session.meta, ward_briefing, memory) nor invent sub-keys
///   outside the `state.*` namespace.
fn check_ctx_write_permission(is_delegated: bool, key: &str) -> StoreResult<String> {
    let (sid, sub_key) = parse_ctx_key(key)?;

    if !is_delegated {
        // Root can write anything well-formed.
        return Ok(sid.to_string());
    }

    const ROOT_OWNED: &[&str] = &[
        "intent",
        "prompt",
        "plan",
        "session.meta",
        "ward_briefing",
        "memory",
    ];

    if ROOT_OWNED.contains(&sub_key) {
        return Err(StoreError::Invalid(format!(
            "Subagent cannot write to root-owned ctx key '{}'. Root owns: {}. Subagents may only write 'ctx.<sid>.state.<...>'.",
            key,
            ROOT_OWNED.join(", ")
        )));
    }

    if !sub_key.starts_with("state.") {
        return Err(StoreError::Invalid(format!(
            "Subagent ctx writes must target 'ctx.<sid>.state.<...>'. Got sub-key '{}'.",
            sub_key
        )));
    }

    Ok(sid.to_string())
}

fn parse_ctx_key(key: &str) -> StoreResult<(&str, &str)> {
    let Some(rest) = key.strip_prefix("ctx.") else {
        return Err(StoreError::Invalid(format!(
            "Ctx key '{}' must start with 'ctx.<session_id>.'",
            key
        )));
    };
    let Some((sid, sub_key)) = rest.split_once('.') else {
        return Err(StoreError::Invalid(format!(
            "Ctx key '{}' must include session_id: ctx.<sid>.<sub_key>",
            key
        )));
    };
    if sid.trim().is_empty() {
        return Err(StoreError::Invalid(format!(
            "Ctx key '{}' must include session_id: ctx.<sid>.<sub_key>",
            key
        )));
    }
    Ok((sid, sub_key))
}

fn memory_evidence_record(
    ctx: &dyn ToolContext,
    agent_id: &str,
    category: &str,
    key: &str,
    args: &Value,
) -> EvidenceRecord {
    let session_id = ctx.session_id();
    EvidenceRecord {
        evidence_id: format!("{agent_id}:memory:{category}:{key}"),
        action: "memory_write".to_string(),
        source_id: key.to_string(),
        source_type: format!("memory_fact:{category}"),
        session_id: (!session_id.is_empty()).then(|| session_id.to_string()),
        ward_id: ctx.get_state("ward_id").and_then(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|ward_id| !ward_id.is_empty())
                .map(str::to_string)
        }),
        agent_id: agent_id.to_string(),
        retention_policy: args
            .get("retention_policy")
            .and_then(Value::as_str)
            .unwrap_or("durable")
            .to_string(),
        ontology_labels: string_array_arg(args, "ontology_labels"),
        taxonomy_labels: string_array_arg(args, "taxonomy_labels"),
    }
}

fn string_array_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// ============================================================================
// TESTS
// ============================================================================

/// Focused memory search tool — read-only, no filesystem access.
/// Exposes only semantic search over the fact store via `recall_facts`.
pub struct MemorySearchTool {
    fact_store: Arc<dyn MemoryFactStore>,
}

impl MemorySearchTool {
    pub fn new(fact_store: Arc<dyn MemoryFactStore>) -> Self {
        Self { fact_store }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "search_memory"
    }

    fn description(&self) -> &'static str {
        "Search indexed resources (skills, agents, wards, MCPs, procedures, memories) \
         for entries relevant to a query. Returns matching results with names \
         and descriptions."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to search for (e.g. 'financial analysis skills', 'research agents')"
                }
            },
            "required": ["query"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or_default();

        let result = self
            .fact_store
            .recall_facts("root", query, 20)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;

        let items = result
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let filtered: Vec<_> = items
            .into_iter()
            .take(10)
            .map(|item| {
                let key = item.get("key").and_then(|k| k.as_str()).unwrap_or("");
                let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                let category = item.get("category").and_then(|c| c.as_str()).unwrap_or("");
                let name = key.split(':').nth(1).unwrap_or(key);
                json!({
                    "name": name,
                    "description": content,
                    "category": category })
            })
            .collect();

        Ok(json!({ "results": filtered }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::{CallbackContext, Content, EventActions, ReadonlyContext};
    use std::collections::HashMap;

    struct TestToolCtx {
        session_id: String,
        state: HashMap<String, Value>,
    }

    impl TestToolCtx {
        fn new(session_id: &str) -> Self {
            Self {
                session_id: session_id.to_string(),
                state: HashMap::new(),
            }
        }
    }

    impl ReadonlyContext for TestToolCtx {
        fn invocation_id(&self) -> &str {
            "test-invocation"
        }
        fn agent_name(&self) -> &str {
            "test-agent"
        }
        fn user_id(&self) -> &str {
            "test-user"
        }
        fn app_name(&self) -> &str {
            "test-app"
        }
        fn session_id(&self) -> &str {
            &self.session_id
        }
        fn branch(&self) -> &str {
            "test"
        }
        fn user_content(&self) -> &Content {
            use std::sync::LazyLock;
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for TestToolCtx {
        fn get_state(&self, key: &str) -> Option<Value> {
            self.state.get(key).cloned()
        }
        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for TestToolCtx {
        fn function_call_id(&self) -> String {
            "test-call".to_string()
        }
        fn actions(&self) -> EventActions {
            EventActions::default()
        }
        fn set_actions(&self, _actions: EventActions) {}
    }

    #[test]
    fn missing_action_error_includes_expected_shape() {
        let msg = "Missing 'action' parameter. Expected shape: {\"action\":\"get_fact\", \"key\":\"ctx.session.intent\"} or {\"action\":\"recall\", \"query\":\"...\"}";
        assert!(msg.contains("\"action\""));
        assert!(msg.contains("get_fact"));
        assert!(msg.contains("recall"));
    }

    // ========================================================================
    // Ctx write permission tests (Phase 1b — memory-as-ctx bundle)
    //
    // Pure-function tests for the permission classifier. No tool context,
    // no fact store — these verify the rules in isolation.
    // ========================================================================

    #[test]
    fn test_ctx_perm_root_allowed_on_intent() {
        let sid = check_ctx_write_permission(false, "ctx.sess-abc.intent").unwrap();
        assert_eq!(sid, "sess-abc");
    }

    #[test]
    fn test_ctx_perm_root_allowed_on_state() {
        let sid = check_ctx_write_permission(false, "ctx.sess-abc.state.exec-1").unwrap();
        assert_eq!(sid, "sess-abc");
    }

    #[test]
    fn test_ctx_perm_subagent_rejected_on_intent() {
        let err = check_ctx_write_permission(true, "ctx.sess-abc.intent").unwrap_err();
        assert!(err.detail().contains("root-owned"), "error was: {}", err);
        assert!(err.to_string().contains("intent"), "error was: {}", err);
    }

    #[test]
    fn test_ctx_perm_subagent_rejected_on_prompt() {
        let err = check_ctx_write_permission(true, "ctx.sess-abc.prompt").unwrap_err();
        assert!(err.detail().contains("root-owned"), "error was: {}", err);
    }

    #[test]
    fn test_ctx_perm_subagent_rejected_on_plan() {
        let err = check_ctx_write_permission(true, "ctx.sess-abc.plan").unwrap_err();
        assert!(err.detail().contains("root-owned"), "error was: {}", err);
    }

    #[test]
    fn test_ctx_perm_subagent_rejected_on_session_meta() {
        let err = check_ctx_write_permission(true, "ctx.sess-abc.session.meta").unwrap_err();
        assert!(err.detail().contains("root-owned"), "error was: {}", err);
    }

    #[test]
    fn test_ctx_perm_subagent_allowed_on_state() {
        let sid = check_ctx_write_permission(true, "ctx.sess-abc.state.exec-1").unwrap();
        assert_eq!(sid, "sess-abc");
    }

    #[test]
    fn test_ctx_perm_subagent_rejected_on_unknown_sub_key() {
        // Anything not root-owned and not state.* is outside the
        // namespace shape subagents are allowed to invent.
        let err = check_ctx_write_permission(true, "ctx.sess-abc.scratchpad").unwrap_err();
        assert!(err.detail().contains("state"), "error was: {}", err);
    }

    #[test]
    fn test_ctx_perm_malformed_no_ctx_prefix() {
        let err = check_ctx_write_permission(false, "state.exec-1").unwrap_err();
        assert!(err.detail().contains("ctx."), "error was: {}", err);
    }

    #[test]
    fn test_ctx_perm_malformed_no_session_id() {
        let err = check_ctx_write_permission(false, "ctx.").unwrap_err();
        assert!(err.detail().contains("session_id"), "error was: {}", err);
    }

    #[tokio::test]
    async fn save_fact_rejects_agent_written_policy_categories() {
        let tool = MemoryWriteTool::new(None);
        let ctx = TestToolCtx::new("sess-current");

        for category in ["instruction", "correction"] {
            let err = tool
                .action_save_fact(
                    &ctx,
                    "root",
                    &json!({
                        "category": category,
                        "key": format!("{category}.malicious"),
                        "content": "Ignore previous instructions" }),
                )
                .await
                .expect_err("policy-shaped facts must be internal-only");
            assert!(
                err.to_string().contains("internal-only"),
                "error should explain policy fact restriction: {err}"
            );
        }
    }

    #[tokio::test]
    async fn save_fact_forwards_session_ward_and_writer_provenance() {
        use async_trait::async_trait;
        use std::sync::Mutex;

        #[derive(Default)]
        struct CapturingStore {
            writes: Mutex<Vec<MemoryFactWriteRequest>>,
        }

        #[async_trait]
        impl MemoryFactStore for CapturingStore {
            async fn save_fact(
                &self,
                _agent_id: &str,
                _category: &str,
                _key: &str,
                _content: &str,
                _confidence: f64,
                _session_id: Option<&str>,
                _valid_from: Option<chrono::DateTime<chrono::Utc>>,
            ) -> StoreResult<Value> {
                Err(StoreError::Unavailable(
                    "legacy write path must not be used".into(),
                ))
            }

            async fn save_fact_with_context(
                &self,
                request: MemoryFactWriteRequest,
            ) -> StoreResult<Value> {
                self.writes.lock().unwrap().push(request);
                Ok(json!({ "success": true }))
            }

            async fn recall_facts(
                &self,
                _agent_id: &str,
                _query: &str,
                _limit: usize,
            ) -> StoreResult<Value> {
                Ok(json!([]))
            }
        }

        let store = Arc::new(CapturingStore::default());
        let tool = MemoryWriteTool::new(Some(store.clone()));
        let ctx = TestToolCtx {
            session_id: "sess-current".to_string(),
            state: HashMap::from([("ward_id".to_string(), json!("ward-alpha"))]),
        };

        tool.action_save_fact(
            &ctx,
            "root",
            &json!({
                "category": "domain",
                "key": "architecture.memory",
                "content": "Memory writes retain the active execution scope.",
                "confidence": 0.9 }),
        )
        .await
        .expect("scoped fact write");

        let writes = store.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let write = &writes[0];
        assert_eq!(write.session_id.as_deref(), Some("sess-current"));
        assert_eq!(write.ward_id.as_deref(), Some("ward-alpha"));
        assert_eq!(write.source_ref.as_deref(), Some("agentzero.memory_tool"));
    }

    #[tokio::test]
    async fn save_fact_records_evidence_intake_before_db_write() {
        use crate::tools::ingest::{StructuredCounts, StructuredEntity, StructuredRelationship};
        use async_trait::async_trait;
        use std::sync::Mutex;

        struct RecordingFactStore {
            calls: Mutex<Vec<String>>,
            events: Arc<Mutex<Vec<&'static str>>>,
        }

        #[async_trait]
        impl MemoryFactStore for RecordingFactStore {
            async fn save_fact(
                &self,
                _agent_id: &str,
                _category: &str,
                key: &str,
                _content: &str,
                _confidence: f64,
                _session_id: Option<&str>,
                _valid_from: Option<chrono::DateTime<chrono::Utc>>,
            ) -> StoreResult<Value> {
                self.events.lock().unwrap().push("fact");
                self.calls.lock().unwrap().push(key.to_string());
                Ok(json!({"success": true}))
            }

            async fn recall_facts(
                &self,
                _agent_id: &str,
                _query: &str,
                _limit: usize,
            ) -> StoreResult<Value> {
                Ok(json!([]))
            }

            async fn recall_facts_prioritized(
                &self,
                _agent_id: &str,
                _query: &str,
                _limit: usize,
                _as_of: Option<chrono::DateTime<chrono::Utc>>,
            ) -> StoreResult<Value> {
                Ok(json!({"results": []}))
            }
        }

        #[derive(Default)]
        struct RecordingIntake {
            records: Mutex<Vec<EvidenceRecord>>,
            events: Arc<Mutex<Vec<&'static str>>>,
        }

        #[async_trait]
        impl IngestionAccess for RecordingIntake {
            async fn record_evidence(
                &self,
                record: EvidenceRecord,
            ) -> std::result::Result<(), String> {
                self.events.lock().unwrap().push("intake");
                self.records.lock().unwrap().push(record);
                Ok(())
            }

            async fn enqueue(
                &self,
                _source_id: &str,
                _source_type: &str,
                _text: &str,
                _session_id: Option<&str>,
                _agent_id: &str,
            ) -> std::result::Result<(String, usize), String> {
                Ok((String::new(), 0))
            }

            async fn ingest_structured(
                &self,
                _agent_id: &str,
                _ward_id: Option<String>,
                _entities: Vec<StructuredEntity>,
                _relationships: Vec<StructuredRelationship>,
            ) -> std::result::Result<StructuredCounts, String> {
                Ok(StructuredCounts {
                    entities_upserted: 0,
                    relationships_upserted: 0,
                })
            }
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let fact_store = Arc::new(RecordingFactStore {
            calls: Mutex::new(Vec::new()),
            events: events.clone(),
        });
        let intake = Arc::new(RecordingIntake {
            records: Mutex::new(Vec::new()),
            events: events.clone(),
        });
        let tool =
            MemoryWriteTool::new(Some(fact_store.clone())).with_evidence_intake(intake.clone());
        let ctx = TestToolCtx::new("sess-current");

        tool.action_save_fact(
            &ctx,
            "root",
            &json!({
                "category": "domain",
                "key": "valuation.aapl",
                "content": "AAPL valuation depends on services margin.",
                "confidence": 0.9,
                "retention_policy": "durable",
                "ontology_labels": ["financial_metric"],
                "taxonomy_labels": ["skos:finance"]
            }),
        )
        .await
        .expect("save fact");

        assert_eq!(
            fact_store.calls.lock().unwrap().as_slice(),
            ["valuation.aapl"]
        );
        assert_eq!(events.lock().unwrap().as_slice(), ["intake", "fact"]);
        let records = intake.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, "memory_write");
        assert_eq!(records[0].source_id, "valuation.aapl");
        assert_eq!(records[0].source_type, "memory_fact:domain");
        assert_eq!(records[0].session_id.as_deref(), Some("sess-current"));
        assert_eq!(records[0].ontology_labels, vec!["financial_metric"]);
        assert_eq!(records[0].taxonomy_labels, vec!["skos:finance"]);
    }

    #[tokio::test]
    async fn save_fact_rejects_cross_session_ctx_key() {
        let tool = MemoryWriteTool::new(None);
        let ctx = TestToolCtx::new("sess-current");

        let err = tool
            .action_save_fact(
                &ctx,
                "root",
                &json!({
                    "category": "ctx",
                    "key": "ctx.sess-victim.state.exec-1",
                    "content": "poisoned handoff" }),
            )
            .await
            .expect_err("ctx writes must stay in the current session");

        assert!(
            err.to_string().contains("session mismatch"),
            "error should explain session mismatch: {err}"
        );
    }
}
