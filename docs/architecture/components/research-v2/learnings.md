# Research v2 — Learnings

Hard-earned during the `/research-v2` build. Every item was an observed bug, not a speculative concern. Read these before modifying the hook, event-map, or WS subscription code.

Builds on `components/chat-v2/learnings.md` (which covered server-owned session identity, self-healing init, deterministic pill, wire-format drift, stable deps, StrictMode bootstrap, history ≠ reducer stream). Those rules still apply.

## 1. Respond tool message lives in `toolCalls`, not `message`

**Bug:** Assistant rows returned by `/api/sessions/:id/messages` with `content === "[tool calls]"` look like empty placeholders. The real final answer lives in the parallel `toolCalls` (camelCase) or `tool_calls` (snake) column — a JSON string whose entries have `tool_name: "respond"` and `args.message`.

**Rule:** When hydrating history, parse every `[tool calls]` assistant row's `toolCalls` column. If a respond entry exists, substitute `args.message` for the placeholder. Also extract `args.artifacts` while you're there.

**Where enforced:** `session-snapshot.ts:extractRespondMessage`, per-execution-id grouping.

## 2. `turn_complete.final_message` is empty for respond-tool responses

**Bug:** The gateway's WS handler translates `GatewayEvent::Respond` → `ServerMessage::TurnComplete { final_message: Some(message) }`. Expected this to carry the respond tool's message. It doesn't — `turn_complete.final_message` is populated only from streamed tokens (via `StreamEvent::Done`), never from the respond tool action. The respond's `args.message` is only available as a `tool_call` event for `tool_name === "respond"`.

**Rule:** Synthesize a RESPOND action from any `tool_call` event where `tool_name === "respond"` and `args.message` is a non-empty string. See `useResearchSession.ts:respondActionFromToolCall`.

## 3. Delegation events don't carry a top-level conversation_id

**Bug:** `GatewayEvent::DelegationStarted` / `DelegationCompleted` have `parent_conversation_id` and `child_conversation_id` but NO plain `conversation_id`. The client-side transport routes by `conversation_id` first, falls back to `session_id`. A subscription keyed only on the client-minted conv_id ("research-xxx") NEVER receives these events because they don't match the conv_id and the fallback session_id key wasn't registered.

**Rule:** Subscribe on BOTH the client-minted conv_id AND the server-assigned session_id. Transport seq-based dedup handles overlap. Pattern shipped as R14g.

## 4. Session-scope subscription filters out child execution events

**Bug:** Set up the session subscription with `scope: "session"` expecting all per-session events. The server-side filter (`gateway/src/websocket/subscriptions.rs:should_send_to_scope`) passes only root-execution events + delegation lifecycle. Child subagents' `thinking` / `tool_call` / `tool_result` events all get filtered at the server. The top pill goes silent after delegation starts.

**Rule:** Use `scope: "all"` for the session-id subscription. Routing is still by session_id so you don't get cross-session noise, but the scope filter becomes pass-through. R14j.

## 5. Subscription ack races earlier events

**Bug:** `subscribeConversation` returns immediately, but the server-side ack (reported by `console.log("Subscribed to ... at seq N")`) arrives at some seq > 0. Any scope-filterable events that fired between our `invoke` and the ack are dropped forever — scope-subscribed events aren't replayed on ack.

**Rule:** Immediately after `subscribeConversation` returns, fire a one-shot `hydrateFromSnapshot()` to pull turns + title + artifacts from REST. Reducer idempotency means duplicate state with events that *do* arrive later is harmless. R14g catch-up.

## 6. Ping timeout reconnect loses `invoke_accepted` forever

**Bug:** Default PONG_TIMEOUT is 30s. A long-running send can trigger reconnect. `invoke_accepted` is sent directly to the WS client (not replayed on reconnect). `state.sessionId` stays null forever → R14g can't subscribe → UI stuck.

**Rule:** On WS `connected` transition (reconnect), if `state.status === "running"` && `state.sessionId === null` && a recent sendMessage timestamp is cached, match against `/api/logs/sessions` for a running row started within ±15s and bind its session_id. R14h.

## 7. Snapshot-on-open + subscribe-only-while-running, no polling

**Original design:** 5s artifact poll + hydrate-messages-only + WS subscribe. Polling was a hack for the "title not updating" bug. Polling felt wrong; the REST endpoints ARE the source of truth.

**Rule:** On session open, one REST fan-out (logs + messages + artifacts) rebuilds the full state. Subscribe only while `status === "running"`. Re-snapshot on root `agent_completed` to backfill anything dropped. No timers. R14f.

Exceptions: event-driven reconcile (R14i) fires a debounced 800ms re-snapshot on `delegate_to_agent` tool_call or `delegation_started/_completed` markers — those signals are almost always reliable (tool_call comes via conv-id subscription) and fill gaps when delegation happened during the subscription ack race.

## 8. LogSession.conversation_id is the sess-*, session_id is the exec-*

**Bug:** Naming is inverted from intuition.

- `LogSession.session_id` → execution id (exec-*)
- `LogSession.conversation_id` → real session id (sess-*)
- `LogSession.parent_session_id` → parent execution id on children (empty/null on root)

**Rule:** When mapping `/api/logs/sessions` rows, always use `row.conversation_id` as the "session id" that callers know. Document the quirk in every function that touches this shape. `rowToSummary`, `session-snapshot.ts:buildTurns`, the R14h recovery matcher.

## 9. Subagents run on their own conversation id

**Bug:** Tried to match delegation events by conv_id. Child subagents have their own conv_id ("research-xxx-sub-yyy") that's passed back in the delegate tool_result. Our root subscription doesn't see it.

**Rule:** Subscribe at session_id scope (R14g, scope="all" per R14j). All executions share the session_id, so that's the one routing key that catches every agent's events regardless of who they belong to.

## 10. Subagent messages include system-injected user rows

**Bug:** Subagent executions' message history contains `role: "user"` rows that are NOT real user prompts — they're system-injected context (`<ward_snapshot>` blocks, delegation preambles). Rendering them all as user bubbles produced 5 extra bubbles for a session with one real prompt.

**Rule:** On hydrate, filter user-role messages to: (a) `execution_id === rootRow.session_id`, AND (b) content does NOT start with a known system marker (`<ward_snapshot`, `[Delegation `). `session-snapshot.ts:isRealUserPrompt`.

## 11. Turn.timeline is the per-agent ticker data

**Pattern — not a bug:** Every execution's `thinking` / `tool_call` / `tool_result` event gets appended to `turn.timeline[]` keyed by the event's `execution_id`. Works for both root and subagents. The minimalist redesign stopped rendering timeline entries inline but the reducer kept populating them.

**Rule:** If you need "what is this agent doing right now" context per card (LiveTicker, R14j), read `turn.timeline[turn.timeline.length - 1]`. Format based on `entry.kind`. Hide when `turn.status !== "running"`.

## 12. Collapse-on-completion breaks assertions that rely on always-visible content

**Bug (only in tests):** Subagent card auto-collapses on completion (UX: clean reference shape). Tests written against the always-expanded layout broke when the fix landed.

**Rule:** Tests verifying completed-card inner content must click to expand first:
```tsx
fireEvent.click(screen.getByRole("button", { name: /expand subagent/i }));
```

## 13. React controlled inputs + chrome-devtools `fill`

**Bug (only in manual browser smoke):** chrome-devtools `fill` sets DOM value and fires `input`. React's controlled `<textarea>` doesn't always pick up the value → Send button stays disabled even though the DOM shows the typed text.

**Workaround in the driver script:** use `Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set.call(ta, text)` + dispatch `new Event("input", {bubbles: true})`. The native setter + bubbled event is what React's ReactDOMInput listener hooks into.

Not a real-user bug.

## 14. Test harness must match the Transport interface precisely

**Bug:** Adding a transport method (e.g. `listLogSessions` in R14h, `deleteSession` in R19, `onConnectionStateChange` in R14h) crashes `useResearchSession` tests with `transport.X is not a function`. Mocks drift from the Transport interface over time.

**Rule:** When adding a Transport method, also add a stub in `useResearchSession.test.ts`'s `vi.mock("@/services/transport", ...)` factory. CI catches this; local dev doesn't. The integration `__tests__/transport-mock.ts` centralises this — new methods go there, all tests import the factory.

## 15. Reducer idempotency is the safety net

**Observation:** Dual subscription (conv_id + session_id), snapshot + WS events racing, reconnect recovery re-dispatching SESSION_BOUND — all would cause state corruption without strict reducer idempotency.

**Rule — already enforced but call it out for future changes:**
- `AGENT_STARTED` must no-op when the turn id exists (via `ensureTurn`).
- `SESSION_BOUND` with `sessionId: null` must NOT clobber an existing non-null sessionId.
- `RESPOND` is last-writer-wins; safe to re-dispatch with the same text.
- `HYDRATE` overwrites — only dispatched from snapshot paths, never interleaved with a live stream.
- `TITLE_CHANGED`, `WARD_CHANGED`: overwriting fields — safe on repeat.

Any new action that doesn't fit this contract risks dual-write bugs.
