# Plan: Agent-Driven Surfaces

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

## Approach

Create `runtime/agent-surfaces` as the portable model/validation core. Gateway,
renderers, and a future MCP adapter depend on it but never on one another.

## Tasks

### T1: Portable catalog and surface validation [complete]
**Depends on:** none
**Touches:** `runtime/agent-surfaces/`, workspace `Cargo.toml`, `contracts/asyncapi/agent-surfaces.yaml`
**Tests:** TDD for valid descriptors plus unknown component, action, property, and payload limit rejection.
**Approach:** Add pure `WorkSurface`, capability, action-request, catalog, and validator types with only serde/schema dependencies.
**Done when:** focused tests prove validation and `cargo test -p agent-surfaces` passes.

**Result:** `agent-surfaces` provides portable types, validation, capability
negotiation, and action-request contracts with no adapter dependencies.

### T2: Capability-gated gateway events
**Depends on:** T1
**Tests:** integration checks for capable delivery and non-capable compatibility.
**Approach:** Add optional surface events and validation telemetry to existing gateway event/WS paths.

### T3: Native React renderer
**Depends on:** T1, T2
**Tests:** visual/integration tests for six initial components and safe unknown fallback.
**Approach:** Static registry-based renderer in Chat/Research; no dynamic HTML.

### T4: Gateway-owned actions and fallbacks
**Depends on:** T1-T3
**Tests:** action-policy TDD and web/CLI/headless integration journeys.
**Approach:** Registry maps stable action IDs to domain handlers; adapters format fallbacks only.

### T5: Observability and rollout
**Depends on:** T1-T4
**Tests:** flag-off compatibility and validation/action trace verification.
**Approach:** Add opt-in delivery, bounded traces, and final gates.
