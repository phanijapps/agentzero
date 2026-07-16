# Plan: Quiet Instrument UI

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

First establish a shared Quiet Instrument token and component vocabulary, then
apply it to the application shell and all route surfaces. Page work reuses a
small set of layouts: conversation canvas, operational split pane, canvas-first
inspector, file-workbench, and configuration shell. This keeps the redesign
coherent while leaving gateway and UI data contracts unchanged.

The [Attention Radar UI Rollout](../attention-radar-ui-rollout/spec.md) is the
successor visual contract for subsequent route composition and art direction.
This plan's shared-token, behavior-preservation, and accessibility constraints
remain the baseline; it no longer competes as visual implementation authority.

## Constraints

- Preserve the existing React, Tailwind, Radix, and shared UI component stack.
- Do not add UI dependencies, routes, or API contracts.
- Preserve semantic colors and accessible focus states.

## Construction tests

**Integration tests:** existing page tests continue to render their primary
route states and interactive controls.

**Manual verification:** load each route at 1440px and 390px widths; verify
navigation, empty/loading/live states, panels, drawers, and primary controls
remain readable, operable, and non-overlapping.

## Design (LLD)

### Design decisions

Quiet Instrument uses graphite/zinc neutral layers, a single configurable
electric accent, 4–8px control geometry, sans UI typography, and mono metadata.
Live work is communicated through compact status marks and motion, not permanent
decorative effects. Traces to: AC1, AC3, AC4.

### Component / module decomposition

`styles/theme.css` defines tokens; `styles/components.css` supplies reusable
shell, panel, toolbar, and inspector vocabulary; existing shared shadcn-style
components inherit the tokens. Feature-local CSS applies each page's layout
without inventing a parallel component system. Traces to: AC1, AC2.

### State & control flow

Existing route state and data hooks remain the source of truth. Redesign work
only changes visual hierarchy: active selection, running state, detail views,
and empty/loading/error states map to existing component state. Traces to: AC2,
AC3, AC5.

### Behavior & rules

Operational pages use split panes and right inspectors; graph pages remain
canvas-first; conversation pages reserve the central canvas for messages;
configuration routes use the same form/table/section patterns. Traces to: AC2,
AC4.

### Quality attributes (NFRs)

Use responsive grid constraints, stable panel dimensions, visible focus rings,
and semantic labels. Verify real browser output at desktop and mobile widths.
Traces to: AC3, AC4, AC5.

## Tasks

### T1: Shared system renders as Quiet Instrument

**Depends on:** none

**Touches:** `apps/ui/src/styles/*.css`, `apps/ui/src/shared/ui/*.tsx`

**Tests:**
- Goal-based: UI lint and production build pass after token and primitive changes
  (AC1, AC5).
- Visual/manual QA: controls have consistent geometry, focus, and semantic state
  treatments at desktop and mobile widths (AC1, AC3, AC4).

**Approach:**
- Replace decorative dark-theme tokens with neutral operational tokens.
- Tighten shared Button, Card, and other primitives to shadcn-style geometry.
- Add shared shell, pane, toolbar, and inspector classes.

**Done when:** the app shell and shared primitives visibly use one restrained
design language without changing their public props.

### T2: Application shell and navigation share the new hierarchy

**Depends on:** T1

**Touches:** `apps/ui/src/App.tsx`, `apps/ui/src/styles/components.css`

**Tests:**
- Goal-based: existing app-shell tests and build pass (AC1, AC2, AC5).
- Visual/manual QA: desktop and narrow navigation remain readable and operable
  without overflow (AC4).

**Approach:**
- Restyle the global shell, top bar, route navigation, page headers, connection,
  empty, and loading surfaces.

**Done when:** navigating between routes retains a stable, polished frame.

### T3: Primary operational routes use the shared workspace vocabulary

**Depends on:** T1, T2

**Touches:** `apps/ui/src/features/chat-v2/**`, `apps/ui/src/features/mission-control/**`, `apps/ui/src/features/memory/**`, `apps/ui/src/features/observatory*/**`

**Tests:**
- Goal-based: affected feature tests and UI build pass (AC2, AC5).
- Visual/manual QA: chat, mission-control, memory, and graph pages show clear
  focus, compact live status, and usable panels at both target widths (AC2–AC4).

**Approach:**
- Restyle conversation, operations, recall, and graph canvases using shared
  layouts and local CSS only where each workflow needs distinct structure.

**Done when:** primary runtime pages look related and retain their current
interactions.

### T4: Research, vault, and configuration routes complete the system

**Depends on:** T1, T2

**Touches:** `apps/ui/src/features/research-v2/**`, `apps/ui/src/features/vault/**`, `apps/ui/src/features/agent/**`, `apps/ui/src/features/integrations/**`, `apps/ui/src/features/settings/**`, `apps/ui/src/features/setup/**`

**Tests:**
- Goal-based: affected feature tests, lint, and UI build pass (AC2, AC5).
- Visual/manual QA: research, vault, agent, integration, setting, and setup
  surfaces use the same form, table, panel, and inspector treatment (AC2, AC4).

**Approach:**
- Restyle secondary routes and setup flows to use the established page grammar.

**Done when:** no current route visually depends on the retired holographic
language.

### T5: Route-level visual verification is recorded

**Depends on:** T1-T4

**Touches:** `apps/ui/`, `docs/specs/quiet-instrument-ui/`

**Tests:**
- Visual/manual QA: load every route at 1440px and 390px widths; exercise one
  primary interaction on the operational routes (AC1–AC5).
- Goal-based: `npm run lint`, `npm test`, and `npm run build` pass (AC5).

**Approach:**
- Run the automated gates and visual browser pass.
- Capture any residual defects in the spec or backlog rather than leaving them
  implicit.

**Done when:** every acceptance criterion has direct verification evidence.

## Rollout

- **Delivery:** one reversible UI-only branch; no migration or backend rollout.
- **Deployment sequencing:** shared tokens and shell land before page-local
  styling; existing APIs and routes remain continuously compatible.

## Risks

- Existing feature-local CSS may override shared tokens, requiring targeted
  cleanup to prevent mixed visual states.
- Visual quality cannot be established by unit tests alone, so browser QA is a
  required final gate.

## Changelog

- 2026-07-09: initial plan.
- 2026-07-09: implemented the Quiet Instrument system across all existing
  routes; full UI suite, lint, production build, and desktop/mobile route
  sweeps completed.
