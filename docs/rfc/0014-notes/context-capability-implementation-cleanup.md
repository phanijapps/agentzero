# Context Capability Implementation Cleanup

Date: 2026-07-07

## Shipped Surface

- `/api/tools` is backed by the live first-party registry and actor policy.
- Unified recall can produce `ContextAtom`s and gateway execution can assemble
  and render bounded `ContextPacket`s with selected/dropped trace metadata.
- Micro-recall now records typed `ContextPacketDelta`s for tool errors, ward
  entry, pre-delegation, delegation callback, and entity mentions.
- `load_skill(skill=...)` returns a bounded skill packet with handles; full body
  access requires an explicit skill file handle.
- Session titles are derived by runtime service and persisted/published without
  a model-visible title tool.
- `wait_agent` remains registered for parallel joins and is marked
  `default_visible=false` with `visible_when_parallel_children_active`.

## Deleted Or Quarantined

- Removed retired model-visible tool implementations from `agent-tools`:
  `list_tools`, `list_skills`, `list_mcps`, `set_session_title`, and `todos`.
- Removed the retired tools from `core_tools`, `optional_tools`,
  `builtin_tools_with_fs`, active gateway templates, and default model-facing
  registry construction.
- Removed `AppState.knowledge_db`, gateway boot SQLite semantic reindex, and
  SQLite KG backfill hooks from production runtime composition.
- Memory stats now size configured Engram storage instead of `knowledge.db`.
- Embedding configure/health now use backend-neutral semantics instead of the
  old sqlite-vec table fallback.

## Allowlist

The following references are expected and are not model-visible tool support:

- MCP protocol methods named `list_tools`.
- HTTP route handlers named `list_tools`, `list_skills`, and `list_mcps`.
- Historical `set_session_title` log replay so old conversation DBs keep titles.
- RFC/spec/backlog text and e2e fixtures that document old baseline behavior.
- SQLite semantic store tests and migration/reference code inside store or
  memory test modules.

`tools/context_capability_cleanup.py` enforces the active deny-list for prompts,
model registry construction, retired `agent-tools` implementations, and
production SQLite semantic startup/reindex hooks.

## Still Open

- Complete live MCP/connector/resource/memory/graph catalog population:
  `context-capability-resource-catalog-completion`.
- Finish broad read-heavy tool splits:
  `context-capability-broad-tool-split-completion`.
- Route memory writes, resource-read distillation, and tool-result distillation
  through the same evidence intake boundary as `ingest`:
  `context-capability-evidence-intake-completion`.
