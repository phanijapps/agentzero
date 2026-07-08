//! AgentZero compatibility layer for Engram-backed memory stores.
//!
//! This crate is intentionally an adapter boundary. AgentZero keeps owning
//! gateway routes, UI DTOs, settings, and sleep-cycle scheduling; Engram stays a
//! contract-first Rust library behind focused mapping and store modules.

pub mod bootstrap;
pub mod capabilities;
pub mod config;
pub mod dependency_checklist;
pub mod error;
pub mod fixtures;
pub mod governance;
pub mod mapping;
pub mod migration;
pub mod recall;
pub mod scope;
pub mod stores;

pub use bootstrap::EngramProvider;
pub use capabilities::{AdapterFeature, Capability, CapabilityReport, CapabilityStatus};
pub use config::{
    AdapterConfig, AdapterEmbeddingProviderConfig, AdapterSqliteStorageLayout, EmbeddingMode,
    MigrationMode, ProviderMode, ResolvedEngramPath, ResolvedGovernanceDefinitionPaths,
};
pub use dependency_checklist::{
    DependencyChecklist, DependencyChecklistItem, DependencyEvidence, DependencyItemStatus,
};
pub use error::{AdapterError, AdapterErrorKind, AdapterResult};
pub use fixtures::{
    FixtureCase, FixtureCaseKind, FixtureOutcome, FixtureProvider, FixtureRegistry, FixtureReport,
    FixtureStatus, ScopeDimension, ScopeExpectation, ScopeFixture, StoreTraitFixture,
};
pub use governance::{
    AllowUnclassifiedPolicy, GovernanceOverlay, GovernancePolicy, GovernanceScope,
    GovernanceSelection, SkosExpansionPolicy, ValidationMode,
};
pub use migration::{
    apply_migration, run_migration_dry_run, MigrationApplyReceipt, MigrationBlocker,
    MigrationDiagnostic, MigrationDiagnosticSeverity, MigrationDryRunReport, MigrationInput,
    MigrationManifest, MigrationSource, MigrationSourceKind, MigrationSourceReport,
};
pub use recall::{RecallBlocker, RecallParityArtifact, RecallSupportReport};
pub use scope::{ScopeMapper, ScopeTarget};
pub use stores::beliefs::EngramBeliefStore;
pub use stores::knowledge_graph::EngramKnowledgeGraphStore;
pub use stores::memory_facts::EngramMemoryFactStore;
pub use stores::sidecars::EngramSidecarStores;
pub use stores::wiki::EngramWikiStore;
