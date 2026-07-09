# LLM Client — Data Flow

## Message Lifecycle (Text-Only)

```
Agent builds ChatMessage::user("hello")
  │
  ▼
ChatMessage { role: "user", content: [Part::Text { text: "hello" }] }
  │
  ▼
Executor calls llm_client.chat_stream(messages, tools, callback)
  │
  ▼
OpenAiClient.build_request_body(messages, tools)
  ├── rehydrate_messages() — no-op for text-only
  └── serde serialize ChatMessage
      └── Custom Serialize: text-only → "content": "hello" (plain string)
  │
  ▼
POST {baseUrl}/chat/completions
  │
  ▼
Streaming response → tokens → ChatResponse { content, tool_calls }
```

## Message Lifecycle (Multimodal)

```
Agent builds multimodal message
  │
  ▼
ChatMessage {
  role: "user",
  content: [
    Part::Text { text: "What's in this image?" },
    Part::Image { source: Base64("..."), mime_type: "image/png", detail: Some(High) }
  ]
}
  │
  ▼
OpenAiClient.build_request_body()
  ├── rehydrate_messages()
  │   └── FileRef → read disk → Base64 (only if FileRef present)
  └── serde serialize ChatMessage
      └── Custom Serialize: has multimodal → "content": [
            { "type": "text", "text": "..." },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,...", "detail": "high" } }
          ]
  │
  ▼
POST {baseUrl}/chat/completions
```

## Base64 Flush (Before DB Persistence)

```
ChatMessage with Part::Image { source: Base64("huge blob...") }
  │
  ▼
flush_part_to_disk(part, attachments_dir)
  ├── Decode base64 → raw bytes
  ├── SHA-256 hash → "a1b2c3d4..."
  ├── Write to {attachments_dir}/a1b2c3d4.png (skip if exists = dedup)
  └── Return Part::Image { source: FileRef("/attachments/a1b2c3d4.png") }
  │
  ▼
DB stores: content = [Part::Text{...}, Part::Image{FileRef("...")}]
  (no base64 blobs in DB)
```

## Rehydration (Before LLM Call)

```
Message loaded from DB: Part::Image { source: FileRef("/attachments/a1b2c3d4.png") }
  │
  ▼
rehydrate_source(&FileRef(path))
  ├── std::fs::read(path) → raw bytes
  └── base64::encode → ContentSource::Base64("...")
  │
  ▼
Part::Image { source: Base64("...") } → ready for API encoding
```

## multimodal_analyze Tool Flow

```
Agent calls multimodal_analyze({ content: [{type: "image", source: "/path/to/img.png"}], prompt: "..." })
  │
  ▼
Tool.execute()
  ├── ctx.get_state("multimodal_config")  ← injected by ExecutorBuilder
  │   └── { baseUrl, apiKey, model, temperature, maxTokens }
  ├── resolve_source("/path/to/img.png")
  │   └── Read file → base64 encode → ContentSource::Base64("...")
  ├── Build OpenAI content array:
  │   └── [{ type: "text", text: prompt }, { type: "image_url", image_url: { url: "data:..." } }]
  └── POST {baseUrl}/chat/completions (direct reqwest call)
  │
  ▼
Response: { choices: [{ message: { content: "I see a ..." } }] }
  │
  ▼
Return to agent: { "analysis": "I see a ..." }
```

## Config Injection Flow

```
Daemon starts
  │
  ▼
ExecutorBuilder.build() called for each agent session
  ├── SettingsService::new_legacy(vault_dir).load()
  │   └── Reads ~/Documents/zbot/config/settings.json
  │   └── Extracts execution.multimodal { providerId, model, temperature, maxTokens }
  ├── Read ~/Documents/zbot/config/providers.json
  │   └── Find provider by ID → extract baseUrl, apiKey
  └── executor_config.with_initial_state("multimodal_config", { ... })
  │
  ▼
Executor state contains "multimodal_config"
  │
  ▼
multimodal_analyze tool reads via ctx.get_state("multimodal_config")
```

## Backward Compatibility — Deserialization

```
Old DB record:  {"role":"user","content":"hello"}
  │
  ▼
Custom Deserialize: content is Value::String
  └── Convert to vec![Part::Text { text: "hello" }]

New DB record:  {"role":"user","content":[{"type":"text","text":"hello"}]}
  │
  ▼
Custom Deserialize: content is Value::Array
  └── Deserialize as Vec<Part>
```
