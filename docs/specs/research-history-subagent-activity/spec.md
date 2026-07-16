# Spec: Research History Subagent Activity

Mode: light (no risk trigger fired)

- **Status:** Shipped
- **Shape:** ui regression fix

## Objective

Show the safe, recorded tool-call history for delegated agents in Research's
right-hand context inspector after a session is reopened.

## Acceptance Criteria

- [x] Given a completed Research session whose delegated execution recorded
  non-response tool calls, reopening it retains those tool names and final
  response on that agent's card in the right context inspector.
- [x] Given nested delegated executions, reopening the session preserves their
  actual parent-child tree and does not duplicate their cards in the central
  conversation.
- [x] Given a persisted tool call, the UI shows only its name and existing
  human-readable activity label; arguments and tool results remain hidden.
- [x] Given a reopened session with delegated executions, its snapshot queries
  the logs API by that session's conversation id, so unrelated newer
  executions cannot push its agent cards out of the default global page.
- [x] Given an agent delegated by a continuation turn, filtering by the
  canonical session id returns that agent after the API normalizes its internal
  `-cont-…` conversation id.

## Tasks

1. Extend snapshot reconstruction to hydrate each delegated execution from its
   already-fetched historical messages.
2. Render the reconstructed safe tool activity in the existing subagent card.
3. Pin the regression with focused history and card-rendering tests, then run
   the UI gates and an adversarial review.
4. Scope the existing log-list transport call to the reopened conversation and
   retain a regression test for both the snapshot result and generated query.
5. Make the API's `conversation_id` predicate use the same canonicalization as
   its returned conversation ids, and cover a delegated continuation turn.

## Declined design alternatives

- A second per-agent history request: declined because `scope=all` already
  returns every execution message for the session.
- Tool arguments or result previews: declined because the existing activity
  surface deliberately exposes only safe tool names.
- Raising the global log-list limit: declined because it only postpones the
  failure and wastes bandwidth; the API already provides an exact
  `conversation_id` filter.
