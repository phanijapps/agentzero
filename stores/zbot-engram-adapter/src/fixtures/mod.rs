//! Provider parity fixture registry used by capability support gates.

use std::collections::BTreeSet;

#[cfg(test)]
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::AdapterErrorKind;
use crate::{AdapterError, AdapterFeature, AdapterResult};

/// Provider implementation a fixture outcome came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureProvider {
    /// Existing AgentZero SQLite store implementation.
    CurrentSqlite,
    /// Engram-backed adapter candidate.
    EngramCandidate,
}

/// Result of one provider running one fixture case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureStatus {
    /// Fixture passed.
    Passed,
    /// Fixture ran and failed.
    Failed,
    /// Fixture is not implemented for this provider yet.
    Unsupported,
}

/// Scope axis that must have positive and negative fixture coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeDimension {
    /// Tenant isolation.
    Tenant,
    /// Ward/workspace isolation.
    Ward,
    /// Session isolation.
    Session,
    /// Partition isolation.
    Partition,
}

/// Whether a scope case proves accepted or rejected behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeExpectation {
    /// Valid scoped behavior is accepted.
    Positive,
    /// Cross-scope or invalid scoped behavior is rejected or isolated.
    Negative,
}

/// One scope coverage marker attached to an executable scope fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeFixture {
    /// Scope axis covered by this marker.
    pub dimension: ScopeDimension,
    /// Accepted/rejected behavior covered by this marker.
    pub expectation: ScopeExpectation,
}

impl ScopeFixture {
    /// Build a static scope marker.
    pub const fn new(dimension: ScopeDimension, expectation: ScopeExpectation) -> Self {
        Self {
            dimension,
            expectation,
        }
    }
}

/// Scope coverage required before any feature can report supported.
pub const REQUIRED_SCOPE_FIXTURES: &[ScopeFixture] = &[
    ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Positive),
    ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Negative),
    ScopeFixture::new(ScopeDimension::Ward, ScopeExpectation::Positive),
    ScopeFixture::new(ScopeDimension::Ward, ScopeExpectation::Negative),
    ScopeFixture::new(ScopeDimension::Session, ScopeExpectation::Positive),
    ScopeFixture::new(ScopeDimension::Session, ScopeExpectation::Negative),
    ScopeFixture::new(ScopeDimension::Partition, ScopeExpectation::Positive),
    ScopeFixture::new(ScopeDimension::Partition, ScopeExpectation::Negative),
];

/// Stable adapter fixture id for `memory_save_and_count`.
pub const MEMORY_SAVE_AND_COUNT_ID: &str = "memory.save_and_count";

/// Stable adapter fixture id for `belief_upsert_get_round_trip`.
pub const BELIEF_UPSERT_GET_ID: &str = "belief.upsert_get";

/// Reference to an exported `zbot-stores-conformance` function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreTraitFixture {
    /// Stable conformance case id.
    pub id: &'static str,
    /// Exported conformance function name.
    pub function: &'static str,
}

/// Executable fixture case kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureCaseKind {
    /// Store-trait behavior case that provider backends execute by calling the
    /// referenced conformance function.
    StoreTrait(StoreTraitFixture),
    /// Scope case with positive or negative isolation behavior.
    Scope(ScopeFixture),
}

/// One parity fixture case that must be runnable against both providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureCase {
    id: &'static str,
    feature: AdapterFeature,
    kind: FixtureCaseKind,
}

impl FixtureCase {
    /// Build a store-trait conformance fixture case.
    pub(crate) const fn store_trait(
        id: &'static str,
        feature: AdapterFeature,
        function: &'static str,
    ) -> Self {
        Self {
            id,
            feature,
            kind: FixtureCaseKind::StoreTrait(StoreTraitFixture { id, function }),
        }
    }

    /// Build a scope fixture case.
    pub(crate) const fn scope(
        id: &'static str,
        feature: AdapterFeature,
        scope: ScopeFixture,
    ) -> Self {
        Self {
            id,
            feature,
            kind: FixtureCaseKind::Scope(scope),
        }
    }

    /// Stable fixture id.
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// Feature protected by this case.
    pub fn feature(&self) -> AdapterFeature {
        self.feature
    }

    /// Executable case kind.
    pub fn kind(&self) -> FixtureCaseKind {
        self.kind
    }

    /// Scope coverage supplied by this case, if any.
    pub fn scope_fixture(&self) -> Option<ScopeFixture> {
        match self.kind {
            FixtureCaseKind::StoreTrait(_) => None,
            FixtureCaseKind::Scope(scope) => Some(scope),
        }
    }

    /// True when the case references a store-trait conformance function.
    pub fn is_store_trait(&self) -> bool {
        matches!(self.kind, FixtureCaseKind::StoreTrait(_))
    }
}

const DEFAULT_CASES: &[FixtureCase] = &[
    FixtureCase::store_trait(
        MEMORY_SAVE_AND_COUNT_ID,
        AdapterFeature::MemoryFacts,
        "memory_save_and_count",
    ),
    FixtureCase::scope(
        "memory.scope.tenant.positive",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "memory.scope.tenant.negative",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Negative),
    ),
    FixtureCase::scope(
        "memory.scope.ward.positive",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Ward, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "memory.scope.ward.negative",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Ward, ScopeExpectation::Negative),
    ),
    FixtureCase::scope(
        "memory.scope.session.positive",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Session, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "memory.scope.session.negative",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Session, ScopeExpectation::Negative),
    ),
    FixtureCase::scope(
        "memory.scope.partition.positive",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Partition, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "memory.scope.partition.negative",
        AdapterFeature::MemoryFacts,
        ScopeFixture::new(ScopeDimension::Partition, ScopeExpectation::Negative),
    ),
    FixtureCase::store_trait(
        BELIEF_UPSERT_GET_ID,
        AdapterFeature::Beliefs,
        "belief_upsert_get_round_trip",
    ),
    FixtureCase::scope(
        "belief.scope.tenant.positive",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "belief.scope.tenant.negative",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Negative),
    ),
    FixtureCase::scope(
        "belief.scope.ward.positive",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Ward, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "belief.scope.ward.negative",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Ward, ScopeExpectation::Negative),
    ),
    FixtureCase::scope(
        "belief.scope.session.positive",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Session, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "belief.scope.session.negative",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Session, ScopeExpectation::Negative),
    ),
    FixtureCase::scope(
        "belief.scope.partition.positive",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Partition, ScopeExpectation::Positive),
    ),
    FixtureCase::scope(
        "belief.scope.partition.negative",
        AdapterFeature::Beliefs,
        ScopeFixture::new(ScopeDimension::Partition, ScopeExpectation::Negative),
    ),
];

/// Registry of accepted parity fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureRegistry {
    cases: Vec<FixtureCase>,
}

impl FixtureRegistry {
    /// Build a registry from explicit cases.
    pub(crate) fn new(cases: impl IntoIterator<Item = FixtureCase>) -> Self {
        Self {
            cases: cases.into_iter().collect(),
        }
    }

    /// Iterate all registered cases.
    pub fn cases(&self) -> impl Iterator<Item = &FixtureCase> {
        self.cases.iter()
    }

    /// Iterate cases for one feature.
    pub fn cases_for_feature(&self, feature: AdapterFeature) -> impl Iterator<Item = &FixtureCase> {
        self.cases
            .iter()
            .filter(move |case| case.feature == feature)
    }

    /// True when a stable case id is registered.
    pub fn contains_case(&self, id: &str) -> bool {
        self.cases.iter().any(|case| case.id == id)
    }

    /// Validate that a feature has registered store-trait parity and complete
    /// executable scope coverage before it can be considered for support.
    pub fn validate_feature_scope(&self, feature: AdapterFeature) -> AdapterResult<()> {
        let cases = self.cases_for_feature(feature).collect::<Vec<_>>();
        if cases.is_empty() {
            return Err(AdapterError::UnsupportedFeature {
                feature: feature.as_str(),
                reason: "feature has no registered parity fixture".to_string(),
            });
        }
        if !cases.iter().any(|case| case.is_store_trait()) {
            return Err(AdapterError::UnsupportedFeature {
                feature: feature.as_str(),
                reason: "feature has no registered store-trait conformance fixture".to_string(),
            });
        }

        let covered = cases
            .iter()
            .filter_map(|case| case.scope_fixture())
            .collect::<BTreeSet<_>>();

        for required in REQUIRED_SCOPE_FIXTURES {
            if !covered.contains(required) {
                return Err(AdapterError::UnsupportedFeature {
                    feature: feature.as_str(),
                    reason: format!(
                        "feature parity fixture missing {:?} {:?} scope coverage",
                        required.dimension, required.expectation
                    ),
                });
            }
        }

        Ok(())
    }

    /// True only when the registry is complete and every executable case for
    /// the feature passed against both current SQLite and the Engram candidate.
    pub fn feature_ready(&self, feature: AdapterFeature, report: &FixtureReport) -> bool {
        self.validate_feature_scope(feature).is_ok()
            && self.cases_for_feature(feature).all(|case| {
                report.has_passed(FixtureProvider::CurrentSqlite, case.id)
                    && report.has_passed(FixtureProvider::EngramCandidate, case.id)
            })
    }
}

impl Default for FixtureRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_CASES.iter().copied())
    }
}

/// Outcome from running one fixture against one provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureOutcome {
    provider: FixtureProvider,
    case_id: String,
    status: FixtureStatus,
}

impl FixtureOutcome {
    #[cfg(test)]
    fn passed(provider: FixtureProvider, case_id: impl Into<String>) -> Self {
        Self {
            provider,
            case_id: case_id.into(),
            status: FixtureStatus::Passed,
        }
    }

    #[cfg(test)]
    fn failed(provider: FixtureProvider, case_id: impl Into<String>) -> Self {
        Self {
            provider,
            case_id: case_id.into(),
            status: FixtureStatus::Failed,
        }
    }

    #[cfg(test)]
    fn unsupported(provider: FixtureProvider, case_id: impl Into<String>) -> Self {
        Self {
            provider,
            case_id: case_id.into(),
            status: FixtureStatus::Unsupported,
        }
    }

    /// Provider that produced this outcome.
    pub fn provider(&self) -> FixtureProvider {
        self.provider
    }

    /// Stable fixture id.
    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    /// Outcome status.
    pub fn status(&self) -> FixtureStatus {
        self.status
    }
}

/// Collected fixture outcomes for capability gating.
///
/// This type is intentionally not deserializable. Support decisions must use
/// reports produced in-process by [`ParityRunner`], not persisted diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureReport {
    outcomes: Vec<FixtureOutcome>,
}

impl FixtureReport {
    #[cfg(test)]
    fn push(&mut self, outcome: FixtureOutcome) {
        self.outcomes.push(outcome);
    }

    /// Iterate collected outcomes.
    pub fn outcomes(&self) -> impl Iterator<Item = &FixtureOutcome> {
        self.outcomes.iter()
    }

    /// True when a provider has exactly passing outcomes for the case id.
    pub fn has_passed(&self, provider: FixtureProvider, case_id: &str) -> bool {
        let mut found_pass = false;
        for outcome in self
            .outcomes
            .iter()
            .filter(|outcome| outcome.provider == provider && outcome.case_id == case_id)
        {
            if outcome.status != FixtureStatus::Passed {
                return false;
            }
            found_pass = true;
        }
        found_pass
    }
}

/// Provider-specific fixture executor.
#[cfg(test)]
#[async_trait]
pub(crate) trait FixtureBackend: Send + Sync {
    /// Provider represented by this backend.
    fn provider(&self) -> FixtureProvider;

    /// Run one fixture case. Store-trait cases should call the referenced
    /// `zbot-stores-conformance` function for the provider's concrete store.
    async fn run_fixture(&self, case: &FixtureCase) -> AdapterResult<()>;
}

/// Runs accepted cases against current SQLite and Engram candidates.
#[cfg(test)]
pub(crate) struct ParityRunner<'a> {
    registry: &'a FixtureRegistry,
}

#[cfg(test)]
impl<'a> ParityRunner<'a> {
    /// Build a runner over an accepted registry.
    pub(crate) fn new(registry: &'a FixtureRegistry) -> Self {
        Self { registry }
    }

    /// Execute every case for a feature against both providers.
    pub(crate) async fn run_feature(
        &self,
        feature: AdapterFeature,
        current: &dyn FixtureBackend,
        engram: &dyn FixtureBackend,
    ) -> AdapterResult<FixtureReport> {
        self.registry.validate_feature_scope(feature)?;
        require_provider(current.provider(), FixtureProvider::CurrentSqlite, feature)?;
        require_provider(engram.provider(), FixtureProvider::EngramCandidate, feature)?;

        let mut report = FixtureReport::default();
        for case in self.registry.cases_for_feature(feature) {
            report.push(run_case(current, case).await);
            report.push(run_case(engram, case).await);
        }
        Ok(report)
    }
}

#[cfg(test)]
async fn run_case(backend: &dyn FixtureBackend, case: &FixtureCase) -> FixtureOutcome {
    match backend.run_fixture(case).await {
        Ok(()) => FixtureOutcome::passed(backend.provider(), case.id),
        Err(err) if err.kind() == AdapterErrorKind::UnsupportedFeature => {
            FixtureOutcome::unsupported(backend.provider(), case.id)
        }
        Err(_) => FixtureOutcome::failed(backend.provider(), case.id),
    }
}

#[cfg(test)]
fn require_provider(
    actual: FixtureProvider,
    expected: FixtureProvider,
    feature: AdapterFeature,
) -> AdapterResult<()> {
    if actual == expected {
        return Ok(());
    }

    Err(AdapterError::UnsupportedFeature {
        feature: feature.as_str(),
        reason: format!("fixture backend provider mismatch: expected {expected:?}, got {actual:?}"),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct MockBackend {
        provider: FixtureProvider,
        status: FixtureStatus,
        seen: Arc<Mutex<Vec<(FixtureProvider, String)>>>,
    }

    impl MockBackend {
        fn new(
            provider: FixtureProvider,
            status: FixtureStatus,
            seen: Arc<Mutex<Vec<(FixtureProvider, String)>>>,
        ) -> Self {
            Self {
                provider,
                status,
                seen,
            }
        }
    }

    #[async_trait]
    impl FixtureBackend for MockBackend {
        fn provider(&self) -> FixtureProvider {
            self.provider
        }

        async fn run_fixture(&self, case: &FixtureCase) -> AdapterResult<()> {
            self.seen
                .lock()
                .expect("seen lock")
                .push((self.provider, case.id().to_string()));

            match self.status {
                FixtureStatus::Passed => Ok(()),
                FixtureStatus::Failed => Err(AdapterError::Bootstrap {
                    component: "fixture",
                    reason: "mock fixture failed".to_string(),
                }),
                FixtureStatus::Unsupported => Err(AdapterError::UnsupportedFeature {
                    feature: case.feature().as_str(),
                    reason: "mock fixture unsupported".to_string(),
                }),
            }
        }
    }

    async fn report_for_feature(
        registry: &FixtureRegistry,
        feature: AdapterFeature,
        current_status: FixtureStatus,
        engram_status: FixtureStatus,
    ) -> FixtureReport {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let current = MockBackend::new(
            FixtureProvider::CurrentSqlite,
            current_status,
            Arc::clone(&seen),
        );
        let engram = MockBackend::new(FixtureProvider::EngramCandidate, engram_status, seen);

        ParityRunner::new(registry)
            .run_feature(feature, &current, &engram)
            .await
            .expect("fixture run")
    }

    #[tokio::test]
    async fn runner_executes_default_scope_registry_against_current_and_engram_candidates() {
        let registry = FixtureRegistry::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let current = MockBackend::new(
            FixtureProvider::CurrentSqlite,
            FixtureStatus::Passed,
            Arc::clone(&seen),
        );
        let engram = MockBackend::new(
            FixtureProvider::EngramCandidate,
            FixtureStatus::Passed,
            Arc::clone(&seen),
        );

        let report = ParityRunner::new(&registry)
            .run_feature(AdapterFeature::MemoryFacts, &current, &engram)
            .await
            .expect("fixture run");

        let expected = registry
            .cases_for_feature(AdapterFeature::MemoryFacts)
            .flat_map(|case| {
                [
                    (FixtureProvider::CurrentSqlite, case.id().to_string()),
                    (FixtureProvider::EngramCandidate, case.id().to_string()),
                ]
            })
            .collect::<Vec<_>>();

        assert_eq!(*seen.lock().expect("seen lock"), expected);
        assert_eq!(report.outcomes().count(), expected.len());
        assert!(report
            .outcomes()
            .all(|outcome| outcome.status() == FixtureStatus::Passed));
    }

    #[tokio::test]
    async fn capability_support_requires_passing_current_and_engram_outcomes() {
        let registry = FixtureRegistry::default();
        let partial = report_for_feature(
            &registry,
            AdapterFeature::MemoryFacts,
            FixtureStatus::Passed,
            FixtureStatus::Unsupported,
        )
        .await;

        let report = crate::CapabilityReport::from_fixture_report(
            &crate::AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db"),
            &registry,
            &partial,
        );

        assert!(!report.supports(AdapterFeature::MemoryFacts));

        let complete = report_for_feature(
            &registry,
            AdapterFeature::MemoryFacts,
            FixtureStatus::Passed,
            FixtureStatus::Passed,
        )
        .await;

        let report = crate::CapabilityReport::from_fixture_report(
            &crate::AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db"),
            &registry,
            &complete,
        );

        assert!(report.supports(AdapterFeature::MemoryFacts));
        assert!(!report.supports(AdapterFeature::Beliefs));
        assert!(!report.supports(AdapterFeature::KnowledgeGraph));
    }

    #[tokio::test]
    async fn failed_fixture_outcome_does_not_count_as_passing() {
        let registry = FixtureRegistry::default();
        let fixture_report = report_for_feature(
            &registry,
            AdapterFeature::MemoryFacts,
            FixtureStatus::Failed,
            FixtureStatus::Passed,
        )
        .await;

        assert!(fixture_report
            .outcomes()
            .any(|outcome| outcome.status() == FixtureStatus::Failed));
        assert!(
            !fixture_report.has_passed(FixtureProvider::CurrentSqlite, MEMORY_SAVE_AND_COUNT_ID)
        );

        let report = crate::CapabilityReport::from_fixture_report(
            &crate::AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db"),
            &registry,
            &fixture_report,
        );
        assert!(!report.supports(AdapterFeature::MemoryFacts));
    }

    #[tokio::test]
    async fn current_sqlite_report_ignores_fixture_support_rows() {
        let registry = FixtureRegistry::default();
        let complete = report_for_feature(
            &registry,
            AdapterFeature::MemoryFacts,
            FixtureStatus::Passed,
            FixtureStatus::Passed,
        )
        .await;

        let report = crate::CapabilityReport::from_fixture_report(
            &crate::AdapterConfig::default(),
            &registry,
            &complete,
        );

        assert!(report.capabilities.is_empty());
    }

    #[test]
    fn registry_rejects_missing_scope_coverage_before_support() {
        const INCOMPLETE_SCOPE: ScopeFixture =
            ScopeFixture::new(ScopeDimension::Tenant, ScopeExpectation::Positive);
        let registry = FixtureRegistry::new([
            FixtureCase::store_trait(
                MEMORY_SAVE_AND_COUNT_ID,
                AdapterFeature::MemoryFacts,
                "memory_save_and_count",
            ),
            FixtureCase::scope(
                "memory.scope.tenant.positive",
                AdapterFeature::MemoryFacts,
                INCOMPLETE_SCOPE,
            ),
        ]);

        let err = registry
            .validate_feature_scope(AdapterFeature::MemoryFacts)
            .expect_err("scope coverage must be complete");
        assert_eq!(err.kind(), AdapterErrorKind::UnsupportedFeature);

        assert!(!registry.feature_ready(AdapterFeature::MemoryFacts, &FixtureReport::default()));
    }

    #[test]
    fn registry_rejects_scope_only_feature_before_support() {
        let registry = FixtureRegistry::new(REQUIRED_SCOPE_FIXTURES.iter().enumerate().map(
            |(index, scope)| {
                FixtureCase::scope(
                    match index {
                        0 => "memory.scope.tenant.positive",
                        1 => "memory.scope.tenant.negative",
                        2 => "memory.scope.ward.positive",
                        3 => "memory.scope.ward.negative",
                        4 => "memory.scope.session.positive",
                        5 => "memory.scope.session.negative",
                        6 => "memory.scope.partition.positive",
                        _ => "memory.scope.partition.negative",
                    },
                    AdapterFeature::MemoryFacts,
                    *scope,
                )
            },
        ));

        let err = registry
            .validate_feature_scope(AdapterFeature::MemoryFacts)
            .expect_err("store-trait conformance fixture is required");
        assert_eq!(err.kind(), AdapterErrorKind::UnsupportedFeature);
    }

    #[test]
    fn default_registry_tracks_required_scope_coverage() {
        let registry = FixtureRegistry::default();

        for feature in [AdapterFeature::MemoryFacts, AdapterFeature::Beliefs] {
            registry
                .validate_feature_scope(feature)
                .expect("default registry has complete scope coverage");
        }

        assert_eq!(REQUIRED_SCOPE_FIXTURES.len(), 8);
        assert_eq!(
            registry
                .cases_for_feature(AdapterFeature::MemoryFacts)
                .filter_map(|case| case.scope_fixture())
                .count(),
            REQUIRED_SCOPE_FIXTURES.len()
        );
        assert_eq!(
            registry
                .cases_for_feature(AdapterFeature::Beliefs)
                .filter_map(|case| case.scope_fixture())
                .count(),
            REQUIRED_SCOPE_FIXTURES.len()
        );
    }

    #[test]
    fn default_registry_tracks_conformance_seed_cases() {
        let registry = FixtureRegistry::default();

        for seed in zbot_stores_conformance::parity::seed_store_trait_cases() {
            let case = registry
                .cases()
                .find(|case| case.id() == seed.id)
                .unwrap_or_else(|| {
                    panic!(
                        "adapter registry must track conformance seed case {}",
                        seed.id
                    )
                });

            let FixtureCaseKind::StoreTrait(store_trait) = case.kind() else {
                panic!(
                    "conformance seed case {} must be store-trait backed",
                    seed.id
                );
            };
            assert_eq!(store_trait.function, seed.function);
        }

        assert!(registry.contains_case(MEMORY_SAVE_AND_COUNT_ID));
        assert!(registry.contains_case(BELIEF_UPSERT_GET_ID));
    }
}
