# Spec: Agent Feedback Loops — present_surface & run_procedure errors that teach

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (two tasks, light mode)
- **Constrained by:** none
- **Brief:** session `sess-a0788ab4-9cdd-57b6-82b7-d04b764a2a04` (2026-08-20): present_surface failed 5× with identical args; run_procedure hit a permanently-broken stored procedure
- **Contract:** none
- **Shape:** service

## Mode

Light (no risk trigger: error-message enrichment + schema description inside
one tool; no structure/security/UI change). Stacked on `fix/fast-path-ward-note`
(daemon runs that tree).

## Objective

Tool errors must carry enough detail for the model to correct its next call,
and the tool schema must state the contract the validator enforces. Today
`present_surface` fails five times in a row with byte-identical args because
(a) the schema documents `props` as a free-form object — the per-component
contracts (e.g. DataTable `columns` = array of plain string field keys) exist
nowhere the model can see, and (b) `redacted_validation_error` flattens
`InvalidProperty { component, property }` — which thiserror already renders
safely as "invalid property {property} for component {component:?}" — into
the generic string "invalid component property".

## Acceptance criteria

- [x] AC1 — errors name the target: validation failures surfaced to the model
  include the component type and property name (no property VALUES — data
  stays redacted). The exact payload from the incident session
  (DataTable with `{key,label,format}` column objects) errors with a message
  containing both `DataTable` and `columns`.
- [x] AC2 — schema teaches the contract, across both model-visible
  surfaces: the tool `description()` enumerates the accepted property keys
  per component, and the `props` schema description states the field-key
  rule (DataTable `columns` and chart `series` are arrays of plain string
  field keys into `data`, never objects).
- [x] AC3 — `run_procedure` legacy-template error tells the model what to do:
  the message ends with guidance to abandon the procedure and do the task
  directly (do not retry).
- [x] AC4 — tests pin AC1 (incident payload → named target), AC3, and the
  value-redaction invariant (a sentinel property value never appears).
  Honesty note: fix and tests landed in the same working session — the red
  state was not observed at runtime; the AC1 assertion is structurally red
  against the previous generic-string mapping.

- [x] AC5 — fast-path ward note is a first-action mandate (live evidence:
  soft guidance ignored in sess-a0788ab4 [late entry] and sess-a006af36
  [no entry at all]; the wording now mirrors the graph path's proven
  "Your FIRST tool call must be ward(…)" family). Trivial-placeholder
  suppression unchanged.
- [x] AC6 — status-pill error state recovers on a successful tool result:
  successes emit a `tool_ok` pill event; the reducer clears the sticky
  error to "Recovered — continuing" (live evidence: sess-a006af36 showed
  "Tool error" at the top after the model had already corrected and
  succeeded — successes previously emitted no pill event at all).
  Stickiness against tool_call/respond/agent_completed is preserved.

## Boundaries

### Never do

- No validator semantics change: `{key,label,format}` columns still REJECT
  (enriching objects is a product decision, not this fix).
- No UI changes here (the stuck-error and slideout repros are re-tested
  after this lands; the 5-error sequence that produced them disappears).
- `run_procedure` step semantics unchanged — message text only.

### Always do

- Keep property VALUES out of every surfaced error (sentinel-tested).

## Testing strategy

TDD: `present_surface` tool test with the incident payload (red: today's
message lacks `DataTable`/`columns`); a value-redaction test (a property
carrying a sentinel string must not appear in the error); run_procedure
message test. Gates: fmt, clippy, `cargo test -p gateway-execution -p
agent-runtime`, `cargo check --workspace`, explicit
`--test e2e_ward_pipeline_tests`.
