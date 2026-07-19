# LLM Client — Text & Multimodal Content

## Purpose

The LLM client layer carries content between agents and LLM providers. It supports text, images, and files as first-class content types, with capability-aware encoding that prevents sending visual content to text-only models.

## When It Runs

- **Every LLM call** — content encoding happens in `OpenAiClient.build_request_body()`
- **Before DB persistence** — base64 flush converts inline blobs to disk-backed file references
- **On tool call** — `multimodal_analyze` makes direct one-shot LLM calls to the configured vision model

## Design Principles

1. **Primitive types as backbone** — carry content without owning provider transport
2. **Intelligence in agents** — planning agent decides routing, sharding, delegation
3. **No silent degradation** — unsupported content returns clear errors; agent adapts
4. **No DB bloat** — base64 blobs flushed to content-addressed files on disk

## Content Model

Messages carry `Vec<Part>` where Part is:

| Part | Fields | OpenAI Wire Format |
|------|--------|-------------------|
| `Text` | `text` | `{ "type": "text", "text": "..." }` |
| `Image` | `source, mime_type, detail` | `{ "type": "image_url", "image_url": { "url": "data:...;base64,...", "detail": "high" } }` |
| `File` | `source, mime_type, filename` | `{ "type": "file", "file": { "url": "data:...;base64,..." } }` |
| `FunctionCall` | `name, args, id` | Separate `tool_calls` field |
| `FunctionResponse` | `id, response` | Separate tool result message |

### ContentSource

| Variant | Stored in DB | Sent to LLM |
|---------|-------------|-------------|
| `Url(String)` | Yes | Yes (provider fetches) |
| `Base64(String)` | Never (flushed first) | Yes (inline) |
| `FileRef(String)` | Yes | Rehydrated to Base64 first |

### ImageDetail

- `Low` — 512px, fast, fewer tokens
- `High` — full resolution with tiling
- `Auto` — provider decides

## Backward Compatibility

`ChatMessage` has custom serde:
- **Serialize**: text-only → `"content": "hello"` (plain string); multimodal → `"content": [{ "type": "text", ... }, { "type": "image_url", ... }]`
- **Deserialize**: accepts both string and array (reads old DB records)

## Provider Encoding

`OpenAiClient` owns the OpenAI-compatible wire encoding. It converts text-only
messages to the legacy plain-string content shape and multimodal messages to
provider content blocks after rehydrating any `FileRef` sources from disk.

## Content Persistence

```
Inbound:  Part::Image { Base64("...") }
  → flush_part_to_disk()
  → Part::Image { FileRef("/attachments/{sha256}.png") }  ← DB stores this
  → rehydrate_source()
  → Part::Image { Base64("...") }  ← LLM receives this
```

Content-addressed storage (SHA-256 hash as filename) provides automatic deduplication.

## Multimodal Processing Paths

| Priority | Path | When |
|----------|------|------|
| 1 | **Native** | Agent runs on a vision model — Parts flow directly |
| 2 | **Specialist** | Delegate to domain-expert agent (doc-shard, etc.) |
| 3 | **Tool fallback** | `multimodal_analyze` — one-shot call to default vision model |
| 4 | **Error** | No vision capability configured — clear message |

## `multimodal_analyze` Tool

Universal vision fallback available to ALL agents (root + subagents). Makes a direct HTTP POST to the configured multimodal provider.

**Flow:**
1. Tool reads `multimodal_config` from executor state (injected by `ExecutorBuilder`)
2. Resolves file paths to base64, builds OpenAI content array
3. `POST {baseUrl}/chat/completions` with vision content
4. Returns `{ "analysis": "..." }` or structured JSON if `output_schema` provided

**Config:** Settings > Advanced > Multimodal (provider + vision-capable model)

**Skill:** `eagle-eye` teaches agents when and how to use the tool

## Configuration

`settings.json > execution.multimodal`:
```json
{
  "providerId": "provider-ollama",
  "model": "gemma4:31b-cloud",
  "temperature": 0.3,
  "maxTokens": 4096
}
```

Provider credentials (baseUrl, apiKey) resolved from `config/providers.json` at executor build time.

## Implementation Files

| File | Purpose |
|------|---------|
| `runtime/agent-primitives/src/types.rs` | Part enum, ContentSource, ImageDetail |
| `runtime/agent-primitives/src/multimodal.rs` | flush_part_to_disk, rehydrate_source, MIME utils |
| `runtime/agent-runtime/src/types/messages.rs` | ChatMessage with Vec<Part>, custom serde |
| `runtime/agent-runtime/src/llm/openai.rs` | OpenAiClient with FileRef rehydration and OpenAI-compatible content encoding |
| `runtime/agent-tools/src/tools/multimodal.rs` | multimodal_analyze tool |
| `gateway/gateway-services/src/settings.rs` | MultimodalConfig in ExecutionSettings |
| `gateway/gateway-execution/src/invoke/executor.rs` | Injects multimodal_config into executor state |
| `gateway/templates/skills/eagle-eye/SKILL.md` | Eagle Eye visual intelligence skill |
