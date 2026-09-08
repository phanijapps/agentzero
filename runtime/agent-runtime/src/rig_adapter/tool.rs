//! Bridge from AgentZero (`agent_primitives::Tool`) tools into Rig's tool dispatch.
//!
//! Rig owns the agent loop (multi-turn, tool scheduling, hooks, streaming).
//! AgentZero keeps owning its tools, their executable filtering, and the
//! runtime context they need. This module adapts an existing AgentZero tool
//! into Rig's object-safe [`ToolDyn`] so a Rig agent can call it without the
//! tool, its arguments, or its result knowing Rig exists.
//!
//! ## Hidden context — `ToolCallExtensions`, never model-visible args
//!
//! AgentZero tools receive an `Arc<ToolContext>` carrying session id, agent id,
//! ward, auth scopes, loaded-skill state, and similar runtime data that must
//! never be exposed to the model. Rig's equivalent carrier is
//! [`ToolCallExtensions`] — a typed map threaded into `call_with_extensions`.
//! The engine inserts the shared `Arc<ToolContext>` once per run
//! ([`SharedToolContext`]); each bridged tool reads it back. Because the same
//! `Arc` is shared across every tool call in a turn, state written by one tool
//! (e.g. `load_skill`) persists for the next, matching the current executor.
//!
//! ## Result shape — model-visible string only
//!
//! The adapter returns a model-visible `String`: a bare JSON string passes
//! through verbatim, any other JSON value becomes a JSON string (mirroring
//! Rig's own `serialize_tool_output`). AgentZero's richer `context_result`
//! rewriting — large-result offload, truncation, the raw/context/persisted/UI
//! distinction — is applied later by the engine through Rig's
//! `Flow::RewriteResult` hook, not here.

use std::sync::Arc;

use agent_primitives::Tool as ZeroTool;
use rig::completion::ToolDefinition;
use rig::tool::{ToolCallExtensions, ToolDyn, ToolError};
use rig::wasm_compat::WasmBoxedFuture;
use serde_json::{json, Value};

use super::tool_results::{SharedToolResults, ToolOutcome};
use crate::tools::context::ToolContext;

/// Shared AgentZero tool execution context carried through Rig's
/// `ToolCallExtensions`.
///
/// This is an alias, not a newtype, so the engine can keep constructing the
/// existing `Arc<ToolContext>` and merely `insert` it. The indirection through
/// a named type keeps the extension key stable and self-documenting at the
/// call site (`extensions.get::<SharedToolContext>()`).
pub type SharedToolContext = Arc<ToolContext>;

/// Adapter wrapping an existing AgentZero tool as a Rig [`ToolDyn`].
///
/// One adapter per AgentZero tool. Register the resulting `Box<dyn ToolDyn>`
/// values with `AgentBuilder::tools`.
pub struct RigToolAdapter {
    inner: Arc<dyn ZeroTool>,
}

#[derive(Debug, thiserror::Error)]
#[error("Host tool execution context is missing")]
struct MissingToolContext;

impl RigToolAdapter {
    /// Wrap an existing AgentZero tool.
    #[must_use]
    pub fn new(inner: Arc<dyn ZeroTool>) -> Self {
        Self { inner }
    }

    /// Wrap an existing AgentZero tool as a boxed Rig dynamic tool.
    #[must_use]
    pub fn boxed(inner: Arc<dyn ZeroTool>) -> Box<dyn ToolDyn> {
        Box::new(Self::new(inner))
    }
}

impl ToolDyn for RigToolAdapter {
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    fn definition<'a>(&'a self, _prompt: String) -> WasmBoxedFuture<'a, ToolDefinition> {
        // Clone the Arc so the returned future does not borrow `self`.
        let inner = self.inner.clone();
        Box::pin(async move {
            let parameters = crate::tool_schema::harden_tool_schema(
                inner
                    .parameters_schema()
                    .filter(|v| !v.is_null())
                    .unwrap_or_else(empty_object_schema),
            );
            ToolDefinition {
                name: inner.name().to_string(),
                description: inner.description().to_string(),
                parameters,
            }
        })
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        // No extensions on this path; the engine is expected to go through the
        // `call_with_extensions` entry point for real runs.
        self.dispatch(args, None, None)
    }

    fn call_with_extensions<'a>(
        &'a self,
        args: String,
        extensions: &'a ToolCallExtensions,
    ) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        // Extract hidden runtime context into an owned value up front so the
        // returned future does not borrow `extensions` (keeps the borrow off
        // the await boundary and off the `call` path's temporary).
        self.dispatch(
            args,
            extensions.get::<SharedToolContext>().cloned(),
            extensions.get::<SharedToolResults>().cloned(),
        )
    }
}

impl RigToolAdapter {
    /// Core dispatch shared by both `ToolDyn` entry points.
    ///
    /// Takes an owned context so the returned future borrows nothing from the
    /// caller. `shared_ctx` is required and cannot be supplied by model arguments;
    /// the engine inserts a real shared context for every run.
    ///
    /// Per-tool-call id is deliberately NOT threaded here: rig builds one
    /// `ToolCallExtensions` per request, so it cannot represent a distinct id
    /// per tool call in a turn, and writing it onto the shared `ToolContext`'s
    /// single `function_call_id` field would race under `tool_concurrency > 1`.
    /// The engine owns call-id fidelity (and its concurrency model) in T7.
    fn dispatch<'a>(
        &'a self,
        args: String,
        shared_ctx: Option<SharedToolContext>,
        results: Option<SharedToolResults>,
    ) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        let ctx = match shared_ctx {
            Some(ctx) => ctx,
            None => {
                return Box::pin(async {
                    Err(ToolError::ToolCallError(Box::new(MissingToolContext)))
                });
            }
        };
        let inner = self.inner.clone();
        Box::pin(async move {
            let started = std::time::Instant::now();
            let result = async {
                if results
                    .as_ref()
                    .is_some_and(|results| results.peer_influenced())
                    && inner.name() != "respond"
                {
                    return Ok(
                        json!({"blocked":true,"reason":"peer_data_authority_boundary"}).to_string(),
                    );
                }
                // LLMs send `null` for tools whose arguments are all optional. JSON
                // `null` parses to `Value::Null`, so normalize both the parsed-null
                // and the unparseable cases to an empty object.
                let args_value: Value = match serde_json::from_str::<Value>(&args) {
                    Ok(Value::Null) => Value::Object(Default::default()),
                    Ok(v) => v,
                    Err(_) if args.trim() == "null" => Value::Object(Default::default()),
                    Err(e) => return Err(ToolError::JsonError(e)),
                };

                if agent_tools::guards::planning_gate_blocks_tool(
                    ctx.as_ref(),
                    inner.name(),
                    &args_value,
                ) {
                    return Ok(agent_tools::guards::cold_graph_redirect().to_string());
                }

                if let Some(result) = crate::tool_replay::intercept(ctx.as_ref(), inner.name()) {
                    return Ok(result);
                }

                let result = inner
                    .execute(ctx, args_value)
                    .await
                    .map_err(|e| ToolError::ToolCallError(Box::new(e)))?;

                Ok(serialize_model_visible(result))
            }
            .await;
            if let Some(results) = results {
                let duration_ms = started.elapsed().as_millis() as i64;
                match result {
                    Ok(raw) => {
                        results.record(ToolOutcome {
                            raw: Some(raw.clone()),
                            duration_ms,
                            ..ToolOutcome::default()
                        });
                        Ok(raw)
                    }
                    Err(error) => {
                        let error = error.to_string();
                        results.record(ToolOutcome {
                            raw: Some(String::new()),
                            error: Some(error.clone()),
                            duration_ms,
                            ..ToolOutcome::default()
                        });
                        Ok(json!({"error":error}).to_string())
                    }
                }
            } else {
                result
            }
        })
    }
}

/// Render a tool result the way the model should see it.
///
/// Bare JSON string → verbatim; any other JSON value → JSON string. This is
/// the model-visible slice only; raw persistence and UI payloads are shaped
/// downstream by the engine.
fn serialize_model_visible(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

/// Empty JSON-Schema object used when an AgentZero tool declares no parameters.
fn empty_object_schema() -> Value {
    json!({"type": "object", "properties": {}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::CallbackContext;
    use agent_primitives::ToolContext as ZeroToolContext;
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// AgentZero tool that records what it was called with.
    struct RecordingTool {
        name: String,
        description: String,
        schema: Option<Value>,
        seen: Arc<Mutex<Vec<RecordedCall>>>,
    }

    #[derive(Clone, Debug)]
    struct RecordedCall {
        args: Value,
        agent_id: Option<String>,
        conversation_id: Option<String>,
        secret_from_state: Option<Value>,
    }

    impl RecordingTool {
        fn new(seen: Arc<Mutex<Vec<RecordedCall>>>) -> Self {
            Self {
                name: "record".to_string(),
                description: "Records its call".to_string(),
                schema: Some(json!({
                    "type": "object",
                    "properties": {"x": {"type": "number"}},
                    "required": ["x"]
                })),
                seen,
            }
        }
    }

    #[async_trait]
    impl ZeroTool for RecordingTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn parameters_schema(&self) -> Option<Value> {
            self.schema.clone()
        }
        async fn execute(
            &self,
            ctx: Arc<dyn ZeroToolContext>,
            args: Value,
        ) -> Result<Value, agent_primitives::error::AgentError> {
            let secret_from_state = ctx.get_state("app:hidden_auth_token");
            let record = RecordedCall {
                args,
                agent_id: ctx
                    .get_state("app:agent_id")
                    .and_then(|v| v.as_str().map(str::to_string)),
                conversation_id: ctx
                    .get_state("app:conversation_id")
                    .and_then(|v| v.as_str().map(str::to_string)),
                secret_from_state,
            };
            self.seen.lock().unwrap().push(record);
            Ok(json!({"ok": true}))
        }
    }

    fn shared_context_with_secret() -> SharedToolContext {
        let mut state = HashMap::new();
        state.insert(
            "app:hidden_auth_token".to_string(),
            json!("sk-secret-never-for-model"),
        );
        state.insert("app:agent_id".to_string(), json!("agent-7"));
        state.insert("app:conversation_id".to_string(), json!("conv-7"));
        Arc::new(ToolContext::full_with_state(
            "agent-7".to_string(),
            Some("conv-7".to_string()),
            Vec::new(),
            state,
        ))
    }

    #[tokio::test]
    async fn definition_maps_name_description_and_schema() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = RigToolAdapter::new(Arc::new(RecordingTool::new(seen)));
        let def = adapter.definition("ignored prompt".to_string()).await;

        assert_eq!(def.name, "record");
        assert_eq!(def.description, "Records its call");
        assert_eq!(
            def.parameters,
            json!({
                "type": "object",
                "properties": {"x": {"type": "number"}},
                "required": ["x"],
                "additionalProperties": false
            })
        );
    }

    #[tokio::test]
    async fn hidden_context_flows_via_extensions_not_args() {
        // AC7/AC10: the secret reaches the tool through the shared context, and
        // neither the args string nor the model-visible schema contains it.
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = RigToolAdapter::new(Arc::new(RecordingTool::new(seen.clone())));

        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(shared_context_with_secret());

        let args_string = json!({"x": 42}).to_string();
        let model_visible = adapter
            .call_with_extensions(args_string.clone(), &extensions)
            .await
            .expect("tool call");

        let call = {
            let calls = seen.lock().unwrap();
            assert_eq!(calls.len(), 1, "tool executed once");
            calls[0].clone()
        };
        // Tool received the model-supplied args verbatim.
        assert_eq!(call.args, json!({"x": 42}));
        // Hidden runtime context reached the tool from extensions.
        assert_eq!(call.agent_id.as_deref(), Some("agent-7"));
        assert_eq!(call.conversation_id.as_deref(), Some("conv-7"));
        assert_eq!(
            call.secret_from_state.as_ref().and_then(|v| v.as_str()),
            Some("sk-secret-never-for-model")
        );
        // The secret never reached the tool through args — it rode the shared
        // context — so what the tool observed as args, and what the model sees
        // as the result, both omit it. (Asserting against the caller-side
        // `args_string` would be tautological: the adapter cannot mutate it.)
        let observed_args = call.args.to_string();
        assert!(!observed_args.contains("sk-secret-never-for-model"));
        assert!(!model_visible.contains("sk-secret-never-for-model"));
        // And it is not in the schema the model sees either.
        let def = adapter.definition(String::new()).await;
        let schema_str = def.parameters.to_string();
        assert!(!schema_str.contains("sk-secret-never-for-model"));
        assert!(!schema_str.contains("auth_token"));
    }

    #[tokio::test]
    async fn cold_graph_gate_blocks_rig_tool_dispatch_before_execution() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = RigToolAdapter::new(Arc::new(RecordingTool {
            name: "shell".to_string(),
            description: "mutates the workspace".to_string(),
            schema: None,
            seen: seen.clone(),
        }));
        let ctx = shared_context_with_secret();
        ctx.set_state(
            agent_tools::guards::PLANNING_GATE_STATE.to_string(),
            serde_json::to_value(agent_tools::guards::PlanningGate::awaiting_ward(
                "Plan this graph request",
            ))
            .unwrap(),
        );
        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(ctx);

        let result = adapter
            .call_with_extensions("{}".to_string(), &extensions)
            .await
            .expect("gate returns a redirect");

        assert!(result.contains("redirect"));
        assert!(result.contains("planner-agent"));
        // Canonical shared message (guards::cold_graph_redirect) — pins the
        // exact text so this site can never silently drift from the executor's.
        assert!(result.contains("do not call MCP tools or other tools yet."));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn null_args_normalize_to_empty_object() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = RigToolAdapter::new(Arc::new(RecordingTool::new(seen.clone())));

        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(shared_context_with_secret());

        adapter
            .call_with_extensions("null".to_string(), &extensions)
            .await
            .expect("tool call");

        let calls = seen.lock().unwrap();
        assert_eq!(calls[0].args, Value::Object(Default::default()));
    }

    #[tokio::test]
    async fn direct_dispatch_respects_host_owned_peer_authority() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = RigToolAdapter::new(Arc::new(RecordingTool::new(seen.clone())));
        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(shared_context_with_secret());
        let results = Arc::new(super::super::tool_results::ToolResults::default());
        results.mark_peer_influenced();
        extensions.insert::<SharedToolResults>(results);
        let response = adapter
            .call_with_extensions("{\"peer_influenced\":false}".into(), &extensions)
            .await
            .unwrap();
        assert!(response.contains("peer_data_authority_boundary"));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn result_string_passes_through_object_becomes_json() {
        // AC11 (model-visible slice): bare string verbatim, object -> JSON string.
        struct StringTool;
        #[async_trait]
        impl ZeroTool for StringTool {
            fn name(&self) -> &str {
                "string-tool"
            }
            fn description(&self) -> &str {
                "returns text"
            }
            async fn execute(
                &self,
                _ctx: Arc<dyn ZeroToolContext>,
                _args: Value,
            ) -> Result<Value, agent_primitives::error::AgentError> {
                Ok(Value::String("plain text result".to_string()))
            }
        }
        struct ObjectTool;
        #[async_trait]
        impl ZeroTool for ObjectTool {
            fn name(&self) -> &str {
                "object-tool"
            }
            fn description(&self) -> &str {
                "returns object"
            }
            async fn execute(
                &self,
                _ctx: Arc<dyn ZeroToolContext>,
                _args: Value,
            ) -> Result<Value, agent_primitives::error::AgentError> {
                Ok(json!({"path": "/a/b", "bytes": 10}))
            }
        }

        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(shared_context_with_secret());

        let s = RigToolAdapter::new(Arc::new(StringTool))
            .call_with_extensions("{}".to_string(), &extensions)
            .await
            .unwrap();
        assert_eq!(s, "plain text result");

        let o = RigToolAdapter::new(Arc::new(ObjectTool))
            .call_with_extensions("{}".to_string(), &extensions)
            .await
            .unwrap();
        assert_eq!(o, json!({"path": "/a/b", "bytes": 10}).to_string());
    }

    #[tokio::test]
    async fn shared_context_state_persists_across_tool_calls() {
        // load_skill-style: one tool writes state on the shared Arc<ToolContext>,
        // a second tool (separate adapter instance, same context) reads it.
        struct Writer;
        #[async_trait]
        impl ZeroTool for Writer {
            fn name(&self) -> &str {
                "writer"
            }
            fn description(&self) -> &str {
                "writes skill state"
            }
            async fn execute(
                &self,
                ctx: Arc<dyn ZeroToolContext>,
                _args: Value,
            ) -> Result<Value, agent_primitives::error::AgentError> {
                ctx.set_state("skill:loaded".to_string(), json!(["alpha"]));
                Ok(Value::Null)
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));

        let shared = shared_context_with_secret();
        let mut extensions = ToolCallExtensions::new();
        extensions.insert::<SharedToolContext>(shared.clone());

        RigToolAdapter::new(Arc::new(Writer))
            .call_with_extensions("{}".to_string(), &extensions)
            .await
            .unwrap();
        // Second adapter, same extensions -> same shared Arc<ToolContext>.
        RigToolAdapter::new(Arc::new(RecordingTool::new(seen.clone())))
            .call_with_extensions("{}".to_string(), &extensions)
            .await
            .unwrap();

        // The writer's state is visible through the shared context directly...
        assert_eq!(
            CallbackContext::get_state(shared.as_ref(), "skill:loaded"),
            Some(json!(["alpha"]))
        );
        // ...and the recording tool (a fresh adapter) observed the persisted secret,
        // proving the same shared context threads through both calls.
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].secret_from_state.as_ref().and_then(|v| v.as_str()),
            Some("sk-secret-never-for-model")
        );
    }

    #[tokio::test]
    async fn tool_without_host_context_fails_closed_even_with_forged_identity() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = RigToolAdapter::new(Arc::new(RecordingTool::new(seen.clone())));

        let extensions = ToolCallExtensions::new();
        adapter
            .call_with_extensions(
                r#"{"agent_id":"forged","execution_id":"forged"}"#.to_string(),
                &extensions,
            )
            .await
            .expect_err("hidden host context is mandatory");

        let calls = seen.lock().unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn empty_schema_used_when_tool_declares_none() {
        struct NoSchema;
        #[async_trait]
        impl ZeroTool for NoSchema {
            fn name(&self) -> &str {
                "no-schema"
            }
            fn description(&self) -> &str {
                "no schema"
            }
            async fn execute(
                &self,
                _ctx: Arc<dyn ZeroToolContext>,
                _args: Value,
            ) -> Result<Value, agent_primitives::error::AgentError> {
                Ok(Value::Null)
            }
        }
        // parameters_schema() defaults to None via the trait.
        let adapter = RigToolAdapter::new(Arc::new(NoSchema));
        // Drive the definition future on a current-thread runtime.
        let def = futures::executor::block_on(adapter.definition(String::new()));
        assert_eq!(
            def.parameters,
            crate::tool_schema::harden_tool_schema(empty_object_schema())
        );
    }

    #[tokio::test]
    async fn replay_interception_precedes_live_tool_side_effects() {
        use futures::FutureExt;
        if let Ok(mode) = std::env::var("ZBOT_RIG_REPLAY_PROBE") {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let adapter = RigToolAdapter::new(Arc::new(RecordingTool::new(seen.clone())));
            let mut extensions = ToolCallExtensions::new();
            extensions.insert::<SharedToolContext>(shared_context_with_secret());
            let output = std::panic::AssertUnwindSafe(
                adapter.call_with_extensions("{}".into(), &extensions),
            )
            .catch_unwind()
            .await;
            match mode.as_str() {
                "hit" => {
                    assert_eq!(output.unwrap().unwrap(), "recorded without execution");
                    assert!(seen.lock().unwrap().is_empty());
                }
                "strict" | "drift" => {
                    assert!(output.is_err());
                    assert!(seen.lock().unwrap().is_empty());
                }
                "lenient" => {
                    output.unwrap().unwrap();
                    assert_eq!(seen.lock().unwrap().len(), 1);
                }
                _ => panic!("unknown probe"),
            }
            return;
        }
        // The replay store intentionally initializes once per process. Fresh
        // child test processes exercise its real environment boundary without
        // racing unrelated tests or injecting a test-only dispatch mechanism.
        for mode in ["hit", "strict", "drift", "lenient"] {
            let dir = tempfile::tempdir().unwrap();
            let record = if mode == "hit" || mode == "drift" {
                json!({"execution_id":"conv-7","tool_index":0,"tool_name":if mode=="drift" {"other"}else{"record"},"args_hash":"fixture","result":"recorded without execution"}).to_string()
            } else {
                String::new()
            };
            std::fs::write(dir.path().join("tool-results.jsonl"), record).unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "rig_adapter::tool::tests::replay_interception_precedes_live_tool_side_effects",
                    "--nocapture",
                ])
                .env("ZBOT_RIG_REPLAY_PROBE", mode)
                .env("ZBOT_REPLAY_DIR", dir.path())
                .env(
                    "ZBOT_REPLAY_STRICT",
                    if mode == "lenient" { "0" } else { "1" },
                )
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
    }
}
