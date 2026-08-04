// ============================================================================
// OPENAI COMPATIBLE CLIENT
// OpenAI-compatible API implementation
// ============================================================================

use std::{
    collections::HashSet,
    fmt::Write as _,
    io::{self, Write},
    sync::Arc,
};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_stream::StreamExt;

use crate::llm::client::{
    ChatResponse, LlmClient, LlmError, StreamCallback, StreamChunk, TokenUsage, ToolCallChunk,
};
use crate::llm::config::LlmConfig;
use crate::types::{ChatMessage, ToolCall};
use agent_primitives::multimodal::rehydrate_source;
use agent_primitives::types::{ContentSource, Part};

/// OpenAI-compatible LLM client
///
/// This client works with any LLM provider that implements
/// the `OpenAI` API format (including many self-hosted models)
pub struct OpenAiClient {
    config: Arc<LlmConfig>,
    http_client: reqwest::Client,
}

const MAX_MODEL_VISIBLE_TOOLS: usize = 128;
const MAX_SINGLE_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_TOOL_SCHEMA_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolSchemaFootprint {
    tool_count: usize,
    serialized_bytes: usize,
    estimated_tokens: usize,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SerializedMeasure {
    bytes: usize,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeasureError {
    LimitExceeded,
    Serialization,
}

struct BoundedHashWriter {
    limit: usize,
    observed: usize,
    limit_exceeded: bool,
    hasher: Sha256,
}

impl BoundedHashWriter {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            observed: 0,
            limit_exceeded: false,
            hasher: Sha256::new(),
        }
    }

    fn finish(self) -> SerializedMeasure {
        let digest = self.hasher.finalize();
        SerializedMeasure {
            bytes: self.observed,
            sha256: bytes_to_hex(&digest),
        }
    }
}

impl Write for BoundedHashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.limit_exceeded {
            return Err(io::Error::other("serialized tool limit exceeded"));
        }

        let remaining = self.limit.saturating_sub(self.observed);
        if bytes.len() > remaining {
            self.hasher.update(&bytes[..remaining]);
            self.observed = self.limit.saturating_add(1);
            self.limit_exceeded = true;
            return Err(io::Error::other("serialized tool limit exceeded"));
        }

        self.hasher.update(bytes);
        self.observed += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn measure_serialized<T: Serialize>(
    value: &T,
    limit: usize,
) -> Result<SerializedMeasure, MeasureError> {
    let mut writer = BoundedHashWriter::new(limit);
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.finish()),
        Err(_) if writer.limit_exceeded => Err(MeasureError::LimitExceeded),
        Err(_) => Err(MeasureError::Serialization),
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    bytes_to_hex(&Sha256::digest(bytes))
}

fn prepare_tools(tools: &Value) -> Result<(Value, ToolSchemaFootprint), LlmError> {
    let tools = tools
        .as_array()
        .ok_or_else(|| LlmError::InvalidRequest("tool_schema_rule=inventory_type".to_string()))?;
    if tools.len() > MAX_MODEL_VISIBLE_TOOLS {
        return Err(LlmError::InvalidRequest(format!(
            "tool_schema_rule=tool_count_limit observed={} limit={MAX_MODEL_VISIBLE_TOOLS}",
            tools.len()
        )));
    }

    let mut names = HashSet::with_capacity(tools.len());
    let mut canonical = Vec::with_capacity(tools.len());
    for tool in tools {
        let object = tool
            .as_object()
            .ok_or_else(|| LlmError::InvalidRequest("tool_schema_rule=tool_object".to_string()))?;
        if object.get("type").and_then(Value::as_str) != Some("function") {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=tool_type".to_string(),
            ));
        }
        if object.keys().any(|key| key != "type" && key != "function") {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=tool_fields".to_string(),
            ));
        }
        let function = object
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                LlmError::InvalidRequest("tool_schema_rule=function_object".to_string())
            })?;
        if function.keys().any(|key| {
            key != "name" && key != "description" && key != "parameters" && key != "strict"
        }) {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=function_fields".to_string(),
            ));
        }
        if function
            .get("description")
            .is_some_and(|value| !value.is_string())
        {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=function_description".to_string(),
            ));
        }
        if function
            .get("parameters")
            .is_some_and(|value| !value.is_object())
        {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=function_parameters".to_string(),
            ));
        }
        if function
            .get("strict")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=function_strict".to_string(),
            ));
        }
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                LlmError::InvalidRequest("tool_schema_rule=function_name".to_string())
            })?;
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=function_name".to_string(),
            ));
        }
        if !names.insert(name) {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=duplicate_name".to_string(),
            ));
        }
        canonical.push((name, tool));
    }

    canonical.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let canonical_tools: Vec<&Value> = canonical.iter().map(|(_, tool)| *tool).collect();
    for tool in &canonical_tools {
        match measure_serialized(tool, MAX_SINGLE_TOOL_SCHEMA_BYTES) {
            Ok(_) => {}
            Err(MeasureError::LimitExceeded) => {
                return Err(LlmError::InvalidRequest(format!(
                    "tool_schema_rule=tool_bytes_limit observed_at_least={} limit={MAX_SINGLE_TOOL_SCHEMA_BYTES}",
                    MAX_SINGLE_TOOL_SCHEMA_BYTES + 1
                )));
            }
            Err(MeasureError::Serialization) => {
                return Err(LlmError::InvalidRequest(
                    "tool_schema_rule=serialization".to_string(),
                ));
            }
        }
    }

    let aggregate = match measure_serialized(&canonical_tools, MAX_TOOL_SCHEMA_BYTES) {
        Ok(measure) => measure,
        Err(MeasureError::LimitExceeded) => {
            return Err(LlmError::InvalidRequest(format!(
                "tool_schema_rule=total_bytes_limit observed_at_least={} limit={MAX_TOOL_SCHEMA_BYTES}",
                MAX_TOOL_SCHEMA_BYTES + 1
            )));
        }
        Err(MeasureError::Serialization) => {
            return Err(LlmError::InvalidRequest(
                "tool_schema_rule=serialization".to_string(),
            ));
        }
    };
    let tool_count = canonical_tools.len();
    let canonical = Value::Array(canonical_tools.into_iter().cloned().collect());
    Ok((
        canonical,
        ToolSchemaFootprint {
            tool_count,
            serialized_bytes: aggregate.bytes,
            estimated_tokens: aggregate.bytes.saturating_add(3) / 4,
            sha256: aggregate.sha256,
        },
    ))
}

async fn bounded_error_text(mut response: reqwest::Response) -> String {
    const LIMIT: usize = 64 * 1024;
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let remaining = LIMIT.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            break;
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

/// Extract the provider-reported cached prompt-token count from a `usage`
/// object, if any. Handles the two shapes we see in production:
///
///   - OpenAI: `usage.prompt_tokens_details.cached_tokens`
///   - GLM / DeepSeek / z.ai: `usage.prompt_cache_hit_tokens`
///
/// Returns `None` when neither field is present, so downstream code can
/// distinguish "no cache info reported" from "cache hit was zero".
fn extract_cached_prompt_tokens(usage: &Value) -> Option<u32> {
    if let Some(v) = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
    {
        return Some(v as u32);
    }
    usage
        .get("prompt_cache_hit_tokens")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
}

/// Log cache-hit ratio at INFO when the provider reported it. Silent when
/// not reported — keeps the log surface clean for providers that don't
/// expose cache stats.
fn log_cache_hit(tag: &str, usage: &TokenUsage) {
    let Some(cached) = usage.cached_prompt_tokens else {
        return;
    };
    let pct = if usage.prompt_tokens > 0 {
        (f64::from(cached) / f64::from(usage.prompt_tokens)) * 100.0
    } else {
        0.0
    };
    tracing::info!(
        cached_tokens = cached,
        prompt_tokens = usage.prompt_tokens,
        hit_pct = format!("{pct:.1}"),
        "{tag} prompt_cache"
    );
}

fn stream_transport_error(
    emitted_chars: usize,
    tool_call_count: usize,
    error: impl std::fmt::Display,
) -> LlmError {
    LlmError::ApiError(format!(
        "Streaming response terminated after {emitted_chars} chars and {tool_call_count} tool call(s); refusing to treat partial content as complete: {error}"
    ))
}

/// Attempt to recover the first JSON object from a concatenated string like `{"a":"b"}{"c":"d"}`.
/// Returns Some(Value) if recovery succeeds, None otherwise.
fn recover_first_json(raw: &str) -> Option<serde_json::Value> {
    if let Some(pos) = raw.find("}{") {
        let first = &raw[..=pos];
        serde_json::from_str(first).ok()
    } else {
        None
    }
}

/// Check if a JSON string is complete (has balanced braces, brackets, and strings).
///
/// Used to detect truncated tool call arguments when the LLM hits `max_tokens` mid-argument.
fn is_json_complete(json_str: &str) -> bool {
    let mut brace_count = 0i32;
    let mut bracket_count = 0i32;
    let mut in_string = false;
    let mut escape_next = false;

    for ch in json_str.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => {
                escape_next = true;
            }
            '"' => {
                in_string = !in_string;
            }
            '{' if !in_string => {
                brace_count += 1;
            }
            '}' if !in_string => {
                brace_count -= 1;
            }
            '[' if !in_string => {
                bracket_count += 1;
            }
            ']' if !in_string => {
                bracket_count -= 1;
            }
            _ => {}
        }

        if brace_count < 0 || bracket_count < 0 {
            return false;
        }
    }

    !in_string && brace_count == 0 && bracket_count == 0
}

impl OpenAiClient {
    /// Create a new OpenAI-compatible client
    pub fn new(config: LlmConfig) -> Result<Self, LlmError> {
        tracing::debug!("Creating OpenAI client for model: {}", config.model);
        if config.provider_id == "provider-ollama-cloud"
            && config.base_url != "https://ollama.com/v1"
        {
            return Err(LlmError::ApiError(
                "Ollama Cloud endpoint must be https://ollama.com/v1".to_string(),
            ));
        }
        let mut client_builder = reqwest::Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .timeout(std::time::Duration::from_secs(600));
        if config.provider_id == "provider-ollama-cloud" {
            client_builder = client_builder.redirect(reqwest::redirect::Policy::none());
        }
        let http_client = client_builder.build()?;
        Ok(Self {
            config: Arc::new(config),
            http_client,
        })
    }

    /// Get the configuration
    #[must_use]
    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    /// Rehydrate any `FileRef` sources in messages to Base64 before sending to the API.
    fn rehydrate_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        messages
            .into_iter()
            .map(|mut msg| {
                msg.content = msg
                    .content
                    .into_iter()
                    .map(|part| match &part {
                        Part::Image {
                            source: source @ ContentSource::FileRef(path),
                            mime_type,
                            detail,
                        } => match rehydrate_source(source) {
                            Ok(new_source) => Part::Image {
                                source: new_source,
                                mime_type: mime_type.clone(),
                                detail: detail.clone(),
                            },
                            Err(e) => {
                                tracing::warn!("Failed to rehydrate FileRef {}: {}", path, e);
                                part
                            }
                        },
                        Part::File {
                            source: source @ ContentSource::FileRef(path),
                            mime_type,
                            filename,
                        } => match rehydrate_source(source) {
                            Ok(new_source) => Part::File {
                                source: new_source,
                                mime_type: mime_type.clone(),
                                filename: filename.clone(),
                            },
                            Err(e) => {
                                tracing::warn!("Failed to rehydrate FileRef {}: {}", path, e);
                                part
                            }
                        },
                        _ => part,
                    })
                    .collect();
                msg
            })
            .collect()
    }

    /// Build the request body for the API.
    ///
    /// # Cache stability contract
    ///
    /// The serialized JSON produced here forms the prefix that upstream
    /// providers hash for prompt caching (OpenAI auto-caches at ≥1024
    /// tokens; GLM / DeepSeek / z.ai use similar schemes). Any per-call
    /// noise — timestamps, UUIDs, randomized ordering, debug markers —
    /// invalidates that cache and is billed as a miss.
    ///
    /// The body is deterministic today because:
    ///   - `json!` macro uses `serde_json::Map` (BTreeMap) → keys sorted
    ///   - no `Uuid::new_v4`, `chrono::now`, or `rand` calls in this fn
    ///   - `messages` are passed through unchanged
    ///   - `tools` are validated and canonically ordered by function name
    ///
    /// Do not introduce per-call noise here without updating
    /// `request_body_is_byte_stable_across_identical_calls` to detect
    /// the new axis and making the noise cache-friendly (e.g. moved
    /// behind the stable prefix).
    ///
    /// References:
    ///   - OpenAI prompt caching: <https://platform.openai.com/docs/guides/prompt-caching>
    ///   - GLM / z.ai cache fields: `prompt_cache_hit_tokens` in response usage
    fn build_request_body(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<Value, LlmError> {
        let prepared_tools = tools.as_ref().map(prepare_tools).transpose()?;
        let messages = Self::rehydrate_messages(messages);
        let mut body_obj = json!({
            "model": self.config.model,
            "messages": messages,
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_tokens,
            "stream": false,
        });

        // Add tools if present
        if let Some((tools_val, footprint)) = prepared_tools {
            if footprint.tool_count > 0 {
                tracing::debug!(
                    tool_count = footprint.tool_count,
                    serialized_bytes = footprint.serialized_bytes,
                    estimated_tokens = footprint.estimated_tokens,
                    sha256 = %footprint.sha256,
                    "LLM tool schema footprint"
                );
            }
            if let Some(body_map) = body_obj.as_object_mut() {
                body_map.insert("tools".to_string(), tools_val);
            }
        }

        if let Some(schema) = output_schema {
            if let Some(body_map) = body_obj.as_object_mut() {
                body_map.insert(
                    "response_format".to_string(),
                    json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": "structured_output",
                            "strict": true,
                            "schema": schema,
                        }
                    }),
                );
            }
        }

        // Add thinking parameter if enabled (for DeepSeek, GLM, etc.)
        if self.config.thinking_enabled {
            if let Some(body_map) = body_obj.as_object_mut() {
                body_map.insert("thinking".to_string(), json!({"type": "enabled"}));
            }
        }

        // Only serialize for logging at debug level (avoids 100KB serialization on every call)
        if tracing::enabled!(tracing::Level::DEBUG) {
            let request_json = serde_json::to_string(&body_obj).unwrap_or_default();
            let estimated_chars = request_json.len();
            let estimated_tokens = estimated_chars / 4;
            tracing::debug!(
                "Request size: ~{} chars (~{} tokens estimated)",
                estimated_chars,
                estimated_tokens
            );

            if let Some(messages_val) = body_obj.get("messages") {
                let messages_json = serde_json::to_string(messages_val).unwrap_or_default();
                let messages_tokens = messages_json.len() / 4;
                tracing::debug!(
                    "Messages: ~{} chars (~{} tokens)",
                    messages_json.len(),
                    messages_tokens
                );
            }
        }

        if self.config.thinking_enabled {
            tracing::debug!("Thinking mode enabled");
        }

        Ok(body_obj)
    }

    /// Make a non-streaming request to the API
    async fn make_request(&self, body: Value) -> Result<Value, LlmError> {
        let url = format!("{}/chat/completions", self.config.base_url);

        tracing::debug!("Making POST request to: {}", url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = bounded_error_text(response).await;
            let error_text = if self.config.api_key.is_empty() {
                error_text
            } else {
                error_text.replace(&self.config.api_key, "[redacted]")
            };
            tracing::error!("API error ({}): {}", status, error_text);
            return Err(LlmError::ApiError(format!("({status}): {error_text}")));
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| LlmError::ParseError(format!("Failed to parse response: {e}")))
    }

    /// Parse the API response
    fn parse_response(&self, response: Value) -> ChatResponse {
        let content = response
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Parse reasoning_content (for DeepSeek, GLM, etc.)
        let reasoning = response
            .pointer("/choices/0/message/reasoning_content")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        // Parse tool calls if present
        let tool_calls = self.parse_tool_calls(&response);

        // Parse token usage
        let usage = response.get("usage").and_then(|u| {
            Some(TokenUsage {
                prompt_tokens: u.get("prompt_tokens")?.as_u64()? as u32,
                completion_tokens: u.get("completion_tokens")?.as_u64()? as u32,
                total_tokens: u.get("total_tokens")?.as_u64()? as u32,
                cached_prompt_tokens: extract_cached_prompt_tokens(u),
            })
        });

        if let Some(ref u) = usage {
            log_cache_hit("chat", u);
        }

        ChatResponse {
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            reasoning,
            usage,
        }
    }

    /// Parse tool calls from the response
    fn parse_tool_calls(&self, response: &Value) -> Vec<ToolCall> {
        if let Some(calls) = response.pointer("/choices/0/message/tool_calls") {
            if let Some(calls_array) = calls.as_array() {
                return calls_array
                    .iter()
                    .filter_map(|call| {
                        let id = call.get("id")?.as_str()?.to_string();
                        let name = call.get("function")?.get("name")?.as_str()?.to_string();
                        let arguments_str = call
                            .get("function")?
                            .get("arguments")?
                            .as_str()?
                            .to_string();

                        // Parse arguments from string to Value for internal use.
                        // If the arguments can't be parsed or recovered, skip
                        // this tool call (return None) rather than panicking —
                        // the previous `from_str("null").unwrap_err()` crashed
                        // because `from_str("null")` is `Ok(Null)`.
                        let arguments = serde_json::from_str::<Value>(&arguments_str)
                            .ok()
                            .or_else(|| recover_first_json(&arguments_str))?;

                        Some(ToolCall::new(id, name, arguments))
                    })
                    .collect();
            }
        }
        Vec::new()
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn provider(&self) -> &str {
        &self.config.provider_id
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        tracing::info!("Starting chat with {} messages", messages.len());

        let body = self.build_request_body(messages, tools, None)?;
        let response = self.make_request(body).await?;
        let parsed = self.parse_response(response);

        tracing::info!("Chat completed, response length: {}", parsed.content.len());
        Ok(parsed)
    }

    async fn chat_with_schema(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        tracing::info!(
            has_schema = output_schema.is_some(),
            "Starting structured chat with {} messages",
            messages.len()
        );

        let body = self.build_request_body(messages, tools, output_schema)?;
        let response = self.make_request(body).await?;
        let parsed = self.parse_response(response);

        tracing::info!(
            "Structured chat completed, response length: {}",
            parsed.content.len()
        );
        Ok(parsed)
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        tracing::info!("Starting streaming chat with {} messages", messages.len());

        // Clone messages+tools for non-streaming fallback if stream breaks
        let fallback_messages = messages.clone();
        let fallback_tools = tools.clone();

        let url = format!("{}/chat/completions", self.config.base_url);

        let mut body_obj = self.build_request_body(messages, tools, None)?;
        // Enable streaming with usage reporting
        if let Some(obj) = body_obj.as_object_mut() {
            obj.insert("stream".to_string(), json!(true));
            obj.insert(
                "stream_options".to_string(),
                json!({ "include_usage": true }),
            );
        }

        tracing::debug!("Making streaming POST request to: {}", url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body_obj)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = bounded_error_text(response).await;
            let error_text = if self.config.api_key.is_empty() {
                error_text
            } else {
                error_text.replace(&self.config.api_key, "[redacted]")
            };
            tracing::error!("API error ({}): {}", status, error_text);
            return Err(LlmError::ApiError(format!("({status}): {error_text}")));
        }

        let mut full_content = String::new();
        let mut reasoning_content = String::new();
        let mut _finish_reason: Option<String> = None;
        let mut stream_usage: Option<TokenUsage> = None;

        // Accumulate streaming tool call deltas by index.
        // OpenAI sends tool calls as incremental deltas keyed by index:
        //   Delta 1: {index: 0, id: "call_123", function: {name: "write", arguments: ""}}
        //   Delta 2: {index: 0, function: {arguments: "{\"path\""}}
        //   Delta 3: {index: 0, function: {arguments: ": \"app.js\"}"}}
        // We must accumulate the id, name, and argument fragments per index,
        // then parse the complete JSON arguments after the stream ends.
        struct ToolCallAccumulator {
            id: String,
            name: String,
            arguments: String,
        }
        let mut tool_accumulators: std::collections::HashMap<u64, ToolCallAccumulator> =
            std::collections::HashMap::new();

        // Track provider-side accumulated text to handle providers that return
        // accumulated content instead of true deltas in streaming responses.
        // (e.g., Z.AI/GLM sends the full text so far in each delta.content)
        let mut provider_accumulated = String::new();

        // Read streaming response using line-buffered SSE parsing.
        // Each SSE line is processed exactly once — the buffer only retains
        // incomplete (partial) lines between HTTP chunks.
        let mut stream = response.bytes_stream();
        let mut sse_buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        emitted_chars = full_content.len(),
                        tool_calls = tool_accumulators.len(),
                        "Stream decode error — falling back to non-streaming: {}",
                        e
                    );

                    // If we haven't emitted anything yet, retry as non-streaming
                    if full_content.is_empty() && tool_accumulators.is_empty() {
                        tracing::info!("No content emitted yet, retrying as non-streaming request");
                        let body =
                            self.build_request_body(fallback_messages, fallback_tools, None)?;
                        let response = self.make_request(body).await?;
                        let parsed = self.parse_response(response);
                        // Emit the full response as a single token
                        if !parsed.content.is_empty() {
                            callback(StreamChunk::Token(parsed.content.clone()));
                        }
                        return Ok(parsed);
                    }

                    // Do not commit partial assistant text as a successful turn.
                    // A continuation can otherwise treat a truncated preamble as
                    // the delegate's final answer and make the wrong next move.
                    tracing::warn!(
                        "Stream broke after {} chars emitted — failing partial response",
                        full_content.len()
                    );
                    return Err(stream_transport_error(
                        full_content.len(),
                        tool_accumulators.len(),
                        e,
                    ));
                }
            };
            sse_buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Find the last complete line boundary
            let Some(last_nl) = sse_buffer.rfind('\n') else {
                continue; // No complete line yet, keep buffering
            };

            // Split: everything up to last newline is complete; remainder is partial
            let complete = sse_buffer[..last_nl].to_string();
            sse_buffer = sse_buffer[last_nl + 1..].to_string();

            // Process each complete SSE line exactly once
            for line in complete.lines() {
                let line = line.trim();
                if !line.starts_with("data: ") {
                    continue;
                }
                let data_payload = &line[6..];
                if data_payload == "[DONE]" {
                    continue;
                }

                let json_data = match serde_json::from_str::<Value>(data_payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Capture finish_reason from the final chunk
                if let Some(reason) = json_data
                    .pointer("/choices/0/finish_reason")
                    .and_then(|v| v.as_str())
                {
                    _finish_reason = Some(reason.to_string());
                    if reason == "length" {
                        tracing::warn!(
                            "Stream finished with reason 'length' — response may be truncated"
                        );
                    }
                }

                // Capture usage from the final chunk (sent when stream_options.include_usage=true)
                if let Some(u) = json_data.get("usage") {
                    if let (Some(pt), Some(ct), Some(tt)) = (
                        u.get("prompt_tokens").and_then(serde_json::Value::as_u64),
                        u.get("completion_tokens")
                            .and_then(serde_json::Value::as_u64),
                        u.get("total_tokens").and_then(serde_json::Value::as_u64),
                    ) {
                        stream_usage = Some(TokenUsage {
                            prompt_tokens: pt as u32,
                            completion_tokens: ct as u32,
                            total_tokens: tt as u32,
                            cached_prompt_tokens: extract_cached_prompt_tokens(u),
                        });
                    }
                }

                let Some(delta) = json_data.pointer("/choices/0/delta") else {
                    continue;
                };

                // Regular content
                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                    // Handle both delta-style and accumulated-style providers:
                    // - OpenAI sends true deltas: "Hi", "!", " I", "'m"
                    // - Some providers (Z.AI/GLM) send accumulated text: "Hi", "Hi!", "Hi! I"
                    // Detect accumulated mode: if new content extends what we've seen so far,
                    // extract only the new suffix as the actual delta.
                    let actual_delta = if !provider_accumulated.is_empty()
                        && content.starts_with(&provider_accumulated)
                    {
                        &content[provider_accumulated.len()..]
                    } else {
                        content
                    };

                    // Update tracking
                    if !provider_accumulated.is_empty()
                        && content.starts_with(&provider_accumulated)
                    {
                        provider_accumulated = content.to_string();
                    } else {
                        provider_accumulated.push_str(content);
                    }

                    if !actual_delta.is_empty() {
                        full_content.push_str(actual_delta);
                        callback(StreamChunk::Token(actual_delta.to_string()));
                    }
                }

                // Reasoning content (for models with thinking enabled)
                if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                    reasoning_content.push_str(reasoning);
                    callback(StreamChunk::Reasoning(reasoning.to_string()));
                }

                // Tool calls — accumulate deltas by index
                if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
                    for call in calls {
                        let index = call
                            .get("index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);

                        let acc =
                            tool_accumulators
                                .entry(index)
                                .or_insert_with(|| ToolCallAccumulator {
                                    id: String::new(),
                                    name: String::new(),
                                    arguments: String::new(),
                                });

                        // First delta for this index carries the id and name
                        if let Some(id) = call.get("id").and_then(|i| i.as_str()) {
                            if !id.is_empty() {
                                acc.id = id.to_string();
                            }
                        }
                        if let Some(name) = call
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                        {
                            if !name.is_empty() {
                                acc.name = name.to_string();
                            }
                        }

                        // Every delta may carry an argument fragment — append it
                        if let Some(args_fragment) = call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                        {
                            acc.arguments.push_str(args_fragment);
                        }

                        // Emit StreamChunk::ToolCall for UI feedback
                        callback(StreamChunk::ToolCall(ToolCallChunk {
                            id: if acc.id.is_empty() {
                                None
                            } else {
                                Some(acc.id.clone())
                            },
                            name: if acc.name.is_empty() {
                                None
                            } else {
                                Some(acc.name.clone())
                            },
                            arguments: acc.arguments.clone(),
                        }));
                    }
                }
            }
        }

        // Build final tool calls from accumulated deltas
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut indices: Vec<u64> = tool_accumulators.keys().copied().collect();
        indices.sort_unstable();
        for index in indices {
            if let Some(acc) = tool_accumulators.remove(&index) {
                if acc.name.is_empty() {
                    tracing::warn!("Skipping tool call at index {} with empty name", index);
                    continue;
                }
                let args_value = if is_json_complete(&acc.arguments) {
                    match serde_json::from_str::<serde_json::Value>(&acc.arguments) {
                        Ok(args) => args,
                        Err(e) => {
                            // Attempt recovery: model may have concatenated multiple JSON objects
                            if let Some(recovered) = recover_first_json(&acc.arguments) {
                                tracing::info!(
                                    "Recovered first JSON from concatenated tool call '{}' — \
                                     original had trailing data after first object",
                                    acc.name
                                );
                                recovered
                            } else {
                                tracing::warn!(
                                    "Failed to parse tool call arguments for '{}': {} — raw: {}",
                                    acc.name,
                                    e,
                                    &acc.arguments[..acc.arguments.len().min(200)]
                                );
                                json!({
                                    "__error__": "PARSE_ERROR",
                                    "__message__": "Only one tool call per response. Send one tool call, wait for the result, then call the next.",
                                    "__truncated__": false
                                })
                            }
                        }
                    }
                } else {
                    tracing::error!(
                        "Tool '{}' arguments JSON is incomplete (truncated). Args (first 200): '{}'",
                        acc.name, &acc.arguments[..acc.arguments.len().min(200)]
                    );
                    json!({
                        "__error__": "TRUNCATED_ARGUMENTS",
                        "__message__": "Tool call arguments were truncated. Try a shorter command or split into multiple calls.",
                        "__original_length__": acc.arguments.len(),
                        "__truncated__": true
                    })
                };
                tool_calls.push(ToolCall::new(acc.id, acc.name, args_value));
            }
        }

        // Use provider-reported usage if available, otherwise estimate from character count
        let usage = stream_usage.unwrap_or_else(|| {
            let estimated_completion = (full_content.len() + reasoning_content.len()) as u32 / 4;
            TokenUsage {
                prompt_tokens: 0,
                completion_tokens: estimated_completion,
                total_tokens: estimated_completion,
                cached_prompt_tokens: None,
            }
        });

        log_cache_hit("stream", &usage);

        tracing::info!(
            "Streaming completed: prompt={} completion={} total={} tokens, {} tool calls",
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens,
            tool_calls.len()
        );

        Ok(ChatResponse {
            content: full_content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            reasoning: if reasoning_content.is_empty() {
                None
            } else {
                Some(reasoning_content)
            },
            usage: Some(usage),
        })
    }

    fn supports_reasoning(&self) -> bool {
        self.config.thinking_enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{
        completion::{CompletionModel as _, CompletionRequest, Message, ToolDefinition},
        one_or_many::OneOrMany,
    };
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct SharedLog(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedLog {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_openai_client_creation() {
        let config = LlmConfig::new(
            "https://api.openai.com".to_string(),
            "test-key".to_string(),
            "gpt-4".to_string(),
            "openai".to_string(),
        );

        let client = OpenAiClient::new(config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_tool_call_parsing() {
        let tool_call = ToolCall::new(
            "call_123".to_string(),
            "search".to_string(),
            json!({"query": "test"}),
        );

        assert_eq!(tool_call.id, "call_123");
        assert_eq!(tool_call.name, "search");
    }

    // ------------------------------------------------------------------
    // Cache-stability + cache-token parsing tests (Plan A).
    //
    // The cache-stability test is the single most important invariant
    // for upstream prompt caching: any per-call drift in the serialized
    // body invalidates the provider-side cache and is billed as a miss.
    // ------------------------------------------------------------------

    #[test]
    fn parse_tool_calls_skips_unparseable_arguments_without_panicking() {
        // Regression: a tool call whose `arguments` can't be parsed as JSON
        // must be skipped, not panic. Previously the recovery fallback did
        // `serde_json::from_str::<Value>("null").unwrap_err()`, which panics
        // because `from_str("null")` is `Ok(Null)`. This was latent until the
        // Rig extractor (intent analysis) started sending a `submit` tool and
        // the model occasionally returned malformed submit arguments.
        let client = test_client();
        let response = serde_json::json!({
            "choices": [{"message": {"tool_calls": [
                {"id": "call_bad", "function": {"name": "submit", "arguments": "not valid json {{"}},
                {"id": "call_ok", "function": {"name": "submit", "arguments": "{\"x\":1}"}}
            ]}}]
        });
        let calls = client.parse_tool_calls(&response);
        assert_eq!(
            calls.len(),
            1,
            "the malformed-args call must be skipped, the valid one kept: {calls:?}"
        );
        assert_eq!(calls[0].id, "call_ok");
    }

    fn test_client() -> OpenAiClient {
        let config = LlmConfig::new(
            "https://api.openai.com".to_string(),
            "test-key".to_string(),
            "gpt-4-turbo".to_string(),
            "openai".to_string(),
        );
        OpenAiClient::new(config).expect("client")
    }

    fn fixture_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("You are a helpful assistant.".to_string()),
            ChatMessage::user("What is 2 + 2?".to_string()),
        ]
    }

    fn fixture_tools() -> Value {
        json!([
            {
                "type": "function",
                "function": {
                    "name": "calculator",
                    "description": "Evaluate a math expression",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "expression": { "type": "string" }
                        },
                        "required": ["expression"]
                    }
                }
            }
        ])
    }

    fn named_tool(name: &str) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": name,
                "description": "",
                "parameters": {"type": "object"}
            }
        })
    }

    fn tool_with_serialized_size(name: &str, target: usize) -> Value {
        let mut tool = named_tool(name);
        let baseline = serde_json::to_vec(&tool).expect("serialize baseline").len();
        assert!(target >= baseline, "target must fit the valid envelope");
        tool.pointer_mut("/function/description")
            .expect("description")
            .clone_from(&json!("x".repeat(target - baseline)));
        assert_eq!(
            serde_json::to_vec(&tool)
                .expect("serialize sized tool")
                .len(),
            target
        );
        tool
    }

    fn tool_with_utf8_serialized_size(name: &str, target: usize) -> Value {
        let mut tool = named_tool(name);
        let baseline = serde_json::to_vec(&tool).expect("serialize baseline").len();
        assert!(target >= baseline, "target must fit the valid envelope");
        let remaining = target - baseline;
        let ascii_prefix = if remaining.is_multiple_of(2) { "" } else { "x" };
        let description = format!(
            "{ascii_prefix}{}",
            "é".repeat((remaining - ascii_prefix.len()) / 2)
        );
        tool.pointer_mut("/function/description")
            .expect("description")
            .clone_from(&json!(description));
        assert_eq!(
            serde_json::to_vec(&tool)
                .expect("serialize UTF-8 sized tool")
                .len(),
            target
        );
        tool
    }

    fn invalid_tool_rule(tools: Value) -> String {
        match prepare_tools(&tools) {
            Err(LlmError::InvalidRequest(message)) => message,
            Err(other) => panic!("expected InvalidRequest, got {other:?}"),
            Ok(_) => panic!("expected invalid tool inventory to be rejected"),
        }
    }

    // STUB: AC1 — equivalent inventories have one canonical byte representation.
    #[test]
    fn tool_inventories_are_canonicalized_by_function_name() {
        let forward = json!([named_tool("alpha"), named_tool("zeta")]);
        let reverse = json!([named_tool("zeta"), named_tool("alpha")]);

        let (forward, forward_footprint) = prepare_tools(&forward).expect("forward tools");
        let (reverse, reverse_footprint) = prepare_tools(&reverse).expect("reverse tools");

        assert_eq!(forward, reverse);
        assert_eq!(forward_footprint, reverse_footprint);
        assert_eq!(
            forward.pointer("/0/function/name").and_then(Value::as_str),
            Some("alpha")
        );
    }

    #[test]
    fn request_body_canonicalizes_equivalent_tool_inventories() {
        let client = test_client();
        let forward = json!([named_tool("alpha"), named_tool("zeta")]);
        let reverse = json!([named_tool("zeta"), named_tool("alpha")]);

        let forward = client
            .build_request_body(fixture_messages(), Some(forward), None)
            .expect("forward request");
        let reverse = client
            .build_request_body(fixture_messages(), Some(reverse), None)
            .expect("reverse request");

        assert_eq!(
            serde_json::to_vec(&forward).expect("serialize forward request"),
            serde_json::to_vec(&reverse).expect("serialize reverse request")
        );
        assert_eq!(
            forward
                .pointer("/tools/0/function/name")
                .and_then(Value::as_str),
            Some("alpha")
        );
        assert_eq!(
            forward
                .pointer("/tools/1/function/name")
                .and_then(Value::as_str),
            Some("zeta")
        );
    }

    // STUB: AC2/AC5 — malformed authority envelopes fail with redacted rule codes.
    #[test]
    fn malformed_tool_envelopes_are_rejected_without_echoing_input() {
        let secret = "do-not-log-this-secret";
        let invalid_cases = [
            json!({"not": "an array"}),
            json!([{"type": "web_search", "function": {"name": "search"}}]),
            json!([{"type": "function", "function": secret}]),
            json!([{"type": "function", "function": {}}]),
            json!([{"type": "function", "function": {"name": 42}}]),
            json!([{"type": "function", "function": {"name": ""}}]),
            json!([{"type": "function", "function": {"name": "bad.name"}}]),
            json!([{"type": "function", "function": {"name": "x".repeat(65)}}]),
            json!([{"type": "function", "function": {"name": "safe"}, "provider_behavior": true}]),
            json!([{"type": "function", "function": {"name": "safe", "provider_behavior": true}}]),
            json!([{"type": "function", "function": {"name": "safe", "description": 42}}]),
            json!([{"type": "function", "function": {"name": "safe", "parameters": "object"}}]),
            json!([{"type": "function", "function": {"name": "safe", "strict": "true"}}]),
            json!([named_tool(secret), named_tool(secret)]),
        ];

        for tools in invalid_cases {
            let message = invalid_tool_rule(tools);
            assert!(message.starts_with("tool_schema_rule="));
            assert!(!message.contains(secret));
            assert!(!message.contains("bad.name"));
        }
    }

    // STUB: AC2 — the complete provider-compatible name boundary is accepted.
    #[test]
    fn tool_name_exact_boundary_is_accepted() {
        let name = "a".repeat(64);
        let mut tool = named_tool(&name);
        tool.pointer_mut("/function")
            .and_then(Value::as_object_mut)
            .expect("function")
            .insert("strict".to_string(), json!(true));
        let (tools, _) = prepare_tools(&json!([tool])).expect("64-byte name with strict flag");
        assert_eq!(
            tools.pointer("/0/function/name").and_then(Value::as_str),
            Some(name.as_str())
        );
    }

    // STUB: AC3 — count, item, and aggregate byte boundaries are fail-closed.
    #[test]
    fn tool_inventory_limits_accept_exact_boundaries_and_reject_above() {
        let exact_count = Value::Array(
            (0..MAX_MODEL_VISIBLE_TOOLS)
                .map(|index| named_tool(&format!("tool_{index}")))
                .collect(),
        );
        prepare_tools(&exact_count).expect("exact tool-count boundary");
        let above_count = Value::Array(
            (0..=MAX_MODEL_VISIBLE_TOOLS)
                .map(|index| named_tool(&format!("tool_{index}")))
                .collect(),
        );
        assert!(invalid_tool_rule(above_count).contains("tool_count_limit"));

        let exact_item = json!([tool_with_serialized_size(
            "exact_item",
            MAX_SINGLE_TOOL_SCHEMA_BYTES
        )]);
        prepare_tools(&exact_item).expect("exact per-tool byte boundary");
        let above_item = json!([tool_with_serialized_size(
            "above_item",
            MAX_SINGLE_TOOL_SCHEMA_BYTES + 1
        )]);
        assert!(invalid_tool_rule(above_item).contains("tool_bytes_limit"));

        let exact_total = json!([
            tool_with_serialized_size("a", MAX_SINGLE_TOOL_SCHEMA_BYTES),
            tool_with_serialized_size("b", MAX_SINGLE_TOOL_SCHEMA_BYTES),
            tool_with_serialized_size("c", MAX_SINGLE_TOOL_SCHEMA_BYTES),
            tool_with_serialized_size("d", MAX_SINGLE_TOOL_SCHEMA_BYTES - 5)
        ]);
        assert_eq!(
            serde_json::to_vec(&exact_total)
                .expect("serialize exact total")
                .len(),
            MAX_TOOL_SCHEMA_BYTES
        );
        prepare_tools(&exact_total).expect("exact aggregate byte boundary");

        let above_total = json!([
            tool_with_serialized_size("a", MAX_SINGLE_TOOL_SCHEMA_BYTES),
            tool_with_serialized_size("b", MAX_SINGLE_TOOL_SCHEMA_BYTES),
            tool_with_serialized_size("c", MAX_SINGLE_TOOL_SCHEMA_BYTES),
            tool_with_serialized_size("d", MAX_SINGLE_TOOL_SCHEMA_BYTES - 4)
        ]);
        assert_eq!(
            serde_json::to_vec(&above_total)
                .expect("serialize above total")
                .len(),
            MAX_TOOL_SCHEMA_BYTES + 1
        );
        assert!(invalid_tool_rule(above_total).contains("total_bytes_limit"));
    }

    #[test]
    fn tool_inventory_limits_count_serialized_utf8_bytes() {
        let unicode_tool = json!([{
            "type": "function",
            "function": {
                "name": "unicode",
                "description": "ééé",
                "parameters": {"type": "object"}
            }
        }]);
        let (canonical, footprint) = prepare_tools(&unicode_tool).expect("unicode tool");
        let serialized = serde_json::to_vec(&canonical).expect("serialize unicode tool array");
        assert_eq!(footprint.serialized_bytes, serialized.len());
        assert!(
            serialized.len()
                > String::from_utf8(serialized.clone())
                    .unwrap()
                    .chars()
                    .count()
        );

        let exact = json!([tool_with_utf8_serialized_size(
            "unicode_exact",
            MAX_SINGLE_TOOL_SCHEMA_BYTES
        )]);
        prepare_tools(&exact).expect("exact UTF-8 byte boundary");

        let above = json!([tool_with_utf8_serialized_size(
            "unicode_above",
            MAX_SINGLE_TOOL_SCHEMA_BYTES + 1
        )]);
        assert!(invalid_tool_rule(above).contains("tool_bytes_limit"));
    }

    // STUB: AC4 — aggregate metadata is exact and contains only a digest.
    #[test]
    fn tool_footprint_is_deterministic_and_exact() {
        let input = json!([named_tool("zeta"), named_tool("alpha")]);
        let (canonical, footprint) = prepare_tools(&input).expect("tools");
        let bytes = serde_json::to_vec(&canonical).expect("serialize canonical tools");

        assert_eq!(footprint.tool_count, 2);
        assert_eq!(footprint.serialized_bytes, bytes.len());
        assert_eq!(
            footprint.estimated_tokens,
            bytes.len().saturating_add(3) / 4
        );
        assert_eq!(footprint.sha256, sha256_hex(&bytes));
        assert_eq!(footprint.sha256.len(), 64);
        assert!(footprint
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert!(!footprint.sha256.contains("alpha"));
    }

    // STUB: AC3 — bounded measurement stops as soon as the limit is crossed.
    #[test]
    fn bounded_hash_writer_stops_at_limit_plus_one() {
        let mut writer = BoundedHashWriter::new(4);
        assert_eq!(writer.write(b"abcd").expect("exact boundary"), 4);

        let error = writer.write(b"ef").expect_err("fifth byte must fail");

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(writer.observed, 5);
        assert!(writer.limit_exceeded);
    }

    // STUB: AC4/AC5 — one aggregate-only diagnostic is emitted per inventory.
    #[test]
    #[ignore = "manual tracing capture must run in an isolated test process"]
    fn tool_footprint_log_is_single_and_redacted() {
        let client = test_client();
        let secret = "schema-secret-must-not-appear";
        let tools = json!([{
            "type": "function",
            "function": {
                "name": "safe_tool",
                "description": secret,
                "parameters": {"type": "object", "x-secret": secret}
            }
        }]);
        let (_, footprint) = prepare_tools(&tools).expect("footprint");
        let captured = SharedLog::default();
        let sink = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || sink.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            client
                .build_request_body(fixture_messages(), Some(tools), None)
                .expect("request body");
        });

        let output = String::from_utf8(captured.0.lock().expect("log buffer").clone())
            .expect("UTF-8 log output");
        assert_eq!(output.matches("LLM tool schema footprint").count(), 1);
        assert!(output.contains("tool_count=1"));
        assert!(output.contains(&format!("serialized_bytes={}", footprint.serialized_bytes)));
        assert!(output.contains(&format!("estimated_tokens={}", footprint.estimated_tokens)));
        assert!(output.contains(&format!("sha256={}", footprint.sha256)));
        assert!(!output.contains(secret));
        assert!(!output.contains("safe_tool"));
    }

    // STUB: AC2 — validation propagates through the client before network I/O.
    #[tokio::test]
    async fn chat_rejects_invalid_tools_before_network_io() {
        let client = test_client();

        let error = client
            .chat(
                fixture_messages(),
                Some(json!([{"type": "web_search", "name": "unsafe"}])),
            )
            .await
            .expect_err("provider-native tools must fail locally");

        match error {
            LlmError::InvalidRequest(message) => {
                assert_eq!(message, "tool_schema_rule=tool_type");
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn structured_and_streaming_chat_reject_invalid_tools_before_network_io() {
        let client = test_client();
        let invalid_tools = || Some(json!([{"type": "web_search", "name": "unsafe"}]));

        let structured = client
            .chat_with_schema(
                fixture_messages(),
                invalid_tools(),
                Some(json!({"type": "object"})),
            )
            .await
            .expect_err("structured chat must reject before network I/O");
        assert!(matches!(structured, LlmError::InvalidRequest(_)));

        let streaming = client
            .chat_stream(fixture_messages(), invalid_tools(), Box::new(|_| {}))
            .await
            .expect_err("streaming chat must reject before network I/O");
        assert!(matches!(streaming, LlmError::InvalidRequest(_)));

        // The fallback path reuses an immutable clone of the inventory already
        // validated above, so it cannot acquire a newly invalid schema between
        // the streaming and fallback requests.
    }

    fn invalid_rig_request() -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::one(Message::user("hello".to_string())),
            documents: Vec::new(),
            tools: vec![ToolDefinition {
                name: "x".repeat(65),
                description: "invalid provider tool name".to_string(),
                parameters: json!({"type": "object"}),
            }],
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    #[tokio::test]
    async fn rig_completion_and_stream_reject_invalid_tools_before_network_io() {
        let model = crate::rig_adapter::model::LlmCompletionModel::new(
            Arc::new(test_client()) as Arc<dyn LlmClient>,
            "gpt-4-turbo",
        );

        let completion = model
            .completion(invalid_rig_request())
            .await
            .expect_err("Rig completion must surface local validation");
        assert!(completion
            .to_string()
            .contains("tool_schema_rule=function_name"));

        let mut stream = model
            .stream(invalid_rig_request())
            .await
            .expect("Rig stream should initialize");
        let stream_error = stream
            .next()
            .await
            .expect("Rig stream must surface an error")
            .expect_err("Rig stream must reject the invalid tool");
        assert!(stream_error
            .to_string()
            .contains("tool_schema_rule=function_name"));
    }

    #[test]
    fn request_body_is_byte_stable_across_identical_calls() {
        let client = test_client();
        let a = client
            .build_request_body(fixture_messages(), Some(fixture_tools()), None)
            .expect("request a");
        let b = client
            .build_request_body(fixture_messages(), Some(fixture_tools()), None)
            .expect("request b");

        let a_bytes = serde_json::to_vec(&a).expect("serialize a");
        let b_bytes = serde_json::to_vec(&b).expect("serialize b");

        assert_eq!(
            a_bytes, b_bytes,
            "build_request_body must be byte-stable — per-call drift breaks \
             upstream prompt caching and is billed as a cache miss"
        );
    }

    #[test]
    fn request_body_is_byte_stable_without_tools() {
        // Absence of `tools` must not introduce drift either.
        let client = test_client();
        let a = client
            .build_request_body(fixture_messages(), None, None)
            .expect("request a");
        let b = client
            .build_request_body(fixture_messages(), None, None)
            .expect("request b");

        let a_bytes = serde_json::to_vec(&a).expect("serialize a");
        let b_bytes = serde_json::to_vec(&b).expect("serialize b");
        assert_eq!(a_bytes, b_bytes);
    }

    #[test]
    fn request_body_includes_json_schema_response_format() {
        let client = test_client();
        let schema = json!({
            "type": "object",
            "properties": {
                "intent": { "type": "string" }
            },
            "required": ["intent"]
        });

        let body = client
            .build_request_body(fixture_messages(), None, Some(schema.clone()))
            .expect("structured request");

        assert_eq!(
            body.pointer("/response_format/type")
                .and_then(Value::as_str),
            Some("json_schema")
        );
        assert_eq!(
            body.pointer("/response_format/json_schema/strict")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.pointer("/response_format/json_schema/schema"),
            Some(&schema)
        );
    }

    #[test]
    fn extract_cached_prompt_tokens_reads_openai_shape() {
        let usage = json!({
            "prompt_tokens": 1200,
            "completion_tokens": 50,
            "total_tokens": 1250,
            "prompt_tokens_details": { "cached_tokens": 1024 }
        });
        assert_eq!(extract_cached_prompt_tokens(&usage), Some(1024));
    }

    #[test]
    fn extract_cached_prompt_tokens_reads_glm_deepseek_fallback() {
        // GLM / DeepSeek / z.ai report a flat `prompt_cache_hit_tokens`.
        let usage = json!({
            "prompt_tokens": 2048,
            "completion_tokens": 40,
            "total_tokens": 2088,
            "prompt_cache_hit_tokens": 1800
        });
        assert_eq!(extract_cached_prompt_tokens(&usage), Some(1800));
    }

    #[test]
    fn extract_cached_prompt_tokens_returns_none_when_unreported() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120
        });
        assert_eq!(extract_cached_prompt_tokens(&usage), None);
    }

    #[test]
    fn extract_cached_prompt_tokens_prefers_openai_shape_when_both_present() {
        // If a proxy somehow fills both fields, the nested OpenAI shape
        // wins — it's the documented source of truth on OpenAI proper.
        let usage = json!({
            "prompt_tokens": 500,
            "prompt_tokens_details": { "cached_tokens": 400 },
            "prompt_cache_hit_tokens": 300
        });
        assert_eq!(extract_cached_prompt_tokens(&usage), Some(400));
    }

    #[test]
    fn stream_transport_error_rejects_partial_success() {
        let err = stream_transport_error(86, 0, "connection closed");

        match err {
            LlmError::ApiError(message) => {
                assert!(message.contains("terminated after 86 chars"));
                assert!(message.contains("refusing to treat partial content as complete"));
            }
            other => panic!("expected ApiError, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod json_recovery_tests {
    use super::*;

    #[test]
    fn test_recover_concatenated_json() {
        let raw = r#"{"action":"recall","query":"test"}{"title":"My Title"}{"action":"use"}"#;
        let result = recover_first_json(raw);
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["action"], "recall");
        assert_eq!(val["query"], "test");
    }

    #[test]
    fn test_recover_single_json_returns_none() {
        let raw = r#"{"action":"recall","query":"test"}"#;
        let result = recover_first_json(raw);
        assert!(result.is_none());
    }

    #[test]
    fn test_recover_invalid_json_returns_none() {
        let raw = r"not json at all";
        let result = recover_first_json(raw);
        assert!(result.is_none());
    }

    #[test]
    fn test_recover_nested_braces() {
        let raw = r#"{"args":{"nested":"value"}}{"second":"obj"}"#;
        let result = recover_first_json(raw);
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["args"]["nested"], "value");
    }
}
