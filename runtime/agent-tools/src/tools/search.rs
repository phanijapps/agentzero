// ============================================================================
// SEARCH TOOLS
// Glob tool
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{Result, Tool, ToolContext};

// ============================================================================
// GLOB TOOL
// ============================================================================

/// Tool for finding files with glob patterns
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files using glob patterns like '*.rs' or '**/*.txt'."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern"
                },
                "include_hidden": {
                    "type": "boolean",
                    "default": false
                }
            },
            "required": ["pattern"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'pattern' parameter".to_string())
            })?;

        let _include_hidden = args
            .get("include_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        tracing::debug!("Glob: pattern={}", pattern);

        let matches = glob::glob(pattern)
            .map_err(|e| {
                agent_primitives::AgentError::Tool(format!("Invalid glob pattern: {}", e))
            })?
            .filter_map(|entry| entry.ok())
            .filter(|path| path.is_file())
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();

        Ok(json!({
            "matches": matches,
            "count": matches.len() }))
    }
}
