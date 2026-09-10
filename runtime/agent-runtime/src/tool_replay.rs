//! Shared replay interception before any live tool side effect.
use crate::tools::ToolContext;
use agent_tools::replay::LookupOutcome;

pub(crate) fn intercept(context: &ToolContext, tool_name: &str) -> Option<String> {
    let store = agent_tools::replay::global_store()?;
    let mut store = store
        .lock()
        .expect("tool replay store poisoned; live execution denied");
    let execution_id = agent_primitives::ReadonlyContext::invocation_id(context);
    match store.lookup(execution_id, tool_name) {
        LookupOutcome::Hit(result) => Some(result),
        LookupOutcome::MissLenient => None,
        LookupOutcome::Drift {
            expected_tool,
            got_tool } => panic!(
            "[tool-replay] drift on exec {execution_id}: expected '{expected_tool}' got '{got_tool}'"
        ),
        LookupOutcome::MissStrict {
            exec_id,
            tool_index } => panic!("[tool-replay] strict miss on exec {exec_id} tool_index {tool_index}") }
}
