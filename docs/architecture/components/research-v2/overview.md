# Research v2 (`/research`)

Multi-session research surface served at `/research` (the live route; `/research-v2` is a legacy bookmark redirect). One session per user request. Renders root agent + subagent delegations + live streaming + artifacts. Replaced the old `/` MissionControl page; legacy MissionControl still reachable via `/mission-control`.

## Purpose

- Long-running research tasks that spawn subagents (planner, builder, writer, research-agent).
- One row per session in a drawer, browsable history, per-row delete.
- Live streaming of tool activity (top StatusPill + per-turn inline LiveTicker).
- Hydrate any completed session from REST; re-attach live streaming to running sessions via dual WS subscription.

## When it runs vs `/chat`

| Axis | `/chat` (Quick Chat) | `/research` (Research) |
|---|---|---|
| Session model | One reserved session, persistent | One per user prompt, new on send |
| Mode flag | `mode="fast"` (SessionMode::Chat) | default (SessionMode::Research) |
| Subagents | Typically none | Usually delegates (planner → builder → writer) |
| Ward | No | Agent can call `ward` tool → sets sticky ward |
| Intent analysis | Skipped | Fires before execution |
| Title | No title field | `set_session_title` tool fires early; header + browser tab flip |
| Artifacts | Rare | HTML reports, markdown, JSON data |
| Layout | Linear message list | Root block with nested subagent cards |

## Location

```
apps/ui/src/features/research-v2/
├── ResearchPage.tsx           page — header, drawer, pill strip, body, artifact strip, composer
├── useResearchSession.ts      state hook — snapshot + dual WS subscribe + reconnect recovery + reconcile
├── reducer.ts                 pure reducer — 22 action variants, idempotent, per-case helpers
├── event-map.ts               gateway event → ResearchAction + pill mapper (two separate functions)
├── session-snapshot.ts        REST fan-out → full ResearchSnapshot (replaces the old hydrate-only flow)
├── types.ts                   ResearchSessionState, AgentTurn, TimelineEntry, ResearchMessage, …
├── turn-tree.ts               pure helpers — rootTurns, childrenOf (flat 2-level model)
├── AgentTurnBlock.tsx         renders root block + nested subagent cards + LiveTicker
├── ResearchMessages.tsx       UserMessage, AssistantMessage (hydrated history), AgentAvatar, CopyButton
├── ArtifactStrip.tsx          live chips above the composer
├── SessionsList.tsx           drawer row renderer (presentation; hover-Trash button)
├── SessionsDrawer.tsx         drawer shell (Esc-to-close, backdrop click)
├── useSessionsList.ts         sessions list hook + delete orchestrator
├── ThinkingTimeline.tsx       (reserved; not rendered in the current minimalist layout)
├── artifact-poll.ts           pure helpers (toArtifactRef, fetchArtifactsOnce); timer removed in R14f
├── research.css               scoped styles; all theme-token driven
└── __tests__/
    ├── transport-mock.ts      reusable Transport mock + `ev` event-stream DSL
    └── flows.integration.test.tsx  happy + 14 negative scenarios
```

Shared with chat-v2:
- `apps/ui/src/features/shared/statusPill/` — the top StatusPill, `tool-phrase.ts`, aggregator
- `apps/ui/src/features/chat/ArtifactSlideOut.tsx` — the preview panel for artifact chips

## Backend endpoints it uses

| Method | Endpoint | Role |
|---|---|---|
| `GET` | `/api/logs/sessions?limit=N` | Sessions drawer + snapshot (wire quirk: `conversation_id` is the real sess-* id, `session_id` is the execution id) |
| `GET` | `/api/sessions/:id/messages?scope=all` | Hydrate history; `toolCalls` column carries respond tool's `args.message` |
| `GET` | `/api/sessions/:id/artifacts` | Artifact chips on snapshot + post-completion refresh |
| `GET` | `/api/artifacts/:id/content` | Slide-out preview |
| `POST` | `/api/wards/:id/open` | Ward chip button — launches OS file browser (xdg-open / open / explorer.exe) |
| `DELETE` | `/api/sessions/:id` | Per-row drawer Delete. Memory-preserving cascade (R18). |
| WS | `invoke` | `agent_id="root"`, no mode flag. Client mints its own `research-{uuid}` conversation_id (R14a). |
| WS | `subscribe` conv_id (scope="all" default) | Conv-id-routed events (invoke_accepted, tokens, some tool_calls) |
| WS | `subscribe` session_id (scope="all", R14j) | Session-routed events (delegation_started/_completed, session_title_changed, subagent lifecycle). BOTH subscriptions run concurrently while status === "running"; transport dedupes by seq. |

Intentionally NOT used:
- `/api/sessions/:id/state` — returns 404 for extant sessions (known gateway bug; sidestepped by the 3-way snapshot REST fan-out).

## Session lifecycle — two entry points

**A. New session (`sendMessage` from a fresh `/research`)**:

```
sendMessage(text)
 → APPEND_USER dispatch (status=running, user bubble renders)
 → mint convId = "research-{uuid}"
 → ensureSubscription(convId, scope=all)   ← R14a pre-invoke subscribe
 → dispatch SESSION_BOUND(convId, sessionId=null)
 → executeAgent(root, convId, text)
 → server emits invoke_accepted with server sessionId
 → SESSION_BOUND re-dispatches with real sessionId
 → R14g effect fires:
     subscribe(sessionId, scope=all)        ← second subscription
     hydrateFromSnapshot(sessionId)          ← R14g catch-up for events before sub ack
 → live WS stream (token, thinking, tool_call, delegation_started/_completed,
     respond, agent_completed…) feeds reducer + pill + inline tickers
 → on delegate_to_agent tool_call → onReconcileHint → debounced re-snapshot in 800ms
 → on root agent_completed → re-snapshot (R14f) to pull final title + artifacts
```

**B. Opening a session via URL `/research/:sessionId`** (second tab, reload, drawer navigation):

```
urlSessionId from useParams
 → hydrateFromSnapshot(urlSessionId)
     ↳ parallel: GET /api/logs/sessions, GET /messages?scope=all, GET /artifacts
     ↳ builds full state.turns (root + children), title, artifacts, respond per turn
 → HYDRATE dispatch
 → if snapshot.status === "running":
     R14g effect fires: subscribe(sessionId, scope=all) + catch-up snapshot
   else:
     no subscription (session is done; snapshot is truth)
```

## State shape invariants

- `turns[]` is flat. Children link to parent via `parentExecutionId`. Tree is exactly 2 levels deep (root + direct children). If a subagent ever spawns its own subagent, the rendering recurses through `childrenOf()` but the data model stays flat.
- **Sticky ward**: `WARD_CHANGED` is the only writer of `state.wardId` / `state.wardName`. `AGENT_STARTED` inherits the current sticky ward onto its new turn; null ward on the event does NOT clear state ward.
- **Idempotent reducer actions**: `AGENT_STARTED` no-ops when the turn id already exists. `RESPOND` is last-writer-wins. This is what makes snapshot + live WS dual writes safe.
- **`request` on AgentTurn**: populated only via `delegation_started.task`. Null for root turns.
- **Silent-crash inference**: `AGENT_COMPLETED` on a turn with no meaningful content (no respond, no streaming, no tool_call/tool_result entries) flips status to `"error"` + sets `errorMessage` (workaround for the gateway not emitting a proper error event on silent failures).

## Top StatusPill vs per-turn LiveTicker

Both coexist by design.

- **Top StatusPill** (global): shared with chat-v2. Deterministic. Last-wins across ALL agents on the session. Shows current tool phrase / agent narration / error. Glanceable.
- **LiveTicker** (per-turn, R14j): inline in each SubagentCard header + root avatar row. Reads `turn.timeline[-1]` keyed by execution_id. Shows only while `turn.status === "running"`. Survives auto-collapse, keeps context when a different agent takes over the global pill.

Both drive off the same WS event stream. The pill maps through `mapGatewayEventToPillEvent`; the ticker reads directly from reducer state.

## Rendering model

- **Root turn**: no bordered card; renders as `research-msg--assistant` (avatar top-left, inline LiveTicker, body with nested subagent cards, final respond markdown, copy button).
- **Subagent card**: bordered box with left accent stripe (agent-colour). Header is `[chevron][agent-name][LiveTicker][status-icon duration]`. Expanded body shows `Request:` (delegation task) + `Response:` (respond or "waiting…"). Auto-collapses on completion; click to re-expand.
- **Hydrated history** (no live stream): assistant messages render through `AssistantMessage` / `UserMessage` (not turn blocks). Respond is extracted from `toolCalls.args.message` if the agent called the `respond` tool; falls back to last assistant row per user turn.

## Known edge cases (handled)

- WS ping timeout (30s no-pong) + reconnect during a live run — `invoke_accepted` gets lost. R14h binds `state.sessionId` by matching `/api/logs/sessions` by send timestamp.
- Session-scope subscription acks at seq N > 1 — events 2..N-1 that are session-scoped get dropped. R14g catch-up snapshot fires immediately after the subscribe call to backfill.
- `delegation_started` filtered server-side under scope="session" (filter passes only root-execution events) — R14j switched to scope="all" so child `tool_call` / `thinking` reach us.
- Delegation started/completed landing after our subscription ack but before a snapshot refresh — R14i debounces a reconcile on `delegate_to_agent` tool_call + `delegation_started` + `delegation_completed` markers.
- System-injected user-role rows (ward_snapshot, delegation preamble) appearing in subagent executions' message history — filtered in `session-snapshot.ts:isRealUserPrompt` by root-execution_id + known-prefix check.
