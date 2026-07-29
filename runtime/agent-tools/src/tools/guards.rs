// ============================================================================
// TOOL GUARDS
// Shared guard functions that redirect tools when preconditions aren't met.
// ============================================================================

use agent_primitives::ToolContext;
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
