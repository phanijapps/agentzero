// ============================================================================
// TOOL MODULES
// ============================================================================

mod agent;
mod connectors;
mod execution;
mod file;
mod goal;
mod graph_query;
pub mod guards;
mod ingest;
mod memory;
mod multimodal;
mod search;
mod ui;
mod ward;
mod web;

use std::sync::Arc;

use agent_primitives::FileSystemContext;
use agent_primitives::Tool;
use serde::{Deserialize, Serialize};
use zbot_stores_traits::MemoryFactStore;

pub use agent::{CreateAgentTool, ListAgentsTool};
pub use connectors::QueryResourceTool;
pub use execution::EditFileTool;
pub use execution::ExecutionGraphTool;
pub use execution::PythonTool;
pub use execution::ShellTool;
pub use execution::UpdatePlanTool;
pub use execution::WriteFileTool;
pub use execution::skills::LoadSkillTool;
pub use file::{EditTool, ReadTool, WriteTool};
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
pub use memory::{MemoryEntry, MemoryStore, MemoryTool, MemoryWriteTool};
pub use multimodal::MultimodalAnalyzeTool;
pub use search::GlobTool;
pub use ui::{RequestInputTool, ShowContentTool};
pub use ward::{WardTool, WardUsageAccess};
pub use web::WebFetchTool;

// ============================================================================
// TOOL SETTINGS
// ============================================================================

/// Settings for optional tools.
///
/// These settings control which optional tools are enabled beyond the core set.
///
/// Core tools (always enabled):
/// - shell: Primary execution — commands
/// - read: Read files by path with optional line limits
/// - write_file: Create new files
/// - edit_file: Targeted find-and-replace edits on existing files
/// - memory: Persist/recall information
/// - ward: Project directory management
/// - update_plan: Lightweight task checklist
/// - load_skill: Bounded skill packet loading by explicit skill name
///
/// Note: respond, delegate_to_agent, and list_agents are registered separately
/// in the runner as action tools.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolSettings {
    /// Enable python tool (run Python scripts)
    #[serde(default)]
    pub python: bool,

    /// Enable web_fetch tool (HTTP requests).
    /// Disabled by default as large responses can cause context explosion.
    #[serde(default)]
    pub web_fetch: bool,

    /// Enable UI tools (request_input, show_content)
    #[serde(default)]
    pub ui_tools: bool,

    /// Enable create_agent tool
    #[serde(default)]
    pub create_agent: bool,

    /// Enable legacy file tools (write, edit, glob) as separate tools.
    /// `read` is a core safe tool. When false (default), the model uses the
    /// core `read` / `write_file` / `edit_file` tools instead. These optional
    /// tools layer on extra capabilities (legacy write/edit names, glob
    /// patterns).
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

fn default_offload_threshold() -> usize {
    5000 // ~20000 characters
}

fn default_offload_enabled() -> bool {
    true // Enabled by default to prevent context explosion
}

// ============================================================================
// BUILT-IN TOOLS FACTORY
// ============================================================================

/// Get core tools — the minimal, high-signal set.
///
/// Shell for commands; read / write_file / edit_file for file operations.
/// Legacy file aliases and glob are optional.
///
/// Core tools:
/// - shell: Primary execution — commands
/// - read: Read file contents with optional line limits
/// - write_file: Create new files
/// - edit_file: Find-and-replace edits on existing files
/// - memory: Persistent KV store
/// - ward: Project directory management
/// - update_plan: Lightweight task checklist
/// - load_skill: Bounded skill packet loading by explicit skill name
#[must_use]
pub fn core_tools(
    fs: Arc<dyn FileSystemContext>,
    fact_store: Option<Arc<dyn MemoryFactStore>>,
    ward_usage: Option<Arc<dyn WardUsageAccess>>,
) -> Vec<Arc<dyn Tool>> {
    vec![
        // Primary execution tool
        Arc::new(ShellTool::new()),
        Arc::new(ReadTool::new(fs.clone())),
        // File operations
        Arc::new(WriteFileTool::new(fs.clone())),
        Arc::new(EditFileTool::new(fs.clone())),
        // Persistent memory (with optional DB-backed fact store)
        Arc::new(MemoryTool::new(fs.clone(), fact_store.clone())),
        // Ward management (named project directories, with recall on entry)
        Arc::new(WardTool::new(fs.clone(), fact_store, ward_usage)),
        // Lightweight plan tracking
        Arc::new(UpdatePlanTool::new()),
        // DAG workflow engine for multi-step orchestration
        Arc::new(ExecutionGraphTool::new()),
        // Skill packet loading
        Arc::new(LoadSkillTool::new(fs.clone())),
    ]
}

/// Get optional tools based on settings.
///
/// Includes legacy file tools (write/edit/glob), python, web_fetch, etc.
#[must_use]
pub fn optional_tools(
    fs: Arc<dyn FileSystemContext>,
    settings: &ToolSettings,
) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    // File tools — separate legacy write/edit names and glob (opt-in).
    // `read` is core.
    if settings.file_tools {
        tools.push(Arc::new(WriteTool::new(fs.clone())));
        tools.push(Arc::new(EditTool::new(fs.clone())));
        tools.push(Arc::new(GlobTool));
    }

    if settings.python {
        tools.push(Arc::new(PythonTool::new(fs.clone())));
    }

    if settings.web_fetch {
        tools.push(Arc::new(WebFetchTool::new()));
    }

    if settings.ui_tools {
        tools.push(Arc::new(RequestInputTool));
        tools.push(Arc::new(ShowContentTool));
    }

    if settings.create_agent {
        tools.push(Arc::new(CreateAgentTool::new(fs.clone())));
    }

    // Multimodal analysis — always available as a vision fallback
    tools.push(Arc::new(multimodal::MultimodalAnalyzeTool::new()));

    tools
}

/// Get all built-in tools with a file system context.
///
/// This is the legacy function that returns all tools.
/// For new code, prefer using `core_tools()` + `optional_tools()`.
#[must_use]
pub fn builtin_tools_with_fs(fs: Arc<dyn FileSystemContext>) -> Vec<Arc<dyn Tool>> {
    // Return all tools (core + all optional enabled)
    let all_enabled = ToolSettings {
        python: true,
        web_fetch: true,
        ui_tools: true,
        create_agent: true,
        file_tools: true,
        offload_large_results: false, // Not relevant for this legacy function
        offload_threshold_tokens: default_offload_threshold(),
    };

    let mut tools = core_tools(fs.clone(), None, None);
    tools.extend(optional_tools(fs, &all_enabled));
    tools
}
