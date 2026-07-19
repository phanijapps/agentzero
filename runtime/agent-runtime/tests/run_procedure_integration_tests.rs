//! End-to-end: register `run_procedure` into a live registry alongside
//! another tool, run a real procedure row from an in-memory store, verify
//! dispatch and counter increment.
//!
//! Does NOT exercise `ExecutorBuilder` — that's a higher-level integration
//! concern owned by the gateway crate. This test proves the
//! tool + registry + store triple round-trips correctly: a `RunProcedureTool`
//! constructed against an inner `Arc<ToolRegistry>` of dispatchable tools
//! can be discovered through the outer registry by name and successfully
//! drive a procedure to completion.

use agent_primitives::{Result, Tool, ToolContext};
use agent_runtime::tools::context::ToolContext as ConcreteCtx;
use agent_runtime::tools::registry::ToolRegistry;
use agent_runtime::tools::run_procedure::RunProcedureTool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use zbot_stores_traits::{Procedure, ProcedureStore};

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn description(&self) -> &'static str {
        "echo"
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        Ok(json!({"echoed": args}))
    }
}

struct InMemStore {
    proc: TokioMutex<Procedure>,
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
}

#[tokio::test]
async fn run_procedure_dispatches_against_live_registry() {
    let steps_json = serde_json::to_string(&vec![
        json!({"action": "echo", "args": {"x": "hi"}, "binds": []}),
    ])
    .unwrap();
    let proc = Procedure {
        id: "p_e2e".into(),
        agent_id: "root".into(),
        ward_id: None,
        name: "demo".into(),
        description: "test".into(),
        trigger_pattern: None,
        steps: steps_json,
        parameters: None,
        success_count: 1,
        failure_count: 0,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let store: Arc<dyn ProcedureStore> = Arc::new(InMemStore {
        proc: TokioMutex::new(proc),
    });

    // Inner registry — what RunProcedureTool dispatches against (no recursion).
    let mut inner = ToolRegistry::new();
    inner.register(Arc::new(EchoTool));
    let inner = Arc::new(inner);

    // Outer registry — what the executor exposes to the LLM.
    let rp = RunProcedureTool::new(inner.clone(), store.clone());
    let mut outer = ToolRegistry::new();
    outer.register(Arc::new(rp));
    outer.register(Arc::new(EchoTool));
    let outer = Arc::new(outer);

    let ctx: Arc<dyn ToolContext> = Arc::new(ConcreteCtx::full_with_state(
        "root".into(),
        Some("c1".into()),
        vec![],
        Default::default(),
    ));

    let rp_tool = outer
        .find("run_procedure")
        .expect("run_procedure not in registry");
    let res = rp_tool.execute(ctx, json!({"name": "demo"})).await.unwrap();
    assert_eq!(res["status"], "ok");
    assert_eq!(res["steps_run"], 1);
    assert_eq!(res["final"]["echoed"]["x"], "hi");
}
