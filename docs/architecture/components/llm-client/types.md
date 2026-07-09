# LLM Client — Types Reference

## Backend Types (Rust)

### Part — `runtime/agent-primitives/src/types.rs`

Content element. Tagged enum with `#[serde(tag = "type")]`.

```rust
pub enum Part {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "function_call")]
    FunctionCall {
        name: String,
        args: Value,
        id: Option<String>,
    },

    #[serde(rename = "function_response")]
    FunctionResponse {
        id: String,
        response: String,
    },

    #[serde(rename = "image")]
    Image {
        source: ContentSource,
        mime_type: String,
        detail: Option<ImageDetail>,  // skip_serializing_if None
    },

    #[serde(rename = "file")]
    File {
        source: ContentSource,
        mime_type: String,
        filename: Option<String>,  // skip_serializing_if None
    },
}
```

Helper methods:
- `Part::text(s)` — create Text part
- `Part::function_call(name, args)` — create FunctionCall
- `Part::type_name() -> &'static str` — "text", "image", "file", etc.
- `Part::is_multimodal() -> bool` — true for Image/File

### ContentSource — `runtime/agent-primitives/src/types.rs`

Where content bytes live. Tagged with `#[serde(tag = "type", content = "value")]`.

```rust
pub enum ContentSource {
    #[serde(rename = "url")]
    Url(String),         // Remote URL — stored as-is in DB and sent to LLM

    #[serde(rename = "base64")]
    Base64(String),      // Inline bytes — ephemeral, NEVER persisted to DB

    #[serde(rename = "file_ref")]
    FileRef(String),     // Local path — what DB stores after flush
}
```

### ImageDetail — `runtime/agent-primitives/src/types.rs`

```rust
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Low,   // 512px, fast
    High,  // Full resolution, tiling
    Auto,  // Provider decides
}
```

### ChatMessage — `runtime/agent-runtime/src/types/messages.rs`

Custom Serialize/Deserialize (NOT derive). Text-only → string, multimodal → array.

```rust
pub struct ChatMessage {
    pub role: String,              // "system" | "user" | "assistant" | "tool"
    pub content: Vec<Part>,        // Was String — now multimodal
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}
```

Factory methods:
- `ChatMessage::user(content: String)` → wraps in `vec![Part::Text{...}]`
- `ChatMessage::assistant(content: String)`
- `ChatMessage::system(content: String)`
- `ChatMessage::tool_result(id: String, content: String)`

Accessors:
- `text_content() -> String` — joins all Text parts with `\n`
- `has_multimodal_content() -> bool` — any Image/File parts?

### OpenAI-Compatible Encoding — `runtime/agent-runtime/src/llm/openai.rs`

`OpenAiClient` encodes `ChatMessage.content: Vec<Part>` directly while building
the request body. Text-only content remains a plain string for compatibility;
multimodal content becomes OpenAI-compatible content blocks after `FileRef`
sources are rehydrated through `agent_primitives::multimodal::rehydrate_source`.

### MultimodalConfig — `gateway/gateway-services/src/settings.rs`

```rust
#[serde(rename_all = "camelCase")]
pub struct MultimodalConfig {
    pub provider_id: Option<String>,   // Provider with vision model
    pub model: Option<String>,         // Vision-capable model ID
    pub temperature: f64,              // Default: 0.3
    pub max_tokens: u32,               // Default: 4096
}
```

## Frontend Types (TypeScript)

### MultimodalConfig — `apps/ui/src/services/transport/types.ts`

```typescript
export interface MultimodalConfig {
    providerId?: string | null;
    model?: string | null;
    temperature: number;    // default 0.3
    maxTokens: number;      // default 4096
}
```

Part of `ExecutionSettings`:
```typescript
export interface ExecutionSettings {
    // ... other fields ...
    multimodal?: MultimodalConfig;
}
```

## OpenAI Wire Format Mapping

| Part Type | `supports_part` check | JSON output |
|-----------|-----------------------|-------------|
| `Text` | Always true | `{ "type": "text", "text": "..." }` |
| `Image` (Base64) | `capabilities.vision` | `{ "type": "image_url", "image_url": { "url": "data:{mime};base64,{data}", "detail": "auto" } }` |
| `Image` (Url) | `capabilities.vision` | `{ "type": "image_url", "image_url": { "url": "https://...", "detail": "high" } }` |
| `Image` (FileRef) | `capabilities.vision` | Read → Base64 → same as Base64 above |
| `File` (Base64) | `capabilities.vision` | `{ "type": "file", "file": { "url": "data:{mime};base64,{data}" } }` |
| `File` (Url) | `capabilities.vision` | `{ "type": "file", "file": { "url": "https://..." } }` |
| `FunctionCall` | `capabilities.tools` | Not in content — separate `tool_calls` field |
| `FunctionResponse` | `capabilities.tools` | Not in content — separate tool result message |

## Executor State Keys

| Key | Set By | Read By | Value |
|-----|--------|---------|-------|
| `multimodal_config` | `ExecutorBuilder.build()` | `multimodal_analyze` tool | `{ providerId, model, temperature, maxTokens, baseUrl, apiKey }` |
