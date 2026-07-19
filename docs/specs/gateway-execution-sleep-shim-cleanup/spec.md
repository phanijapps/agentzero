# Spec: Gateway Execution Sleep Shim Cleanup

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** agent tool surface cleanup; Engram memory engine cutover
- **Brief:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it.

## Objective

Remove compatibility sleep-module shims from `gateway-execution` now that the
sleep operations live in `gateway-memory`. Success means `gateway-execution`
only owns execution-specific handoff writing under `sleep`, while gateway and
tests import sleep maintenance operations directly from `gateway_memory::sleep`.

## Boundaries

### Always do

- Keep `gateway-execution/src/sleep/handoff_writer.rs` because it formats and
  injects zbot execution handoff context.
- Move direct callers of moved sleep operations to `gateway_memory::sleep`.
- Prove the public gateway build still compiles after removing re-export shims.

### Ask first

- Removing `HandoffWriter` or changing handoff prompt behavior.
- Moving sleep worker construction out of gateway state wiring.
- Removing gateway-memory sleep implementations.

### Never do

- Keep compatibility re-export files whose only purpose is old import paths.
- Add new facade modules to replace the removed shims.
- Change sleep-cycle behavior or scheduling in this cleanup.

## Testing Strategy

- **Goal-based checks:** compile `gateway-execution` and `gateway` after import
  rewiring.
- **Targeted tests:** run the sleep worker trigger test that previously imported
  sleep operations through the old shim path.
- **Search checks:** `rg` proves no active code imports moved sleep operations
  from `gateway_execution::sleep`.

## Acceptance Criteria

- [x] `gateway/gateway-execution/src/sleep/` contains `handoff_writer.rs` and
  `mod.rs`, but no moved-operation compatibility shim files.
- [x] Gateway state and tests import moved sleep operations from
  `gateway_memory::sleep`.
- [x] Active code no longer imports `Compactor`, `DecayEngine`, `Pruner`,
  `SleepTimeWorker`, or other moved sleep operations through
  `gateway_execution::sleep`.
- [x] `cargo check -p gateway-execution -p gateway` passes.
- [x] `cargo test -p gateway-execution --test sleep_worker_trigger` passes.

## Assumptions

- Technical: `gateway-memory` owns the moved sleep operation implementations.
  (source: `gateway/gateway-memory/src/sleep/mod.rs`)
- Technical: `gateway-execution::sleep` currently keeps compatibility shims for
  moved sleep operations. (source: `gateway/gateway-execution/src/sleep/*.rs`)
- Technical: `HandoffWriter` remains execution-specific and is still used by
  runner and runtime wiring. (source: `rg HandoffWriter gateway`)
- Product: user prefers deletion over compatibility shims when code is not
  needed. (source: user confirmation 2026-07-09)
