# Plan: Quick Chat Terminal Response Deduplication

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Make terminal-result application idempotent in the existing Quick Chat reducer:
retain an already rendered final assistant bubble when the root completion
arrives, while preserving the existing fallback for a missed
`turn_complete` event.

## Tasks

### T1: Deduplicate a root terminal fallback in the reducer

**Depends on:** none

**Touches:** `apps/ui/src/features/chat-v2/reducer.ts, apps/ui/src/features/chat-v2/reducer.test.ts`

**Tests:**

- TDD: a `RESPOND`/`TURN_COMPLETE`/same-result `AGENT_COMPLETED` sequence
  leaves exactly one final assistant bubble (AC 1).
- Existing missed-event fallback test remains green (AC 2).

**Approach:**

- Add the red reducer regression test.
- Apply the terminal result only when it has not already been rendered.

**Done when:** focused Quick Chat reducer and event-map tests pass.

## Changelog

- 2026-07-17: Implemented and verified the terminal-response deduplication.
