# Agent Tools

Built-in tool implementations for AgentZero agents. Organized by category and registered via `core_tools()`, `optional_tools()`, and `builtin_tools_with_fs()`.

## Build & Test

```bash
cargo test -p agent-tools      # 25 tests
```

## Tool Modules

| Module | Key Tools |
|--------|-----------|
| `execution/shell.rs` | `ShellTool` — shell commands (cwd = ward dir, uses venv) |
| `execution/write_file.rs` | `WriteFileTool` — create/overwrite files in ward |
| `execution/edit_file.rs` | `EditFileTool` — targeted find-and-replace |
| `execution/skills.rs` | `LoadSkillTool` |
| `execution/update_plan.rs` | `UpdatePlanTool` — lightweight task checklist |
| `execution/graph.rs` | `ExecutionGraphTool` — DAG workflow engine |
| `file.rs` | `ReadTool`, `WriteTool`, `EditTool` (optional file-tools group) |
| `search.rs` | `GlobTool` |
| `ward.rs` | `WardTool` — ward use/list/create/info; emits `WardChanged` |
| `memory.rs` | `MemoryTool` — persistent key-value (shared/agent/ward scopes) |
| `web.rs` | `WebFetchTool` (optional) |
| `graph_query.rs` | `GraphQueryTool` — query knowledge graph entities/relationships |
| `goal.rs` | `GoalTool` — agent intent lifecycle |
| `ingest.rs` | `IngestTool` — enqueue text for background extraction |
| `ui.rs` | `RequestInputTool`, `ShowContentTool` (optional) |
| `agent.rs` | `ListAgentsTool`, `CreateAgentTool` (optional) |
| `multimodal.rs` | `MultimodalAnalyzeTool` — vision fallback |
| `connectors.rs` | `ConnectorResourceTool`, `ConnectorInvokeTool`, `QueryResourceTool` compatibility wrapper |

## Registration Functions

```rust
pub fn core_tools(fs: Arc<dyn FileSystemContext>, fact_store: Option<Arc<dyn MemoryFactStore>>)
    -> Vec<Arc<dyn Tool>>
pub fn optional_tools(fs: Arc<dyn FileSystemContext>, settings: &ToolSettings)
    -> Vec<Arc<dyn Tool>>
pub fn builtin_tools_with_fs(fs: Arc<dyn FileSystemContext>) -> Vec<Arc<dyn Tool>>
```

Core tools are always enabled. Optional tools depend on `ToolSettings` boolean flags:
`file_tools`, `python`, `web_fetch`, `ui_tools`, `create_agent`.

## Security / Guards

`tools/guards.rs` (re-exported from crate root):
- Path sanitization: rejects `..`, absolute paths; resolves relative to ward or agent data dir
- Network blocking: `WebFetchTool` blocks localhost, private networks, cloud metadata
- Shell safety: blocks dangerous commands, enforces timeouts

## Key Intra-Repo Dependencies

- `agent-primitives` — `Tool`, `ToolContext`, `FileSystemContext`
- `zbot-stores-traits` — `MemoryFactStore` (for `MemoryTool`, `WardTool`)
