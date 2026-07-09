// ============================================================================
// APP-TOOLS - Built-in Tools for z-Bot Application
// ============================================================================

//! # App Tools
//!
//! Built-in tool implementations for the z-Bot application.
//!
//! This crate provides concrete tool implementations that use
//! the abstractions defined in agent-primitives.

pub mod replay;
mod tools;

/// Re-exported guard predicates so other crates (gateway-execution
/// bootstrap) share a single source of truth for ward-state checks.
pub use tools::guards;

pub use tools::{
    EditFileTool,
    // Knowledge graph query types
    EntityInfo,
    // Ingestion tool (enqueue text for background extraction + bulk structured)
    EvidenceRecord,
    GlobTool,
    // Goal tool (agent intent lifecycle)
    GoalAccess,
    GoalSummary,
    GoalTool,
    GraphQueryTool,
    GraphStorageAccess,
    IngestTool,
    IngestionAccess,
    ListAgentsTool,
    LoadSkillTool,
    MemoryEntry,
    MemoryStore,
    MemoryTool,
    MemoryWriteTool,
    // Multimodal vision fallback
    MultimodalAnalyzeTool,
    NeighborInfo,
    QueryResourceTool,
    ReadTool,
    // Individual tools for lean subagent registries
    ShellTool,
    StructuredCounts,
    StructuredEntity,
    StructuredRelationship,
    ToolSettings,
    UpdatePlanTool,
    // Orchestrator tools
    WardTool,
    // Ward-curator observer trait (see gateway/gateway-execution/.../ward_usage_adapter.rs)
    WardUsageAccess,
    WriteFileTool,
    builtin_tools_with_fs,
    core_tools,
    optional_tools,
};

// Re-export from agent-primitives
pub use agent_primitives::{FileSystemContext, Tool};
