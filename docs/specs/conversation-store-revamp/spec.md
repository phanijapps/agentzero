# Conversation Store Revamp — Design Spec

**Status:** Approved (design) → pending implementation plan
**Date:** 2026-07-07
**Owner:** research / architecture
**Depends on:** none (greenfield crates, clean cutover)
**Related:** memory-layer modularization (`stores/zbot-*` pattern); bi-temporal / belief-network work (already shipped)

---

## TL;DR

The conversation database (`~/Documents/zbot/data/conversations.db`) is bloated because it co-locates
the append-only message log, the per-event execution trace, and (historically) memory/KG/embeddings in one
SQLite file, with **dual writes** that duplicate every tool call's args (2 places) and results (3 places)
and **no real state checkpoint** — agent state is *reconstructed by replay* on every read.

This revamp splits the conversation + trace domain into **two new, self-contained `stores/zbot-*` crates**
with narrow, single-purpose traits (no god classes), aligns the model to the cross-framework agentic
consensus (append-only messages + mutable checkpoint + append-only trace), and moves full-fidelity trace
payloads out of SQLite into streamed per-session `.jsonl.zst` files queryable by DuckDB. Rollout is a
**clean cutover** (delete the old DB — not prod) followed by **deletion of all superseded code**.

Locked decisions: **Option B** (agentic-aligned split) · **B1** (slim-hot `execution_logs` + streamed
`.jsonl.zst` + DuckDB) · **clean cutover, no migration** · **boundary (ii)** (move only
`messages`/`checkpoints`/`thread_summaries` + trace; keep `sessions`/`agent_executions` in place).

---

## 1. Context & motivation

### 1.1 Where the bloat comes from (evidence)

`conversations.db` (schema frozen at `SCHEMA_VERSION = 22`, `stores/zbot-stores-sqlite/src/schema.rs:9`)
holds: `sessions`, `agent_executions`, `messages`, `execution_logs`, `artifacts`, `distillation_runs`,
`recall_log`, `bridge_outbox`. Two structural problems drive the bloat:

1. **Dual-write per stream event.** Every tool call is written to *both* `messages` and `execution_logs`.
   A single `ToolResult` event → 2 `messages` rows + 1 `execution_logs` row. Tool **args** live in 2
   places (`messages.tool_calls` + `execution_logs.metadata.args`); tool **result** text lives in **3**
   places (full in `messages.content` for `role='tool'`; 1000-char-truncated in
   `execution_logs.metadata.result` via `services/api-logs`; 500-char-truncated via gateway
   `event_logging.rs` — *two different truncation policies for the same logical event*). Row growth is
   **O(events), ≈ 4N+5 rows per turn** (N = tool calls in the turn).
2. **No persisted state checkpoint.** `gateway/gateway-execution/src/session_state.rs` is a **read-only
   builder** that *reconstructs* state on every `/api/logs/sessions/:id` call by replaying
   `execution_logs` + `messages`. The only persisted execution-state column is
   `agent_executions.checkpoint` (an ad-hoc opaque JSON blob). Most agentic frameworks checkpoint state
   explicitly (LangGraph, Letta, Temporal).

### 1.2 What is already clean (out of scope)

- KG / embeddings / facts / beliefs already live in `knowledge.db` / `engram_data.db` (schema v21→v22
  relocation; `knowledge_schema.rs`). The only residual coupling is `distillation_runs` + `recall_log`
  staying on `conversations.db` for write-side status — unchanged by this revamp.
- `messages` are queryable **rows** (not serialized blobs) — good; preserved.
- `gateway-execution/src/archiver.rs` already implements a cold tier: SELECT → `.jsonl.gz` → DELETE rows.
  This revamp generalizes that pattern (gzip→zstd, archive-on-close → stream-live).

---

## 2. Research summary (grounding)

### 2.1 Trace storage formats

| Format | Live append | Filtered read | Schema evolution | Footprint | Rust maturity |
|---|---|---|---|---|---|
| **JSONL** (1 file/session) | Excellent | Poor (scan+parse) | Trivial | Verbose | `serde_json` (tier-1) |
| **Parquet** (1 file/session) | **Bad** — no append after writer close ([arrow-rs#557](https://github.com/apache/arrow-rs/issues/557), [DuckDB#3870](https://github.com/duckdb/duckdb/discussions/3870)) | Excellent (10–100×, predicate pushdown) | Stricter | Smallest | `arrow-rs`/`parquet` (tier-1) |
| **SQLite table/BLOB** | Good (WAL) | Excellent (indexed SQL) | Moderate | Native | `rusqlite` |
| **DuckDB** (single file) | Single-writer/multi-reader | Excellent (SQL over Parquet/JSON) | Strong | Native | `duckdb-rs` (official) |
| ND-MessagePack/CBOR | Excellent | Poor | Trivial | ~30% < JSON | `rmp-serde`/`cbor` |
| OTel spans → files | N/A (interchange, not store) | backend-dependent | Fast-moving | = JSONL | exporter crates |

**What production systems do:** every serious backend lands on columnar OLAP (ClickHouse) + blob storage
for large payloads — Langfuse v3 ([ClickHouse](https://langfuse.com/self-hosting/deployment/infrastructure/clickhouse)),
LangSmith ([SmithDB](https://www.langchain.com/blog/introducing-smithdb) + [blob storage](https://docs.langchain.com/langsmith/self-host-blob-storage)),
Braintrust. **Nobody writes live traces as Parquet** — Parquet is an *export* format. Arize Phoenix
defaults to local **SQLite + file exports** ([config](https://arize.com/docs/phoenix/self-hosting/configuration)).

**Conclusion:** JSONL-zstd per session (write) + DuckDB as analytical reader (query). zstd beats gzip
~6% smaller + ~2× faster decode ([Lemire](https://lemire.me/blog/2021/06/30/compressing-json-gzip-vs-zstd/)).
Adopt OTel GenAI attribute names inside the records for portability — OTel is a *schema*, not a store.

### 2.2 Agentic conversation/state best practices

Cross-framework consensus is a **5-way separation** (LangGraph, Letta/MemGPT, OpenAI Threads, Zep,
Temporal, Mem0, CrewAI):

| Store | Mutability | agentzero today |
|---|---|---|
| Message log | append-only / immutable | `messages` (rows ✓) |
| State checkpoint | mutable, latest-wins (history for time-travel) | ❌ none — replay-derived |
| Trace / execution events | append-only, per-step | `execution_logs` (co-located) |
| Vector / embeddings | derived | ✓ already in `knowledge.db` |
| Knowledge graph | mutable + versioned (invalidate, don't delete) | ✓ already separated |

- **Append-only messages + mutable checkpoint + append-only trace** is the strong consensus; full
  Temporal-style event-sourcing is overkill for a desktop agent → adopt the **snapshot model**
  (LangGraph) without the per-channel WAL day-one.
- **Compaction-without-loss:** never delete originals; derive summaries stored separately; context =
  latest summary + message tail (Letta, LangChain, Zep).
- **Messages stay queryable rows, never a serialized blob inside state JSON** — LangGraph's
  `checkpoint_blobs` column serializes the whole message list into an opaque binary value → 4s history
  loads, unqueryable ([lordpatil postmortem](https://blog.lordpatil.com/posts/langgraph-postgres-checkpointer/)).
  agentzero already does this right (messages are rows); preserve it.
- Recommended **message-row shape** (cross-framework union): monotonic id, thread_id, role, source,
  content (text or content-blocks), tool_calls jsonb, tool_call_id, metadata, token counts, created_at,
  per-thread `seq`.

> *"No source suggested co-locating messages, state, trace, embeddings, and KG in a single physical store
> is a good idea — the consensus runs firmly against the current architecture."*

---

## 3. Goals / non-goals

### Goals
1. Eliminate the dual-write duplication (args 2→, results 3→ places) and the dual truncation policy.
2. Introduce a real, versioned state checkpoint; replace replay-derived `session_state.rs` with O(1) read.
3. Move full-fidelity trace payloads out of SQLite into streamed per-session `.jsonl.zst`; add DuckDB
   cross-session analytics.
4. Add compaction-without-loss (`thread_summaries`).
5. Deliver as two clean, self-contained `stores/zbot-*` crates with narrow traits (no god classes).
6. Preserve the UI/API contracts (HTTP routes + response DTOs unchanged).

### Non-goals
- Touching `sessions`, `agent_executions`, `artifacts`, `distillation_runs`, `recall_log`,
  `bridge_outbox` (they stay in `zbot-stores-sqlite`).
- Moving memory/KG/embeddings (already separated).
- Full event-sourcing / state-as-projection (Temporal-style).
- WAL for partial-turn crash recovery (future enhancement).
- Evolving `messages.content` to typed multimodal content-blocks (future; keep TEXT day-one).
- Consolidating the two near-duplicate runner write loops (follow-up; only their *emit* shape changes here).

---

## 4. Target architecture

```
~/Documents/zbot/data/
├── conversations.db  (hot, SQLite)
│     sessions, agent_executions              ← stay in zbot-stores-sqlite (untouched)
│     artifacts, distillation_runs,
│     recall_log, bridge_outbox               ← stay (untouched)
│     ─── owned by zbot-conversation ───
│       messages           (append-only, +seq, -tool_results)
│       checkpoints        (NEW, versioned)
│       thread_summaries   (NEW, compaction-without-loss)
│     ─── owned by zbot-trace ───
│       execution_logs     (SLIM — writers emit no payload blobs)
│
└── traces/
      <session_id>.jsonl.zst   (streamed live, OTel-genai attrs)
      ...

analytics (read-only, on demand):  duckdb-rs ─ read_json_auto('traces/*.jsonl.zst')
```

**Dedup principle** — each consumer reads exactly one store:

| Consumer | Reads | No longer reads |
|---|---|---|
| LLM turn replay | `messages` | — |
| Live `/api/logs` UI | `execution_logs` (slim) | payload blobs |
| Session-detail / resume (`session_state.rs`) | `checkpoints` (O(1)) | replay |
| Cross-session analytics | DuckDB over `traces/` | SQLite scans |
| Compacted context window | latest `thread_summaries` + message tail | full history |

Full tool args/results live in exactly **two** places, each serving a distinct consumer: `messages`
(LLM replay) and `traces/*.jsonl.zst` (analytics/observability). `execution_logs.metadata` keeps only
display scalars (`tool_name`, error flag) — never payloads.

**Connection & pooling.** `zbot-conversation` and `zbot-trace` each own their tables in `conversations.db`
but do **not** own the connection pool. They accept a connection (DI — a thin `ConnectionProvider` trait
or `&rusqlite::Connection`) and the gateway wires all stores to the **same shared pool** at
`state/mod.rs`. This avoids WAL single-writer contention across crates and avoids re-implementing pool
setup in each crate. It is a *consumer* relationship — the new crates depend on rusqlite/connection
abstractions, **not** on `zbot-stores-sqlite::DatabaseManager` (so "don't migrate existing code" holds).
The existing `BatchWriter` remains the write serializer for the conversation tables.

---

## 5. Crate 1 — `stores/zbot-conversation/`

Self-contained: owns domain POD types, narrow traits, SQLite schema (its own v1). **No fat facade** —
consumers compose the traits they need.

```
stores/zbot-conversation/
  src/domain.rs       POD: Message, Checkpoint, ThreadSummary
  src/messages.rs     trait MessageStore  + SqliteMessageStore
  src/checkpoints.rs  trait CheckpointStore + SqliteCheckpointStore
  src/summaries.rs    trait SummaryStore  + SqliteSummaryStore
  src/schema.rs       v1 DDL (messages, checkpoints, thread_summaries)
  src/lib.rs          re-exports only
```

### 5.1 `MessageStore` (messages.rs)
```rust
pub trait MessageStore: Send + Sync {
    fn append(&self, msg: Message) -> Result<()>;
    /// Replay for the next LLM turn / UI history. ORDER BY seq; cursor pagination via after_seq.
    fn replay(&self, session_id: &str, after_seq: Option<i64>, limit: usize) -> Result<Vec<Message>>;
    fn next_seq(&self, session_id: &str) -> Result<i64>;
}
```
Small. ~100 LOC. The DTO adapter (`Message` → `SessionMessageResponse`) lives at the handler, not here.

### 5.2 `CheckpointStore` (checkpoints.rs)
```rust
pub trait CheckpointStore: Send + Sync {
    fn write(&self, cp: Checkpoint) -> Result<()>;
    fn latest(&self, execution_id: &str) -> Result<Option<Checkpoint>>;   // ORDER BY id DESC LIMIT 1
}
```
Promotes the **existing** `Checkpoint` struct (`services/execution-state/src/types.rs:667`) verbatim, +`schema_version`:

```sql
CREATE TABLE checkpoints (
  id              TEXT PRIMARY KEY,            -- monotonic UUIDv7 → latest = ORDER BY id DESC LIMIT 1
  execution_id    TEXT NOT NULL,
  session_id      TEXT NOT NULL,
  llm_turn        INTEGER NOT NULL,
  last_message_id TEXT NOT NULL,
  pending_tool_calls TEXT,                      -- JSON (existing field)
  context_state   TEXT,                         -- JSON: typed {plan, recalled_facts, intent, ward, subagents}
  child_executions TEXT,                        -- JSON (existing field)
  schema_version  INTEGER NOT NULL DEFAULT 1,   -- AutoGen-style blob versioning
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_checkpoints_exec ON checkpoints(execution_id, id DESC);
```
- `context_state` stays JSON but **typed** (a documented struct), versioned — *not* an opaque blob.
  Messages stay rows; whole-state snapshot stays a versioned blob. (The LangGraph anti-pattern is
  serializing the *message list* into an opaque value; we don't.)
- **No WAL day-one** (YAGNI). Partial-turn crash recovery noted as future.

### 5.3 `SummaryStore` (summaries.rs)
```rust
pub trait SummaryStore: Send + Sync {
    fn write(&self, summary: ThreadSummary) -> Result<()>;
    fn latest(&self, session_id: &str) -> Result<Option<ThreadSummary>>;   // highest as_of_seq
}
```
```sql
CREATE TABLE thread_summaries (
  id               TEXT PRIMARY KEY,
  session_id       TEXT NOT NULL,
  as_of_message_id TEXT NOT NULL,
  as_of_seq        INTEGER NOT NULL,
  summary          TEXT NOT NULL,
  schema_version   INTEGER NOT NULL DEFAULT 1,
  created_at       TEXT NOT NULL
);
CREATE INDEX idx_summaries_session ON thread_summaries(session_id, as_of_seq DESC);
```
Context-window assembly = latest `thread_summaries` + `messages WHERE seq > as_of_seq`.

> ⚠️ **Honors `feedback_orchestrator_context_high_stakes`:** this compacts **message history** only. It
> does **not** alter committed-plan-inline delivery. Plan/goal delivery is untouched.

### 5.4 `messages` schema changes (v1 in this crate; old DDL deleted from `zbot-stores-sqlite`)
```sql
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    execution_id TEXT,
    session_id   TEXT NOT NULL,
    role         TEXT NOT NULL,
    content      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    token_count  INTEGER DEFAULT 0,
    tool_calls   TEXT,          -- JSON (kept: LLM replay needs it)
    tool_call_id TEXT,
    seq          INTEGER NOT NULL   -- NEW: per-session monotonic
    -- tool_results DROPPED (legacy, already NULL on current write path)
);
CREATE INDEX idx_messages_session_seq ON messages(session_id, seq);
```

---

## 6. Crate 2 — `stores/zbot-trace/`

```
stores/zbot-trace/
  src/domain.rs       POD: TraceEvent (OTel-genai), SlimLog
  src/slim_logs.rs    trait SlimLogStore + SqliteSlimLogStore   (execution_logs, payload-free)
  src/writer.rs       TraceWriter — per-session .jsonl.zst streaming append (zstd, frame-per-flush)
  src/analytics.rs    TraceAnalytics — duckdb-rs, read_json_auto('traces/*.jsonl.zst'), read-only
  src/schema.rs       slim execution_logs DDL (columns unchanged from today)
  src/lib.rs          re-exports only
```

### 6.1 `SlimLogStore` (slim_logs.rs)
`execution_logs` **columns unchanged** — the fix is **writer-discipline**: writers stop emitting
`args`/`result` strings into `metadata`; they keep only display scalars (`{tool_name}`, `{error}`). The
500 vs 1000-char dual truncation policy becomes moot (nothing to truncate). The `/api/logs` UI keeps
working with the same SQL + response shape — **zero integration work**.

### 6.2 `TraceWriter` (writer.rs)
Per-session zstd streaming encoder. Frame-per-flush so appends are safe and crash-recovery leaves a valid
file up to the last flushed frame. Flushed on the existing `BatchWriter` cadence (100ms / 10 items,
`gateway-execution/src/invoke/batch_writer.rs:144`). Replaces the archiver's gzip+`.jsonl.gz`; the
archiver becomes a **finalizer** (flush + close on session end), not a relocate-and-delete.

### 6.3 `TraceAnalytics` (analytics.rs)
`duckdb-rs`, **read-only**, **no persistent DuckDB file**: queries run over
`read_json_auto('traces/*.jsonl.zst')` with predicate pushdown. Opened per-query or pooled; the
single-writer constraint does not apply (we never *write* via DuckDB). Exposed via a new
`/api/traces/query` endpoint (additive — no existing integration to break).

---

## 7. Write path & read path changes

### 7.1 Write path
- **`BatchWriter`** (`invoke/batch_writer.rs`) gains a third sink: a `TraceWriter` handle per active
  session. New request kind `TraceEvent` (full OTel payload). Existing `SessionMessage` and `LogEntry`
  (now slim) shapes unchanged.
- **Dual write sites** — `runner/execution_stream.rs` (primary) **and** `runner/core.rs:1400-1552`
  (continuation/resume) — both get the payload-routing change (args/results → trace file; scalars →
  `execution_logs.metadata`; conversational turn → `messages` via `MessageStore`;
  turn boundary → `CheckpointStore`). *Consolidating the two loops is a noted follow-up, not bundled.*
- **Trace events** routed from `stream_event_processor.rs:105-141` (`ToolCallStart`→`log_tool_call`,
  `ToolResult`→`log_tool_result`) now also emit a `TraceEvent` to the `TraceWriter`.
- **Subagent results** (`delegation/callback.rs:238`) continue appending a `system` message to
  `messages`; the same payload also emits a trace event.

### 7.2 Read path
- `http/chat.rs:185` (`GET /api/sessions/:id/messages`) → `MessageStore::replay`. DTO adapter fills
  `tool_results: None` to preserve the UI contract.
- `session_state.rs` → `CheckpointStore::latest` (O(1)), replacing the replay loop.
- `runner/core.rs:1262` + `invoke_bootstrap.rs:480` (replay to `ChatMessage`) → `MessageStore::replay`.
- `distillation.rs:363`, `sleep/handoff_writer.rs:262`, `sleep/pattern_extractor.rs:165`,
  `services/execution-state/src/repository.rs:1182` → corresponding narrow-trait reads.

---

## 8. API-layer integration (the two seams)

**Seam 1 — `AppState` concrete coupling.** `gateway/src/state/mod.rs:57` holds
`pub conversations: Arc<ConversationRepository>` (concrete, bypasses the `ConversationStore` trait at
`stores/zbot-stores-traits/src/conversation.rs:33`). The codebase already does this correctly for memory:
`pub memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>` (line 82). We follow that exact pattern:
```rust
pub messages:   Arc<dyn zbot_conversation::MessageStore>,
pub checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
pub summaries:  Arc<dyn zbot_conversation::SummaryStore>,
pub slim_logs:  Arc<dyn zbot_trace::SlimLogStore>,
pub trace_writer: Arc<dyn zbot_trace::TraceWriter>,
pub trace_analytics: Arc<dyn zbot_trace::TraceAnalytics>,
```
Touches: the `AppState` struct + **3 construction sites** (`state/mod.rs:842, 942, 1093`) + **~4 handler
files** (`chat.rs`, `conversations.rs`, `sessions.rs`, `graph.rs`). Mechanical; strictly an improvement
(each handler depends on the minimum).

**Seam 2 — UI response contract.** `SessionMessageResponse` (`chat.rs:35`, incl. `tool_results` line 42)
and `apps/ui/src/services/transport/types.ts:121` expose `tool_results`. We **keep the DTO field mapped
to `None`** (drop the DB column, keep the wire field). Handler is a thin adapter. **Zero frontend work.**
A paired UI cleanup to drop the dead field can follow.

**Already seamless:** `/api/logs` (execution_logs columns unchanged); `/api/traces/query` (additive).

---

## 9. OTel GenAI attribute set (trace records)

Each `.jsonl.zst` record carries OTel GenAI semconv names for portability to Datadog/Langfuse/Phoenix:
`trace_id`, `span_id`, `session_id`, `execution_id`, `agent_id`, `parent_session_id`, `timestamp`,
`level`, `category`, `duration_ms`, `gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.input_tokens` /
`output_tokens`, `tool.name`, `gen_ai.tool.call.input` / `.output` (full payloads here, not in SQLite),
and span events for prompt/completion. Ref: [OTel GenAI agent spans](https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-agent-spans/).

---

## 10. Testing & verification (goal-driven — CLAUDE.md §4)

1. **Round-trip equivalence** (gate for the replay→checkpoint switch): write a synthetic session; assert
   `MessageStore::replay` == original `ChatMessage` sequence; assert `CheckpointStore::latest` ==
   replay-derived state (compute both, diff must be empty).
2. **Bloat assertion**: same session; assert `execution_logs.metadata` contains **no** full payload
   (regex: no `args`/`result` string > N chars); assert the full payload **is** present in the
   `.jsonl.zst`.
3. **DuckDB query**: seed N synthetic sessions; `read_json_auto` returns correct row counts;
   `WHERE tool.name='X' AND level='error'` filters correctly.
4. **Append-safety**: kill mid-session; assert `.jsonl.zst` is valid (frame-complete) up to last flush.
5. **API contract**: `GET /api/sessions/:id/messages` and `/api/logs` response shapes byte-identical
   pre/post cutover (snapshot/golden tests).

Mechanical gates after each phase: `cargo check --workspace` (Rust), `npm run build` (TS, UI unchanged
but verify).

---

## 11. Rollout — clean cutover + dead-code deletion

**Not prod → delete `conversations.db`; new path is the only path. No migration/backfill job.**

### 11.1 Build (new crates alongside, old code untouched)
1. `stores/zbot-conversation/` — domain + 3 traits + 3 SQLite impls + schema v1.
2. `stores/zbot-trace/` — domain + `SlimLogStore` + `TraceWriter` + `TraceAnalytics` + schema.

### 11.2 Cutover (flip consumers)
3. Rewire `AppState` (Seam 1) + handler reads (§7.2).
4. Rewire `BatchWriter` + dual write sites (§7.1) to emit slim logs + trace events + messages + checkpoints.
5. Switch `session_state.rs` replay → `CheckpointStore::latest`.
6. Add `/api/traces/query`.

### 11.3 Delete all superseded code (dead after cutover)
- `stores/zbot-stores-sqlite/src/schema.rs`: `messages` + `execution_logs` DDL — **moved** to the new
  crates' `schema.rs` (`messages` → `zbot-conversation`, `execution_logs` → `zbot-trace`; the
  `execution_logs` shape is unchanged, just relocated + slim-by-writer-discipline).
- `stores/zbot-stores-sqlite/src/repository.rs`: `ConversationRepository` (+ `get_session_conversation`,
  `append_session_message`, `add_message_with_tools`, `tool_sequence_for_session`).
- `stores/zbot-stores-domain/src/message.rs`: `Message` POD (moved to `zbot-conversation/domain`).
- `stores/zbot-stores-traits/src/conversation.rs`: `ConversationStore` trait (superseded by narrow traits).
- `services/execution-state/src/types.rs`: `Checkpoint` struct (moved to `zbot-conversation/domain`).
- `gateway-execution/src/archiver.rs`: old gz archiver (replaced by `TraceWriter` finalize).
- `gateway-execution/src/session_state.rs`: replay loop (replaced by checkpoint read).
- `services/api-logs` truncation logic (500/1000-char) — moot.

---

## 12. Risks & open questions

- **`duckdb-rs` cross-compile** for Windows/macOS bundled desktop builds — needs an early build-smoke
  check. Mitigation: the analytics layer is isolated behind `TraceAnalytics`; if it fails to build on a
  target, it can be feature-flagged off without affecting the conversation store or live logs UI.
- **zstd frame-per-flush** compresses less than one-frame-per-file. Acceptable (zstd still beats gzip).
  Optional: periodic re-compression on session close.
- **Messages never-deleted** → unbounded growth on a long-lived desktop install. The existing
  `SessionArchiver` still handles old-session archival (now archiving slim messages to file). Flagged,
  not solved here.
- **Checkpoint `context_state` shape** is typed-but-JSON day-one; if field-level queryability of state is
  ever needed, promote columns later (additive).
- **Two runner write loops remain duplicated** — consolidation is a follow-up; only their emit shape
  changes here (surgical).

---

## 13. Alternatives considered

- **Option A (surgical slim):** keep `conversations.db` structurally as-is, just dedupe + auto-archive +
  DuckDB reader, no checkpoint. Rejected — doesn't fix the missing-checkpoint root cause; not
  "agentic-aligned."
- **Option C (full rearchitecture):** separate `messages.db` + `state.db`, OTel everywhere,
  event-sourced state-as-projection (Temporal-style). Rejected — highest risk/effort, touches every
  write/read site, full event-sourcing overkill for desktop.
- **B2 (file-only trace + DuckDB):** remove `execution_logs` from SQLite entirely; rewire `/api/logs` to
  read via DuckDB. Rejected — makes DuckDB a hard dependency for basic log viewing and forces logs-UI
  rework. B1 keeps the live UI on SQLite.
- **B3 (defer DuckDB):** no `duckdb-rs` now. Rejected — the user wants cross-session trace analytics;
  B1 delivers it.
- **M1/M3 (backfill / cutover-with-migration):** rejected — not prod, old DB deleted; no migration needed.
- **Boundary (i) (move sessions/agent_executions too):** rejected — widens API rewiring surface to every
  session-lifecycle consumer for no bloat benefit (they aren't "conversational memory/state").

---

## 14. Out of scope / future

- Checkpoint WAL for partial-turn crash recovery.
- `messages.content` → typed multimodal content-blocks (align with the `Part` enum).
- Consolidating `runner/execution_stream.rs` + `runner/core.rs` write loops.
- Parquet-on-close for closed sessions (faster analytics than JSONL) — optional optimization once
  `.jsonl.zst` is canonical.
- Archival policy for unbounded `messages` growth on long-lived installs.
