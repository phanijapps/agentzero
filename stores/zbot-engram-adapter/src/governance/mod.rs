//! Zbot-owned ontology and taxonomy governance policy.
//!
//! This module is deliberately inert by default. It describes which local
//! definitions are active for a scope, and bootstraps the built-in definitions
//! once governance is selected. Later dynamic-ontology tasks wire governed
//! mapping, validation, and recall behavior.

pub mod bootstrap;

use std::{collections::BTreeSet, path::PathBuf};

use knowledge_graph::{EntityType, RelationshipType};
use serde::{Deserialize, Serialize};

/// Built-in ontology ID used when no local ontology overlay is selected.
pub const ZBOT_BASE_ONTOLOGY_ID: &str = "zbot.base:v1";

/// Built-in SKOS scheme ID used when no local taxonomy overlay is selected.
pub const ZBOT_GENERAL_SCHEME_ID: &str = "zbot.general:v1";

/// Default ontology validation behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationMode {
    /// Record findings but preserve writes.
    #[default]
    Advisory,
    /// Validation is disabled.
    Disabled,
}

/// Whether records without an ontology/taxonomy match remain writable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AllowUnclassifiedPolicy {
    /// Preserve unclassified records and optionally emit findings later.
    #[default]
    Allow,
    /// Report unclassified records as findings while still preserving writes.
    Warn,
}

/// Bounded SKOS expansion settings used by later recall work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkosExpansionPolicy {
    /// Maximum broader/narrower/related traversal depth.
    #[serde(default = "default_expansion_depth")]
    pub max_depth: u8,
    /// Maximum neighbors considered per concept.
    #[serde(default = "default_expansion_fan_out")]
    pub max_fan_out: u16,
    /// Maximum expansion candidates added to a query.
    #[serde(default = "default_expansion_candidates")]
    pub max_candidates: u16,
}

impl Default for SkosExpansionPolicy {
    fn default() -> Self {
        Self {
            max_depth: default_expansion_depth(),
            max_fan_out: default_expansion_fan_out(),
            max_candidates: default_expansion_candidates(),
        }
    }
}

/// Active ontology/taxonomy IDs for a selected scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceSelection {
    /// Versioned ontology IDs selected for the scope.
    #[serde(default)]
    pub ontology_ids: Vec<String>,
    /// Versioned SKOS concept-scheme IDs selected for the scope.
    #[serde(default)]
    pub taxonomy_scheme_ids: Vec<String>,
}

impl GovernanceSelection {
    /// Returns true when no governance definition is selected.
    pub fn is_empty(&self) -> bool {
        self.ontology_ids.is_empty() && self.taxonomy_scheme_ids.is_empty()
    }
}

/// Scoped selection overlay. The most specific matching dimension wins:
/// task > source > session > project > ward > default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceOverlay {
    #[serde(default)]
    pub ward_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub selection: GovernanceSelection,
}

/// Scope attributes available to the governance selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GovernanceScope<'a> {
    pub ward_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub source_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
}

/// Configured zbot governance policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernancePolicy {
    /// Local ontology definition files, relative to the trusted config root or
    /// absolute paths confined under that root.
    #[serde(default)]
    pub ontology_definition_paths: Vec<PathBuf>,
    /// Local SKOS taxonomy definition files, relative to the trusted config
    /// root or absolute paths confined under that root.
    #[serde(default)]
    pub taxonomy_definition_paths: Vec<PathBuf>,
    /// Fallback governance selection.
    #[serde(default)]
    pub default_selection: GovernanceSelection,
    /// Ordered scope overlays. Specificity wins before order.
    #[serde(default)]
    pub overlays: Vec<GovernanceOverlay>,
    /// Default validation mode.
    #[serde(default)]
    pub validation_mode: ValidationMode,
    /// Default unclassified-record behavior.
    #[serde(default)]
    pub allow_unclassified: AllowUnclassifiedPolicy,
    /// Bounded SKOS expansion policy.
    #[serde(default)]
    pub skos_expansion: SkosExpansionPolicy,
}

/// Code-owned ontology definition that can later be bootstrapped into Engram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyDefinition {
    pub ontology_id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entity_classes: Vec<OntologyClassDefinition>,
    #[serde(default)]
    pub relationship_properties: Vec<OntologyPropertyDefinition>,
}

/// Entity class in the zbot base ontology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyClassDefinition {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub open: bool,
}

/// Relationship property in the zbot base ontology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyPropertyDefinition {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub domain: Vec<String>,
    #[serde(default)]
    pub range: Vec<String>,
    #[serde(default)]
    pub open: bool,
}

/// Result of classifying a zbot value against a built-in definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationOutcome {
    /// The value mapped to a known definition ID.
    Classified(String),
    /// The value was preserved through the `Custom` escape hatch.
    UnclassifiedCustom(String),
}

/// Sanitized severity for persisted governance validation findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceFindingSeverity {
    Info,
    Warning,
}

/// Sanitized advisory finding persisted by the zbot adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceValidationFinding {
    pub id: String,
    pub ontology_id: String,
    pub relationship_id: String,
    pub code: String,
    pub severity: GovernanceFindingSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_entity_type: Option<String>,
}

/// Code-owned SKOS-style concept scheme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkosSchemeDefinition {
    pub scheme_id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub concepts: Vec<SkosConceptDefinition>,
}

/// SKOS-style concept with direct relation IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkosConceptDefinition {
    pub id: String,
    pub pref_label: String,
    #[serde(default)]
    pub alt_labels: Vec<String>,
    #[serde(default)]
    pub broader: Vec<String>,
    #[serde(default)]
    pub narrower: Vec<String>,
    #[serde(default)]
    pub related: Vec<String>,
    #[serde(default)]
    pub deprecated: bool,
}

/// Validate direct SKOS references in a concept scheme.
pub fn validate_skos_scheme(scheme: &SkosSchemeDefinition) -> Result<(), String> {
    let ids: BTreeSet<&str> = scheme
        .concepts
        .iter()
        .map(|concept| concept.id.as_str())
        .collect();
    if ids.len() != scheme.concepts.len() {
        return Err("duplicate concept id".to_string());
    }

    for concept in &scheme.concepts {
        if concept.pref_label.trim().is_empty() {
            return Err(format!("concept `{}` has empty prefLabel", concept.id));
        }
        for reference in concept
            .broader
            .iter()
            .chain(concept.narrower.iter())
            .chain(concept.related.iter())
        {
            if !ids.contains(reference.as_str()) {
                return Err(format!(
                    "concept `{}` references missing concept `{reference}`",
                    concept.id
                ));
            }
        }
    }
    Ok(())
}

/// Built-in base ontology derived from zbot's current KG vocabulary.
pub fn builtin_base_ontology() -> OntologyDefinition {
    OntologyDefinition {
        ontology_id: ZBOT_BASE_ONTOLOGY_ID.to_string(),
        label: "Zbot Base Ontology".to_string(),
        description: "Base advisory ontology for zbot memory and knowledge graph records."
            .to_string(),
        entity_classes: vec![
            class("person", "Person", &[]),
            class("organization", "Organization", &["org", "company"]),
            class("location", "Location", &["place"]),
            class("concept", "Concept", &["topic"]),
            class("tool", "Tool", &["technology"]),
            class("project", "Project", &[]),
            class("file", "File", &[]),
            class("event", "Event", &[]),
            class("time_period", "Time Period", &["year", "era"]),
            class("document", "Document", &["doc"]),
            class("role", "Role", &["title"]),
            class("artifact", "Artifact", &[]),
            class("ward", "Ward", &[]),
            OntologyClassDefinition {
                id: "custom".to_string(),
                label: "Custom".to_string(),
                aliases: Vec::new(),
                open: true,
            },
        ],
        relationship_properties: vec![
            property("works_for", "Works For", &["person"], &["organization"]),
            property("located_in", "Located In", &[], &["location"]),
            property("related_to", "Related To", &[], &[]),
            property("created", "Created", &[], &[]),
            property("uses", "Uses", &[], &[]),
            property("part_of", "Part Of", &[], &[]),
            property("mentions", "Mentions", &[], &[]),
            property(
                "before",
                "Before",
                &["event", "time_period"],
                &["event", "time_period"],
            ),
            property(
                "after",
                "After",
                &["event", "time_period"],
                &["event", "time_period"],
            ),
            property("during", "During", &["event"], &["event", "time_period"]),
            property("concurrent_with", "Concurrent With", &["event"], &["event"]),
            property("succeeded_by", "Succeeded By", &[], &[]),
            property("preceded_by", "Preceded By", &[], &[]),
            property(
                "president_of",
                "President Of",
                &["person"],
                &["organization", "location"],
            ),
            property(
                "founder_of",
                "Founder Of",
                &["person"],
                &["organization", "project"],
            ),
            property(
                "member_of",
                "Member Of",
                &["person", "organization"],
                &["organization"],
            ),
            property(
                "author_of",
                "Author Of",
                &["person", "organization"],
                &["document", "artifact"],
            ),
            property("held_role", "Held Role", &["person"], &["role"]),
            property("employed_by", "Employed By", &["person"], &["organization"]),
            property("held_at", "Held At", &["event"], &["location"]),
            property("born_in", "Born In", &["person"], &["location"]),
            property("died_in", "Died In", &["person"], &["location"]),
            property("caused", "Caused", &[], &[]),
            property("enabled", "Enabled", &[], &[]),
            property("prevented", "Prevented", &[], &[]),
            property("triggered_by", "Triggered By", &[], &[]),
            property("contains", "Contains", &[], &[]),
            property("instance_of", "Instance Of", &[], &["concept"]),
            property("subtype_of", "Subtype Of", &["concept"], &["concept"]),
            OntologyPropertyDefinition {
                id: "custom".to_string(),
                label: "Custom".to_string(),
                domain: Vec::new(),
                range: Vec::new(),
                open: true,
            },
        ],
    }
}

/// Built-in starter SKOS scheme for general zbot work.
pub fn builtin_starter_skos_scheme() -> SkosSchemeDefinition {
    SkosSchemeDefinition {
        scheme_id: ZBOT_GENERAL_SCHEME_ID.to_string(),
        label: "Zbot General Taxonomy".to_string(),
        description: "Starter SKOS-style concept scheme for zbot work modes.".to_string(),
        concepts: vec![
            concept("work", "Work", &["task", "job", "activity"], &[], &[], &[]),
            concept(
                "software_engineering",
                "Software Engineering",
                &["code", "coding", "development", "programming"],
                &["work"],
                &[],
                &["tools", "artifacts"],
            ),
            concept(
                "rust",
                "Rust",
                &["rustlang", "cargo", "crate"],
                &["software_engineering"],
                &[],
                &["tools"],
            ),
            concept(
                "agent_runtime",
                "Agent Runtime",
                &["agent engine", "orchestration", "execution engine"],
                &["software_engineering"],
                &[],
                &["tools", "memory", "context_graph"],
            ),
            concept(
                "tools",
                "Tools",
                &["tooling", "tool call", "capability"],
                &["work"],
                &[],
                &["context_graph", "artifacts"],
            ),
            concept(
                "memory",
                "Memory",
                &["durable memory", "recall", "facts"],
                &["work"],
                &[],
                &["knowledge_graph", "context_graph"],
            ),
            concept(
                "knowledge_graph",
                "Knowledge Graph",
                &["graph", "kg", "entities", "relationships"],
                &["memory"],
                &[],
                &["context_graph"],
            ),
            concept(
                "context_graph",
                "Context Graph",
                &["context packet", "context", "working context"],
                &["memory"],
                &[],
                &["tools", "knowledge_graph"],
            ),
            concept(
                "research",
                "Research",
                &["analysis", "investigation", "source review"],
                &["work"],
                &[],
                &["documents", "memory"],
            ),
            concept(
                "finance",
                "Finance",
                &["valuation", "market analysis", "equity research", "stocks"],
                &["research"],
                &[],
                &["documents"],
            ),
            concept(
                "documents",
                "Documents",
                &["article", "paper", "report", "spec", "rfc"],
                &["work"],
                &[],
                &["research", "artifacts"],
            ),
            concept(
                "artifacts",
                "Artifacts",
                &["file", "code artifact", "generated output"],
                &["work"],
                &[],
                &["software_engineering", "documents"],
            ),
        ],
    }
}

/// Classify an entity type using the built-in ontology.
pub fn classify_entity_type(entity_type: &EntityType) -> ClassificationOutcome {
    match entity_type {
        EntityType::Custom(value) => ClassificationOutcome::UnclassifiedCustom(value.clone()),
        _ => ClassificationOutcome::Classified(entity_type.as_str().to_string()),
    }
}

/// Classify a relationship type using the built-in ontology.
pub fn classify_relationship_type(relationship_type: &RelationshipType) -> ClassificationOutcome {
    match relationship_type {
        RelationshipType::Custom(value) => ClassificationOutcome::UnclassifiedCustom(value.clone()),
        _ => ClassificationOutcome::Classified(relationship_type.as_str().to_string()),
    }
}

/// Validate a relationship against the built-in ontology in advisory mode.
pub fn validate_relationship_against_builtin_ontology(
    relationship_id: &str,
    relationship_type: &RelationshipType,
    source: Option<(&str, &EntityType)>,
    target: Option<(&str, &EntityType)>,
    ontology_id: &str,
) -> Vec<GovernanceValidationFinding> {
    if ontology_id != ZBOT_BASE_ONTOLOGY_ID {
        return vec![finding(
            ontology_id,
            relationship_id,
            "missing_ontology_definition",
            GovernanceFindingSeverity::Warning,
            "selected ontology definition is not available to the adapter",
            None,
        )];
    }

    let ontology = builtin_base_ontology();
    let property_id = match classify_relationship_type(relationship_type) {
        ClassificationOutcome::Classified(id) => id,
        ClassificationOutcome::UnclassifiedCustom(_) => {
            return vec![finding(
                ontology_id,
                relationship_id,
                "unknown_predicate",
                GovernanceFindingSeverity::Warning,
                "relationship predicate is not declared by the active ontology",
                None,
            )];
        }
    };
    let Some(property) = ontology
        .relationship_properties
        .iter()
        .find(|property| property.id == property_id)
    else {
        return vec![finding(
            ontology_id,
            relationship_id,
            "unknown_predicate",
            GovernanceFindingSeverity::Warning,
            "relationship predicate is not declared by the active ontology",
            None,
        )];
    };

    let mut findings = Vec::new();
    match class_id_for_endpoint(source) {
        Some((entity_id, class_id)) => {
            if !property.domain.is_empty() && !property.domain.iter().any(|id| id == class_id) {
                findings.push(finding(
                    ontology_id,
                    relationship_id,
                    "domain_mismatch",
                    GovernanceFindingSeverity::Warning,
                    "relationship source class is outside the property domain",
                    Some((entity_id, class_id)),
                ));
            }
        }
        None => findings.push(finding(
            ontology_id,
            relationship_id,
            "missing_source_class",
            GovernanceFindingSeverity::Warning,
            "relationship source class is missing or unclassified",
            source.map(|(entity_id, entity_type)| (entity_id, entity_type.as_str())),
        )),
    }

    match class_id_for_endpoint(target) {
        Some((entity_id, class_id)) => {
            if !property.range.is_empty() && !property.range.iter().any(|id| id == class_id) {
                findings.push(finding(
                    ontology_id,
                    relationship_id,
                    "range_mismatch",
                    GovernanceFindingSeverity::Warning,
                    "relationship target class is outside the property range",
                    Some((entity_id, class_id)),
                ));
            }
        }
        None => findings.push(finding(
            ontology_id,
            relationship_id,
            "missing_target_class",
            GovernanceFindingSeverity::Warning,
            "relationship target class is missing or unclassified",
            target.map(|(entity_id, entity_type)| (entity_id, entity_type.as_str())),
        )),
    }

    findings
}

impl Default for GovernancePolicy {
    fn default() -> Self {
        Self {
            ontology_definition_paths: Vec::new(),
            taxonomy_definition_paths: Vec::new(),
            default_selection: GovernanceSelection::default(),
            overlays: Vec::new(),
            validation_mode: ValidationMode::Advisory,
            allow_unclassified: AllowUnclassifiedPolicy::Allow,
            skos_expansion: SkosExpansionPolicy::default(),
        }
    }
}

impl GovernancePolicy {
    /// Returns true when this policy preserves no-definition behavior.
    pub fn is_inert(&self) -> bool {
        self.ontology_definition_paths.is_empty()
            && self.taxonomy_definition_paths.is_empty()
            && self.default_selection.is_empty()
            && self.overlays.is_empty()
            && self.validation_mode == ValidationMode::Advisory
            && self.allow_unclassified == AllowUnclassifiedPolicy::Allow
            && self.skos_expansion == SkosExpansionPolicy::default()
    }

    /// Select the active governance IDs for a scope.
    pub fn select(&self, scope: GovernanceScope<'_>) -> GovernanceSelection {
        self.overlays
            .iter()
            .enumerate()
            .filter_map(|(idx, overlay)| {
                overlay
                    .match_rank(scope)
                    .map(|rank| (rank, idx, overlay.selection.clone()))
            })
            .max_by_key(|(rank, idx, _)| (*rank, *idx))
            .map(|(_, _, selection)| selection)
            .unwrap_or_else(|| self.default_selection.clone())
    }
}

impl GovernanceOverlay {
    fn match_rank(&self, scope: GovernanceScope<'_>) -> Option<u8> {
        let mut rank = 0;

        if let Some(expected) = self.ward_id.as_deref() {
            if Some(expected) != scope.ward_id {
                return None;
            }
            rank = rank.max(1);
        }
        if let Some(expected) = self.project_id.as_deref() {
            if Some(expected) != scope.project_id {
                return None;
            }
            rank = rank.max(2);
        }
        if let Some(expected) = self.session_id.as_deref() {
            if Some(expected) != scope.session_id {
                return None;
            }
            rank = rank.max(3);
        }
        if let Some(expected) = self.source_id.as_deref() {
            if Some(expected) != scope.source_id {
                return None;
            }
            rank = rank.max(4);
        }
        if let Some(expected) = self.task_id.as_deref() {
            if Some(expected) != scope.task_id {
                return None;
            }
            rank = rank.max(5);
        }

        (rank > 0).then_some(rank)
    }
}

fn default_expansion_depth() -> u8 {
    1
}

fn default_expansion_fan_out() -> u16 {
    8
}

fn default_expansion_candidates() -> u16 {
    16
}

fn class(id: &str, label: &str, aliases: &[&str]) -> OntologyClassDefinition {
    OntologyClassDefinition {
        id: id.to_string(),
        label: label.to_string(),
        aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
        open: false,
    }
}

fn property(id: &str, label: &str, domain: &[&str], range: &[&str]) -> OntologyPropertyDefinition {
    OntologyPropertyDefinition {
        id: id.to_string(),
        label: label.to_string(),
        domain: domain.iter().map(|id| (*id).to_string()).collect(),
        range: range.iter().map(|id| (*id).to_string()).collect(),
        open: false,
    }
}

fn concept(
    id: &str,
    pref_label: &str,
    alt_labels: &[&str],
    broader: &[&str],
    narrower: &[&str],
    related: &[&str],
) -> SkosConceptDefinition {
    SkosConceptDefinition {
        id: id.to_string(),
        pref_label: pref_label.to_string(),
        alt_labels: alt_labels
            .iter()
            .map(|label| (*label).to_string())
            .collect(),
        broader: broader.iter().map(|id| (*id).to_string()).collect(),
        narrower: narrower.iter().map(|id| (*id).to_string()).collect(),
        related: related.iter().map(|id| (*id).to_string()).collect(),
        deprecated: false,
    }
}

fn class_id_for_endpoint<'a>(
    endpoint: Option<(&'a str, &'a EntityType)>,
) -> Option<(&'a str, &'a str)> {
    let (entity_id, entity_type) = endpoint?;
    match classify_entity_type(entity_type) {
        ClassificationOutcome::Classified(_) => Some((entity_id, entity_type.as_str())),
        ClassificationOutcome::UnclassifiedCustom(_) => None,
    }
}

fn finding(
    ontology_id: &str,
    relationship_id: &str,
    code: &str,
    severity: GovernanceFindingSeverity,
    message: &str,
    target: Option<(&str, &str)>,
) -> GovernanceValidationFinding {
    let ontology_id = sanitize_identifier(ontology_id);
    let relationship_id = sanitize_identifier(relationship_id);
    let target_suffix = target
        .map(|(entity_id, entity_type)| {
            format!(
                ":{}:{}",
                sanitize_identifier(entity_id),
                sanitize_identifier(entity_type)
            )
        })
        .unwrap_or_default();
    GovernanceValidationFinding {
        id: format!("governance:{ontology_id}:{relationship_id}:{code}{target_suffix}"),
        ontology_id,
        relationship_id,
        code: code.to_string(),
        severity,
        message: message.to_string(),
        target_entity_id: target.map(|(entity_id, _)| sanitize_identifier(entity_id)),
        target_entity_type: target.map(|(_, entity_type)| sanitize_identifier(entity_type)),
    }
}

fn sanitize_identifier(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    sanitized.truncate(128);
    if sanitized.trim_matches('_').is_empty() {
        "redacted".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(id: &str) -> GovernanceSelection {
        GovernanceSelection {
            ontology_ids: vec![format!("ontology.{id}")],
            taxonomy_scheme_ids: vec![format!("scheme.{id}")],
        }
    }

    #[test]
    fn default_governance_policy_is_inert() {
        assert!(GovernancePolicy::default().is_inert());
    }

    #[test]
    fn selector_precedence_is_task_source_session_project_ward_default() {
        let policy = GovernancePolicy {
            default_selection: selection("default"),
            overlays: vec![
                GovernanceOverlay {
                    ward_id: Some("ward-a".to_string()),
                    selection: selection("ward"),
                    ..GovernanceOverlay::default()
                },
                GovernanceOverlay {
                    project_id: Some("project-a".to_string()),
                    selection: selection("project"),
                    ..GovernanceOverlay::default()
                },
                GovernanceOverlay {
                    session_id: Some("session-a".to_string()),
                    selection: selection("session"),
                    ..GovernanceOverlay::default()
                },
                GovernanceOverlay {
                    source_id: Some("source-a".to_string()),
                    selection: selection("source"),
                    ..GovernanceOverlay::default()
                },
                GovernanceOverlay {
                    task_id: Some("task-a".to_string()),
                    selection: selection("task"),
                    ..GovernanceOverlay::default()
                },
            ],
            ..GovernancePolicy::default()
        };

        let selected = policy.select(GovernanceScope {
            ward_id: Some("ward-a"),
            project_id: Some("project-a"),
            session_id: Some("session-a"),
            source_id: Some("source-a"),
            task_id: Some("task-a"),
        });

        assert_eq!(selected, selection("task"));

        let selected = policy.select(GovernanceScope {
            ward_id: Some("ward-a"),
            project_id: Some("project-a"),
            session_id: Some("session-a"),
            source_id: Some("source-a"),
            task_id: None,
        });

        assert_eq!(selected, selection("source"));
    }

    #[test]
    fn base_ontology_covers_current_zbot_vocabulary() {
        let ontology = builtin_base_ontology();
        let class_ids: BTreeSet<&str> = ontology
            .entity_classes
            .iter()
            .filter(|class| !class.open)
            .map(|class| class.id.as_str())
            .collect();
        let property_ids: BTreeSet<&str> = ontology
            .relationship_properties
            .iter()
            .filter(|property| !property.open)
            .map(|property| property.id.as_str())
            .collect();

        assert_eq!(
            class_ids,
            EntityType::BUILTIN_IDS
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            property_ids,
            RelationshipType::BUILTIN_IDS
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn custom_values_remain_unclassified_escape_hatches() {
        assert_eq!(
            classify_entity_type(&EntityType::Custom("dataset".to_string())),
            ClassificationOutcome::UnclassifiedCustom("dataset".to_string())
        );
        assert_eq!(
            classify_relationship_type(&RelationshipType::Custom("indexes".to_string())),
            ClassificationOutcome::UnclassifiedCustom("indexes".to_string())
        );
    }

    #[test]
    fn relationship_validation_reports_missing_classes_and_sanitizes_ids() {
        let findings = validate_relationship_against_builtin_ontology(
            "rel-/home/example/providers.json",
            &RelationshipType::WorksFor,
            None,
            Some(("/tmp/raw-target", &EntityType::Organization)),
            ZBOT_BASE_ONTOLOGY_ID,
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "missing_source_class");
        let json = serde_json::to_string(&findings[0]).expect("finding json");
        assert!(!json.contains("/home/example"));
        assert!(!json.contains("/tmp/raw-target"));
        assert!(json.contains("rel-_home_example_providers.json"));
    }

    #[test]
    fn starter_skos_scheme_has_valid_labels_and_relations() {
        let scheme = builtin_starter_skos_scheme();

        validate_skos_scheme(&scheme).expect("valid starter scheme");
        assert_eq!(scheme.scheme_id, ZBOT_GENERAL_SCHEME_ID);
        assert!(scheme
            .concepts
            .iter()
            .any(|concept| concept.alt_labels.iter().any(|label| label == "kg")));
    }
}
