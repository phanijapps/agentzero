//! Zbot-owned ontology and taxonomy governance policy.
//!
//! This module is deliberately inert by default. It describes which local
//! definitions are active for a scope, but it does not bootstrap definitions or
//! change writes until later dynamic-ontology tasks wire those behaviors.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
}
