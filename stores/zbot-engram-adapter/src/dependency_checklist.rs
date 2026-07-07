//! Auditable dependency gate for Engram provider selection.

use serde::{Deserialize, Serialize};

/// Checklist item status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyItemStatus {
    /// Public Engram capability is implemented and used by the adapter.
    Implemented,
    /// Product-specific behavior is intentionally isolated in the adapter.
    Sidecar,
    /// A human-approved waiver exists.
    Waived,
    /// Required capability or evidence is still missing.
    Missing,
}

impl DependencyItemStatus {
    fn blocks_provider_selection(self) -> bool {
        matches!(self, Self::Missing)
    }
}

/// One upstream dependency/capability row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyChecklistItem {
    /// Stable row id.
    pub id: String,
    /// Current status.
    pub status: DependencyItemStatus,
    /// Path-free evidence note.
    pub evidence: String,
    /// Whether provider selection requires this row to be non-missing.
    pub required_for_provider_selection: bool,
}

impl DependencyChecklistItem {
    /// Build one checklist row.
    pub fn new(
        id: impl Into<String>,
        status: DependencyItemStatus,
        evidence: impl Into<String>,
        required_for_provider_selection: bool,
    ) -> Self {
        Self {
            id: id.into(),
            status,
            evidence: evidence.into(),
            required_for_provider_selection,
        }
    }
}

/// Build/provenance evidence needed before provider selection or apply mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEvidence {
    /// `cargo metadata --locked` was captured.
    pub cargo_metadata: bool,
    /// `Cargo.lock` contains the Engram dependency set.
    pub lockfile: bool,
    /// Dependency scanner evidence was recorded.
    pub dependency_scanner: bool,
    /// Exact Engram source revision/provenance, path-free.
    pub source_revision: Option<String>,
    /// Dirty-state policy for the Engram source.
    pub dirty_state_policy: Option<String>,
}

impl DependencyEvidence {
    fn blockers(&self) -> Vec<String> {
        let mut blockers = Vec::new();
        if !self.cargo_metadata {
            blockers.push("cargo_metadata".to_string());
        }
        if !self.lockfile {
            blockers.push("lockfile".to_string());
        }
        if !self.dependency_scanner {
            blockers.push("dependency_scanner".to_string());
        }
        if self
            .source_revision
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            blockers.push("source_revision".to_string());
        }
        if self
            .dirty_state_policy
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            blockers.push("dirty_state_policy".to_string());
        }
        blockers
    }
}

/// Provider-selection dependency checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyChecklist {
    /// Required and informational rows.
    pub items: Vec<DependencyChecklistItem>,
    /// Build/provenance evidence.
    pub evidence: DependencyEvidence,
}

impl DependencyChecklist {
    /// Build the current Engram cutover checklist.
    pub fn current() -> Self {
        Self {
            items: vec![
                DependencyChecklistItem::new(
                    "provider_facade",
                    DependencyItemStatus::Implemented,
                    "adapter opens Engram through EngramConfig/bootstrap_provider",
                    true,
                ),
                DependencyChecklistItem::new(
                    "sqlite_open_options",
                    DependencyItemStatus::Implemented,
                    "confined storage path maps to upstream SQLite open options",
                    true,
                ),
                DependencyChecklistItem::new(
                    "scope_mapping",
                    DependencyItemStatus::Implemented,
                    "tenant, ward, session, and partition scope fixtures exist",
                    true,
                ),
                DependencyChecklistItem::new(
                    "memory_repository",
                    DependencyItemStatus::Implemented,
                    "memory facts map through Engram memory records plus sidecar parity",
                    true,
                ),
                DependencyChecklistItem::new(
                    "knowledge_repository",
                    DependencyItemStatus::Implemented,
                    "wiki, graph, and hierarchy writes use Engram knowledge/hierarchy ports",
                    true,
                ),
                DependencyChecklistItem::new(
                    "belief_repository",
                    DependencyItemStatus::Implemented,
                    "belief and contradiction lifecycle uses Engram belief ports",
                    true,
                ),
                DependencyChecklistItem::new(
                    "adapter_sidecars",
                    DependencyItemStatus::Sidecar,
                    "zbot-only procedures, episodes, recall logs, goals, outbox, and compaction audit remain adapter-owned",
                    true,
                ),
                DependencyChecklistItem::new(
                    "retrieval_ranking_trace",
                    DependencyItemStatus::Missing,
                    "upstream retrieval/ranking trace port is not yet available; recall stays unsupported",
                    false,
                ),
                DependencyChecklistItem::new(
                    "migration_manifest_gate",
                    DependencyItemStatus::Implemented,
                    "dry-run manifest fingerprint gates apply",
                    true,
                ),
            ],
            evidence: DependencyEvidence::default(),
        }
    }

    /// Return all blocker identifiers for provider selection.
    pub fn provider_selection_blockers(&self) -> Vec<String> {
        let mut blockers = self
            .items
            .iter()
            .filter(|item| {
                item.required_for_provider_selection && item.status.blocks_provider_selection()
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        blockers.extend(
            self.evidence
                .blockers()
                .into_iter()
                .map(|id| format!("evidence.{id}")),
        );
        blockers
    }

    /// True only when provider selection has auditable dependency evidence.
    pub fn provider_selection_ready(&self) -> bool {
        self.provider_selection_blockers().is_empty()
    }
}
