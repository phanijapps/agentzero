//! Startup capability reporting for Engram-backed provider selection.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::{AdapterConfig, ProviderMode};
use crate::fixtures::{FixtureRegistry, FixtureReport};
use crate::governance::{
    bootstrap::GovernanceBootstrapReport, AllowUnclassifiedPolicy, GovernanceScope,
    GovernanceValidationFinding, SkosExpansionPolicy, ValidationMode,
};

/// Adapter features that can be independently supported or disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterFeature {
    /// Memory fact listing, searching, writing, deletion, and lifecycle behavior.
    MemoryFacts,
    /// Ward content aggregation and global/session-local scope handling.
    Wards,
    /// Wiki article/source-backed knowledge behavior.
    Wiki,
    /// Entity and relationship graph behavior.
    KnowledgeGraph,
    /// Procedures and learned patterns.
    Procedures,
    /// Episodes and KG episodes.
    Episodes,
    /// Compaction state and related maintenance records.
    Compaction,
    /// Outbox persistence.
    Outbox,
    /// Goal, recall-log, and distillation auxiliary stores.
    Auxiliary,
    /// Belief store behavior.
    Beliefs,
    /// Belief contradiction store behavior.
    Contradictions,
    /// Unified recall and ranking.
    Recall,
    /// Hierarchy stats/build read models.
    Hierarchy,
    /// Migration dry-run and apply tooling.
    Migration,
}

impl AdapterFeature {
    /// Stable feature name for errors and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemoryFacts => "memory_facts",
            Self::Wards => "wards",
            Self::Wiki => "wiki",
            Self::KnowledgeGraph => "knowledge_graph",
            Self::Procedures => "procedures",
            Self::Episodes => "episodes",
            Self::Compaction => "compaction",
            Self::Outbox => "outbox",
            Self::Auxiliary => "auxiliary",
            Self::Beliefs => "beliefs",
            Self::Contradictions => "contradictions",
            Self::Recall => "recall",
            Self::Hierarchy => "hierarchy",
            Self::Migration => "migration",
        }
    }
}

/// Support status for a feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Feature is ready for provider use.
    Supported,
    /// Feature is intentionally disabled until implementation lands.
    Unsupported,
}

/// One feature capability row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    /// Feature described by this row.
    pub feature: AdapterFeature,
    /// Whether the feature is available.
    pub status: CapabilityStatus,
    /// Human-readable reason for unsupported rows.
    pub reason: Option<String>,
}

/// Startup report used before workers are started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
    /// Provider mode the report describes.
    pub provider_mode: ProviderMode,
    /// Per-feature support rows.
    pub capabilities: Vec<Capability>,
}

/// Sanitized governance health row for gateway/Observatory read models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceCapabilityHealth {
    /// True only when Engram mode has an active governance selection and
    /// governance bootstrap completed.
    pub supported: bool,
    /// Active ontology IDs selected by the default governance policy.
    pub ontology_ids: Vec<String>,
    /// Active SKOS scheme IDs selected by the default governance policy.
    pub taxonomy_scheme_ids: Vec<String>,
    /// Active validation mode.
    pub validation_mode: ValidationMode,
    /// Active unclassified-record policy.
    pub allow_unclassified: AllowUnclassifiedPolicy,
    /// Active bounded SKOS expansion policy.
    pub skos_expansion: SkosExpansionPolicy,
    /// Bootstrap counts when governance definitions were loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<GovernanceBootstrapHealth>,
    /// Number of current advisory findings included in the source sample.
    pub finding_count: usize,
    /// Unique sanitized finding codes from the source sample.
    pub finding_codes: Vec<String>,
}

/// Path-free governance bootstrap counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceBootstrapHealth {
    pub ontology_id: String,
    pub taxonomy_scheme_id: String,
    pub class_count: usize,
    pub property_count: usize,
    pub concept_count: usize,
    pub relation_count: usize,
}

impl GovernanceCapabilityHealth {
    /// Build an Observatory-safe governance health summary. It intentionally
    /// omits file paths, raw messages, entity IDs, and entity names.
    pub fn from_config_and_findings(
        config: &AdapterConfig,
        bootstrap: Option<&GovernanceBootstrapReport>,
        findings: &[GovernanceValidationFinding],
    ) -> Self {
        let selection = config.governance.select(GovernanceScope::default());
        let bootstrap = bootstrap.map(GovernanceBootstrapHealth::from);
        let mut finding_codes = findings
            .iter()
            .map(|finding| sanitize_finding_code(&finding.code))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        finding_codes.truncate(20);

        Self {
            supported: config.provider_mode == ProviderMode::Engram
                && !selection.is_empty()
                && bootstrap.is_some(),
            ontology_ids: selection.ontology_ids,
            taxonomy_scheme_ids: selection.taxonomy_scheme_ids,
            validation_mode: config.governance.validation_mode,
            allow_unclassified: config.governance.allow_unclassified,
            skos_expansion: config.governance.skos_expansion.clone(),
            bootstrap,
            finding_count: findings.len(),
            finding_codes,
        }
    }
}

impl From<&GovernanceBootstrapReport> for GovernanceBootstrapHealth {
    fn from(report: &GovernanceBootstrapReport) -> Self {
        Self {
            ontology_id: report.ontology_id.clone(),
            taxonomy_scheme_id: report.taxonomy_scheme_id.clone(),
            class_count: report.class_count,
            property_count: report.property_count,
            concept_count: report.concept_count,
            relation_count: report.relation_count,
        }
    }
}

impl CapabilityReport {
    /// Build the conservative initial report for this scaffold.
    pub fn from_config(config: &AdapterConfig) -> Self {
        let mut capabilities = all_features()
            .into_iter()
            .map(|feature| Capability {
                feature,
                status: CapabilityStatus::Unsupported,
                reason: Some("adapter implementation pending parity fixtures".to_string()),
            })
            .collect::<Vec<_>>();

        if config.provider_mode == ProviderMode::CurrentSqlite {
            capabilities.clear();
        }

        Self {
            provider_mode: config.provider_mode,
            capabilities,
        }
    }

    /// Build a report where support is allowed only for features with a
    /// complete fixture registry and passing current/Engram parity outcomes.
    pub fn from_fixture_report(
        config: &AdapterConfig,
        registry: &FixtureRegistry,
        report: &FixtureReport,
    ) -> Self {
        if config.provider_mode == ProviderMode::CurrentSqlite {
            return Self {
                provider_mode: config.provider_mode,
                capabilities: Vec::new(),
            };
        }

        let capabilities = all_features()
            .into_iter()
            .map(|feature| {
                if registry.feature_ready(feature, report) {
                    Capability {
                        feature,
                        status: CapabilityStatus::Supported,
                        reason: None,
                    }
                } else {
                    Capability {
                        feature,
                        status: CapabilityStatus::Unsupported,
                        reason: Some(
                            "feature lacks passing current/Engram parity fixtures".to_string(),
                        ),
                    }
                }
            })
            .collect();

        Self {
            provider_mode: config.provider_mode,
            capabilities,
        }
    }

    /// Build a report from an explicit set of implementation-verified
    /// features. This is narrower than `from_config`: unsupported features stay
    /// fail-closed even when one feature lands ahead of the rest.
    pub fn from_verified_features(
        config: &AdapterConfig,
        features: impl IntoIterator<Item = AdapterFeature>,
    ) -> Self {
        if config.provider_mode == ProviderMode::CurrentSqlite {
            return Self {
                provider_mode: config.provider_mode,
                capabilities: Vec::new(),
            };
        }

        let supported = features.into_iter().collect::<BTreeSet<_>>();
        let capabilities = all_features()
            .into_iter()
            .map(|feature| {
                if supported.contains(&feature) {
                    Capability {
                        feature,
                        status: CapabilityStatus::Supported,
                        reason: None,
                    }
                } else {
                    Capability {
                        feature,
                        status: CapabilityStatus::Unsupported,
                        reason: Some("feature has not been implementation-verified".to_string()),
                    }
                }
            })
            .collect();

        Self {
            provider_mode: config.provider_mode,
            capabilities,
        }
    }

    /// True when the feature is explicitly supported.
    pub fn supports(&self, feature: AdapterFeature) -> bool {
        self.capabilities.iter().any(|capability| {
            capability.feature == feature && capability.status == CapabilityStatus::Supported
        })
    }
}

fn all_features() -> Vec<AdapterFeature> {
    vec![
        AdapterFeature::MemoryFacts,
        AdapterFeature::Wards,
        AdapterFeature::Wiki,
        AdapterFeature::KnowledgeGraph,
        AdapterFeature::Procedures,
        AdapterFeature::Episodes,
        AdapterFeature::Compaction,
        AdapterFeature::Outbox,
        AdapterFeature::Auxiliary,
        AdapterFeature::Beliefs,
        AdapterFeature::Contradictions,
        AdapterFeature::Recall,
        AdapterFeature::Hierarchy,
        AdapterFeature::Migration,
    ]
}

fn sanitize_finding_code(code: &str) -> String {
    let mut sanitized = code
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    sanitized.truncate(96);
    if sanitized.trim_matches('_').is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        config::ProviderMode,
        governance::{
            AllowUnclassifiedPolicy, GovernanceFindingSeverity, GovernancePolicy,
            GovernanceSelection, GovernanceValidationFinding, SkosExpansionPolicy, ValidationMode,
        },
    };

    #[test]
    fn current_provider_report_has_no_adapter_disables() {
        let report = CapabilityReport::from_config(&AdapterConfig::default());

        assert_eq!(report.provider_mode, ProviderMode::CurrentSqlite);
        assert!(report.capabilities.is_empty());
    }

    #[test]
    fn engram_provider_report_fails_closed_until_features_land() {
        let config = AdapterConfig {
            provider_mode: ProviderMode::Engram,
            engram_path: Some(PathBuf::from("engram.db")),
            ..AdapterConfig::default()
        };

        let report = CapabilityReport::from_config(&config);

        assert_eq!(report.provider_mode, ProviderMode::Engram);
        assert!(!report.supports(AdapterFeature::MemoryFacts));
        assert!(report.capabilities.iter().all(|capability| {
            capability.status == CapabilityStatus::Unsupported
                && capability.reason.as_deref().is_some()
        }));
    }

    #[test]
    fn governance_health_reports_support_counts_and_sanitized_codes_only() {
        let config = AdapterConfig {
            provider_mode: ProviderMode::Engram,
            engram_path: Some(PathBuf::from("engram.db")),
            governance: GovernancePolicy {
                default_selection: GovernanceSelection {
                    ontology_ids: vec!["zbot.base:v1".to_string()],
                    taxonomy_scheme_ids: vec!["zbot.general:v1".to_string()],
                },
                validation_mode: ValidationMode::Advisory,
                allow_unclassified: AllowUnclassifiedPolicy::Warn,
                skos_expansion: SkosExpansionPolicy {
                    max_depth: 2,
                    max_fan_out: 4,
                    max_candidates: 10,
                },
                ..GovernancePolicy::default()
            },
            ..AdapterConfig::default()
        };
        let bootstrap = GovernanceBootstrapReport {
            ontology_id: "zbot.base:v1".to_string(),
            taxonomy_scheme_id: "zbot.general:v1".to_string(),
            class_count: 3,
            property_count: 4,
            concept_count: 5,
            relation_count: 6,
        };
        let findings = vec![GovernanceValidationFinding {
            id: "finding-secret".to_string(),
            ontology_id: "zbot.base:v1".to_string(),
            relationship_id: "rel-secret".to_string(),
            code: "unknown relationship/type".to_string(),
            severity: GovernanceFindingSeverity::Warning,
            message: "raw entity name and /home/user/secret/path are hidden".to_string(),
            target_entity_id: Some("entity-secret".to_string()),
            target_entity_type: Some("custom type".to_string()),
        }];

        let health = GovernanceCapabilityHealth::from_config_and_findings(
            &config,
            Some(&bootstrap),
            &findings,
        );
        let json = serde_json::to_string(&health).expect("json");

        assert!(health.supported);
        assert_eq!(health.ontology_ids, vec!["zbot.base:v1"]);
        assert_eq!(health.taxonomy_scheme_ids, vec!["zbot.general:v1"]);
        assert_eq!(health.finding_count, 1);
        assert_eq!(health.finding_codes, vec!["unknown_relationship_type"]);
        assert_eq!(health.bootstrap.expect("bootstrap").concept_count, 5);
        assert!(!json.contains("raw entity name"));
        assert!(!json.contains("/home/user"));
        assert!(!json.contains("entity-secret"));
    }

    #[test]
    fn governance_health_is_unsupported_without_bootstrap() {
        let config = AdapterConfig {
            provider_mode: ProviderMode::Engram,
            engram_path: Some(PathBuf::from("engram.db")),
            governance: GovernancePolicy {
                default_selection: GovernanceSelection {
                    ontology_ids: vec!["zbot.base:v1".to_string()],
                    taxonomy_scheme_ids: vec!["zbot.general:v1".to_string()],
                },
                ..GovernancePolicy::default()
            },
            ..AdapterConfig::default()
        };

        let health = GovernanceCapabilityHealth::from_config_and_findings(&config, None, &[]);

        assert!(!health.supported);
        assert!(health.bootstrap.is_none());
        assert_eq!(health.finding_count, 0);
    }
}
