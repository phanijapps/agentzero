# Chat v2 (`/chat`)

Ephemeral, single-session, memory-aware chat surface served at `/chat` (the live route; `/chat-v2` is a legacy bookmark redirect). Built for the Quick Chat workflow.

## Purpose

- Fast single-turn / few-turn conversations anchored in memory recall.
- One persistent server-owned session (not per-message). Backed by the reserved `settings.chat.{sessionId, conversationId}` slot.
- Uses `mode=fast` (`SessionMode::Chat`) so sends skip intent-analysis / planning / research delegation — they do NOT pollute Research sessions.

## Location

```
apps/ui/src/features/chat-v2/
├── QuickChat.tsx              page component (5 sub-components: MessageRow, AssistantBubble, EmptyState, ArtifactCard, refToArtifact shim)
├── useQuickChat.ts            state hook: bootstrap + subscribe + send + stop + clear
├── reducer.ts                 pure reducer with 10 action variants
├── event-map.ts               maps ConversationEvent → QuickChatAction + → PillEvent
├── types.ts                   QuickChatState, QuickChatMessage, QuickChatArtifactRef
├── InlineActivityChip.tsx     recall / skill / delegate chips shown inside assistant bubble
├── quick-chat.css             scoped styles (theme-aware)
└── index.ts                   barrel → QuickChat only

apps/ui/src/features/shared/statusPill/   (shared with Research UI when it lands)
├── StatusPill.tsx             two-row: header (verb) + terminal ($ command)
├── use-status-pill-aggregator.ts  reducer-backed hook with stable sink ref
├── tool-phrase.ts             tool name → { narration, suffix, category } dictionary
├── types.ts                   PillState, PillCategory
└── status-pill.css            theme tokens, two-row layout, shimmer animation
```

## Backend endpoints it uses

| Method | Endpoint | Notes |
|---|---|---|
| `POST` | `/api/chat/init` | Idempotent. Self-heals orphan slots (`get_session()` check before return). |
| `DELETE` | `/api/chat/session` | Clears `settings.chat`; keeps DB rows. Used by the top-right Clear button. |
| `GET` | `/api/executions/v2/sessions/:id/messages?scope=root` | History tail (last 50). Filtered client-side: drops `role="tool"` rows and `content="[tool calls]"` placeholders. |
| `GET` | `/api/sessions/:id/artifacts` | Refetched on turn completion to surface files the agent wrote. |
| WS | `invoke` | `agent_id="root"`, `mode="fast"`. No session_id leads to new-session creation; we always pass the reserved id. |

## Session lifecycle

```
mount → initChatSession() → HYDRATE (messages + artifacts) → subscribe(conversationId)
                                                                       ↓
 user send → APPEND_USER + executeAgent(root, conv, text, sess, "fast")
                                                                       ↓
             ← WS events → reducer (user-visible bubble stream + chips)
                                                                       ↓
         turn_complete → status=idle → refetch listSessionArtifacts()
                                                                       ↓
   clear (user action) → DELETE /api/chat/session → re-bootstrap → HYDRATE fresh
```

The sessionId never changes once bootstrapped (bar the explicit Clear flow). Tab close + reopen rehydrates through `initChatSession`.

## Status Pill

Two-row indicator. Drives from deterministic events only — Thinking is NOT surfaced on the pill.

```
  ┌────────────────────────────┐
  │ ● Running shell             │   header: verb + pulsing dot (category colour)
  ├────────────────────────────┤
  │ $ ls -la ~                  │   terminal: green $ + monospace command, truncated
  └────────────────────────────┘
```

| Event | Pill state |
|---|---|
| `agent_started` | `narration="Thinking…"`, suffix hidden, neutral |
| `tool_call` | narration from dictionary, suffix = raw arg (no `·` prefix), category drives stripe colour |
| `respond` | `narration="Responding"`, green |
| `agent_completed` | fade out |

All colours come from theme tokens (`--sidebar`, `--background`, `--success`, `--blue`, `--teal`, `--purple`). No hardcoded hex.

## UI invariants

1. **No `New chat` button.** The session is persistent. Reset is via the trash-icon Clear button (top-right), gated by a `window.confirm`.
2. **No empty-ward chip.** The ward chip renders only when `activeWardName` is truthy.
3. **No `/chat-v2/:sessionId` route.** The URL is always `/chat-v2`; the session is implicit / server-owned.
4. **Composer disabled until sessionId resolves.** Bootstrap-in-flight states don't silently drop sends.
5. **Tool-call placeholders and tool-result rows are filtered from history.** The UI shows only user / assistant / respond content, not tool orchestration noise.
6. **`pillSink` is memoised.** Previous bug: re-creating the sink on every render made the subscribe-effect re-fire, which tore down and rebuilt the WS subscription, dropping events.
7. **Bootstrap idempotency ref is set AFTER async completes**, not before, so React StrictMode's synthetic unmount can't leave us in a "started but never completed" state.

## Artifact slide-out

- `QuickChatArtifactRef` (id / fileName / fileType / fileSize / label) tracked in state.
- Refetched on every `status==="idle"` transition.
- `ArtifactCard` → click → `ArtifactSlideOut` (reused from legacy `/chat`).
- Slide-out refetches the content via `transport.getArtifactContentUrl(id)`; the ref shape is a minimum-viable subset of `Artifact`.

## Related

- [learnings.md](learnings.md) — what to carry into the Research UI plan.
- [backlog.md](backlog.md) — pending work (artifact auto-registration, compaction, etc.).
