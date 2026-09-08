// ============================================================================
// TOOL MODULES
// ============================================================================

mod connectors;
mod execution;
mod file;
mod goal;
mod graph_query;
pub mod guards;
mod ingest;
mod memory;
mod multimodal;
mod recall;
mod search;
mod ward;

use serde::{Deserialize, Serialize};

pub use connectors::{ConnectorInvokeTool, ConnectorResourceTool, QueryResourceTool};
pub use execution::EditFileTool;
pub use execution::ShellTool;
pub use execution::UpdatePlanTool;
pub use execution::WriteFileTool;
pub use execution::skills::LoadSkillTool;
pub use file::ReadTool;
// graph_query types are public API for downstream crates (e.g., pi-mono wiring)
#[allow(unused_imports)]
pub use graph_query::{EntityInfo, GraphQueryTool, GraphStorageAccess, NeighborInfo};
// goal types are public API for downstream crates (gateway wiring)
#[allow(unused_imports)]
pub use goal::{GoalAccess, GoalSummary, GoalTool};
// ingest types are public API for downstream crates (gateway wiring)
#[allow(unused_imports)]
pub use ingest::{
    EvidenceRecord, IngestTool, IngestionAccess, StructuredCounts, StructuredEntity,
    StructuredRelationship,
};
pub use memory::{MemoryEntry, MemorySearchTool, MemoryStore, MemoryTool, MemoryWriteTool};
pub use multimodal::MultimodalAnalyzeTool;
pub use recall::{
    RecallAuthorizationAccess, RecallAuthorizationContext, RecallContentVisibility, RecallFailure,
    RecallItemKind, RecallLogicalSource, RecallMode, RecallOutputPolicy, RecallProvenance,
    RecallReasonCode, RecallSourceState, RecallSourceStatus, RecallSourceSummary,
    RecallTaxonomyCandidate, RecallTaxonomyExpansion, RecallTool, RecallVisibilityScope,
    TaxonomyRelation, UnifiedRecallAccess, UnifiedRecallBinding, UnifiedRecallItem,
    UnifiedRecallRequest, UnifiedRecallResponse, recall_parameters_schema,
};
pub use search::GlobTool;
pub use ward::{
    WardAudience, WardLayoutAccess, WardLayoutState, WardTool, WardUsageAccess, ensure_ward_catalog,
};

// ============================================================================
// TOOL SETTINGS
// ============================================================================

/// Settings that affect live gateway tool behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSettings {
    /// Enable optional file-discovery tools such as glob in the live gateway
    /// registry. Read/write/edit are registered separately.
    #[serde(default)]
    pub file_tools: bool,

    /// Offload large tool results to filesystem instead of keeping in context.
    /// When a tool result exceeds the token threshold, it's saved to a temp file
    /// and the agent is instructed to read it with a CLI tool.
    #[serde(default = "default_offload_enabled")]
    pub offload_large_results: bool,

    /// Token threshold for offloading tool results (default: 5000 tokens ≈ 20000 chars).
    /// Results larger than this are saved to filesystem.
    #[serde(default = "default_offload_threshold")]
    pub offload_threshold_tokens: usize,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            file_tools: false,
            offload_large_results: default_offload_enabled(),
            offload_threshold_tokens: default_offload_threshold(),
        }
    }
}

fn default_offload_threshold() -> usize {
    5000 // ~20000 characters
}

fn default_offload_enabled() -> bool {
    true // Enabled by default to prevent context explosion
}
