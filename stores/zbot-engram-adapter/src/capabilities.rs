//! Startup capability reporting for Engram-backed provider selection.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::{AdapterConfig, ProviderMode};
use crate::fixtures::{FixtureRegistry, FixtureReport};

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::ProviderMode;

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
}
