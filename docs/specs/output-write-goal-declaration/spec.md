# Spec: outputs/ writes auto-declare as goal artifacts

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light mode with live gate)
- **Constrained by:** #259 (respond goal-flag default)
- **Brief:** sess-22816ad4 variance — deliverable built but never declared in respond
- **Contract:** none
- **Shape:** service

## Objective

When the model builds a deliverable but omits it from `respond`, the
artifact stays invisible (the "missing chart" class, part 2). The ward's
`outputs/` directory IS the deliverable convention — so a successful
`write_file` into `outputs/` auto-declares a goal artifact, gateway-side.
No prompt teaching; respond declarations are unchanged (explicit
declarations still work and take their own path).

## Acceptance criteria

- [x] AC1 — `StreamContext.output_write_calls` tracks `write_file` calls
  whose normalized path starts at `outputs/` (bare relative and
  absolute-under-vault both normalize; non-outputs writes and other tools
  are ignored).
- [x] AC2 — On the corresponding successful ToolResult, the processor
  declares the file via the existing `process_artifact_declarations` with
  `is_goal_artifact: true`; the validator rejects writes that ultimately
  failed (file absent).
- [x] AC3 — Unit test pins normalization + tool/path gating; artifact suite
  green (16); full gates green.
- [x] AC4 — Live gate: a ward session whose deliverable is written to
  `outputs/` produces a goal-artifact row without any respond declaration
  of it. Verified on sess-2d3d4eca: log line `auto-declared outputs/
  deliverable as goal artifact artifact=fedex-ups-comparison.html` at
  write time; rows present with is_goal_artifact=1.

## Boundaries

### Never do

- No dedup of pre-existing duplicate artifact rows (pre-dates this change;
  the store keys by id, not (session, path)) — separate cleanup if wanted.
- No prompt text, no tool changes — gateway processor only.

## Testing strategy

Unit (normalization/gating), artifact suite, full gates, live ward run.
