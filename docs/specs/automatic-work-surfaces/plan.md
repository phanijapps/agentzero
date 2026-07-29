# Plan: Automatic work surfaces

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog.

## Approach

Add a gateway-owned `present_surface` tool beside the other session tools. The
tool owns the catalog identifier, accepts a stable surface identifier,
display-only components, bound data, and a create/update mode, then validates
the constructed `WorkSurface` through the existing portable catalog before
returning the marker already consumed by both runtime engines. Register it
under a dedicated least-privilege capability for root and ward agents only.
Its model-visible description supplies positive use cases and explicit
text-only skip cases. Reuse the current WebSocket projection and Quick Chat
surface reducer without adding a transport or renderer path.

## Constraints

- Preserve `docs/specs/agent-driven-surfaces/spec.md` and
  `docs/architecture/security.md`: model output is untrusted, surfaces are
  optional, and canonical results remain authoritative.
- Preserve the existing `zbot/work-surface/v1` schema and event names in
  `contracts/asyncapi/agent-surfaces.yaml`.
- Register through the gateway's actor/capability allowlist; do not bypass the
  established tool registry or runtime marker parser.
- Do not expose `ApprovalGate` or any write-capable action through
  `present_surface`.

## Construction tests

- Integration: execute `present_surface`, feed its JSON result through the
  existing runtime marker path, and assert a valid create/update event reaches
  gateway conversion while invalid input produces none (AC2-AC4).
- Client regression: retain the existing `surface_created` followed by
  `surface_updated` stable-ID Quick Chat test (AC3).
- Model smoke check: in a deterministic test harness or recorded manual run,
  hold model, prompt template, tool set, and run configuration fixed; record a
  `present_surface` call for an eligible chart-shaped request and no call for a
  paired simple factual request (AC1, AC7).

## Design (LLD)

### Design decisions

- Use an explicit tool call, not parsing assistant text. This preserves the
  instruction/data boundary and makes invalid descriptors fail closed.
- Give presentation its own capability instead of reusing `respond`, so
  delegated agents do not inherit UI publication authority.
- Let the tool own `catalog_id`; model input cannot select another catalog.

### Data & schema

`present_surface` accepts `surface_id`, `components`, `data`, and optional
`update`. `components` uses the existing component descriptor shape, but the
tool rejects `ApprovalGate`. The tool constructs `WorkSurface`, sets
`catalog_id`, and invokes `ZbotWorkSurfaceCatalog.validate`.

### Interfaces & contracts

No new wire operation is added. Successful tool results use the existing
internal `__work_surface` / `__work_surface_updated` markers, which project to
the AsyncAPI `surface.created` / `surface.updated` events. The existing
contract gains a reverse `x-spec` pointer only.

### Component / module decomposition

- `gateway/gateway-execution/src/tools/present_surface.rs`: tool schema,
  display-only filter, construction, validation, and marker result.
- `gateway/gateway-execution/src/tools/mod.rs`: tool export.
- `gateway/gateway-execution/src/invoke/executor.rs`: dedicated capability,
  actor allowlist, metadata classification, and registration.
- Existing runtime marker parsers, gateway event conversion, and Quick Chat
  renderer remain consumers without new branches.

### State & control flow

The model chooses `present_surface`; the tool validates and returns a marker;
the runtime emits `WorkSurface` or `WorkSurfaceUpdated`; gateway validation
runs again as defense in depth; capable clients receive the event and Quick
Chat keys it by `surface_id`. The model independently calls `respond` with the
canonical answer.

### Behavior & rules

- Use for compact visual summaries of comparative, metric, status, timeline,
  table, or chart-shaped data.
- Skip for simple facts, short prose, code-only answers, clarification
  questions, or when a surface would duplicate the response without improving
  comprehension.
- Prefer one coherent surface over many cards; update a stable surface instead
  of creating duplicates.

### Failure, edge cases & resilience

Malformed arguments, non-display components, catalog validation errors, and
oversized data return a bounded, generic tool error without marker fields or
rejected payload values. A rejected or unsupported surface never blocks the
canonical response.

### Quality attributes (NFRs)

The tool is side-effect-free apart from emitting a presentation event, performs
no I/O, uses the existing 64 KiB/catalog budgets, and exposes no executable
configuration.

### Dependencies & integration

No external dependency is added. `gateway-execution` already depends on
`agent-surfaces`; the tool reuses that dependency and the established
`agent-primitives::Tool` interface.

## Tasks

### T1: Valid tool calls produce only bounded display-surface markers

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/tools/present_surface.rs`, `gateway/gateway-execution/src/tools/mod.rs`

**Tests:**
- TDD: valid create/update calls set the server-owned catalog and corresponding
  marker; invalid descriptors and `ApprovalGate` return errors without markers
  (AC2, AC5, AC9).
- TDD: rejected inputs produce bounded field/reason errors without raw
  component properties, data values, or JSON in output/loggable error text
  (AC11).
- TDD: URL/HTML/code/action-shaped properties are rejected or unrepresentable,
  and tool execution performs no filesystem or network I/O (AC9).
- `present_surface_emits_bounded_create_and_update_markers`,
  `present_surface_rejects_actionable_or_executable_descriptors_without_echo`
  in `gateway/gateway-execution/src/invoke/executor.rs` are PLAN-stage red
  tests. `stub: true` (AC2, AC3, AC5, AC9, AC11).

**Approach:**
- Implement `PresentSurfaceTool` with a closed JSON schema and existing catalog
  validation.
- Add a display-only component guard before returning a marker.

**Done when:** focused tool tests cover create, update, malformed, actionable,
and over-budget inputs.

### T2: Only user-facing root and ward agents can select presentation

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**
- TDD: registry/capability tests assert the tool is visible to root and ward
  actors and absent for delegated executors/reviewers, and its guidance covers
  every prohibited data class (AC1, AC6, AC7, AC10).
- TDD: tool metadata classifies presentation as bounded local output rather
  than filesystem/network execution (AC9).
- `present_surface_is_limited_to_user_facing_actors` in
  `gateway/gateway-execution/src/invoke/executor.rs` is a PLAN-stage red test.
  `stub: true` (AC6, AC9, AC10).

**Approach:**
- Add `SurfacePresent` to the actor capability matrix and context catalog.
- Register the tool through `register_if_allowed`.
- Put positive and negative automatic-selection guidance in the tool
  description, including explicit data-minimization rules that prohibit
  secrets, hidden prompts/context, and unrelated tool or connector data.

**Done when:** actor matrices and model-visible tool schemas enforce
least-privilege presentation.

### T3: Existing runtime and Quick Chat paths deliver automatic surfaces

**Depends on:** T1, T2

**Touches:** `runtime/agent-runtime/src/executor.rs`, `runtime/agent-runtime/src/rig_adapter/engine.rs`, `apps/ui/src/features/chat-v2/useQuickChat.test.ts`, `apps/ui/src/features/research-v2/useResearchSession.test.ts`

**Tests:**
- Goal-based integration: valid tool results become create/update stream
  events; invalid tool results produce no surface event (AC2, AC3, AC5).
- Regression: stable-ID Quick Chat replacement and canonical response behavior
  remain green for capable and headless clients (AC3, AC8).
- Regression: Research's existing A2UI surface state replaces create/update
  events by stable ID without duplication (AC4).
- Integration: a rejected presentation call followed by `respond` preserves
  the canonical message and terminal state (AC12).
- `stub: no stub (goal-based integration mode)`.

**Approach:**
- Reuse existing parsers and event projection; add tests only unless a proven
  compatibility gap requires a minimal correction.

**Done when:** integration tests demonstrate tool-to-client delivery without a
new wire path.

### T4: Automatic selection behavior is documented and verified

**Depends on:** T1-T3

**Touches:** `contracts/asyncapi/agent-surfaces.yaml`, `docs/specs/automatic-work-surfaces/*`

**Tests:**
- Goal-based: contract reverse link, Rust format/clippy/test, workspace check,
  and scoped UI tests/build pass.
- Recorded smoke check: chart-shaped prompts can select `present_surface`;
  under one fixed configuration the eligible prompt must select
  `present_surface` and the paired simple factual prompt must remain text-only;
  retain the transcript/configuration artifact (AC1, AC7).
- `stub: no stub (goal-based/manual mode)`.

**Approach:**
- Add the reverse `x-spec` pointer and record final verification evidence.

**Done when:** all gates pass, reviewers are clean, and the shipped spec has no
unchecked acceptance criteria.

## Rollout

Ship additively with the server and bundled web client. No migration or stored
state changes are required. Rollback removes the tool registration; the
existing catalog and renderer remain compatible.

## Risks

- Model tool selection is probabilistic; strong positive and negative
  description examples reduce over-presentation but cannot guarantee a surface
  for every eligible response.
- Exposing a generic descriptor without the existing validator would expand
  model agency; validation before marker emission and again at gateway
  projection is mandatory.
- Tool-list growth can affect model choice, so the schema and description must
  stay compact.

## Changelog

- 2026-07-28: initial plan; user confirmed automatic, display-only presentation
  for A2UI-capable web sessions.
