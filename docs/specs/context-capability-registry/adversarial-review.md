# Adversarial Review: Context Capability Registry

- **Date:** 2026-07-06
- **Reviewer:** local adversarial pass
- **Target:** `docs/rfc/0014-context-capability-registry-and-context-graph.md`, `docs/specs/context-capability-registry/spec.md`, `docs/specs/context-capability-registry/plan.md`, and the related Engram cutover cleanup contract.

## Verdict

Not clean on first pass. The RFC/spec was directionally right, but too soft on
two terminal invariants:

1. Old model-visible tools could survive indefinitely as compatibility/debug
   paths.
2. The previous SQLite memory/knowledge provider could remain as a selectable
   fallback after Engram parity.

Both undermine the user's stated goal: simplify zbot and make the old tool and
memory framework disappear after migration.

## Findings

### Blocker 1: Hiding obsolete tools is not the same as removing them

The spec said discovery/UI-only tools could be hidden while remaining available
through compatibility/debug paths. The plan also said not to delete tool
implementations. That creates an easy failure mode: the catalog improves, but
the old tools keep leaking back through prompts, settings, templates, replay,
or debug/admin surfaces.

Evidence:

- `docs/specs/context-capability-registry/spec.md` accepted compatibility/debug
  paths for `list_tools`, `list_skills`, `list_mcps`, `set_session_title`,
  `todo`, legacy `write`/`edit`, and `glob`.
- `docs/specs/context-capability-registry/plan.md` said broad tools should be
  hidden in stages and explicitly said not to delete implementations.
- Repository search still finds active tool code and prompts for `list_skills`,
  `list_tools`, `list_mcps`, `set_session_title`, `load_skill`, `graph_query`,
  `query_resource`, `memory(action=...)`, and related legacy guidance.

Fix applied:

- The spec now defines terminal cleanup as an acceptance criterion.
- Compatibility paths are allowed only before journey parity, not as the final
  state.
- The plan now adds a terminal cleanup task that removes old model-visible
  registrations, prompt/template mentions, and stale UI/debug assumptions after
  parity passes.

### Blocker 2: The previous SQLite memory layer was still allowed to survive

The Engram cutover spec already moved toward the adapter, but the plan still
allowed the current SQLite memory provider as a quarantine/fallback provider.
That conflicts with the requirement that the previous SQLite memory layer be
completely gone. SQLite may remain for conversations, execution state, and
outbox reliability, but not as the durable semantic memory/knowledge provider.

Evidence:

- Root `Cargo.toml` still includes `stores/zbot-stores-sqlite`.
- `gateway/src/state/*` still references `KnowledgeDatabase`,
  `MemoryRepository`, and old SQLite memory/knowledge wiring.
- `stores/zbot-stores-sqlite/src/*` still contains `MemoryRepository`,
  `GatewayMemoryFactStore`, `SqliteMemoryStore`, `KnowledgeDatabase`,
  `SqliteKgStore`, `SqliteVecIndex`, and `memory_facts`/`knowledge.db`
  implementation details.
- `docs/specs/engram-memory-engine-cutover/plan.md` marked T9 done for the
  selectable Engram path while allowing the current SQLite fallback to remain.

Fix applied:

- RFC-0014 and the context capability spec now state that terminal completion
  requires no production memory/knowledge path through the old SQLite provider.
- The Engram cutover spec/plan now treats the old SQLite memory fallback as
  reopened cleanup, not done.
- Repository checks now deny production uses of old SQLite memory/knowledge
  types except migration readers, tests, and non-memory SQLite concerns such as
  `conversations.db`, execution state, and bridge outbox.

### Concern 1: "Old tools" needed a precise definition

Without a definition, cleanup could accidentally target useful action tools
like `wait_agent`, `read`, `write_file`, `edit_file`, `delegate_to_agent`, or
`respond`.

Fix applied:

- The RFC/spec now separates obsolete discovery/UI/broad context tools from
  retained action tools.
- `wait_agent` remains explicitly preserved as a BPMN-style parallel join
  primitive.

### Concern 2: Prompt/template cleanup must be part of the same terminal gate

Even if tool registrations are removed, stale system shards and intent prompts
can keep instructing the model to call retired tools.

Fix applied:

- The terminal cleanup task includes gateway templates, prompt shards, intent
  prompts, UI tool phrases, and architecture/docs that describe the active
  model-visible surface.
