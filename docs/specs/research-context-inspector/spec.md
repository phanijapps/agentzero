# Spec: Research Context Inspector

Mode: light (no risk trigger fired)

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make the active Research goal easier to follow without turning it into a
dashboard. The bound ward filesystem remains on the left, the conversation and
AG-UI stay in the central focus pane, and the existing intent analysis plus
delegated-agent activity move to an independently scrollable right context
inspector.

## Boundaries

### Always do

- Reuse the existing Research session state and subagent card presentation.
- Keep final assistant replies, user messages, AG-UI surfaces, artifacts, and
  their current actions in the central focus pane.
- Render the inspector only when the current session has real intent or
  delegated-agent context.

### Ask first

- Adding an API, polling path, new session state, or a generic metrics panel.

### Never do

- Do not replace the left ward filesystem or reintroduce static counters,
  duplicate artifact controls, or fabricated agent data.

## Testing Strategy

- **TDD:** focused React tests prove intent and subagent cards render only in
  the labelled inspector while final replies remain in the thread.
- **Goal-based:** focused UI tests, lint, and build prove the existing page and
  its routes still compile.
- **Visual/manual QA:** inspect the bound-ward active session at 1440px,
  1024px, and 390px for independent desktop scrolling and accessible stacked
  content without horizontal overflow.

## Acceptance Criteria

- [x] Given an active Research session with an intent analysis, when it loads,
  the user can read the intent status and full analysis in a labelled right
  context inspector rather than a hidden title popover.
- [x] Given an active Research session with delegated agents, when it loads,
  the user can inspect its existing root/subagent request and response cards in
  the right context inspector while the primary thread does not duplicate them.
- [x] Given the Research landing page or a session without confirmed intent or
  delegated-agent context, when it loads, no context inspector or intent
  request appears; otherwise its inspector contains only the recorded intent
  state and delegated agents, never placeholder metrics or changed
  artifact/AG-UI behavior.
- [x] Given desktop, tablet, and narrow layouts, when a pane has more content
  than its height, all real content remains accessible without horizontal page
  overflow.
- [x] Given the desktop three-pane layout, the central conversation stays
  comfortably narrower while the right Research context inspector receives a
  wider, readable working column; tablet and narrow layouts keep their current
  stacked flow.
- [x] Given recorded intent analysis, the inspector always shows an accessible
  intent summary and offers a labelled toggle for its full analysis; completed
  sessions begin collapsed while an in-progress analysis opens automatically.

## Assumptions

- Technical: `ResearchPage` already owns the session state, renders
  `SessionTurnBlock`, and contains the existing intent summary and title
  popover entry point (source: `apps/ui/src/features/research-v2/ResearchPage.tsx`).
- Technical: every session turn already contains its complete flat delegated
  agent list, and `SubagentCardTree` can preserve nested display without a new
  data model (source: `apps/ui/src/features/research-v2/types.ts`,
  `AgentTurnBlock.tsx`).
- Process: this is a living feature spec with a plan and verifiable acceptance
  criteria (source: `docs/CONVENTIONS.md` §4).
- Product: the filesystem stays on the left; the right pane contains only
  intent and agent/subagent context, not metric cards or duplicate controls
  (source: user confirmation 2026-07-15).
- Product: intent detail should not displace agent history by default after it
  has completed, but must remain one keyboard-accessible toggle away (source:
  user confirmation 2026-07-15).
