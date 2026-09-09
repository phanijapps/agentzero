//! Engram provider bootstrap and fail-closed feature gating.

use std::sync::Arc;

use engram_belief::BeliefRepository;
use engram_domain::CapabilityState;
use engram_hierarchy::HierarchyRepository;
use engram_integration::EngramProvider as UpstreamEngramProvider;
use engram_knowledge::{KnowledgeRepository, TaxonomyRepository};
use engram_memory::MemoryService;
use tokio::runtime::Handle;

use crate::{
    capabilities::{AdapterFeature, CapabilityReport},
    config::{AdapterConfig, ProviderMode},
    error::{AdapterError, AdapterResult},
    governance::{
        bootstrap::{
            bootstrap_governance_definitions, GovernanceBootstrapReport, GovernanceDefinitions,
        },
        SkosSchemeDefinition,
    },
};

/// Engram provider facade plus AgentZero-specific capability gates.
pub struct EngramProvider {
    pub(crate) provider: UpstreamEngramProvider,
    capabilities: CapabilityReport,
    governance_bootstrap: Option<GovernanceBootstrapReport>,
    governance_definitions: Option<GovernanceDefinitions>,
    governance_policy: crate::governance::GovernancePolicy,
    tenant: String,
}

impl EngramProvider {
    /// Open an Engram provider for `ProviderMode::Engram`.
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "engram_provider",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        let engram_config = config.to_engram_config()?;
        let provider =
            UpstreamEngramProvider::open(&engram_config).map_err(|_| AdapterError::Bootstrap {
                component: "provider",
                reason: "provider facade bootstrap failed".to_string(),
            })?;
        let (governance_definitions, governance_bootstrap) =
            bootstrap_governance_if_configured(&config, &provider)?;
        let capabilities = CapabilityReport::from_verified_features(
            &config,
            adapter_features_supported_by(provider.capabilities()),
        );

        Ok(Self {
            provider,
            capabilities,
            governance_bootstrap,
            governance_definitions,
            governance_policy: config.governance.clone(),
            tenant: config.tenant,
        })
    }

    /// Names of Engram provider components opened during bootstrap.
    pub fn opened_components(&self) -> Vec<&'static str> {
        let mut components = Vec::new();
        if self.provider.memory().is_some() {
            components.push("memory");
        }
        if self.provider.knowledge().is_some() {
            components.push("knowledge");
        }
        if self.provider.graph().is_some() {
            components.push("graph");
        }
        if self.provider.ontology().is_some() {
            components.push("ontology");
        }
        if self.provider.taxonomy().is_some() {
            components.push("taxonomy");
        }
        if self.provider.beliefs().is_some() {
            components.push("beliefs");
        }
        if self.provider.hierarchy().is_some() {
            components.push("hierarchy");
        }
        if self.provider.vectors().is_some() {
            components.push("vectors");
        }
        if self.provider.migration().is_some() {
            components.push("migration");
        }
        if self.provider.provenance().is_some() {
            components.push("provenance");
        }
        if self.provider.batch().is_some() {
            components.push("batch");
        }
        if self.provider.recall().is_some() {
            components.push("engram_unified_recall");
        }
        if self.provider.observability().is_some() {
            components.push("observability");
        }
        components
    }

    /// Startup capability report for AgentZero feature families.
    pub fn capabilities(&self) -> &CapabilityReport {
        &self.capabilities
    }

    /// Governance bootstrap summary, if governance definitions were configured.
    pub fn governance_bootstrap(&self) -> Option<&GovernanceBootstrapReport> {
        self.governance_bootstrap.as_ref()
    }

    /// Immutable taxonomy definitions that were bootstrapped alongside this
    /// provider. Recall expansion uses this snapshot for relation traversal
    /// while the taxonomy repository remains the durable source of concepts.
    pub(crate) fn taxonomy_definitions(&self) -> Option<&[SkosSchemeDefinition]> {
        self.governance_definitions
            .as_ref()
            .map(|definitions| definitions.taxonomies.as_slice())
    }

    /// Confirms that an adapter-facing config is the same governance and
    /// tenant identity that opened this provider. This prevents a caller from
    /// combining one provider's durable definitions with another policy.
    pub(crate) fn matches_governance_config(&self, config: &AdapterConfig) -> bool {
        self.tenant == config.tenant && self.governance_policy == config.governance
    }

    /// Upstream Engram capability report.
    pub fn upstream_capabilities(&self) -> &engram_integration::CapabilityReport {
        self.provider.capabilities()
    }

    /// Require a feature before side effects are attempted.
    pub fn require_feature(&self, feature: AdapterFeature) -> AdapterResult<()> {
        if self.capabilities.supports(feature) {
            return Ok(());
        }

        Err(AdapterError::UnsupportedFeature {
            feature: feature.as_str(),
            reason: "feature has not passed Engram parity fixtures".to_string(),
        })
    }

    pub(crate) fn memory(&self) -> AdapterResult<Arc<dyn MemoryService>> {
        self.provider
            .memory()
            .cloned()
            .ok_or_else(|| unsupported_upstream_handle("memory"))
    }

    pub(crate) fn knowledge(&self) -> AdapterResult<Arc<dyn KnowledgeRepository>> {
        self.provider
            .knowledge()
            .cloned()
            .ok_or_else(|| unsupported_upstream_handle("knowledge"))
    }

    pub(crate) fn beliefs(&self) -> AdapterResult<Arc<dyn BeliefRepository>> {
        self.provider
            .beliefs()
            .cloned()
            .ok_or_else(|| unsupported_upstream_handle("beliefs"))
    }

    pub(crate) fn hierarchy(&self) -> AdapterResult<Arc<dyn HierarchyRepository>> {
        self.provider
            .hierarchy()
            .cloned()
            .ok_or_else(|| unsupported_upstream_handle("hierarchy"))
    }

    pub(crate) fn taxonomy(&self) -> AdapterResult<Arc<dyn TaxonomyRepository>> {
        self.provider
            .taxonomy()
            .cloned()
            .ok_or_else(|| unsupported_upstream_handle("taxonomy"))
    }
}

fn bootstrap_governance_if_configured(
    config: &AdapterConfig,
    provider: &UpstreamEngramProvider,
) -> AdapterResult<(
    Option<GovernanceDefinitions>,
    Option<GovernanceBootstrapReport>,
)> {
    if config.governance.is_inert() {
        return Ok((None, None));
    }

    // Fail-soft: if the upstream conformance check didn't expose ontology or
    // taxonomy handles, skip governance rather than crashing the daemon.
    // The conformance check runs against an in-memory store and can fail
    // independently of the on-disk database's health.
    let (Some(ontology_repo), Some(taxonomy_repo)) =
        (provider.ontology().cloned(), provider.taxonomy().cloned())
    else {
        tracing::warn!(
            ontology_available = provider.ontology().is_some(),
            taxonomy_available = provider.taxonomy().is_some(),
            "Engram governance: upstream handles unavailable — skipping governance bootstrap"
        );
        return Ok((None, None));
    };

    let config = config.clone();
    let result = match Handle::try_current() {
        Ok(_) => std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| AdapterError::Bootstrap {
                    component: "governance_runtime",
                    reason: "failed to create bootstrap runtime".to_string(),
                })?
                .block_on(bootstrap_governance_definitions(
                    &config,
                    ontology_repo,
                    taxonomy_repo,
                ))
        })
        .join()
        .map_err(|_| AdapterError::Bootstrap {
            component: "governance_runtime",
            reason: "bootstrap runtime thread panicked".to_string(),
        })?,
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| AdapterError::Bootstrap {
                component: "governance_runtime",
                reason: "failed to create bootstrap runtime".to_string(),
            })?
            .block_on(bootstrap_governance_definitions(
                &config,
                ontology_repo,
                taxonomy_repo,
            )),
    }?;

    let (definitions, report) = result;
    Ok((Some(definitions), Some(report)))
}

fn adapter_features_supported_by(
    report: &engram_integration::CapabilityReport,
) -> Vec<AdapterFeature> {
    let mut features = Vec::new();
    if supported(&report.memory) {
        features.push(AdapterFeature::MemoryFacts);
    }
    if supported(&report.knowledge) {
        features.push(AdapterFeature::Wiki);
    }
    if supported(&report.knowledge) && supported(&report.graph) {
        features.push(AdapterFeature::KnowledgeGraph);
    }
    if supported(&report.hierarchy) {
        features.push(AdapterFeature::Hierarchy);
    }
    if supported(&report.beliefs) {
        features.push(AdapterFeature::Beliefs);
        features.push(AdapterFeature::Contradictions);
    }
    features
}

fn supported(state: &CapabilityState) -> bool {
    state.is_supported()
}

fn unsupported_upstream_handle(feature: &'static str) -> AdapterError {
    AdapterError::UnsupportedFeature {
        feature,
        reason: "Engram provider did not expose a supported handle".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_domain::{CapabilityReason, CapabilityState};
    use engram_integration::CapabilityReport as EngramCapabilityReport;

    #[test]
    fn adapter_features_require_matching_upstream_support() {
        let unsupported = CapabilityState::Unsupported {
            reason: CapabilityReason::ProviderUnavailable,
        };
        let report = EngramCapabilityReport::builder()
            .memory(CapabilityState::Supported)
            .knowledge(CapabilityState::Supported)
            .graph(unsupported.clone())
            .beliefs(CapabilityState::Supported)
            .hierarchy(unsupported)
            .build();

        let features = adapter_features_supported_by(&report);

        assert!(features.contains(&AdapterFeature::MemoryFacts));
        assert!(features.contains(&AdapterFeature::Wiki));
        assert!(features.contains(&AdapterFeature::Beliefs));
        assert!(features.contains(&AdapterFeature::Contradictions));
        assert!(!features.contains(&AdapterFeature::KnowledgeGraph));
        assert!(!features.contains(&AdapterFeature::Hierarchy));
    }

    #[test]
    fn semantic_services_require_capability_and_handle() {
        assert!(semantic_handle_ready(&CapabilityState::Supported, true));
        assert!(!semantic_handle_ready(&CapabilityState::Supported, false));
        assert!(!semantic_handle_ready(
            &CapabilityState::Unsupported {
                reason: CapabilityReason::ProviderUnavailable,
            },
            true,
        ));
    }
}
