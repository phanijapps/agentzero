// ============================================================================
// FILE TOOLS
// Read, Write, and Edit tools
// ============================================================================

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{FileSystemContext, NoFileSystemContext};
use agent_primitives::{Result, Tool, ToolContext, ToolPermissions};

// ============================================================================
// READ TOOL
// ============================================================================

/// Tool for reading file contents
pub struct ReadTool {
    /// File system context for ward-relative fallback reads.
    fs: Arc<dyn FileSystemContext>,
}

impl ReadTool {
    /// Create a new read tool with file system context.
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self { fs }
    }
}

impl Default for ReadTool {
    fn default() -> Self {
        Self::new(Arc::new(NoFileSystemContext))
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read contents of a file. Supports optional offset and limit for line-by-line reading. Relative paths fall back to the current ward when direct reads fail."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Starting line number (0-indexed)",
                    "default": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["path"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'path' parameter".to_string())
        })?;

        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64());

        tracing::debug!(
            file = %file!(),
            line = %line!(),
            "Reading file: {} (offset: {}, limit: {:?})",
            path, offset, limit
        );

        let content = read_with_ward_fallback(&self.fs, &ctx, path)?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = offset.min(total_lines);
        let end = if let Some(lim) = limit {
            (start + lim as usize).min(total_lines)
        } else {
            total_lines
        };

        let selected_lines = lines[start..end].join("\n");

        Ok(json!({
            "content": selected_lines,
            "total_lines": total_lines,
            "lines_read": end - start,
            "offset": start,
        }))
    }
}

fn read_with_ward_fallback(
    fs: &Arc<dyn FileSystemContext>,
    ctx: &Arc<dyn ToolContext>,
    path: &str,
) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(direct_err) => {
            if !can_try_ward_relative(path) {
                return Err(agent_primitives::AgentError::Tool(format!(
                    "Failed to read file: {}",
                    direct_err
                )));
            }

            let ward_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "scratch".to_string());
            let Some(ward_dir) = fs.ward_dir(&ward_id) else {
                return Err(agent_primitives::AgentError::Tool(format!(
                    "Failed to read file: {}",
                    direct_err
                )));
            };

            let ward_relative = path.trim_start_matches("./");
            let ward_path = ward_dir.join(ward_relative);
            std::fs::read_to_string(&ward_path).map_err(|ward_err| {
                agent_primitives::AgentError::Tool(format!(
                    "Failed to read file: {}; ward fallback {} failed: {}",
                    direct_err,
                    ward_path.display(),
                    ward_err
                ))
            })
        }
    }
}

fn can_try_ward_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with("~/")
        && !path.starts_with('\\')
        && !Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

// ============================================================================
// WRITE TOOL
// ============================================================================

/// Tool for writing content to files
pub struct WriteTool {
    /// File system context
    fs: Arc<dyn FileSystemContext>,
}

impl WriteTool {
    /// Create a new write tool with file system context
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write or append content to a file. Path routing:\n\
         Default → current ward directory (code files, visible to shell)\n\
         'attachments/' → agent_data/{session}/attachments/ (final outputs: .docx, .pptx)\n\
         'scratchpad/' → agent_data/{session}/scratchpad/ (intermediate work)\n\
         For large files (200+ lines), write the skeleton first, then use append calls."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path. Default goes to code dir. Prefix 'attachments/' or 'scratchpad/' routes to agent_data."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "mode": {
                    "type": "string",
                    "enum": ["write", "append"],
                    "default": "write",
                    "description": "write: Create/overwrite file. append: Add to end of existing file. Use append for large content that must be split across multiple calls."
                }
            },
            "required": ["path", "content"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::moderate(vec!["filesystem:write".into()])
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Check for error markers from truncated tool calls
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            let _truncated = args
                .get("__truncated__")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            return Err(agent_primitives::AgentError::Tool(format!(
                "{}: {}",
                error_type, message
            )));
        }

        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'path' parameter".to_string())
        })?;

        // Extract filename for logging
        let filename = path.rsplit('/').next().unwrap_or(path);

        tracing::info!(
            file = %file!(),
            line = %line!(),
            "WRITE tool called: filename='{}', requested_path='{}'",
            filename, path
        );

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'content' parameter".to_string())
            })?;

        // Get write mode (default: "write", can be "append")
        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("write");
        let is_append = mode == "append";

        // Security: Reject paths with parent directory components
        if path.contains("..") {
            return Err(agent_primitives::AgentError::Tool(
                "Path cannot contain '..' for security reasons.".to_string(),
            ));
        }

        // Security: Reject absolute paths
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(agent_primitives::AgentError::Tool(
                "Absolute paths are not allowed. Use a relative path within the agent data directory.".to_string()
            ));
        }

        // Get session_id from state for path routing
        let session_id = ctx
            .get_state("session_id")
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("session_id not found in state.".to_string())
            })?;

        tracing::info!(
            file = %file!(),
            line = %line!(),
            "{} file: path='{}' ({} bytes), session_id={}",
            if is_append { "Appending to" } else { "Writing" },
            path, content.len(), session_id
        );

        // Route based on path prefix:
        // attachments/ and scratchpad/ → agent_data/{session}/ (session-scoped)
        // everything else → wards/{ward_id}/ (ward-scoped, where shell runs)
        let final_path = if path.starts_with("attachments/") || path.starts_with("scratchpad/") {
            let data_dir = self.fs.session_data_dir(&session_id).ok_or_else(|| {
                agent_primitives::AgentError::Tool("Session data dir unavailable".to_string())
            })?;
            data_dir.join(path)
        } else {
            // Use ward_id if set, otherwise fall back to "scratch"
            let ward_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                .unwrap_or_else(|| "scratch".to_string());

            let ward_dir = self.fs.ward_dir(&ward_id).ok_or_else(|| {
                agent_primitives::AgentError::Tool("Ward dir unavailable".to_string())
            })?;
            ward_dir.join(path)
        };

        // Create parent directories
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                agent_primitives::AgentError::Tool(format!("Failed to create directories: {}", e))
            })?;
        }

        // Write or append to the file
        tracing::info!(
            file = %file!(),
            line = %line!(),
            "{} file: {}",
            if is_append { "Appending to" } else { "Writing to" },
            final_path.display()
        );

        if is_append {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&final_path)
                .map_err(|e| {
                    agent_primitives::AgentError::Tool(format!(
                        "Failed to open file for append: {}",
                        e
                    ))
                })?;
            file.write_all(content.as_bytes()).map_err(|e| {
                agent_primitives::AgentError::Tool(format!("Failed to append to file: {}", e))
            })?;
        } else {
            std::fs::write(&final_path, content).map_err(|e| {
                agent_primitives::AgentError::Tool(format!("Failed to write file: {}", e))
            })?;
        }

        // Return the original requested path (not the absolute resolved path)
        // so the LLM continues to use relative paths
        Ok(json!({
            "path": path,
            "bytes_written": content.len(),
            "mode": mode
        }))
    }
}

// ============================================================================
// EDIT TOOL
// ============================================================================

/// Tool for editing files with search and replace
///
/// Note: This tool requires conversation context to resolve paths.
/// The conversation_id is read from the ToolContext's state during execution.
pub struct EditTool {
    /// File system context for resolving conversation paths
    fs: Arc<dyn FileSystemContext>,
}

impl EditTool {
    /// Create a new edit tool with file system context
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file using search/replace. Same path routing as write: default → current ward, \
         'attachments/' and 'scratchpad/' → agent_data/{session}/."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path. Default goes to code dir. Prefix 'attachments/' or 'scratchpad/' routes to agent_data."
                },
                "replacements": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old": {"type": "string"},
                            "new": {"type": "string"}
                        },
                        "required": ["old", "new"]
                    },
                    "description": "List of search/replace operations"
                }
            },
            "required": ["path", "replacements"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::moderate(vec!["filesystem:write".into()])
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Check for error markers from truncated tool calls
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            let _truncated = args
                .get("__truncated__")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            return Err(agent_primitives::AgentError::Tool(format!(
                "{}: {}",
                error_type, message
            )));
        }

        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'path' parameter".to_string())
        })?;

        // Extract filename for logging
        let filename = path.rsplit('/').next().unwrap_or(path);

        tracing::info!(
            file = %file!(),
            line = %line!(),
            "EDIT tool called: filename='{}', requested_path='{}'",
            filename, path
        );

        let replacements = args
            .get("replacements")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'replacements' parameter".to_string())
            })?;

        // Security: Reject paths with parent directory components
        if path.contains("..") {
            return Err(agent_primitives::AgentError::Tool(
                "Path cannot contain '..' for security reasons.".to_string(),
            ));
        }

        // Security: Reject absolute paths
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(agent_primitives::AgentError::Tool(
                "Absolute paths are not allowed. Use a relative path within the agent data directory.".to_string()
            ));
        }

        // Get session_id from state for path routing
        let session_id = ctx
            .get_state("session_id")
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("session_id not found in state.".to_string())
            })?;

        tracing::info!(
            file = %file!(),
            line = %line!(),
            "Editing file: path='{}', session_id={}",
            path, session_id
        );

        // Route based on path prefix (same as write tool)
        let final_path = if path.starts_with("attachments/") || path.starts_with("scratchpad/") {
            let data_dir = self.fs.session_data_dir(&session_id).ok_or_else(|| {
                agent_primitives::AgentError::Tool("Session data dir unavailable".to_string())
            })?;
            data_dir.join(path)
        } else {
            // Use ward_id if set, otherwise fall back to "scratch"
            let ward_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                .unwrap_or_else(|| "scratch".to_string());

            let ward_dir = self.fs.ward_dir(&ward_id).ok_or_else(|| {
                agent_primitives::AgentError::Tool("Ward dir unavailable".to_string())
            })?;
            ward_dir.join(path)
        };

        let mut content = std::fs::read_to_string(&final_path).map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to read file: {}", e))
        })?;

        let mut count = 0;
        for repl in replacements {
            let old = repl.get("old").and_then(|v| v.as_str()).ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'old' in replacement".to_string())
            })?;

            let new = repl.get("new").and_then(|v| v.as_str()).ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'new' in replacement".to_string())
            })?;

            count += content.matches(old).count();
            content = content.replace(old, new);
        }

        tracing::info!(
            file = %file!(),
            line = %line!(),
            "Editing file: {} ({} replacements)",
            final_path.display(), count
        );

        std::fs::write(&final_path, content).map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to write file: {}", e))
        })?;

        // Return the original requested path (not the absolute resolved path)
        Ok(json!({
            "path": path,
            "replacements_made": count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::LazyLock;

    use agent_primitives::types::Content;
    use agent_primitives::{CallbackContext, EventActions, ReadonlyContext};
    use serde_json::json;

    struct TestFs {
        wards_root: PathBuf,
    }

    impl FileSystemContext for TestFs {
        fn conversation_dir(&self, _conversation_id: &str) -> Option<PathBuf> {
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

        fn python_executable(&self) -> Option<PathBuf> {
            None
        }

        fn wards_root_dir(&self) -> Option<PathBuf> {
            Some(self.wards_root.clone())
        }
    }

    struct TestCtx {
        state: HashMap<String, Value>,
    }

    impl TestCtx {
        fn with_ward(ward_id: &str) -> Self {
            Self {
                state: HashMap::from([("ward_id".to_string(), json!(ward_id))]),
            }
        }

        fn empty() -> Self {
            Self {
                state: HashMap::new(),
            }
        }
    }

    impl ReadonlyContext for TestCtx {
        fn invocation_id(&self) -> &str {
            "test-invocation"
        }

        fn agent_name(&self) -> &str {
            "test-agent"
        }

        fn user_id(&self) -> &str {
            "test-user"
        }

        fn app_name(&self) -> &str {
            "test-app"
        }

        fn session_id(&self) -> &str {
            "test-session"
        }

        fn branch(&self) -> &str {
            "test"
        }

        fn user_content(&self) -> &Content {
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for TestCtx {
        fn get_state(&self, key: &str) -> Option<Value> {
            self.state.get(key).cloned()
        }

        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for TestCtx {
        fn function_call_id(&self) -> String {
            "test-call".to_string()
        }

        fn actions(&self) -> EventActions {
            EventActions::default()
        }

        fn set_actions(&self, _actions: EventActions) {}
    }

    #[tokio::test]
    async fn read_falls_back_to_active_ward_for_relative_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let wards_root = temp.path().join("wards");
        let target = wards_root
            .join("financial-analysis")
            .join("xom-valuation")
            .join("code");
        std::fs::create_dir_all(&target).expect("create ward target");
        std::fs::write(
            target.join("fetch_catalysts_risk.py"),
            "line one\nline two\nline three\n",
        )
        .expect("write fixture");

        let tool = ReadTool::new(Arc::new(TestFs { wards_root }));
        let result = tool
            .execute(
                Arc::new(TestCtx::with_ward("financial-analysis")),
                json!({
                    "path": "xom-valuation/code/fetch_catalysts_risk.py",
                    "offset": 1,
                    "limit": 1
                }),
            )
            .await
            .expect("read should fall back to ward");

        assert_eq!(result["content"], "line two");
        assert_eq!(result["total_lines"], 3);
        assert_eq!(result["lines_read"], 1);
        assert_eq!(result["offset"], 1);
    }

    #[tokio::test]
    async fn read_preserves_absolute_path_behavior() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("absolute.txt");
        std::fs::write(&path, "absolute content").expect("write fixture");

        let tool = ReadTool::default();
        let result = tool
            .execute(
                Arc::new(TestCtx::empty()),
                json!({ "path": path.to_string_lossy() }),
            )
            .await
            .expect("absolute path should read directly");

        assert_eq!(result["content"], "absolute content");
    }
}
