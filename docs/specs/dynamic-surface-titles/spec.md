# Spec: Dynamic surface titles

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/asyncapi/agent-surfaces.yaml`](../../../contracts/asyncapi/agent-surfaces.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Replace generic A2UI headings such as `LineChart` and `Callout` with
content-aware titles. Every catalog component may bind `title_path` to a
string in the existing surface `data` object; the renderer resolves the title
in this order: a non-empty bound title, a non-empty static `title`, then a
human-readable form of the component `id`. Replacing a surface or its data
updates the visible heading and every title-derived accessible label without
requiring a reload.

## Boundaries

### Always do

- Treat bound titles as untrusted display data and render them only as escaped
  React text.
- Reuse the catalog's existing JSON-pointer validation, data limits, and
  `surface.updated` replacement behavior.
- Apply one title-resolution rule to all current A2UI component types,
  including `ApprovalGate`.

### Ask first

- Changing the `zbot/work-surface/v1` catalog identifier or event envelope.
- Allowing titles to contain HTML, markup, formatters, URLs, or executable
  content.
- Adding a new component type, transport, persistence mechanism, or runtime
  dependency.

### Never do

- Never evaluate or inject title content as HTML, CSS, code, or a template.
- Never let a missing, empty, or wrong-shaped `title_path` hide the component
  or break sibling rendering.
- Never add a new module boundary, top-level directory, dependency, or
  title-specific state store for this feature.

## Testing Strategy

- Catalog property and pointer validation: **TDD** in `agent-surfaces`,
  because `title_path` is a closed property with deterministic validation and
  bounded-data rules.
- Renderer precedence, escaping, accessible labels, and update behavior:
  **TDD** with React Testing Library, because each case has an observable DOM
  result.
- Tool guidance and contract synchronization: **goal-based checks** against
  the tool schema text and AsyncAPI parser, because these are declarative
  artifacts.
- Quick Chat and Research integration: **goal-based integration check** using
  the shared renderer tests and production UI build; no transport behavior
  changes.

## Acceptance Criteria

- [x] Given a component whose `title_path` resolves to a non-empty string, the
  bound value is its visible heading and any title-derived accessible label.
- [x] Given both a valid bound title and static `title`, the bound title wins;
  when the bound value is missing, empty, or not a string, the static title
  wins.
- [x] Given neither a usable bound nor static title, the renderer humanizes the
  component `id` (for example, `monthly-revenue_chart` becomes
  `Monthly revenue chart`) and never displays the raw component type as its
  fallback.
- [x] Given replacement surface data for the same descriptor, a changed bound
  title rerenders in the heading and chart/table accessible label without a
  page reload.
- [x] Given a malformed `title_path`, an unsupported title property, an
  overlong bound string, or a bound value that violates existing surface
  limits, catalog validation rejects the surface before publication.
- [x] Given title text containing HTML-like input, React renders it as inert
  text and no element or executable content is created.
- [x] Given an agent chooses an A2UI component, the `present_surface` tool
  guidance tells it to supply a specific contextual `title` or `title_path`
  instead of relying on a generic type label.
- [x] Given an existing surface that uses a static title, its visible title and
  title-derived accessible labels remain unchanged; given any existing
  component descriptor, its non-title content and action availability remain
  unchanged.

## Assumptions

- Technical: component props already allow a static `title`, while the React
  renderer currently falls back to `component.type` when it is absent (source:
  `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx`).
- Technical: every `*_path` property already passes through JSON-pointer and
  bounded-value validation before publication (source:
  `runtime/agent-surfaces/src/lib.rs`).
- Technical: the existing AsyncAPI contract is the authoritative typed event
  interface for component properties (source:
  `contracts/asyncapi/agent-surfaces.yaml`).
- Technical: Quick Chat and Research share `A2uiSurfaceRenderer`, so one
  renderer change covers both clients (source:
  `apps/ui/src/features/chat-v2/QuickChat.tsx` and
  `apps/ui/src/features/research-v2/ResearchPage.tsx`).
- Process: changing component properties requires explicit approval under the
  shipped catalog spec, which the user provided (source:
  `docs/specs/a2ui-component-catalog/spec.md`; user confirmation 2026-07-29).
- Product: `title_path` takes precedence over static `title`, with a humanized
  component ID as the final fallback, across every component (source: user
  confirmation 2026-07-29).
- Product: this is a mixed runtime-contract and UI-rendering change to the
  existing AsyncAPI surface contract (source: user confirmation 2026-07-29).
