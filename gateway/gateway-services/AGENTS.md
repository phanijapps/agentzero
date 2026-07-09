# gateway-services

File-backed configuration services with `RwLock` caching for agents, LLM providers, MCP servers, skills, settings, embeddings, models, and plugins.

## Build & Test

```bash
cargo test -p gateway-services    # 5 tests
```

## Key Types

| Type | Purpose |
|------|---------|
| `AgentService` | Agent config CRUD with `RwLock` cache |
| `ProviderService` | LLM provider management |
| `McpService` | MCP server config management |
| `SkillService` | Skill discovery, loading (`Skill`, `SkillFrontmatter`, `SkillSource`) |
| `SettingsService` | App settings (`AppSettings`, `ChatConfig`, `ExecutionSettings`, `OrchestratorConfig`, `MultimodalConfig`, `DistillationConfig`) |
| `AgentRegistry` | Agent delegation permission registry |
| `EmbeddingService` | Embedding backend (Ollama / local) with curated model list |
| `ModelRegistry` | Fallback-only model metadata registry |
| `OllamaClient` | HTTP client for Ollama API |
| `PluginService` | Plugin config CRUD |
| `VaultPaths` / `SharedVaultPaths` | Path resolution for vault directory |
| `RecallConfig` | Memory recall configuration |
| `LangConfig` | Language-specific configurations |
| `FileWatcher` | Config file change watching |
| `LogSettings` | Logging configuration |

## Common CRUD Pattern

```rust
service.list() -> Vec<Config>
service.get(id) -> Option<Config>
service.create(request) -> Config
service.update(id, request) -> Config
service.delete(id)
```

## File Structure

| File | Purpose |
|------|---------|
| `agents.rs` | `AgentService` |
| `providers.rs` | `ProviderService` |
| `mcp.rs` | `McpService` |
| `skills.rs` | `SkillService`, `Skill`, `WardSetup` |
| `settings.rs` | `SettingsService`, `AppSettings`, `ChatConfig`, `OrchestratorConfig` |
| `agent_registry.rs` | `AgentRegistry` (5 tests) |
| `embedding_service.rs` | `EmbeddingService`, `EmbeddingConfig`, `CuratedModel`, `CURATED_MODELS` |
| `models.rs` | `ModelRegistry` |
| `ollama_client.rs` | `OllamaClient` |
| `plugin_service.rs` | `PluginService` |
| `paths.rs` | `VaultPaths`, `SharedVaultPaths` |
| `recall_config.rs` | `RecallConfig` |
| `lang_config.rs` | `LangConfig` |
| `watcher.rs` | `FileWatcher`, `WatchConfig` |
| `logging.rs` | `LogSettings` |
