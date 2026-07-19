# Reflective Memory Phase 1 — Session Handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire a `HandoffWriter` that fires on session completion, writes a 3-5 sentence LLM summary to `session_summaries.json`, and injects a `## Last Session` context block at every new session start — raising the agent's self-awareness from 2/10 to ~4/10.

**Architecture:** `HandoffWriter` (new struct in `sleep/`) is triggered fire-and-forget on session completion in both `execution_stream.rs` and `invoke_continuation`. At session start, `InvokeBootstrap::finish_setup` reads `handoff.latest` from the shared KV file and prepends a formatted block to history via the free function `read_handoff_block`. No new DB tables, no new tools, no new HTTP endpoints.

**Tech Stack:** Rust async (tokio), `serde_json`, `chrono`, `async-trait`, `agent_tools::{MemoryStore, MemoryEntry}` for shared KV I/O, `agent_runtime::ChatMessage` for message types.

---

## File Map

| File | Change |
|------|--------|
| `gateway/gateway-execution/src/sleep/handoff_writer.rs` | **New** — all handoff types, trait, impl, free functions |
| `gateway/gateway-execution/src/sleep/mod.rs` | Add `pub mod handoff_writer` + re-exports |
| `gateway/gateway-execution/src/runner/execution_stream.rs` | Add `handoff_writer` field + trigger after distiller spawn |
| `gateway/gateway-execution/src/runner/core.rs` | `ExecutionRunner`, `ExecutionRunnerConfig`, `ContinuationArgs`, `with_config`, `make_continuation_invoker`, `invoke_continuation`, `ExecutionStream` construction |
| `gateway/gateway-execution/src/runner/continuation_watcher.rs` | `RunnerContinuationInvoker` field + pass-through |
| `gateway/gateway-execution/src/runner/invoke_bootstrap.rs` | Call `read_handoff_block` in `finish_setup` after recall injection |
| `gateway/src/state/mod.rs` | Construct `LlmHandoffWriter` + pass `handoff_writer: Some(...)` to config |

---

## Task 1: Create `handoff_writer.rs` with stub types and 4 failing unit tests

**Files:**
- Create: `gateway/gateway-execution/src/sleep/handoff_writer.rs`

- [ ] **Step 1: Create the file with type stubs**

Write the full file. `write_with_messages`, `persist`, `should_inject`, and `read_handoff_block` all call `todo!()` so tests will fail with "not yet implemented" — not compile errors.

```rust
//! HandoffWriter — writes session handoff to `session_summaries.json` on completion,
//! and provides `read_handoff_block` for injection at session start.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_runtime::ChatMessage;
use agent_tools::{MemoryEntry, MemoryStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zbot_stores_sqlite::ConversationRepository;

pub const HANDOFF_MAX_AGE_DAYS: i64 = 7;

/// Stored JSON shape inside each `handoff.*` entry in `session_summaries.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffEntry {
    pub summary: String,
    pub session_id: String,
    pub completed_at: String,
    pub ward_id: String,
    pub intent_key: String,
    pub goal_count: u32,
    pub open_task_count: u32,
    pub correction_count: u32,
    pub turns: u32,
}

/// Input passed to the LLM for summarisation.
#[derive(Debug, Clone)]
pub struct HandoffInput {
    pub messages: Vec<ChatMessage>,
    pub ward_id: String,
}

/// Mockable LLM interface for generating 3-5 sentence handoff summaries.
#[async_trait]
pub trait HandoffLlm: Send + Sync {
    async fn summarize(&self, input: &HandoffInput) -> Result<String, String>;
}

/// Writes session handoff to the shared `session_summaries.json` KV file.
pub struct HandoffWriter {
    llm: Arc<dyn HandoffLlm>,
    /// Path to `agents_data/shared/` directory.
    shared_kv_dir: PathBuf,
    conversation_repo: Arc<ConversationRepository>,
}

impl HandoffWriter {
    pub fn new(
        llm: Arc<dyn HandoffLlm>,
        shared_kv_dir: PathBuf,
        conversation_repo: Arc<ConversationRepository>,
    ) -> Self {
        Self { llm, shared_kv_dir, conversation_repo }
    }

    /// Fire-and-forget entry point: loads last 50 messages then calls
    /// `write_with_messages`. All errors are logged at warn and swallowed.
    pub async fn write(&self, session_id: &str, agent_id: &str, ward_id: &str) {
        let messages_raw = match self.conversation_repo
            .get_session_conversation(session_id, 50)
        {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(session_id, "handoff: failed to load messages: {e}");
                return;
            }
        };
        let messages = self.conversation_repo
            .session_messages_to_chat_format(&messages_raw);
        if let Err(e) = self.write_with_messages(session_id, ward_id, messages).await {
            tracing::warn!(session_id, "handoff: write failed: {e}");
        }
    }

    /// Testable core: accepts pre-loaded messages; returns Err on LLM or I/O failure.
    pub async fn write_with_messages(
        &self,
        session_id: &str,
        ward_id: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<(), String> {
        todo!("implement in Task 2")
    }

    fn persist(&self, session_id: &str, entry: &HandoffEntry) -> Result<(), String> {
        todo!("implement in Task 2")
    }
}

/// Returns false if the entry is older than `HANDOFF_MAX_AGE_DAYS` or unparseable.
pub fn should_inject(entry: &HandoffEntry) -> bool {
    todo!("implement in Task 3")
}

/// Reads `handoff.latest` from `<vault_dir>/agents_data/shared/session_summaries.json`.
/// Returns `None` if absent, unparseable, or older than `HANDOFF_MAX_AGE_DAYS`.
/// Returns `Some(block)` where `block` is the `## Last Session` formatted string.
pub fn read_handoff_block(vault_dir: &Path) -> Option<String> {
    todo!("implement in Task 3")
}

// ============================================================================
// Internal KV helpers (no file locking needed — single writer per session)
// ============================================================================

fn load_kv_store(path: &Path) -> Result<MemoryStore, String> {
    if !path.exists() {
        return Ok(MemoryStore::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("parse session_summaries: {e}"))
}

fn save_kv_store(path: &Path, store: &MemoryStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, content.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

// ============================================================================
// Production LLM impl
// ============================================================================

/// Production `HandoffLlm` wired to the default configured provider.
/// Propagates errors so `write` can log + swallow.
pub struct LlmHandoffWriter {
    provider_service: Arc<gateway_services::ProviderService>,
}

impl LlmHandoffWriter {
    pub fn new(provider_service: Arc<gateway_services::ProviderService>) -> Self {
        Self { provider_service }
    }

    fn build_client(
        &self,
    ) -> Result<Arc<dyn agent_runtime::llm::LlmClient>, String> {
        use agent_runtime::llm::{LlmConfig, openai::OpenAiClient};
        let providers = self
            .provider_service
            .list()
            .map_err(|e| format!("list providers: {e}"))?;
        let provider = providers
            .iter()
            .find(|p| p.is_default)
            .or_else(|| providers.first())
            .ok_or_else(|| "no providers configured".to_string())?;
        let model = provider.default_model().to_string();
        let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());
        let config = LlmConfig::new(
            provider.base_url.clone(),
            provider.api_key.clone(),
            model,
            provider_id,
        )
        .with_temperature(0.2)
        .with_max_tokens(256);
        let client = OpenAiClient::new(config)
            .map_err(|e| format!("build client: {e}"))?;
        Ok(Arc::new(client) as Arc<dyn agent_runtime::llm::LlmClient>)
    }
}

#[async_trait]
impl HandoffLlm for LlmHandoffWriter {
    async fn summarize(&self, input: &HandoffInput) -> Result<String, String> {
        let client = self.build_client()?;
        let conversation = input
            .messages
            .iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .take(40)
            .map(|m| format!("{}: {}", m.role, m.text_content()))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Summarize this conversation in 3-5 sentences. Cover:\n\
             - What was accomplished\n\
             - What was left incomplete or in progress\n\
             - What the user seemed most focused on or interested in next\n\n\
             Be specific. Do not use filler phrases like 'the user and assistant discussed'.\n\
             Use past tense. Write for an agent reading this at the start of the NEXT session.\n\n\
             Ward: {ward}\n\n\
             Conversation:\n{conversation}",
            ward = input.ward_id,
            conversation = conversation,
        );
        let messages = vec![
            ChatMessage::system(
                "You are a concise session summarizer. Return only the summary text, no JSON.".to_string(),
            ),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM call: {e}"))?;
        Ok(response.content)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;
    use zbot_stores_sqlite::{ConversationRepository, DatabaseManager};

    // ---- Mock LLM ----

    struct MockLlm {
        response: Mutex<Result<String, String>>,
        calls: Mutex<u32>,
    }

    impl MockLlm {
        fn ok(summary: &str) -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(Ok(summary.to_string())),
                calls: Mutex::new(0),
            })
        }
        fn err() -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(Err("mock LLM error".to_string())),
                calls: Mutex::new(0),
            })
        }
        fn call_count(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl HandoffLlm for MockLlm {
        async fn summarize(&self, _input: &HandoffInput) -> Result<String, String> {
            *self.calls.lock().unwrap() += 1;
            self.response.lock().unwrap().clone()
        }
    }

    // ---- Harness ----

    fn make_conversation_repo(tmp: &TempDir) -> Arc<ConversationRepository> {
        let db_path = tmp.path().join("test.db");
        let db = Arc::new(DatabaseManager::new(db_path.to_str().unwrap()).expect("db"));
        Arc::new(ConversationRepository::new(db))
    }

    fn make_writer(
        tmp: &TempDir,
        llm: Arc<dyn HandoffLlm>,
    ) -> HandoffWriter {
        let shared_kv_dir = tmp.path().join("agents_data").join("shared");
        HandoffWriter::new(llm, shared_kv_dir, make_conversation_repo(tmp))
    }

    fn sample_messages(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    ChatMessage::user(format!("user message {i}"))
                } else {
                    ChatMessage::assistant(format!("assistant reply {i}"))
                }
            })
            .collect()
    }

    // ---- Test 1: generates_summary_from_messages ----

    #[tokio::test]
    async fn generates_summary_from_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let llm = MockLlm::ok("User explored memory limits. Wrote reflective spec. Left impl incomplete.");
        let writer = make_writer(&tmp, llm.clone());

        let result = writer
            .write_with_messages(
                "sess-abc",
                "test-ward",
                sample_messages(6),
            )
            .await;

        assert!(result.is_ok(), "write_with_messages failed: {result:?}");
        assert_eq!(llm.call_count(), 1, "LLM should be called exactly once");

        // Verify the stored entry has non-empty summary
        let path = tmp
            .path()
            .join("agents_data")
            .join("shared")
            .join("session_summaries.json");
        let store = load_kv_store(&path).unwrap();
        let latest = store.entries.get("handoff.latest").expect("handoff.latest missing");
        let entry: HandoffEntry = serde_json::from_str(&latest.value).unwrap();
        assert!(!entry.summary.is_empty(), "summary should be non-empty");
        assert_eq!(entry.session_id, "sess-abc");
        assert_eq!(entry.ward_id, "test-ward");
        assert_eq!(entry.turns, 3, "3 user messages in sample_messages(6)");
    }

    // ---- Test 2: writes_latest_and_archived_keys ----

    #[tokio::test]
    async fn writes_latest_and_archived_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let llm = MockLlm::ok("Session summary here.");
        let writer = make_writer(&tmp, llm);

        writer
            .write_with_messages("sess-xyz", "my-ward", sample_messages(4))
            .await
            .unwrap();

        let path = tmp
            .path()
            .join("agents_data")
            .join("shared")
            .join("session_summaries.json");
        let store = load_kv_store(&path).unwrap();

        assert!(
            store.entries.contains_key("handoff.latest"),
            "handoff.latest missing"
        );
        assert!(
            store.entries.contains_key("handoff.sess-xyz"),
            "handoff.sess-xyz missing"
        );

        // Both entries should have the same summary
        let latest: HandoffEntry =
            serde_json::from_str(&store.entries["handoff.latest"].value).unwrap();
        let archived: HandoffEntry =
            serde_json::from_str(&store.entries["handoff.sess-xyz"].value).unwrap();
        assert_eq!(latest.summary, archived.summary);
    }

    // ---- Test 3: failure_is_silent ----

    #[tokio::test]
    async fn failure_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let llm = MockLlm::err();
        let writer = make_writer(&tmp, llm);

        // Should NOT panic, should NOT write anything
        let result = writer
            .write_with_messages("sess-fail", "ward", sample_messages(2))
            .await;

        // write_with_messages returns Err (callers swallow it via `write`)
        assert!(result.is_err(), "expected Err when LLM fails");

        let path = tmp
            .path()
            .join("agents_data")
            .join("shared")
            .join("session_summaries.json");
        assert!(!path.exists(), "no file should be written on LLM failure");
    }

    // ---- Test 4: stale_handoff_excluded ----

    #[test]
    fn stale_handoff_excluded() {
        // 8 days ago — should NOT inject
        let stale = HandoffEntry {
            summary: "old summary".to_string(),
            session_id: "sess-old".to_string(),
            completed_at: (Utc::now() - chrono::Duration::days(8)).to_rfc3339(),
            ward_id: "ward".to_string(),
            intent_key: "ctx.sess-old.intent".to_string(),
            goal_count: 0,
            open_task_count: 0,
            correction_count: 0,
            turns: 5,
        };
        assert!(!should_inject(&stale), "8-day-old handoff should be excluded");

        // 6 days ago — should inject
        let fresh = HandoffEntry {
            completed_at: (Utc::now() - chrono::Duration::days(6)).to_rfc3339(),
            ..stale.clone()
        };
        assert!(should_inject(&fresh), "6-day-old handoff should be injected");
    }
}
```

- [ ] **Step 2: Run tests to confirm they all fail with "not yet implemented"**

```bash
cd /home/videogamer/projects/agentzero
cargo test -p gateway-execution sleep::handoff_writer 2>&1 | grep -E "test result|FAILED|panicked|todo"
```

Expected: 4 tests fail with `panicked at 'not yet implemented'` (or compile errors fixed below). If you see compile errors about missing `todo!()` expansions, just check the Rust edition — `todo!()` has been stable since 2018.

---

## Task 2: Implement `HandoffWriter` (make 4 unit tests pass)

**Files:**
- Modify: `gateway/gateway-execution/src/sleep/handoff_writer.rs`

- [ ] **Step 1: Implement `write_with_messages`**

Replace the `todo!("implement in Task 2")` body in `write_with_messages`:

```rust
pub async fn write_with_messages(
    &self,
    session_id: &str,
    ward_id: &str,
    messages: Vec<ChatMessage>,
) -> Result<(), String> {
    let turns = messages.iter().filter(|m| m.role == "user").count() as u32;
    let input = HandoffInput {
        messages,
        ward_id: ward_id.to_string(),
    };
    let summary = self.llm.summarize(&input).await?;
    let entry = HandoffEntry {
        summary,
        session_id: session_id.to_string(),
        completed_at: Utc::now().to_rfc3339(),
        ward_id: ward_id.to_string(),
        intent_key: format!("ctx.{session_id}.intent"),
        goal_count: 0,
        open_task_count: 0,
        correction_count: 0,
        turns,
    };
    self.persist(session_id, &entry)
}
```

- [ ] **Step 2: Implement `persist`**

Replace the `todo!("implement in Task 2")` body in `persist`:

```rust
fn persist(&self, session_id: &str, entry: &HandoffEntry) -> Result<(), String> {
    let path = self.shared_kv_dir.join("session_summaries.json");
    let mut store = load_kv_store(&path)?;
    let value = serde_json::to_string(entry)
        .map_err(|e| format!("serialize entry: {e}"))?;
    let now = Utc::now().to_rfc3339();
    let make_mem_entry = |v: String| MemoryEntry {
        value: v,
        tags: vec![],
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    store.entries.insert("handoff.latest".to_string(), make_mem_entry(value.clone()));
    store
        .entries
        .insert(format!("handoff.{session_id}"), make_mem_entry(value));
    save_kv_store(&path, &store)
}
```

- [ ] **Step 3: Run the 4 tests and verify all pass**

```bash
cargo test -p gateway-execution sleep::handoff_writer 2>&1 | grep -E "test result|FAILED|ok"
```

Expected output:
```
test sleep::handoff_writer::tests::generates_summary_from_messages ... ok
test sleep::handoff_writer::tests::writes_latest_and_archived_keys ... ok
test sleep::handoff_writer::tests::failure_is_silent ... ok
test sleep::handoff_writer::tests::stale_handoff_excluded ... ok
test result: ok. 4 passed; 0 failed
```

- [ ] **Step 4: Commit**

```bash
git add gateway/gateway-execution/src/sleep/handoff_writer.rs
git commit -m "$(cat <<'EOF'
feat(handoff): add HandoffWriter with 4 passing unit tests

HandoffEntry, HandoffInput, HandoffLlm trait, HandoffWriter struct,
persist(), write_with_messages(). LlmHandoffWriter production impl.
Four tests: generates summary, writes both keys, silent on failure,
stale exclusion. read_handoff_block/should_inject are todo() stubs.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Implement `should_inject` and `read_handoff_block` with 3 tests

**Files:**
- Modify: `gateway/gateway-execution/src/sleep/handoff_writer.rs`

- [ ] **Step 1: Add 3 tests for `read_handoff_block` to the existing `#[cfg(test)]` block**

Append these tests inside the existing `mod tests { }` block:

```rust
    // ---- Test 5: read_handoff_block returns formatted block ----

    #[test]
    fn read_handoff_block_returns_formatted_block() {
        let tmp = tempfile::tempdir().unwrap();
        let shared_dir = tmp.path().join("agents_data").join("shared");
        std::fs::create_dir_all(&shared_dir).unwrap();

        let entry = HandoffEntry {
            summary: "User explored memory. Spec written.".to_string(),
            session_id: "sess-abc".to_string(),
            completed_at: Utc::now().to_rfc3339(),
            ward_id: "memory-ward".to_string(),
            intent_key: "ctx.sess-abc.intent".to_string(),
            goal_count: 2,
            open_task_count: 0,
            correction_count: 3,
            turns: 10,
        };
        let mut store = MemoryStore::default();
        let value = serde_json::to_string(&entry).unwrap();
        let now = Utc::now().to_rfc3339();
        store.entries.insert(
            "handoff.latest".to_string(),
            MemoryEntry { value, tags: vec![], created_at: now.clone(), updated_at: now },
        );
        let path = shared_dir.join("session_summaries.json");
        save_kv_store(&path, &store).unwrap();

        let block = read_handoff_block(tmp.path()).expect("should return a block");
        assert!(block.contains("## Last Session"), "block must start with ## Last Session");
        assert!(block.contains("User explored memory"), "block must contain summary");
        assert!(block.contains("memory-ward"), "block must contain ward_id");
        assert!(block.contains("Corrections active: 3"), "block must contain correction_count");
        assert!(block.contains("Goals: 2"), "block must contain goal_count");
        assert!(block.contains("handoff.sess-abc"), "block must contain session key");
    }

    // ---- Test 6: read_handoff_block returns None for stale entry ----

    #[test]
    fn read_handoff_block_returns_none_when_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let shared_dir = tmp.path().join("agents_data").join("shared");
        std::fs::create_dir_all(&shared_dir).unwrap();

        let entry = HandoffEntry {
            summary: "old summary".to_string(),
            session_id: "sess-old".to_string(),
            completed_at: (Utc::now() - chrono::Duration::days(10)).to_rfc3339(),
            ward_id: "ward".to_string(),
            intent_key: "ctx.sess-old.intent".to_string(),
            goal_count: 0,
            open_task_count: 0,
            correction_count: 0,
            turns: 5,
        };
        let mut store = MemoryStore::default();
        let value = serde_json::to_string(&entry).unwrap();
        let now = Utc::now().to_rfc3339();
        store.entries.insert(
            "handoff.latest".to_string(),
            MemoryEntry { value, tags: vec![], created_at: now.clone(), updated_at: now },
        );
        save_kv_store(&shared_dir.join("session_summaries.json"), &store).unwrap();

        let block = read_handoff_block(tmp.path());
        assert!(block.is_none(), "stale handoff should return None");
    }

    // ---- Test 7: read_handoff_block returns None when no file ----

    #[test]
    fn read_handoff_block_returns_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let block = read_handoff_block(tmp.path());
        assert!(block.is_none(), "absent file should return None");
    }
```

- [ ] **Step 2: Run the 3 new tests to confirm they fail with "not yet implemented"**

```bash
cargo test -p gateway-execution "handoff_block" 2>&1 | grep -E "FAILED|panicked|ok|todo"
```

Expected: 3 tests fail with `panicked at 'not yet implemented'`.

- [ ] **Step 3: Implement `should_inject`**

Replace the `todo!("implement in Task 3")` in `should_inject`:

```rust
pub fn should_inject(entry: &HandoffEntry) -> bool {
    let Ok(completed_at) = entry.completed_at.parse::<DateTime<Utc>>() else {
        return false;
    };
    let age_days = (Utc::now() - completed_at).num_days();
    age_days <= HANDOFF_MAX_AGE_DAYS
}
```

- [ ] **Step 4: Implement `read_handoff_block`**

Replace the `todo!("implement in Task 3")` in `read_handoff_block`:

```rust
pub fn read_handoff_block(vault_dir: &Path) -> Option<String> {
    let path = vault_dir
        .join("agents_data")
        .join("shared")
        .join("session_summaries.json");
    let store = load_kv_store(&path).ok()?;
    let latest = store.entries.get("handoff.latest")?;
    let entry: HandoffEntry = serde_json::from_str(&latest.value).ok()?;
    if !should_inject(&entry) {
        return None;
    }
    let date_str = entry
        .completed_at
        .parse::<DateTime<Utc>>()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| entry.completed_at.chars().take(10).collect());
    Some(format!(
        "## Last Session  ({date} · ward: {ward} · {turns} turns)\n\
         {summary}\n\n\
         Corrections active: {corrections} · Goals: {goals}\n\
         Full context: memory(action=get, scope=shared, file=session_summaries, key=handoff.{sid})\n\
         Last intent:  memory(action=get_fact, key={intent_key})",
        date = date_str,
        ward = entry.ward_id,
        turns = entry.turns,
        summary = entry.summary,
        corrections = entry.correction_count,
        goals = entry.goal_count,
        sid = entry.session_id,
        intent_key = entry.intent_key,
    ))
}
```

- [ ] **Step 5: Run all 7 tests and verify all pass**

```bash
cargo test -p gateway-execution sleep::handoff_writer 2>&1 | grep -E "test result|FAILED|ok"
```

Expected: `test result: ok. 7 passed; 0 failed`

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-execution/src/sleep/handoff_writer.rs
git commit -m "$(cat <<'EOF'
feat(handoff): implement should_inject + read_handoff_block

7/7 unit tests passing. should_inject gates on HANDOFF_MAX_AGE_DAYS=7.
read_handoff_block reads handoff.latest, checks staleness, formats
## Last Session block with summary, corrections, goals, and lookup keys.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Export from `sleep/mod.rs`

**Files:**
- Modify: `gateway/gateway-execution/src/sleep/mod.rs`

- [ ] **Step 1: Add module declaration and re-exports**

In `sleep/mod.rs`, add after the existing `pub mod synthesizer;` line:

```rust
pub mod handoff_writer;
```

And add to the existing `pub use` block at the bottom of the file:

```rust
pub use handoff_writer::{
    HandoffEntry, HandoffInput, HandoffLlm, HandoffWriter, LlmHandoffWriter,
    read_handoff_block, should_inject,
};
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p gateway-execution 2>&1 | grep -E "error|warning: unused"
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add gateway/gateway-execution/src/sleep/mod.rs
git commit -m "$(cat <<'EOF'
feat(handoff): export HandoffWriter types from sleep/mod.rs

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire `HandoffWriter` through execution structs

This task adds `handoff_writer: Option<Arc<HandoffWriter>>` to six structs and one function signature. Each change is a mechanical one-liner except for the two trigger spawn blocks.

**Files:**
- Modify: `gateway/gateway-execution/src/runner/execution_stream.rs`
- Modify: `gateway/gateway-execution/src/runner/core.rs`
- Modify: `gateway/gateway-execution/src/runner/continuation_watcher.rs`

### 5a — `execution_stream.rs`

- [ ] **Step 1: Add field to `ExecutionStream` struct**

In `execution_stream.rs`, add to the `ExecutionStream` struct after the `distiller` field (line ~51):

```rust
    pub handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
```

- [ ] **Step 2: Add trigger after the distiller spawn block**

Locate the closing `}` of the `if let Some(distiller) = self.distiller.as_ref() { ... }` block (around line 581). Add immediately after:

```rust
                // Session handoff — fire-and-forget, silent on failure.
                if let Some(writer) = self.handoff_writer.as_ref() {
                    let writer = writer.clone();
                    let sid = session_id.clone();
                    let wid = session_ward.clone().unwrap_or_default();
                    tokio::spawn(async move {
                        writer.write(&sid, &sid, &wid).await;
                    });
                }
```

Note: `agent_id` and `session_id` are the same for subagent sessions. The root `agent_id` is in scope as `agent_id` in that block — use it:

```rust
                // Session handoff — fire-and-forget, silent on failure.
                if let Some(writer) = self.handoff_writer.as_ref() {
                    let writer = writer.clone();
                    let sid = session_id.clone();
                    let aid = agent_id.clone();
                    let wid = session_ward.clone().unwrap_or_default();
                    tokio::spawn(async move {
                        writer.write(&sid, &aid, &wid).await;
                    });
                }
```

- [ ] **Step 3: Set `handoff_writer: None` in the test stub constructor**

Find the `ExecutionStream { ... }` literal in the test helper (around line 737). Add:

```rust
            handoff_writer: None,
```

### 5b — `runner/core.rs`

- [ ] **Step 4: Add field to `ExecutionRunner` struct**

After the `distiller` field (line ~89):

```rust
    /// Handoff writer for session completion → KV summary.
    handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
```

- [ ] **Step 5: Add field to `ExecutionRunnerConfig`**

After the `distiller` field (line ~157):

```rust
    pub handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
```

- [ ] **Step 6: Add field to `ContinuationArgs`**

After the `distiller` field (line ~191):

```rust
    pub(super) handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
```

- [ ] **Step 7: Destructure `handoff_writer` in `with_config`**

In the `let ExecutionRunnerConfig { ... } = config;` destructuring (line ~363), add:

```rust
            handoff_writer,
```

- [ ] **Step 8: Store `handoff_writer` in runner fields in `with_config`**

In the `let runner = Self { ... }` literal (line ~430), add after `distiller,`:

```rust
            handoff_writer,
```

- [ ] **Step 9: Pass `handoff_writer` to `ExecutionStream` construction**

In the `ExecutionStream { ... }` literal (line ~645), add after `distiller: self.distiller.clone(),`:

```rust
            handoff_writer: self.handoff_writer.clone(),
```

- [ ] **Step 10: Pass `handoff_writer` in `make_continuation_invoker`**

In the `RunnerContinuationInvoker { ... }` literal (line ~530), add after `distiller: self.distiller.clone(),`:

```rust
            handoff_writer: self.handoff_writer.clone(),
```

- [ ] **Step 11: Extract `handoff_writer` in `invoke_continuation`**

In `invoke_continuation`'s `let ContinuationArgs { ... } = args;` destructuring (line ~1042), add:

```rust
        handoff_writer,
```

- [ ] **Step 12: Add trigger in `invoke_continuation` after the distiller spawn block**

After the closing `}` of `if let Some(distiller) = distiller { ... }` (line ~1473), add:

```rust
                // Session handoff — fire-and-forget, silent on failure.
                if let Some(writer) = handoff_writer {
                    let sid = session_id_clone.clone();
                    let aid = agent_id_clone.clone();
                    let wid = state_service
                        .get_session(&sid)
                        .ok()
                        .flatten()
                        .and_then(|s| s.ward_id)
                        .unwrap_or_default();
                    tokio::spawn(async move {
                        writer.write(&sid, &aid, &wid).await;
                    });
                }
```

### 5c — `continuation_watcher.rs`

- [ ] **Step 13: Add field to `RunnerContinuationInvoker`**

After the `distiller` field (line ~68):

```rust
    pub(crate) handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
```

- [ ] **Step 14: Pass `handoff_writer` in `spawn_continuation`**

In `invoke_continuation(ContinuationArgs { ... })` call (line ~92), add after `distiller: self.distiller.clone(),`:

```rust
            handoff_writer: self.handoff_writer.clone(),
```

- [ ] **Step 15: Verify compilation**

```bash
cargo check -p gateway-execution 2>&1 | grep -c "^error"
```

Expected: `0` (no errors). If you get "missing field" errors, add the field to the struct literal shown in the error.

- [ ] **Step 16: Commit**

```bash
git add gateway/gateway-execution/src/runner/execution_stream.rs \
        gateway/gateway-execution/src/runner/core.rs \
        gateway/gateway-execution/src/runner/continuation_watcher.rs
git commit -m "$(cat <<'EOF'
feat(handoff): wire HandoffWriter through execution pipeline

Add handoff_writer field to ExecutionRunner, ExecutionRunnerConfig,
ContinuationArgs, RunnerContinuationInvoker, ExecutionStream.
Trigger fire-and-forget write() after session completion in both
execution_stream.rs and invoke_continuation.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Inject handoff block in `invoke_bootstrap.rs`

**Files:**
- Modify: `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`

- [ ] **Step 1: Add the injection after the memory_recall block in `finish_setup`**

Locate the end of the `if let Some(recall) = &self.memory_recall { ... }` block (around line 314). Add immediately after its closing `}`:

```rust
        // Session handoff — injected after recall so it lands at history[0]
        // (the last insert(0, ..) call wins the front slot; agent reads
        // handoff first, giving orientation before noisy recall facts).
        if let Some(block) =
            crate::sleep::handoff_writer::read_handoff_block(self.paths.vault_dir())
        {
            history.insert(0, ChatMessage::system(block));
        }
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p gateway-execution 2>&1 | grep -c "^error"
```

Expected: `0`.

- [ ] **Step 3: Commit**

```bash
git add gateway/gateway-execution/src/runner/invoke_bootstrap.rs
git commit -m "$(cat <<'EOF'
feat(handoff): inject ## Last Session block at session start

read_handoff_block reads handoff.latest from session_summaries.json
and prepends a ## Last Session orientation block to history before
the agent's first message. Injected after recall so it sits at
history[0] — agent sees it first.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Construct `LlmHandoffWriter` in `gateway/src/state/mod.rs`

**Files:**
- Modify: `gateway/src/state/mod.rs`

- [ ] **Step 1: Construct the writer after the distiller block**

Find the line `let distiller_ref: Option<Arc<SessionDistiller>> = distiller.clone();` (around line 619). Add immediately after it:

```rust
        let handoff_writer: Option<Arc<gateway_execution::sleep::HandoffWriter>> = {
            let llm = Arc::new(gateway_execution::sleep::LlmHandoffWriter::new(
                provider_service.clone(),
            ));
            let shared_kv_dir = paths.vault_dir().join("agents_data").join("shared");
            Some(Arc::new(gateway_execution::sleep::HandoffWriter::new(
                llm,
                shared_kv_dir,
                conversation_repo.clone(),
            )))
        };
```

- [ ] **Step 2: Pass `handoff_writer` to `ExecutionRunnerConfig`**

Find the `ExecutionRunnerConfig { ... }` struct literal where the runner is constructed (a few lines after the block above). Add after the `distiller:` field:

```rust
            handoff_writer,
```

- [ ] **Step 3: Verify compilation of the entire workspace**

```bash
cargo check --workspace 2>&1 | grep -c "^error"
```

Expected: `0`. If `ExecutionRunnerConfig` is also constructed in test files, add `handoff_writer: None` to those.

- [ ] **Step 4: Commit**

```bash
git add gateway/src/state/mod.rs
git commit -m "$(cat <<'EOF'
feat(handoff): construct LlmHandoffWriter in AppState and wire to runner

LlmHandoffWriter wraps the default provider. HandoffWriter receives
the shared_kv_dir and conversation_repo. Passed via ExecutionRunnerConfig.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Full test run

- [ ] **Step 1: Run all handoff_writer unit tests**

```bash
cargo test -p gateway-execution sleep::handoff_writer 2>&1 | tail -5
```

Expected: `test result: ok. 7 passed; 0 failed; 0 ignored`

- [ ] **Step 2: Run full workspace test suite**

```bash
cargo test --workspace 2>&1 | tail -10
```

Expected: no new test failures. (Some pre-existing tests may be ignored; that is fine.)

- [ ] **Step 3: cargo clippy**

```bash
cargo clippy -p gateway-execution -- -D warnings 2>&1 | grep "^error" | head -10
```

Fix any warnings-as-errors before finishing.

- [ ] **Step 4: Final commit if any clippy fixes were needed**

```bash
git add -p
git commit -m "$(cat <<'EOF'
fix(handoff): address clippy warnings

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Quick Verification Checklist

After all tasks complete, confirm:

- [ ] `cargo test -p gateway-execution sleep::handoff_writer` → 7 passed
- [ ] `cargo check --workspace` → 0 errors
- [ ] `session_summaries.json` is written after a real session ends (manual test: run a chat session, check `~/.agents/vault/agents_data/shared/session_summaries.json`)
- [ ] Next session's context window shows `## Last Session` block in logs (set `RUST_LOG=debug` and look for "Recalled unified context" log line followed by the handoff)

---

## What This Does NOT Cover (Future Phases)

- Recall noise / `min_score` threshold — Phase 2
- `memory(action=list)` surfacing DB strategy facts — Phase 2
- Knowledge graph utilization / pattern abstraction — Phase 3
