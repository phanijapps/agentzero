//! Executor construction entry points. The builder, actor policy, and the
//! per-tool catalog live in their focused modules.

pub(crate) use super::builder::mcp_startup_failure_observer;
pub use super::builder::{
    build_context_capability_catalog, collect_agents_summary, collect_skills_summary,
    ExecutorBuilder,
};
pub use super::policy::RuntimeActorKind;

use agent_runtime::{BoxedAgentEngine, PreparedExecution};

/// Resolve the effective thinking flag for an agent execution.
///
/// Previously this consulted `ModelRegistry.has_capability(model, Thinking)`
/// and silently disabled thinking on models the registry didn't know about.
/// That blocked users from using `thinkingEnabled=true` against any model
/// they typed into Settings > Advanced that wasn't in the curated registry
/// — effectively gating a user-visible setting on a local allowlist.
///
/// Current behaviour: trust the user-declared flag verbatim. If the
/// provider rejects the reasoning payload, the LLM client returns an
/// error that bubbles to the UI through the normal tool_error path.
pub fn resolve_thinking_flag(user_flag: bool, _model: &str) -> bool {
    user_flag
}

/// Build the sole execution engine from prepared session inputs.
///
/// Every local entry point — root invoke, continuation, delegated children,
/// durable Research and A2A ingress — constructs here. The Rig-backed
/// engine drives unconditionally: same `LlmClient`, same actor-filtered tool
/// inventory (built-ins, MCP and skills), same shared context, and the same
/// before/after-tool hooks. There is no engine-selection environment flag
/// and no fallback; a session whose required Rig configuration cannot be
/// resolved fails explicitly.
pub fn build_execution_engine(executor: PreparedExecution) -> Result<BoxedAgentEngine, String> {
    let agent_id = executor.config().agent_id.clone();
    let Some(rig_config) = executor.rig_config.clone() else {
        return Err(format!(
            "rig_execution_config_unresolved: agent {agent_id} resolved no engine configuration"
        ));
    };
    tracing::info!(
        target: "rig_cutover",
        agent = %agent_id,
        "constructing RigAgentEngine"
    );
    Ok(Box::new(agent_runtime::rig_adapter::factory::build_engine(
        executor, rig_config,
    )))
}
