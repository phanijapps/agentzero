# Plan: A2UI component catalog

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog.

## Approach

Extend the existing `zbot/work-surface/v1` catalog additively from the portable
Rust model outward: first add closed component/property contracts and mirror
them in AsyncAPI and TypeScript, then implement small native renderers in the
existing A2UI registry. Structured display components use semantic HTML.
Charts share one bounded data-normalization path and use Recharts 3.x with
responsive containers and the accessibility layer. Focused Rust and React
tests exercise the closed contract, safe fallbacks, and rerender behavior.

Tempted to add a generic component factory; declining because the closed
switch keeps the security boundary obvious. Tempted to add chart formatter
callbacks and palette configuration; declining because model-controlled
functions or styling would violate the catalog boundary. Tempted to add a new
surface transport; declining because `surface.updated` already provides the
required live behavior.

## Constraints

- Preserve the boundaries and canonical-result behavior in
  `docs/specs/agent-driven-surfaces/spec.md`.
- Preserve the `zbot/work-surface/v1` identifier and existing six component
  contracts.
- Follow the runtime → gateway → React dependency direction documented in
  `CLAUDE.md`; the portable surface crate cannot depend on UI or transport
  types.
- Follow `apps/ui/ARCHITECTURE.md`: semantic component classes, theme tokens,
  and no agent-controlled inline styles.

## Construction tests

- **Integration tests:** rerender one chart and one structured component with a
  replacement `WorkSurface` sharing the same identifier; assert updated values
  replace old values. Drive `surface_created` followed by `surface_updated`
  through `useQuickChat`'s subscribed WebSocket handler and assert the stable
  identifier holds only the replacement descriptor (AC3).
- **Goal-based gates:** `cargo test -p agent-surfaces`; UI focused test;
  `npm run build` from `apps/ui`; formatting and lint/type checks for touched
  code.
- **Manual verification:** inspect one mixed fixture at desktop and narrow
  width, verifying legible tables, non-overflowing charts, theme tokens, and
  representative keyboard tooltips. The React integration test performs the
  exhaustive accessible-tooltip check for line, bar, and pie charts before and
  after replacement data (AC1, AC2, AC3).

## Design (LLD)

### Design decisions

- Keep the additive components in `zbot/work-surface/v1`; unknown components
  remain rejected server-side, while capable updated clients render the
  expanded enum. Traces to AC1-AC4 and
  `contracts/asyncapi/agent-surfaces.yaml`.
- Use declarative Recharts primitives only. Agents select data bindings and
  field keys, never component code, colors, formatters, or handlers. Traces to
  AC2 and AC6.

### Data & schema

- `MetricCard`: `value_path`; optional `label`, `detail_path`.
- `ProgressBar`: `value_path`; optional `label`, `max`.
- `StatusBadge`: `value_path`; optional `label`.
- `Callout`: `message_path`; optional `tone` (`info`, `success`, `warning`,
  `error`).
- `KeyValueList`: `items_path`; items are records rendered as key/value pairs.
- `DataTable`: `rows_path`; optional `columns` string array. Without columns,
  columns derive from the first record.
- `Timeline`: `items_path`; each record may provide `title`, `time`,
  `description`, and `status`.
- `LineChart` / `BarChart`: `data_path`, `x_key`, and `series` string array.
- `PieChart`: `data_path`, `name_key`, and `value_key`.

All path properties are JSON pointers validated by the existing catalog
validator. Renderer normalization accepts only arrays, plain records, strings,
and finite numbers appropriate to each component. Validation also enforces the
existing 64 KiB surface payload limit plus 200 bound records, 20 table columns,
8 Cartesian series, 50 pie slices, 64-byte field keys, and 4,096-byte rendered
strings before publication.

### Interfaces & contracts

- Extend the component enum in
  `contracts/asyncapi/agent-surfaces.yaml`, retaining the existing
  `surface.created` and `surface.updated` payload shape and adding the reverse
  `x-spec` pointer to this spec.
- Mirror the enum in `WorkSurfaceComponent` under
  `apps/ui/src/services/transport/types.ts`.

### Component / module decomposition

- `runtime/agent-surfaces/src/lib.rs`: portable component enum and property
  allowlists.
- `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx`: static registry,
  structured components, data normalization, and chart composition.
- `apps/ui/src/features/surfaces/a2ui-surface.css`: token-based layout and
  responsive chart styles.
- `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx`: catalog
  rendering, safety, accessibility, and rerender coverage.

### State & control flow

The existing WebSocket reducers replace a `WorkSurface` by `surface_id`. React
receives the replacement descriptor, resolves every JSON pointer again, and
rerenders components. The chart components own no durable state.

### Behavior & rules

- Non-finite chart values and wrong-shaped records are omitted; no valid rows
  yields the common empty state.
- Component titles and labels render as text nodes only.
- Progress values clamp visually between zero and `max`, while displaying the
  supplied value.
- Status and callout variants map through a closed renderer-owned tone set.

### Failure, edge cases & resilience

An invalid descriptor is rejected by Rust validation. Data-shape problems that
can arise after valid binding render locally as `Nothing to display.` without
throwing. Recharts never receives agent-controlled functions or element types.

### Quality attributes (NFRs)

- Accessibility: semantic HTML for structured components, labelled progress,
  and Recharts `accessibilityLayer` plus an enclosing chart label.
- Responsiveness: charts render in a fixed-minimum-height,
  width-100-percent `ResponsiveContainer`.
- Security: the property allowlist excludes HTML, URLs, functions, actions,
  formatters, and CSS.

### Dependencies & integration

Add `recharts` 3.x to `apps/ui/package.json` and lockfile. No gateway, storage,
or deployment dependency changes are required.

## Tasks

### T1: Expanded portable catalog rejects every undeclared property

**Depends on:** none

**Touches:** `runtime/agent-surfaces/src/lib.rs`, `contracts/asyncapi/agent-surfaces.yaml`, `apps/ui/src/services/transport/types.ts`

**Tests:**
- TDD: every new enum value accepts only its documented properties and valid
  JSON pointers, and rejects descriptors or bound data over the documented
  budgets (AC4, AC5, AC7).
- Regression: all original component validation tests remain green (AC8).
- `catalog_expansion_accepts_only_bounded_descriptors` in
  `runtime/agent-surfaces/src/lib.rs` is materialized as the PLAN-stage red
  test. `stub: true` (AC4, AC5, AC7, AC8).

**Approach:**
- Add the ten `ComponentType` variants and closed property lists.
- Extend the AsyncAPI enum and `x-spec` traceability.
- Mirror the union in the UI transport type.

**Done when:** `cargo test -p agent-surfaces` passes and all three contracts
name the same sixteen component types.

### T2: Structured components render bounded native content

**Depends on:** T1

**Touches:** `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx`, `apps/ui/src/features/surfaces/a2ui-surface.css`, `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx`

**Tests:**
- TDD: fixtures render the seven structured components from bound data (AC1).
- TDD: missing and wrong-shaped values render safe empty states without hiding
  siblings (AC6).
- TDD: supplied markup remains text and cannot create arbitrary elements (AC7).
- `renders_the_expanded_structured_catalog_safely` in
  `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx` is materialized
  as the PLAN-stage red test. `stub: true` (AC1, AC6, AC7).

**Approach:**
- Add small semantic renderers and shared record/value guards.
- Add closed tone mapping and token-based styles.

**Done when:** the focused renderer tests pass for all seven components and
their empty states.

### T3: Responsive charts render and rerender declaratively

**Depends on:** T1

**Touches:** `apps/ui/package.json`, `apps/ui/package-lock.json`, `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx`, `apps/ui/src/features/surfaces/a2ui-surface.css`, `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx`

**Tests:**
- TDD: line, bar, and pie descriptors render labelled chart containers and the
  expected series/slice declarations (AC2).
- TDD: multi-series and multi-slice fixtures expose legends, every Cartesian
  chart enables Recharts' keyboard accessibility layer, and keyboard focus
  exposes tooltip labels and finite values (AC2).
  - Integration: keyboard-focus each line, bar, and pie chart before and after
  replacement data, and assert the accessible tooltip changes to the new datum
  label and finite value (AC2, AC3).
- TDD: malformed and non-finite chart rows produce the safe empty state (AC6).
- Integration: rerendering the same surface with new data removes the old
  values and exposes the replacements (AC3).
- Goal-based integration:
  `useQuickChat — WS subscription lifecycle > replaces an existing surface
  when surface_updated arrives` exercises the subscribed client event path and
  stable-ID replacement. `stub: no stub (goal-based integration mode)` (AC3).
- `renders_accessible_dynamic_charts` in
  `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx` is materialized
  as the PLAN-stage red test. `stub: true` (AC2, AC3, AC6).

**Approach:**
- Install Recharts 3.x.
- Add bounded chart normalization, renderer-owned palette, responsive
  containers, tooltips, legends, and `accessibilityLayer`.

**Done when:** focused UI tests and the production UI build pass.

### T4: Catalog expansion passes repository gates and visual QA

**Depends on:** T2, T3

**Touches:** all files above

**Tests:**
- Goal-based: Rust format/check/test and UI lint/test/build gates pass.
- Visual/manual: mixed fixture remains readable at narrow and desktop widths
  in the existing theme (AC1, AC2).
- `stub: no stub (goal-based and visual/manual QA modes)`.

**Approach:**
- Run mechanical gates, fix only catalog-expansion regressions, and inspect the
  final diff for unrelated changes.

**Done when:** all scoped automated gates pass and manual observations are
recorded in the implementation handoff.

## Rollout

Ship additively in one application release. The server and bundled web client
must land together because the catalog enum expands in both. Rollback is the
code/package-lock revert; there is no migration, stored state, infrastructure,
or irreversible external change.

## Risks

- Recharts' SVG output can make DOM-level tests brittle; tests should assert
  accessible wrappers and data-driven labels instead of internal SVG paths.
- Wide tables and high-cardinality series can become dense; the initial
  renderer remains horizontally bounded and does not promise aggregation.
- Expanding `v1` assumes bundled clients update with the server; independently
  versioned external clients may omit unknown components and still retain the
  canonical result.

## Changelog

- 2026-07-28: initial plan; user approved ten display-only components and
  Recharts.
- 2026-07-28: reopened a focused verification pass after the first work-loop
  reached its review cap; scope is limited to derived-table and pie-rerender
  regression coverage, recorded browser QA, specialist re-review, and final
  gates.
