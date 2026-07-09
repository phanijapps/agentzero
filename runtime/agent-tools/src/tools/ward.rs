// ============================================================================
// WARD TOOL
// Agent-managed project containers (named directories)
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, FileSystemContext, Result, Tool, ToolContext, ToolPermissions};
use zbot_stores_traits::MemoryFactStore;

/// AGENTS.md file name - living readme for agent executions
const WARD_AGENTS_MD: &str = "AGENTS.md";

/// Observer for ward-tool creation events. Implemented gateway-side so the
/// ward curator's telemetry sidecar gets a `created_by = "agent"` mark
/// whenever an agent scaffolds a new ward through this tool.
///
/// The trait is async (mirrors [`IngestionAccess`] / [`GoalAccess`]) even
/// though current implementations only do a brief synchronous bump — keeps
/// the surface uniform if future observers wire IO.
#[async_trait]
pub trait WardUsageAccess: Send + Sync + 'static {
    /// Called immediately after the ward tool creates a new ward directory,
    /// before the tool returns. Implementations should not panic — telemetry
    /// is best-effort.
    async fn mark_created_agent(&self, ward: &str);
}

/// Tool for managing wards (named project directories).
///
/// Wards are persistent, agent-named project directories under `vault/wards/`.
/// The agent autonomously creates and switches between wards.
///
/// Actions:
/// - `use`: Switch to a ward (creates if needed), returns file listing
/// - `create`: Alias for `use` (semantically clearer for new wards)
/// - `list`: List all wards with descriptions
/// - `info`: Detailed info about a specific ward
pub struct WardTool {
    fs: Arc<dyn FileSystemContext>,
    fact_store: Option<Arc<dyn MemoryFactStore>>,
    /// Optional observer that gets a `created_by = "agent"` mark whenever the
    /// `use`/`create` action scaffolds a new ward directory. `None` is a
    /// valid no-op (tests, minimal configurations).
    ward_usage: Option<Arc<dyn WardUsageAccess>>,
}

impl WardTool {
    /// Create a new WardTool with file system context, optional fact store,
    /// and optional ward-usage observer.
    #[must_use]
    pub fn new(
        fs: Arc<dyn FileSystemContext>,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        ward_usage: Option<Arc<dyn WardUsageAccess>>,
    ) -> Self {
        Self {
            fs,
            fact_store,
            ward_usage,
        }
    }

    /// List files in a ward directory (non-recursive, top-level only).
    fn list_ward_files(&self, ward_dir: &std::path::Path) -> Vec<String> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(ward_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }
                if entry.path().is_dir() {
                    files.push(format!("{}/", name));
                } else {
                    files.push(name);
                }
            }
        }
        files.sort();
        files
    }

    /// Read AGENTS.md content from a ward directory, if it exists.
    fn read_agents_md(&self, ward_dir: &std::path::Path) -> Option<String> {
        let agents_md_path = ward_dir.join(WARD_AGENTS_MD);
        std::fs::read_to_string(&agents_md_path).ok()
    }

    /// Write a minimal AGENTS.md seed for a new ward.
    ///
    /// The seed is intentionally just the ward name as an H1 heading — the
    /// agent curates all other content during sessions. We never overwrite an
    /// existing AGENTS.md.
    fn write_agents_md(ward_dir: &std::path::Path, ward_name: &str) {
        let agents_md_path = ward_dir.join(WARD_AGENTS_MD);
        if agents_md_path.exists() {
            return;
        }

        let content = format!("# {}\n", ward_name);

        if let Err(e) = std::fs::write(&agents_md_path, content) {
            tracing::warn!("Failed to create AGENTS.md in ward '{}': {}", ward_name, e);
        }
    }

    /// Create the empty memory-bank scaffold (directory + three zero-byte files).
    /// The agent owns the contents. No other directories are pre-created — the
    /// agent picks language-appropriate names for reusable-primitive locations
    /// (`core/`, `pkg/`, `lib/`, `src/`, etc.).
    fn scaffold_empty_dirs(ward_dir: &std::path::Path, ward_name: &str) {
        let memory_bank = ward_dir.join("memory-bank");
        if let Err(e) = std::fs::create_dir_all(&memory_bank) {
            tracing::warn!(
                "Failed to create memory-bank dir in ward '{}': {}",
                ward_name,
                e
            );
            return;
        }

        for file in ["ward.md", "structure.md", "core_docs.md"] {
            let path = memory_bank.join(file);
            if !path.exists()
                && let Err(e) = std::fs::write(&path, "")
            {
                tracing::warn!(
                    "Failed to create empty memory-bank/{} in ward '{}': {}",
                    file,
                    ward_name,
                    e
                );
            }
        }
    }

    /// Recall facts relevant to the ward being entered.
    ///
    /// Best-effort: if no fact store is configured, or the recall fails,
    /// returns None and the ward switch still succeeds.
    async fn recall_ward_facts(
        &self,
        ward_name: &str,
        ctx: &Arc<dyn ToolContext>,
    ) -> Option<Value> {
        let store = self.fact_store.as_ref()?;

        let agent_id = ctx
            .get_state("app:agent_id")
            .and_then(|v| v.as_str().map(String::from))
            .or_else(|| {
                ctx.get_state("app:root_agent_id")
                    .and_then(|v| v.as_str().map(String::from))
            })?;

        let query = format!("ward {} context patterns corrections", ward_name);
        match store
            .recall_facts_prioritized(&agent_id, &query, 5, None)
            .await
        {
            Ok(result) => {
                let count = result.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
                if count > 0 {
                    tracing::info!(
                        "Ward-entry recall for '{}': {} facts loaded",
                        ward_name,
                        count
                    );
                    Some(result)
                } else {
                    None
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Ward-entry recall failed for '{}': {} (non-fatal)",
                    ward_name,
                    e
                );
                None
            }
        }
    }

    /// Get a short description from AGENTS.md purpose section (if any).
    fn ward_description(&self, ward_dir: &std::path::Path) -> Option<String> {
        let content = self.read_agents_md(ward_dir)?;
        // Look for Purpose section and extract first non-empty, non-comment line
        let mut in_purpose = false;
        for line in content.lines() {
            if line.starts_with("## Purpose") {
                in_purpose = true;
                continue;
            }
            if in_purpose {
                if line.starts_with("## ") {
                    break; // Next section
                }
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with("<!--") {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    }
}

#[async_trait]
impl Tool for WardTool {
    fn name(&self) -> &str {
        "ward"
    }

    fn description(&self) -> &str {
        "Manage code wards (named project directories). Wards persist across sessions.\n\
         Arguments: `action` (required, one of use|create|list|info) and `name` (string).\n\
         No other fields are accepted — do not pass `title`, `label`, or `description`.\n\
         Actions:\n\
         - use: Switch to a ward (creates if needed). Sets working directory for shell/write/edit.\n\
         - create: Alias for use. Creates and switches to a new ward.\n\
         - list: List all wards with descriptions.\n\
         - info: Detailed info about a specific ward."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["use", "create", "list", "info"],
                    "description": "The ward operation to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Ward name (required for use, create, info). Use concise, descriptive names."
                }
            },
            "required": ["action"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Check for error markers from truncated/malformed tool calls
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AgentError::Tool(format!("{}: {}", error_type, message)));
        }

        let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| {
            AgentError::Tool(
                "ward: missing 'action' parameter (one of: use, create, list, info)".to_string(),
            )
        })?;

        let wards_root = self
            .fs
            .wards_root_dir()
            .ok_or_else(|| AgentError::Tool("Wards directory not configured".to_string()))?;

        match action {
            "use" | "create" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    AgentError::Tool("Missing 'name' parameter for use/create".to_string())
                })?;

                // Validate ward name: alphanumeric, hyphens, underscores only
                if !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    return Err(AgentError::Tool(
                        "Ward name must contain only letters, numbers, hyphens, and underscores"
                            .to_string(),
                    ));
                }

                if name.is_empty() || name.len() > 64 {
                    return Err(AgentError::Tool(
                        "Ward name must be 1-64 characters".to_string(),
                    ));
                }

                let ward_dir = wards_root.join(name);
                let created = !ward_dir.exists();

                // Subagents cannot create wards — only root can
                let is_delegated = ctx
                    .get_state("app:is_delegated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if created && is_delegated {
                    return Err(AgentError::Tool(format!(
                        "Subagents cannot create wards. Use the ward specified in your task: '{}' does not exist. \
                         Ask the root agent to create it first.",
                        name
                    )));
                }

                // Create ward directory if needed
                if created {
                    std::fs::create_dir_all(&ward_dir).map_err(|e| {
                        AgentError::Tool(format!("Failed to create ward directory: {}", e))
                    })?;
                }

                // Create ward scaffold for new wards. The seed is intentionally
                // minimal — just the AGENTS.md heading and empty memory-bank +
                // core scaffolds. The agent curates all content itself.
                if created {
                    Self::write_agents_md(&ward_dir, name);
                    Self::scaffold_empty_dirs(&ward_dir, name);
                    let _ = std::fs::create_dir_all(ward_dir.join("specs"));

                    // Tell the ward-curator telemetry sidecar that this ward
                    // is agent-authored. Curator-eligible wards must have
                    // `created_by = "agent"`; without this hook every
                    // scaffolded ward lazy-inserts as `user` and is skipped.
                    if let Some(observer) = self.ward_usage.as_ref() {
                        observer.mark_created_agent(name).await;
                    }
                }

                // Set ward_id in context state
                ctx.set_state("ward_id".to_string(), json!(name));

                // List files in the ward
                let files = self.list_ward_files(&ward_dir);

                // Read AGENTS.md if it exists
                let agents_md = self.read_agents_md(&ward_dir);

                tracing::info!("Ward switched to '{}' (created: {})", name, created);

                // Best-effort recall of ward-scoped knowledge
                let ward_knowledge = self.recall_ward_facts(name, &ctx).await;

                // Return result with __ward_changed__ marker for the executor
                let mut result = json!({
                    "__ward_changed__": true,
                    "ward_id": name,
                    "action": if created { "created" } else { "switched" },
                    "files": files,
                    "file_count": files.len(),
                    "agents_md": agents_md,
                });

                if let Some(knowledge) = ward_knowledge {
                    result["ward_knowledge"] = knowledge;
                }

                // Nudge the agent to recall ward-specific knowledge
                result["recall_nudge"] = json!(format!(
                    "[Recall] You entered ward '{}'. Use the memory tool to recall ward-specific knowledge before proceeding.",
                    name
                ));

                Ok(result)
            }

            "list" => {
                let mut wards = Vec::new();

                if wards_root.exists()
                    && let Ok(entries) = std::fs::read_dir(&wards_root)
                {
                    for entry in entries.flatten() {
                        if entry.path().is_dir() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            // Skip hidden directories (.venv, .node_env)
                            if name.starts_with('.') {
                                continue;
                            }
                            let files = self.list_ward_files(&entry.path());
                            let description = self.ward_description(&entry.path());
                            wards.push(json!({
                                "name": name,
                                "files": files.len(),
                                "description": description,
                            }));
                        }
                    }
                }

                wards.sort_by(|a, b| {
                    a.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
                });

                Ok(json!({
                    "wards": wards,
                    "total": wards.len(),
                }))
            }

            "info" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    AgentError::Tool("Missing 'name' parameter for info".to_string())
                })?;

                let ward_dir = wards_root.join(name);
                if !ward_dir.exists() {
                    return Ok(json!({
                        "found": false,
                        "name": name,
                        "message": "Ward not found",
                    }));
                }

                let files = self.list_ward_files(&ward_dir);
                let agents_md = self.read_agents_md(&ward_dir);

                Ok(json!({
                    "found": true,
                    "name": name,
                    "files": files,
                    "file_count": files.len(),
                    "agents_md": agents_md,
                }))
            }

            _ => Err(AgentError::Tool(format!("Unknown ward action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct TestFs {
        base: PathBuf,
    }

    impl FileSystemContext for TestFs {
        fn conversation_dir(&self, _id: &str) -> Option<PathBuf> {
            None
        }
        fn outputs_dir(&self) -> Option<PathBuf> {
            None
        }
        fn skills_dir(&self) -> Option<PathBuf> {
            None
        }
        fn agents_dir(&self) -> Option<PathBuf> {
            None
        }
        fn agent_data_dir(&self, _id: &str) -> Option<PathBuf> {
            None
        }
        fn python_executable(&self) -> Option<PathBuf> {
            None
        }
        fn vault_path(&self) -> Option<PathBuf> {
            Some(self.base.clone())
        }
    }

    #[test]
    fn test_list_ward_files_empty() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None);
        let ward_dir = dir.path().join("wards").join("test");
        std::fs::create_dir_all(&ward_dir).unwrap();

        let files = tool.list_ward_files(&ward_dir);
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_ward_files_with_content() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None);
        let ward_dir = dir.path().join("wards").join("test");
        std::fs::create_dir_all(&ward_dir).unwrap();

        // Create some files
        std::fs::write(ward_dir.join("app.js"), "console.log('hi')").unwrap();
        std::fs::write(ward_dir.join("readme.md"), "# Test").unwrap();
        std::fs::create_dir(ward_dir.join("src")).unwrap();

        // Create hidden file (should be excluded)
        std::fs::write(ward_dir.join(".hidden_file"), "{}").unwrap();

        let files = tool.list_ward_files(&ward_dir);
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"app.js".to_string()));
        assert!(files.contains(&"readme.md".to_string()));
        assert!(files.contains(&"src/".to_string()));
    }

    #[test]
    fn test_ward_description_from_agents_md() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None);

        std::fs::write(
            dir.path().join("AGENTS.md"),
            "# My Project\n\n## Purpose\nStock tracker using yfinance\n\n## Structure\n",
        )
        .unwrap();

        let desc = tool.ward_description(dir.path());
        assert_eq!(desc, Some("Stock tracker using yfinance".to_string()));
    }

    #[test]
    fn test_ward_description_missing() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None);

        let desc = tool.ward_description(dir.path());
        assert!(desc.is_none());
    }

    #[test]
    fn test_write_agents_md_minimal_seed() {
        let ward_dir = TempDir::new().unwrap();
        let ward_path = ward_dir.path().to_path_buf();

        WardTool::write_agents_md(&ward_path, "test-project");

        let content = std::fs::read_to_string(ward_path.join("AGENTS.md")).unwrap();
        // Heading is the only non-whitespace content
        let non_ws_lines: Vec<&str> = content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(non_ws_lines, vec!["# test-project"]);
        // No opinionated sections
        assert!(!content.contains("## Purpose"));
        assert!(!content.contains("## Directory Layout"));
        assert!(!content.contains("## Core Modules"));
        assert!(!content.contains("## History"));
    }

    #[test]
    fn test_ward_create_scaffolds_empty_memory_bank_files() {
        let ward_dir = TempDir::new().unwrap();
        let ward_path = ward_dir.path().to_path_buf();

        WardTool::scaffold_empty_dirs(&ward_path, "minimal");

        for file in ["ward.md", "structure.md", "core_docs.md"] {
            let path = ward_path.join("memory-bank").join(file);
            assert!(path.exists(), "memory-bank/{} should exist", file);
            let meta = std::fs::metadata(&path).unwrap();
            assert_eq!(meta.len(), 0, "memory-bank/{} should be empty", file);
        }
    }

    #[test]
    fn test_ward_create_does_not_precreate_code_dirs() {
        // No opinionated directories like core/, pkg/, lib/, src/ are
        // pre-created — the agent picks names appropriate for the language.
        let ward_dir = TempDir::new().unwrap();
        let ward_path = ward_dir.path().to_path_buf();

        WardTool::scaffold_empty_dirs(&ward_path, "minimal");

        for opinionated in ["core", "pkg", "lib", "src", "internal"] {
            assert!(
                !ward_path.join(opinionated).exists(),
                "ward scaffold must not pre-create {opinionated}/"
            );
        }
    }

    #[test]
    fn test_read_agents_md() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None);

        std::fs::write(dir.path().join("AGENTS.md"), "# My Project\n\nTest content").unwrap();

        let content = tool.read_agents_md(dir.path());
        assert!(content.is_some());
        assert!(content.unwrap().contains("# My Project"));
    }

    #[test]
    fn test_create_agents_md_does_not_overwrite() {
        let ward_dir = TempDir::new().unwrap();
        let ward_path = ward_dir.path().to_path_buf();
        std::fs::write(ward_path.join("AGENTS.md"), "# Custom content").unwrap();

        WardTool::write_agents_md(&ward_path, "existing");

        let content = std::fs::read_to_string(ward_path.join("AGENTS.md")).unwrap();
        assert!(content.contains("# Custom content")); // Not overwritten
    }
}
