# Plan: Dynamic surface titles

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation learns.

## Approach

Extend the existing component property contract with `title_path`, reusing the
current JSON-pointer validation and bound-data limits. Resolve one title per
component in the React renderer and pass it to the heading, charts, tables, and
approval button. Update the model-facing tool guidance and AsyncAPI property
schemas so agents intentionally choose contextual titles. Preserve the
catalog identifier, event envelopes, and shared Quick Chat/Research rendering
path.

Tempted to add a title formatter or template language; declining because bound
plain text covers the requested behavior without creating an execution sink.
Tempted to add per-component title rules; declining because a single precedence
rule is predictable and testable. Tempted to add a new title component or
registry; declining because titles are already a shared component property.

## Constraints

- Preserve the `zbot/work-surface/v1` identifier and current event envelopes.
- Preserve the existing maximum title and bound-string sizes.
- Follow the runtime → gateway → React dependency direction in `CLAUDE.md`.
- Render agent-authored values as native escaped text and keep every existing
  surface component compatible.
- Register the active spec in `docs/specs/README.md` as lifecycle bookkeeping
  required by the `new-spec` workflow; this is not a product implementation
  task or acceptance criterion.

## Construction tests

- Integration: rerender the same surface descriptor with changed data and
  assert the heading and chart/table accessible label both update.
- Contract: parse `contracts/asyncapi/agent-surfaces.yaml` and verify
  `title_path` is present on every component property schema.
- Manual verification: none beyond renderer-visible DOM assertions and the
  production UI build.

## Design (LLD)

### Design decisions

- `title_path` is additive within catalog v1 because existing consumers already
  accept property maps and updated clients can resolve it without transport
  changes. Traces to AC1-AC5 and the AsyncAPI contract.
- Resolution order is bound title → static title → humanized component ID.
  Raw component type is never a user-facing fallback. Traces to AC1-AC4.

### Data & schema

- `title_path`: optional JSON pointer into surface `data`; a usable title is a
  non-empty string after trimming for the emptiness check, while displayed text
  preserves the supplied string.
- `title`: existing optional static string.
- `id`: existing stable component identifier and final fallback source.

### Interfaces & contracts

- Add `title_path` to all component prop schemas in
  `contracts/asyncapi/agent-surfaces.yaml` and its portable Rust allowlists.
- Keep `surface.created`, `surface.updated`, saved-surface REST payloads, and
  TypeScript transport envelopes unchanged.

### Component / module decomposition

- `runtime/agent-surfaces/src/lib.rs`: allow and validate `title_path`.
- `gateway/gateway-execution/src/tools/present_surface.rs`: tell agents to
  supply contextual static or bound titles.
- `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx`: resolve the title
  once per component and reuse it everywhere.
- Existing focused Rust and React test files carry the construction tests.

### State & control flow

On each render, resolve `title_path` against the current surface data. A
replacement surface naturally re-runs resolution; no component state,
persistence migration, or additional event is required.

### Behavior & rules

- Only a non-empty string is a usable bound or static title.
- Humanization replaces separator runs with spaces, splits lower-to-upper
  camel-case boundaries, trims whitespace, and capitalizes the first character.
- If humanization yields no text, use `Component` as the last defensive label,
  never the raw component type.

### Failure, edge cases & resilience

Malformed paths fail catalog validation. Missing, empty, or non-string bound
values fall through without hiding the component. React escaping keeps
HTML-like titles inert.

### Quality attributes (NFRs)

Existing 128-byte property and 4,096-byte bound-string limits remain in force.
Title-derived chart/table labels update consistently with headings.

## Tasks

### T1: Dynamic title descriptors pass the portable catalog contract

**Depends on:** none

**Touches:** `runtime/agent-surfaces/src/lib.rs`, `contracts/asyncapi/agent-surfaces.yaml`

**Verification modes:** TDD for the Rust property contract; goal-based check
for AsyncAPI parsing and schema synchronization.

**Tests:**
- A surface with `title_path` on every component type validates (AC1, AC8).
- Malformed pointers, unsupported title-like properties, overlong bound
  titles, and a representative over-budget collection bound through
  `title_path` fail through existing errors (AC5).

**Approach:**
- Add `title_path` to the closed property allowlists.
- Mirror it in every AsyncAPI component property schema.

**Done when:** `cargo test -p agent-surfaces` and the AsyncAPI parse check pass.

### T2: Every rendered title follows dynamic precedence

**Depends on:** T1

**Touches:** `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx`, `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx`

**Verification mode:** TDD

**Tests:**
- Bound, static, and humanized-ID title precedence is visible across structured,
  chart, table, and approval components (AC1-AC3, AC8).
- Rerendering data updates headings and accessible labels (AC4).
- HTML-like text remains inert (AC6).
- Static-title headings and labels remain unchanged, while component content
  and approval-action availability are identical before and after title
  resolution (AC8).

**Approach:**
- Resolve one title from component and data before the type switch.
- Reuse the resolved value for headings and title-derived labels.

**Done when:** the focused Vitest suite passes with the new precedence cases.

### T3: Agents are instructed to publish contextual titles

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/tools/present_surface.rs`

**Verification mode:** goal-based check

**Tests:**
- Tool tests assert the description documents contextual `title` and
  `title_path` behavior (AC7).

**Approach:**
- Update the tool description without changing its argument envelope.

**Done when:** the focused gateway tool tests pass.

## Rollout

Ship additively in the current catalog. Rollback removes `title_path` support;
no data migration or deployment sequencing is required.

## Risks

- Older clients may ignore `title_path`; static `title` remains available for
  mixed-version deployments.
- A blank or malformed component ID could produce a poor fallback; catalog
  validation already requires non-empty bounded IDs, and renderer tests cover
  separator-heavy IDs.

## Changelog

- 2026-07-29: Initial plan from confirmed dynamic-title precedence.
- 2026-07-29: Preserved legacy static-title validation for original catalog
  components and added a defensive fallback for IDs equal to raw type names.
