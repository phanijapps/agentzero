# Spec: Quiet Instrument UI

- **Status:** Implementing
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Give zbot one striking, restrained desktop-command-center visual language across
every route. Users can move between conversation, research, operations, memory,
knowledge graph, vault, and configuration views without encountering a legacy
surface or a different interaction vocabulary. The redesign preserves all
existing routes, data contracts, and runtime behavior.

## Boundaries

### Always do

- Use the existing React, Tailwind, Radix, Lucide, and shared UI primitives.
- Apply the same semantic tokens, typography, spacing, focus treatment, and
  status language to every route.
- Preserve accessible labels, keyboard behavior, existing routes, and transport
  contracts while visual structure changes.

### Ask first

- Adding a new UI dependency or changing the public UI/API data contracts.
- Replacing an interaction that changes session, message, memory, or tool-call
  behavior rather than its presentation.

### Never do

- Do not introduce a new top-level UI framework, component library, or route
  boundary.
- Do not alter gateway APIs, persistence, session behavior, or agent execution.
- Do not retain decorative scanlines, page grids, broad gradients, or always-on
  neon glows as the visual identity.

## Testing Strategy

- Shared primitive and route rendering behavior: existing React/Vitest tests and
  targeted updates where markup changes make an observable behavior ambiguous.
- Type safety, linting, and production build: goal-based checks, because these
  establish that all routes still compile against their unchanged contracts.
- Visual system, responsive layout, and operational states: visual/manual QA in
  a running UI at desktop and mobile widths, because the acceptance criteria are
  about what users can see and operate.

## Acceptance Criteria

- [x] Given any application route, when it renders, the user sees the shared
  Quiet Instrument token system: neutral dark surfaces, restrained accent use,
  compact radii, consistent typography, and coherent focus/selection states.
- [x] Given Quick Chat, Research, Mission Control, Memory, Observatory, Vault,
  Agents, Integrations, or Settings, when the user changes routes, the page uses
  the shared navigation, page-header, control, status, and inspector vocabulary.
- [x] Given an active, waiting, successful, or failed operation, when its state
  is visible, the user can distinguish it through consistent semantic status
  treatment without relying on broad panel glows or color alone.
- [x] Given desktop and narrow mobile widths, when a primary route is loaded,
  its content remains readable, scrollable, operable, and free of unintended
  overlap or horizontal page overflow.
- [x] Given the redesign branch, when the UI lint, test, and build gates run,
  existing UI behavior and route contracts remain valid.

## Assumptions

- Technical: React 19, Tailwind 4, Radix primitives, Lucide icons, and shadcn-
  style shared components are already available (source: `apps/ui/package.json`).
- Technical: the current UI token system and shared component layer can be
  reskinned without a dependency change (source: `apps/ui/src/styles/theme.css`,
  `apps/ui/src/shared/ui/`).
- Product: every current route adopts one consistent visual language, including
  configuration pages (source: user confirmation 2026-07-09).
- Product: this branch remains dark-only and preserves behavior while the
  visuals are evaluated (source: user confirmation 2026-07-09).
- Process: multi-page UI work follows the full implementation loop with visual
  QA and mechanical UI gates (source: `docs/CONVENTIONS.md`).
