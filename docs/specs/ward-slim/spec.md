# Spec: Ward Slim — Payload & Redirect Dedup (P1+P2)

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [PR #249 planner-template handoff](../planner-template-handoff/spec.md) (template-unavailable fail-closed must hold)
- **Brief:** critical review 2026-08-20 (session `sess-e7d37779`, this conversation)
- **Contract:** none
- **Shape:** refactor

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Mode

Full (risk trigger: multi-phase dependent tasks on the orchestrator hot path).
Loop-engine/cohort scripts absent from this repo; spine runs manually. Phases
P3-P5 (prompt dedup, audience split, gate vocabulary) are **separate follow-up
PRs** — this spec covers P1+P2 only.

## Objective

The `ward` tool's entry result carries machine plumbing the model cannot act
on, and the cold-graph redirect rule exists as two drifted copies. Slim the
ward-entry payload to model-actionable fields only, and give the redirect one
canonical source. Zero behavior change: the planning gate, auto-delegation,
template fail-closed, and all guard errors are untouched.

### Evidence (measured, incident session `sess-e7d37779`, `career` ward)

- Ward-entry result: **14,749 bytes ≈ 3.7k tokens, every ward entry**.
  Composition: `ward_knowledge` 9,832 B (5 facts), `ward_template` 3,250 B
  (2,712 B raw WardLayout projection + 3 SHA digests + schema/id plumbing),
  `agents_md` 1,310 B (unbounded read, ward.rs:143-146), `recall_nudge` 110 B
  contradicting the auto-recall beside it.
- The `ward_template` packet is already delivered through executor state
  (`executor.rs:1223` `with_initial_state("ward_template", …)`) into the next
  executor's system instruction (`executor.rs:1187-1221,1398`) — the result
  copy is a same-turn duplicate the model cannot act on: template-dependent
  actions (`lint`/`search`/`dry_run` via `validate_template_context`,
  ward.rs:432) validate against **state**, and after a mid-executor ward
  switch that state is stale until the next bootstrap (immutable-bootstrap
  contract, ward.rs:1134-1137).
- Cold-graph redirect: two live copies with drifted text —
  `rig_adapter/tool.rs:156` ("do not call other tools yet") vs
  `executor.rs:1709` ("do not call MCP tools or other tools yet").
- No UI consumers of `ward_template`/`recall_nudge`/`planner_started`
  (grep `apps/ui`); consumers are tests only.

## Acceptance criteria

- [x] **AC1 — slim result schema.** `ward(use|create)` returns exactly:
  `__ward_changed__`, `ward_id`, `action` (`created|switched`), `ward_status`
  (`{status, archetype}` — archetype is `null` when the packet declares
  none, never fabricated; no projection/digests/session ids), `files`,
  `file_count`, `agents_md` (bounded), `ward_knowledge` (store envelope,
  ≤3 results, `count` = returned result count), and `planner`
  (`"started"` | `"pending-template"`, present only when the planning gate
  was involved). No `ward_template` object, no `recall_nudge`, no
  `planner_started`, no `planner_error`.
- [x] **AC2 — bounded agents_md.** `agents_md` is capped at 32 KiB; oversized
  doctrine truncates with a visible marker naming the cap; doctrine that is
  not valid UTF-8 within the cap is dropped (previous failure semantics) —
  never a misleading marker.
- [x] **AC3 — knowledge trim.** Ward-entry recall requests 3 facts (was 5);
  `degraded` flag and best-effort semantics unchanged. (Declined: a separate
  `total` store count — would cost a second store call for a number the
  model cannot act on; `count` now reports the trimmed result count.)
- [x] **AC4 — one redirect source.** `guards.rs` exports one canonical
  cold-graph redirect envelope (Value + canonical message); both enforcement
  sites (`rig_adapter/tool.rs`, `executor.rs`) call it; no literal copy of the
  message remains outside `guards.rs`. Message text = the executor variant
  ("…do not call MCP tools or other tools yet.").
- [x] **AC5 — behavior preservation.** All existing gate/planner/template
  semantics hold: gate consumption + auto-delegate (`wait_for_result`,
  non-parallel), `try_claim` single-start, template-unavailable fail-closed
  with gate still awaiting + zero delegation, delegated-planner
  `planner_ward_locked`, subagent ward-creation rejection. Test assertions
  updated to the new schema, not deleted.
- [x] **AC6 — payload regression bound.** A test pins the slim contract: the
  entry result for a ward with knowledge and a template contains none of the
  removed fields and no `projection`/`digest` strings anywhere in the payload.

## Boundaries

### Always do

- Keep `__ward_changed__` and `ward_id` exactly as-is (stream processor and
  session-ward persistence depend on them).
- Keep the full packet in executor state flow untouched (bootstrap → state →
  next-turn system instruction).
- Keep every guard error string unchanged (`planner_ward_locked`, subagent
  create rejection, name validation).

### Ask first

- Any change to the planning-gate state machine, delegation shape, or intent
  injection (P3 territory).
- Trimming fact *content* (only the fact *count* is trimmed here).

### Never do

- Don't touch prompt shards, intent_analysis.rs, or tool registration (P3/P4).
- Don't merge the placeholder-specs gate into the planning gate (P5).

## Testing strategy

TDD in-crate: update the five ward test assertions to the new schema (red),
add `ward_entry_result_is_slim` (AC6, no fact store), `ward_entry_recall_is_trimmed_and_present`
(AC3/AC6 "with knowledge" clause via a recording mock `MemoryFactStore`),
and `agents_md_is_bounded` (AC2, temp ward with >32 KiB AGENTS.md). Gates:
`cargo fmt --check`, `cargo clippy -p agent-tools -p agent-runtime`,
`cargo test -p agent-tools -p agent-runtime`, `cargo check --workspace`.
Redirect coverage: the guards unit test pins the canonical envelope text;
the rig_adapter dispatch test asserts that canonical suffix; the builtin
executor site is a pure pass-through of the same tested function with no
local literal (AC4's no-literal grep is the drift guard) — building a full
`AgentExecutor` fixture for the pass-through line was declined as
disproportionate.

## Deferred

- P3 prompt-teaching dedup — gated on live multi-step session per
  `feedback_orchestrator_context_high_stakes`; backlog slug `ward-slim-p3-prompt-dedup`.
- P4 tool audience split — backlog slug `ward-slim-p4-audience-split`.
- P5 gate vocabulary unification — backlog slug `ward-slim-p5-gate-vocabulary`.
- `ward.rs` module split (~600 lines catalog/search plumbing) — declined, cosmetic.
