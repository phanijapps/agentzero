# Plan: Conversation Store Revamp

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

> **Plan contract:** implementation strategy. Changes as we learn; substantial
> changes noted in the changelog at the bottom.

## Approach

Build two new self-contained crates alongside the old code, flip consumers to
them, then delete the dead old code. Four phases: (1) `stores/zbot-conversation`
— messages + checkpoints behind narrow traits (and rewire the legacy
`ConversationStore` consumers); (2) `stores/zbot-trace` — slim `execution_logs`
+ confined streamed `.jsonl.zst` + DuckDB analytics with `$1` binds; (3)
cutover — rewire `AppState`, `BatchWriter`, the dual write sites (incl. the
`context_state` checkpoint writer), `session_state`, and read sites; add
`/api/traces/query`; (4) delete all superseded code. Clean cutover (old
`conversations.db` deleted, not prod) — no migration/backfill. Riskiest part:
Phase 3 (live read/write paths); one task per commit + a daemon smoke after each.

`thread_summaries` / `SummaryStore` is **deferred** (spec §Deferred) — not in
this plan.

## Constraints

- Two new crates only: `stores/zbot-conversation`, `stores/zbot-trace`. New deps
  limited to `duckdb` (pinned minor), `zstd` (pinned minor). No god-class facades.
- `sessions`/`agent_executions`/`artifacts`/`distillation_runs`/`recall_log`/
  `bridge_outbox` stay in `zbot-stores-sqlite; only their *consumers* rewire.
- HTTP routes + DTO *field sets* frozen. `tool_results` wire field stays (`None`);
  `execution_logs.metadata` values intentionally shrink (AC#3).
- The new stores share **one** `r2d2` pool, constructed in the gateway from the
  `conversations.db` path (not 3 pools; not depending on `DatabaseManager`).
- `rusqlite = "0.32"` with `["bundled"]`; `Uuid` (keep `msg-` prefix for
  messages; `now_v7()` only for internal checkpoint/trace ids).
- Path confinement (spec AC#9) + parameterized SQL everywhere (spec AC#10).

## Construction tests

Per-task `Tests:` below. Cross-cutting:
- **Integration:** `context_state` round-trip (write at turn boundary, read via
  `latest`, assert == replay-derived `SessionState`). Dual-path golden test (old
  `ConversationRepository` vs new stores, same seed) asserting DTO field-set
  equality + the slim metadata shape.
- **Manual verification:** one conversation through the daemon end-to-end.

## Design (LLD)

### Design decisions

- Two crates, narrow traits, no facade. Rejected: mega-trait; in-place migration.
- Slim by writer-discipline (`execution_logs` columns unchanged; writers emit
  only `{tool_name, tool_id, error, blocked_by_hook}`). Rejected: file-only trace
  (`B2`); deferred DuckDB (`B3`).
- JSONL-zstd per session + DuckDB `$1`-bound reader. Rejected: live Parquet.
- Promote the existing `Checkpoint`; add a real `context_state` writer at turn
  boundaries; `session_state` switches replay→O(1). Rejected: parallel system;
  full event-sourcing.
- Clean cutover, delete old DB. Rejected: backfill; cutover-with-migration.
- `latest` checkpoint by `(llm_turn DESC, created_at DESC)` — not UUIDv7
  monotonicity (fragile within a millisecond).
- `seq` assigned atomically server-side inside the INSERT — no
  `next_seq`-then-`append` TOCTOU.
- Shared pool for the new stores — not per-crate duplication.

Traces to: ACs · `gateway/src/http/openapi.yaml`.

### Data & schema

`zbot-conversation` v1: `messages` (id `msg-<uuid>`, execution_id, session_id,
role, content, created_at, token_count, tool_calls, tool_call_id, **seq**) +
`idx_messages_session_seq`; `checkpoints` (id, execution_id, session_id,
llm_turn, last_message_id, pending_tool_calls, **context_state** JSON, child_executions,
schema_version, created_at) + `idx_checkpoints_exec_turn`.
(`thread_summaries` removed — deferred.)

`zbot-trace`: `execution_logs` — columns unchanged + existing indexes.

### Interfaces & contracts

`MessageStore { append(&self,&Message), replay(session_id, after_seq, limit),
tool_sequence_for_session(session_id) }` (`seq` is assigned atomically inside `append`; no separate `next_seq` API),
`CheckpointStore { write(&self,&Checkpoint), latest(execution_id) }`,
`SlimLogStore { append, query }`, `TraceWriter { open_confined, append, flush,
close }`, `TraceAnalytics { open, sessions_with_failed_tool(tool), query }`.
`AppState` holds `Arc<dyn …>` each (the `memory_store` pattern); the new stores
share one pool.

Public contract: `POST /api/traces/query { preset: enum<{sessions_with_failed_tool}>, params }`
→ JSON rows; preset matched by `match` (400 unknown); added to
`gateway/src/http/openapi.yaml` (hand-authored; `api-contract` skill absent — noted).

### Component / module decomposition

`stores/zbot-conversation/`: `domain.rs` (Message, Checkpoint), `pool.rs`,
`schema.rs`, `messages.rs`, `checkpoints.rs`, `lib.rs`.
`stores/zbot-trace/`: `domain.rs` (TraceEvent OTel-genai, SlimLog), `pool.rs`,
`schema.rs`, `slim_logs.rs`, `writer.rs`, `analytics.rs`, `lib.rs`.

### Behavior & rules

Payload routing:

| Event | `messages` | `execution_logs.metadata` | trace `.jsonl.zst` |
|---|---|---|---|
| User input | role=user (`msg-` id) | — | TraceEvent(session) |
| ToolCallStart | role=assistant w/ tool_calls | `{tool_name, tool_id}` | TraceEvent(tool_call, payload=args) |
| ToolResult | role=tool (full result) | `{tool_name, tool_id, error, blocked_by_hook}` | TraceEvent(tool_result, payload=result) |
| Turn boundary | — | — | `CheckpointStore::write(context_state)` |
| Delegation result | role=system | — | TraceEvent(delegation) |

`context_state` JSON schema, written at the **turn boundary** — the point where the assistant's final/respond turn completes (stream-end flush in `execution_stream.rs`, mirrored in `runner/core.rs:1400-1552`). Snapshot fields and their write-time source: `ward` ← `session.ward_id`; `plan` ← in-memory plan tracker; `intent` ← the `Intent` execution_log row / in-memory intent; `response` ← the respond tool call's args in `messages.tool_calls`; `title` ← session title; `model` ← model in use; `recalled_facts`/`subagents` ← in-memory runtime state. Message-derived fields (`user_message`, `token_count`) are **not** in the snapshot — T12 reads them via `MessageStore::replay`.

### Failure, edge cases & resilience

- `TraceWriter`: one zstd frame per flush → crash leaves a valid, decodable file
  to the last frame. Path confined (AC#9).
- `seq` atomic server-side → no concurrent-collision.
- Shared pool + WAL `busy_timeout` → cross-store writes serialized.
- DuckDB `$1` binds + 8 MB/line + file-count caps → no injection, no memory blowup.
- `duckdb` `bundled` cross-compile resolved at T8 (isolated behind `TraceAnalytics`).

### Dependencies & integration

New: `duckdb` (pinned minor, `bundled`), `zstd` (pinned minor). Reused:
`rusqlite` 0.32 bundled, `r2d2`/`r2d2_sqlite`, `serde`/`serde_json`,
`anyhow`/`thiserror`, `uuid`, `chrono`. New crates consume no `zbot-stores-sqlite` types.

## Tasks

### T1: `zbot-conversation` crate — domain + schema v1

**Depends on:** none

**Tests:** schema initializes `messages` + `checkpoints` + indexes (in-memory DB; goal-based).

**Approach:** add `"stores/zbot-conversation",` to workspace `members`. `Cargo.toml` (rusqlite 0.32 bundled, r2d2, r2d2_sqlite, serde, serde_json, anyhow, thiserror, uuid, chrono; dev-dep tempfile). `src/domain.rs` — `Message` (id, execution_id, session_id, role, content, created_at, token_count, tool_calls, tool_call_id, seq), `Checkpoint` (id, execution_id, session_id, llm_turn, last_message_id, pending_tool_calls, context_state, child_executions, schema_version, created_at). `src/schema.rs` — `messages` + `checkpoints` DDL (per Design §Data & schema) + `initialize`. `src/pool.rs` — `open_conversation_pool(path)`. `src/lib.rs` re-exports. `tests/schema.rs`.

**Done when:** `cargo test -p zbot-conversation --test schema` green; `cargo check --workspace` clean.

### T2: `MessageStore` (atomic seq, tool_sequence, `msg-` ids)

**Depends on:** T1

**Tests:** append→replay round-trip; `seq` is monotonic; **2×100 concurrent appends yield 200 distinct ordered seqs** (AC: atomic seq, append-only); `tool_sequence_for_session` parses `tool_calls` as the legacy method did.

**Approach:** `src/messages.rs` — `trait MessageStore: Send+Sync { fn append(&self,&Message)->Result<()>; fn replay(&self,session_id,after_seq:Option<i64>,limit)->Result<Vec<Message>>; fn tool_sequence_for_session(&self,session_id)->Result<Vec<ToolCallEntry>>; }` + `SqliteMessageStore { pool }`. `append` assigns `seq` **atomically** in one statement: `INSERT INTO messages (...,seq) VALUES (..., (SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE session_id=?))` — no separate `next_seq` call at the write site. `replay` = `WHERE session_id=? [AND seq>?] ORDER BY seq ASC LIMIT ?`. Message id `msg-<uuid>` preserved. Re-export; `tests/messages.rs` incl. `#[tokio::test]` concurrent property test.

**Done when:** `cargo test -p zbot-conversation --test messages` green (incl. concurrency).

### T3: `CheckpointStore` (latest by turn, context_state)

**Depends on:** T1

**Tests:** write 3 checkpoints (turns 1–3); `latest` returns turn 3 (AC: O(1) latest-wins).

**Approach:** `src/checkpoints.rs` — `trait CheckpointStore { write(&self,&Checkpoint); latest(&self,execution_id)->Option<Checkpoint>; }` + `SqliteCheckpointStore`. `latest` = `ORDER BY llm_turn DESC, created_at DESC LIMIT 1`. Re-export; `tests/checkpoints.rs`.

**Done when:** `cargo test -p zbot-conversation --test checkpoints` green.

### T4: Rewire `ConversationStore` consumers off the legacy trait

**Depends on:** T2

**Touches:** `gateway-memory/src/services.rs`, `gateway-memory/src/sleep/{pattern_extractor.rs,worker.rs}`, `gateway/src/services/runtime.rs`, `gateway/src/state/mod.rs`, `gateway-execution/src/sleep/handoff_writer.rs`

**Tests:** `cargo check --workspace` clean — no site references `dyn ConversationStore` (AC: precondition for T16).

**Approach:** the legacy `ConversationStore` trait (`stores/zbot-stores-traits/src/conversation.rs:33`) exposes `tool_sequence_for_session` (→ now `MessageStore`, T2), `get_session_ward_id`, `get_session_agent_id` (read `sessions`/`agent_executions`, which stay in `zbot-stores-sqlite`). Rewire each of the 6 sites: `tool_sequence_for_session` → `MessageStore`; ward/agent-id lookups → the existing `sessions`/`agent_executions` accessors (or a thin `SessionMetaStore` trait in `zbot-conversation` backed by the shared pool, if a trait is cleaner than direct access). Do **not** delete the trait yet (T16).

**Done when:** `cargo check --workspace` clean with no `dyn ConversationStore` consumer remaining.

### T5: `zbot-trace` crate — domain + schema

**Depends on:** none

**Tests:** schema initializes `execution_logs` + 3 indexes (goal-based).

**Approach:** add `"stores/zbot-trace",` to `members`. `Cargo.toml` (same as T1 + `zstd = "<pinned minor>"`, `duckdb = { version = "<pinned minor>", features = ["bundled"] }`, `tracing`). `src/domain.rs` — `SlimLog`, `TraceEvent` (OTel-genai fields, `skip_serializing_if Option::is_none`). `src/schema.rs` — `execution_logs` DDL (unchanged) + indexes. `src/pool.rs` — `open_trace_pool(path)`. `tests/schema.rs`. Pin `duckdb`/`zstd` to specific reviewed minors in `Cargo.lock`; run `cargo audit`/`cargo deny` (AC#12).

**Done when:** `cargo test -p zbot-trace --test schema` green; `cargo check --workspace` clean; audit green.

### T6: `SlimLogStore`

**Depends on:** T5

**Tests:** `append` stores a row whose `metadata` is within `{tool_name, tool_id, error, blocked_by_hook}` — asserts **no** `args`/`result` keys; `query` returns it (AC: slim logs).

**Approach:** `src/slim_logs.rs` — `trait SlimLogStore { append(&self,&SlimLog); query(&self,session_id,limit)->Vec<SlimLog>; }` + `SqliteSlimLogStore`. INSERT (params!); SELECT `ORDER BY timestamp`. Re-export; `tests/slim_logs.rs`.

**Done when:** `cargo test -p zbot-trace --test slim_logs` green.

### T7: `TraceWriter` (confined streaming `.jsonl.zst`)

**Depends on:** T5

**Tests:** append a,b,flush,append c,close → decodes to 3 lines; crash (drop without close after an unflushed append) → decodes to the flushed prefix only; **hostile `session_id` (`../x`, `a/b`, NUL, `C:\`) is rejected** (AC: confinement + crash-safety).

**Approach:** `src/writer.rs` — `TraceWriter::open_confined(traces_dir, session_id)`: validate `session_id` (UUID or reject `/`,`..`,NUL,drive-prefix), `let path = traces_dir.join(format!("{session_id}.jsonl.zst"))`, `canonicalize(traces_dir)` and assert `path.canonicalize()` starts_with it before open (per `docs/architecture/security.md` §Path Confinement). `append` writes JSON+`\n` into a `zstd::stream::write::Encoder`; `flush` flushes the frame; `close` finishes. `tests/writer.rs` + a `read_zstd_lines` helper.

**Done when:** `cargo test -p zbot-trace --test writer` green (all three).

### T8: `TraceAnalytics` (`$1`-bound DuckDB, caps)

**Depends on:** T5, T7

**Tests:** seed 2 `.jsonl.zst` via `TraceWriter` (one with a `tool_result` error for `read_file`); `sessions_with_failed_tool("read_file")` returns 1 session; **`traces_dir` with spaces works**; an **8 MB line is skipped+counted, not abort** (AC: cross-session query, DoS bound).

**Approach:** `src/analytics.rs` — `TraceAnalytics::open(traces_dir)` holding an in-memory `duckdb::Connection`; `sessions_with_failed_tool(tool)` uses a **prepared statement with `$1` bind** for `tool` (no `format!`). Set `read_json_auto` options to the form tests confirm; enforce per-line cap (8 MB) + per-query file-count cap (256), skip+count oversized. `tests/analytics.rs`. **De-risk gate:** `cargo build -p zbot-trace` must succeed (the one cross-compile risk).

**Done when:** tests green; `cargo build -p zbot-trace` succeeds; `cargo check --workspace` clean.

### T9: Wire stores into `AppState` (shared pool)

**Depends on:** T2, T3, T6, T7, T8

**Touches:** `gateway/Cargo.toml`, `gateway/src/state/mod.rs`, `gateway-services/src/paths.rs`

**Tests:** `cargo check -p gateway` clean (goal-based).

**Approach:** `gateway/Cargo.toml`: add `zbot-conversation`, `zbot-trace`. `state/mod.rs`: add `messages: Arc<dyn MessageStore>`, `checkpoints: Arc<dyn CheckpointStore>`, `slim_logs: Arc<dyn SlimLogStore>`, `trace_analytics: Arc<dyn TraceAnalytics>` (mirror `memory_store:82`). Construct **one** shared `Pool<SqliteConnectionManager>` from the conversations.db path in a `build_conversation_stores(paths)` helper; pass clones to the message/checkpoint/slim_logs stores. Add `traces_dir` to `gateway-services::paths` **and** `ensure_dirs_exist()` (AC#11). Keep old `conversations` field until T16.

**Done when:** `cargo check -p gateway` clean.

### T10: `BatchWriter` gains the `TraceWriter` sink

**Depends on:** T7, T9

**Touches:** `gateway-execution/src/invoke/batch_writer.rs`, `gateway-execution/src/lifecycle.rs`

**Tests:** 3 `TraceEvent`s for a session → close → `.jsonl.zst` has 3 lines (integration).

**Approach:** add `TraceEvent { session_id, event }` to the mpsc type; `HashMap<session_id, TraceWriter>` in the task; 100ms tick also flushes all open writers; `close_session_trace(session_id)` from session-end in `lifecycle.rs`. Writers opened via `TraceWriter::open_confined(traces_dir, session_id)`.

**Done when:** integration test green; `cargo check --workspace` clean.

### T11: Rewire dual write sites + `context_state` writer + retained metadata

**Depends on:** T9, T10

**Touches:** `gateway-execution/src/invoke/{event_logging.rs,stream_event_processor.rs}`, `gateway-execution/src/runner/{execution_stream.rs,core.rs}`, `gateway-execution/src/delegation/callback.rs`

**Tests:** after one conversation: `execution_logs.metadata` has **no** `args`/`result` (CLI assert); `.jsonl.zst` has them; a `checkpoints` row per turn with populated `context_state`; `GET /api/logs/sessions/:id` returns non-empty plan/ward (smoke after T11 gates T12).

**Approach:** `event_logging.rs` — `metadata` = retained key set only: `{tool_name, tool_id}` for tool_call, `{tool_name, tool_id, error, blocked_by_hook}` for tool_result; delete the 500/1000-char truncation; keep signatures. `stream_event_processor.rs` — alongside each `log_*`, push a `TraceEvent` (full payload). `execution_stream.rs`/`core.rs:1400-1552` — write messages via `MessageStore::append` (seq auto-assigned; `msg-` id); at the **turn boundary** (assistant final/respond turn completes — the stream-end flush in `execution_stream.rs`, mirrored in `core.rs:1400-1552`) call `CheckpointStore::write` with `context_state` per the schema + sources in Design §Behavior & rules. `delegation/callback.rs:238` — system message via `MessageStore` + delegation TraceEvent. Delete `conversations.db`, run one conversation, verify.

**Done when:** post-conversation assertions hold; `cargo check --workspace` clean.

### T12: `session_state` — replay → `CheckpointStore::latest`

**Depends on:** T3, T11

**Touches:** `gateway-execution/src/session_state.rs`

**Tests:** seed a checkpoint (via T11's writer); `GET /api/logs/sessions/:id` returns its plan/ward/response; equivalence to an independent replay (AC: O(1) state).

**Approach:** replace the replay branch of `SessionStateBuilder::build` with `checkpoints.latest(execution_id)`; deserialize `context_state` into the `SessionState` fields. Keep the returned JSON shape (UI contract). Delete the now-dead `extract_*` helpers that read `metadata.args/result`.

**Done when:** test green; response shape's field set unchanged.

### T13: Eliminate all `ConversationRepository` consumers (concrete holders + trait + read sites)

**Depends on:** T2, T4, T11

**Touches:** `gateway/src/services/runtime.rs`, `gateway/gateway-execution/src/tools/wait_agent.rs`, `gateway-execution/src/runner/{continuation_watcher.rs,delegation_dispatcher.rs,core.rs,invoke_bootstrap.rs}`, `gateway-execution/src/invoke/executor.rs`, `gateway-execution/src/{distillation.rs,session_state.rs}`, `gateway-execution/src/sleep/{handoff_writer.rs,pattern_extractor.rs}`, `gateway/src/http/chat.rs`, `services/execution-state/src/repository.rs`

**Tests:** `grep -rn 'ConversationRepository'` finds only the struct/repo definitions (deleted in T16) — no consumer; `GET /api/sessions/:id/messages` returns rows via `MessageStore::replay`; `tool_results` wire field `None`; `extract_user_message`/`sum_token_count` (`session_state.rs`) read via `MessageStore::replay`.

**Approach:** swap every concrete `Arc<ConversationRepository>` field/param/setter for `Arc<dyn MessageStore>` and adapt method calls (`get_session_conversation`/`get_messages` → `MessageStore::replay`; `tool_sequence_for_session` is now on `MessageStore`, T2). Sites: `runtime.rs:60,104,464`; `wait_agent.rs:13,20,81`; `continuation_watcher.rs:61,106`; `delegation_dispatcher.rs:242,302`; `executor.rs:721,867,1321,1802` (field + `with_conversation_repo` setter + use); `distillation.rs:43,212,362,737,768,1131`; `chat.rs:185`; `core.rs:1262` + `invoke_bootstrap.rs:480`; `handoff_writer.rs`/`pattern_extractor.rs`; `session_state.rs:137,144,266-292` (`extract_user_message`/`sum_token_count` → `MessageStore::replay`). **`execution-state/repository.rs:1193,1403,1414`**: drop the `tool_results` SELECT/bind/parse; map to `None`.

**Done when:** `grep -rn ConversationRepository` finds no consumer (only the soon-deleted definition); UI history loads; `cargo check --workspace` clean.

### T14: `/api/traces/query` (preset enum + `$1` binds + openapi)

**Depends on:** T8, T9

**Touches:** `gateway/src/http/traces.rs` (new), `gateway/src/http/mod.rs`, `gateway/src/http/openapi.yaml`

**Tests:** seed `.jsonl.zst`; `POST /api/traces/query {preset:"sessions_with_failed_tool", params:{tool:"read_file"}}` returns the sessions; unknown preset → 400 (AC: query + injection-safe).

**Approach:** `traces.rs` — `POST /api/traces/query { preset: Preset, params: Params }`; `match preset` (fixed enum, 400 on unknown); only the matched arm's typed params reach `TraceAnalytics` (`$1`-bound). Mount; extend `openapi.yaml` (hand-authored, note skill absent).

**Done when:** test green; openapi carries the path; `npm run build` unaffected.

### T15: API golden tests + UI test updates

**Depends on:** T13, T14

**Touches:** `gateway/tests/api_contract.rs` (new), `apps/ui/src/features/mission-control/SessionDetailPane.test.tsx`, `apps/ui/src/features/logs/useSessionTrace.test.ts`

**Tests:** dual-path test — old `ConversationRepository` vs new stores against mirrored fixtures assert DTO **field-set** equality (not byte-identical) + slim metadata shape; UI tests updated to assert the retained key set (AC: field set unchanged; UI updated).

**Approach:** author the gateway golden test (drive both paths, diff field sets). Update the two UI tests to assert `{tool_name, tool_id, error, blocked_by_hook}` instead of `args`/`result`.

**Done when:** `cargo test -p gateway --test api_contract` green; `npm run build` green.

### T16: Delete superseded code

**Depends on:** T4, T11, T12, T13, T14, T15

**Touches:** `stores/zbot-stores-sqlite/src/{schema.rs,repository.rs}`, `stores/zbot-stores-traits/src/conversation.rs`, `stores/zbot-stores-domain/src/message.rs`, `services/execution-state/src/{types.rs,repository.rs,service.rs}`, `gateway-execution/src/{archiver.rs,session_state.rs}`, `gateway-execution/src/invoke/batch_writer.rs`, `services/api-logs`, `gateway/src/state/mod.rs`

**Tests:** `grep` finds none of: `ConversationRepository`, legacy `Message` POD, `ConversationStore` trait, gz archiver, replay branch, `agent_executions.checkpoint` column + `AgentExecution.checkpoint` field + `save_execution_checkpoint`, `BatchWrite::SessionMessage` + `conversation_repo` param (AC: no superseded symbols).

**Approach:** remove in dependency order (consumers first — T4 already off the trait); `cargo check --workspace` after each (green proves no missed site). Delete: `schema.rs` messages+execution_logs DDL; `repository.rs` `ConversationRepository` + methods; `zbot-stores-traits/conversation.rs` `ConversationStore`; `zbot-stores-domain/message.rs` `Message`; `execution-state/types.rs` `Checkpoint` + the `agent_executions.checkpoint` column and **all** its touch-sites — CREATE TABLE `service.rs:893`, column def `schema.rs:436`, SELECT/INSERT/parse at `repository.rs:344,758,774,797,817,884,981,1024,1286,1300`, `AgentExecution.checkpoint` field, and `save_execution_checkpoint` (`repository.rs:981`/`service.rs:573`). T13 already removed every consumer, so these are orphaned definitions — delete in one coordinated commit; `archiver.rs` (retires the unconfined `<session_id>.jsonl.gz` path — security delta); `session_state.rs` replay branch; `batch_writer.rs` `BatchWrite::SessionMessage` + `conversation_repo` param + flush branch; `services/api-logs` truncation; `state/mod.rs` old `conversations` field + 3 construction lines.

**Done when:** grep clean; `cargo test --workspace` green; `npm run build` green.

## Rollout

- **Delivery:** single clean cutover on `feat/conversation-store-revamp`. Old
  `conversations.db` deleted before Phase 3 (not prod). Reversible up to T16
  (deletion); per-task commits mitigate. Irreversible: T16 deletions — note T16
  retires the existing unconfined `archiver.rs:142` filename-construction path
  (security benefit, replaced by the confined writer).
- **Infrastructure:** `traces/` under the vault data root, via
  `VaultPaths::ensure_dirs_exist()`.
- **External-system integration:** `duckdb` (bundled, pinned) + `zstd` (pinned);
  cross-compile resolved at T8.
- **Deployment sequencing:** Phase 1–2 compile standalone; Phase 3 flips
  consumers task-by-task with a daemon smoke; Phase 4 deletes only after T4 +
  T11–T15 are off the old code.

## Risks

- `duckdb` `bundled` cross-compile (T8 smoke; isolated behind `TraceAnalytics`).
- zstd frame-per-flush compresses less than one-frame-per-file (acceptable;
  optional re-compress on close later).
- `messages` never-deleted → unbounded growth; existing `SessionArchiver` still
  handles archival (now of slim rows) — flagged, not solved.
- Two near-duplicate runner write loops remain after T11 (emit shape changed
  only); consolidation is a follow-up.
- Logs/mission-control detail UI shows less inline (args/result gone) — by design
  (AC#3/#5); detail via trace file + `/api/traces/query`.

## Changelog

- 2026-07-07: initial plan (canonical new-spec format).
- 2026-07-07: pre-EXECUTE review revisions — deferred `thread_summaries`/`SummaryStore` (was T4); added T4 (rewire `ConversationStore` consumers); atomic `seq` + concurrency test (T2); `context_state` writer at turn boundary (T11) + schema; `latest` by `(llm_turn,created_at)` (T3); trace path confinement (T7); DuckDB `$1` binds + caps (T8); shared pool (T9); retained metadata key set (T11); `execution-state` `tool_results` fix (T13); preset enum + binds (T14); dual-path golden + UI test updates (T15); complete deletion list incl. checkpoint column/field/method + `BatchWrite::SessionMessage` (T16); new ACs for confinement/injection/seq/traces_dir/pinning/caps; resolved AC#3↔#5 contradiction.
