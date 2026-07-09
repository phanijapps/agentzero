//! End-to-end procedure-as-callable test.
//!
//! Builds a 2-step procedure (with arg interpolation between steps),
//! runs it via RunProcedureTool against a real ToolRegistry, asserts:
//!   - success_count goes 1 → 2 (insert sets to 1, success increments)
//!   - The final result contains the upper-cased greeting
//!   - Interpolation properly threads step_0.echoed → step_1.in

use agent_primitives::{Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use zbot_stores_traits::{Procedure, ProcedureStore};

use agent_runtime::tools::context::ToolContext as ConcreteCtx;
use agent_runtime::tools::registry::ToolRegistry;
use agent_runtime::tools::run_procedure::RunProcedureTool;

/// Echoes input args as `{ echoed: <args> }`.
struct EchoTool;
#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn description(&self) -> &'static str {
        "echo input"
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        Ok(json!({ "echoed": args }))
    }
}

/// Takes args.in (string) and returns its uppercase form.
struct UpperTool;
#[async_trait]
impl Tool for UpperTool {
    fn name(&self) -> &'static str {
        "to_upper"
    }
    fn description(&self) -> &'static str {
        "uppercase a string"
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let s = args.get("in").and_then(|v| v.as_str()).unwrap_or("");
        Ok(json!(s.to_uppercase()))
    }
}

struct InMemStore {
    proc: TokioMutex<Procedure>,
}
impl InMemStore {
    fn new(proc: Procedure) -> Self {
        Self {
            proc: TokioMutex::new(proc),
        }
    }
    async fn current(&self) -> Procedure {
        self.proc.lock().await.clone()
    }
}
#[async_trait]
impl ProcedureStore for InMemStore {
    async fn get_procedure_by_name(
        &self,
        _agent_id: &str,
        name: &str,
    ) -> std::result::Result<Option<Procedure>, String> {
        let p = self.proc.lock().await;
        if p.name == name {
            Ok(Some(p.clone()))
        } else {
            Ok(None)
        }
    }
    async fn increment_success(
        &self,
        id: &str,
        _d: Option<i64>,
        _t: Option<i64>,
    ) -> std::result::Result<(), String> {
        let mut p = self.proc.lock().await;
        if p.id == id {
            p.success_count += 1;
        }
        Ok(())
    }
    async fn increment_failure(&self, id: &str) -> std::result::Result<(), String> {
        let mut p = self.proc.lock().await;
        if p.id == id {
            p.failure_count += 1;
        }
        Ok(())
    }
}

#[tokio::test]
async fn procedure_e2e_writes_runs_and_increments_success() {
    // Build a 2-step procedure:
    //   step_0: echo {in: "{args.greeting}"} → {echoed: {in: "hello"}}
    //   step_1: to_upper {in: "{step_0.echoed.in}"} → "HELLO"
    let steps_json = serde_json::to_string(&vec![
        json!({
            "action": "echo",
            "args": {"in": "{args.greeting}"},
            "binds": ["echoed"]
        }),
        json!({
            "action": "to_upper",
            "args": {"in": "{step_0.echoed.in}"},
            "binds": []
        }),
    ])
    .unwrap();

    let proc = Procedure {
        id: "proc_e2e".into(),
        agent_id: "root".into(),
        ward_id: Some("test_ward".into()),
        name: "shout".into(),
        description: "uppercase a greeting".into(),
        trigger_pattern: None,
        steps: steps_json,
        parameters: Some(r#"["greeting"]"#.into()),
        success_count: 1,
        failure_count: 0,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: "".into(),
        updated_at: "".into(),
    };
    let store = Arc::new(InMemStore::new(proc));

    // Build the registry the run_procedure tool will dispatch against.
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    registry.register(Arc::new(UpperTool));
    let registry = Arc::new(registry);

    let rp = RunProcedureTool::new(registry.clone(), store.clone());
    let ctx: Arc<dyn ToolContext> = Arc::new(ConcreteCtx::full_with_state(
        "root".into(),
        Some("c1".into()),
        vec![],
        Default::default(),
    ));

    let result = rp
        .execute(
            ctx,
            json!({
                "name": "shout",
                "args": {"greeting": "hello"}
            }),
        )
        .await
        .expect("procedure should succeed");

    assert_eq!(result["status"], "ok");
    assert_eq!(result["steps_run"], 2);
    assert_eq!(result["final"], "HELLO");

    let after = store.current().await;
    assert_eq!(
        after.success_count, 2,
        "success_count should be incremented"
    );
    assert_eq!(after.failure_count, 0, "failure_count should be untouched");

    // Sanity: the intermediate step_0 result was bound through to step_1.
    // We verify this by the final value matching the upper-cased greeting.
}

#[tokio::test]
async fn procedure_e2e_failure_increments_failure_counter() {
    // Single unknown action — should trip the strict-mode gate
    let steps_json = serde_json::to_string(&vec![
        json!({"action": "unknown_tool", "args": {}, "binds": []}),
    ])
    .unwrap();
    let proc = Procedure {
        id: "proc_fail".into(),
        agent_id: "root".into(),
        ward_id: None,
        name: "broken".into(),
        description: "broken".into(),
        trigger_pattern: None,
        steps: steps_json,
        parameters: None,
        success_count: 1,
        failure_count: 0,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: "".into(),
        updated_at: "".into(),
    };
    let store = Arc::new(InMemStore::new(proc));

    let registry = Arc::new(ToolRegistry::new()); // empty: no tools registered
    let rp = RunProcedureTool::new(registry, store.clone());
    let ctx: Arc<dyn ToolContext> = Arc::new(ConcreteCtx::full_with_state(
        "root".into(),
        Some("c1".into()),
        vec![],
        Default::default(),
    ));

    let result = rp.execute(ctx, json!({"name": "broken"})).await;
    assert!(result.is_err());

    let after = store.current().await;
    assert_eq!(after.success_count, 1, "success_count should be unchanged");
    assert_eq!(
        after.failure_count, 1,
        "failure_count should be incremented"
    );
}
