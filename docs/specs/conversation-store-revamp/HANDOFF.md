# Conversation Store Revamp — Handoff (remaining: T13, T14, T15, T16)

**Branch:** `feat/conversation-store-revamp` · **HEAD:** `ed18a139` · **Date:** 2026-07-07
**Spec/Plan:** `docs/specs/conversation-store-revamp/{spec.md,plan.md}` (canonical, review-clean).

## TL;DR

The **bloat reduction + monitoring are done and daemon-verified**: `execution_logs.metadata` is slimmed (tool `args`/`result` removed; they live only in `messages` + `traces/*.jsonl.gz`), and Mission Control (`/api/sessions/:id/state`) renders fully — plan, response, recalled_facts, intent, subagents, and per-tool-call input/output — all sourced from `messages`. Both crates built + tested.

**What remains is the cutover tail:**
- **T13** — migrate the `ConversationRepository` *read* consumers (21 files still reference it).
- **T14** — add the `/api/traces/query` endpoint (DuckDB backend already built; route missing).
- **T15** — finish UI tests (`ToolsPane.tsx`, `ToolDetailPopover.test.tsx` assert the old metadata shape).
- **T16** — delete the dead code (**irreversible — gate on the user**).

## What's DONE + verified (don't redo)

- `stores/zbot-conversation/` — `MessageStore` (atomic server-side `seq`, `msg-<uuid>` ids), `CheckpointStore` (latest by `llm_turn`,`created_at`).
- `stores/zbot-trace/` — `SlimLogStore`, `TraceWriter` (`.jsonl.gz`, gzip-member-per-event, path-confined), `TraceAnalytics` (DuckDB `$1`-bound, 256-file cap). **gzip not zstd** — DuckDB reads gzip natively; zstd needs the `parquet` extension (network fetch).
- `AppState` holds `messages`/`checkpoints`/`slim_logs`/`trace_analytics` (one shared r2d2 pool; `traces_dir` in `VaultPaths` + `ensure_dirs_exist`).
- Message writes via `MessageStore::append`; turn-boundary checkpoints via `write_turn_checkpoint` (⚠️ vestigial — see gotchas).
- `event_logging` slimmed: `tool_call`→`{tool_id,tool_name}`, `tool_result`→`{tool_id,error,blocked_by_hook}`.
- `session_state` reads **messages** for response/plan/recalled_facts/tool-IO, **execution_logs** for intent/model, **sessions** for ward/title, child sessions for subagents.
- Tests: 19 crate + 482 gateway-execution lib + 14 session_state — all green.

Key commits: `ed18a139` (T13/T15 WIP checkpoint), `1a063faa` (tool IO from messages), `5a768da3` (sess/exec key fix), `c05206c2` (slim + messages-based session_state), `af6c502d` (trace streaming), `f663a394` (MessageStore writes + checkpoints), `40cb75d4`/`d269b74f`/`9efd1dea` (wiring/sink/analytics). A `context_state`-snapshot attempt was reverted (`5dc1edee`→`95a92c4b`) — **don't reintroduce snapshots for monitoring; messages is the source.**

## ⚠️ Critical gotchas (read first)

1. **`sess-*` vs `exec-*` keying.** `messages.session_id` = conversation_id (`sess-*`); `execution_logs.session_id` = execution_id (`exec-*`). `SessionDetail.session.session_id` is exec-; `.conversation_id` is sess-. **Message lookups use `conversation_id`; log lookups use exec id.** This broke two attempts — unit tests don't capture it.
2. **Unit tests passing ≠ real sessions working.** Delegation `system`-message plan format + keying only show up against real data. **Daemon smoke is the gate** (`curl localhost:18791/api/sessions/<id>/state` + `sqlite3`), not `cargo test`.
3. **rust-analyzer diagnostics are stale here** (phantom E0061/E0063). **`cargo` is authoritative.**
4. **Gateway port 18791** (`httpPort` in `~/Documents/zbot/config/settings.json`); no auth.
5. **`context_state` checkpoint is vestigial** — `write_turn_checkpoint` still writes it but `session_state` reads messages, not the checkpoint. Either drop the checkpoint writer or repurpose as an O(1) cache; don't make monitoring depend on it.

## What REMAINS — with exact file lists

### T13 — migrate `ConversationRepository` READ consumers (21 references)
These files still reference `ConversationRepository`. Group them:

**Concrete holders (`Arc<ConversationRepository>` field/param — swap to `Arc<dyn MessageStore>`):**
- `gateway-execution/src/invoke/executor.rs`, `runner/{continuation_watcher.rs,delegation_dispatcher.rs,core.rs,execution_stream.rs,invoke_bootstrap.rs}`, `delegation/{spawn.rs,callback.rs}`, `tools/wait_agent.rs`, `distillation.rs`, `src/services/runtime.rs`, `src/state/mod.rs`.

**Read consumers (call its methods):**
- `gateway-memory/src/sleep/{pattern_extractor.rs,worker.rs}` (use `tool_sequence_for_session` — now on `MessageStore`; ward/agent-id via `sessions`/`agent_executions`).
- `gateway-execution/src/sleep/handoff_writer.rs`.
- `gateway-execution/src/invoke/batch_writer.rs` (the `conversation_repo` fallback param — remove with `BatchWrite::SessionMessage`).

**Definitions (delete in T16, not T13):**
- `stores/zbot-stores-sqlite/src/{repository.rs,lib.rs}` (the struct + impl), `stores/zbot-stores-domain/src/message.rs` (legacy `Message` POD), `stores/zbot-stores-traits/src/conversation.rs` (`ConversationStore` trait).

**Test fixture:** `gateway-execution/tests/session_state_tests.rs` (references `ConversationRepository`).

**Notes:** `tool_sequence_for_session` already moved to `MessageStore`. `get_session_ward_id`/`get_session_agent_id` read `sessions`/`agent_executions` (stay in `zbot-stores-sqlite`). `session_state.rs` still uses `conversation_repo` for `extract_user_message`/`sum_token_count` — move those to `MessageStore::replay`. `chat.rs` (`GET /api/sessions/:id/messages`) was partly migrated in the WIP — confirm it uses `MessageStore`.

### T14 — `/api/traces/query` endpoint
- `TraceAnalytics` (`stores/zbot-trace/src/analytics.rs`) is built: `sessions_with_failed_tool(tool)` via DuckDB `$1`-bound `read_json_auto('traces/*.jsonl.gz')`, 256-file cap.
- **Missing:** the HTTP handler + route mount. Create `gateway/src/http/traces.rs` (`POST /api/traces/query { preset, params }` → `state.trace_analytics`; **preset enum `match`, 400 on unknown — no raw client SQL**), mount in `gateway/src/http/mod.rs`, extend `gateway/src/http/openapi.yaml` (hand-authored — no `api-contract` skill).
- `AppState.trace_analytics` already wired (T9).

### T15 — UI tests
- `apps/ui/src/features/mission-control/ToolsPane.tsx` + `ToolDetailPopover.test.tsx` assert the old `metadata.args`/`result` shape (now slimmed). Update to the new shape: tool-call IO comes via `ToolCallEntry.input`/`output` (from `messages`), not `execution_logs.metadata`.
- Add a gateway golden test: `GET /api/sessions/:id/messages` + `/api/sessions/:id/state` DTO field-sets stable.
- `useSessionTrace.test.ts` was updated in the WIP — confirm.

### T16 — delete dead code (IRREVERSIBLE — gate on user)
After T13 (zero consumers), delete: `ConversationRepository` (struct + impl + lib re-export), legacy `Message` POD (`zbot-stores-domain/src/message.rs`), `ConversationStore` trait (`zbot-stores-traits/src/conversation.rs`), gz archiver (`gateway-execution/src/archiver.rs`), `session_state.rs` replay branch (if any remains), `agent_executions.checkpoint` column + `AgentExecution.checkpoint` field + `save_execution_checkpoint`, `BatchWrite::SessionMessage` + the `conversation_repo` param, `spawn_batch_writer_with_repo`, the `AppState.conversations` field. Also slim `services/api-logs/src/service.rs` `LogService::log_tool_*` (test-only, still write args/result). Run `grep -rn ConversationRepository` to confirm zero hits before deleting.

**Deferred (backlog, not blocking):** `thread_summaries`/`SummaryStore` (compaction-without-loss) — anchor `conversation-store-revamp-summary-store` in `docs/backlog.md`.

## How to verify

```bash
npm run daemon:watch         # auto-rebuilds (watchexec ~2s); gateway on 18791, UI on :3000
curl -s localhost:18791/api/sessions/<sess-id>/state | python3 -m json.tool   # Mission Control detail
DB=~/Documents/zbot/data/conversations.db
sqlite3 $DB "SELECT category, metadata FROM execution_logs WHERE category IN ('tool_call','tool_result') LIMIT 4;"  # slimmed
sqlite3 $DB "SELECT seq, role FROM messages WHERE session_id='<sess-id>' ORDER BY seq;"
zcat ~/Documents/zbot/data/traces/<sess-id>.jsonl.gz | head
cargo check --workspace && cargo test -p gateway-execution --lib && cargo test -p gateway-execution --test session_state_tests
```

## Recommendation

Do **T13 first** (the bulk; mechanical `Arc<ConversationRepository>`→`Arc<dyn MessageStore>` swaps + read-call adaptations), verify on a **delegation-based** session via daemon, then **T14** (one contained endpoint), **T15** (tests), and finally **T16** with the user's explicit sign-off. Keep `cargo` as the source of truth (ignore stale rust-analyzer). Never make monitoring read a snapshot — `messages` is the source.
