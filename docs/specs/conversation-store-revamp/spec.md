# Spec: Conversation Store Revamp

- **Status:** Draft
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`docs/architecture/security.md`](../../architecture/security.md) §Path Confinement, §Engram Dependency Gate
- **Brief:** none
- **Contract:** [`gateway/src/http/openapi.yaml`](../../../gateway/src/http/openapi.yaml) (extended with `/api/traces/query`; hand-authored — the `api-contract` skill is not installed; deviation from CONVENTIONS §4 noted, tracked as a Nit)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

The conversation store persists each agent session's messages, state, and
execution trace without duplication, returns the latest state in O(1), and keeps
the on-disk footprint of `conversations.db` bounded. A developer or operator of
the desktop agent can load any session's history through the existing UI routes,
resume a paused session from an explicit versioned checkpoint written at each
turn boundary (rather than a state reconstructed by replay), query "which
sessions saw tool X error" across the full trace history, and trust that tool
payloads live in exactly one hot store and one cold file — never duplicated
across three SQLite columns. The trace is streamed live to a per-session
compressed file confined under the vault root and is analytically queryable; the
live logs route continues to serve the same DTO fields, with inline metadata
values intentionally slimmed (full detail lives in the trace file and the new
analytics endpoint).

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.

### Always do

- Build each store as one narrow trait + one impl, ~100 LOC per file. Consumers
  compose only the traits they need at the call site.
- Keep `messages` strictly append-only — the write path never `UPDATE`s or
  `DELETE`s a message row.
- Preserve HTTP routes and the DTO *field set*. Dropping a DB column
  (`tool_results`) keeps the wire field (mapped to `None`); slimming
  `execution_logs.metadata` values is intentional and documented.
- Open SQLite with the shared pragmas (WAL, `synchronous=NORMAL`,
  `busy_timeout=5000`, `foreign_keys=ON`); the new stores share **one** pool
  constructed in the gateway.
- Confine every trace-file path under `VaultPaths::traces_dir()` — validate
  `session_id`, canonicalize before open, reject escapes (per
  `docs/architecture/security.md` §Path Confinement).
- Parameterize all SQL in the new crates — `rusqlite::params!` for SQLite,
  DuckDB `$1` binds for analytics. No `format!`/concat SQL.
- Commit per task; conventional-commit messages.

### Ask first

- Touching `sessions` or `agent_executions` rows/DDL (they stay in
  `zbot-stores-sqlite`; only their *consumers* are rewired).
- Adding any dependency other than `duckdb` and `zstd`.
- Changing the orchestrator's committed plan/goal delivery.

### Never do

- No god-class facade. There is no `ConversationStore` or `TraceStore`
  mega-trait — only the narrow per-concern traits.
- No in-place migration of `zbot-stores-sqlite`. The new crates are built
  alongside; the old conversation/trace code is deleted only at cutover.
- No co-locating trace payloads in `execution_logs.metadata`. Full tool
  args/results live in `messages` (replay) and `traces/*.jsonl.zst` (analytics).
- No full event-sourcing / state-as-projection. State is a versioned snapshot.
- No new top-level dependency beyond `duckdb` and `zstd`.
- No `format!`/string-concatenated SQL anywhere in `zbot-conversation` or
  `zbot-trace`.

## Testing Strategy

- **MessageStore / CheckpointStore / SlimLogStore correctness — TDD.** Append,
  replay, latest are compressible invariants. Includes a concurrent-append test
  for `seq` atomicity (AC: atomic seq, append-only).
- **TraceWriter append-safety + round-trip — TDD**, including the crash-leaves-
  valid-file property and a path-confinement test (hostile `session_id` rejected).
- **TraceAnalytics — TDD.** `$1`-bound queries; seed files via `TraceWriter`;
  path-with-spaces and oversized-line tests.
- **`context_state` round-trip — integration.** Write a checkpoint at a turn
  boundary, read it back via `CheckpointStore::latest`, assert the
  `SessionState` fields equal what a replay would have produced.
- **Schema init — goal-based.** `cargo test -p <crate>` asserts tables/indexes.
- **API contracts — integration.** `/api/sessions/:id/messages` and `/api/logs`
  DTO *field sets* unchanged; metadata values intentionally shrink — proven by a
  dual-path golden test (old `ConversationRepository` vs new stores, same seed)
  that asserts field-set equality, plus updated UI tests for the slim shape.
- **Cutover + end-to-end — manual QA** through the daemon: one conversation;
  observe messages (seq-ordered, `msg-` ids), a populated checkpoint per turn,
  `.jsonl.zst` with full payloads, slim `execution_logs`, logs route serving.

## Acceptance Criteria

- [ ] Tool args appear in exactly one SQLite column (`messages.tool_calls`) — never in `execution_logs.metadata`.
- [ ] Tool result text appears in `messages.content` and the session's `traces/<session_id>.jsonl.zst` — never in `execution_logs.metadata`.
- [ ] `execution_logs.metadata` carries only the retained display key set `{tool_name, tool_id, error, blocked_by_hook}` — no `args`/`result` payloads; the dual 500/1000-char truncation is gone.
- [ ] The state *snapshot* (`intent, ward, plan, recalled_facts, response, title, model, subagents`) is returned by a single `checkpoints` lookup — no replay of `execution_logs.metadata.args/result`; `context_state` is **written** at each turn boundary. Message-derived fields (`user_message`, `token_count`) read via `MessageStore::replay`; session meta and child-session enumeration still read via `log_service` (so assembly is 1 + N lookups for N subagents, not strictly O(1)).
- [ ] `GET /api/sessions/:id/messages` and `/api/logs` DTO **field sets** are unchanged; `tool_results` wire field is present (value `None`); `metadata` values intentionally shrink per AC#3 — UI tests updated to the slim shape.
- [ ] `messages.id` keeps the existing `msg-<uuid>` wire shape (no bare-UUID change on the message route).
- [ ] `messages.seq` is assigned atomically (server-side, no `next_seq`-then-`append` TOCTOU); a 2×100 concurrent-append test yields 200 distinct, ordered seqs.
- [ ] A `.jsonl.zst` for a session killed mid-turn is valid and decodable up to the last flushed frame.
- [ ] Trace paths are confined: `session_id` is validated (UUID, or rejected for `/`, `..`, NUL, drive-prefix), joined under `VaultPaths::traces_dir()`, canonicalized before open, escapes rejected (`docs/architecture/security.md` §Path Confinement).
- [ ] `POST /api/traces/query` accepts `{ preset: enum, params }`, matches `preset` against a fixed `match` (400 on unknown), and `TraceAnalytics` parameterizes all filters via DuckDB `$1` binds — no `format!`/concat SQL in `zbot-trace` (grep-enforced).
- [ ] `VaultPaths::traces_dir()` returns `data_dir.join("traces")` and is a member of `ensure_dirs_exist()`; the trace writer never creates the directory itself.
- [ ] `duckdb` and `zstd` are pinned to a reviewed minor in `Cargo.lock`; `cargo audit` (or `cargo deny`) is green for both before merge.
- [ ] The JSONL reader enforces a per-line decode cap (8 MB) and a per-query file-count cap (256); oversized lines are skipped with a counter, not abort.
- [ ] No superseded symbols remain: `ConversationRepository`, legacy `Message` POD, `ConversationStore` trait, gz archiver, `session_state` replay branch, `agent_executions.checkpoint` column + `AgentExecution.checkpoint` field + `save_execution_checkpoint`, `BatchWrite::SessionMessage` + the `conversation_repo` param (grep clean).
- [ ] `cargo check --workspace` and `cargo test --workspace` green; `npm run build` green (UI tests updated).
- [ ] One real conversation through the daemon produces: `messages` rows (seq-ordered, `msg-` ids), one `checkpoints` row per turn with populated `context_state`, a `traces/<id>.jsonl.zst` with full payloads, slim `execution_logs` — observed end-to-end.

## Deferred

- [ ] `thread_summaries` / `SummaryStore` (compaction-without-loss context-window assembly) — `(deferred: conversation-store-revamp-summary-store)`. This spec ships the immutable message log + checkpoint; the derived-summary writer/consumer is a follow-up.

## Assumptions

- Technical: Rust 2021 workspace; members at `Cargo.toml:5-47` (source: Cargo.toml).
- Technical: `rusqlite` 0.32 bundled + `r2d2`/`r2d2_sqlite`, WAL pragmas (source: `stores/zbot-stores-sqlite/src/connection.rs:38-52`).
- Technical: `conversations.db` schema frozen at v22 (source: `stores/zbot-stores-sqlite/src/schema.rs:9`).
- Technical: a `Checkpoint` struct already exists at `services/execution-state/src/types.rs:667` — promoted, not invented (source: read); its `context_state` field is currently `Value::Null` (populated only by a test) — this spec adds the writer.
- Technical: `AppState.conversations` is concrete `Arc<ConversationRepository>` (field at `gateway/src/state/mod.rs:57`); `memory_store: Arc<dyn …>` at `:82` is the trait-DI pattern to copy; the concrete repo is constructed at `:237/:396/:894/:2134` and threaded through `executor.rs`, `continuation_watcher.rs`, `delegation_dispatcher.rs`, `wait_agent.rs`, `distillation.rs`, `runtime.rs` (source: read).
- Technical: `ConversationStore` trait (`stores/zbot-stores-traits/src/conversation.rs:33`) is consumed by 6 sites for `ward_id`/`agent_id`/`tool_sequence_for_session` (source: adversarial-reviewer finding, grep-confirmed) — rewired in this spec.
- Technical: `gateway-execution/src/archiver.rs:142` constructs `<session_id>.jsonl.gz` without confinement — T16's deletion retires this vector (source: read).
- Technical: `messages.id` is `format!("msg-{}", Uuid::new_v4())` today (`stores/zbot-stores-sqlite/src/repository.rs:56,148`) — preserved (source: read).
- Technical: repo uses `contracts/<type>/` convention; `gateway/src/http/openapi.yaml` is the HTTP contract home; no auth middleware (single-user desktop) (source: `gateway/src/http/mod.rs`, `openapi.yaml:15-16`).
- Technical: `duckdb-rs` tracks DuckDB v1.5.x (source: [crates.io](https://crates.io/crates/duckdb)); exact version pinned in `Cargo.lock` at T5.
- Process: full-mode `work-loop`; specs in `docs/specs/<feature>/` (source: skills).
- Process: PRs never direct to `main` (source: user memory `feedback_pr_workflow`).
- Product: desktop app, single user, not prod; old `conversations.db` deleted, no migration/backfill (source: user confirmation 2026-07-07).
- Product: `duckdb-rs` accepted unconditionally — fresh build, cutover (source: user confirmation 2026-07-07).
