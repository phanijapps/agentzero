# Reflective Memory — Phase 1: Session Handoff

**Date:** 2026-05-11
**Status:** Approved
**Rating before:** 2/10 · **Rating after:** ~4/10

---

## Problem

Every session starts blind. The agent has no structured record of what happened last time — what was accomplished, what's incomplete, what the user cared about. The `session_summaries` shared KV file exists as a scaffold but has never been wired to anything. The sleep-time Synthesizer produces strategy facts but writes to the DB where the agent can't browse them. The net result: each session begins with noisy fuzzy recall and no thread continuity.

This spec covers Phase 1 only: session handoff. It does not address recall noise, strategy fact visibility, or the knowledge graph.

---

## Approach

**Option C — Inject summary, expose detail on demand.**

The system generates a handoff note on session completion and injects a compact block into the next session's context automatically. The agent gets orientation without any tool calls. Structured pointers in the block let the agent pull full detail on demand using existing tools.

Rejected:
- **Option A** (gateway-transparent only): no dynamic access, agent can't pull detail
- **Option B** (agent-pulls only): requires agent cooperation; the amnesiac can't write its own memory

---

## Section 1 — Data Model

**Storage:** `session_summaries` shared KV file (`agents_data/shared/session_summaries.json`). No new tables, no schema changes.

**Two keys per completed session:**

| Key | Purpose |
|-----|---------|
| `handoff.latest` | Always the most recent handoff. Overwritten on each session completion. Read at session start without knowing the session ID. |
| `handoff.{session_id}` | Archived copy per session. Agent reads this for full historical context. |

**Entry shape:**

```json
{
  "summary": "User explored memory system limitations and rated it 2/10. Wrote a 518-line reflective memory spec covering 5 reflection stages. Left incomplete: implementation plan. User seemed most focused on the session handoff gap as the highest-priority fix.",
  "session_id": "sess-chat-1bbcfebc-58c9-41f8-a3e6-34509d53f648",
  "completed_at": "2026-05-11T22:06:22Z",
  "ward_id": "reflective-memory-spec",
  "intent_key": "ctx.sess-chat-1bbcfebc-58c9-41f8-a3e6-34509d53f648.intent",
  "goal_count": 0,
  "open_task_count": 0,
  "correction_count": 5,
  "turns": 79
}
```

**Staleness threshold:** Handoffs older than 7 days are not injected (stale context is worse than no context). The 7-day window is a constant `HANDOFF_MAX_AGE_DAYS = 7` in the writer.

---

## Section 2 — Handoff Writer

**Location:** `gateway/gateway-execution/src/sleep/handoff_writer.rs`

**Trigger:** When a session transitions to `status='completed'` in `core.rs`. Fired as `tokio::spawn` — non-blocking, does not delay session teardown.

**Trait for LLM calls (mockable in tests):**

```rust
#[async_trait]
pub trait HandoffLlm: Send + Sync {
    async fn summarize(&self, input: &HandoffInput) -> Result<String, String>;
}

pub struct HandoffInput {
    pub messages: Vec<(String, String)>,  // (role, content), last 50 turns
    pub ward_id: String,
}
```

**Writer struct:**

```rust
pub struct HandoffWriter {
    llm: Arc<dyn HandoffLlm>,
    shared_kv_path: PathBuf,  // path to agents_data/shared/
}
```

**`HandoffWriter::write()` steps:**

1. Load last 50 messages from `ConversationRepository` for the session
2. Call `llm.summarize(input)` → 3-5 sentence string
3. Read `goal_count` from `StateService` (count of active goals for the session's ward)
4. Read `correction_count` from `MemoryFactStore::get_facts_by_category(agent_id, "correction", 100)` → `.len()`
5. Set `open_task_count = 0` (Phase 1 does not track in-flight tasks; reserved field for Phase 2)
6. Build the entry struct
7. Load `session_summaries.json`, insert/overwrite `handoff.latest` and `handoff.{session_id}`, save

**Failure handling:** Any error (LLM timeout, file I/O, parse failure) is logged at `warn` level and swallowed. A missing handoff degrades gracefully to current behavior — no panic, no retry.

**LLM prompt:**

```
Summarize this conversation in 3-5 sentences. Cover:
- What was accomplished
- What was left incomplete or in progress
- What the user seemed most focused on or interested in next

Be specific. Do not use filler phrases like "the user and assistant discussed".
Use past tense. Write for an agent reading this at the start of the NEXT session.
```

---

## Section 3 — Context Injection

**Location:** `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`

**Trigger:** At bootstrap time, right before the `memory_recall` injection (line ~276). Read `handoff.latest` from the shared KV file. If found and within the 7-day staleness window, prepend a `## Last Session` block to the injected context string.

**Injected block format:**

```
## Last Session  ({date} · ward: {ward_id} · {turns} turns)
{summary}

Corrections active: {correction_count} · Goals: {goal_count}
Full context: memory(action=get, scope=shared, file=session_summaries, key=handoff.{session_id})
Last intent:  memory(action=get_fact, key={intent_key})
```

**Injection is silent on miss:** No log, no error if `handoff.latest` is absent or stale. The bootstrap continues normally.

**Position:** Before the `memory_recall` block. The agent reads last-session context before noisy recall results, giving it orientation before it processes the rest.

---

## Section 4 — Dynamic Access

No new tools. The agent uses existing tools to fetch detail on demand, using the exact keys provided in the injected block:

| What | Tool call |
|------|-----------|
| Full handoff JSON | `memory(action=get, scope=shared, file=session_summaries, key=handoff.{sid})` |
| Last session intent | `memory(action=get_fact, key=ctx.{sid}.intent)` |
| Active goals | `goal(action=list)` |
| Any other context | `memory(action=recall, query=...)` |

The `intent_key` field in the handoff entry gives the exact key for the ctx lookup — the agent never has to reconstruct or guess it.

---

## Section 5 — Tests

All LLM calls use a `HandoffLlm` mock — no live provider calls in tests.

| Test | File | What it verifies |
|------|------|-----------------|
| `generates_summary_from_messages` | `handoff_writer.rs` | Given N messages, LLM called once, output has non-empty `summary`, correct `session_id`, `ward_id`, `correction_count` |
| `writes_latest_and_archived_keys` | `handoff_writer.rs` | After `write()`, both `handoff.latest` and `handoff.{sid}` exist in the shared KV with identical content |
| `failure_is_silent` | `handoff_writer.rs` | LLM returns `Err` → no panic, no write, returns `Ok(())` |
| `stale_handoff_excluded` | `handoff_writer.rs` | Handoff `completed_at` > 7 days ago → `should_inject()` returns `false` |
| `injects_handoff_block_when_present` | `invoke_bootstrap.rs` | Bootstrap with valid `handoff.latest` → injected context string contains `## Last Session` and the summary text |
| `no_handoff_no_injection` | `invoke_bootstrap.rs` | Bootstrap with absent `handoff.latest` → injected context unchanged |

---

## Section 6 — Files Touched

| File | Change |
|------|--------|
| `gateway/gateway-execution/src/sleep/handoff_writer.rs` | **New** — `HandoffWriter`, `HandoffLlm` trait, `HandoffInput`, `HANDOFF_MAX_AGE_DAYS` |
| `gateway/gateway-execution/src/sleep/mod.rs` | Export `HandoffWriter`, `HandoffLlm`, `HandoffInput` |
| `gateway/gateway-execution/src/runner/core.rs` | On `status='completed'`: `tokio::spawn(handoff_writer.write(session_id))` — `HandoffWriter` injected at construction time alongside `memory_recall` |
| `gateway/gateway-execution/src/runner/invoke_bootstrap.rs` | Read `handoff.latest`, prepend `## Last Session` block before `memory_recall` injection |

**No DB schema changes. No new HTTP endpoints. No new tools.**

---

## Out of Scope (Future Phases)

- Recall noise / `min_score` threshold — Phase 2
- Strategy fact visibility (`memory(list)` across DB facts) — Phase 2
- Knowledge graph utilization — Phase 3
- Pattern abstraction / schema promotion — Phase 3
- Conflict resolution — Phase 4
