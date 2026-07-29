# Spec: Session Observability Reconciliation

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [ADR-0001](../../adr/0001-use-versioned-ward-layout-contracts.md), [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md)
- **Brief:** none
- **Contract:** none; this corrects the established session-list semantics without adding an endpoint
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> change must match this spec, or update it.

## Objective

Make one logical `sess-*` conversation appear exactly once in legacy log
listing and Mission Control even when it has several continuation root
executions. The persisted execution-state session remains the authoritative
source of lifecycle status. New wards must receive usable, template-neutral
`AGENTS.md` instructions that direct agents to the active `ward-conf.yaml`.

## Boundaries

### Always do

- Aggregate each physical execution before collapsing eligible root executions
  by canonical logical conversation ID, after strict continuation-suffix
  normalization; retain the representative root execution ID for detail views.
- Preserve the existing post-aggregation `root_only` protection against
  malformed/missing child execution rows.
- Map the status persisted in `sessions` loss-aware into the log API:
  `queued`/`paused`/`running` → `running`, `completed` → `completed`, and
  `crashed` → `error`; do not infer terminal state from a final log row. A
  log-only record without a `sessions` row is explicitly `unknown`.
- Normalize only the runtime's exact `<base>-cont-<8 lowercase-hex>` grammar;
  malformed or unrelated suffixes remain separate conversations.
- Scaffold only generic, user-editable `AGENTS.md` doctrine that points at the
  active template rather than naming artifact roles.

### Ask first

- Adding an endpoint, storage schema field, migration, role registry, or an
  automatic edit to an existing ward's files.

### Never do

- Reuse a continuation execution ID merely to satisfy a legacy log query.
- Hide stale lifecycle state solely in React or make the UI infer execution
  status from timestamps.
- Hard-code `src`, `data`, `reports`, `outputs`, specs, or concepts into Rust
  scaffolding.
- Overwrite a user-authored `AGENTS.md`.
- Interpolate template-controlled prose into generated agent instructions or
  create `AGENTS.md` outside the root conventional path.

## Testing Strategy

- **TDD:** repository tests create distinct continuation root execution IDs and
  prove `list_sessions(root_only=true)` returns exactly one canonical row.
- **TDD:** log service tests prove every persisted lifecycle status maps
  loss-aware into the log API despite terminal-looking log timestamps; rows
  without lifecycle state remain `unknown`.
- **TDD:** Mission Control summary tests select the latest root execution
  deterministically and keep the persisted session status.
- **TDD:** ward creation tests prove a new default ward receives useful generic
  `AGENTS.md` content while arbitrary template paths remain unaffected.
- **Goal-based check:** Rust formatting, clippy, targeted crate tests, and UI
  typecheck/test suite verify no UI contract regression.

## Acceptance Criteria

- [x] Given several continuation root execution IDs for one `sess-*` ID, when
  `GET /api/logs/sessions?root_only=true` is listed, it returns one canonical
  conversation row rather than one row per execution.
- [x] The returned log-session row uses the highest root execution under the
  documented timestamp/ID ordering for detail navigation and aggregates log
  counts/timestamps across its continuation roots.
- [x] A persisted lifecycle status is mapped loss-aware
  (`queued`/`paused`/`running` to `running`, `completed` to `completed`,
  `crashed` to `error`) and is not
  overwritten by log-derived heuristics; a log-only row is `unknown`.
- [x] Mission Control exposes the latest root execution for a logical session
  and faithfully displays the canonical persisted session status.
- [x] A new ward created from the default editable template includes a
  non-empty `AGENTS.md` that directs agents to `ward-conf.yaml`, generic
  template conformance, and user-editable local instructions without assuming
  named directory roles.
- [x] Existing user-created ward files are not mutated by this change.
- [x] Malformed continuation suffixes and distinct sessions cannot be merged by
  session-list canonicalization.
- [x] Linux ward staging publishes only with a no-clobber rename anchored to
  the verified wards root; portable targets reserve the final directory
  exclusively before writing. Any creation failure is cleaned up and a
  subsequent create can retry.
- [x] Session listing succeeds against the production `sessions` schema when
  both the aggregate and joined lifecycle table expose `started_at`.

## Assumptions

- Technical: continuation roots are distinct `agent_executions` records marked
  with the continuation task sentinel (source: `services/execution-state/src/types.rs`).
- Technical: `/api/logs/sessions` currently normalizes continuation conversation
  IDs but groups rows by execution ID, and its service then recomputes status
  from logs (source: `services/api-logs/src/{repository,service}.rs`).
- Technical: Mission Control already obtains session status from execution
  state, but selects the first root execution rather than the latest one
  (source: `services/execution-state/src/repository.rs`).
- Technical: ward creation scaffolds Markdown files through the generic layout
  creator (source: `gateway/gateway-services/src/ward_layout/create.rs`).
- Product: a ward template is fluid; code must not introduce typed directory
  roles (source: user confirmation 2026-07-20).
- Product: logical session identity, not physical execution identity, is the
  unit users expect in APIs and Mission Control (source: user confirmation 2026-07-20).
- Process: the existing accepted ward-layout decisions constrain the scaffold
  without reopening the RFC (source: ADR-0001 and RFC-0016).
- Technical: continuation conversation IDs use the first UUID segment, which
  is eight lowercase hexadecimal characters (source:
  `gateway/gateway-execution/src/runner/core.rs`).
- Technical: executions have no persisted creation timestamp, so unstarted
  roots use an explicit stable representative fallback rather than claiming
  chronological recency (source: `services/execution-state/src/types.rs`).
