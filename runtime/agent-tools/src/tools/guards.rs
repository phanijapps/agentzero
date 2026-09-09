// ============================================================================
// TOOL GUARDS
// Shared guard functions that redirect tools when preconditions aren't met.
// ============================================================================

use agent_primitives::ToolContext;
use serde::{Deserialize, Serialize};

/// Root-context key used to require the planner before cold graph work starts.
pub const PLANNING_GATE_STATE: &str = "app:planning_gate";

const PLANNING_GATE_CLAIM: &str = "app:planning_gate_claim";

/// Invocation-local state bridging graph intent analysis and ward entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningGate {
    pub task: String,
    #[serde(default)]
    pub phase: PlanningGatePhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanningGatePhase {
    #[default]
    AwaitingWard,
    PlannerStarted,
}

impl PlanningGate {
    #[must_use]
    pub fn awaiting_ward(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            phase: PlanningGatePhase::AwaitingWard,
        }
    }
}

#[must_use]
pub fn active_planning_gate(ctx: &dyn ToolContext) -> Option<PlanningGate> {
    let is_delegated = ctx
        .get_state("app:is_delegated")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if is_delegated {
        return None;
    }

    let raw = ctx.get_state(PLANNING_GATE_STATE)?;
    let gate: PlanningGate = serde_json::from_value(raw).ok()?;
    (!gate.task.trim().is_empty()).then_some(gate)
}

#[must_use]
pub fn planning_gate_awaits_ward(ctx: &dyn ToolContext) -> bool {
    matches!(
        active_planning_gate(ctx).as_ref().map(|gate| gate.phase),
        Some(PlanningGatePhase::AwaitingWard)
    )
}

#[must_use]
pub fn planning_gate_blocks_tool(
    ctx: &dyn ToolContext,
    tool_name: &str,
    args: &serde_json::Value,
) -> bool {
    if !planning_gate_awaits_ward(ctx) {
        return false;
    }
    if tool_name != "ward" {
        return true;
    }

    !matches!(
        args.get("action").and_then(serde_json::Value::as_str),
        Some("create" | "use" | "list" | "info")
    )
}

pub fn start_planning_after_ward(ctx: &dyn ToolContext, ward_id: &str) -> Option<String> {
    let mut gate = active_planning_gate(ctx)?;
    if gate.phase != PlanningGatePhase::AwaitingWard || !ctx.try_claim(PLANNING_GATE_CLAIM) {
        return None;
    }

    gate.phase = PlanningGatePhase::PlannerStarted;
    ctx.set_state(
        PLANNING_GATE_STATE.to_string(),
        serde_json::to_value(&gate).ok()?,
    );

    Some(format!(
        "{}\n\nActive ward: `{ward_id}`. This is the authoritative workspace selected by the root; write the plan and steps for this ward.",
        gate.task
    ))
}
/// Check if the given `specs/` directory holds any unfilled placeholder
/// spec — a file containing the literal text `"Status: placeholder"`.
///
/// Single source of truth for the placeholder check. Pure file-IO (takes a
/// concrete path), so it's also callable from non-tool contexts like
/// the bootstrap path that doesn't have a `ToolContext` yet.
pub fn specs_dir_has_placeholders(specs_dir: &std::path::Path) -> bool {
    if !specs_dir.exists() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(specs_dir) else {
        return false;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        if entry.path().is_dir() && dir_has_placeholder_spec(&entry.path()) {
            return true;
        }
    }
    false
}

/// Check if the active ward has unfilled placeholder specs.
///
/// Returns `true` when the root agent (not a delegated subagent) has a ward
/// whose `specs/` folder contains files with `Status: placeholder`. This
/// signals that the planning pipeline hasn't been completed yet and shortcut
/// tools such as `load_skill` and `update_plan` should redirect.
pub(crate) fn has_placeholder_specs(ctx: &dyn ToolContext) -> bool {
    // Only check for root agents (not delegated subagents)
    let is_delegated = ctx
        .get_state("app:is_delegated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_delegated {
        return false; // Subagents can use all tools freely
    }

    // Check ward_id
    let ward_id = match ctx
        .get_state("ward_id")
        .and_then(|v| v.as_str().map(String::from))
    {
        Some(id) if id != "scratch" => id,
        _ => return false,
    };

    // Check for placeholder specs in the ward
    let specs_dir = dirs::document_dir()
        .or_else(dirs::home_dir)
        .map(|d| d.join("zbot").join("wards").join(&ward_id).join("specs"));

    let Some(specs_dir) = specs_dir else {
        return false;
    };
    specs_dir_has_placeholders(&specs_dir)
}

fn dir_has_placeholder_spec(dir: &std::path::Path) -> bool {
    let Ok(files) = std::fs::read_dir(dir) else {
        return false;
    };
    for file in files.filter_map(|f| f.ok()) {
        if let Ok(content) = std::fs::read_to_string(file.path())
            && content.contains("Status: placeholder")
        {
            return true;
        }
    }
    false
}

// The AGENTS.md write gate (reject_agents_md_from_subagent /
// check_agents_md_write_gate) was removed in the four-agent redesign:
// solution-agent owns architecture + AGENTS.md authoring, and it runs as a
// delegated subagent. Blocking subagent writes to AGENTS.md would block
// solution-agent's core responsibility. Root is no longer the exclusive
// writer of ward doctrine; any agent the plan assigns Step 0 to is.

/// Canonical placeholder-specs redirect envelope (ward-slim P5).
///
/// One core message for every enforcement site (delegate, load_skill,
/// update_plan) — the site passes only its action-specific tail. The same
/// unification the cold-graph redirect got in P2; three drifted copies of
/// this text were the divergence it replaces.
#[must_use]
pub fn placeholder_specs_redirect(instead: &str) -> serde_json::Value {
    redirect_envelope(&format!(
        "This ward has placeholder specs — planning is not finished. {instead}"
    ))
}

/// The one redirect envelope shape shared by every guard helper.
#[must_use]
pub fn redirect_envelope(message: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "redirect",
        "message": message
    })
}

/// Canonical cold-graph redirect envelope.
///
/// Single source for every enforcement site (rig_adapter MCP dispatch,
/// builtin executor dispatch). Both sites must render exactly this — a
/// second literal copy is how the two drifted messages happened.
#[must_use]
pub fn cold_graph_redirect() -> serde_json::Value {
    redirect_envelope(
        "This is cold graph work. First call ward(action: \"create\" or \"use\") to establish the workspace. That transition starts planner-agent automatically; do not call MCP tools or other tools yet.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_specs_redirect_core_is_canonical() {
        let envelope = placeholder_specs_redirect("Do X instead.");
        assert_eq!(envelope["status"], "redirect");
        let message = envelope["message"].as_str().expect("message is a string");
        assert!(
            message.starts_with("This ward has placeholder specs — planning is not finished."),
            "core must be identical at every site: {message}"
        );
        assert!(message.ends_with("Do X instead."));
    }

    #[test]
    fn cold_graph_redirect_envelope_is_canonical() {
        let envelope = cold_graph_redirect();
        assert_eq!(envelope["status"], "redirect");
        let message = envelope["message"].as_str().expect("message is a string");
        assert!(message.contains("ward(action: \"create\" or \"use\")"));
        assert!(message.ends_with("do not call MCP tools or other tools yet."));
    }
}
