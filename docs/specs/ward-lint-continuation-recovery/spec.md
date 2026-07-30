# Spec: Ward Lint Continuation Recovery

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [ADR-0001](../../adr/0001-use-versioned-ward-layout-contracts.md), [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md)
- **Brief:** none
- **Contract:** none
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> change must match this spec, or update it.

## Objective

Keep ward-backed orchestration recoverable when an agent creates content that
does not conform to the active template. Ordinary lint findings must reach the
model as an actionable repair nudge instead of preventing the next continuation
from starting. If continuation setup fails for a genuinely fatal reason, the
persisted session and root execution must become terminal with the error rather
than remaining falsely `running`. Every planned step must also recommend an
available agent as well as any required capability.

## Boundaries

### Always do

- Treat the active `ward-conf.yaml` snapshot as the sole layout authority.
- Preserve a bounded, allowlisted projection of the lint report and its matching
  snapshot digest in model context.
- Render recoverable findings only inside a product-owned, delimited JSON data
  block; never treat finding values as instructions or filesystem paths.
- Atomically bind a pending continuation to the exact queued root execution,
  session, and agent before work starts.
- Persist a truthful terminal state and error when continuation construction
  fails before an execution task is spawned.
- Recommend an agent from the live agent catalog for every planned step.

### Ask first

- Adding a new public API, persistent schema field, or automatic ward-content
  mutation outside the existing tool and continuation paths.

### Never do

- Silently ignore a lint finding or let a step claim conformance while the ward
  remains invalid.
- Treat configuration, integrity, confinement, unreadable-input, or budget
  failures as model-repairable conformance findings.
- Hard-code spec, plan, task, source, data, or report roles into Rust.
- Add a new module boundary, dependency, retry loop, or compatibility path.

## Testing Strategy

- **TDD:** executor construction with an allowlisted ordinary conformance
  finding injects a bounded, delimited repair data block and remains runnable;
  configuration, integrity, confinement, unreadable-input, and budget failures
  remain fatal; valid wards retain their existing prompt byte-for-byte.
- **TDD:** a fatal continuation-construction error cannot leave both the session
  and root execution in `running`, and the continuation flag is not consumed
  before setup succeeds.
- **Goal-based check:** bundled planner and builder instructions require a live
  recommended agent per step and forbid completion while lint is invalid.
- **Integration test:** the reproduced `undeclared_markdown` transition can
  reach a repair continuation rather than deadlocking before the next step.

## Acceptance Criteria

- [x] An ordinary `ward_lint.valid=false` report is injected as a model-visible
  repair nudge and does not make executor construction return an error.
- [x] The repair packet contains a fixed product-owned instruction followed by
  a delimited JSON data block containing only the bounded structured report and
  matching snapshot digest; adversarial path text cannot become instructions.
- [x] The repair nudge directs the active agent to restore conformance and lint
  again before completing or delegating normal work.
- [x] Continuation state is cleared only after setup succeeds; fatal setup
  failure preserves the continuation request, records a sanitized stable error
  on the exact root execution, makes the session terminal, and emits the
  existing error event only after persistence.
- [x] A failed continuation cannot remain `running` with zero pending
  delegations and no continuation requested.
- [x] Duplicate or identity-mismatched ready events cannot start work or mutate
  an unrelated execution.
- [x] The planner contract requires every generated step to record a
  recommended agent selected from the live catalog, keeps capability
  recommendations separate, and requires a bounded replan nudge rather than a
  fallback when that agent is absent at dispatch.
- [x] Recovery exposes exactly the agent's normal tool allowlist and resolves
  every repair through existing ward-confined tools; lint-report strings are
  never used directly as filesystem paths.
- [x] Valid ward behavior and the generic, role-agnostic template contract are
  unchanged.

## Assumptions

- Technical: the reproduced failure occurs before the continuation task is
  spawned, after the flag is cleared (source: captured session log and
  `runner/continuation_watcher.rs`, 2026-07-20).
- Product: ordinary template failures should nudge the model into repair rather
  than terminate orchestration (source: user confirmation 2026-07-20).
- Product: every plan step should recommend an available agent (source: user
  confirmation 2026-07-20).
- Process: the shipped fluid-runtime spec remains frozen and this repair is
  tracked separately (source: `docs/CONVENTIONS.md`).
