// ============================================================================
// WRITE FILE TOOL
// Creates or overwrites a file with the given content.
// Simple, reliable — no diff format, no context matching.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, FileSystemContext, Result, Tool, ToolContext};
use zbot_stores_traits::MemoryFactStore;

use super::ast_hook;
use super::ward_cwd::resolve_ward_cwd;

/// Tool that creates or overwrites a file with content.
pub struct WriteFileTool {
    fs: Arc<dyn FileSystemContext>,
    /// Optional fact store; when present, successful writes to .py
    /// files trigger the AST post-hook that upserts primitive rows.
    fact_store: Option<Arc<dyn MemoryFactStore>>,
}

impl WriteFileTool {
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self {
            fs,
            fact_store: None,
        }
    }

    /// Attach a fact store so the AST post-hook can upsert primitives.
    #[must_use]
    pub fn with_fact_store(mut self, fact_store: Arc<dyn MemoryFactStore>) -> Self {
        self.fact_store = Some(fact_store);
        self
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with the given content. Path is relative to the current ward."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to the ward (e.g., 'core/utils.py', 'data/config.json')"
                },
                "content": {
                    "type": "string",
                    "description": "Complete file content to write"
                }
            },
            "required": ["path", "content"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'path' parameter".to_string()))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'content' parameter".to_string()))?;

        if path.is_empty() {
            return Err(AgentError::Tool("Path cannot be empty".to_string()));
        }

        // Sanitize path — reject absolute paths and home-relative paths
        let mut path = path
            .trim_start_matches("~/")
            .trim_start_matches("/")
            .trim_start_matches("./")
            .to_string();

        // Reject paths that try to escape the ward
        if path.contains("..") {
            return Err(AgentError::Tool(
                "Path cannot contain '..' — all paths must be relative to the ward".to_string(),
            ));
        }

        // Resolve CWD from ward context
        let cwd = resolve_ward_cwd(&self.fs, &ctx);

        // Fix path doubling: if the agent used an absolute path like
        // ~/Documents/zbot/wards/{ward}/specs/plan.md, the sanitizer strips ~/
        // but leaves Documents/zbot/wards/{ward}/specs/plan.md.
        // Detect and extract only the relative part after the ward directory.
        path = fix_doubled_ward_path(
            path,
            &cwd,
            args.get("path").and_then(|v| v.as_str()).unwrap_or(""),
        );

        let full_path = cwd.join(&path);

        // Create parent directories
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentError::Tool(format!("Failed to create directories for {}: {}", path, e))
            })?;
        }

        // Write the file
        let is_new = !full_path.exists();
        std::fs::write(&full_path, content)
            .map_err(|e| AgentError::Tool(format!("Failed to write {}: {}", path, e)))?;

        let action = if is_new { "created" } else { "overwritten" };
        tracing::debug!("write_file: {} {} ({} bytes)", action, path, content.len());

        // Post-hook: AST extract primitives for the next subagent's snapshot.
        // Fire-and-forget so the tool returns immediately; extraction runs
        // off the critical path. No-op for non-.py files.
        if let Some(fs) = self.fact_store.clone() {
            let ward_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(String::from));
            if let Some(ward_id) = ward_id {
                let abs = full_path.clone();
                let rel = path.clone();
                tokio::spawn(async move {
                    ast_hook::run(&ward_id, &abs, &rel, &fs).await;
                });
            }
        }

        // Size warning for large files — nudge agent to split into modules
        let warning = if content.len() > 5120 {
            Some(format!(
                "⚠ This file is {}KB — consider splitting into smaller modules. Files > 5KB are harder to maintain and reuse.",
                content.len() / 1024
            ))
        } else {
            None
        };

        let mut result = json!({
            "success": true,
            "path": path,
            "action": action,
            "bytes": content.len(),
            "message": format!("File {} ({} bytes)", action, content.len())
        });
        if let Some(warn) = warning {
            result["warning"] = json!(warn);
        }
        Ok(result)
    }
}

/// Resolve path doubling where a home-relative or wards-relative prefix was not
/// stripped by the sanitizer. Returns the corrected path.
fn fix_doubled_ward_path(mut path: String, cwd: &std::path::Path, original: &str) -> String {
    let cwd_str = cwd.to_string_lossy();
    if let Some(home) = dirs::home_dir() {
        let home_relative = cwd_str
            .trim_start_matches(home.to_string_lossy().as_ref())
            .trim_start_matches('/');
        if path.starts_with(home_relative) {
            let fixed = path[home_relative.len()..]
                .trim_start_matches('/')
                .to_string();
            tracing::debug!(original = %original, resolved = %fixed, "Fixed doubled ward path");
            path = fixed;
        }
    }
    if let Some(ward_pos) = path.find("wards/") {
        let after_wards = &path[ward_pos + 6..]; // skip "wards/"
        if let Some(slash) = after_wards.find('/') {
            let ward_relative = &after_wards[slash + 1..];
            if !ward_relative.is_empty() {
                tracing::debug!(original = %path, resolved = %ward_relative, "Stripped wards/ prefix from path");
                return ward_relative.to_string();
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_file_schema() {
        let tool = WriteFileTool::new(Arc::new(agent_primitives::NoFileSystemContext));
        assert_eq!(tool.name(), "write_file");
        let schema = tool.parameters_schema().unwrap();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["content"].is_object());
    }
}
