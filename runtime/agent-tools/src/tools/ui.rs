// ============================================================================
// UI TOOLS
// RequestInput and ShowContent tools
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{Result, Tool, ToolContext};

// ============================================================================
// REQUEST INPUT TOOL
// ============================================================================

/// Tool for requesting structured user input
pub struct RequestInputTool;

#[async_trait]
impl Tool for RequestInputTool {
    fn name(&self) -> &str {
        "request_input"
    }

    fn description(&self) -> &str {
        "CRITICAL: MUST use this tool whenever you need to collect 2+ pieces of related information from the user. \
        NEVER ask multiple separate questions in plain text - ALWAYS use this tool to request all information at once via a form. \
        This is REQUIRED for better user experience. \
        Examples: collecting input details (name, age, address), form data, configuration details, etc."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "form_id": {
                    "type": "string",
                    "description": "Unique identifier for this form"
                },
                "title": {
                    "type": "string",
                    "description": "Form title"
                },
                "description": {
                    "type": "string",
                    "description": "Form description"
                },
                "schema": {
                    "type": "object",
                    "description": "JSON Schema for the form"
                }
            },
            "required": ["form_id", "title", "schema"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let form_id = args
            .get("form_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'form_id' parameter".to_string())
            })?
            .to_string();

        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'title' parameter".to_string())
            })?
            .to_string();

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let schema = args.get("schema").cloned().ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'schema' parameter".to_string())
        })?;

        let submit_button = args
            .get("submit_button")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        tracing::debug!("Requesting input form: {}", form_id);

        // Return the form request with the special marker
        Ok(json!({
            "__request_input": true,
            "form_id": form_id,
            "form_type": "json_schema",
            "title": title,
            "description": description,
            "schema": schema,
            "submit_button": submit_button
        }))
    }
}

// ============================================================================
// SHOW CONTENT TOOL
// ============================================================================

/// Tool for displaying content in the UI
pub struct ShowContentTool;

#[async_trait]
impl Tool for ShowContentTool {
    fn name(&self) -> &str {
        "show_content"
    }

    fn description(&self) -> &str {
        "IMPORTANT: Use this tool to display saved content (HTML, PDF, images, markdown, etc.) in a specialized viewer. ALWAYS save content first using write, then use this tool to display it. This provides a much better user experience than displaying raw content."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "content_type": {
                    "type": "string",
                    "description": "Type of content (pdf, image, html, text, etc.)"
                },
                "title": {
                    "type": "string",
                    "description": "Title for the content"
                },
                "content": {
                    "type": "string",
                    "description": "The content to display (or file path)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Path to a previously saved file"
                }
            },
            "required": ["content_type", "title"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let content_type = args
            .get("content_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'content_type' parameter".to_string())
            })?
            .to_string();

        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'title' parameter".to_string())
            })?
            .to_string();

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Get agent_id for constructing the proper relative path
        let agent_id = ctx
            .get_state("app:root_agent_id")
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .or_else(|| {
                ctx.get_state("app:agent_id")
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
            });

        // Normalize file_path to construct proper relative path for attachments
        // The write tool uses paths like "outputs/report.html" which resolves to
        // agent_data/{agent_id}/outputs/report.html
        // We need to construct: {agent_id}/outputs/report.html for the frontend
        let normalized_file_path = if let Some(ref fp) = file_path {
            // Remove any trailing slashes
            let fp = fp.trim_end_matches('/').trim_end_matches('\\');

            // Check if it's empty after trimming
            if fp.is_empty() {
                tracing::warn!("show_content received empty file_path");
                None
            } else if fp.contains(':') || fp.starts_with('/') || fp.starts_with('\\') {
                // Absolute path - extract the relative part after agent_data/{agent_id}/
                // Look for known patterns like "outputs/", "attachments/", etc.
                let path_lower = fp.to_lowercase();
                if let Some(idx) = path_lower.find("outputs") {
                    let relative = &fp[idx..];
                    if let Some(ref aid) = agent_id {
                        Some(format!("{}/{}", aid, relative))
                    } else {
                        Some(relative.to_string())
                    }
                } else if let Some(idx) = path_lower.find("attachments") {
                    let relative = &fp[idx..];
                    if let Some(ref aid) = agent_id {
                        Some(format!("{}/{}", aid, relative))
                    } else {
                        Some(relative.to_string())
                    }
                } else {
                    // Fall back to just filename
                    let filename = fp.rsplit(&['/', '\\'][..]).next().unwrap_or(fp);
                    if !filename.is_empty() {
                        if let Some(ref aid) = agent_id {
                            Some(format!("{}/outputs/{}", aid, filename))
                        } else {
                            Some(format!("outputs/{}", filename))
                        }
                    } else {
                        tracing::warn!(
                            "show_content: could not extract filename from path: {}",
                            fp
                        );
                        None
                    }
                }
            } else {
                // Already a relative path like "outputs/report.html"
                // Prepend agent_id to make it: agent_id/outputs/report.html
                if let Some(ref aid) = agent_id {
                    Some(format!("{}/{}", aid, fp))
                } else {
                    Some(fp.to_string())
                }
            }
        } else {
            None
        };

        let metadata = args.get("metadata").cloned();

        // Auto-detect is_attachment: true when file_path is provided
        let is_attachment = args
            .get("is_attachment")
            .and_then(|v| v.as_bool())
            .unwrap_or(normalized_file_path.is_some());

        let base64 = args.get("base64").and_then(|v| v.as_bool());

        tracing::info!(
            "show_content: type={}, title={}, is_attachment={}, original_path={:?}, normalized={:?}, agent_id={:?}",
            content_type,
            title,
            is_attachment,
            file_path,
            normalized_file_path,
            agent_id
        );

        // Return the content display request with the special marker
        Ok(json!({
            "__show_content": true,
            "content_type": content_type,
            "title": title,
            "content": content.unwrap_or_default(),
            "file_path": normalized_file_path,
            "metadata": metadata,
            "is_attachment": is_attachment,
            "base64": base64
        }))
    }
}
