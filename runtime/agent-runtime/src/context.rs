//! Context capability catalog and context packet contracts.
//!
//! These types are pure data contracts. They do not enforce actor policy,
//! execute tools, or reach into memory stores; later gateway/execution layers
//! build these values from the existing policy and recall surfaces.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Actor class used by capability and context visibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextActorKind {
    /// Root agent.
    Root,
    /// Delegated executor agent.
    DelegatedExecutor,
    /// Delegated reviewer agent.
    DelegatedReviewer,
    /// Ward-scoped agent.
    WardAgent,
}

/// Actor-filtered catalog of actions, resources, and context providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCapabilityCatalog {
    /// Contract version.
    pub version: String,
    /// Actor the catalog was built for.
    pub actor_kind: ContextActorKind,
    /// Optional session id associated with this catalog snapshot.
    pub session_id: Option<String>,
    /// Optional agent id associated with this catalog snapshot.
    pub agent_id: Option<String>,
    /// Actor-visible capabilities.
    pub capabilities: Vec<ContextCapability>,
}

impl ContextCapabilityCatalog {
    /// Small root catalog example used by schema and serialization tests.
    pub fn example_root() -> Self {
        Self {
            version: "2026-07-06".to_string(),
            actor_kind: ContextActorKind::Root,
            session_id: Some("sess-example".to_string()),
            agent_id: Some("root".to_string()),
            capabilities: vec![
                ContextCapability {
                    id: "shell".to_string(),
                    kind: ContextCapabilityKind::Tool,
                    display_name: "Shell".to_string(),
                    description: "Run a local command through the execution sandbox.".to_string(),
                    actor_policy: vec![ContextActorKind::Root, ContextActorKind::DelegatedExecutor],
                    risk_level: ContextRiskLevel::High,
                    side_effects: ContextSideEffects::Execute,
                    input_schema: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "cmd": { "type": "string" }
                        },
                        "required": ["cmd"]
                    })),
                    output_schema: None,
                    resource_uri_template: None,
                    cost_hint: Some(ContextCostHint::Cheap),
                    latency_hint: Some(ContextLatencyHint::Local),
                    token_hint: Some(300),
                    health: ContextCapabilityHealth::Available,
                    owner_crate: Some("agent-tools".to_string()),
                    audit_policy: Some("log_invocation".to_string()),
                    default_visible: true,
                    visibility_policy: "default_visible".to_string(),
                    split_target: Some("resource:file_read; action:shell_execute".to_string()),
                },
                ContextCapability {
                    id: "wait_agent".to_string(),
                    kind: ContextCapabilityKind::Tool,
                    display_name: "Wait Agent".to_string(),
                    description: "Join parallel child-agent executions.".to_string(),
                    actor_policy: vec![ContextActorKind::Root],
                    risk_level: ContextRiskLevel::Low,
                    side_effects: ContextSideEffects::ReadExternal,
                    input_schema: None,
                    output_schema: None,
                    resource_uri_template: None,
                    cost_hint: Some(ContextCostHint::Free),
                    latency_hint: Some(ContextLatencyHint::Background),
                    token_hint: Some(80),
                    health: ContextCapabilityHealth::Available,
                    owner_crate: Some("gateway-execution".to_string()),
                    audit_policy: Some("log_join".to_string()),
                    default_visible: false,
                    visibility_policy: "visible_when_parallel_children_active".to_string(),
                    split_target: Some("action:parallel_join".to_string()),
                },
            ],
        }
    }
}

/// One actor-visible capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCapability {
    /// Stable capability id.
    pub id: String,
    /// Capability kind.
    pub kind: ContextCapabilityKind,
    /// Human-readable name.
    pub display_name: String,
    /// Human-readable description.
    pub description: String,
    /// Actor kinds allowed to see this capability in the catalog.
    pub actor_policy: Vec<ContextActorKind>,
    /// Operational risk level.
    pub risk_level: ContextRiskLevel,
    /// Side-effect class.
    pub side_effects: ContextSideEffects,
    /// Optional JSON Schema for input arguments.
    pub input_schema: Option<Value>,
    /// Optional JSON Schema for returned values.
    pub output_schema: Option<Value>,
    /// Optional resource URI template when this capability exposes handles.
    pub resource_uri_template: Option<String>,
    /// Cost hint.
    pub cost_hint: Option<ContextCostHint>,
    /// Latency hint.
    pub latency_hint: Option<ContextLatencyHint>,
    /// Approximate token budget hint.
    pub token_hint: Option<u32>,
    /// Current health.
    pub health: ContextCapabilityHealth,
    /// Owning crate or component.
    pub owner_crate: Option<String>,
    /// Audit policy name.
    pub audit_policy: Option<String>,
    /// Whether this capability belongs in the default model-visible set.
    pub default_visible: bool,
    /// Visibility policy name or short policy expression.
    pub visibility_policy: String,
    /// Optional target capability/resource that should replace a broad wrapper.
    pub split_target: Option<String>,
}

/// Capability kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCapabilityKind {
    /// Callable tool/action.
    Tool,
    /// Readable resource.
    Resource,
    /// Context graph builder/provider.
    ContextGraph,
    /// Catalog/discovery capability.
    Catalog,
}

/// Capability risk level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRiskLevel {
    /// Low risk.
    Low,
    /// Moderate risk.
    Moderate,
    /// High risk.
    High,
    /// Critical risk.
    Critical,
}

/// Capability side-effect class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSideEffects {
    /// No side effects.
    None,
    /// Reads outside model context.
    ReadExternal,
    /// Writes local state.
    WriteLocal,
    /// Writes external state.
    WriteExternal,
    /// Executes code or commands.
    Execute,
}

/// Cost hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCostHint {
    /// No material cost.
    Free,
    /// Cheap.
    Cheap,
    /// Moderate cost.
    Moderate,
    /// Expensive.
    Expensive,
}

/// Latency hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextLatencyHint {
    /// Local, usually synchronous.
    Local,
    /// Fast remote/local operation.
    Fast,
    /// Slow operation.
    Slow,
    /// Background operation.
    Background,
}

/// Capability health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCapabilityHealth {
    /// Available.
    Available,
    /// Available but degraded.
    Degraded,
    /// Unsupported by the current runtime.
    Unsupported,
    /// Disabled by config or policy.
    Disabled,
}

/// Bounded, traceable context assembled before an LLM call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPacket {
    /// Request id.
    pub request_id: String,
    /// Agent id.
    pub agent_id: String,
    /// Optional conversation/session id.
    pub conversation_id: Option<String>,
    /// Optional ward id.
    pub ward_id: Option<String>,
    /// Actor kind.
    pub actor_kind: ContextActorKind,
    /// Token budget metadata.
    pub budget: ContextBudget,
    /// Selected context atoms.
    pub atoms: Vec<ContextAtom>,
    /// Context graph nodes.
    pub graph_nodes: Vec<ContextGraphNode>,
    /// Context graph edges.
    pub graph_edges: Vec<ContextGraphEdge>,
    /// Resource handles.
    pub resource_handles: Vec<ContextResourceHandle>,
    /// Tool-result handles.
    pub tool_result_handles: Vec<ContextResourceHandle>,
    /// Dropped candidates and reasons.
    pub dropped: Vec<DroppedContextCandidate>,
    /// Assembly trace.
    pub trace: ContextTrace,
}

/// Incremental context produced during a running execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPacketDelta {
    /// Request or execution-local delta id.
    pub delta_id: String,
    /// The packet or execution this delta is associated with.
    pub request_id: String,
    /// Agent id.
    pub agent_id: String,
    /// Actor kind.
    pub actor_kind: ContextActorKind,
    /// Iteration that produced the delta.
    pub iteration: u32,
    /// Trigger type, such as tool_error or entity_mention.
    pub trigger_kind: String,
    /// Selected incremental atoms.
    pub atoms: Vec<ContextAtom>,
    /// Dropped candidates and reasons.
    pub dropped: Vec<DroppedContextCandidate>,
    /// Assembly trace.
    pub trace: ContextTrace,
}

impl ContextPacket {
    /// Small packet example used by schema and serialization tests.
    pub fn example() -> Self {
        let mut source_mix = BTreeMap::new();
        source_mix.insert("memory".to_string(), 1);
        source_mix.insert("graph".to_string(), 1);

        Self {
            request_id: "req-example".to_string(),
            agent_id: "root".to_string(),
            conversation_id: Some("sess-example".to_string()),
            ward_id: Some("ward-example".to_string()),
            actor_kind: ContextActorKind::Root,
            budget: ContextBudget {
                max_tokens: 2000,
                estimated_tokens: 240,
            },
            atoms: vec![ContextAtom {
                id: "atom-memory-1".to_string(),
                kind: "memory_fact".to_string(),
                content: "Use bounded context packets for model-visible recall.".to_string(),
                score: 0.92,
                confidence: 0.88,
                source: "memory".to_string(),
                source_id: Some("fact-1".to_string()),
                provenance: vec!["engram:memory/fact-1".to_string()],
                valid_from: Some(
                    "2026-07-06T12:00:00Z"
                        .parse::<DateTime<Utc>>()
                        .expect("valid example timestamp"),
                ),
                valid_until: None,
                visibility: vec![ContextActorKind::Root, ContextActorKind::DelegatedExecutor],
                route_hint: Some(serde_json::json!({ "lane": "memory" })),
                token_estimate: 32,
                render_policy: ContextRenderPolicy::Summary,
            }],
            graph_nodes: vec![ContextGraphNode {
                id: "node-context-packet".to_string(),
                kind: "concept".to_string(),
                label: "ContextPacket".to_string(),
                confidence: Some(0.91),
            }],
            graph_edges: vec![ContextGraphEdge {
                source: "node-context-packet".to_string(),
                target: "node-memory".to_string(),
                kind: "summarizes".to_string(),
                confidence: Some(0.83),
                provenance: vec!["engram:knowledge/edge-1".to_string()],
            }],
            resource_handles: vec![ContextResourceHandle {
                uri: "zbot://skills/context/sections/overview".to_string(),
                kind: "skill_section".to_string(),
                summary: "Context packet overview section.".to_string(),
                token_estimate: Some(120),
            }],
            tool_result_handles: vec![ContextResourceHandle {
                uri: "zbot://tool-results/req-example/shell-1".to_string(),
                kind: "tool_result".to_string(),
                summary: "Shell output stored behind a handle.".to_string(),
                token_estimate: Some(60),
            }],
            dropped: vec![DroppedContextCandidate {
                id: "atom-noisy-1".to_string(),
                reason: "over_budget".to_string(),
            }],
            trace: ContextTrace {
                selected_count: 4,
                dropped_count: 1,
                source_mix,
            },
        }
    }
}

/// Token budget metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    /// Maximum allowed tokens.
    pub max_tokens: u32,
    /// Estimated selected tokens.
    pub estimated_tokens: u32,
}

/// One selected context atom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextAtom {
    /// Stable atom id.
    pub id: String,
    /// Atom kind, such as memory_fact, wiki, procedure, graph, or episode.
    pub kind: String,
    /// Renderable content or summary.
    pub content: String,
    /// Ranking score.
    pub score: f64,
    /// Confidence in the atom.
    pub confidence: f64,
    /// Source lane.
    pub source: String,
    /// Optional source id.
    pub source_id: Option<String>,
    /// Provenance handles.
    pub provenance: Vec<String>,
    /// Valid-from timestamp.
    pub valid_from: Option<DateTime<Utc>>,
    /// Valid-until timestamp.
    pub valid_until: Option<DateTime<Utc>>,
    /// Actor visibility.
    pub visibility: Vec<ContextActorKind>,
    /// Optional routing hint.
    pub route_hint: Option<Value>,
    /// Estimated token count.
    pub token_estimate: u32,
    /// Render policy.
    pub render_policy: ContextRenderPolicy,
}

/// Render policy for a selected atom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRenderPolicy {
    /// Render content inline.
    Inline,
    /// Render summary.
    Summary,
    /// Render only a handle.
    HandleOnly,
    /// Keep hidden from prompt text.
    Hidden,
}

/// Context graph node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextGraphNode {
    /// Node id.
    pub id: String,
    /// Node kind.
    pub kind: String,
    /// Display label.
    pub label: String,
    /// Optional confidence.
    pub confidence: Option<f64>,
}

/// Context graph edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextGraphEdge {
    /// Source node id.
    pub source: String,
    /// Target node id.
    pub target: String,
    /// Edge kind.
    pub kind: String,
    /// Optional confidence.
    pub confidence: Option<f64>,
    /// Provenance handles.
    pub provenance: Vec<String>,
}

/// Resource or tool-result handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextResourceHandle {
    /// Resource URI.
    pub uri: String,
    /// Handle kind.
    pub kind: String,
    /// Human-readable summary.
    pub summary: String,
    /// Optional token estimate.
    pub token_estimate: Option<u32>,
}

/// Candidate omitted from the packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedContextCandidate {
    /// Candidate id.
    pub id: String,
    /// Drop reason.
    pub reason: String,
}

/// Context assembly trace metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextTrace {
    /// Count of selected context items.
    pub selected_count: u32,
    /// Count of dropped candidates.
    pub dropped_count: u32,
    /// Counts by source lane.
    pub source_mix: BTreeMap<String, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn object_keys(value: &Value) -> BTreeSet<String> {
        value
            .as_object()
            .expect("serialized value is an object")
            .keys()
            .cloned()
            .collect()
    }

    fn property_keys(schema: &Value, pointer: &str) -> BTreeSet<String> {
        schema
            .pointer(pointer)
            .expect("schema pointer exists")
            .as_object()
            .expect("schema properties are an object")
            .keys()
            .cloned()
            .collect()
    }

    fn required_keys(schema: &Value, pointer: &str) -> BTreeSet<String> {
        schema
            .pointer(pointer)
            .expect("schema pointer exists")
            .as_array()
            .expect("required list is an array")
            .iter()
            .map(|v| v.as_str().expect("required key is a string").to_string())
            .collect()
    }

    fn assert_keys_allowed(serialized: &Value, schema: &Value, properties_pointer: &str) {
        let allowed = property_keys(schema, properties_pointer);
        let actual = object_keys(serialized);
        for key in actual {
            assert!(allowed.contains(&key), "unexpected serialized key: {key}");
        }
    }

    fn assert_required_present(serialized: &Value, schema: &Value, required_pointer: &str) {
        let required = required_keys(schema, required_pointer);
        let actual = object_keys(serialized);
        for key in required {
            assert!(actual.contains(&key), "missing required key: {key}");
        }
    }

    #[test]
    fn context_capability_catalog_example_matches_schema_shape() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../contracts/jsonschema/context-capability-catalog.schema.json"
        ))
        .expect("catalog schema parses");
        let value = serde_json::to_value(ContextCapabilityCatalog::example_root())
            .expect("catalog serializes");

        assert_required_present(&value, &schema, "/required");
        assert_keys_allowed(&value, &schema, "/properties");
        assert_eq!(value["actor_kind"], "root");

        let capabilities = value["capabilities"]
            .as_array()
            .expect("capabilities is an array");
        assert!(!capabilities.is_empty());
        for capability in capabilities {
            assert_required_present(capability, &schema, "/$defs/capability/required");
            assert_keys_allowed(capability, &schema, "/$defs/capability/properties");
        }

        let wait_agent = capabilities
            .iter()
            .find(|capability| capability["id"] == "wait_agent")
            .expect("wait_agent capability present");
        assert_eq!(wait_agent["side_effects"], "read_external");
        assert_eq!(wait_agent["risk_level"], "low");
    }

    #[test]
    fn context_packet_example_matches_schema_shape() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../contracts/jsonschema/context-packet.schema.json"
        ))
        .expect("packet schema parses");
        let value = serde_json::to_value(ContextPacket::example()).expect("packet serializes");

        assert_required_present(&value, &schema, "/required");
        assert_keys_allowed(&value, &schema, "/properties");
        assert_eq!(value["actor_kind"], "root");
        assert_eq!(value["budget"]["max_tokens"], 2000);
        assert_eq!(value["atoms"][0]["render_policy"], "summary");

        assert_required_present(&value["budget"], &schema, "/$defs/budget/required");
        assert_keys_allowed(&value["budget"], &schema, "/$defs/budget/properties");

        for atom in value["atoms"].as_array().expect("atoms is an array") {
            assert_required_present(atom, &schema, "/$defs/atom/required");
            assert_keys_allowed(atom, &schema, "/$defs/atom/properties");
            assert!(atom.get("embedding").is_none());
        }
        for node in value["graph_nodes"]
            .as_array()
            .expect("graph_nodes is an array")
        {
            assert_required_present(node, &schema, "/$defs/graph_node/required");
            assert_keys_allowed(node, &schema, "/$defs/graph_node/properties");
        }
        for edge in value["graph_edges"]
            .as_array()
            .expect("graph_edges is an array")
        {
            assert_required_present(edge, &schema, "/$defs/graph_edge/required");
            assert_keys_allowed(edge, &schema, "/$defs/graph_edge/properties");
        }
        for handle in value["resource_handles"]
            .as_array()
            .expect("resource_handles is an array")
            .iter()
            .chain(
                value["tool_result_handles"]
                    .as_array()
                    .expect("tool_result_handles is an array"),
            )
        {
            assert_required_present(handle, &schema, "/$defs/resource_handle/required");
            assert_keys_allowed(handle, &schema, "/$defs/resource_handle/properties");
        }
        for dropped in value["dropped"].as_array().expect("dropped is an array") {
            assert_required_present(dropped, &schema, "/$defs/dropped/required");
            assert_keys_allowed(dropped, &schema, "/$defs/dropped/properties");
        }
        assert_required_present(&value["trace"], &schema, "/$defs/trace/required");
        assert_keys_allowed(&value["trace"], &schema, "/$defs/trace/properties");
        assert_eq!(value["trace"]["selected_count"], 4);
        assert_eq!(value["trace"]["dropped_count"], 1);
    }
}
