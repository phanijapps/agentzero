//! Micro-Recall — targeted memory lookups triggered by tool results.
//!
//! Detects decision-point triggers (delegation, errors, ward entry, entity mentions)
//! from tool output and executes focused memory/graph lookups, injecting findings
//! into working memory so the LLM has relevant context at the right moment.

use super::working_memory::WorkingMemory;
use agent_runtime::{
    ContextActorKind, ContextAtom, ContextPacketDelta, ContextRenderPolicy, ContextTrace,
};
use knowledge_graph::kg_trait::KnowledgeGraphStore;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::LazyLock;
use tracing::debug;
use zbot_stores_traits::MemoryFact;
use zbot_stores_traits::MemoryFactStore;

/// Regex for extracting entity candidates from text (same pattern as working_memory_middleware).
/// Matches: "quoted strings", PascalCase words, ALLCAPS (3+ chars).
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:"([^"]{2,30})"|([A-Z][a-z]+(?:[A-Z][a-z]+)+)|(\b[A-Z]{3,}\b))"#)
        .unwrap_or_else(|_| Regex::new(".^").expect("fallback regex must compile"))
});

/// Maximum number of entity triggers per tool result.
const MAX_ENTITY_TRIGGERS: usize = 3;

// ---------------------------------------------------------------------------
// Trigger enum
// ---------------------------------------------------------------------------

/// A detected decision-point that warrants a targeted memory lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicroRecallTrigger {
    /// About to delegate to a subagent — recall corrections/procedures for that agent.
    PreDelegation { agent_id: String },
    /// A tool returned an error — look for prior corrections or known fixes.
    ToolError {
        tool_name: String,
        error_msg: String,
    },
    /// Agent entered a ward (sandbox) — load ward-specific wiki/facts.
    WardEntry { ward_id: String },
    /// A new entity was mentioned that isn't already in working memory.
    EntityMention { entity_name: String },
}

// ---------------------------------------------------------------------------
// Context — holds optional repo/storage handles
// ---------------------------------------------------------------------------

/// Shared context for micro-recall handlers. All fields are optional so
/// micro-recall degrades gracefully when stores aren't available.
pub struct MicroRecallContext {
    pub memory_store: Option<Arc<dyn MemoryFactStore>>,
    pub kg_store: Option<Arc<dyn KnowledgeGraphStore>>,
    pub agent_id: String,
}

// ---------------------------------------------------------------------------
// Trigger detection (sync)
// ---------------------------------------------------------------------------

/// Parse `result` as JSON and return the first present field string from
/// `field_keys`. Returns `None` if the JSON is invalid or none of the keys
/// resolve to a string.
fn first_json_string<'a>(result: &str, field_keys: &'a [&'a str]) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(result).ok()?;
    field_keys
        .iter()
        .find_map(|k| value.get(*k).and_then(|v| v.as_str()).map(String::from))
}

/// Detect micro-recall triggers from a tool invocation result.
///
/// This is intentionally synchronous — it only inspects the tool name/result
/// strings and working memory state without doing any I/O.
pub fn detect_triggers(
    tool_name: &str,
    result: &str,
    error: Option<&str>,
    wm: &WorkingMemory,
) -> Vec<MicroRecallTrigger> {
    if let Some(err) = error {
        return vec![MicroRecallTrigger::ToolError {
            tool_name: tool_name.to_string(),
            error_msg: truncate_safe(err, 200),
        }];
    }

    let mut triggers = Vec::new();

    if tool_name == "delegate_to_agent" {
        if let Some(agent_id) = first_json_string(result, &["agent_id", "child_agent_id"]) {
            triggers.push(MicroRecallTrigger::PreDelegation { agent_id });
        }
    }

    if is_ward_tool(tool_name) {
        if let Some(ward_id) = first_json_string(result, &["ward_id"]) {
            triggers.push(MicroRecallTrigger::WardEntry { ward_id });
        }
    }

    if tool_name != "respond" {
        for name in extract_new_entities(result, wm) {
            triggers.push(MicroRecallTrigger::EntityMention { entity_name: name });
        }
    }

    triggers
}

/// Check if a tool name is ward-related.
fn is_ward_tool(tool_name: &str) -> bool {
    tool_name.starts_with("ward_")
        || tool_name == "enter_ward"
        || tool_name == "switch_ward"
        || tool_name == "create_ward"
}

// ---------------------------------------------------------------------------
// Trigger execution (async)
// ---------------------------------------------------------------------------

/// Execute a micro-recall trigger, injecting findings into working memory.
///
/// Each handler is fail-safe: errors are logged but never propagated.
pub async fn execute_micro_recall(
    wm: &mut WorkingMemory,
    trigger: &MicroRecallTrigger,
    ctx: &MicroRecallContext,
    iteration: u32,
) {
    match trigger {
        MicroRecallTrigger::PreDelegation { agent_id } => {
            handle_pre_delegation(wm, agent_id, ctx, iteration).await;
        }
        MicroRecallTrigger::ToolError {
            tool_name,
            error_msg,
        } => {
            handle_tool_error(wm, tool_name, error_msg, ctx, iteration).await;
        }
        MicroRecallTrigger::WardEntry { ward_id } => {
            handle_ward_entry(wm, ward_id, ctx, iteration).await;
        }
        MicroRecallTrigger::EntityMention { entity_name } => {
            handle_entity_mention(wm, entity_name, ctx, iteration).await;
        }
    }
    wm.add_context_packet_delta(context_packet_delta_for_trigger(trigger, ctx, iteration));
}

/// Build the structured delta for a micro-recall trigger.
pub fn context_packet_delta_for_trigger(
    trigger: &MicroRecallTrigger,
    ctx: &MicroRecallContext,
    iteration: u32,
) -> ContextPacketDelta {
    let (trigger_kind, atom_id, atom_kind, content) = match trigger {
        MicroRecallTrigger::PreDelegation { agent_id } => (
            "pre_delegation",
            format!("pre-delegation-{agent_id}"),
            "delegation_context",
            format!("Preparing delegation to {agent_id}. Recall corrections, procedures, and constraints for that child agent."),
        ),
        MicroRecallTrigger::ToolError {
            tool_name,
            error_msg } => (
            "tool_error",
            format!("tool-error-{tool_name}"),
            "tool_error_context",
            format!("{tool_name} error: {}", truncate_safe(error_msg, 240)),
        ),
        MicroRecallTrigger::WardEntry { ward_id } => (
            "ward_entry",
            format!("ward-entry-{ward_id}"),
            "ward_context",
            format!("Entered ward {ward_id}. Load ward-scoped facts, graph entities, and local operating constraints."),
        ),
        MicroRecallTrigger::EntityMention { entity_name } => (
            "entity_mention",
            format!("entity-mention-{entity_name}"),
            "entity_context",
            format!("New entity mentioned: {entity_name}. Recall graph and memory context before acting on it."),
        ) };
    build_single_atom_delta(
        &format!("micro-recall:{iteration}:{}", slugify(&atom_id)),
        &ctx.agent_id,
        actor_kind_for_agent(&ctx.agent_id),
        iteration,
        trigger_kind,
        delta_atom(
            slugify(&atom_id),
            atom_kind,
            content,
            "micro_recall",
            trigger_kind,
        ),
    )
}

/// Build the structured delta for a completed delegated-agent callback.
pub fn context_packet_delta_for_delegation_callback(
    agent_id: &str,
    result: &str,
    iteration: u32,
) -> ContextPacketDelta {
    build_single_atom_delta(
        &format!("micro-recall:{iteration}:delegation-callback-{agent_id}"),
        "root",
        ContextActorKind::Root,
        iteration,
        "delegation_callback",
        delta_atom(
            format!("delegation-callback-{}", slugify(agent_id)),
            "delegation_callback",
            format!(
                "Delegation callback from {agent_id}: {}",
                truncate_safe(result, 240)
            ),
            "delegation_callback",
            agent_id,
        ),
    )
}

fn build_single_atom_delta(
    delta_id: &str,
    agent_id: &str,
    actor_kind: ContextActorKind,
    iteration: u32,
    trigger_kind: &str,
    atom: ContextAtom,
) -> ContextPacketDelta {
    let mut source_mix = BTreeMap::new();
    source_mix.insert(atom.source.clone(), 1);
    ContextPacketDelta {
        delta_id: delta_id.to_string(),
        request_id: format!("micro-recall:{agent_id}:{iteration}"),
        agent_id: agent_id.to_string(),
        actor_kind,
        iteration,
        trigger_kind: trigger_kind.to_string(),
        atoms: vec![atom],
        dropped: Vec::new(),
        trace: ContextTrace {
            selected_count: 1,
            dropped_count: 0,
            source_mix,
        },
    }
}

fn delta_atom(
    id: impl Into<String>,
    kind: &str,
    content: String,
    source: &str,
    source_id: &str,
) -> ContextAtom {
    let token_estimate = content.chars().count().div_ceil(4).max(1) as u32;
    ContextAtom {
        id: id.into(),
        kind: kind.to_string(),
        content,
        score: 1.0,
        confidence: 1.0,
        source: source.to_string(),
        source_id: Some(source_id.to_string()),
        provenance: vec![format!("{source}:{source_id}")],
        valid_from: None,
        valid_until: None,
        visibility: vec![
            ContextActorKind::Root,
            ContextActorKind::DelegatedExecutor,
            ContextActorKind::WardAgent,
        ],
        route_hint: None,
        token_estimate,
        render_policy: ContextRenderPolicy::Summary,
    }
}

fn actor_kind_for_agent(agent_id: &str) -> ContextActorKind {
    if agent_id.starts_with("ward:") {
        ContextActorKind::WardAgent
    } else {
        ContextActorKind::Root
    }
}

fn slugify(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "unknown".to_string()
    } else {
        slug
    }
}

// ---------------------------------------------------------------------------
// Handler: PreDelegation
// ---------------------------------------------------------------------------

/// Push every correction fact in `result` into `wm` with optional filtering
/// by `needle`. Logs and swallows errors with a `failure_label` debug line.
fn push_corrections(
    wm: &mut WorkingMemory,
    result: Result<Vec<MemoryFact>, String>,
    needle: Option<&str>,
    failure_label: &str,
) {
    match result {
        Ok(facts) => {
            for fact in facts {
                let matches_needle = needle
                    .map(|n| fact.content.contains(n) || fact.key.contains(n))
                    .unwrap_or(true);
                if matches_needle {
                    wm.add_correction(&truncate_safe(&fact.content, 150));
                }
            }
        }
        Err(e) => debug!("micro-recall: {failure_label} lookup failed: {e}"),
    }
}

/// Look up corrections and procedures relevant to the target agent.
async fn handle_pre_delegation(
    wm: &mut WorkingMemory,
    agent_id: &str,
    ctx: &MicroRecallContext,
    iteration: u32,
) {
    if let Some(store) = &ctx.memory_store {
        push_corrections(
            wm,
            store
                .get_facts_by_category(agent_id, "correction", 5)
                .await
                .map_err(|e| e.to_string()),
            None,
            "pre-delegation correction",
        );
        push_corrections(
            wm,
            store
                .get_facts_by_category(&ctx.agent_id, "correction", 10)
                .await
                .map_err(|e| e.to_string()),
            Some(agent_id),
            "pre-delegation self-correction",
        );
    }

    wm.add_discovery(
        &format!("Preparing delegation to {agent_id}"),
        iteration,
        "micro-recall",
    );
}

// ---------------------------------------------------------------------------
// Handler: ToolError
// ---------------------------------------------------------------------------

/// Look for prior corrections or known fixes related to the error.
async fn handle_tool_error(
    wm: &mut WorkingMemory,
    tool_name: &str,
    error_msg: &str,
    ctx: &MicroRecallContext,
    iteration: u32,
) {
    if let Some(store) = &ctx.memory_store {
        // Search for corrections related to this tool
        match store
            .get_facts_by_category(&ctx.agent_id, "correction", 10)
            .await
        {
            Ok(facts) => {
                for fact in facts {
                    if fact.content.contains(tool_name) || fact.key.contains(tool_name) {
                        wm.add_discovery(
                            &format!(
                                "Prior fix for {tool_name}: {}",
                                truncate_safe(&fact.content, 120)
                            ),
                            iteration,
                            "micro-recall",
                        );
                    }
                }
            }
            Err(e) => {
                debug!("micro-recall: tool-error correction lookup failed: {e}");
            }
        }
    }

    // Always record the error itself
    wm.add_discovery(
        &format!("{tool_name} error: {}", truncate_safe(error_msg, 150)),
        iteration,
        "micro-recall",
    );
}

// ---------------------------------------------------------------------------
// Handler: WardEntry
// ---------------------------------------------------------------------------

/// Load ward-specific entities from the knowledge graph.
async fn handle_ward_entry(
    wm: &mut WorkingMemory,
    ward_id: &str,
    ctx: &MicroRecallContext,
    iteration: u32,
) {
    // Try to find the ward entity in the knowledge graph
    if let Some(kg) = &ctx.kg_store {
        match kg.get_entity_by_name(&ctx.agent_id, ward_id).await {
            Ok(Some(entity)) => {
                let summary = entity
                    .properties
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ward context loaded");
                wm.add_entity(
                    &entity.name,
                    Some(&format!("{:?}", entity.entity_type)),
                    &truncate_safe(summary, 100),
                    iteration,
                );
            }
            Ok(None) => {
                debug!("micro-recall: no graph entity for ward {ward_id}");
            }
            Err(e) => {
                debug!("micro-recall: ward entity lookup failed: {e}");
            }
        }
    }

    // Load ward-scoped memory facts
    if let Some(store) = &ctx.memory_store {
        match store
            .list_memory_facts_typed(Some(&ctx.agent_id), None, Some(ward_id), 5, 0)
            .await
        {
            Ok(facts) => {
                for fact in facts {
                    wm.add_entity(
                        &fact.key,
                        Some(&fact.category),
                        &truncate_safe(&fact.content, 100),
                        iteration,
                    );
                }
            }
            Err(e) => {
                debug!("micro-recall: ward facts lookup failed: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handler: EntityMention
// ---------------------------------------------------------------------------

/// Look up a newly-mentioned entity in the knowledge graph.
async fn handle_entity_mention(
    wm: &mut WorkingMemory,
    entity_name: &str,
    ctx: &MicroRecallContext,
    iteration: u32,
) {
    if let Some(kg) = &ctx.kg_store {
        match kg.get_entity_by_name(&ctx.agent_id, entity_name).await {
            Ok(Some(entity)) => {
                let summary = entity
                    .properties
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| truncate_safe(s, 100))
                    .unwrap_or_else(|| {
                        format!(
                            "{:?}, seen {} times",
                            entity.entity_type, entity.mention_count
                        )
                    });
                wm.add_entity(
                    &entity.name,
                    Some(&format!("{:?}", entity.entity_type)),
                    &summary,
                    iteration,
                );
            }
            Ok(None) => {
                debug!("micro-recall: entity '{entity_name}' not in graph");
            }
            Err(e) => {
                debug!("micro-recall: entity lookup failed for '{entity_name}': {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entity extraction helpers
// ---------------------------------------------------------------------------

/// Extract entity names from text that are NOT already in working memory.
/// Capped at [`MAX_ENTITY_TRIGGERS`].
pub fn extract_new_entities(text: &str, wm: &WorkingMemory) -> Vec<String> {
    let scan_text = if text.len() > 2000 {
        &text[..safe_byte_boundary(text, 2000)]
    } else {
        text
    };

    let mut found = Vec::new();
    for cap in ENTITY_RE.captures_iter(scan_text) {
        if found.len() >= MAX_ENTITY_TRIGGERS {
            break;
        }
        let name = cap
            .get(1)
            .or(cap.get(2))
            .or(cap.get(3))
            .map(|m| m.as_str().to_string());

        if let Some(name) = name {
            if name.len() < 3 || is_common_word(&name) {
                continue;
            }
            if wm.has_entity(&name) {
                continue;
            }
            if !found.contains(&name) {
                found.push(name);
            }
        }
    }
    found
}

/// Check if a word is too common to be a useful entity.
fn is_common_word(word: &str) -> bool {
    matches!(
        word.to_uppercase().as_str(),
        "THE"
            | "AND"
            | "FOR"
            | "NOT"
            | "THIS"
            | "THAT"
            | "WITH"
            | "FROM"
            | "HAVE"
            | "WILL"
            | "ARE"
            | "BUT"
            | "ALL"
            | "CAN"
            | "HAS"
            | "HER"
            | "WAS"
            | "ONE"
            | "OUR"
            | "OUT"
            | "YOU"
            | "HAD"
            | "HOT"
            | "HIS"
            | "GET"
            | "LET"
            | "SAY"
            | "SHE"
            | "TOO"
            | "USE"
            | "WAY"
            | "WHO"
            | "DID"
            | "ITS"
            | "SET"
            | "TRY"
            | "ASK"
            | "MEN"
            | "RUN"
            | "GOT"
            | "OLD"
            | "END"
            | "NOW"
            | "PUT"
            | "BOX"
            | "ROW"
            | "COL"
            | "KEY"
            | "MAP"
            | "JSON"
            | "HTTP"
            | "URL"
            | "API"
            | "CSS"
            | "HTML"
            | "NONE"
            | "NULL"
            | "TRUE"
            | "FALSE"
            | "SELF"
            | "TODO"
            | "NOTE"
            | "INFO"
            | "WARN"
            | "DEBUG"
    )
}

/// Truncate a string safely at a char boundary, appending "..." if truncated.
pub fn truncate_safe(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let boundary = safe_byte_boundary(s, max_len);
    format!("{}...", &s[..boundary])
}

/// Find the largest valid byte boundary <= target for a UTF-8 string.
fn safe_byte_boundary(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    // Walk back from target until we hit a char boundary
    let mut pos = target;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- detect_triggers tests ----

    #[test]
    fn test_detect_triggers_tool_error() {
        let wm = WorkingMemory::new(5000);
        let triggers = detect_triggers("shell", "", Some("command not found: xyz"), &wm);
        assert_eq!(triggers.len(), 1);
        match &triggers[0] {
            MicroRecallTrigger::ToolError {
                tool_name,
                error_msg,
            } => {
                assert_eq!(tool_name, "shell");
                assert!(error_msg.contains("command not found"));
            }
            other => panic!("expected ToolError, got {other:?}"),
        }
    }

    #[test]
    fn test_detect_triggers_delegation() {
        let wm = WorkingMemory::new(5000);
        let result = r#"{"agent_id": "research-agent", "task": "fetch stock data"}"#;
        let triggers = detect_triggers("delegate_to_agent", result, None, &wm);
        assert!(triggers
            .iter()
            .any(|t| matches!(t, MicroRecallTrigger::PreDelegation { agent_id } if agent_id == "research-agent")));
    }

    #[test]
    fn test_detect_triggers_ward_entry() {
        let wm = WorkingMemory::new(5000);
        let result = r#"{"ward_id": "stock-analysis", "status": "entered"}"#;
        let triggers = detect_triggers("enter_ward", result, None, &wm);
        assert!(triggers
            .iter()
            .any(|t| matches!(t, MicroRecallTrigger::WardEntry { ward_id } if ward_id == "stock-analysis")));
    }

    #[test]
    fn test_detect_triggers_entity_mention() {
        let wm = WorkingMemory::new(5000);
        let result = r#"Analyzing DataFrame from PandaFrame module"#;
        let triggers = detect_triggers("shell", result, None, &wm);
        assert!(triggers
            .iter()
            .any(|t| matches!(t, MicroRecallTrigger::EntityMention { entity_name } if entity_name == "DataFrame")));
    }

    #[test]
    fn test_detect_triggers_no_entity_if_already_in_wm() {
        let mut wm = WorkingMemory::new(5000);
        wm.add_entity("DataFrame", Some("class"), "pandas data structure", 1);
        let result = r#"Analyzing DataFrame from PandaFrame module"#;
        let triggers = detect_triggers("shell", result, None, &wm);
        // DataFrame is already tracked, so no EntityMention for it
        assert!(!triggers
            .iter()
            .any(|t| matches!(t, MicroRecallTrigger::EntityMention { entity_name } if entity_name == "DataFrame")));
    }

    #[test]
    fn test_detect_triggers_respond_no_entities() {
        let wm = WorkingMemory::new(5000);
        let result = r#"Here is your DataFrame analysis for PandaFrame"#;
        let triggers = detect_triggers("respond", result, None, &wm);
        assert!(!triggers
            .iter()
            .any(|t| matches!(t, MicroRecallTrigger::EntityMention { .. })));
    }

    // ---- execute_micro_recall tests ----

    #[tokio::test]
    async fn test_execute_tool_error_adds_discovery() {
        let mut wm = WorkingMemory::new(5000);
        let ctx = MicroRecallContext {
            memory_store: None,
            kg_store: None,
            agent_id: "root".to_string(),
        };
        let trigger = MicroRecallTrigger::ToolError {
            tool_name: "shell".to_string(),
            error_msg: "permission denied".to_string(),
        };
        execute_micro_recall(&mut wm, &trigger, &ctx, 3).await;
        let output = wm.format_for_prompt();
        assert!(output.contains("shell error: permission denied"));
        assert!(wm
            .context_packet_deltas()
            .iter()
            .any(|delta| delta.trigger_kind == "tool_error"));
    }

    #[tokio::test]
    async fn test_execute_ward_entry_no_repos() {
        // When stores are None, handler should not panic — just return gracefully
        let mut wm = WorkingMemory::new(5000);
        let ctx = MicroRecallContext {
            memory_store: None,
            kg_store: None,
            agent_id: "root".to_string(),
        };
        let trigger = MicroRecallTrigger::WardEntry {
            ward_id: "test-ward".to_string(),
        };
        execute_micro_recall(&mut wm, &trigger, &ctx, 1).await;
        // Should not panic; WM may or may not have content (no repos = no data)
        assert!(wm
            .context_packet_deltas()
            .iter()
            .any(|delta| delta.trigger_kind == "ward_entry"));
    }

    #[tokio::test]
    async fn micro_recall_packet_deltas_cover_required_triggers() {
        let ctx = MicroRecallContext {
            memory_store: None,
            kg_store: None,
            agent_id: "root".to_string(),
        };
        let triggers = [
            MicroRecallTrigger::PreDelegation {
                agent_id: "research-agent".to_string(),
            },
            MicroRecallTrigger::ToolError {
                tool_name: "shell".to_string(),
                error_msg: "permission denied".to_string(),
            },
            MicroRecallTrigger::WardEntry {
                ward_id: "finance".to_string(),
            },
            MicroRecallTrigger::EntityMention {
                entity_name: "AAPL".to_string(),
            },
        ];

        let mut wm = WorkingMemory::new(5000);
        for (iteration, trigger) in triggers.iter().enumerate() {
            execute_micro_recall(&mut wm, trigger, &ctx, iteration as u32).await;
        }

        let kinds = wm
            .context_packet_deltas()
            .iter()
            .map(|delta| delta.trigger_kind.as_str())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"pre_delegation"));
        assert!(kinds.contains(&"tool_error"));
        assert!(kinds.contains(&"ward_entry"));
        assert!(kinds.contains(&"entity_mention"));
        for delta in wm.context_packet_deltas() {
            assert_eq!(delta.trace.selected_count, 1);
            assert_eq!(delta.trace.dropped_count, 0);
            assert!(!delta.atoms[0].provenance.is_empty());
            assert!(delta.atoms[0].visibility.contains(&ContextActorKind::Root));
        }
    }

    #[test]
    fn delegation_callback_delta_preserves_provenance() {
        let delta = context_packet_delta_for_delegation_callback(
            "analyst-agent",
            "Found 5 useful facts",
            4,
        );

        assert_eq!(delta.trigger_kind, "delegation_callback");
        assert_eq!(delta.iteration, 4);
        assert_eq!(delta.trace.selected_count, 1);
        assert!(delta.atoms[0].content.contains("analyst-agent"));
        assert_eq!(
            delta.atoms[0].provenance,
            vec!["delegation_callback:analyst-agent".to_string()]
        );
    }

    // ---- extract_new_entities tests ----

    #[test]
    fn test_extract_new_entities_caps_at_max() {
        let wm = WorkingMemory::new(5000);
        // 5 PascalCase entities, only 3 should be returned
        let text = "Found DataFrame and PandaFrame and MultiIndex and StockData and MarketCap";
        let entities = extract_new_entities(text, &wm);
        assert!(entities.len() <= MAX_ENTITY_TRIGGERS);
    }

    // ---- truncate_safe tests ----

    #[test]
    fn test_truncate_safe_multibyte() {
        // "cafe\u{0301}" = "café" — the accent is a combining char at byte 5
        let s = "caf\u{00e9} latte is great";
        let result = truncate_safe(s, 5);
        // Should not panic and should produce valid UTF-8
        assert!(result.ends_with("..."));
        // The truncated portion must be valid UTF-8
        let prefix = result.trim_end_matches("...");
        assert!(prefix.len() <= 5);
    }

    #[test]
    fn test_truncate_safe_ascii() {
        let s = "hello world";
        assert_eq!(truncate_safe(s, 5), "hello...");
        assert_eq!(truncate_safe(s, 50), "hello world");
    }
}
