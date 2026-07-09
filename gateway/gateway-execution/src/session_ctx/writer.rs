//! # Ctx writer functions
//!
//! Each function writes a single fact into the ctx namespace via
//! `MemoryFactStore::save_ctx_fact`. Owner is always `"root"` — these
//! are runtime-driven writes fired on lifecycle events, not agent
//! initiated. Subagent state handoffs (which have subagent owners) are
//! written via [`state_handoff`] below.
//!
//! All functions are async (the fact store is async). All short-circuit
//! to `Ok(())` when the fact store is `None` (test/dev environments
//! without a DB-backed store) or log + skip on save errors — a failed
//! ctx write is non-fatal to the session.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use zbot_stores::MemoryFactStore;

/// Write the session's meta fact.
///
/// Key: `ctx.<sid>.session.meta`.
/// Content: JSON with `sid`, `ward`, `root_agent`, `started_at`.
pub async fn session_meta(
    store: &Arc<dyn MemoryFactStore>,
    sid: &str,
    ward: &str,
    root_agent: &str,
    started_at: DateTime<Utc>,
) {
    let content = json!({
        "sid": sid,
        "ward": ward,
        "root_agent": root_agent,
        "started_at": started_at.to_rfc3339(),
    })
    .to_string();

    let key = format!("ctx.{}.session.meta", sid);
    if let Err(e) = store
        .save_ctx_fact(sid, ward, &key, &content, "root", true)
        .await
    {
        tracing::warn!("session_ctx.session_meta write failed: {}", e);
    }
}

/// Write the intent-analyzer's decision alongside the original prompt.
///
/// Two facts are written: `ctx.<sid>.intent` (JSON blob) and
/// `ctx.<sid>.prompt` (verbatim user message). Both root-owned, pinned.
pub async fn intent_snapshot(
    store: &Arc<dyn MemoryFactStore>,
    sid: &str,
    ward: &str,
    intent: &Value,
    prompt: &str,
) {
    let intent_key = format!("ctx.{}.intent", sid);
    let intent_content = intent.to_string();
    if let Err(e) = store
        .save_ctx_fact(sid, ward, &intent_key, &intent_content, "root", true)
        .await
    {
        tracing::warn!("session_ctx.intent write failed: {}", e);
    }

    let prompt_key = format!("ctx.{}.prompt", sid);
    if let Err(e) = store
        .save_ctx_fact(sid, ward, &prompt_key, prompt, "root", true)
        .await
    {
        tracing::warn!("session_ctx.prompt write failed: {}", e);
    }
}

/// Write a snapshot of the current plan file.
///
/// Called after the planner agent returns a plan. Stores the full plan
/// text so subagents can fetch it without locating the file on disk.
pub async fn plan_snapshot(store: &Arc<dyn MemoryFactStore>, sid: &str, ward: &str, plan_md: &str) {
    let key = format!("ctx.{}.plan", sid);
    if let Err(e) = store
        .save_ctx_fact(sid, ward, &key, plan_md, "root", true)
        .await
    {
        tracing::warn!("session_ctx.plan write failed: {}", e);
    }
}

/// Write a ward-entry briefing snapshot (ward tree + recent writes).
///
/// Called on first ward entry in a session. Size cap: 2 KB. Larger
/// briefings are truncated with a pointer tail.
pub async fn ward_briefing(
    store: &Arc<dyn MemoryFactStore>,
    sid: &str,
    ward: &str,
    briefing: &str,
) {
    let key = format!("ctx.{}.ward_briefing", sid);
    let content = cap_content(briefing, 2048);
    if let Err(e) = store
        .save_ctx_fact(sid, ward, &key, &content, "root", true)
        .await
    {
        tracing::warn!("session_ctx.ward_briefing write failed: {}", e);
    }
}

/// Write a subagent's handoff summary after it completes.
///
/// Key: `ctx.<sid>.state.<execution_id>`. Pinned=false so a rerun of
/// the same step overwrites the previous handoff. Owner is the
/// subagent's id (not root — this is the one writer that is NOT root-
/// owned).
///
/// The `summary` argument is the markdown body the subagent produced
/// (typically from its `respond()` output, distilled to fit 2 KB). The
/// hook is fire-and-forget: a missing summary just writes frontmatter.
#[allow(clippy::too_many_arguments)]
pub async fn state_handoff(
    store: &Arc<dyn MemoryFactStore>,
    sid: &str,
    ward: &str,
    execution_id: &str,
    agent_id: &str,
    step: Option<u32>,
    completed_at: DateTime<Utc>,
    summary: &str,
    artifacts: &[String],
) {
    let key = format!("ctx.{}.state.{}", sid, execution_id);
    let owner = format!("subagent:{}", agent_id);

    // Compose: YAML frontmatter with structured fields + body.
    let mut content = String::new();
    content.push_str("---\n");
    content.push_str(&format!("execution_id: {}\n", execution_id));
    content.push_str(&format!("agent_id: {}\n", agent_id));
    if let Some(n) = step {
        content.push_str(&format!("step: {}\n", n));
    }
    content.push_str(&format!("completed_at: {}\n", completed_at.to_rfc3339()));
    if !artifacts.is_empty() {
        content.push_str("artifacts:\n");
        for a in artifacts {
            content.push_str(&format!("  - {}\n", a));
        }
    }
    content.push_str("---\n\n");
    content.push_str(summary);

    let content = cap_content(&content, 2048);

    if let Err(e) = store
        .save_ctx_fact(sid, ward, &key, &content, &owner, false)
        .await
    {
        tracing::warn!(
            "session_ctx.state_handoff write failed (exec {}): {}",
            execution_id,
            e
        );
    }
}

/// Truncate a content string to the given byte cap.
///
/// If the input exceeds the cap, returns the first `cap - TAIL_LEN`
/// bytes followed by a truncation pointer. The pointer tells readers
/// that full detail lives in the artifacts referenced by the handoff.
fn cap_content(s: &str, cap: usize) -> String {
    const TAIL: &str = "\n\n[…truncated, see artifacts for full detail]";
    if s.len() <= cap {
        return s.to_string();
    }
    let keep = cap.saturating_sub(TAIL.len());
    // Truncate at a UTF-8 char boundary.
    let mut end = keep.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(cap);
    out.push_str(&s[..end]);
    out.push_str(TAIL);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Recorded `save_ctx_fact` call — (agent, key, value, category, scope, persistent).
    type RecordedCall = (String, String, String, String, String, bool);

    /// A fake store that records all save_ctx_fact calls.
    struct RecordingStore {
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl RecordingStore {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MemoryFactStore for RecordingStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> Result<Value, String> {
            Ok(json!({"success": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> Result<Value, String> {
            Ok(json!({"results": []}))
        }

        async fn save_ctx_fact(
            &self,
            session_id: &str,
            ward_id: &str,
            key: &str,
            content: &str,
            owner: &str,
            pinned: bool,
        ) -> Result<Value, String> {
            self.calls.lock().unwrap().push((
                session_id.to_string(),
                ward_id.to_string(),
                key.to_string(),
                content.to_string(),
                owner.to_string(),
                pinned,
            ));
            Ok(json!({"success": true}))
        }
    }

    fn make_store() -> (Arc<RecordingStore>, Arc<dyn MemoryFactStore>) {
        let concrete = Arc::new(RecordingStore::new());
        let dyn_store: Arc<dyn MemoryFactStore> = concrete.clone();
        (concrete, dyn_store)
    }

    #[tokio::test]
    async fn test_session_meta_writes_one_fact() {
        let (recorder, store) = make_store();

        session_meta(
            &store,
            "sess-1",
            "my-ward",
            "root",
            "2026-04-17T10:00:00Z".parse().unwrap(),
        )
        .await;

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1);
        let (sid, ward, key, content, owner, pinned) = &calls[0];
        assert_eq!(sid, "sess-1");
        assert_eq!(ward, "my-ward");
        assert_eq!(key, "ctx.sess-1.session.meta");
        assert_eq!(owner, "root");
        assert!(*pinned);
        // Content is a JSON string; should include sid + ward
        assert!(content.contains("sess-1"));
        assert!(content.contains("my-ward"));
    }

    #[tokio::test]
    async fn test_intent_snapshot_writes_two_facts() {
        let (recorder, store) = make_store();

        let intent = json!({
            "interpretation": "test interp",
            "ward_chosen": "w",
        });
        intent_snapshot(&store, "sess-2", "w", &intent, "original prompt text").await;

        let calls = recorder.calls();
        assert_eq!(calls.len(), 2);
        let keys: Vec<&str> = calls.iter().map(|c| c.2.as_str()).collect();
        assert!(keys.contains(&"ctx.sess-2.intent"));
        assert!(keys.contains(&"ctx.sess-2.prompt"));

        // Verify the prompt fact carries the verbatim text
        let prompt = calls.iter().find(|c| c.2 == "ctx.sess-2.prompt").unwrap();
        assert_eq!(prompt.3, "original prompt text");
        assert_eq!(prompt.4, "root");
        assert!(prompt.5, "prompt fact must be pinned");
    }

    #[tokio::test]
    async fn test_plan_snapshot_writes_plan_key() {
        let (recorder, store) = make_store();

        plan_snapshot(&store, "sess-3", "w", "# Plan\nStep 1: foo").await;

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, "ctx.sess-3.plan");
        assert_eq!(calls[0].3, "# Plan\nStep 1: foo");
    }

    #[tokio::test]
    async fn test_state_handoff_emits_frontmatter_and_is_unpinned() {
        let (recorder, store) = make_store();

        state_handoff(
            &store,
            "sess-4",
            "w",
            "exec-xyz",
            "code-agent",
            Some(3),
            "2026-04-17T10:24:00Z".parse().unwrap(),
            "## Handoff\nwrote foo.py",
            &["models/foo.py".to_string(), "data/x.json".to_string()],
        )
        .await;

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1);
        let (_, _, key, content, owner, pinned) = &calls[0];
        assert_eq!(key, "ctx.sess-4.state.exec-xyz");
        assert_eq!(owner, "subagent:code-agent");
        assert!(
            !*pinned,
            "state_handoff must be unpinned so reruns overwrite"
        );
        assert!(content.contains("execution_id: exec-xyz"));
        assert!(content.contains("agent_id: code-agent"));
        assert!(content.contains("step: 3"));
        assert!(content.contains("artifacts:"));
        assert!(content.contains("  - models/foo.py"));
        assert!(content.contains("  - data/x.json"));
        assert!(content.contains("## Handoff"));
    }

    #[test]
    fn test_cap_content_under_limit_unchanged() {
        let s = "short string";
        assert_eq!(cap_content(s, 2048), s);
    }

    #[test]
    fn test_cap_content_over_limit_truncated_with_marker() {
        let big = "a".repeat(3000);
        let out = cap_content(&big, 2048);
        assert!(out.len() <= 2048);
        assert!(out.ends_with("[…truncated, see artifacts for full detail]"));
    }

    #[test]
    fn test_cap_content_utf8_boundary() {
        // 4-byte codepoint at an awkward position.
        let mut s = String::from("prefix");
        for _ in 0..100 {
            s.push('🦀'); // 4 bytes
        }
        let out = cap_content(&s, 100);
        // Should not panic on UTF-8 slice, and must end with the truncation marker.
        assert!(out.len() <= 100);
        assert!(out.ends_with("[…truncated, see artifacts for full detail]"));
    }
}
