# Agent Tools

Concrete model-tool implementations for AgentZero. Runtime registration is owned
by `gateway/gateway-execution/src/invoke/executor.rs`; this crate should not grow
a second registry or factory path.

## Build & Test

```bash
cargo test -p agent-tools
```

## Tool Modules

| Module | Key Tools |
|--------|-----------|
| `execution/shell.rs` | `ShellTool` - shell commands |
| `execution/write_file.rs` | `WriteFileTool` - create or overwrite files |
| `execution/edit_file.rs` | `EditFileTool` - targeted find-and-replace |
| `execution/skills.rs` | `LoadSkillTool` |
| `execution/update_plan.rs` | `UpdatePlanTool` - lightweight task checklist |
| `file.rs` | `ReadTool` |
| `search.rs` | `GlobTool` |
| `ward.rs` | `WardTool` - ward use/list/create/info; emits `WardChanged` |
| `memory.rs` | `MemoryTool`, `MemoryWriteTool` |
| `graph_query.rs` | `GraphQueryTool` - query knowledge graph entities/relationships |
| `goal.rs` | `GoalTool` - agent intent lifecycle |
| `ingest.rs` | `IngestTool` - enqueue text for background extraction |
| `multimodal.rs` | `MultimodalAnalyzeTool` - vision fallback |
| `connectors.rs` | `ConnectorResourceTool`, `ConnectorInvokeTool`, `QueryResourceTool` compatibility wrapper |

## Settings

`ToolSettings` only contains settings consumed by live gateway code:
`file_tools`, `offload_large_results`, and `offload_threshold_tokens`.

Do not add toggles for tools unless `gateway-execution` actually reads them.

## Security / Guards

`tools/guards.rs` is re-exported from the crate root so gateway bootstrap and
tool implementations share the same checks:

- Path sanitization rejects `..` and absolute paths and resolves relative paths
  to ward or agent data directories where appropriate.
- Shell safety blocks dangerous commands and enforces timeouts.

## Key Intra-Repo Dependencies

- `agent-primitives` - `Tool`, `ToolContext`, `FileSystemContext`
- `zbot-stores-traits` - `MemoryFactStore` for memory and ward tools
