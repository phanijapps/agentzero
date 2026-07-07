# Conversation Store Revamp — Handoff Report

**Branch:** `feat/conversation-store-revamp` (off the `work/context-capability-registry` WIP).
**Spec/Plan:** `docs/specs/conversation-store-revamp/{spec.md,plan.md}` (canonical, review-clean).
**State as of 2026-07-07:** Bloat reduction + monitoring are **working and daemon-verified**. The cutover is ~60% done; T13–T16 remain.

---

## 1. What this revamp is

`conversations.db` was bloated: every tool call's args lived in 2 places and results in 3 (dual-write to `messages` + `execution_logs`, with two different truncation policies), and there was no real state checkpoint (state was *reconstructed by replay* on every read).

The revamp splits the conversation+trace domain into two new self-contained crates with narrow traits (no god-classes), aligns to the cross-framework agentic consensus (append-only messages + mutable checkpoint + append-only trace), and moves full-fidelity trace payloads out of SQLite into streamed per-session `.jsonl.gz` files queryable by DuckDB.

## 2. Current behavior (what's live + verified)

- **`execution_logs.metadata` is slimmed** (the bloat reduction): `tool_call` → `{tool_id, tool_name}` (no `args`); `tool_result` → `{tool_id, error, blocked_by_hook}` (no `result`, no truncation). Intent/model/delegation logs are **un-slimmed** (monitoring reads them).
- **Full tool payloads live in `messages`** (via `MessageStore`) **and `traces/<session_id>.jsonl.gz`** (streamed live, DuckDB-queryable) — exactly two places, each serving a distinct consumer.
- **Messages** are written via `MessageStore::append` (atomic server-side `seq`, `msg-<uuid>` ids).
- **Turn-boundary checkpoints** are written (`write_turn_checkpoint`) — but see §6 (currently vestigial).
- **Mission Control (`/api/sessions/:id/state`) renders fully**: plan, response, recalled_facts, intent, ward, title, subagents, **and each tool call's input/output** — all sourced from `messages` (NOT from the slimmed metadata).

## 3. What's DONE + committed (~20 commits)

| Task | Status |
|---|---|
| T1–T3 `stores/zbot-conversation` (MessageStore w/ atomic seq, CheckpointStore) | ✅ done, tested |
| T5–T8 `stores/zbot-trace` (SlimLogStore, TraceWriter `.jsonl.gz`, TraceAnalytics DuckDB) | ✅ done, tested |
| T9 AppState wiring (4 store fields, one shared pool, `traces_dir`) | ✅ done |
| T10 BatchWriter trace sink (`TraceEvent`/`CloseSessionTrace`, `spawn_batch_writer_with_traces`) | ✅ done |
| T11 Slice 1: trace streaming live (`stream_event_processor` emits TraceEvents; 3 callers use `_with_traces`) | ✅ done, verified |
| T11 Slice 2: message writes via `MessageStore` + turn-boundary checkpoints | ✅ done, verified |
| T11 Slice 3 redo: slim metadata + messages-based `session_state` | ✅ done, verified |
| Fix: `messages.replay` by `conversation_id` (sess-), not `session_id` (exec-) | ✅ critical fix |
| Fix: `ToolCallEntry.input/output` sourced from messages | ✅ critical fix |

**Tests:** 19 crate tests (zbot-conversation + zbot-trace) + 482 gateway-execution lib + 14 session_state — all green. `cargo check --workspace` clean.

**A prior Slice 3 attempt was reverted** (commit `5dc1edee` → reverted by `95a92c4b`): it rerouted `session_state` to a `context_state` snapshot that was under-sourced for real (delegation-based) sessions. The redo reads **messages** instead (robust).

## 4. Key architectural decisions

- **Two crates, narrow traits, no facade.** `MessageStore`/`CheckpointStore`/`SlimLogStore` are traits; `TraceWriter`/`TraceAnalytics` are concrete structs. Consumers compose only what they need.
- **gzip, not zstd**, for trace files: DuckDB reads `.jsonl.gz` natively; zstd needs the `parquet` extension (network `INSTALL` — unsuitable for desktop). ~6% worse compression, accepted.
- **`TraceWriter` = one complete gzip member per `append`** (via `flate2::write::GzEncoder` + `finish`). Every append is durable immediately; a crash leaves prior members decodable (tolerant reader skips a truncated tail).
- **Path confinement:** `session_id` validated + `traces_dir` canonicalized before join (`docs/architecture/security.md` §Path Confinement).
- **Monitoring reads messages, not a snapshot.** This is the load-bearing decision after two failed snapshot attempts.
- **Clean cutover:** old `conversations.db` is disposable (not prod). The legacy `ConversationRepository` is retained for READS until T13.

## 5. ⚠️ Critical gotchas (read before touching anything)

1. **`sess-*` vs `exec-*` keying.** `messages.session_id` holds the **conversation_id** (`sess-*`); `execution_logs.session_id` holds the **execution_id** (`exec-*`). `SessionDetail.session.session_id` is the **exec-** id; `.conversation_id` is the **sess-** id. **Any message lookup must use `conversation_id`; any log lookup uses the exec id.** This bit two attempts — the unit-test fixtures don't capture it. Always verify against a real session.
2. **Subagent unit tests passing ≠ real sessions working.** The delegation `system`-message plan format + the sess/exec keying only surface against real data. **The daemon smoke is the real gate**, not `cargo test`. Verify with `curl localhost:18791/api/sessions/<id>/state` + `sqlite3`.
3. **rust-analyzer diagnostics are stale in this repo** (showed phantom `E0061`/`E0063` repeatedly). **`cargo` is authoritative** — run it, don't trust the IDE squiggles.
4. **Gateway runs on port 18791** (`httpPort` in `~/Documents/zbot/config/settings.json`). No auth (single-user desktop).

## 6. Known loose ends / follow-ups

- **`write_turn_checkpoint`'s `context_state` is now vestigial** — Slice 3 redo made `session_state` read from messages, not the checkpoint. The checkpoint is still written (Slice 2) but unused. Either drop it or repurpose as an O(1) perf cache later. `context_state` only has `ward`+`response` reliably populated; `llm_turn` = `handle.current_iteration()` (cumulative steps, not turn count).
- **`services/api-logs/src/service.rs` `LogService::log_tool_call`/`log_tool_result`** still write `args`/`result` (test-only path, no production callers — production goes through `gateway-execution/src/invoke/event_logging.rs`, which is slimmed). Slim or remove in T16.
- **`thread_summaries` / `SummaryStore` deferred** (compaction-without-loss) — backlog anchor `conversation-store-revamp-summary-store` in `docs/backlog.md`.
- **`spawn_batch_writer_with_repo`** is now unused (callers switched to `_with_traces`); remove in T16.

## 7. What REMAINS (the cutover tail)

- **T13 — move the `ConversationRepository` *read* consumers off it.** Sites: `gateway/src/http/chat.rs` (`GET /api/sessions/:id/messages`), runner replay (`core.rs:1262`, `invoke_bootstrap.rs:480`), `distillation.rs`, `sleep/handoff_writer.rs`, `sleep/pattern_extractor.rs`, `services/execution-state/src/repository.rs`. Plus the **~15 concrete `Arc<ConversationRepository>` holders** (executor.rs, continuation_watcher.rs, delegation_dispatcher.rs, wait_agent.rs, runtime.rs) — swap field types to `Arc<dyn MessageStore>`. Note `session_state.rs` still uses `conversation_repo` for `extract_user_message`/`sum_token_count` (T13). The `ConversationStore` trait (`stores/zbot-stores-traits/src/conversation.rs`) has 6 consumers using `tool_sequence_for_session`/`get_session_ward_id`/`get_session_agent_id` — `tool_sequence` is now on `MessageStore`; ward/agent-id lookups read `sessions`/`agent_executions`.
- **T14 — `/api/traces/query`** endpoint + extend `gateway/src/http/openapi.yaml`. `TraceAnalytics` already implements `sessions_with_failed_tool` (DuckDB `$1`-bound, 256-file cap). Hand-authored (no `api-contract` skill installed).
- **T15 — golden + UI tests.** Gateway golden test (DTO field-set unchanged). **UI tests need updating**: `apps/ui/src/features/mission-control/SessionDetailPane.test.tsx` + `apps/ui/src/features/logs/useSessionTrace.test.ts` assert the old `metadata.args`/`result` shape — now slimmed. The tool-call IO now comes via `ToolCallEntry.input/output` (from messages), so update those assertions.
- **T16 — delete dead code (IRREVERSIBLE — gate on user confirmation).** `ConversationRepository`, legacy `Message` POD (`stores/zbot-stores-domain/src/message.rs`), `ConversationStore` trait, gz archiver (`gateway-execution/src/archiver.rs`), `session_state.rs` replay branch, `agent_executions.checkpoint` column + `AgentExecution.checkpoint` field + `save_execution_checkpoint`, `BatchWrite::SessionMessage` + the `conversation_repo` param, `spawn_batch_writer_with_repo`. Run `grep -rn ConversationRepository` to confirm zero consumers before deleting.

## 8. How to verify

```bash
npm run daemon:watch         # auto-rebuilds on .rs changes (watchexec, ~2s poll)
# gateway on port 18791; UI on :3000

# Mission Control detail (the session-state endpoint):
curl -s localhost:18791/api/sessions/<session_id>/state | python3 -m json.tool

# DB checks:
DB=~/Documents/zbot/data/conversations.db
sqlite3 $DB "SELECT category, metadata FROM execution_logs WHERE category IN ('tool_call','tool_result') LIMIT 4;"  # slimmed
sqlite3 $DB "SELECT seq, role, substr(content,1,40) FROM messages WHERE session_id='<sess-id>' ORDER BY seq;"
ls -la ~/Documents/zbot/data/traces/    # .jsonl.gz files
zcat ~/Documents/zbot/data/traces/<sess-id>.jsonl.gz | head    # full-fidelity events

# Gates:
cargo check --workspace
cargo test -p zbot-conversation -p zbot-trace   # 19 crate tests
cargo test -p gateway-execution --lib            # 482
cargo test -p gateway-execution --test session_state_tests   # 14
```

## 9. Key files

- Crates: `stores/zbot-conversation/` (domain, schema, messages, checkpoints, pool), `stores/zbot-trace/` (domain, schema, slim_logs, writer, analytics, pool).
- Wiring: `gateway/src/state/mod.rs` (`build_conversation_stores`, AppState fields), `gateway/gateway-services/src/paths.rs` (`traces_dir`).
- Write path: `gateway-execution/src/invoke/{batch_writer.rs,event_logging.rs,stream_event_processor.rs}`, `runner/{execution_stream.rs,core.rs}`, `delegation/spawn.rs`.
- Monitoring: `gateway-execution/src/session_state.rs` (the messages-based extractors + `build_tool_calls`), `gateway/src/http/sessions.rs` (`get_session_state`).

## 10. Recommendation for the next agent

The hard part (bloat reduction without breaking monitoring) is **done and verified**. What's left (T13–T16) is mechanical cutover + deletion — lower-risk, but T13 touches many files. Do T13 first (read-consumer migration), verify via daemon on a **delegation-based** session, then T14/T15, then T16 with the user's explicit sign-off. Do **not** reintroduce a `context_state` snapshot for monitoring — messages is the source. Always verify against a real session, not just unit tests.
