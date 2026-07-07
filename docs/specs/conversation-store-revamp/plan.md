# Plan: Conversation Store Revamp

- **Spec:** [`spec.md`](spec.md)
- **Status:** Drafting

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Build two new self-contained crates alongside the old code, flip consumers to
them, then delete the dead old code. Four phases: (1) `stores/zbot-conversation`
— messages/checkpoints/thread_summaries behind narrow traits; (2)
`stores/zbot-trace` — slim `execution_logs` + streamed `.jsonl.zst` + DuckDB
analytics; (3) cutover — rewire `AppState`, `BatchWriter`, the dual write sites,
`session_state`, and the read sites; add `/api/traces/query`; (4) delete all
superseded code. Clean cutover (old `conversations.db` deleted, not prod) — no
migration/backfill. The riskiest part is Phase 3 (live read/write paths across
~15 files); it is done one task per commit with a daemon smoke after each.

Each consumer reads exactly one store (dedup principle): LLM replay →
`messages`; live logs UI → slim `execution_logs`; session detail/resume →
`checkpoints`; cross-session analytics → DuckDB over `traces/`; compacted context
→ latest `thread_summaries` + message tail.

## Constraints

- Two new crates only: `stores/zbot-conversation`, `stores/zbot-trace`. New deps
  limited to `duckdb` and `zstd`. No god-class facades.
- `sessions`/`agent_executions`/`artifacts`/`distillation_runs`/`recall_log`/
  `bridge_outbox` stay in `zbot-stores-sqlite`, untouched.
- UI/API contracts frozen (DTOs unchanged; `tool_results` wire field stays,
  mapped to `None`).
- Each new crate opens its own `r2d2` pool to `conversations.db` (WAL +
  `busy_timeout`); it does **not** depend on `DatabaseManager`.
- `rusqlite = "0.32"` with `["bundled"]` in both crates (unified sqlite link).
- `Uuid::now_v7()` for monotonic ids (add `features = ["v7"]` to workspace
  `uuid`).

## Construction tests

Most construction tests live under **Tasks** below (per-task `Tests:`). Cross-cutting:

- **Integration:** `session_state` equivalence — seed a session through the new
  write path, compute state two ways (checkpoint read vs. an independent replay
  of messages), assert equal. Golden snapshot of `GET /api/sessions/:id/messages`
  + `/api/logs` shapes before/after cutover.
- **Manual verification:** one conversation run through the daemon — observe
  messages/checkpoints/trace file/slim logs/logs UI end-to-end.

## Design (LLD)

### Design decisions

- **Two crates, narrow traits, no facade** — each store is one trait + one impl.
  Rejected: a single `ConversationStore`/`TraceStore` mega-trait (god class).
  Rejected: extending the old `ConversationRepository` (in-place migration).
- **Slim by writer-discipline, not schema change** — `execution_logs` keeps its
  columns; writers stop emitting payloads into `metadata`. Keeps `/api/logs`
  untouched. Rejected: dropping `execution_logs` from SQLite (would force the
  logs UI onto DuckDB — `B2`).
- **JSONL-zstd per session + DuckDB reader** — Parquet can't append after
  writer-close, so it is export-only; nobody writes live traces as Parquet.
  Rejected: live Parquet; deferred DuckDB (`B3`).
- **Promote the existing `Checkpoint`** to a versioned table; `session_state`
  switches replay→O(1) read. Rejected: inventing a parallel checkpoint system;
  full event-sourcing (Temporal-style).
- **Clean cutover, delete old DB** — not prod. Rejected: backfill (`M1`),
  cutover-with-migration (`M3`).

Traces to: ACs (args/result single-column, O(1) state, slim logs, crash-safe
trace, cross-session query, contracts unchanged) · `gateway/src/http/openapi.yaml`.

### Data & schema

`zbot-conversation` v1: `messages` (id, execution_id, session_id, role, content,
created_at, token_count, tool_calls, tool_call_id, **seq**; legacy `tool_results`
dropped) + `idx_messages_session_seq`; `checkpoints` (id, execution_id,
session_id, llm_turn, last_message_id, pending_tool_calls, context_state,
child_executions, schema_version, created_at) + `idx_checkpoints_exec`;
`thread_summaries` (id, session_id, as_of_message_id, as_of_seq, summary,
schema_version, created_at) + `idx_summaries_session`.

`zbot-trace`: `execution_logs` — **columns unchanged** (id, session_id,
conversation_id, agent_id, parent_session_id, timestamp, level, category,
message, metadata, duration_ms) + existing indexes.

Traces to: ACs (single-column payloads, O(1) state) · openapi.yaml.

### Interfaces & contracts

Narrow traits (in the crates): `MessageStore { append, replay, next_seq }`,
`CheckpointStore { write, latest }`, `SummaryStore { write, latest }`,
`SlimLogStore { append, query }`, `TraceWriter { open, append, flush, close }`,
`TraceAnalytics { open, sessions_with_failed_tool, query }`. Each `Send + Sync`,
sync methods, `anyhow::Result`. `AppState` holds them as `Arc<dyn Trait>` (the
`memory_store` pattern).

Public contract: `POST /api/traces/query` added to
`gateway/src/http/openapi.yaml` — accepts a preset name + params (no raw
client SQL; injection-safe), returns JSON rows.

Traces to: ACs (cross-session query, contracts unchanged) · openapi.yaml.

### Component / module decomposition

`stores/zbot-conversation/`: `domain.rs` (Message/Checkpoint/ThreadSummary POD),
`pool.rs` (`open_conversation_pool`), `schema.rs`, `messages.rs`,
`checkpoints.rs`, `summaries.rs`, `lib.rs` (re-exports only).

`stores/zbot-trace/`: `domain.rs` (TraceEvent OTel-genai / SlimLog), `pool.rs`,
`schema.rs`, `slim_logs.rs`, `writer.rs`, `analytics.rs`, `lib.rs`.

Traces to: AC (no superseded symbols) · openapi.yaml.

### Behavior & rules

Payload routing (the core dedup):

| Event | `messages` | `execution_logs.metadata` | trace `.jsonl.zst` |
|---|---|---|---|
| User input | role=user | — | TraceEvent(session) |
| ToolCallStart | role=assistant w/ tool_calls | `{tool_name}` | TraceEvent(tool_call, payload=args) |
| ToolResult | role=tool (full result) | `{tool_name}` | TraceEvent(tool_result, payload=result) |
| Turn boundary | — | — | CheckpointStore.write |
| Delegation result | role=system | — | TraceEvent(delegation) |

Traces to: ACs (single-column payloads, slim logs) · openapi.yaml.

### Failure, edge cases & resilience

- `TraceWriter` writes one zstd frame per flush → a crash leaves the file valid
  and decodable up to the last flushed frame. (AC: crash-safe trace.)
- Cross-pool writes to `conversations.db` are serialized by WAL +
  `busy_timeout=5000`; the `BatchWriter` remains the write serializer.
- `duckdb-rs` `bundled` cross-compile to Windows/macOS builds DuckDB from source;
  resolve at build time (the analytics layer is isolated behind `TraceAnalytics`,
  so a cross-compile problem is contained to Task 8 / the analytics endpoint).

### Dependencies & integration

New: `duckdb` (v1.5.x line, `bundled`), `zstd` (0.13). Reused: `rusqlite` 0.32
bundled, `r2d2`/`r2d2_sqlite`, `serde`/`serde_json`, `anyhow`/`thiserror`,
`uuid` (+v7), `chrono`. The new crates consume no `zbot-stores-sqlite` types.

## Tasks

### T1: `zbot-conversation` crate — domain + schema v1

**Depends on:** none

**Tests:**
- Schema initializes all three tables + indexes in an in-memory DB (AC: foundational; goal-based).

**Approach:**
- Add `"stores/zbot-conversation",` to workspace `members` (`Cargo.toml`).
- Create `Cargo.toml` (rusqlite 0.32 bundled, r2d2, r2d2_sqlite, serde, serde_json, anyhow, thiserror, uuid +v7, chrono; dev-dep tempfile).
- `src/domain.rs` — `Message` (id, execution_id, session_id, role, content, created_at, token_count, tool_calls, tool_call_id, seq), `Checkpoint` (id, execution_id, session_id, llm_turn, last_message_id, pending_tool_calls, context_state, child_executions, schema_version, created_at), `ThreadSummary` (id, session_id, as_of_message_id, as_of_seq, summary, schema_version, created_at).
- `src/schema.rs` — `SCHEMA_SQL` (messages/checkpoints/thread_summaries DDL + indexes, per Design §Data & schema) + `initialize(&Connection)`.
- `src/pool.rs` — `open_conversation_pool(path) -> Result<Pool>` (r2d2, WAL/busy_timeout/foreign_keys pragmas, calls `schema::initialize`).
- `src/lib.rs` — `pub mod domain/schema; mod pool;` re-export POD types + `open_conversation_pool`.
- `tests/schema.rs` — assert 3 tables exist after `initialize`.

**Done when:** `cargo test -p zbot-conversation --test schema` green and `cargo check --workspace` clean.

### T2: `MessageStore`

**Depends on:** T1

**Tests:**
- `append` then `replay` round-trips content and seq; `next_seq` is monotonic across 3 appends (AC: messages append-only).

**Approach:**
- `src/messages.rs` — `trait MessageStore: Send+Sync { fn append(&self,&Message)->Result<()>; fn replay(&self,session_id,after_seq:Option<i64>,limit)->Result<Vec<Message>>; fn next_seq(&self,session_id)->Result<i64>; }` + `SqliteMessageStore { pool }`.
- INSERT all fields; replay `WHERE session_id=? [AND seq>?] ORDER BY seq ASC LIMIT ?`; `next_seq` = `MAX(seq)+1`.
- Re-export in `lib.rs`; `tests/messages.rs`.

**Done when:** `cargo test -p zbot-conversation --test messages` green.

### T3: `CheckpointStore`

**Depends on:** T1

**Tests:**
- `write` 3 checkpoints (turns 1–3); `latest` returns turn 3 (AC: O(1) latest-wins state).

**Approach:**
- `src/checkpoints.rs` — `trait CheckpointStore { write(&self,&Checkpoint); latest(&self,execution_id)->Option<Checkpoint>; }` + `SqliteCheckpointStore`.
- INSERT; `latest` = `ORDER BY id DESC LIMIT 1` (UUIDv7 monotonic). Re-export; `tests/checkpoints.rs`.

**Done when:** `cargo test -p zbot-conversation --test checkpoints` green.

### T4: `SummaryStore`

**Depends on:** T1

**Tests:**
- `write` summaries at as_of_seq 10 and 20; `latest` returns 20 (AC: compaction-without-loss).

**Approach:**
- `src/summaries.rs` — `trait SummaryStore { write(&self,&ThreadSummary); latest(&self,session_id)->Option<ThreadSummary>; }` + `SqliteSummaryStore`. `latest` = `ORDER BY as_of_seq DESC LIMIT 1`. Re-export; `tests/summaries.rs`.

**Done when:** `cargo test -p zbot-conversation --test summaries` green; full `cargo test -p zbot-conversation` green.

### T5: `zbot-trace` crate — domain + schema

**Depends on:** none

**Tests:**
- Schema initializes `execution_logs` + 3 indexes (goal-based).

**Approach:**
- Add `"stores/zbot-trace",` to workspace `members`.
- `Cargo.toml` (same as T1 + `zstd = "0.13"`, `duckdb = { version = "1", features = ["bundled"] }`, `tracing`).
- `src/domain.rs` — `SlimLog` (id, session_id, conversation_id, agent_id, parent_session_id, timestamp, level, category, message, metadata, duration_ms) and `TraceEvent` (trace_id, span_id, session_id, execution_id, agent_id, parent_session_id, timestamp, level, category, message, duration_ms, tool_name, payload: Option<Value>, usage: Option<Value>, model) with OTel-genai field names + `skip_serializing_if = "Option::is_none"`.
- `src/schema.rs` — `execution_logs` DDL (columns unchanged) + indexes.
- `src/pool.rs` — `open_trace_pool(path)` (mirrors T1's pool, calls `schema::initialize`).
- `tests/schema.rs`.

**Done when:** `cargo test -p zbot-trace --test schema` green; `cargo check --workspace` clean.

### T6: `SlimLogStore`

**Depends on:** T5

**Tests:**
- `append` stores a row whose `metadata` is `{"tool_name":"read_file"}` — asserts **no** `args`/`result` keys; `query` returns it (AC: slim logs payload-free).

**Approach:**
- `src/slim_logs.rs` — `trait SlimLogStore { append(&self,&SlimLog); query(&self,session_id,limit)->Vec<SlimLog>; }` + `SqliteSlimLogStore`. INSERT; SELECT `ORDER BY timestamp`. Re-export; `tests/slim_logs.rs`.

**Done when:** `cargo test -p zbot-trace --test slim_logs` green.

### T7: `TraceWriter` (streaming `.jsonl.zst`)

**Depends on:** T5

**Tests:**
- append a,b,flush,append c,close → file decodes to 3 lines (round-trip).
- Property: append a, flush, append b (no flush), drop writer without close → file decodes to **1** line (crash leaves a valid, frame-complete file) (AC: crash-safe trace).

**Approach:**
- `src/writer.rs` — `TraceWriter::open(path)`, `append(&mut self,&TraceEvent)` (serialize JSON + `\n` into a `zstd::stream::write::Encoder`), `flush(&mut self)` (flush the current zstd frame so the file is decodable to here), `close(self)` (finish). Frame-per-flush = safe appends. Add a `read_zstd_lines` test helper.
- `tests/writer.rs`.

**Done when:** `cargo test -p zbot-trace --test writer` green (both tests).

### T8: `TraceAnalytics` (DuckDB over `.jsonl.zst`)

**Depends on:** T5, T7

**Tests:**
- Write 2 `.jsonl.zst` via `TraceWriter` (one with a `tool_result` error for `read_file`, one without); `sessions_with_failed_tool("read_file")` returns exactly 1 session (AC: cross-session trace query).

**Approach:**
- `src/analytics.rs` — `TraceAnalytics::open(traces_dir)`, holding an in-memory `duckdb::Connection`; `sessions_with_failed_tool(tool)` runs `SELECT DISTINCT json_extract(line,'$.session_id') FROM read_json_auto('<dir>/*.jsonl.zst') WHERE json_extract(line,'$.tool_name')='"tool"' AND json_extract(line,'$.level')='"error"'`. Tune `read_json_auto` options to the form the test confirms. `tests/analytics.rs`.

**Done when:** `cargo test -p zbot-trace --test analytics` green; `cargo check --workspace` clean. (If `duckdb` `bundled` fails to build on this host, surface — it is the one cross-compile risk.)

### T9: Wire stores into `AppState`

**Depends on:** T2, T3, T4, T6, T7, T8

**Touches:** `gateway/Cargo.toml`, `gateway/src/state/mod.rs`, `gateway-services/src/paths.rs`

**Tests:**
- `cargo check -p gateway` clean (goal-based; the wiring compiles and the 3 construction sites resolve).

**Approach:**
- `gateway/Cargo.toml`: add `zbot-conversation`, `zbot-trace` path deps.
- `gateway/src/state/mod.rs`: add `messages: Arc<dyn MessageStore>`, `checkpoints: Arc<dyn CheckpointStore>`, `summaries: Arc<dyn SummaryStore>`, `slim_logs: Arc<dyn SlimLogStore>`, `trace_analytics: Arc<dyn TraceAnalytics>` (mirror `memory_store` at `:82`). Keep the old `conversations` field until T16.
- Add a `build_conversation_stores(paths)` helper; call from the 3 construction sites (`:842/:942/:1093`).
- Add `traces_dir` to `gateway-services::paths` (mirror `conversations_db`).

**Done when:** `cargo check -p gateway` clean.

### T10: `BatchWriter` gains the `TraceWriter` sink

**Depends on:** T7, T9

**Touches:** `gateway-execution/src/invoke/batch_writer.rs`, `gateway-execution/src/lifecycle.rs`

**Tests:**
- Feed 3 `TraceEvent`s for a session through the handle, close, assert the `.jsonl.zst` has 3 lines (integration).

**Approach:**
- Add `TraceEvent { session_id, event }` to the mpsc channel type. Hold a `HashMap<session_id, TraceWriter>` in the writer task; the 100ms tick (`batch_writer.rs:144`) now also flushes all open trace writers. Add `close_session_trace(session_id)` called from session-end in `lifecycle.rs`.

**Done when:** the integration test green; `cargo check --workspace` clean.

### T11: Rewire the dual write sites (payload routing)

**Depends on:** T9, T10

**Touches:** `gateway-execution/src/invoke/event_logging.rs`, `gateway-execution/src/invoke/stream_event_processor.rs`, `gateway-execution/src/runner/execution_stream.rs`, `gateway-execution/src/runner/core.rs`, `gateway-execution/src/delegation/callback.rs`

**Tests:**
- After one conversation: `execution_logs.metadata` has **no** `args`/`result` keys (sqlite3 CLI assertion); `.jsonl.zst` contains them; a `checkpoints` row exists per turn (AC: single-column payloads, slim logs).

**Approach:**
- `event_logging.rs`: `log_tool_call`/`log_tool_result` write `metadata = {tool_name}` (+`{error}` for results) only; delete the 500-char truncation. Keep signatures.
- `stream_event_processor.rs:105-141`: alongside each `log_*`, push a `TraceEvent` (full payload) to the BatchWriter.
- `execution_stream.rs:159-266,325,541-559`: replace `conversation_repo.append_session_message` with `messages.append` (set `seq = next_seq`); at turn end `checkpoints.write`.
- Mirror in `runner/core.rs:1400-1552`.
- `delegation/callback.rs:238`: system-message append via `MessageStore` + a delegation `TraceEvent`.
- Delete `conversations.db` (empty/disposable), run one conversation, verify the assertions above.

**Done when:** the post-conversation assertions hold; `cargo check --workspace` clean.

### T12: `session_state` — replay → `CheckpointStore::latest`

**Depends on:** T3, T11

**Touches:** `gateway-execution/src/session_state.rs`

**Tests:**
- Seed a checkpoint; `GET /api/logs/sessions/:id` returns its plan/ward (AC: O(1) state, no replay); equivalence to a replay computed independently.

**Approach:**
- Replace the replay aggregation with `checkpoints.latest(execution_id)`; deserialize `context_state` into the `SessionState` fields. Keep the returned JSON shape (UI contract).

**Done when:** the test green; response shape unchanged.

### T13: Rewire read sites

**Depends on:** T2, T11

**Touches:** `gateway/src/http/chat.rs`, `gateway-execution/src/runner/core.rs`, `gateway-execution/src/runner/invoke_bootstrap.rs`, `gateway-execution/src/distillation.rs`, `gateway-execution/src/sleep/handoff_writer.rs`, `gateway-execution/src/sleep/pattern_extractor.rs`, `services/execution-state/src/repository.rs`

**Tests:**
- `GET /api/sessions/:id/messages` returns the same rows via `MessageStore::replay`; `tool_results` wire field is `None` (AC: contracts unchanged).

**Approach:**
- `chat.rs:185-215` → `messages.replay(.., None, limit)`; map `Message`→`SessionMessageResponse` (`tool_results: None`).
- `core.rs:1262` + `invoke_bootstrap.rs:480` → `replay(.., 200)` then `session_messages_to_chat_format` (drop the `tool_results` parse).
- distillation / handoff / pattern_extractor → `replay` (pattern_extractor still parses `tool_calls` JSON).
- `services/execution-state/src/repository.rs:1182` join → `replay` + execution metadata.

**Done when:** UI history loads; `cargo check --workspace` clean.

### T14: `/api/traces/query` + openapi

**Depends on:** T8, T9

**Touches:** `gateway/src/http/traces.rs` (new), `gateway/src/http/mod.rs`, `gateway/src/http/openapi.yaml`

**Tests:**
- Seed `.jsonl.zst`; `POST /api/traces/query` returns the failed-tool sessions (AC: cross-session query; documented contract).

**Approach:**
- `traces.rs`: `POST /api/traces/query { preset, params }` → `trace_analytics.*`; return JSON rows. Whitelist presets (no raw client SQL — injection-safe).
- Mount in `http/mod.rs`; extend `openapi.yaml` with the path + schema (hand-authored; note `api-contract` skill absent).

**Done when:** test green; `openapi.yaml` carries the new path; `npm run build` unaffected.

### T15: API contract golden tests

**Depends on:** T13, T14

**Touches:** `gateway/tests/api_contract.rs` (new)

**Tests:**
- Snapshot the JSON of `GET /api/sessions/:id/messages` and `/api/logs/sessions/:id` against a seeded store; assert byte-identical to the pre-cutover shape (the `tool_results: null` field present) (AC: contracts unchanged).

**Approach:**
- Author the golden tests; seed via the new stores.

**Done when:** `cargo test -p gateway --test api_contract` green.

### T16: Delete superseded code

**Depends on:** T11, T12, T13, T14, T15

**Touches:** `stores/zbot-stores-sqlite/src/{schema.rs,repository.rs}`, `stores/zbot-stores-domain/src/message.rs`, `stores/zbot-stores-traits/src/conversation.rs`, `services/execution-state/src/types.rs`, `gateway-execution/src/{archiver.rs,session_state.rs}`, `services/api-logs`, `gateway/src/state/mod.rs`

**Tests:**
- `grep` finds no `ConversationRepository`, legacy `Message` POD, `ConversationStore` trait, gz archiver, or replay loop (AC: no superseded symbols).

**Approach:**
- Remove in dependency order (consumers first). After each removal run `cargo check --workspace` — green proves no Phase-3 site was missed (don't paper over a lingering reference). Then: `zbot-stores-sqlite/schema.rs` messages+execution_logs DDL; `repository.rs` `ConversationRepository` + its methods; `zbot-stores-domain/message.rs` `Message`; `zbot-stores-traits/conversation.rs` `ConversationStore`; `execution-state/types.rs` `Checkpoint`; `archiver.rs` gz archiver; `session_state.rs` dead replay; `services/api-logs` truncation; `state/mod.rs` old `conversations` field + its 3 construction lines.

**Done when:** grep clean; `cargo test --workspace` green; `npm run build` green.

## Rollout

- **Delivery:** single clean cutover on `feat/conversation-store-revamp`. Old
  `conversations.db` deleted before Phase 3 (not prod — no data to preserve);
  new tables initialize empty. Reversible up to Phase 4 (deletion) — until then,
  the old code still compiles alongside. Irreversible step: T16's deletions
  (mitigated by branch isolation + per-task commits).
- **Infrastructure:** none beyond the new `traces/` directory under the vault
  data root (`gateway-services::paths.traces_dir`).
- **External-system integration:** `duckdb` (bundled) and `zstd` added to the
  build; duckdb's cross-compile is resolved at build time (isolated to
  `TraceAnalytics` / T8).
- **Deployment sequencing:** Phase 1–2 (crates) land first and compile
  standalone; Phase 3 flips consumers task-by-task with a daemon smoke each;
  Phase 4 deletes only after every consumer (T11–T15) is off the old code.

## Risks

- `duckdb` `bundled` cross-compile to Windows/macOS (builds from source) — T8
  smoke catches it; isolated behind `TraceAnalytics`.
- zstd frame-per-flush compresses less than one-frame-per-file — acceptable
  (still beats gzip); optional re-compress on close later.
- `messages` never-deleted → unbounded growth on a long-lived install; the
  existing `SessionArchiver` still handles old-session archival — flagged, not
  solved here.
- Two near-duplicate runner write loops remain after T11 (only their emit shape
  changes); consolidating them is a follow-up.

## Changelog

- 2026-07-07: initial plan (converted from the brainstorming/writing-plans draft into the canonical `new-spec`/`work-loop` format; content unchanged, structure + ACs + DAG added).
