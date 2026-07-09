# Research v2 — Backlog

Known gaps and UX items deferred from the R1–R20 ship.

## High priority

### B1. LiveTicker is correct but underwhelming in practice

**Observed:** User reports "that is not how I am seeing it." The inline ticker code ships correctly (see `AgentTurnBlock.tsx:LiveTicker`) but the tickers only render while `status === "running"`, AND subagents in most test sessions complete in < 1 second. By the time the DOM has a chance to paint, the ticker's already gone.

**Options to explore:**
- Keep the last ticker phrase visible for ~2 seconds after completion (fade out). Would need a separate "last-displayed" ref plus a timer.
- Persist the ticker as a read-only last-entry line in collapsed state (shows "last: read_file plan.md" under the agent name). Trades visual density for signal.
- Render the FULL timeline (all entries, not just the latest) inside the expanded card as an optional "Activity" section. Users opting in by expanding the card get depth; default collapsed state stays clean.

**Data path is intact** — `turn.timeline` has all entries.

### B2. `memory_facts_index` missing causes silent recall degradation — RESOLVED

Every `memory.recall` tool call used to fail with `no such table: memory_facts_index`. Agents continued but without the recall context. Tool error surfaced once per call in the StatusPill (R14e behaved correctly).

Fixed; defect note retired. Kept here only for traceability.

### B3. E2E mock-LLM test harness

**Cross-reference:** `docs/superpowers/specs/2026-04-20-e2e-mock-llm-harness-requirements.md`.

17 scripted scenarios. Every R14 fix (ping timeout, subscription race, dropped title, second-tab live) becomes a failing-then-green scenario so regressions fail CI before manual QA. Deferred while we finish other plan work.

## Medium priority

### B4. Title never changes when agent doesn't call `set_session_title`

**Symptom:** Simple prompts (e.g. "what time is it?") skip the title-generation tool. Header shows a derived fallback ("New research · HH:MM" from the drawer; the first-user-message truncation from `deriveTitle` in the header).

**Not a bug per se** — agent behaviour. Could add a UI-side "if no title after 5s, derive from first user message and display" but `deriveTitle` already does this. The real concern is the sessions drawer showing "New research · HH:MM" for dozens of rows that all ran the same type of simple query.

**Fix candidates:** backend could synthesize a title from the first user message on session close if none was set.

### B5. Ward chip link only works on root's ward

**Symptom:** When root enters a ward (`ward` tool → `__ward_changed__` tool_result), state.wardId gets set and the chip appears. If a subagent changes ward for its own execution, we don't track it.

**Not a bug today** — subagents inherit root's ward by convention. Flag for if agent behaviour changes.

### B6. Delete button doesn't handle child-session rows in DB

**Symptom:** `DELETE /api/sessions/:id` cascades per-session rows but the `parent_session_id` column on `agent_executions` / `execution_logs` isn't separately cascaded. When a root session is deleted, its child execution rows remain pointed at a now-dangling parent.

**Impact:** Today the sessions drawer filters out rows with `parent_session_id` set, so these dangling children are invisible. But `/api/logs/sessions` still returns them — future features that enumerate without the filter will see phantoms.

**Fix:** extend `delete_session_cascade` to also delete rows whose `parent_session_id` matches the target. Needs a second pass or `WHERE session_id = ? OR parent_session_id = ?`. Schema-check before writing.

## Low priority / polish

### B9. Persist per-subagent expansion state across reloads

Currently expansion is `useState` inside each card — reloading a session resets all cards to collapsed. If a user had a specific subagent expanded when they left, restore it on return. Requires URL-state or localStorage.

### B10. Artifact strip doesn't deduplicate between snapshot refs and respond `args.artifacts`

**Symptom:** Hydrating a session that had an artifact + a respond tool call referring to the same artifact: the strip can show the same artifact twice.

**Where to fix:** `session-snapshot.ts:buildArtifacts` — dedupe by artifact id OR by filename+path tuple.

### B11. Subagent depth limit

Tree model is explicitly 2 levels (root + direct children). If the backend ever starts spawning grandchildren, `childrenOf` + `SubagentCardTree` already recurse, but:
- Request-task zip in snapshot is only for root's direct children — grandchildren would have `request: null`.
- UI indentation may get cramped.

**Current policy:** user explicitly said "if subagents spawn subagents I'll use A2C/A2A" — out of scope.
