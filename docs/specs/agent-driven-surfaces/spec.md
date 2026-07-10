# Spec: Agent-Driven Surfaces

- **Status:** Implementing
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/asyncapi/agent-surfaces.yaml`](../../../contracts/asyncapi/agent-surfaces.yaml)
- **Shape:** mixed

## Objective

Allow zbot to render validated, agent-produced work surfaces for capable
clients without changing the canonical result or requiring a renderer for
headless, CLI, Telegram, email, API, or cron execution.

## Boundaries

### Always do
- Keep the canonical execution result, artifacts, and approval policy independent of surfaces.
- Validate every surface against a versioned zbot catalog and trace validation/action outcomes.
- Route actions through a gateway-owned allowlist, state check, and audit record.

### Ask first
- Add a write-capable action, external side effect, new interactive channel adapter, protocol upgrade, or dependency.

### Never do
- Never accept executable frontend code, arbitrary HTML/URLs, tool names, shell commands, or HTTP endpoints in a surface/action.
- Never require a surface for headless execution or replace the React shell/canonical result.
- Never make portable surface contracts depend on gateway, React, channel adapter, or MCP types.
- Never add a top-level crate or persistence backend.

## Testing Strategy

- TDD for catalog, capability, and action invariants.
- Integration checks for gateway/event compatibility and client fallback.
- Visual/manual QA for native web rendering and channel journeys.

## Acceptance Criteria

- [ ] A capable web client incrementally renders catalog-valid work surfaces with its canonical result retained.
- [ ] Non-capable/headless clients complete with canonical text, artifacts, and structured outcome; optional adapters are non-authoritative.
- [ ] Invalid catalog versions, components, properties, bindings, or actions fail closed and leave the session result unchanged.
- [ ] A listed action applies the same policy/audit behavior from web, CLI, or Telegram; all other actions are denied.
- [ ] A surface cannot invoke tools, shell, HTTP, filesystem, or an unapproved external/write action.
- [ ] The first catalog supports `DecisionMatrix`, `EvidenceTable`, `AssumptionRegister`, `PlanChecklist`, `ApprovalGate`, and read-only `OpenLoops`.
- [ ] Existing event consumers remain compatible when surfaces are disabled or unsupported.

## Assumptions

- Technical: zbot maps agent events into gateway events and typed WebSocket consumers (source: `gateway/gateway-execution/src/events.rs`, `apps/ui/src/services/transport/types.ts`).
- Technical: A2UI v0.9.1 supports catalog-constrained declarative WebSocket surfaces (source: https://a2ui.org/).
- Product: native gateway integration first; a future MCP adapter reuses the stable portable model (source: user confirmation 2026-07-10).
