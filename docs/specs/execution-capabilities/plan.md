# Plan: Execution Capabilities

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation reveals new facts.

## Approach

Introduce one execution-scoped capability-assignment value and carry it through
the existing intent, planner, delegation, and executor paths. Intent produces
recommended assignments. For graph work, bootstrap forwards those assignments
as planner guidance plus a safe global catalog; the planner records selected
per-step `Skills` and canonical `MCPs` in the existing step briefings, and the
root forwards those values as the final `skills` and `mcps` delegation mapping. For simple
and Quick Chat root work, the selected root assignment is final. Child startup
normalizes the mapping against live services, starts only its effective MCPs,
and exposes their tools through the existing executor registry before the first
model request. Dynamic skills retain the existing recommendation-plus-
`load_skill` behavior. Static agent MCP mappings remain the fallback only when
the dynamic mapping is absent, avoiding configuration migration.

## Constraints

- Reuse `McpService::get_multiple_for_runtime`, `SkillService`,
  `ExecutorBuilder`, `DelegateAction`, `DelegationRequest`, and the existing
  tool registry; do not create a second MCP lifecycle or tool-discovery path.
- Follow the existing `AgentEngine` event contract so legacy and Rig engines
  retain equivalent delegation behavior. Configured MCPs already select the
  legacy engine path, so MCP discovery coverage must exercise that path.
- Keep capability metadata safe and bounded: IDs, names, descriptions, and
  readiness only. Whitelist and truncate it before it enters prompt state or
  lookup output; runtime auth injection remains inside `McpService`.
- Register capability lookup through `ExecutorBuilder`'s existing tool-registry
  path only when a host-owned planning-context state flag is present; root and
  ordinary step executors must not receive the tool.
- Do not add persistence schema, public WebSocket events, UI work, or a config
  migration in this feature.

## Construction tests

**Integration tests:** a scripted root/simple path; a cold graph path from
intent guidance through planner delegation into child tool discovery; an
unavailable-MCP startup case; and legacy fallback/explicit-empty distinction.

**Manual verification:** run a Blender graph request with the configured MCP;
inspect execution logs for intent guidance, planner assignment, effective child
mapping, and an actual Blender MCP tool call.

**Existing implementation baseline (2026-08-04):** commit `cb07ebdc` already
materialized the T1-T5 implementation and focused construction tests on the
current `develop` history. This fresh work-loop run therefore treats those
tests as the pre-existing TDD baseline and audits them against every criterion
before adding any missing red test. It will not rewrite already-green tests
merely to recreate historical red/green order.

## Execution assumptions

- **Expected touches:** the existing capability-assignment transport, bounded
  catalog, intent/planner routing, delegation startup, and focused tests only
  where the audit finds a concrete spec gap; otherwise this branch changes the
  execution-capabilities spec evidence and workflow records only.
- **Done is demonstrated by:** capability-focused tests in
  `agent-primitives`, `agent-runtime`, `gateway-services`, and
  `gateway-execution`; formatting, clippy/type checks, the gateway-execution
  suite, its required Mode Full E2E, and a real configured Blender MCP smoke
  when that local server is available.
- **Not changing:** agent configuration files, the static MCP fallback,
  persistence schemas, public REST/WebSocket/UI contracts, Rig MCP lifecycle,
  or unrelated Observatory work.

**Declined temptations:**

- Add a new capability service/module — declined because the current runtime
  already has the zbot-owned transport, resolver, lookup tool, and executor
  registration seams required by the spec.
- Mount MCPs on the planner to simplify discovery — declined because it widens
  agency and violates the descriptive-only planner boundary.
- Fold durable work queues or static-config removal into this branch — declined
  because both require separate product and migration decisions.

**Domain grounding:** current code and history confirm that `cb07ebdc` added
`AgentCapabilityAssignment`, `CapabilityCatalogTool`, intent sanitization,
dynamic MCP resolution, delegation propagation, and focused tests. The audit
will use those concrete seams rather than the July plan's paths as authority.

**Pre-execution hardening findings (2026-08-04):** implementation may begin
only after construction tests cover these review-discovered gaps:

- exact canonical-ID startup must prefer the exact record over every
  display-name alias collision while preserving legacy name lookup when no
  exact ID exists;
- catalog provenance must travel host-side for deleted-versus-unknown
  diagnostics, but complete lookup state is registered only for a real
  planner or ward-backed planning transition, never a `step_executor`;
- `step_executor` mode must omit agent delegation so assigned work cannot
  widen its MCP privilege through a grandchild;
- plan-composer must emit exact `## Skills` and `## MCPs` step fields, and
  sanitized resolution logs must distinguish validated requested IDs from the
  effective assignment;
- all claimed criteria need exact test/evidence anchors before shipment.

## Design (LLD)

### Design decisions

Define a shared `AgentCapabilityAssignment` with `agent_id`, `skills`, and
`mcps`, plus host-owned origin metadata where it is logged. The runtime carrier
uses `Option<AgentCapabilityAssignment>`: `None` preserves current static MCP
behavior, while `Some { skills: [], mcps: [] }` deliberately chooses none. A
present assignment owns both dimensions; a partial list does not merge static
MCPs. Assigned skills are injected through the existing recommendation and
`load_skill` path, not eagerly loaded. This avoids a sentinel flag and
preserves existing configs unchanged. Traces to **AC-fallback-semantics**.

Intent adds `recommended_capabilities: Vec<AgentCapabilityAssignment>` instead
of trying to infer ownership from agent config. It validates assignment targets
against configured agents plus `root` and valid existing ward targets. Graph
planning receives those validated recommendations as guidance plus a safe
overview and lookup access to the complete live catalog; planner assignments
win for the children it creates. Root simple/chat applies only its own
assignment. Traces to **AC-root-simple**, **AC-graph-catalog**,
**AC-planner-nonceiling**, and **AC-target-validation**.

### Data & schema

`IntentAnalysis`, its serialized log/snapshot payload, and graph-node planning
guidance gain the shared mapping. `DelegateAction`, `StreamEvent::ActionDelegate`,
and `DelegationRequest` gain an optional assignment or equivalent `mcps` field
that preserves whether the model actually supplied a mapping. Crash-resume
constructors use `None`, retaining legacy MCP behavior. No database migration
is needed because execution logs already hold structured metadata. Traces to
**AC-graph-catalog**, **AC-fallback-semantics**, and **AC-audit-logging**.

### State & control flow

Bootstrap forms a live capability catalog after loading services. Research
intent retrieves relevant candidates and emits validated recommendation maps;
the chat path uses a small root-only selector that cannot choose graph
execution. The root carries graph guidance through both the cold ward-triggered
planner transition and the graduated existing-ward delegation path. Each
planning executor receives a safe catalog overview and a bounded
`lookup_capabilities` tool over the complete live catalog held in context
state. That tool supports canonical-ID lookup, metadata search, and paging but
cannot start an MCP. The planner records selected `skills` and `mcps` in the
per-step briefing; the root invokes `delegate_to_agent` with those exact values
and the event/handler forwards that optional mapping unchanged.

Bootstrap marks only cold `planner-agent` and warm `ward:<name>` planning
executions with the host-owned lookup state and catalog. `ExecutorBuilder`
registers `CapabilityCatalogTool` from that state; root simple/chat and ordinary
child workers neither receive the lookup tool nor its complete catalog. Traces
to **AC-graph-catalog** and **AC-catalog-complete**.

At child spawn, validate the target, canonicalize and deduplicate skills,
resolve MCP IDs with `get_multiple_for_runtime`, and record unavailable values.
For a present assignment, replace `agent.mcps` with its effective dynamic list
and append its skill recommendations before `ExecutorBuilder::build`; for no
assignment, leave existing agent fields untouched. The MCP manager starts
selected servers before the executor is exposed to the model, so normal
registry construction discovers their tools. Traces to **AC-root-simple**,
**AC-delegated-discovery**, **AC-target-validation**, and
**AC-fallback-semantics**.

### Failure, edge cases & resilience

Invalid, disabled, deleted, and OAuth-disconnected MCP references do not block
the child: canonical resolved IDs may be logged with a closed reason code;
unknown raw model requests are represented only by a count plus `unknown_id`.
An MCP that passes readiness but fails startup is fail-closed for that server:
no partial tools, no automatic retry, and only `startup_failed` is logged.
Planning only receives whitelist-sanitized, length-capped descriptions, never
live transports, and cannot accidentally start servers. Lookup enforces a
maximum 25-result page, 256-character query, and 512-character description;
the existing planner tool/turn budget bounds total calls. An explicit empty or
partial map suppresses static MCP fallback. The dynamic path is additive and
reversible: removing the assignment returns execution to current static
configuration. Traces to **AC-target-validation**,
**AC-fallback-semantics**, **AC-runtime-revalidation**,
**AC-startup-failure**, and **AC-audit-logging**.

### Dependencies & integration

`gateway-services` owns safe catalog and runtime-readiness resolution;
`agent-primitives` and `agent-runtime` carry action/event types;
`gateway-execution` owns intent, planner guidance, delegation, executor build,
and log records. Changes flow bottom-to-top: primitives/runtime, services,
gateway execution, then documentation. Traces to all acceptance criteria.

## Tasks

### T1: Dynamic and legacy capability assignments have unambiguous semantics

**Depends on:** none

**Touches:** `runtime/agent-primitives/src/event.rs`,
`runtime/agent-runtime/src/types/events.rs`,
`gateway/gateway-execution/src/delegation/context.rs`, focused event and
delegation tests

**Tests:**

- TDD: an assignment round-trips through `DelegateAction`, `StreamEvent`, and
  `DelegationRequest`, including `Some(empty)`, partial, and `None` maps.
  Covers **AC-fallback-semantics**.
- TDD: legacy/crash-resume construction remains `None` and retains current
  static MCP behavior. Covers **AC-fallback-semantics**.
- TDD: a present empty or partial assignment suppresses static MCP fallback
  and passes only dynamic skills as `load_skill` recommendations. Covers
  **AC-fallback-semantics**.

**Approach:**

- Add the smallest shared serializable assignment value in the existing
  primitive event boundary, with a separate origin used only by host logs.
- Thread its optional form through existing constructors and test fixtures;
  do not add a new dispatcher or persistence type.

**Done when:** the transport can distinguish omitted, empty, and partial
assignments without changing existing static MCP or skill-loading behavior.

### T2: Live capability catalog and intent guidance are safe and canonical

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/mcp.rs`,
`gateway/gateway-execution/src/{middleware/intent_analysis.rs,invoke/executor.rs,runner/invoke_bootstrap.rs}`,
focused service and intent tests

**Tests:**

- TDD: catalog includes enabled, runtime-ready MCPs and skills with only safe
  metadata; disabled and disconnected entries are excluded. Covers
  **AC-runtime-revalidation**.
- TDD: exact canonical-ID startup selects one exact record even when another
  record has a colliding display name; legacy display-name lookup still works
  only when there is no exact-ID record. Covers **AC-fallback-semantics**.
- TDD: a previously cataloged ID missing at final resolution reports `deleted`,
  while a never-cataloged ID reports `unknown_id`, without logging either raw
  unresolved value. Covers **AC-runtime-revalidation** and
  **AC-audit-logging**.
- TDD: typed intent output returns per-agent recommendations; invalid IDs and
  unknown agent targets are removed against the live catalog, while `root` and
  a valid `ward:<name>` remain legal. Covers **AC-root-simple**,
  **AC-graph-catalog**, and **AC-target-validation**.
- TDD: the Quick Chat selector can map a non-trivial request to root while
  retaining chat-mode prohibition of wards, planning, and delegation. Covers
  **AC-root-simple**.

**Approach:**

- Extend resource indexing/retrieval with MCP metadata and add a safe catalog
  helper rather than passing `McpServerConfig` into prompts.
- Add intent recommendation maps and preserve old serialized records with
  serde defaults.
- Carry graph recommendations through both cold and warm ward routes, and use
  the root assignment only for simple/chat executor construction.

**Done when:** intent and Quick Chat produce validated root/agent guidance
without starting an MCP process or leaking configuration.

### T3: Planner can assign any live capability to each delegated step

**Depends on:** T1-T2

**Touches:** `gateway/gateway-execution/src/{middleware/intent_analysis.rs,runner/invoke_bootstrap.rs,invoke/{executor.rs,stream_event_processor.rs,delegation_handler.rs}}`, `runtime/agent-runtime/src/{tools.rs,tools/{delegate.rs,capabilities.rs}}`, `runtime/agent-tools/src/tools/ward.rs`

**Tests:**

- TDD: cold and warm planning contexts contain intent guidance, a safe catalog
  overview, and the planner-only lookup tool but no executable MCP tool.
  Covers **AC-graph-catalog**.
- TDD: `delegate_to_agent` accepts an `mcps` list and preserves a planner
  choice absent from both intent guidance and semantic retrieval. Covers
  **AC-delegated-discovery** and **AC-planner-nonceiling**.
- TDD: a catalog larger than the prompt overview pages or searches a capability
  by canonical ID, then delegates it without MCP startup. Lookup rejects
  overlong queries, caps pages at 25 sanitized entries with 512-character
  descriptions, and planner tool-budget exhaustion prevents an unbounded loop.
  Covers **AC-catalog-complete** and **AC-lookup-bounds**.
- TDD: cold `planner-agent` and warm `ward:<name>` planning executors receive
  `lookup_capabilities`, while root and ordinary step-executor children do not.
  Covers **AC-graph-catalog** and **AC-catalog-complete**.
- TDD: a `ward:<name>` child in `step_executor` mode receives neither
  `lookup_capabilities` nor `delegate_to_agent`; the same ward target in
  ward-backed planning mode retains both. Covers **AC-step-least-privilege**.
- TDD: cold-graph ward transition and warm ward delegation both forward
  capability guidance to their planning executor. Covers
  **AC-graph-catalog**.
- TDD: plan-composer's persisted step briefing contract contains exact
  `## Skills` and `## MCPs` fields, and session plan extraction preserves the
  briefing verbatim for root delegation. Covers **AC-delegated-discovery**,
  **AC-planner-nonceiling**, and **AC-catalog-complete**.

**Approach:**

- Add `mcps` alongside existing `skills` in the delegate tool schema and use
  the optional assignment carrier to preserve absence versus explicit empty.
- Render a bounded safe catalog overview in planner context and register a
  planner-only `lookup_capabilities` tool backed by the complete catalog in
  context state. It returns only whitelisted canonical IDs, names, truncated
  descriptions, and readiness and supports ID lookup, metadata search, and
  paging with hard 25-result/256-query/512-description limits.
- Implement `CapabilityCatalogTool` in the existing runtime tool area and gate
  its `ExecutorBuilder::build_tool_registry` registration on host-owned
  planning state; do not make a model argument or agent ID name the authority.
- Keep planner MCPs unmounted; only its child delegation carries an effective
  mapping.

**Done when:** planner output can dynamically map a step to any available MCP
or skill and that mapping reaches the child unchanged.

### T4: Delegated executors discover exactly their mapped MCP tools

**Depends on:** T1-T3

**Touches:** `gateway/gateway-execution/src/delegation/spawn.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`, focused spawn/executor
tests

**Tests:**

- TDD: a present dynamic MCP assignment replaces static agent MCPs before manager
  construction and only the mapped server's tools are registered before the
  first model request. Covers **AC-delegated-discovery**.
- TDD: a disabled/OAuth-disconnected server is omitted without aborting the
  child; safe diagnostic metadata is emitted. Covers
  **AC-runtime-revalidation**.
- TDD: an MCP that is runtime-ready but fails process or transport startup
  registers no tools, retries nowhere, logs only `startup_failed`, and lets the
  child continue. Covers **AC-startup-failure**.
- TDD: `None` retains configured static MCPs, while `Some(empty)` and partial
  assignments start only the explicit dynamic MCP list. Covers
  **AC-fallback-semantics**.
- TDD: dynamic skills are represented in child instructions as `load_skill`
  recommendations and are not pre-loaded into executor state. Covers
  **AC-fallback-semantics**.
- TDD: a delegated request whose dynamic assignment target is missing or does
  not match the requested child is rejected before executor construction;
  neither dynamic nor static MCPs are mounted and no specialist is
  auto-created. Covers **AC-target-validation**.

**Approach:**

- Resolve assignments once in child spawning through the existing runtime
  resolver, validate the destination agent, deduplicate canonical IDs, and
  mutate only the child clone of the loaded agent.
- Add the same resolution to root simple/chat construction, but never to the
  planner-only graph bootstrap.
- Reuse `build_mcp_manager`; do not add a parallel tool registry.

**Done when:** every executing agent has a bounded, assigned MCP tool surface
at discovery time, with existing static MCPs working only as fallback.

### T5: Capability assignments are auditable without a UI/protocol change

**Depends on:** T2-T4

**Touches:** `gateway/gateway-execution/src/{runner/invoke_bootstrap.rs,invoke/delegation_handler.rs,delegation/spawn.rs}`, execution-log tests

**Tests:**

- TDD: intent, planner request, and effective child assignment logs include
  validated canonical IDs, origin, and closed reason codes but exclude raw
  unknown IDs, startup errors, auth, URLs, commands, and headers. Covers
  **AC-startup-failure** and **AC-audit-logging**.
- Goal-based: existing session-state and execution-log consumers tolerate the
  added metadata without a WebSocket/UI payload change. Covers
  **AC-audit-logging**.

**Approach:**

- Add structured metadata to existing `ExecutionLog` records at each decision
  point, rather than a database table or client event. Use a closed reason enum
  and an unresolved-count field instead of untrusted strings.
- Ensure fallback and dynamic origins are visibly distinguishable during
  operational investigation.

**Done when:** a session trace explains why each child did or did not receive
an MCP, with no secret-bearing configuration recorded.

### T6: Prove behavior end-to-end and document the compatibility phase

**Depends on:** T1-T5

**Touches:** `gateway/gateway-execution/tests/*`, `docs/specs/execution-capabilities/*`, `docs/specs/README.md`

**Tests:**

- Goal-based: run focused primitive, runtime, services, and gateway-execution
  tests; `cargo fmt --all -- --check`; `cargo check -p gateway-execution`;
  and `git diff --check`. Covers **AC-verification**.
- Goal-based E2E: run the gateway full-mode test required by
  `gateway-execution/AGENTS.md` after delegation/spawn changes. Covers
  **AC-verification**.
- Manual QA: reproduce Blender graph work and verify MCP tool discovery/call
  plus complete non-secret execution logs. Covers
  **AC-delegated-discovery** and **AC-audit-logging**.

**Approach:**

- Add the active-spec entry and record exact command results when implementation
  completes.
- Document that static config is a compatibility fallback and its eventual
  removal requires a separate approved change.

**Done when:** automated and manual evidence prove dynamic assignment without
config migration or unrelated worktree changes.

## Rollout

Ship as an additive runtime capability. Existing agent YAML is unchanged. New
intent/planner assignments take precedence only when present; otherwise static
agent MCP mappings preserve current behavior. Emit `legacy_fallback` log origin
so usage can be measured. Removing that fallback, modifying saved config, or
adding a user-facing assignment surface is explicitly deferred to a later
approved spec. Rollback is reverting dynamic assignment production, restoring
the unchanged static behavior.

## Risks

- Treating an empty mapping as absent would accidentally start a static MCP;
  the optional assignment carrier and focused tests are mandatory.
- A capability lookup result can still be large; paging and result limits must
  be enforced and tested so planner discovery is complete without consuming the
  whole context window.
- Dynamic MCP assignment continues to force legacy-engine execution until MCP
  lifecycle support reaches Rig; tests must cover the current engine-selection
  behavior.
- A future static-map removal could break users who do not exercise dynamic
  planning; fallback origin logs provide the evidence for a separate migration
  decision.

## Changelog

- 2026-07-18: Initial plan; revised from static agent ownership to dynamic
  intent/planner assignment with no-migration legacy fallback.
