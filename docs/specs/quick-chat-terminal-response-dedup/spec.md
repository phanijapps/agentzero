# Spec: Quick Chat Terminal Response Deduplication

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Mode:** light (no risk trigger fired)

## Objective

When a Quick Chat response is delivered through both `turn_complete` and
`agent_completed`, show one final assistant bubble. Keep the terminal result
as the fallback when `turn_complete` is absent.

## Acceptance Criteria

- [x] Given a Quick Chat turn whose `respond` final message is followed by a
  root `agent_completed` event with the same result, the user sees exactly one
  final assistant bubble and the turn becomes idle.
- [x] Given a Quick Chat turn that misses `turn_complete`, a root
  `agent_completed.result` still renders one final assistant bubble.
- [x] A delegated child `agent_completed` event cannot alter the root Quick
  Chat response.

## Assumptions

- Technical: `turn_complete.final_message` and root `agent_completed.result`
  both dispatch into the Quick Chat reducer (source:
  `apps/ui/src/features/chat-v2/useQuickChat.ts`).
- Product: the terminal result is a fallback, never a second response (source:
  user report 2026-07-17).
- Process: this is a reducer-only defect correction with no wire, dependency,
  or schema change (source: `docs/CONVENTIONS.md`).
