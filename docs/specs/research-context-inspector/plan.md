# Plan: Research Context Inspector

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Keep the current Research hook and data contracts intact. Recompose the
existing intent detail and nested subagent cards into a conditional right
inspector, tell the central turn component not to render duplicate cards, and
extend the established workbench CSS for a third independently scrollable pane
that becomes normal document flow on narrow screens.

## Tasks

### T1: Show real Research context in a dedicated inspector

**Depends on:** none

**Touches:** `apps/ui/src/features/research-v2/ResearchPage.tsx`,
`apps/ui/src/features/research-v2/SessionTurnBlock.tsx`,
`apps/ui/src/features/research-v2/IntentInfoButton.tsx`,
`apps/ui/src/features/research-v2/research.css`,
`apps/ui/src/features/research-v2/*.test.tsx`

**Tests:**
- A session with intent analysis exposes it through the labelled right inspector
  and not a title-only popover (AC 1).
- A nested delegated-agent tree appears once in the inspector while the central
  reply remains visible (AC 2).
- The Research landing has no inspector or intent request; an existing session
  uses only its recorded intent state and delegated-agent data, with no new
  transport method or placeholder metrics (AC 3).
- Focused Research tests, `npm run lint`, and `npm run build` pass; manual
  1440px/1024px/390px checks find no clipped critical content or horizontal
  overflow (AC 4).

**Approach:**
- Extend the existing intent component with an inline inspector presentation
  that keeps its session-scoped fetch/cache behavior.
- Add an optional presentation flag to the existing session-turn component so
  it preserves final responses and root tool activity while moving only
  delegated-agent cards.
- Render the existing nested agent-card tree in a conditional labelled
  inspector and use responsive CSS to retain independent desktop scrolling.

**Done when:** intent and delegated-agent context are visible in a real right
inspector, the central thread has no duplicate cards, and all listed checks
are green.

### T2: Rebalance the desktop focus and collapse completed intent detail

**Depends on:** T1

**Touches:** `apps/ui/src/features/research-v2/ResearchPage.tsx`,
`apps/ui/src/features/research-v2/research.css`,
`apps/ui/src/features/research-v2/ResearchPage.test.tsx`

**Tests:**
- A completed session renders an intent-summary disclosure as collapsed, and
  clicking it exposes and hides the recorded analysis without affecting agent
  cards.
- Focused Research tests, lint, and build pass; manual desktop inspection
  confirms the wider context pane and a slightly narrower central thread.

**Approach:**
- Use a local accessible button, with `aria-expanded` and `aria-controls`,
  rather than adding a generic accordion.
- Reserve more grid width for the right inspector only above the existing
  tablet breakpoint, preserving the stacked responsive layout below it.

**Done when:** completed intent analysis is compact but accessible, running
analysis opens itself, and agent history has visibly more horizontal room.

## Declined design alternatives

- A new aggregate context API or polling loop: declined because the current
  session state and intent endpoint already contain the needed data.
- Generic Plan/Agents/Artifacts metric cards: declined because they would not
  add a user action and would duplicate existing real surfaces.
- Copying the agent cards into both panes: declined because the request is to
  move hidden context, not create competing versions of it.
- A new accordion component: declined because the inspector needs only one
  small, semantic disclosure control and an extra abstraction would add no
  reuse value.
