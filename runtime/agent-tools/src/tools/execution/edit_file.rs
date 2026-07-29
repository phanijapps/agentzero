// ============================================================================
// EDIT FILE TOOL
// Find and replace text in an existing file.
// Simple string matching — no diff format, no context lines.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, FileSystemContext, Result, Tool, ToolContext};
use zbot_stores_traits::MemoryFactStore;

use super::ast_hook;
use super::ward_cwd::resolve_ward_cwd;

/// Tool that performs find-and-replace edits on existing files.
pub struct EditFileTool {
    fs: Arc<dyn FileSystemContext>,
    fact_store: Option<Arc<dyn MemoryFactStore>>,
}

impl EditFileTool {
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self {
            fs,
            fact_store: None,
        }
    }

    /// Attach a fact store so the AST post-hook can upsert primitives
    /// after successful edits to supported-language source files.
    #[must_use]
    pub fn with_fact_store(mut self, fact_store: Arc<dyn MemoryFactStore>) -> Self {
        self.fact_store = Some(fact_store);
        self
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit an existing file by finding and replacing text. Provide the exact text to find and the replacement text."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to the ward"
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to find in the file (must match exactly)"
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text"
                }
            },
            "required": ["path", "old_text", "new_text"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'path' parameter".to_string()))?;

        let old_text = args
            .get("old_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'old_text' parameter".to_string()))?;

        let new_text = args
            .get("new_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'new_text' parameter".to_string()))?;

        if path.is_empty() {
            return Err(AgentError::Tool("Path cannot be empty".to_string()));
        }

        if old_text.is_empty() {
            return Err(AgentError::Tool("old_text cannot be empty".to_string()));
        }

        // Sanitize path
        let path = path
            .trim_start_matches("~/")
            .trim_start_matches("/")
            .trim_start_matches("./");
        if path.contains("..") {
            return Err(AgentError::Tool(
                "Path cannot contain '..' — all paths must be relative to the ward".to_string(),
            ));
        }

        // Resolve CWD from ward context
        let cwd = resolve_ward_cwd(&self.fs, &ctx);
        let full_path = cwd.join(path);

        if !full_path.exists() {
            return Ok(json!({
                "success": false,
                "error": format!("File not found: {}", path)
            }));
        }

        // Read the file
        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| AgentError::Tool(format!("Failed to read {}: {}", path, e)))?;

        // Count occurrences — old_text MUST be unique in the file
        let count = content.matches(old_text).count();

        if count == 0 {
            // Try trimmed match as fallback
            let trimmed_old = old_text.trim();
            let trimmed_count = content.matches(trimmed_old).count();

            if trimmed_count == 1 {
                let new_content = content.replacen(trimmed_old, new_text, 1);
                std::fs::write(&full_path, &new_content)
                    .map_err(|e| AgentError::Tool(format!("Failed to write {}: {}", path, e)))?;
                tracing::debug!("edit_file: replaced (trimmed match) in {}", path);
                let result = json!({
                    "success": true,
                    "path": path,
                    "match_type": "trimmed",
                    "message": format!("Replaced 1 occurrence (trimmed match) in {}", path)
                });
                return Ok(result);
            } else if trimmed_count > 1 {
                return Ok(json!({
                    "success": false,
                    "error": format!("Found {} occurrences of old_text in {}. The text must be unique — include more surrounding context to disambiguate.", trimmed_count, path)
                }));
            }

            return Ok(json!({
                "success": false,
                "error": format!("old_text not found in {}. The text must match exactly (including whitespace).", path),
                "hint": "Use shell with rg or grep to find the exact text first, then copy it precisely."
            }));
        }

        if count > 1 {
            return Ok(json!({
                "success": false,
                "error": format!("Found {} occurrences of old_text in {}. The text must be unique — include more surrounding context to disambiguate.", count, path)
            }));
        }

        // Exactly one match — safe to replace
        let new_content = content.replacen(old_text, new_text, 1);

        // Verify the edit actually changed something
        if new_content == content {
            return Ok(json!({
                "success": false,
                "error": "old_text and new_text are identical — no change made."
            }));
        }

        std::fs::write(&full_path, &new_content)
            .map_err(|e| AgentError::Tool(format!("Failed to write {}: {}", path, e)))?;

        tracing::debug!("edit_file: replaced unique match in {}", path);

        // Post-hook: AST extract primitives so the next subagent sees
        // the updated signatures. Fire-and-forget, .py files only.
        if let Some(fs) = self.fact_store.clone()
            && let Some(ward_id) = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(String::from))
        {
            let abs = full_path.clone();
            let rel = path.to_string();
            tokio::spawn(async move {
                ast_hook::run(&ward_id, &abs, &rel, &fs).await;
            });
        }

        let result = json!({
            "success": true,
            "path": path,
            "message": format!("Edit applied to {}", path)
        });
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edit_file_schema() {
        let tool = EditFileTool::new(Arc::new(agent_primitives::NoFileSystemContext));
        assert_eq!(tool.name(), "edit_file");
        let schema = tool.parameters_schema().unwrap();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["old_text"].is_object());
        assert!(schema["properties"]["new_text"].is_object());
    }
}
