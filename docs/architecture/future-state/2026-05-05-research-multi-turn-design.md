# Research UI — multi-turn rendering

**Status:** Design (ready to implement)
**Date:** 2026-05-05
**Owner:** phanijapps
**Target branch:** `develop` (`feat/research-multi-turn`)

## The bug, one paragraph

A research session can carry multiple user→assistant exchanges within a single root execution (the user sends a follow-up message; the daemon adds it to the same root). The backend stores everything correctly: timestamps on every message, `started_at`/`ended_at` on every execution row, `parent_session_id` linking subagents to the root. The UI ignores all of that and renders the session as a single flat unit: every user bubble at the top, every subagent in one cluster (sorted by `started_at`), and exactly one assistant reply (the latest text in the root execution, last-write-wins). The result for a two-turn session is `user1 → user2 → all-subagents-in-a-clump → reply2`, which destroys the conversational chronology and the user can't tell which subagents did which work.

The fix is purely frontend. Every primitive we need is already in `/api/logs/sessions` and `/api/executions/v2/sessions/:id/messages?scope=all`. There is no backend change.

## Concrete repro: `sess-9f40dc55-ae46-405f-8557-b7017084fefc`

| exec_id (last 12) | agent | parent | started_at | ended_at | turn |
|---|---|---|---|---|---|
| `6b0d5cbb6c30` | **root** | — | 2026-05-03 13:05:20 | **2026-05-05 13:15:02** | spans both |
| `6725ed1adfc1` | builder-agent | root | 2026-05-03 13:05:49 | 2026-05-03 13:08:39 | turn 1 |
| `34a19a7ce5aa` | research-agent | root | 2026-05-03 13:08:48 | 2026-05-03 13:10:55 | turn 1 |
| `404284b644b6` | writing-agent | root | 2026-05-03 13:11:00 | 2026-05-03 13:12:42 | turn 1 |
| `7358afe7441f` | builder-agent | root | **2026-05-05 13:11:35** | **2026-05-05 13:14:58** | turn 2 |

Root-execution user messages:
- `2026-05-03 13:05:34` — *"help me with this assignment …"*
- `2026-05-05 13:11:30` — *"can you make it into a presentation"*

Root-execution final assistant text replies (role=assistant, non-empty `content`):
- `2026-05-03 13:12:50` — *"The assignment is fully complete! …"*
- `2026-05-05 13:14:58` — *"Done! I've created a 22-slide HTML presentation …"*

What the UI shows today: both user bubbles, then four subagent cards in one cluster, then one reply (the latest, since `extractRespondByExecId` is last-wins-per-execution).

What it should show: `user1 → 3 subagents → reply1 → user2 → 1 subagent → reply2`.

## Data model the rollup uses

### Inputs (already provided by today's APIs)

- **`/api/logs/sessions`** — execution table rows. We filter by `conversation_id === sessionId`. Yields the root row (parent empty) and every child row (parent = root.session_id).
- **`/api/executions/v2/sessions/:id/messages?scope=all`** — every message across the whole session. Each carries `execution_id`, `role`, `content`, `created_at`, optional `tool_calls`.

That's it. No new endpoints, no schema migration.

### Derived: `SessionTurn`

A `SessionTurn` is the half-open interval `[user_msg_i.created_at, user_msg_{i+1}.created_at)`. Last turn's right edge is `+∞` while the root execution is running, otherwise `root.ended_at`. Within that window:

```ts
interface SessionTurn {
  /** Stable id, derived from the user-message id. */
  id: string;
  /** Index 0..N-1 in chronological order. */
  index: number;
  /** The user message that opens this turn. */
  userMessage: { id: string; content: string; createdAt: string };
  /** Subagents whose started_at falls in [startedAt, endedAt). */
  subagents: AgentTurn[];
  /** Last assistant text reply in the window — preferred over respond() tool call,
   *  but falls back to it if the model only emitted a respond() call. Null while
   *  the turn is still streaming. */
  assistantText: string | null;
  /** Streaming buffer for the in-flight reply, if any. Promoted to assistantText
   *  on turn end, mirroring how today's reducer handles the single-turn case. */
  assistantStreaming: string;
  /** Per-turn timeline (root-execution thinking / tool_call / system events
   *  in this window). Drives the live ticker pill while running. */
  timeline: TurnTimelineEntry[];
  /** Per-turn status. */
  status: "running" | "completed" | "stopped" | "error";
  /** ISO timestamp of the user message. */
  startedAt: string;
  /** Right edge: next user message minus 1 ms, or session ended_at, or null. */
  endedAt: string | null;
  /** End - start in ms (null while running). */
  durationMs: number | null;
}
```

`AgentTurn` (the subagent execution wrapper) keeps its current shape — it represents a single execution row, not a turn. Confusing name but we leave it alone to keep the diff scoped.

### Snapshot shape change

Today:
```ts
ResearchSnapshot {
  messages: ResearchMessage[]    // user bubbles, flat
  turns: AgentTurn[]             // root + flat children
  …
}
```

After:
```ts
ResearchSnapshot {
  turns: SessionTurn[]           // chronological, each contains its own user msg + subagents + reply
  …
}
```

Session-level fields stay where they are: `intentAnalysis`, `ward`, `artifacts`, `status`, `conversationId`. Those are session-scoped, not turn-scoped.

## The rollup algorithm

```
inputs:
  rootRow                    (one row from /logs/sessions, parent_session_id empty)
  childRows[]                (rows whose parent_session_id == rootRow.session_id)
  messages[]                 (everything from /messages)

steps:
  1. rootMessages = messages.filter(m => m.execution_id == rootRow.session_id)
                            .sort_by(created_at)

  2. boundaries = []
     for m in rootMessages where m.role == "user":
         boundaries.push({ userMessage: m, startedAt: m.created_at, endedAt: null })

     // Right-edge of each boundary is the next boundary's startedAt;
     // the last one stays null while running, else root.ended_at.
     for i in 0..boundaries.len() - 1:
         boundaries[i].endedAt = boundaries[i+1].startedAt
     boundaries.last.endedAt = (root.status == "running") ? null : root.ended_at

  3. turns = boundaries.map((b, i) => SessionTurn {
         id: "turn-" + b.userMessage.id,
         index: i,
         userMessage: b.userMessage,
         subagents: [],
         assistantText: null,
         assistantStreaming: "",
         timeline: [],
         status: derived (see step 6),
         startedAt: b.startedAt,
         endedAt: b.endedAt,
         durationMs: (b.endedAt) ? endedAt - startedAt : null,
     })

  4. for child in childRows.sorted_by(started_at):
         turn = turns.find(t => t.startedAt <= child.started_at < (t.endedAt ?? +Inf))
         if turn:
             turn.subagents.push(turnFromLogRow(child, root.session_id))

  5. for turn in turns:
         windowMessages = rootMessages.filter(m =>
                              turn.startedAt <= m.created_at < (turn.endedAt ?? +Inf))
         turn.assistantText = extractAssistantReplyForTurn(windowMessages)

  6. for turn in turns:
         turn.status = derive_status(turn, root.status, root.ended_at)
              // "completed" if assistantText set and no running subagent
              // "running"   if root.status == "running" and turn is the latest open one
              // "error"/"stopped" propagated from root.status
```

### `extractAssistantReplyForTurn` — exactly what we mean by "the reply"

```
preferred = last message in window where:
    role == "assistant" AND
    content is non-empty AND
    content != "[tool calls]" placeholder

fallback = last respond() tool call in the window (any assistant message whose
           tool_calls includes { tool_name: "respond", args: { message: <str> } })

return preferred ?? fallback
```

The "preferred over `respond()`" choice continues the policy we shipped in #108: plain text wins over tool-call wrapping when both exist, and the absence of a `respond()` call is no longer a dead end.

### Edge cases the algorithm handles

- **Subagent started exactly at a turn boundary** (`child.started_at == turn.startedAt`): half-open interval `[startedAt, endedAt)` puts it in the new turn. Symmetric: a subagent that ends exactly at the next user-message timestamp still belongs to the older turn (we bucket on `started_at`, not `ended_at`).
- **Last turn still streaming** (`assistantText` is null, no end edge): `status = "running"`, `durationMs = null`. The reducer keeps appending to the same turn until a `respond` / `turn_complete` event closes it.
- **No user message yet** (fresh session, intent analysis still running): `turns = []`. Page renders empty state.
- **`root.ended_at` predates the latest user message** (data corruption): trust the user message timestamp, don't filter by `endedAt` past the data we see.

## WebSocket reducer changes

Today's reducer maintains a single open root turn and a flat children array. After:

```ts
state: {
  turns: SessionTurn[]
  // No `latestOpenTurnIndex` field — it's always `turns.length - 1` because
  // we only ever open a new turn at the end and never close out-of-order.
}

events:
  user_message       → state.turns.push(newOpenTurn(payload))
  delegation_started → state.turns.last.subagents.push(newSubagent(payload))
  token              → state.turns.last.assistantStreaming += payload.text
                       // OR routed to a subagent if execution_id matches a child
  thinking | tool_call | system_note
                     → append to state.turns.last.timeline
                       // OR to the relevant subagent's timeline, same as today
  respond            → state.turns.last.assistantText = payload.text
                       state.turns.last.assistantStreaming = ""
  turn_complete      → finalize state.turns.last (status=completed, endedAt=now)
```

The "open turn is always the last one" invariant simplifies the bookkeeping. The router that decides "is this event for the root or a subagent?" stays the same — it keys on `execution_id`.

## Render

`ResearchPage.tsx` becomes a thin loop:

```tsx
{snap.turns.map((turn) => (
  <SessionTurnBlock key={turn.id} turn={turn} allSubagents={turn.subagents} />
))}
```

`SessionTurnBlock` (new component) renders one turn:

```
┌─ user bubble ─┐
│ "help me with…" |
└────────────────┘
   ┌─ subagent: builder-agent ─┐
   ┌─ subagent: research-agent ─┐
   ┌─ subagent: writing-agent ─┐
┌─ assistant ─┐
│ "The assignment is fully complete! …" |
└──────────────┘
═══════════════ (turn divider)
┌─ user bubble ─┐
│ "can you make it into a presentation" |
└────────────────┘
   ┌─ subagent: builder-agent ─┐
┌─ assistant ─┐
│ "Done! I've created a 22-slide…" |
└──────────────┘
```

The existing `SubagentCardTree` and `RespondBody` components are reused unchanged — only the parent block is new. Live ticker pill keys to `state.turns.last`.

## What we are NOT doing in this PR

- **No backend changes.** `/state.userMessage` and `/state.response` stay singular (first message, last reply). They're used only by the boot-time intent-chip and the title; not on the turn-rendering path. A separate follow-up can add `state.turns[]` if we want symmetry.
- **No per-turn collapse/expand affordances.** Visual polish for a follow-up; out of scope.
- **No multi-root-execution changes.** A continuation still reuses the root execution row. The "turn" concept is a UI grouping, derived from message timestamps. We don't introduce per-turn execution rows.
- **No backwards-compat shim.** The old flat shape goes away in this commit. `ResearchSnapshot.messages` is removed; consumers move to `turns[i].userMessage`.
- **No theme work.** The turn divider is a thin `border-top` between blocks — minimum visual marker, no fancy treatment.

## Verification

1. `cd apps/ui && npm run build` — clean.
2. `cd apps/ui && npm test` — all green; new tests in `turns.test.ts`, `reducer.test.ts`, `ResearchPage.test.tsx`.
3. Live: navigate to `http://localhost:3000/research/sess-9f40dc55-ae46-405f-8557-b7017084fefc` (or the equivalent on the Pi). Confirm the order:
   `user1 → builder → research → writing → reply1 → DIVIDER → user2 → builder → reply2`.
4. Live regression: open any single-turn session — render is identical to today (one user bubble, one set of subagents, one reply, no divider).

## Risks

- **Reducer invariants subtler than they look.** Multi-turn means events from a still-running subagent-of-turn-1 could arrive after turn-2 has opened. Solution: route by `execution_id` (we already do this), not by "which turn is open." Test this with an out-of-order WS sequence.
- **Old test files mock the flat snapshot shape.** Many tests will need updating in lock-step with the new types. Plan accordingly — Task 7 is half the work of this PR.
- **`extractRespondByExecId` is removed.** Anything that imported it (the previous research-v2 fix added one) needs to migrate to `extractAssistantReplyForTurn`. Grep before deleting.
