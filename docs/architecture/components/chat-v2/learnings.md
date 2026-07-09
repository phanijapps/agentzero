# Chat v2 — Learnings to apply to Research UI

Hard-earned during the `/chat-v2` build. Every item was a bug we hit, not a speculative concern. Apply to the upcoming Research UI plan *before* writing code.

## 1. Server-owned session identity, not client

**Bug:** First cut generated a client-side UUID for conversationId on every mount. Each send created a new server session. Research sessions list filled with phantom entries.

**Rule:** The backend is the source of truth for which session a UI is bound to. UI calls an idempotent init endpoint at mount and uses whatever ids come back, never generating its own. No localStorage fallback, no URL-encoded session id.

**Apply to Research:** the sessions drawer fetches from `/api/logs/sessions?mode=research` (or equivalent). When the user picks one, the URL updates via `navigate`, and the state hook loads snapshot from the server. The client never invents a session id.

## 2. `init_chat_session`-style endpoints must be self-healing

**Bug:** The reserved session stored in `settings.chat.session_id` can outlive its DB row (disk edits, schema migrations, daemon crashes). The init endpoint was trusting the settings slot and returning a stale id; every invoke then created a new session silently.

**Rule:** Any cache-backed id lookup must verify the backing row exists. When it doesn't, rebuild and overwrite the cache atomically. Errors in the create path must bubble up (no `warn!` on failed persistence).

**Apply to Research:** any "recent session", "active session", "pinned session" pointer that lives in settings needs a freshness check before returning it.

## 3. Deterministic pill, not model-driven narration

**Bug:** First pill design read the `Thinking` event's `content` field to populate the narration. glm-5-turbo emits ~100 per-token thinking deltas per turn — the narration flashed unreadably. Nemotron emits zero — the narration stayed empty, pill looked lopsided.

**Rule:** UI progress indicators are driven by event *kinds*, never by provider-specific content streams. Content stream → bubble. Event kind → pill.

**Apply to Research:** the per-agent-turn block should still show Thinking content inside an expandable chevron (users expect depth there), but any top-of-page progress indicator must be category-based (`planning → ▶ Planning`, `delegating → ▶ Delegating to writer-agent`, etc.).

## 4. Wire-format field names drift from docs

**Bugs we hit:**
- `Token.delta` (not `content`)
- `Respond.message` (not `content`)
- `ToolCall.tool_name` (not `tool`)
- `WardChanged.ward_id` (flat string, not `{ward:{name}}`)
- `AgentCompleted` has no `last` or `is_final` field
- Backend emits `invoke_accepted`, not `session_initialized`

**Rule:** Before writing the event mapper, grep `gateway/gateway-events/src/lib.rs` for the exact struct shape of each event you plan to consume. Write a snapshot test that exercises the real shape. Don't trust docs or prior code.

**Apply to Research:** the research event-map will consume more events (`PlanUpdate`, `DelegationStarted/Completed`, `SessionTitleChanged`, `IntentAnalysisStarted/Complete/Skipped`, `SessionContinuationReady`) — verify every field name against the Rust source before mapping.

## 5. Hook effect deps — stable identities or you get a teardown storm

**Bug:** `useStatusPill` returned a fresh `sink` object every render. A subscribe-effect that had `pillSink` in its deps tore down + re-subscribed on every render → WS events were lost in the gaps.

**Rule:** Any value a `useEffect` includes in deps must be stable across renders. `useMemo`, `useCallback`, or `useRef` the object at the source. If the effect doesn't *actually* need to re-run when the value changes, drop it from deps with an eslint-disable and a comment explaining why (the closure captures the latest value via re-render).

**Apply to Research:** every cross-hook dependency (pill sink, sessions-list refresh callback, artifact fetcher) gets a stable identity at the producer. Subscribe effects use `[state.conversationId]` only, with explicit disable-eslint and a reason.

## 6. StrictMode-safe bootstrap: set the "done" flag AFTER, not before

**Bug:** React 18 StrictMode double-mounts components in dev. First mount starts async bootstrap, sets `ref.current = true`, gets unmounted. Second mount checks `ref.current`, skips bootstrap. First mount's async completes and dispatches HYDRATE to a torn-down component — result: state never hydrates.

**Rule:** When using a ref to guarantee "this runs once", set the flag AFTER the async work resolves, inside the then-block, right before the state dispatch. Two concurrent bootstraps in dev are harmless if the server side is idempotent; a ref set post-completion prevents duplicate dispatches without creating the dev-mode soft-hang.

**Apply to Research:** session hydration, sessions list initial fetch, any one-shot init.

## 7. History is NOT the reducer's stream

**Bug:** Fetched the backend's root-scoped message list and rendered it directly. Got `[tool calls]` placeholder bubbles and raw tool-result JSON as separate assistant messages.

**Rule:** The message list returned by `/api/.../messages` is the internal conversation format (user + assistant + tool + placeholder + delegation rows). The chat UI only renders a filtered subset. Write a `isVisibleChatMessage` filter at the hook boundary.

**Apply to Research:** the per-agent-turn block view is more tolerant (it can collapse tool rows into the Thinking timeline), but the follow-up user input field should still respect the same filter when surfacing a "latest exchange".

## 8. Clear path for recoverable errors

**Bug:** Reserved session hit nemotron's 262k token cap. Agent crashed silently (`turn_complete` with `final_message: ""`, no `error` event). User's sends vanished.

**Rule:** Every long-lived surface needs a user-visible way to recover from a poisoned state. For `/chat-v2` we added the Clear button → DELETE endpoint → re-bootstrap. For a generic surface: a "reset / archive / start over" action that clears the server-side pointer to the stuck state.

**Apply to Research:** already partially here (new session button). But also need: when a session ends in `crashed`, surface the error text in the UI instead of showing an empty reply. Tie `execution.error` into the WS stream or show it from the sessions list row.

## 9. CSS theming — no hardcoded colours in component files

**Bug:** First pill version used `rgb(100, 200, 255)` style literals. Dark-theme flip would be a one-by-one grep-and-replace.

**Rule:** Every colour comes from a CSS variable in `apps/ui/src/styles/theme.css`. If the token doesn't exist yet, add one — don't embed the hex. Surfaces like the terminal row reuse existing tokens (`--sidebar`, `--background`) rather than minting new ones unless semantically distinct.

**Apply to Research:** drawer surface, per-agent-turn block left-accent stripe (category colours already exist), active-session dot in the sessions list — all via tokens.

## 10. E2E: route mocks > live-agent waits

**Rule:** Flow tests (artifact card → slide-out, Clear button → DELETE) should stub the backend so they run in seconds and don't depend on the daemon being up. Reserve live-agent tests for smoke-level "it replies" assertions, and auto-skip those when `/api/health` is unreachable.

**Apply to Research:** sessions-drawer group rendering, new-research flow, artifact open — all route-mockable. Only "agent completes a real turn end-to-end" needs the daemon.

## 11. SonarQube cognitive-complexity — extract per-branch helpers early

**Rule:** If a `switch` or `match` grows past 5 cases, extract per-case helpers before the body itself grows. Kept the event-mapper, reducer, and pill-aggregator all well under the 15 threshold.

**Apply to Research:** the research reducer will have ~14 actions (AGENT_STARTED / AGENT_COMPLETED / THINKING_DELTA / TOOL_CALL / TOOL_RESULT / TOKEN / RESPOND / TOGGLE_THINKING / INTENT_ANALYSIS_* / PLAN_UPDATE / SESSION_COMPLETE / ERROR / RESET). Extract each branch to a named function up front.

## 12. Live verification AFTER tests, not instead of them

**Rule:** `vitest` green + `cargo check` clean is necessary but not sufficient. Every feature needs a live browser sample against a running daemon — Playwright route-mocked tests verify logic, but the actual WS event shape, the actual markdown rendering, the actual theme colours — those need a human or scripted browser check.

**Apply to Research:** plan includes explicit "browser verification" steps per task that fire an actual prompt and inspect the DOM + event capture.
