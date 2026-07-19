# Plan: Agent-Driven Surfaces

- **Spec:** [`spec.md`](spec.md)
- **Status:** Ready for implementation

## Approach

Define a zbot-owned work-surface model and catalog before changing an agent or
renderer. Add an additive gateway event projection and a single web renderer;
then add capability negotiation and safe action routing. The canonical agent
result remains the source of truth, so non-capable and headless clients require
no new rendering path.

## Constraints

- Follow the existing Axum gateway, `GatewayEvent`, WebSocket, and React
  transport layering described in `docs/architecture/architecture.md`.
- Implement `contracts/asyncapi/agent-surfaces.yaml`; it is authored directly
  because no `event-contract` authoring skill is installed.
- Do not add an A2UI renderer dependency until the custom catalog and wire
  format prove the capability; React renders zbot-owned components directly.

## Construction tests

**Integration tests:** gateway accepts valid surface events, rejects invalid
ones without changing a session, and preserves normal clients without the
capability.

**Manual verification:** render a research decision surface in web; view its
fallback in CLI and Telegram; run the same request headlessly; approve one
allowlisted ledger action and verify its audit trail.

## Design (LLD)

### Design decisions

`runtime/agent-surfaces` is a portable Rust crate containing `WorkSurface`,
catalog validation, capability types, and action requests. It depends only on
domain-level serialization/schema libraries. Gateway publication, React and
channel rendering, and future MCP interoperability each depend on that crate,
not on one another. A2UI is the web event encoding, rather than a new source of
execution truth. A custom catalog mirrors the zbot design system and constrains
component, property, and action names. The first implementation is native
gateway integration; a future MCP adapter may consume the same stable model but
is out of scope.
Traces to: AC1-AC7 · `contracts/asyncapi/agent-surfaces.yaml`.

### Data & schema

Surface descriptors are bounded, transient execution projections: surface id,
catalog id/version, component tree, data model, action references, originating
session/execution, and validation status. Trace records retain descriptor hash,
catalog version, validation result, and action audit reference, but never raw
unbounded component payloads. No semantic-memory or new database ownership is
introduced. Traces to: AC1, AC3-AC5.

### Interfaces & contracts

The gateway adds optional `surface.created`, `surface.updated`,
`surface.deleted`, and `surface.validation_failed` events. Clients advertise
`presentation.a2ui` and supported catalog IDs during their existing connection
or session bootstrap. Surface actions enter a gateway endpoint/command adapter
as `{ action_id, surface_id, target, expected_state, context }`; the registry
resolves action IDs only. Traces to: AC1-AC5, AC7 ·
`contracts/asyncapi/agent-surfaces.yaml`.

### Component / module decomposition

`runtime/agent-surfaces` owns pure domain types, catalog validation, and the
`SurfaceActionRequest` port. `gateway-events` owns event types;
`gateway-execution` owns validated surface production; gateway HTTP/WS owns
capability routing and implements the action-policy port; `apps/ui` owns
`A2uiSurfaceRenderer` and native component implementations. Channel adapters
own fallback formatting but do not own policy. A future `gateway-mcp` adapter
would consume the portable crate and the gateway action port. Traces to: AC1-AC7.

### State & control flow

An execution may create/update/delete a surface only after catalog validation.
The gateway sends it only to clients advertising the catalog. A user action is
revalidated against the current domain state and approval policy before the
registered handler runs. Unsupported clients receive no surface event and
continue through canonical result delivery. Traces to: AC1-AC5, AC7.

### Behavior & rules

Initial components are `DecisionMatrix`, `EvidenceTable`,
`AssumptionRegister`, `PlanChecklist`, `ApprovalGate`, and read-only
`OpenLoops`. Initial actions are read/inspect, `plan.request_revision`, and
existing ledger lifecycle transitions; write/external actions are absent.
Catalog validation rejects unknown components, properties, data paths, and
action IDs. Traces to: AC3-AC6.

### Failure, edge cases & resilience

Malformed messages fail closed with a validation event and canonical-result
fallback. A disconnect never blocks execution. An action replay is idempotent
through target state and action audit checks. Unknown catalog versions are not
rendered. Traces to: AC2-AC5, AC7.

### Quality attributes (NFRs)

Batch web rendering within one animation frame for a burst of surface updates;
bound catalog payload size and component count; preserve keyboard access and
native zbot theme tokens. Traces to: AC1, AC3, AC7.

## Tasks

### T1: Define the bounded work-surface and catalog contract

**Depends on:** none

**Touches:** `contracts/asyncapi/agent-surfaces.yaml`, `runtime/agent-surfaces/`,
`gateway/gateway-events/`

**Tests:**
- TDD: valid initial catalog descriptors deserialize; unknown components,
  properties, action IDs, and oversized payloads fail (AC3, AC6).
- Goal-based: AsyncAPI contract validates against the declared event examples
  (AC1, AC7).

**Approach:**
- Create `runtime/agent-surfaces` with channel-neutral `WorkSurface`,
  `SurfaceActionRequest`, capabilities, validator trait, and v1 catalog
  allowlist; no gateway, renderer, or MCP dependencies are permitted.
- Publish the additive event contract and backward spec pointer.

**Done when:** valid/invalid catalog fixtures prove the model is bounded and
the event contract has complete payload examples.

### T2: Add capability-gated gateway event projection

**Depends on:** T1

**Touches:** `gateway/gateway-events/`, `gateway/gateway-execution/`,
`gateway/src/websocket/`, `apps/ui/src/services/transport/`

**Tests:**
- TDD: a client without `presentation.a2ui` never receives a surface event
  (AC2, AC7).
- Integration: a capable client receives validated create/update/delete events
  without affecting canonical text/artifact events (AC1, AC3).

**Approach:**
- Extend connection/session capabilities additively.
- Validate surfaces before publishing and emit validation telemetry on failure.

**Done when:** mixed capable/headless client integration tests preserve normal
session completion and route only validated surfaces.

### T3: Render native zbot web work surfaces

**Depends on:** T1, T2

**Touches:** `apps/ui/src/features/`, `apps/ui/src/services/transport/`,
`apps/ui/src/styles/`

**Tests:**
- Visual/integration: each initial component renders native theme-aware,
  keyboard-operable output (AC1, AC6).
- TDD: unknown components and bindings render the safe fallback, never dynamic
  HTML (AC3, AC5).

**Approach:**
- Implement `A2uiSurfaceRenderer` with a static component registry.
- Attach surfaces to Chat and Research result areas; retain canonical response
  and artifacts adjacent to the surface.

**Done when:** a fixture decision result renders all six initial components and
the UI build/tests pass.

### T4: Route server-owned surface actions and channel fallbacks

**Depends on:** T1, T2, T3

**Touches:** `gateway/src/http/`, `gateway/src/websocket/`,
`gateway/gateway-execution/`, channel adapters, `apps/cli/`

**Tests:**
- TDD: action registry rejects unknown/replayed/disallowed actions and records
  permitted lifecycle actions exactly once (AC4, AC5).
- Integration/manual: web, CLI, Telegram fallback, email fallback, and
  headless execution produce equivalent canonical outcomes (AC2, AC4).

**Approach:**
- Introduce a server-only action registry with domain-state and approval checks.
- Add formatting adapters; no adapter receives direct tool authority.

**Done when:** an approved ledger transition works through web and CLI while a
headless run completes unchanged and unapproved actions are denied.

### T5: Observability, rollout flag, and verification

**Depends on:** T1-T4

**Touches:** gateway settings/telemetry, trace schema, docs/specs

**Tests:**
- Goal-based: feature flag off emits no surface events and preserves existing
  event snapshots (AC7).
- Manual: inspect validation failure, action audit, and client fallback traces
  for one research journey (AC2-AC5).

**Approach:**
- Add opt-in configuration, bounded trace fields, and dashboards/logs for
  catalog validation and action decisions.

**Done when:** the feature can be enabled per capable client, disabled without
rollback work, and all focused gates pass.

## Rollout

- **Delivery:** additive and opt-in; emit no surface events until a client
  advertises the supported catalog and the gateway feature flag is enabled.
- **Infrastructure:** none; use existing WebSocket, trace, and channel
  delivery paths.
- **Deployment sequencing:** contract/model first, gateway producer second,
  renderer third, actions last. Disable the feature flag to roll back.

## Risks

- A model may attempt non-catalog components or action authority; validation and
  registry routing must fail closed.
- Surface updates can inflate WebSocket traffic; component and payload limits
  are required before streaming.
- Channel fallback can diverge from web semantics; canonical result remains the
  comparison point in integration tests.
- Premature MCP packaging would duplicate surface/action policy and add tool
  overhead; defer it until the native contract is proven.

## Changelog

- 2026-07-10: initial plan.
