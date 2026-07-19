# Chat v2 — Backlog

Items not fixed in the initial Quick Chat ship but known gaps.

## High priority

### B1. Artifact auto-registration for fast-mode writes

**Symptom:** User prompts "Generate a markdown in scratch ward with today's time". Agent calls `write_file`, file lands on disk (`~/Documents/zbot/wards/scratch/notes-2026-04-19.md`), agent reports success. **But no row in the `artifacts` table** → UI's artifact strip stays empty.

**Root cause:** `gateway/gateway-execution/src/artifacts.rs::process_artifact_declarations()` only fires from `ArtifactDeclaration`s attached to the respond() action. In `fast`/`SessionMode::Chat` the agent's prompt doesn't coach it to emit respond-side artifact declarations, and `write_file` itself has no side-effect that inserts a row.

**Fixes, in preference order:**

1. **Writer-side hook inside the `write_file` tool.** When executing in a ward context, synthesise an ArtifactDeclaration as a side-effect and persist via `process_artifact_declarations`. Pros: fast-mode friendly, one implementation covers every agent using `write_file`. Cons: needs careful ward-resolution plumbing in the tool.

2. **Extend the quick-chat / root system prompt** to instruct the agent to return `artifacts: [{ path, label }]` in its respond() when it wrote a file. Brittle — prompt engineering.

3. **Post-turn disk scan** against the ward's directory, compared to a pre-turn snapshot. Expensive and fragile.

**Recommended:** Option 1. Scope: roughly one file change in `runtime/agent-tools/src/tools/write_file.rs` (or wherever `write_file` lives), one in `artifacts.rs` to expose a synchronous artifact-insert entry point, maybe one test.

**UI is already ready.** When the backend registers, the existing artifact strip + slide-out light up automatically. `b317cc7`.

### B2. Context compaction for the reserved chat session

**Symptom:** Heavy use of `/chat-v2` (or `/chat`) causes the reserved session's history to exceed the model's context window. Nemotron crashed with `prompt is too long: 440905, model maximum context length: 262144`. The UI shows no reply — the execution goes `crashed` silently.

**Workaround shipped:** Clear button (trash icon, top-right) → `DELETE /api/chat/session` → fresh session. User-triggered, not automatic.

**Fix:** server-side compaction in the executor's message builder.
- Before each LLM call, estimate prompt token count.
- If > 80% of model window: summarise the oldest N turns into a system note, drop the raw messages, prepend the note.
- Summarisation uses a cheap provider (nemotron / ollama local).
- Persist the synthesis as a row in `messages` table with a `compacted_from_ids` column so the original turns can still be recovered from the DB even though the LLM no longer sees them verbatim.

**Where it lives:** probably `gateway/gateway-execution/src/{prompt,message_builder}.rs`. Scope: significant — half-day to one day. Touches tests.

### B3. Surface silent crashes to the UI

**Symptom:** LLM provider 500s, context-blown turns, etc. end up in `agent_executions.error` but **no `error` event is emitted** to the WebSocket. The UI shows `turn_complete` with empty `final_message` and nothing else. Users see a composer that keeps spinning or just stays idle after a send.

**Fix:** in the executor's error path, emit a `GatewayEvent::Error { session_id, execution_id, message }` alongside (or instead of) the empty `turn_complete`. The UI already has an `error` case in the event mapper that dispatches `ERROR` action → `status="error"`. The pill should show a red/destructive variant.

**Scope:** one-to-two call sites in `gateway/gateway-execution/src/runner.rs` (or wherever the crash-catch is). Plus one UI tweak to visibly render the error state (currently `status="error"` just disables the composer).

### B4. Status-pill error category / destructive-colour variant

Ties into B3. When the turn fails, the pill should briefly show `"Turn failed"` on a destructive-coloured surface before fading. Requires:
- `PillCategory` extended with `"error"`.
- One more branch in `reducePillState` driven by the `ERROR` action.
- CSS accent token: `var(--destructive)`.

Trivial after B3 lands.

## Medium priority

### B5. Multi-tab synchronisation

Two `/chat-v2` tabs open against the same reserved session: Tab A's user send does NOT appear in Tab B (Tab B has no optimistic APPEND_USER for it). Both tabs see the same assistant stream once the backend broadcasts.

**Fix:** handle `MessageAdded` events in the event mapper. Dispatch an action that appends the message iff it's not already in the list by id.

**Scope:** small — one case in the mapper, one action variant in the reducer.

### B6. History pagination UI

`getSessionMessages` now takes only `scope`. There's no `before=<cursor>&limit=` support, so we display the last 50 turns and that's it. For long-running reserved sessions, users can't scroll back further.

**Fix (backend):** add `before` + `limit` query params.
**Fix (UI):** "↑ Show earlier turns" affordance at the top of the scroll area — the plumbing was removed in favour of YAGNI; restore when B6 is worth shipping.

### B7. Thinking timeline inside the turn block

Currently the pill ignores Thinking events entirely. When a user wants to dig into *why* the agent did what it did, there's nowhere to look (they'd use the Logs page, a context-switch away).

**Option:** add an expandable "thoughts" chevron to the Chat V2 assistant bubble — same idea as the Research UI's per-agent-turn Thinking block. Low priority for Chat V2 (users want quick answers, not introspection) but worth considering.

## Low priority / deferred

### B8. Hard single-delegation enforcement

Currently prompt-level only (the `quick-chat`-agent-template.md said "AT MOST ONE delegate_to_agent per turn"). Since we deleted that agent and route to `root` with `mode=fast`, this constraint lives entirely in the root agent's behaviour in fast mode. No hard enforcement. Accept for now; add a per-turn tool-call counter in the executor if abuse shows up.

### B9. "Copy to clipboard" on artifact cards

Nice-to-have. The ArtifactSlideOut already has download. A compact copy-path button on the card itself would save a click when the user just wants the file path.

### B10. Inkwell / ticker accessibility

Pill has `aria-live="polite"` + `aria-atomic="true"` which re-announces on every swap. With ~100 Thinking events per turn... wait, we already dropped Thinking from the pill in B3-adjacent work. But ToolCall swaps can still fire rapidly. Consider `aria-live="polite"` only on narration changes with a debounce, not on every key bump.

## Retired items (included only for traceability)

- ~~Parallel `/api/quick-chat/init` endpoint~~ — reverted; `/chat-v2` shares `/api/chat/init` with `/chat`.
- ~~`quick-chat` agent template~~ — reverted; uses `root` with `mode=fast`.
- ~~`settings.quick_chat` ChatConfig slot~~ — reverted.
- ~~client-side conversationId generation~~ — replaced with server-owned ids.
- ~~URL-encoded `/chat-v2/:sessionId`~~ — dropped; session is implicit.
- ~~"New chat" button~~ — dropped; replaced with the Clear trash-icon.

---

## B4. E2E test harness with scripted mock gateway — deferred

Full requirements captured in
`docs/superpowers/specs/2026-04-20-e2e-mock-llm-harness-requirements.md`.

Summary: Playwright-driven e2e harness for `/chat-v2` and `/research-v2`
against a scripted mock gateway (HTTP + WS, wire-format fidelity with
`gateway-ws-protocol`). Deterministic scenarios — happy paths,
multi-subagent flows, ping-timeout reconnects, sequence-gap at
subscribe ack, dropped title events, second-tab live. Each bug the
R14 work caught becomes a failing-then-green scenario so regressions
fail CI before manual QA.

Deferred while we finish R16–R20 of the original research-ui plan.
Pick up when we stop shipping UI features weekly and want stable
regression coverage.
