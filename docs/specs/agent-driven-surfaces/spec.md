# Spec: Agent-Driven Surfaces

- **Status:** Draft
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/asyncapi/agent-surfaces.yaml`](../../../contracts/asyncapi/agent-surfaces.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let zbot present validated, agent-produced work surfaces to capable clients
without changing execution semantics for web, CLI, Telegram, email, API, or
headless runs. A user can inspect structured research and decision work using
native zbot components; an explicitly allowlisted surface action reaches a
gateway-owned domain handler with the same policy and audit behavior regardless
of channel; clients without the capability receive the canonical text,
artifacts, and structured outcome unchanged.

## Boundaries

### Always do

- Keep the canonical execution result, artifacts, session state, and approval
  policy independent of any UI surface.
- Validate every emitted surface against the versioned zbot catalog before it
  reaches a client, and trace validation and action outcomes.
- Route every surface action through a gateway-owned allowlist, state check,
  and audit record.

### Ask first

- Adding a write-capable action, an external side effect, or a new action
  class beyond the initially approved catalog.
- Adding a renderer or interactive adapter for a new external channel.
- Promoting the A2UI protocol version or adding an external runtime dependency.

### Never do

- Never allow an agent to provide executable frontend code, arbitrary HTML,
  arbitrary URLs, an HTTP endpoint, a tool name, or a filesystem command in a
  surface or action.
- Never make a client surface necessary for a headless run to finish.
- Never replace the React application shell, existing gateway event contract,
  or canonical text/artifact result with A2UI.
- Never implement A2UI as an MCP server before the native gateway work-surface
  model, validation, and action registry are stable.
- Never create a new top-level crate or persistence backend for this feature.
- Never let the portable surface model depend on Axum, WebSocket, React, a
  channel adapter, or MCP types.

## Testing Strategy

- Catalog and action validation: TDD, because component/property/action
  allowlists and capability negotiation have strict invariants.
- Gateway event and renderer fallbacks: goal-based integration tests, because
  the behavior crosses the existing execution event, WebSocket, and transport
  boundaries.
- React presentation and channel fallback journeys: visual/manual QA backed by
  focused UI tests, because native rendering and channel-specific degradation
  are user-visible.

## Acceptance Criteria

- [ ] Given a capable web client, when an execution emits a catalog-valid work
  surface, the client incrementally renders only native zbot components and
  retains the canonical text/artifact result.
- [ ] Given a CLI, Telegram, email, API, cron, or other headless client, when
  the same execution emits a work surface, the execution completes without a
  renderer and returns canonical text, artifacts, and structured outcome; an
  adapter may add a channel-appropriate non-authoritative fallback.
- [ ] Given an invalid catalog version, component, property, binding, or action,
  when a surface is submitted, the gateway rejects it, emits a traceable
  validation outcome, and leaves the session result unchanged.
- [ ] Given a user invokes a listed action from web, CLI, or Telegram, when the
  target state and approval policy permit it, the gateway performs the same
  domain operation and writes one audit record; all other actions are denied.
- [ ] Given a surface requests a write-capable or external action, when no
  separately approved gateway action exists, the request cannot invoke a tool,
  shell command, HTTP endpoint, or filesystem operation.
- [ ] Given a research decision result, when it is rendered, the base catalog
  supports `DecisionMatrix`, `EvidenceTable`, `AssumptionRegister`,
  `PlanChecklist`, `ApprovalGate`, and read-only `OpenLoops`; approved additive
  display components are specified by
  [`a2ui-component-catalog`](../a2ui-component-catalog/spec.md).
- [ ] Given additive surface events are disabled or a client does not advertise
  support, when normal sessions and WebSocket consumers run, existing event and
  UI behavior remains compatible.

## Assumptions

- Technical: zbot already maps agent stream events into gateway events and
  carries them over a WebSocket transport to typed React consumers (source:
  `gateway/gateway-execution/src/events.rs`,
  `apps/ui/src/services/transport/types.ts`).
- Technical: A2UI v0.9.1 is the current production protocol and supports
  catalog-constrained declarative surfaces over WebSocket (source:
  https://a2ui.org/, https://a2ui.org/concepts/data-flow/).
- Technical: A mature frontend should define a catalog that reflects its own
  design system (source: https://a2ui.org/concepts/catalogs/).
- Process: an owned additive gateway event interface is specified under
  `contracts/asyncapi/` with bidirectional spec traceability (source:
  `docs/CONVENTIONS.md` §Contracts).
- Product: React remains the stable application shell; A2UI is an optional
  presentation projection for agent-produced work (source: user confirmation
  2026-07-10).
- Product: the first catalog delivery contained only the six named
  decision/approval components and server-allowlisted actions; the approved
  additive display expansion is governed by
  [`a2ui-component-catalog`](../a2ui-component-catalog/spec.md) (source: user
  confirmations 2026-07-10 and 2026-07-28).
- Product: native gateway integration is the initial delivery; an MCP adapter is
  a deferred interoperability concern rather than the execution path (source:
  user confirmation 2026-07-10).
- Technical: the implementation separates portable surface contracts and
  validation from gateway publication, channel rendering, and action adapters
  so a future MCP boundary reuses the same model (source: user confirmation
  2026-07-10).
