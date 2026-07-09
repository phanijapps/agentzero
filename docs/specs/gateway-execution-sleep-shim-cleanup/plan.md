# Plan: Gateway Execution Sleep Shim Cleanup

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy.

## Approach

Remove the gateway-execution sleep re-export files for operations that moved to
`gateway-memory`, keep the local handoff writer, and update imports at the few
call sites that still use the old facade. Verification is compile plus the
sleep worker trigger integration test.

## Constraints

- Do not change sleep behavior, only import ownership.
- Do not remove execution-specific handoff writing.
- Do not add another compatibility facade.

## Construction tests

**Integration tests:** `cargo test -p gateway-execution --test sleep_worker_trigger`.
**Goal-based checks:** `cargo check -p gateway-execution -p gateway`; targeted
`rg` for old imports.

## Design (LLD)

### Design decisions

- `gateway-memory::sleep` is the canonical owner for memory/knowledge sleep
  maintenance operations.
- `gateway-execution::sleep` remains only for handoff writer code that depends on
  execution-layer conversation and prompt conventions.

## Tasks

### T1: Moved sleep-operation shims are removed

**Depends on:** none

**Status:** Done

**Tests:**
- `rg -n "pub use gateway_memory::sleep" gateway/gateway-execution/src/sleep`
  returns no hits outside historical docs.
- `find gateway/gateway-execution/src/sleep -maxdepth 1 -type f` shows only
  `handoff_writer.rs` and `mod.rs`.

**Approach:**
- Delete shim files for compactor, conflict resolver, corrections abstractor,
  decay, orphan archiver, pattern extractor, pruner, synthesizer, verifier, and
  worker.
- Trim `sleep/mod.rs` to declare/export only `handoff_writer`.

**Done when:** the shim files are gone and the module tree is minimal.

### T2: Callers import moved sleep operations from gateway-memory

**Depends on:** T1

**Status:** Done

**Tests:**
- `cargo check -p gateway-execution -p gateway`
- `cargo test -p gateway-execution --test sleep_worker_trigger`

**Approach:**
- Update gateway state type references and tests from `gateway_execution::sleep`
  to `gateway_memory::sleep`.
- Keep `HandoffWriter` imports on `gateway_execution::sleep`.

**Done when:** compile and targeted test pass.

## Rollout

Cleanup lands on the dead-code branch. No runtime data migration or deployment
sequencing is required.

## Risks

- External Rust consumers of the old `gateway_execution::sleep::*` facade would
  break. This repo has no such live consumer, and the cleanup branch is
  explicitly deleting compatibility shims.

## Changelog

- 2026-07-09: initial plan.
- 2026-07-09: shipped; moved-operation shims removed and callers rewired to
  `gateway_memory::sleep`.
