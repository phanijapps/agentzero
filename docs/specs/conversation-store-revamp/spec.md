# Spec: Conversation Store Revamp

- **Status:** Draft
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none (independent greenfield crates + clean cutover)
- **Brief:** none
- **Contract:** [`gateway/src/http/openapi.yaml`](../../../gateway/src/http/openapi.yaml) (extended with `/api/traces/query`; hand-authored — the `api-contract` skill is not installed)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

The conversation store persists each agent session's messages, state, and
execution trace without duplication, returns the latest state in O(1), and keeps
the on-disk footprint of `conversations.db` bounded. A developer or operator of
the desktop agent can load any session's history through the existing UI without
change, resume a paused session from an explicit versioned checkpoint (rather
than a state reconstructed by replay), query "which sessions saw tool X error"
across the full trace history, and trust that tool payloads live in exactly one
hot store and one cold file — never duplicated across three SQLite columns. The
trace is streamed live to a per-session compressed file and is analytically
queryable; the live logs UI continues to read the same slim SQLite table it
always has.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.

### Always do

- Build each store as one narrow trait + one impl, ~100 LOC per file. Consumers
  compose only the traits they need at the call site.
- Keep `messages` strictly append-only — the write path never `UPDATE`s or
  `DELETE`s a message row. Compaction is additive (`thread_summaries`), never
  destructive.
- Preserve HTTP routes and response DTOs byte-for-byte. Dropping a DB column
  (`tool_results`) does not drop the wire field — the handler maps it to `None`.
- Open every new SQLite pool with the shared pragmas (WAL, `synchronous=NORMAL`,
  `busy_timeout=5000`, `foreign_keys=ON`).
- Commit per task; conventional-commit messages.

### Ask first

- Touching `sessions` or `agent_executions` (they stay in `zbot-stores-sqlite`;
  this spec moves only `messages`/`checkpoints`/`thread_summaries` + trace).
- Adding any dependency other than `duckdb` and `zstd`.
- Changing the orchestrator's committed plan/goal delivery
  (`feedback_orchestrator_context_high_stakes` — `thread_summaries` compacts
  message history only, never plan delivery).

### Never do

- No god-class facade. There is no `ConversationStore` or `TraceStore`
  mega-trait — only the narrow per-concern traits.
- No in-place migration of `zbot-stores-sqlite`. The new crates are built
  alongside; the old conversation/trace code is deleted only at cutover, never
  rewritten in place.
- No co-locating trace payloads back in `execution_logs.metadata`. Full tool
  args/results live in `messages` (replay) and `traces/*.jsonl.zst` (analytics)
  — never in the slim logs table.
- No full event-sourcing / state-as-projection (Temporal-style). State is a
  versioned snapshot, not a derived projection of an event log.
- No new top-level dependency beyond `duckdb` and `zstd`.

## Testing Strategy

- **MessageStore / CheckpointStore / SummaryStore / SlimLogStore correctness —
  TDD.** Append/replay/latest are compressible invariants; red-green-refactor per
  store. (AC: messages append-only; checkpoint O(1) latest; slim logs
  payload-free.)
- **TraceWriter append-safety + round-trip — TDD**, including a property test
  that a writer dropped mid-session leaves a valid, decodable `.jsonl.zst` up to
  the last flushed frame. (AC: crash-safe trace file.)
- **TraceAnalytics DuckDB queries — TDD.** Seed `.jsonl.zst` files via
  `TraceWriter`, assert `sessions_with_failed_tool` filters correctly.
  (AC: cross-session trace query.)
- **Schema initialization — goal-based check.** `cargo test -p <crate>` asserts
  the tables/indexes exist; no separate test file beyond the crate's own.
- **`session_state` replay→checkpoint switch — integration (TDD-flavored).**
  Assert `CheckpointStore::latest` equals the state a replay would have
  produced (compute both, diff empty). (AC: O(1) state, no replay.)
- **API contract preservation — integration.** Golden/snapshot tests assert
  `GET /api/sessions/:id/messages` and `/api/logs` response shapes are
  byte-identical pre/post cutover. (AC: contracts unchanged.)
- **Cutover + end-to-end — manual QA, exercised through the real daemon.** Run
  one conversation; observe messages rows (seq-ordered), a checkpoint per turn, a
  `.jsonl.zst` with full payloads, slim `execution_logs`, and the live logs UI
  rendering unchanged. (AC: end-to-end observed result.)

## Acceptance Criteria

- [ ] Tool args appear in exactly one SQLite column (`messages.tool_calls`) — never in `execution_logs.metadata`.
- [ ] Tool result text appears in `messages.content` and the session's `traces/<session_id>.jsonl.zst` — never in `execution_logs.metadata`.
- [ ] `execution_logs.metadata` carries only display scalars (`tool_name`, `error`) — no payload blobs, and the dual 500/1000-char truncation is gone.
- [ ] A session's state is returned by a single `checkpoints` lookup (O(1)), not by replaying `execution_logs` + `messages`.
- [ ] `GET /api/sessions/:id/messages` and the `/api/logs` response shapes are byte-identical before and after cutover (golden tests green).
- [ ] A `.jsonl.zst` belonging to a session killed mid-turn is valid and decodable up to the last flushed frame.
- [ ] `/api/traces/query` returns the set of sessions where a given tool errored (seeded test green); the endpoint is documented in `gateway/src/http/openapi.yaml`.
- [ ] `messages` is append-only: the write path issues no `UPDATE`/`DELETE` against it (compaction is via `thread_summaries`).
- [ ] No superseded symbols remain after cutover — `grep` finds no `ConversationRepository`, legacy `Message` POD, `ConversationStore` trait, gz archiver, or replay loop.
- [ ] `cargo check --workspace` and `cargo test --workspace` are green; `npm run build` is green.
- [ ] One real conversation run through the daemon produces: `messages` rows (seq-ordered), one `checkpoints` row per turn, a `traces/<id>.jsonl.zst` with full payloads, and slim `execution_logs` — observed end-to-end.

## Assumptions

- Technical: Rust 2021 workspace; members at `Cargo.toml:5-47` (source: Cargo.toml).
- Technical: `rusqlite` 0.32 bundled + `r2d2`/`r2d2_sqlite`, WAL pragmas (source: `stores/zbot-stores-sqlite/src/connection.rs:38-52`, `Cargo.toml:22-25`).
- Technical: `conversations.db` schema frozen at v22 (source: `stores/zbot-stores-sqlite/src/schema.rs:9`).
- Technical: a `Checkpoint` struct already exists at `services/execution-state/src/types.rs:667` — promoted, not invented (source: read).
- Technical: `AppState.conversations` is concrete `Arc<ConversationRepository>` (`gateway/src/state/mod.rs:57`); `memory_store: Arc<dyn …>` at `:82` is the trait-DI pattern to copy; 3 construction sites `:842/:942/:1093` (source: read).
- Technical: `gateway-execution/src/archiver.rs` already implements a gz JSONL cold tier (source: read).
- Technical: repo uses `contracts/<type>/` convention (`contracts/jsonschema/` in use); `gateway/src/http/openapi.yaml` exists and is the home for HTTP route contracts (source: `ls contracts/`, grep).
- Technical: `duckdb-rs` latest stable tracks DuckDB v1.5.x (crate ver `1.1050x`), rusqlite-inspired API, has a `bundled` feature (source: [crates.io](https://crates.io/crates/duckdb), [duckdb.org/docs/clients/rust](https://duckdb.org/docs/current/clients/rust.html)).
- Process: full-mode `work-loop`; specs live in `docs/specs/<feature>/` (source: skills loaded).
- Process: PRs never direct to `main` (source: user memory `feedback_pr_workflow`).
- Product: desktop app, single user, not prod; old `conversations.db` deleted, no migration/backfill (source: user confirmation 2026-07-07).
- Product: `duckdb-rs` accepted unconditionally — fresh build, cutover, resolve any cross-compile at build time (source: user confirmation 2026-07-07).
