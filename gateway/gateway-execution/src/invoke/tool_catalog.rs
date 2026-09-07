//! Tool catalog — every per-tool policy attribute as data, not match arms.
//!
//! One row per known tool name. Unknown names derive conservative defaults
//! (empty capabilities → derived side-effects/risk/audit). Lookups return
//! owned/derived values exactly matching the retired match-arm functions.

use agent_runtime::{ContextCostHint, ContextLatencyHint, ContextRiskLevel, ContextSideEffects};

use super::policy::ToolCapability;

/// Static per-tool policy row.
pub(crate) struct ToolSpec {
    pub name: &'static str,
    pub caps: &'static [ToolCapability],
    /// Explicit side-effect override; `None` → derive from caps.
    pub side_effects: Option<ContextSideEffects>,
    /// Explicit risk override; `None` → derive from caps.
    pub risk: Option<ContextRiskLevel>,
    pub cost: ContextCostHint,
    /// Explicit latency override; `None` → derive from caps.
    pub latency: Option<ContextLatencyHint>,
    pub token_hint: u32,
    pub owner: &'static str,
    /// Explicit audit override; `None` → derive from caps.
    pub audit: Option<&'static str>,
    /// Hidden from the model's tool schema (alias or context-resource split).
    pub hidden: bool,
    pub visibility_policy: &'static str,
    pub split_target: Option<&'static str>,
}

use ToolCapability as C;

/// The one table. Adding a tool = adding a row here.
#[rustfmt::skip]
pub(crate) const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec { name: "delegate_to_agent",  caps: &[C::AgentDelegate],  side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-runtime",     audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "respond",            caps: &[C::Respond],        side_effects: Some(ContextSideEffects::WriteExternal), risk: None, cost: ContextCostHint::Cheap, latency: None, token_hint: 200, owner: "agent-runtime", audit: None, hidden: false, visibility_policy: "default_visible", split_target: None },
    ToolSpec { name: "run_procedure",      caps: &[C::ProcedureRun],   side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-runtime",     audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "delegate_to_zbot",   caps: &[C::PeerDelegate],   side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "list_zbots",         caps: &[C::PeerDelegate],   side_effects: Some(ContextSideEffects::ReadExternal), risk: Some(ContextRiskLevel::Low), cost: ContextCostHint::Cheap, latency: None, token_hint: 200, owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible", split_target: None },
    ToolSpec { name: "handoff_to_agent",   caps: &[C::AgentControl],   side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "kill_agent",         caps: &[C::AgentControl],   side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "list_session_agents",caps: &[C::AgentControl],   side_effects: Some(ContextSideEffects::ReadExternal), risk: Some(ContextRiskLevel::Low), cost: ContextCostHint::Cheap, latency: None, token_hint: 200, owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible", split_target: None },
    ToolSpec { name: "message_agent",      caps: &[C::AgentControl],   side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "steer_agent",        caps: &[C::AgentControl],   side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "wait_agent",         caps: &[C::AgentControl],   side_effects: Some(ContextSideEffects::ReadExternal), risk: Some(ContextRiskLevel::Low), cost: ContextCostHint::Cheap, latency: Some(ContextLatencyHint::Background), token_hint: 120, owner: "gateway-execution", audit: Some("join_audit"), hidden: true, visibility_policy: "visible_when_parallel_children_active", split_target: Some("action:parallel_join") },
    ToolSpec { name: "reply_to_agent",     caps: &[C::AgentReply],     side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "gateway-execution", audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "present_surface",    caps: &[C::SurfacePresent], side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 300,  owner: "gateway-execution", audit: Some("presentation_audit"), hidden: false, visibility_policy: "default_visible_automatic_presentation", split_target: None },
    ToolSpec { name: "shell",              caps: &[C::Shell],          side_effects: Some(ContextSideEffects::Execute), risk: Some(ContextRiskLevel::High), cost: ContextCostHint::Cheap, latency: None, token_hint: 400, owner: "agent-tools", audit: Some("execution_audit"), hidden: false, visibility_policy: "default_visible_action_tool", split_target: Some("actions:shell_execute; resources:command_result_handles") },
    ToolSpec { name: "read",               caps: &[C::FileRead],       side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 400,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "glob",               caps: &[C::FileRead],       side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "edit",               caps: &[C::FileWrite],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: true,  visibility_policy: "legacy_alias_hidden", split_target: None },
    ToolSpec { name: "edit_file",          caps: &[C::FileWrite],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "write",              caps: &[C::FileWrite],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: true,  visibility_policy: "legacy_alias_hidden", split_target: None },
    ToolSpec { name: "write_file",         caps: &[C::FileWrite],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "goal",               caps: &[C::GoalWrite],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "graph_query",        caps: &[C::GraphRead],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 800,  owner: "agent-tools",        audit: None, hidden: true,  visibility_policy: "hidden_from_model_use_context_resources", split_target: Some("resources:context_graph_retrieval") },
    ToolSpec { name: "ingest",             caps: &[C::IngestWrite],    side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
    ToolSpec { name: "load_skill",         caps: &[C::SkillLoad],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 1200, owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible_bounded_packet", split_target: Some("resources:skill_packet/skill_section_handles") },
    ToolSpec { name: "memory",             caps: &[C::MemoryRead, C::MemoryWrite], side_effects: None, risk: None, cost: ContextCostHint::Cheap, latency: None, token_hint: 800, owner: "agent-tools", audit: None, hidden: true, visibility_policy: "hidden_from_model_use_context_resources", split_target: Some("action:memory_write; resources:memory_recall/context_atoms") },
    ToolSpec { name: "recall",             caps: &[C::MemoryRead],     side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 800,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible_unified_recall_exception", split_target: Some("resources:memory_recall/context_atoms") },
    ToolSpec { name: "memory_write",       caps: &[C::MemoryWrite],    side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible_memory_write_action", split_target: Some("action:memory_write") },
    ToolSpec { name: "multimodal_analyze", caps: &[C::MultimodalAnalyze], side_effects: None, risk: None, cost: ContextCostHint::Moderate, latency: Some(ContextLatencyHint::Slow), token_hint: 200, owner: "agent-tools", audit: None, hidden: false, visibility_policy: "default_visible", split_target: None },
    ToolSpec { name: "query_resource",     caps: &[C::ConnectorQuery], side_effects: None, risk: None, cost: ContextCostHint::Moderate, latency: Some(ContextLatencyHint::Slow), token_hint: 800, owner: "agent-tools", audit: None, hidden: true, visibility_policy: "hidden_from_model_use_connector_split", split_target: Some("action:connector_invoke; resources:connector_resource") },
    ToolSpec { name: "connector_invoke",   caps: &[C::ConnectorInvoke], side_effects: None, risk: None, cost: ContextCostHint::Moderate, latency: Some(ContextLatencyHint::Slow), token_hint: 300, owner: "agent-tools", audit: None, hidden: false, visibility_policy: "default_visible_connector_invoke_action", split_target: Some("action:connector_invoke") },
    ToolSpec { name: "connector_resource", caps: &[C::ConnectorResourceRead], side_effects: None, risk: None, cost: ContextCostHint::Moderate, latency: Some(ContextLatencyHint::Slow), token_hint: 800, owner: "agent-tools", audit: None, hidden: false, visibility_policy: "default_visible_connector_resource_read", split_target: Some("resources:connector_resource") },
    ToolSpec { name: "ward",               caps: &[C::WardRead, C::WardWrite], side_effects: None, risk: None, cost: ContextCostHint::Cheap, latency: None, token_hint: 200, owner: "agent-tools", audit: None, hidden: false, visibility_policy: "default_visible_action_tool", split_target: Some("actions:ward_lifecycle; resources:ward_context") },
    ToolSpec { name: "update_plan",        caps: &[C::PlanWrite],      side_effects: None, risk: None, cost: ContextCostHint::Cheap,     latency: None,                              token_hint: 200,  owner: "agent-tools",        audit: None, hidden: false, visibility_policy: "default_visible",  split_target: None },
];

/// Look up a tool's spec row. Unknown names → `None` (callers derive).
pub(crate) fn spec(name: &str) -> Option<&'static ToolSpec> {
    TOOL_SPECS.iter().find(|row| row.name == name)
}

// ---------------------------------------------------------------------------
// Lookups — behavior-identical to the retired per-name match arms.
// ---------------------------------------------------------------------------

pub(crate) fn capabilities(name: &str) -> Vec<ToolCapability> {
    match spec(name) {
        Some(row) => row.caps.to_vec(),
        None => Vec::new(),
    }
}

pub(crate) fn side_effects(name: &str) -> ContextSideEffects {
    if let Some(row) = spec(name) {
        if let Some(explicit) = row.side_effects {
            return explicit;
        }
        return derive_side_effects(row.caps);
    }
    ContextSideEffects::None
}

fn derive_side_effects(caps: &[ToolCapability]) -> ContextSideEffects {
    if caps.contains(&C::Shell) {
        return ContextSideEffects::Execute;
    }
    if caps.contains(&C::ConnectorInvoke) || caps.contains(&C::Respond) {
        return ContextSideEffects::WriteExternal;
    }
    if caps.iter().any(|capability| {
        matches!(
            capability,
            C::AgentControl
                | C::AgentDelegate
                | C::PeerDelegate
                | C::AgentReply
                | C::FileWrite
                | C::GoalWrite
                | C::IngestWrite
                | C::MemoryWrite
                | C::PlanWrite
                | C::SurfacePresent
                | C::WardWrite
        )
    }) {
        return ContextSideEffects::WriteLocal;
    }
    if caps.is_empty() {
        ContextSideEffects::None
    } else {
        ContextSideEffects::ReadExternal
    }
}

pub(crate) fn risk_level(name: &str) -> ContextRiskLevel {
    if let Some(row) = spec(name) {
        if let Some(explicit) = row.risk {
            return explicit;
        }
        return derive_risk(row.caps);
    }
    ContextRiskLevel::Low
}

fn derive_risk(caps: &[ToolCapability]) -> ContextRiskLevel {
    if caps.contains(&C::Shell) {
        return ContextRiskLevel::High;
    }
    if caps.contains(&C::ConnectorInvoke) {
        return ContextRiskLevel::Moderate;
    }
    if caps.iter().any(|capability| {
        matches!(
            capability,
            C::AgentControl
                | C::AgentDelegate
                | C::PeerDelegate
                | C::FileWrite
                | C::IngestWrite
                | C::WardWrite
        )
    }) {
        return ContextRiskLevel::Moderate;
    }
    ContextRiskLevel::Low
}

pub(crate) fn cost_hint(name: &str) -> ContextCostHint {
    match spec(name) {
        Some(row) => row.cost,
        None => ContextCostHint::Cheap,
    }
}

pub(crate) fn latency_hint(name: &str) -> ContextLatencyHint {
    if let Some(row) = spec(name) {
        if let Some(explicit) = row.latency {
            return explicit;
        }
        if row.caps.iter().any(|capability| {
            matches!(
                capability,
                C::ConnectorQuery
                    | C::ConnectorResourceRead
                    | C::ConnectorInvoke
                    | C::MultimodalAnalyze
            )
        }) {
            return ContextLatencyHint::Slow;
        }
        return ContextLatencyHint::Local;
    }
    ContextLatencyHint::Local
}

pub(crate) fn token_hint(name: &str) -> Option<u32> {
    match spec(name) {
        Some(row) => Some(row.token_hint),
        // The retired fn returned Some(200) for every unknown name.
        None => Some(200),
    }
}

pub(crate) fn owner_crate(name: &str) -> &'static str {
    match spec(name) {
        Some(row) => row.owner,
        None => "agent-tools",
    }
}

pub(crate) fn audit_policy(name: &str) -> &'static str {
    if let Some(row) = spec(name) {
        if let Some(explicit) = row.audit {
            return explicit;
        }
        if row.caps.contains(&C::Shell) {
            return "execution_audit";
        }
        return match derive_side_effects(row.caps) {
            ContextSideEffects::None | ContextSideEffects::ReadExternal => "read_audit",
            _ => "mutation_audit",
        };
    }
    // Unknown names had empty caps → side_effects::None → read_audit.
    "read_audit"
}

pub(crate) fn default_visible(name: &str) -> bool {
    match spec(name) {
        Some(row) => !row.hidden,
        None => true,
    }
}

pub(crate) fn visibility_policy(name: &str) -> &'static str {
    match spec(name) {
        Some(row) => row.visibility_policy,
        None => "default_visible",
    }
}

pub(crate) fn split_target(name: &str) -> Option<&'static str> {
    match spec(name) {
        Some(row) => row.split_target,
        None => None,
    }
}

/// Human display name: snake_case → Title Case.
pub(crate) fn display_name(tool_name: &str) -> String {
    tool_name
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tools hidden from the model's tool schema regardless of actor.
pub(crate) const MODEL_HIDDEN_TOOLS: &[&str] = &["memory", "graph_query", "query_resource"];

#[cfg(test)]
mod tests {
    use super::*;

    /// The table rows are the characterization: each known tool's caps,
    /// owner, and audit classification are pinned by the catalog tests in
    /// builder.rs. Here we pin the derivation edges the retired match arms
    /// encoded: overrides win, fallbacks derive from caps, unknown names
    /// get conservative defaults.
    #[test]
    fn overrides_win_and_unknowns_derive() {
        // Explicit override beats capability derivation.
        assert_eq!(side_effects("wait_agent"), ContextSideEffects::ReadExternal);
        assert_eq!(risk_level("wait_agent"), ContextRiskLevel::Low);
        assert_eq!(latency_hint("wait_agent"), ContextLatencyHint::Background);
        // Derived from caps.
        assert_eq!(side_effects("shell"), ContextSideEffects::Execute);
        assert_eq!(risk_level("shell"), ContextRiskLevel::High);
        assert_eq!(audit_policy("shell"), "execution_audit");
        assert_eq!(audit_policy("read"), "read_audit");
        assert_eq!(audit_policy("write_file"), "mutation_audit");
        // Unknown names: conservative defaults, Some(200) tokens.
        assert!(capabilities("nonexistent_tool").is_empty());
        assert_eq!(side_effects("nonexistent_tool"), ContextSideEffects::None);
        assert_eq!(risk_level("nonexistent_tool"), ContextRiskLevel::Low);
        assert_eq!(token_hint("nonexistent_tool"), Some(200));
        assert_eq!(owner_crate("nonexistent_tool"), "agent-tools");
        assert_eq!(audit_policy("nonexistent_tool"), "read_audit");
        assert!(default_visible("nonexistent_tool"));
        assert_eq!(visibility_policy("nonexistent_tool"), "default_visible");
        assert!(split_target("nonexistent_tool").is_none());
    }

    #[test]
    fn token_hints_match_the_retired_match_arms() {
        assert_eq!(token_hint("load_skill"), Some(1200));
        assert_eq!(token_hint("memory"), Some(800));
        assert_eq!(token_hint("recall"), Some(800));
        assert_eq!(token_hint("graph_query"), Some(800));
        assert_eq!(token_hint("query_resource"), Some(800));
        assert_eq!(token_hint("connector_resource"), Some(800));
        assert_eq!(token_hint("connector_invoke"), Some(300));
        assert_eq!(token_hint("shell"), Some(400));
        assert_eq!(token_hint("read"), Some(400));
        assert_eq!(token_hint("wait_agent"), Some(120));
        assert_eq!(token_hint("present_surface"), Some(300));
        assert_eq!(token_hint("delegate_to_agent"), Some(200));
    }

    #[test]
    fn owner_crate_split_matches_the_retired_match_arms() {
        assert_eq!(owner_crate("delegate_to_agent"), "agent-runtime");
        assert_eq!(owner_crate("respond"), "agent-runtime");
        assert_eq!(owner_crate("run_procedure"), "agent-runtime");
        assert_eq!(owner_crate("delegate_to_zbot"), "gateway-execution");
        assert_eq!(owner_crate("wait_agent"), "gateway-execution");
        assert_eq!(owner_crate("present_surface"), "gateway-execution");
        assert_eq!(owner_crate("shell"), "agent-tools");
    }

    #[test]
    fn hidden_aliases_and_context_splits() {
        // Hidden from the model: legacy aliases + context-resource split tools.
        for name in [
            "edit",
            "write",
            "wait_agent",
            "memory",
            "graph_query",
            "query_resource",
        ] {
            assert!(!default_visible(name), "{name} must be hidden");
        }
        // Everything in MODEL_HIDDEN_TOOLS is also default_visible=false.
        for name in MODEL_HIDDEN_TOOLS {
            assert!(!default_visible(name));
        }
        assert!(default_visible("shell"));
        assert!(default_visible("read"));
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name("shell"), "Shell");
        assert_eq!(display_name("delegate_to_agent"), "Delegate To Agent");
        assert_eq!(display_name("wait_agent"), "Wait Agent");
    }
}
