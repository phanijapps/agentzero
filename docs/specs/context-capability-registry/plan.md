# Plan: Context Capability Registry

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Ship RFC-0014 in additive slices. First add contract-shaped domain types and a
read-only capability catalog over the current tool registry and actor policy.
Then wire `/api/tools`, introduce `ContextPacket` and `ContextAtom` as typed
outputs from existing recall, convert micro-recall to packet deltas, and only
then begin hiding obsolete tools from default model-visible catalogs. The first
release keeps compatibility wrappers and current execution semantics intact so
gateway/UI contracts, Rig adapter behavior, and current DB journey parity can
be verified before deletion or renaming. The terminal cleanup is stricter:
after parity, old model-visible registrations and the previous SQLite
memory/knowledge production path are removed rather than kept as fallback debt.

## Constraints

- [`RFC-0014`](../../rfc/0014-context-capability-registry-and-context-graph.md):
  capability registry, context graph packets, ingestion boundary, and staged
  tool visibility policy.
- [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md): Engram remains
  behind zbot-owned adapter contracts; gateway/UI/sleep-cycle contracts stay
  owned by zbot.
- [`rig-engine-migration`](../rig-engine-migration/spec.md): tool execution and
  hidden runtime context must stay engine-independent across legacy and Rig.
- [`dynamic-ontology-skos-taxonomy`](../dynamic-ontology-skos-taxonomy/spec.md):
  ontology remains dynamic and zbot-owned; taxonomy is durable SKOS-style.
- Existing route paths and public event shapes are preserved during this spec.
- Existing broad tools are hidden in stages; no implementation is deleted until
  compatibility and journey parity are proven. After parity, deletion or
  non-model quarantine is required.
- The SQLite conversation DB contract remains; the SQLite semantic
  memory/knowledge provider does not remain as a production fallback.

## Construction tests

**Integration tests:**

- JSON schema validation for catalog and packet examples against
  `contracts/jsonschema/context-capability-catalog.schema.json` and
  `contracts/jsonschema/context-packet.schema.json`.
- Gateway `/api/tools` handler tests for root, delegated executor, delegated
  reviewer, and ward-agent catalog differences.
- AAPL journey fixture built from the current conversation DB shape, sanitized
  or synthetic where private content would otherwise leak.
- Synthetic skills + MCP + code + research journey fixture that proves MCP
  discovery can flow through catalog metadata/resources.
- Repository-search cleanup gates proving old model-visible tool names are gone
  from active registration/prompts and previous SQLite memory/knowledge types
  are absent from production runtime paths outside the explicit allowlist.

**Manual verification:**

- Run daemon/UI and verify tool monitor or API inspector shows non-empty
  catalog data.
- Run a normal sequential delegation and verify `wait_agent` is not suggested
  or required.
- Run a synthetic parallel delegation and verify `wait_agent` remains available
  as the join primitive.

## Design (LLD)

### Design decisions

- Add registry metadata beside existing tool execution rather than replacing
  `ToolRegistry` in the first slice. Traces to: AC1-AC3.
- Define packet/atom types before changing prompt rendering. Traces to: AC4-AC6.
- Keep title derivation and skill packet changes as compatibility-preserving
  wrappers before hiding old tools. Traces to: AC7-AC10.
- Treat MCP/research coverage as synthetic until the current DB contains a real
  MCP execution. Traces to: AC15.

### Data & schema

- `ContextCapabilityCatalog` and `ContextPacket` serialize to the JSON Schema
  contracts under `contracts/jsonschema/`.
- Catalog entries are generated snapshots, not durable DB rows in the first
  implementation.
- Context packet traces retain selected/dropped candidates and source mix but
  do not include raw private content, embeddings, credentials, or absolute
  private paths.

### Interfaces & contracts

- Existing `/api/tools` and `/api/tools/:name` routes become catalog-backed.
- Existing tool execution APIs remain unchanged.
- The catalog and packet contracts are JSON Schema files, not new REST route
  namespaces.

### Component / module decomposition

- `agent-runtime` or gateway-shared module: catalog/packet domain types,
  serialization, schema examples, and result envelope types.
- `gateway-execution`: bridge current `ToolCapability` / `RuntimeActorKind`
  actor policy into catalog metadata.
- `gateway-memory`: convert recall scored items into `ContextAtom`s and expose
  packet assembly hooks.
- `gateway`: wire `/api/tools` and state access to the catalog provider.
- `agent-tools`: update `load_skill` packet behavior and tool metadata.
- `gateway-execution`: add title service and micro-recall packet delta path.

### State & control flow

1. Gateway builds the same actor-filtered tool registry it uses today.
2. Catalog builder reads actor kind, registered tools, MCP metadata, connector
   provider metadata, memory/graph/recall feature availability, and configured
   resources.
3. Catalog builder emits an actor-filtered snapshot for `/api/tools`, prompt
   tool selection, and observability.
4. Context planner requests recall/resource candidates for the current
   request.
5. Recall emits `ContextAtom`s.
6. Context packet builder fuses atoms, graph nodes/edges, handles, drops, and
   trace metadata under a budget.
7. Renderer converts selected packet contents to model text with handles.
8. Tool results and micro-recall produce packet deltas and distillation
   candidates after each step.

### Behavior & rules

- `wait_agent` catalog metadata sets `kind=tool`, `side_effects=read_external`
  or equivalent join semantics, and default visibility only when active
  parallel children exist.
- `set_session_title` remains callable as compatibility/debug while title
  service becomes the default runtime path until terminal cleanup removes
  model-visible registration.
- `load_skill` default output becomes summary + handles; full body is explicit.
- `shell` remains available for execution but catalog metadata marks file
  discovery, grep, JSON extraction, and validation as better served by resource
  lanes where present.
- `memory` remains callable only until split actions/resources reach parity.

### Failure, edge cases & resilience

- If catalog building fails, gateway execution must still be able to run with
  the existing tool registry and log a degraded catalog state.
- If schema serialization fails in tests, the task is not done.
- If context packet assembly cannot retrieve a source, it drops that candidate
  with a trace reason instead of fabricating context.
- If MCP is configured but unavailable, MCP catalog entries are marked
  degraded/unsupported; no subprocess lifecycle behavior changes in this spec.
- If Engram-backed features are unsupported, catalog entries report unsupported
  and context packets omit those lanes before terminal cleanup. After Engram
  cutover, unsupported durable memory/knowledge features fail closed rather than
  falling back to the old SQLite provider.

### Quality attributes (NFRs)

- Token budget: packet renderer must report estimated tokens and selected vs.
  dropped counts.
- Observability: selected/dropped context and source mix are traceable.
- Security: actor filtering and resource visibility are code-enforced.
- Maintainability: broad tool hiding is metadata-driven and reversible before
  parity; terminal cleanup removes the old registrations once fixtures pass.

### Dependencies & integration

- No new external runtime dependency is required for the first slice.
- JSON schema files are committed as contracts; tests may use existing Rust
  serde/schema tooling or a lightweight dev-only validator if already present.
- Engram adapter feature reports are consumed only through zbot provider
  capability surfaces.

## Tasks

### T1: Catalog and packet contract examples validate

**Depends on:** none

**Status:** Done on 2026-07-07.

**Touches:** `contracts/jsonschema/*.json`, `docs/specs/context-capability-registry/*`, `runtime/agent-runtime/src/*` or `gateway/gateway-execution/src/*`

**Tests:**

- TDD: sample root catalog serializes with valid actor kind, capability kind,
  risk, side effect, health, and no additional properties. Verifies AC1.
- TDD: sample context packet serializes with budget, atoms, graph nodes/edges,
  handles, dropped candidates, and trace metadata. Verifies AC4.
- Goal-based: schema examples validate against both JSON Schema contracts.
  Verifies AC1, AC4.

**Approach:**

- Add Rust domain types for catalog entries, context packets, atoms, handles,
  dropped candidates, budget, and trace metadata.
- Add small fixture/example builders for tests.
- Keep types independent of concrete tool execution and Engram internals.

**Done when:** focused schema/type tests pass and both committed schema files
have at least one passing serialized example.

**Verification:** `cargo test -p agent-runtime context --locked`

### T2: Catalog builder wraps current tool registry and actor policy

**Depends on:** T1

**Status:** Done on 2026-07-07.

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`, `runtime/agent-runtime/src/tools/registry.rs`, `gateway/gateway-execution/src/*`

**Tests:**

- TDD: root catalog contains root-allowed tools and excludes reviewer-only or
  disallowed capabilities according to existing `actor_allows` policy. Verifies
  AC2.
- TDD: delegated reviewer catalog excludes shell/file write and includes read,
  respond, and review-safe capabilities according to current policy. Verifies
  AC2.
- TDD: `wait_agent` catalog metadata marks it as a parallel join action.
  Verifies AC9.

**Approach:**

- Add a catalog builder that consumes the live registered tools and actor kind.
- Map existing `ToolCapability` values to catalog metadata.
- Keep `ToolRegistry` execution unchanged.

**Done when:** catalog snapshots match existing actor policy and no tool
execution test changes are required.

**Verification:** `cargo test -p gateway-execution catalog --locked`

### T3: `/api/tools` returns catalog data

**Depends on:** T2

**Status:** Done on 2026-07-07.

**Touches:** `gateway/src/http/tools.rs`, `gateway/src/state/mod.rs`, `gateway/tests/*`

**Tests:**

- Integration: `GET /api/tools` returns non-empty catalog entries for a root
  context when tools are configured. Verifies AC3.
- Integration: `GET /api/tools/:name` returns one matching catalog entry or
  404 for a real missing id. Verifies AC3.
- Integration: route response does not include secrets, raw tool state, private
  file paths, or hidden runtime context. Verifies boundaries.

**Approach:**

- Add catalog provider access to gateway state.
- Replace placeholder empty/404 handler behavior with catalog lookups.
- Preserve route paths and response compatibility where possible.

**Done when:** targeted gateway tool endpoint tests pass.

**Verification:** `cargo test -p gateway tools_ --locked`

### T4: Recall emits context atoms

**Depends on:** T1

**Status:** Done on 2026-07-07.

**Touches:** `gateway/gateway-memory/src/recall/*`, `gateway/gateway-memory/src/lib.rs`

**Tests:**

- TDD: fact, belief, wiki, procedure, graph node, hierarchy, goal, and episode
  recall items map to `ContextAtom`s with score, confidence, provenance,
  visibility, route hint, token estimate, and render policy. Verifies AC5.
- TDD: contradicted/superseded facts preserve penalty/drop reasons in traceable
  metadata. Verifies AC5.
- TDD: no raw embedding bytes or private DB internals are serialized. Verifies
  boundaries.

**Approach:**

- Add adapter functions from existing `ScoredItem` and recall provenance into
  `ContextAtom`.
- Keep existing recall ranking behavior untouched.
- Add trace/drop metadata without changing current prompt rendering yet.

**Done when:** recall atom tests pass and existing recall tests remain green.

**Verification:** `cargo test -p gateway-memory context_atoms --locked && cargo test -p gateway-memory recall_context_atoms --locked`

### T5: Context packet builder and renderer land behind current prompt path

**Depends on:** T4

**Status:** Done on 2026-07-07.

**Touches:** `gateway/gateway-execution/src/invoke/*`, `gateway/gateway-memory/src/recall/*`, `runtime/agent-runtime/src/context_management.rs`

**Tests:**

- TDD: packet builder respects max token budget, lane caps, visibility, and
  selected/dropped trace counts. Verifies AC4.
- TDD: renderer produces deterministic sections for task state, memory,
  graph/context, resources, tool result handles, and active constraints.
  Verifies AC4.
- Integration: existing prompt path can run with packet generation enabled but
  not yet mandatory. Verifies backward compatibility.

**Approach:**

- Add packet builder over recall atoms, resource handles, graph nodes/edges,
  and tool result handles.
- Add deterministic renderer.
- Gate insertion behind config or internal feature flag until journey fixtures
  pass.

**Done when:** packet builder tests pass and existing execution tests are not
forced onto the new renderer yet.

**Verification:** `cargo test -p gateway-execution recall --locked`

### T6: Micro-recall emits packet deltas

**Depends on:** T5

**Status:** Done on 2026-07-07.

**Touches:** `gateway/gateway-execution/src/invoke/micro_recall.rs`, `gateway/gateway-execution/src/invoke/working_memory.rs`

**Tests:**

- TDD: tool error, ward entry, pre-delegation, delegation callback, and entity
  mention triggers produce `ContextPacketDelta` entries. Verifies AC6.
- TDD: existing working-memory markdown path remains available while packet
  deltas are introduced. Verifies compatibility.
- TDD: packet delta entries preserve provenance and visibility. Verifies AC6.

**Approach:**

- Add delta structs and conversion from current micro-recall handlers.
- Keep existing working-memory injection until renderer cutover is approved.
- Add trace reasons for each trigger.

**Done when:** micro-recall packet tests pass and existing micro-recall tests
remain green.

**Verification:** `cargo test -p gateway-execution micro_recall --locked && cargo test -p gateway-execution working_memory --locked`

### T7: Skill packets and title service replace noisy tools by default

**Depends on:** T2, T5

**Status:** Done on 2026-07-07.

**Touches:** `runtime/agent-tools/src/tools/execution/skills.rs`, `gateway/gateway-execution/src/*`

**Tests:**

- TDD: `load_skill` returns bounded summary, section handles, resource URI, and
  token estimate by default. Verifies AC7.
- TDD: explicit full-body/debug skill read remains possible through a resource
  handle or compatibility flag. Verifies AC7.
- TDD: title service derives title from explicit title, intent hint, first user
  message, and first meaningful plan/result in priority order. Verifies AC8.
- Integration: default catalog hides `set_session_title` while compatibility
  path remains callable. Verifies AC8, AC10.

**Approach:**

- Change skill load output contract behind compatibility metadata.
- Add runtime title service and call it from session/intent orchestration.
- Mark `set_session_title` as hidden from default catalog.

**Done when:** skill/title tests pass and AAPL fixture no longer needs
load-skill offload inspection for the planned journey.

**Verification:** `cargo test -p agent-tools skill_packet --locked && cargo test -p gateway-execution session_title --locked`

### T8: Tool visibility policy lands with compatibility wrappers

**Depends on:** T2, T7

**Status:** Done on 2026-07-07.

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`, `runtime/agent-tools/src/tools/mod.rs`, `runtime/agent-tools/src/tools/*`

**Tests:**

- TDD: default catalog hides `list_tools`, `list_skills`, `list_mcps`,
  `set_session_title`, `todo`, and legacy `write`/`edit` where compatibility
  settings allow. Verifies AC10.
- TDD: broad tools expose split-target metadata for `memory`, `query_resource`,
  `graph_query`, `shell`, `ward`, and `load_skill`. Verifies AC11.
- TDD: `wait_agent` is visible only when parallel join conditions are present
  or debug/admin mode requests it. Verifies AC9.

**Approach:**

- Add default visibility metadata separate from hard actor availability.
- Keep compatibility/debug access only until T11.
- Do not delete tool implementations in T8; T11 owns deletion/quarantine after
  parity.

**Done when:** catalog visibility tests pass and existing tool execution tests
remain green.

**Verification:** `cargo test -p gateway-execution catalog --locked && cargo test -p gateway-execution broad_tools --locked`

### T9: Ingestion boundary records durable memory/knowledge intake

**Depends on:** T4

**Status:** Done on 2026-07-09. The `ingest` tool, `memory save_fact`,
real-time tool-result extraction, and explicit resource-read distillation all
record evidence with retention, ontology, taxonomy, and provenance through the
shared intake boundary. Ordinary resource reads remain read-only by default.

**Touches:** `runtime/agent-tools/src/tools/ingest.rs`, `gateway/gateway-memory/src/*`, `stores/*`

**Tests:**

- TDD: explicit `memory_write`, text/structured `ingest`, tool-result
  distillation, and resource-read distillation all produce evidence records
  with provenance and retention policy. Verifies AC12.
- TDD: read-only resource reads do not persist by default. Verifies AC12.
- TDD: ontology and taxonomy labels are passed as selected zbot policy, not
  inferred as a fixed global schema. Verifies AC12.

**Approach:**

- Introduce an internal evidence intake abstraction behind existing write paths.
- Route existing ingest tool and distillation candidates through it.
- Keep model-facing `memory_write` and `ingest` semantics distinct.

**Done when:** evidence intake tests pass and existing memory/ingest tests
remain green.

**Verification:** `cargo test -p agent-tools ingest_ --locked`

### T10: Journey fixtures prove target behavior

**Depends on:** T3-T9

**Status:** Done on 2026-07-07.

**Touches:** `e2e/*`, `gateway/tests/*`, `docs/rfc/0014-notes/*`, `docs/specs/context-capability-registry/*`

**Tests:**

- Goal-based e2e: AAPL journey fixture records baseline and target counts for
  shell, read, load-skill, and packet tokens. Verifies AC14.
- Goal-based e2e: synthetic skills + MCP + code + research journey discovers
  MCP capabilities through catalog metadata/resources. Verifies AC15.
- Goal-based e2e: parallel delegation journey keeps `wait_agent` as join and
  hides it from sequential flow. Verifies AC9, AC15.

**Approach:**

- Build sanitized fixture data from the current conversation DB shape.
- Add synthetic MCP resource/tool metadata.
- Assert target journey surfaces without committing private transcripts.

**Done when:** journey fixtures pass and the report/RFC notes are updated with
the measured deltas.

**Verification:** `PYTHONPATH=. pytest -q e2e/fixtures/tests`

### T11: Terminal cleanup removes obsolete tools and SQLite memory fallback

**Depends on:** T10 and accepted Engram parity/manual smoke.

**Status:** Done on 2026-07-09 for obsolete model-visible
discovery/title/todo tools, production SQLite semantic fallback removal,
default hiding of broad memory/graph context-pull wrappers, and connector
read/action splitting.

**Touches:** `runtime/agent-tools/src/tools/*`,
`gateway/gateway-execution/src/invoke/*`, `gateway/templates/**`,
`gateway/src/state/*`, `gateway/src/http/*`, `gateway/gateway-memory/src/**`,
`stores/zbot-stores-sqlite/*`, `stores/zbot-engram-adapter/*`,
`apps/ui/src/features/shared/statusPill/*`, active docs that describe the
model-visible tool or memory provider surface.

**Tests:**

- Goal-based: repository search proves obsolete discovery/UI tools are not
  registered in model-visible tool catalogs or prompt/template shards:
  `list_tools`, `list_skills`, `list_mcps`, `set_session_title`, `todo`,
  legacy `write`/`edit`, and retired `glob` after file resources land. Verifies
  AC10.
- Goal-based: repository search proves broad context-pull tools are either
  removed from default model registration or converted to non-model resources:
  `memory`, `graph_query`, `query_resource`, and legacy full-body
  `load_skill`. Verifies AC11. Connector reads use `connector_resource`;
  connector actions use `connector_invoke`.
- Goal-based: repository search proves production memory/knowledge code no
  longer constructs or depends on `KnowledgeDatabase`, `MemoryRepository`,
  `GatewayMemoryFactStore`, `SqliteMemoryStore`, `SqliteKgStore`,
  `SqliteVecIndex`, `knowledge.db`, `memory_facts`, or sqlite-vec memory
  indexes except migration readers, tests, and preserved non-memory SQLite
  concerns. Verifies AC13.
- Integration: gateway/UI tests still pass for Memory, Graph, Observatory,
  Research/Vault, `/api/tools`, and WebSocket event mapping. Verifies AC16.
- Manual QA: fresh Engram DB smoke confirms chat, recall, knowledge activity,
  sleep-cycle cleanup, reload, and Memory/Graph/Observatory tabs without
  creating the old `knowledge.db`.

**Approach:**

- Delete or quarantine obsolete first-party tool registrations after T10 parity.
- Remove prompt/template guidance that instructs the model to call retired
  tools.
- Move any remaining compatibility behavior into non-model runtime services,
  resources, migration readers, or admin-only debug paths.
- Remove old SQLite memory/knowledge provider selection from production startup.
  Preserve SQLite conversation/execution/outbox paths.
- Update active docs in the same change so the documented tool and memory
  surface matches the code.

**Done when:** deny-list searches are clean except for documented allowlist
hits, a fresh Engram DB smoke does not create or use the old knowledge DB, and
all targeted tests pass.

**Verification:** `python3 tools/context_capability_cleanup.py`; `cargo test -p gateway state --locked`; `cargo test -p gateway --test memory_unified_search --locked`; `cargo test -p gateway --test ward_content_endpoint --locked`

## Rollout

- **Delivery:** additive behind catalog/packet configuration first. Default
  prompt hiding happens only after journey fixtures pass. Terminal cleanup
  removes obsolete model-visible registrations after the fixtures and Engram
  parity/manual smoke pass.
- **Infrastructure:** no new infrastructure. Contract schemas are committed
  under `contracts/jsonschema/`.
- **External-system integration:** MCP metadata is read when configured, but MCP
  lifecycle behavior does not change in this spec.
- **Deployment sequencing:** schema/domain types first, catalog API second,
  packet generation third, tool visibility fourth, fixture-gated cleanup last.
- **Rollback:** before T11, disable packet rendering and default visibility
  hiding; current tool execution and compatibility wrappers remain available.
  After T11, rollback is a normal code revert, not a live old-memory fallback.

## Risks

- Catalog metadata may drift from actual actor policy. Mitigation: tests derive
  catalog snapshots from existing actor filters and compare expected entries.
- Packet assembly may hide useful details. Mitigation: selected/dropped traces
  and raw handles stay available.
- Tool hiding may break old prompts. Mitigation: hide from default catalog first;
  compatibility/debug paths stay live only until T11 terminal cleanup.
- Tool hiding may stop at compatibility debt. Mitigation: T11 makes lingering
  model-visible old tools a failing deny-list condition.
- Old SQLite memory/knowledge fallback may survive through provider selection.
  Mitigation: T11 removes it from production startup and allows SQLite only for
  conversations, execution state, outbox/replay, migration readers, and tests.
- MCP synthetic fixture may miss real lifecycle behavior. Mitigation: this spec
  does not change MCP lifecycle; a real MCP fixture should be added when a
  configured MCP run exists.
- Ingestion boundary could become too broad. Mitigation: model-facing actions
  stay distinct: `memory_write`, `ingest`, resources, and context graph reads.

## Changelog

- 2026-07-06: initial plan.
- 2026-07-06: adversarial review added terminal cleanup for obsolete
  model-visible tools and previous SQLite memory/knowledge provider removal.
- 2026-07-07: completed T1. Added engine-independent
  `agent_runtime::context` domain types plus serialized catalog and packet
  examples checked against the committed JSON Schema property/required-field
  contracts.
- 2026-07-07: completed T2/T3. Added catalog snapshots over the live
  `ExecutorBuilder` registry and actor policy, exposed the builder through
  `gateway-execution`, and wired `/api/tools` plus `/api/tools/:name` to return
  actor-filtered context capability data.
- 2026-07-07: completed T4. Added `gateway_memory::recall::context_atoms`,
  projected unified recall `ScoredItem`s into `ContextAtom`s, added a
  `MemoryRecall::recall_context_atoms` hook, and preserved fact-native
  validity plus contradiction/supersession provenance for fact projections.
- 2026-07-07: completed T5-T8. Added context packet build/render helpers,
  packet deltas in working memory, bounded skill packets, runtime session title
  derivation, default visibility metadata, and split-target metadata for broad
  tools while keeping actor enforcement in code.
- 2026-07-07: completed T10 and the terminal-cleanup portion of T11. Added
  synthetic skills/MCP/code/research journey fixtures, deleted retired
  discovery/title/todo tool implementations from `agent-tools`, removed retired
  prompt/template references, removed production `AppState.knowledge_db` and
  SQLite semantic reindex/backfill hooks, and added
  `tools/context_capability_cleanup.py` as the reproducible deny-list gate.
- 2026-07-07: partially completed T9. The `ingest` tool now records evidence
  with retention, ontology, taxonomy, and provenance.
- 2026-07-09: completed AC2 resource catalog population. `/api/tools` now
  enriches the first-party tool catalog with MCP summaries, connector
  resource/action metadata, and memory/graph/recall provider resources without
  changing tool execution registration.
- 2026-07-09: completed T9/T11 broad context-pull cleanup. Added the narrow
  `memory_write` action, hid broad `memory` and `graph_query` from
  model-visible schemas while keeping them internally executable, bridged the
  Rig adapter through model-visible tools only, and updated active templates
  plus local config prompts to prefer injected context packets over recall
  tools.
- 2026-07-09: completed connector split follow-up. Added
  `connector_resource` and `connector_invoke`, hid broad `query_resource` from
  model-visible schemas while preserving internal compatibility dispatch, and
  removed the resolved backlog deferral.
