// ============================================================================
// MULTIMODAL ANALYZE TOOL
// Universal vision fallback — any agent can process images/files via this tool.
// Makes a direct one-shot LLM call to the configured vision model.
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::multimodal::rehydrate_source;
use agent_primitives::types::ContentSource;
use agent_primitives::{AgentError, Result, Tool, ToolContext, ToolPermissions};

pub struct MultimodalAnalyzeTool;

impl Default for MultimodalAnalyzeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl MultimodalAnalyzeTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for MultimodalAnalyzeTool {
    fn name(&self) -> &str {
        "multimodal_analyze"
    }

    fn description(&self) -> &str {
        "Analyze images, PDFs, or documents using a vision-capable model. \
         Send one or more content items with a prompt, get structured analysis back. \
         Use when you need to understand visual content but your current model doesn't support vision."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "required": ["content", "prompt"],
            "properties": {
                "content": {
                    "type": "array",
                    "description": "Content items to analyze",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["image", "file"] },
                            "source": { "type": "string", "description": "File path, URL, or base64 data" },
                            "detail": { "type": "string", "enum": ["low", "high", "auto"] }
                        },
                        "required": ["type", "source"]
                    }
                },
                "prompt": { "type": "string", "description": "What to analyze or extract" },
                "output_schema": { "type": "object", "description": "Optional JSON Schema for structured output" }
            }
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let content_items = args
            .get("content")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AgentError::Tool("'content' must be an array".to_string()))?;

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("'prompt' is required".to_string()))?;

        let output_schema = args.get("output_schema").cloned();

        // Read multimodal config from state (injected by executor builder)
        let config = ctx.get_state("multimodal_config")
            .ok_or_else(|| AgentError::Tool(
                "No multimodal model configured. Add a vision-capable model to Settings > Advanced > Multimodal.".to_string()
            ))?;

        let base_url = config
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::Tool("multimodal provider baseUrl not resolved".to_string())
            })?;
        let api_key = config.get("apiKey").and_then(|v| v.as_str()).unwrap_or("");
        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("multimodal.model not configured".to_string()))?;
        let temperature = config
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.3);
        let max_tokens = config
            .get("maxOutputTokens")
            .or_else(|| config.get("maxTokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(32_000);

        // Build OpenAI content array from inputs
        let mut content_blocks: Vec<Value> = Vec::new();

        // Add the prompt first as text
        content_blocks.push(json!({ "type": "text", "text": prompt }));

        for item in content_items {
            let content_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("image");
            let source_str = item.get("source").and_then(|v| v.as_str()).ok_or_else(|| {
                AgentError::Tool("Each content item must have a 'source'".to_string())
            })?;

            let source = resolve_source(source_str)?;

            match content_type {
                "image" => {
                    let detail = item
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .unwrap_or("auto");
                    let mime_type = infer_image_mime(source_str);
                    let resolved = rehydrate_source(&source)
                        .map_err(|e| AgentError::Tool(format!("Failed to resolve image: {}", e)))?;
                    let url = match &resolved {
                        ContentSource::Base64(data) => {
                            format!("data:{};base64,{}", mime_type, data)
                        }
                        ContentSource::Url(url) => url.clone(),
                        ContentSource::FileRef(_) => unreachable!(),
                    };
                    content_blocks.push(json!({
                        "type": "image_url",
                        "image_url": { "url": url, "detail": detail }
                    }));
                }
                "file" => {
                    let mime_type = infer_file_mime(source_str);
                    let resolved = rehydrate_source(&source)
                        .map_err(|e| AgentError::Tool(format!("Failed to resolve file: {}", e)))?;
                    let url = match &resolved {
                        ContentSource::Base64(data) => {
                            format!("data:{};base64,{}", mime_type, data)
                        }
                        ContentSource::Url(url) => url.clone(),
                        ContentSource::FileRef(_) => unreachable!(),
                    };
                    content_blocks.push(json!({
                        "type": "file",
                        "file": { "url": url }
                    }));
                }
                other => return Err(AgentError::Tool(format!("Unknown content type: {}", other))),
            }
        }

        // Build the OpenAI-compatible request body
        let mut body = json!({
            "model": model,
            "messages": [{
                "role": "user",
                "content": content_blocks,
            }],
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        // Add response_format if output_schema provided
        if let Some(schema) = output_schema {
            body.as_object_mut().unwrap().insert(
                "response_format".to_string(),
                json!({ "type": "json_schema", "json_schema": { "name": "analysis", "schema": schema } }),
            );
        }

        // Make the API call
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        tracing::info!("multimodal_analyze: calling {} with model {}", url, model);

        let client = reqwest::Client::builder().build().expect("reqwest client");
        let mut request = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body);

        if !api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = request
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("Multimodal API call failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AgentError::Tool(format!(
                "Multimodal API error ({}): {}",
                status, error_text
            )));
        }

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| AgentError::Tool(format!("Failed to parse API response: {}", e)))?;

        // Extract the assistant's response content
        let content = response_json
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Try to parse as JSON if output_schema was provided
        if args.get("output_schema").is_some()
            && let Ok(parsed) = serde_json::from_str::<Value>(&content)
        {
            return Ok(parsed);
        }

        Ok(json!({ "analysis": content }))
    }
}

fn resolve_source(source: &str) -> Result<ContentSource> {
    if source.starts_with("data:") {
        if let Some(pos) = source.find(";base64,") {
            let data = &source[pos + 8..];
            return Ok(ContentSource::Base64(data.to_string()));
        }
        return Ok(ContentSource::Url(source.to_string()));
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(ContentSource::Url(source.to_string()));
    }
    // File path
    let path = source.strip_prefix("file://").unwrap_or(source);
    if !std::path::Path::new(path).exists() {
        return Err(AgentError::Tool(format!("File not found: {}", path)));
    }
    use base64::Engine;
    let bytes = std::fs::read(path)
        .map_err(|e| AgentError::Tool(format!("Failed to read file {}: {}", path, e)))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(ContentSource::Base64(encoded))
}

fn infer_image_mime(source: &str) -> String {
    let lower = source.to_lowercase();
    if lower.ends_with(".png") {
        "image/png".to_string()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".to_string()
    } else if lower.ends_with(".webp") {
        "image/webp".to_string()
    } else if lower.ends_with(".gif") {
        "image/gif".to_string()
    } else {
        "image/png".to_string()
    }
}

fn infer_file_mime(source: &str) -> String {
    let lower = source.to_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf".to_string()
    } else if lower.ends_with(".csv") {
        "text/csv".to_string()
    } else if lower.ends_with(".txt") {
        "text/plain".to_string()
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html".to_string()
    } else if lower.ends_with(".json") {
        "application/json".to_string()
    } else {
        "application/octet-stream".to_string()
    }
}
