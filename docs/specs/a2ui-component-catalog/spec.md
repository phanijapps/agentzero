# Spec: A2UI component catalog

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

Expand zbot's native A2UI catalog with ten display-only components so agents
can present metrics, status, structured records, timelines, and responsive
charts without supplying frontend code. Capable React clients render
`MetricCard`, `ProgressBar`, `StatusBadge`, `Callout`, `KeyValueList`,
`DataTable`, `Timeline`, `LineChart`, `BarChart`, and `PieChart` from
JSON-pointer-bound surface data. Replacing a surface through the existing
`surface.updated` event updates every component, including charts, while
canonical text and artifacts remain authoritative for all clients.

## Boundaries

### Always do

- Keep every new component display-only and bind its content through validated
  JSON pointers into the existing surface `data` object.
- Render malformed, missing, or empty bound data as a safe native empty state
  without throwing or blocking the rest of the surface.
- Use native zbot styling tokens and accessible labels; enable Recharts'
  accessibility layer for every chart.

### Ask first

- Adding user input, navigation, write-capable actions, or external side effects
  to any new component.
- Adding another runtime dependency beyond the user-approved Recharts package.
- Changing the catalog identifier, A2UI protocol version, or existing component
  property contract.

### Never do

- Never evaluate agent-supplied code, HTML, CSS, formatter functions, URLs, or
  event handlers.
- Never make a surface or chart renderer necessary for an execution to finish
  or replace canonical text and artifacts.
- Never add a new top-level package, persistence layer, transport, or gateway
  action for this catalog expansion.

## Testing Strategy

- Catalog enum, property allowlists, and JSON-pointer validation: **TDD** in
  `agent-surfaces`, because each component has a closed, deterministic property
  contract.
- Native rendering, empty states, sanitization, and accessible chart output:
  **TDD** with React Testing Library, because each surface descriptor maps to an
  observable DOM result.
- Existing `surface.updated` replacement behavior: **goal-based integration
  check** against the current WebSocket state reducer plus a renderer rerender
  test, because transport semantics already exist and must remain unchanged.
- Responsive theme presentation: **visual/manual QA** at narrow and wide
  container widths after the automated UI build passes.

## Acceptance Criteria

- [x] Given a catalog-valid surface, each of `MetricCard`, `ProgressBar`,
  `StatusBadge`, `Callout`, `KeyValueList`, `DataTable`, and `Timeline` renders
  native, theme-aware content from its documented JSON-pointer bindings.
- [x] Given chart-shaped surface data, `LineChart`, `BarChart`, and `PieChart`
  render responsive Recharts SVG output with an accessible chart label,
  keyboard accessibility layer, tooltip, and legend where multiple series or
  slices are present.
- [x] Given a `surface.updated` event with an existing `surface_id`, the client
  replaces the prior descriptor and every affected value or chart series
  rerenders without a page reload.
- [x] Given a new component with an unsupported property or a malformed
  `*_path` property, catalog validation rejects the surface before publication.
- [x] Given a surface exceeds 64 KiB, a bound collection exceeds 200 records,
  a table declares more than 20 columns, a Cartesian chart declares more than
  8 series, a pie chart binds more than 50 slices, a field key exceeds 64
  bytes, or a rendered string exceeds 4,096 bytes, catalog validation rejects
  it before WebSocket publication and React/Recharts rendering.
- [x] Given missing, empty, or wrong-shaped bound data, the affected component
  renders a safe empty state and sibling components continue rendering.
- [x] Given an agent attempts to provide HTML, executable code, a URL, a
  formatter, or an action through a new component, no new component contract
  accepts or executes it.
- [x] Given a surface using any of the original six components, its validation,
  rendering, and action behavior remains unchanged.

## Assumptions

- Technical: the existing `surface.updated` event replaces a surface by stable
  identifier and needs no transport change (source:
  `apps/ui/src/features/chat-v2/useQuickChat.ts`).
- Technical: the current catalog is a closed Rust enum with per-component
  property allowlists mirrored by the AsyncAPI and TypeScript contracts
  (source: `runtime/agent-surfaces/src/lib.rs`,
  `contracts/asyncapi/agent-surfaces.yaml`,
  `apps/ui/src/services/transport/types.ts`).
- Technical: React 19 and Vite remain the web stack; Recharts 3.x is the
  approved chart dependency (source: `apps/ui/package.json`; user confirmation
  2026-07-28).
- Security: agent-authored surface descriptors and data remain untrusted across
  validation and rendering, and dependency intake follows the repository
  boundary controls (source: `docs/architecture/security.md`).
- Technical: the repository has no normative
  `docs/architecture/reference.md`, so the design follows the established
  runtime → gateway → React layering (source: repository check 2026-07-28).
- Product: the first expansion contains the ten components named in the
  Objective and all are display-only (source: user confirmation 2026-07-28).
- Product: charts are responsive, theme-aware, accessible, and update from
  replacement surface data (source: user confirmation 2026-07-28).
- Process: new executable content, side effects, or catalog-version changes
  remain outside this feature (source:
  `docs/specs/agent-driven-surfaces/spec.md`; user confirmation 2026-07-28).
