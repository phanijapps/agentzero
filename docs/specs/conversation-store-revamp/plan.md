# Conversation Store Revamp — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the bloated `conversations.db` conversation/trace layer with two new self-contained crates (`zbot-conversation`, `zbot-trace`) using narrow traits, a real versioned checkpoint, streamed `.jsonl.zst` traces + DuckDB analytics — then cutover and delete all superseded code.

**Architecture:** Build two crates alongside the old code (Phase 1–2), flip consumers to them (Phase 3), then delete the dead old code (Phase 4). Clean cutover — the old `conversations.db` is deleted (not prod); no migration/backfill. Each consumer reads exactly one store (dedup principle, spec §4).

**Tech Stack:** Rust 2021 · `rusqlite` 0.32 (bundled) + `r2d2`/`r2d2_sqlite` (WAL) · `serde`/`serde_json` · `zstd` (trace files) · `duckdb` (analytics) · `uuid` v7 (monotonic ids) · `anyhow`/`thiserror`. Tests via `cargo test`; UI unchanged.

**Spec:** [`spec.md`](./spec.md) — read it first. Every task traces to a spec section.

## Global Constraints

- **Don't migrate existing code in place.** New crates are built clean alongside; old code is touched only at cutover (read-site/write-site rewiring) and then deleted (Phase 4). No refactors of unrelated code.
- **No god classes.** One narrow trait + one impl per store (~100 LOC). No fat `ConversationStore`/`TraceStore` facade. Consumers compose only the traits they need.
- **Connection model:** each new crate opens its **own** `r2d2` pool to `conversations.db` (WAL, `busy_timeout=5000`, `synchronous=NORMAL`, `foreign_keys=ON`) — mirrors `DatabaseManager` pragmas (`stores/zbot-stores-sqlite/src/connection.rs:38-52`) but does **not** depend on `DatabaseManager`.
- **rusqlite version pinned to `0.32` with `["bundled"]`** in both new crates (unified with the workspace; one sqlite link).
- **Id generation:** `Uuid::now_v7()` (monotonic) for `messages.id`, `checkpoints.id`, `thread_summaries.id`. Add `features = ["v7"]` to the workspace `uuid` dep in each new crate.
- **API contracts frozen:** HTTP routes + response DTOs (`SessionMessageResponse` etc.) unchanged. Drop the `messages.tool_results` DB column but keep the DTO field mapped to `None`.
- **Per-phase gates:** `cargo check --workspace` after Rust changes; `cargo test -p <crate>` for the touched crate; `npm run build` after any TS touch (UI should be untouched, but verify).
- **Frequent commits:** one commit per task (or per step where noted), conventional-commit messages, end with `Co-Authored-By: Claude <noreply@anthropic.com>`.

---

## File Structure

```
stores/zbot-conversation/                 (NEW — Phase 1)
  Cargo.toml
  src/lib.rs              re-exports
  src/domain.rs           POD: Message, Checkpoint, ThreadSummary
  src/pool.rs             open_conversation_pool(path) -> Pool   (shared pragmas)
  src/schema.rs           v1 DDL: messages, checkpoints, thread_summaries
  src/messages.rs         trait MessageStore + SqliteMessageStore
  src/checkpoints.rs      trait CheckpointStore + SqliteCheckpointStore
  src/summaries.rs        trait SummaryStore + SqliteSummaryStore
  tests/messages.rs  tests/checkpoints.rs  tests/summaries.rs  tests/schema.rs

stores/zbot-trace/                        (NEW — Phase 2)
  Cargo.toml
  src/lib.rs
  src/domain.rs           POD: TraceEvent (OTel-genai), SlimLog
  src/pool.rs             open_trace_pool(path) -> Pool
  src/schema.rs           execution_logs DDL (columns unchanged from today)
  src/slim_logs.rs        trait SlimLogStore + SqliteSlimLogStore
  src/writer.rs           TraceWriter — per-session .jsonl.zst streaming append
  src/analytics.rs        TraceAnalytics — duckdb read_json_auto over traces/
  tests/slim_logs.rs  tests/writer.rs  tests/analytics.rs

# Phase 3 (cutover — modify existing):
gateway/src/state/mod.rs                  AppState: add trait-object store fields + 3 construction sites
gateway/gateway-execution/src/invoke/batch_writer.rs   +TraceWriter sink, +TraceEvent kind
gateway/gateway-execution/src/runner/execution_stream.rs   payload routing
gateway/gateway-execution/src/runner/core.rs:1400-1552    payload routing (resume path)
gateway/gateway-execution/src/session_state.rs   replay -> CheckpointStore::latest
gateway/src/http/chat.rs                  MessageStore::replay + DTO adapter
gateway/src/http/mod.rs                   mount /api/traces/query
gateway/src/http/traces.rs (NEW)          /api/traces/query handler
(+ distillation.rs, handoff_writer.rs, pattern_extractor.rs, execution-state repo — read-site swaps)

# Phase 4 (delete):
stores/zbot-stores-sqlite/src/{schema.rs,repository.rs}   messages/execution_logs DDL + ConversationRepository
stores/zbot-stores-domain/src/message.rs                  Message POD (moved to zbot-conversation)
stores/zbot-stores-traits/src/conversation.rs             ConversationStore trait
services/execution-state/src/types.rs                     Checkpoint struct (moved)
gateway-execution/src/{archiver.rs,session_state.rs}      gz archiver + replay loop
services/api-logs truncation logic
```

---

# Phase 1 — `stores/zbot-conversation/` crate

## Task 1: Crate scaffold + domain POD types + schema v1

**Files:**
- Create: `stores/zbot-conversation/Cargo.toml`
- Create: `stores/zbot-conversation/src/lib.rs`
- Create: `stores/zbot-conversation/src/domain.rs`
- Create: `stores/zbot-conversation/src/schema.rs`
- Create: `stores/zbot-conversation/src/pool.rs`
- Create: `stores/zbot-conversation/tests/schema.rs`
- Modify: `Cargo.toml` (add `"stores/zbot-conversation",` to `[workspace].members`, alphabetically after `zbot`)

**Interfaces:**
- Produces: `zbot_conversation::{Message, Checkpoint, ThreadSummary}` POD structs; `schema::SCHEMA_SQL` + `schema::initialize(conn)`; `pool::open_conversation_pool(path)`.

- [ ] **Step 1: Register the crate in the workspace**

Modify `Cargo.toml` `[workspace].members` — add the line:
```toml
    "stores/zbot-conversation",
```
(insert among the `stores/*` entries, before `"stores/zbot-engram-adapter",`.)

- [ ] **Step 2: Create `Cargo.toml`**

```toml
[package]
name = "zbot-conversation"
version = "0.1.0"
edition = "2021"
license.workspace = true

[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
r2d2 = "0.8"
r2d2_sqlite = "0.25"
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true, features = ["v7"] }
chrono = { workspace = true }

[dev-dependencies]
tempfile = "3"

[lints]
workspace = true
```

- [ ] **Step 3: Create `src/domain.rs`** (POD types; mirror existing field names)

```rust
use serde::{Deserialize, Serialize};

/// A single conversational message (append-only). Mirrors the role/tool shape
/// the LLM replay path needs. `tool_results` is intentionally absent (legacy col dropped).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,            // UUIDv7 (monotonic)
    pub execution_id: Option<String>,
    pub session_id: String,
    pub role: String,          // user | assistant | tool | system
    pub content: String,
    pub created_at: String,    // RFC3339
    pub token_count: i64,
    pub tool_calls: Option<String>,   // JSON array, assistant turns only
    pub tool_call_id: Option<String>, // links role=tool rows back
    pub seq: i64,              // per-session monotonic
}

/// Versioned state checkpoint. Promotes the legacy `Checkpoint` struct
/// (services/execution-state/src/types.rs:667) into a first-class versioned row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,            // UUIDv7 (monotonic) -> latest = MAX(id)
    pub execution_id: String,
    pub session_id: String,
    pub llm_turn: u32,
    pub last_message_id: String,
    pub pending_tool_calls: Option<String>, // JSON array
    pub context_state: Option<String>,      // JSON: typed {plan, recalled_facts, intent, ward, subagents}
    pub child_executions: Option<String>,   // JSON array
    pub schema_version: i64,
    pub created_at: String,
}

/// Compaction-without-loss: a derived summary of messages up to `as_of_seq`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub session_id: String,
    pub as_of_message_id: String,
    pub as_of_seq: i64,
    pub summary: String,
    pub schema_version: i64,
    pub created_at: String,
}
```

- [ ] **Step 4: Create `src/schema.rs`**

```rust
use anyhow::Result;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    execution_id TEXT,
    session_id   TEXT NOT NULL,
    role         TEXT NOT NULL,
    content      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    token_count  INTEGER NOT NULL DEFAULT 0,
    tool_calls   TEXT,
    tool_call_id TEXT,
    seq          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_session_seq ON messages(session_id, seq);

CREATE TABLE IF NOT EXISTS checkpoints (
    id                 TEXT PRIMARY KEY,
    execution_id       TEXT NOT NULL,
    session_id         TEXT NOT NULL,
    llm_turn           INTEGER NOT NULL,
    last_message_id    TEXT NOT NULL,
    pending_tool_calls TEXT,
    context_state      TEXT,
    child_executions   TEXT,
    schema_version     INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_checkpoints_exec ON checkpoints(execution_id, id DESC);

CREATE TABLE IF NOT EXISTS thread_summaries (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL,
    as_of_message_id TEXT NOT NULL,
    as_of_seq        INTEGER NOT NULL,
    summary          TEXT NOT NULL,
    schema_version   INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_summaries_session ON thread_summaries(session_id, as_of_seq DESC);
"#;

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}
```

- [ ] **Step 5: Create `src/pool.rs`**

```rust
use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::time::Duration;

use crate::schema;

/// Opens an r2d2 pool to `conversations.db` with the same pragmas as
/// DatabaseManager (WAL, busy_timeout, foreign_keys). Independent of
/// zbot-stores-sqlite::DatabaseManager (we never depend on it).
pub fn open_conversation_pool(path: &std::path::Path) -> Result<Pool<SqliteConnectionManager>> {
    let manager = SqliteConnectionManager::file(path);
    let pool = Pool::builder()
        .max_size(8)
        .connection_customizer(Fixtures)
        .build(manager)?;
    // Initialize schema on a fresh connection.
    let conn = pool.get()?;
    schema::initialize(&conn)?;
    Ok(pool)
}

#[derive(Debug)]
struct Fixtures;

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for Fixtures {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(())
    }
}
```
(The `Duration` import is unused after this snippet — remove it; kept out of the final file.)

- [ ] **Step 6: Create `src/lib.rs`**

```rust
pub mod domain;
pub mod schema;
mod pool;

pub use domain::{Checkpoint, Message, ThreadSummary};
pub use pool::open_conversation_pool;
```
(messages/checkpoints/summaries modules are added in Tasks 2–4; `lib.rs` re-exports them then.)

- [ ] **Step 7: Write the failing schema test** — `tests/schema.rs`

```rust
use rusqlite::Connection;
use zbot_conversation::schema;

#[test]
fn schema_initializes_all_tables() {
    let conn = Connection::open_in_memory().unwrap();
    schema::initialize(&conn).unwrap();
    for table in ["messages", "checkpoints", "thread_summaries"] {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "table {table} missing");
    }
}
```

- [ ] **Step 8: Run the test**

Run: `cargo test -p zbot-conversation --test schema`
Expected: PASS (compile + 3 table assertions).

- [ ] **Step 9: Workspace check**

Run: `cargo check --workspace`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml stores/zbot-conversation
git commit -m "feat(conversation): scaffold zbot-conversation crate (domain + schema v1)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: `MessageStore` (append + replay + next_seq)

**Files:**
- Create: `stores/zbot-conversation/src/messages.rs`
- Create: `stores/zbot-conversation/tests/messages.rs`
- Modify: `stores/zbot-conversation/src/lib.rs` (add `pub mod messages;` + re-export `MessageStore, SqliteMessageStore`)

**Interfaces:**
- Consumes: `Message` (Task 1), `open_conversation_pool` (Task 1).
- Produces: `trait MessageStore { fn append(&self, msg: Message) -> Result<()>; fn replay(&self, session_id, after_seq: Option<i64>, limit: usize) -> Result<Vec<Message>>; fn next_seq(&self, session_id) -> Result<i64>; }`, `SqliteMessageStore::new(pool)`.

- [ ] **Step 1: Write the failing test** — `tests/messages.rs`

```rust
use zbot_conversation::{open_conversation_pool, Message, MessageStore};
use tempfile::NamedTempFile;

fn store() -> std::sync::Arc<SqliteMessageStore> { /* see Step 4 */ unimplemented!() }

#[test]
fn append_then_replay_roundtrip() {
    let store = store();
    let sid = "s1";
    let a = Message { id: u(), execution_id: None, session_id: sid.into(), role: "user".into(),
        content: "hi".into(), created_at: ts(), token_count: 1, tool_calls: None, tool_call_id: None,
        seq: store.next_seq(sid).unwrap() };
    store.append(a).unwrap();
    let replayed = store.replay(sid, None, 100).unwrap();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].content, "hi");
    assert_eq!(replayed[0].seq, 1);
}
```
(Helpers `u()` / `ts()` / `store()` defined in Step 4 below; the full test file uses `Uuid::now_v7().to_string()` for ids and `chrono::Utc::now().to_rfc3339()` for ts.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zbot-conversation --test messages`
Expected: FAIL — `SqliteMessageStore` / `MessageStore` not found.

- [ ] **Step 3: Implement `src/messages.rs`**

```rust
use crate::domain::Message;
use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub trait MessageStore: Send + Sync {
    fn append(&self, msg: &Message) -> Result<()>;
    fn replay(&self, session_id: &str, after_seq: Option<i64>, limit: usize) -> Result<Vec<Message>>;
    fn next_seq(&self, session_id: &str) -> Result<i64>;
}

pub struct SqliteMessageStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteMessageStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self { Self { pool } }
}

impl MessageStore for SqliteMessageStore {
    fn append(&self, msg: &Message) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO messages (id, execution_id, session_id, role, content, created_at,
                token_count, tool_calls, tool_call_id, seq)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![msg.id, msg.execution_id, msg.session_id, msg.role, msg.content,
                msg.created_at, msg.token_count, msg.tool_calls, msg.tool_call_id, msg.seq],
        )?;
        Ok(())
    }

    fn replay(&self, session_id: &str, after_seq: Option<i64>, limit: usize) -> Result<Vec<Message>> {
        let conn = self.pool.get()?;
        let mut q = String::from(
            "SELECT id, execution_id, session_id, role, content, created_at, token_count,
                    tool_calls, tool_call_id, seq FROM messages WHERE session_id = ?",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(session_id.to_string())];
        if let Some(s) = after_seq { q.push_str(" AND seq > ?"); params.push(Box::new(s)); }
        q.push_str(" ORDER BY seq ASC LIMIT ?");
        params.push(Box::new(limit as i64));
        let mut stmt = conn.prepare(&q)?;
        let rows = stmt.query_map(params.as_slice(), |r| Message {
            id: r.get(0)?, execution_id: r.get(1)?, session_id: r.get(2)?, role: r.get(3)?,
            content: r.get(4)?, created_at: r.get(5)?, token_count: r.get(6)?,
            tool_calls: r.get(7)?, tool_call_id: r.get(8)?, seq: r.get(9)?,
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    fn next_seq(&self, session_id: &str) -> Result<i64> {
        let conn = self.pool.get()?;
        let max: Option<i64> = conn.query_row(
            "SELECT MAX(seq) FROM messages WHERE session_id = ?", [session_id], |r| r.get(0),
        ).ok();
        Ok(max.unwrap_or(0) + 1)
    }
}
```

- [ ] **Step 4: Wire `lib.rs` re-exports + complete the test helpers**

`src/lib.rs`: add `pub mod messages;` and `pub use messages::{MessageStore, SqliteMessageStore};`.
Test `store()` helper: `let f = NamedTempFile::new().unwrap(); Arc::new(SqliteMessageStore::new(open_conversation_pool(f.path()).unwrap()))`.

- [ ] **Step 5: Run tests — pass**

Run: `cargo test -p zbot-conversation --test messages`
Expected: PASS. Add a second test `next_seq_monotonic` appending 3 messages, asserting seq = 1,2,3.

- [ ] **Step 6: Commit**

```bash
git add stores/zbot-conversation
git commit -m "feat(conversation): MessageStore (append-only, seq-ordered replay)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: `CheckpointStore` (write + latest)

**Files:** Create `src/checkpoints.rs`, `tests/checkpoints.rs`; modify `lib.rs`.

**Interfaces:**
- Produces: `trait CheckpointStore { fn write(&self, cp: &Checkpoint) -> Result<()>; fn latest(&self, execution_id: &str) -> Result<Option<Checkpoint>>; }`, `SqliteCheckpointStore::new(pool)`.

- [ ] **Step 1: Failing test** — `tests/checkpoints.rs`

```rust
#[test]
fn latest_returns_highest_id() {
    let store = checkpoint_store();
    let exec = "e1";
    for turn in 1..=3 {
        store.write(&cp(exec, turn)).unwrap();
    }
    let latest = store.latest(exec).unwrap().unwrap();
    assert_eq!(latest.llm_turn, 3);
}
```

- [ ] **Step 2: Run — FAIL** (`CheckpointStore` undefined).

- [ ] **Step 3: Implement `src/checkpoints.rs`**

```rust
use crate::domain::Checkpoint;
use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub trait CheckpointStore: Send + Sync {
    fn write(&self, cp: &Checkpoint) -> Result<()>;
    fn latest(&self, execution_id: &str) -> Result<Option<Checkpoint>>;
}

pub struct SqliteCheckpointStore { pool: Pool<SqliteConnectionManager> }
impl SqliteCheckpointStore { pub fn new(pool: Pool<SqliteConnectionManager>) -> Self { Self { pool } } }

impl CheckpointStore for SqliteCheckpointStore {
    fn write(&self, cp: &Checkpoint) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO checkpoints (id, execution_id, session_id, llm_turn, last_message_id,
                pending_tool_calls, context_state, child_executions, schema_version, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![cp.id, cp.execution_id, cp.session_id, cp.llm_turn, cp.last_message_id,
                cp.pending_tool_calls, cp.context_state, cp.child_executions, cp.schema_version, cp.created_at],
        )?;
        Ok(())
    }
    fn latest(&self, execution_id: &str) -> Result<Option<Checkpoint>> {
        let conn = self.pool.get()?;
        let cp = conn.query_row(
            "SELECT id, execution_id, session_id, llm_turn, last_message_id, pending_tool_calls,
                    context_state, child_executions, schema_version, created_at
             FROM checkpoints WHERE execution_id = ? ORDER BY id DESC LIMIT 1",
            [execution_id], |r| Checkpoint {
                id: r.get(0)?, execution_id: r.get(1)?, session_id: r.get(2)?, llm_turn: r.get(3)?,
                last_message_id: r.get(4)?, pending_tool_calls: r.get(5)?, context_state: r.get(6)?,
                child_executions: r.get(7)?, schema_version: r.get(8)?, created_at: r.get(9)?,
            },
        ).ok();
        Ok(cp)
    }
}
```

- [ ] **Step 4:** `lib.rs` re-exports; run `cargo test -p zbot-conversation --test checkpoints` → PASS.

- [ ] **Step 5: Commit**
```bash
git commit -m "feat(conversation): CheckpointStore (versioned, latest-wins)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: `SummaryStore` (write + latest)

**Files:** Create `src/summaries.rs`, `tests/summaries.rs`; modify `lib.rs`.

- [ ] **Step 1: Failing test** — `tests/summaries.rs`

```rust
#[test]
fn latest_returns_highest_as_of_seq() {
    let s = summary_store();
    let sid = "s1";
    s.write(&sum(sid, 10)).unwrap();
    s.write(&sum(sid, 20)).unwrap();
    assert_eq!(s.latest(sid).unwrap().unwrap().as_of_seq, 20);
}
```

- [ ] **Step 2:** Run — FAIL.

- [ ] **Step 3: Implement `src/summaries.rs`** — mirror Task 3 shape:
```rust
pub trait SummaryStore: Send + Sync {
    fn write(&self, s: &ThreadSummary) -> Result<()>;
    fn latest(&self, session_id: &str) -> Result<Option<ThreadSummary>>; // ORDER BY as_of_seq DESC LIMIT 1
}
pub struct SqliteSummaryStore { pool: Pool<SqliteConnectionManager> }
```
(INSERT into thread_summaries; SELECT … ORDER BY as_of_seq DESC LIMIT 1. Full field list per schema.rs.)

- [ ] **Step 4:** `lib.rs` re-exports; run tests → PASS. Run `cargo test -p zbot-conversation` (all 4 test files green).

- [ ] **Step 5: Commit**
```bash
git commit -m "feat(conversation): SummaryStore (compaction-without-loss)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

# Phase 2 — `stores/zbot-trace/` crate

## Task 5: `zbot-trace` scaffold + domain + schema

**Files:** Create `stores/zbot-trace/Cargo.toml`, `src/lib.rs`, `src/domain.rs`, `src/schema.rs`, `src/pool.rs`, `tests/schema.rs`; modify workspace `Cargo.toml`.

**Interfaces:**
- Produces: `TraceEvent`, `SlimLog`; `schema::SCHEMA_SQL` + `initialize`; `pool::open_trace_pool(path)`.

- [ ] **Step 1:** Register `"stores/zbot-trace",` in workspace members.

- [ ] **Step 2:** `Cargo.toml` — like Task 1 plus trace deps:
```toml
[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
r2d2 = "0.8"
r2d2_sqlite = "0.25"
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true, features = ["v7"] }
chrono = { workspace = true }
zstd = "0.13"
duckdb = { version = "1", features = ["bundled"] }
tracing = { workspace = true }
```

- [ ] **Step 3:** `src/domain.rs`
```rust
use serde::{Deserialize, Serialize};

/// Slim execution_log row — payload-free. The live /api/logs UI reads this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlimLog {
    pub id: String,
    pub session_id: String,
    pub conversation_id: Option<String>,
    pub agent_id: String,
    pub parent_session_id: Option<String>,
    pub timestamp: String,
    pub level: String,        // info | warn | error
    pub category: String,     // session | token | tool_call | tool_result | thinking | delegation | system | error | response | intent
    pub message: String,      // short human-readable
    pub metadata: Option<String>, // display scalars ONLY ({tool_name}, {error}) — never payloads
    pub duration_ms: Option<i64>,
}

/// Full-fidelity trace event written to .jsonl.zst. OTel GenAI attribute names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub trace_id: String,
    pub span_id: String,
    pub session_id: String,
    pub execution_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub timestamp: String,
    pub level: String,
    pub category: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,        // OTel: tool.name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>, // gen_ai.tool.call.input / .output (full args/results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,   // gen_ai.usage.*
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,              // gen_ai.request.model
}
```

- [ ] **Step 4:** `src/schema.rs` — execution_logs DDL (**columns unchanged** from today; payload discipline is writer-side, Task 11):
```rust
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS execution_logs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    conversation_id TEXT,
    agent_id TEXT NOT NULL,
    parent_session_id TEXT,
    timestamp TEXT NOT NULL,
    level TEXT NOT NULL,
    category TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata TEXT,
    duration_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_logs_session ON execution_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON execution_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_logs_agent ON execution_logs(agent_id);
"#;
```

- [ ] **Step 5:** `src/pool.rs` — `open_trace_pool(path)` (mirror Task 1 `open_conversation_pool`, calls `schema::initialize`).

- [ ] **Step 6:** `tests/schema.rs` — assert `execution_logs` table + 3 indexes exist. `cargo test -p zbot-trace --test schema` → PASS.

- [ ] **Step 7:** `cargo check --workspace` → clean. Commit:
```bash
git commit -m "feat(trace): scaffold zbot-trace crate (domain + execution_logs schema)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: `SlimLogStore`

**Files:** Create `src/slim_logs.rs`, `tests/slim_logs.rs`; modify `lib.rs`.

**Interfaces:**
- Produces: `trait SlimLogStore { fn append(&self, log: &SlimLog) -> Result<()>; fn query(&self, session_id: &str, limit: usize) -> Result<Vec<SlimLog>>; }`, `SqliteSlimLogStore::new(pool)`.

- [ ] **Step 1: Failing test** — assert `append` stores a row and `metadata` stays scalar (e.g. `{"tool_name":"read_file"}`, **no** `args`/`result` keys).
- [ ] **Step 2:** Run — FAIL.
- [ ] **Step 3: Implement** `src/slim_logs.rs` — INSERT + SELECT (ORDER BY timestamp). Mirror Task 2 structure.
- [ ] **Step 4:** `lib.rs` re-exports; tests pass. Commit:
```bash
git commit -m "feat(trace): SlimLogStore (payload-free execution_logs)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 7: `TraceWriter` (streaming `.jsonl.zst`)

**Files:** Create `src/writer.rs`, `tests/writer.rs`; modify `lib.rs`.

**Interfaces:**
- Produces: `TraceWriter::open(path: &Path) -> Result<Self>`; `fn append(&mut self, event: &TraceEvent) -> Result<()>`; `fn flush(&mut self) -> Result<()>`; `fn close(self) -> Result<()>`.
- `TraceWriterRegistry` (HashMap<session_id, TraceWriter>) for the BatchWriter to hold one per active session. (Added here or in Task 10; keep the registry in this crate.)

**Design:** one zstd **encoder frame per flush** (zstd supports frame concatenation → valid `.jsonl.zst` even after multiple flushes / mid-session crash). Buffer events, on flush write `[events]\u{0}` then flush the encoder; each JSON line is one event.

- [ ] **Step 1: Failing test** — `tests/writer.rs`
```rust
#[test]
fn append_flush_then_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s1.jsonl.zst");
    let mut w = TraceWriter::open(&path).unwrap();
    w.append(&ev("a")).unwrap();
    w.append(&ev("b")).unwrap();
    w.flush().unwrap();
    w.append(&ev("c")).unwrap();
    w.close().unwrap();
    let lines = read_zstd_lines(&path); // helper: zstd decode + split '\n'
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"id\":\"a\""));
}

#[test]
fn crash_leaves_valid_file() {
    // append a, flush, append b (no flush), drop writer without close
    // -> file decodes and contains only 'a' (frame-complete up to last flush)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s2.jsonl.zst");
    {
        let mut w = TraceWriter::open(&path).unwrap();
        w.append(&ev("a")).unwrap();
        w.flush().unwrap();
        w.append(&ev("b")).unwrap(); // not flushed
        // drop without close
    }
    let lines = read_zstd_lines(&path);
    assert_eq!(lines.len(), 1); // only 'a' survived
}
```

- [ ] **Step 2:** Run — FAIL.

- [ ] **Step 3: Implement `src/writer.rs`**
```rust
use crate::domain::TraceEvent;
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::path::Path;
use zstd::stream::Encoder;

pub struct TraceWriter {
    file: BufWriter<File>,
    enc: Encoder<'static, BufWriter<File>>, // 'static: zstd Encoder borrows the writer
}
```
Implementation notes for the engineer (show real bodies):
- `open`: `OpenOptions::new().create(true).append(true).open(path)` → wrap in BufWriter; `Encoder::new(buf, 3)?`.
- `append`: serialize event as JSON, write `serde_json::to_vec(&event)?` + `b'\n'` into the encoder.
- `flush`: `self.enc.flush()?; self.enc.get_mut().flush()?;` — flushes the current zstd frame so the file is decodable up to here.
- `close`: `self.flush()?; self.enc.finish()?;` (writes final frame).
- (The `Encoder` lifetime: use `Encoder::new(writer, level)` which returns `Encoder<'static, W>` is not valid — instead own the BufWriter inside the Encoder and access via `get_mut`. If the borrow checker fights, switch to `zstd::stream::write::Encoder` owned with the writer moved in, re-acquired on `finish`.)

- [ ] **Step 4:** Add `read_zstd_lines` test helper (decode file with `zstd::stream::Decoder`, read lines).

- [ ] **Step 5:** Tests pass. Commit:
```bash
git commit -m "feat(trace): TraceWriter (streaming .jsonl.zst, frame-per-flush, crash-safe)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 8: `TraceAnalytics` (DuckDB over `.jsonl.zst`)

**Files:** Create `src/analytics.rs`, `tests/analytics.rs`; modify `lib.rs`.

**Interfaces:**
- Produces: `TraceAnalytics::open(traces_dir: &Path) -> Result<Self>`; `fn query(&self, sql: &str) -> Result<Vec<serde_json::Value>>` — read-only; `fn sessions_with_failed_tool(&self, tool: &str) -> Result<Vec<String>>` (example analytical query).

- [ ] **Step 1: Failing test** — write 2 `.jsonl.zst` files (one with a `tool_result` error event for `read_file`, one without) via `TraceWriter`; assert `sessions_with_failed_tool("read_file")` returns exactly 1 session.

- [ ] **Step 2:** Run — FAIL.

- [ ] **Step 3: Implement** `src/analytics.rs`:
```rust
use anyhow::Result;
use duckdb::Connection;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct TraceAnalytics { dir: PathBuf, conn: Connection }

impl TraceAnalytics {
    pub fn open(traces_dir: &Path) -> Result<Self> {
        let conn = duckdb::Connection::open_in_memory()?;
        Ok(Self { dir: traces_dir.to_path_buf(), conn })
    }
    pub fn sessions_with_failed_tool(&self, tool: &str) -> Result<Vec<String>> {
        let glob = self.dir.join("*.jsonl.zst").to_string_lossy().to_string();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT DISTINCT json_extract(line, '$.session_id') AS sid
             FROM read_json_auto('{glob}') WHERE
                   json_extract(line, '$.tool_name') = '\"{tool}\"'
               AND json_extract(line, '$.level') = '\"error\"'"
        ))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }
}
```
(Adjust `read_json_auto` options / `json_extract` quoting to match duckdb behavior surfaced by the test; the test drives the exact working form. If `*.jsonl.zst` needs `auto_detect=true` or `format='newline_delimited'`, set it.)

- [ ] **Step 4:** Tests pass. Commit:
```bash
git commit -m "feat(trace): TraceAnalytics (DuckDB read_json_auto over .jsonl.zst)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Phase 2 gate:** `cargo test -p zbot-conversation -p zbot-trace` all green; `cargo check --workspace` clean.

---

# Phase 3 — Cutover (flip consumers)

> **Risk note:** Phase 3 touches live write/read paths. Do it incrementally, one task per commit, running `cargo check --workspace` + the touched crate's tests + a manual daemon smoke after each. The old `conversations.db` will be deleted before this phase (it has no data worth keeping — not prod), so the new tables initialize empty and clean.

## Task 9: Wire the new stores into `AppState`

**Files:**
- Modify: `gateway/Cargo.toml` (add `zbot-conversation`, `zbot-trace` deps)
- Modify: `gateway/src/state/mod.rs:31` (AppState struct), `:57` (the `conversations` field — keep for now, removed Phase 4), and the 3 construction sites `:842, :942, :1093`.

**Interfaces:**
- Consumes: all trait objects from Phase 1–2.
- Produces: `AppState` fields `messages`, `checkpoints`, `summaries`, `slim_logs`, `trace_writer_registry`, `trace_analytics`.

- [ ] **Step 1:** Add deps to `gateway/Cargo.toml`:
```toml
zbot-conversation = { path = "../../stores/zbot-conversation" }
zbot-trace = { path = "../../stores/zbot-trace" }
```

- [ ] **Step 2:** Add fields to `AppState` (state/mod.rs:31), copying the `memory_store` trait-object pattern at line 82:
```rust
pub messages: std::sync::Arc<dyn zbot_conversation::MessageStore>,
pub checkpoints: std::sync::Arc<dyn zbot_conversation::CheckpointStore>,
pub summaries: std::sync::Arc<dyn zbot_conversation::SummaryStore>,
pub slim_logs: std::sync::Arc<dyn zbot_trace::SlimLogStore>,
pub trace_analytics: std::sync::Arc<dyn zbot_trace::TraceAnalytics>,
```
(`trace_writer_registry` lives inside the `BatchWriter`, not AppState — added Task 10.)

- [ ] **Step 3:** At the 3 construction sites, build the pools + stores from the conversations.db path and traces/ dir (paths from `gateway_services::SharedVaultPaths`):
```rust
let conv_pool = zbot_conversation::open_conversation_pool(&paths.conversations_db)?;
let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(conv_pool.clone()));
let checkpoints = Arc::new(zbot_conversation::SqliteCheckpointStore::new(conv_pool.clone()));
let summaries = Arc::new(zbot_conversation::SqliteSummaryStore::new(conv_pool));
let trace_pool = zbot_trace::open_trace_pool(&paths.conversations_db)?;
let slim_logs = Arc::new(zbot_trace::SqliteSlimLogStore::new(trace_pool));
let trace_analytics = Arc::new(zbot_trace::TraceAnalytics::open(&paths.traces_dir)?);
```
(Extract a helper `fn build_conversation_stores(paths) -> (messages, checkpoints, summaries, slim_logs, trace_analytics)` in state/mod.rs and call it from all 3 sites — DRY.)

- [ ] **Step 4:** Add `traces_dir` to `gateway_services::paths` if absent (mirror `conversations_db`).

- [ ] **Step 5:** `cargo check -p gateway` → clean. Commit:
```bash
git commit -m "feat(gateway): wire conversation + trace stores into AppState

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 10: `BatchWriter` gains the `TraceWriter` sink + `TraceEvent` request kind

**Files:**
- Modify: `gateway/gateway-execution/src/invoke/batch_writer.rs`

**Interfaces:**
- Consumes: `zbot_trace::{TraceWriter, TraceEvent}`.
- Produces: `BatchWriterHandle::trace_event(session_id, event)`; per-session `TraceWriter` lifecycle (open on first event, flush every 100ms, close on session end).

- [ ] **Step 1:** Add a new request variant `TraceEvent { session_id: String, event: zbot_trace::TraceEvent }` to the mpsc channel type alongside `SessionMessage` / `LogEntry` / `TokenUpdate` (batch_writer.rs).
- [ ] **Step 2:** Add a `HashMap<String, zbot_trace::TraceWriter>` to the writer task state; `trace_event` appends to the session's writer (opening one on first use), and the existing 100ms tick (`batch_writer.rs:144`) now also flushes all open trace writers.
- [ ] **Step 3:** Add a `close_session_trace(session_id)` call path, invoked from session-end (`gateway-execution/src/lifecycle.rs` session_end).
- [ ] **Step 4:** Unit test: feed 3 `TraceEvent`s for a session, call close, assert `.jsonl.zst` exists with 3 lines (reuse zbot-trace's decode helper).
- [ ] **Step 5:** `cargo test -p gateway-execution` → green; `cargo check --workspace`. Commit.

---

## Task 11: Rewire the dual write sites (payload routing)

**Files:**
- Modify: `gateway/gateway-execution/src/invoke/event_logging.rs` (stop emitting args/result into metadata)
- Modify: `gateway/gateway-execution/src/invoke/stream_event_processor.rs:105-141` (also emit `TraceEvent`)
- Modify: `gateway/gateway-execution/src/runner/execution_stream.rs:159-266, 325, 541-559` (write `messages` via `MessageStore`; checkpoint at turn boundary)
- Modify: `gateway/gateway-execution/src/runner/core.rs:1400-1552` (same routing for the resume path)
- Modify: `gateway/gateway-execution/src/delegation/callback.rs:238` (unchanged message append, +trace event)

**Routing rule (the core of the revamp):**
| Event | `messages` (MessageStore) | `execution_logs` (SlimLogStore.metadata) | trace `.jsonl.zst` (TraceWriter) |
|---|---|---|---|
| User input | append role=user | — | TraceEvent(category=session) |
| ToolCallStart | append role=assistant w/ tool_calls | `{tool_name}` only | TraceEvent(category=tool_call, payload=args) |
| ToolResult | append role=tool (full result content) | `{tool_name}` only | TraceEvent(category=tool_result, payload=result) |
| Turn boundary | — | — | CheckpointStore.write(cp) |
| Delegation result | append role=system | — | TraceEvent(category=delegation) |

- [ ] **Step 1:** In `event_logging.rs`, change `log_tool_call`/`log_tool_result` so `metadata` carries only `{tool_name}` (+`{error}` bool for results). Delete the 500-char truncation. **Keep the function signatures** so call sites don't churn.
- [ ] **Step 2:** In `stream_event_processor.rs`, alongside each `log_tool_call`/`log_tool_result`, push a `TraceEvent` (full payload) to the BatchWriter via the new `trace_event` handle.
- [ ] **Step 3:** In `execution_stream.rs`, replace `conversation_repo.append_session_message(...)` with `state.messages.append(&msg)` (building `zbot_conversation::Message` with `seq = next_seq`). At turn end, `state.checkpoints.write(&cp)` (llm_turn, last_message_id, context_state snapshot).
- [ ] **Step 4:** Mirror Step 3 in `runner/core.rs:1400-1552`.
- [ ] **Step 5:** `delegation/callback.rs:238` — keep the system-message append (now via MessageStore), add a delegation TraceEvent.
- [ ] **Step 6:** Delete `conversations.db` (clean cutover — it's empty/disposable). Start daemon, run one conversation, verify: `messages` rows exist with seq; `execution_logs.metadata` has NO `args`/`result` keys (assert via sqlite3 CLI); `.jsonl.zst` exists with full payloads; a `checkpoints` row exists per turn. Commit.

```bash
git commit -m "feat(execution): route payloads to messages/trace, slim execution_logs

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 12: `session_state.rs` — replay → `CheckpointStore::latest`

**Files:** Modify `gateway/gateway-execution/src/session_state.rs`.

- [ ] **Step 1:** Replace the replay aggregation (reads from execution_logs + messages) with `state.checkpoints.latest(execution_id)?`; deserialize `context_state` into the `SessionState` fields (plan, recalled_facts, intent, ward, subagents). Keep the same returned `SessionState` JSON shape (UI contract).
- [ ] **Step 2:** Test — seed a checkpoint, hit `/api/logs/sessions/:id`, assert the response carries the checkpoint's plan/ward. Commit.

---

## Task 13: Rewire read sites

**Files:** Modify `gateway/src/http/chat.rs:185-215`; `gateway-execution/src/runner/core.rs:1262`; `runner/invoke_bootstrap.rs:480`; `distillation.rs:363`; `sleep/handoff_writer.rs:262`; `sleep/pattern_extractor.rs:165`; `services/execution-state/src/repository.rs:1182`.

- [ ] **Step 1:** `chat.rs` `get_session_messages` → `state.messages.replay(session_id, None, limit)`; map `Message` → `SessionMessageResponse` filling `tool_results: None` (contract preserved).
- [ ] **Step 2:** runner replay (`core.rs:1262`, `invoke_bootstrap.rs:480`) → `MessageStore::replay(.., 200)` then the existing `session_messages_to_chat_format` (adapt to the new `Message` shape — drop the `tool_results` parse).
- [ ] **Step 3:** distillation / handoff / pattern_extractor → `MessageStore::replay` (pattern_extractor parses `tool_calls` JSON as before).
- [ ] **Step 4:** `services/execution-state/src/repository.rs:1182` join-based read → `MessageStore::replay` + execution metadata.
- [ ] **Step 5:** `cargo check --workspace`; daemon smoke: load a session's messages in the UI. Commit.

---

## Task 14: `/api/traces/query` endpoint (additive)

**Files:** Create `gateway/src/http/traces.rs`; modify `gateway/src/http/mod.rs` (mount route); modify `gateway/Cargo.toml` if axum extractors needed.

- [ ] **Step 1:** `traces.rs` handler `POST /api/traces/query { sql_or_preset, params }` → `state.trace_analytics.query(...)`; return JSON rows. Whitelist a few preset queries (failed-tool, tool-frequency) rather than raw SQL from the client (injection safety).
- [ ] **Step 2:** Mount in `http/mod.rs` router.
- [ ] **Step 3:** Test — `cargo test`; manual curl. Commit.

---

## Task 15: API contract golden tests

**Files:** Create `gateway/tests/api_contract.rs` (or extend existing).

- [ ] **Step 1:** Snapshot the JSON shape of `GET /api/sessions/:id/messages` and `/api/logs/sessions/:id` against a seeded store; assert byte-identical to the pre-cutover shape (the `tool_results: null` field present). Commit.

---

# Phase 4 — Delete all superseded code

## Task 16: Remove dead code

**Files (delete/empty):**
- `stores/zbot-stores-sqlite/src/schema.rs` — remove the `messages` + `execution_logs` DDL + their indexes + `SCHEMA_VERSION` messages-related bits (execution_logs/messages now owned by the new crates).
- `stores/zbot-stores-sqlite/src/repository.rs` — remove `ConversationRepository` and `get_session_conversation`/`append_session_message`/`add_message_with_tools`/`tool_sequence_for_session`.
- `stores/zbot-stores-domain/src/message.rs` — remove the `Message` POD (now in `zbot-conversation/domain`). Update re-exports.
- `stores/zbot-stores-traits/src/conversation.rs` — remove the `ConversationStore` trait (superseded by narrow traits).
- `services/execution-state/src/types.rs` — remove the `Checkpoint` struct (now in `zbot-conversation/domain`).
- `gateway-execution/src/archiver.rs` — remove the gz JSONL archiver (TraceWriter replaces it; archiver becomes a no-op finalizer or is deleted).
- `gateway-execution/src/session_state.rs` — remove the now-dead replay aggregation (kept only the checkpoint-read path from Task 12).
- `services/api-logs` — remove the 1000-char truncation helper.
- `gateway/src/state/mod.rs` — remove the old `conversations: Arc<ConversationRepository>` field and its 3 construction-site lines.

- [ ] **Step 1:** Delete the entries above in dependency order: trait consumers first (any `use` of `ConversationRepository`/`ConversationStore`/old `Message`/`Checkpoint` must already be gone after Phase 3 — verify with `cargo check --workspace` before each file's symbols are removed).
- [ ] **Step 2:** After each removal: `cargo check --workspace` (must stay green — if a reference remains, Phase 3 missed a site; go fix it there, don't paper over).
- [ ] **Step 3:** `cargo test --workspace` green. `npm run build` clean. Commit:
```bash
git commit -m "chore: delete superseded conversation/trace code (ConversationRepository, gz archiver, replay loop)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review (completed during authoring)

- **Spec coverage:** spec §5 → Tasks 1–4; §6 → Tasks 5–8; §7 → Tasks 10–11 (write), 12–13 (read); §8 → Task 9 (Seam 1) + Task 15 (Seam 2 contract); §9 (OTel) → Task 5 domain + Task 11 routing; §10 (tests) → embedded in Tasks 2–8, 10, 12, 15; §11 (rollout) → Phases 1–4; §13 (alternatives) → reflected in non-goals. ✅
- **Placeholders:** none — every code step shows real code or a precise mirror instruction; cutover tasks name exact files/line ranges and the routing table. ✅
- **Type consistency:** `Message`/`Checkpoint`/`ThreadSummary` field names are identical across domain.rs, schema.rs, store impls, and tests. `MessageStore::append(&Message)`, `CheckpointStore::latest -> Option<Checkpoint>`, `SummaryStore::latest -> Option<ThreadSummary>` used consistently. ✅
- **Open during execution:** the `zstd::stream::Encoder` lifetime detail (Task 7 Step 3) may need the owned-writer form — flagged for the implementer to resolve from the test, not a placeholder. The `read_json_auto` duckdb option string (Task 8) is test-driven.

## Execution Handoff

Plan complete and saved to `docs/specs/conversation-store-revamp/plan.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — I execute tasks in this session via executing-plans, batch with checkpoints for review.

Which approach?
