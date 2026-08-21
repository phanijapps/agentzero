# Spec: Ward Slim P5 — Placeholder-Gate Redirect Unification

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light mode)
- **Constrained by:** [ward-slim P1+P2](../ward-slim/spec.md) (P2 introduced the shared-envelope pattern)
- **Brief:** ward-slim program, final phase
- **Contract:** none
- **Shape:** service

## Objective

The placeholder-specs gate ("planning not finished in this ward") exists at
three sites with three drifting messages — the same divergence that hit the
cold-graph redirect before P2 unified it. Adopt the shared-envelope helper
pattern: one canonical core message in `guards.rs`, each site appends only
its action-specific tail.

Sites (all `app:has_placeholder_specs`, root-only, state keys unchanged):
- `delegate.rs` (block ad-hoc delegation)
- `execution/skills.rs` (block load_skill)
- `execution/update_plan.rs` (block root-written plans)

## Acceptance criteria

- [x] AC1 — `guards.rs` exports `placeholder_specs_redirect(instead: &str)`
  returning `{"status":"redirect","message": "This ward has placeholder
  specs — planning is not finished. {instead}"}`. One source of truth.
- [x] AC2 — All three sites call it; each `instead` tail names that site's
  correct alternative in one sentence. No literal core message remains at
  the call sites.
- [x] AC3 — Gate conditions, state keys, and allow-lists (planning-task
  pass-through in delegate) are byte-for-byte unchanged — message format
  only.
- [x] AC4 — Tests: helper unit test (core shape), one assertion per site
  (core + tail), existing suites green.

## Boundaries

### Never do

- No merging of the placeholder gate with the planning gate (different
  lifecycles: invocation-local vs ward-persistent).
- No condition or state-key changes anywhere.

## Testing strategy

Unit tests at the helper + the three sites; full gates: fmt, clippy,
`cargo test -p agent-tools -p agent-runtime`, e2e ward binary, workspace
check. No live gate — message-only change, no orchestration-path semantics.
