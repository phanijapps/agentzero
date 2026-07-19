//! # Ward snapshot builder (Phase 7)
//!
//! Constructs the `<ward_snapshot>` block prepended to every subagent
//! delegation task. The block combines four sources into a single
//! view of "what exists" — files, doctrine, curated knowledge, and
//! in-session runtime handoffs:
//!
//! 1. `wards/<ward>/AGENTS.md` — ward purpose, conventions, don'ts
//! 2. `wards/<ward>/memory-bank/ward.md` — accumulated domain patterns
//! 3. `wards/<ward>/memory-bank/core_docs.md` — function signatures in `core/`
//! 4. `ctx.<sid>.state.<exec_id>` facts — recent subagent handoffs this session
//!
//! Read fresh from disk on every delegation. Policy
//! (`policy.update_docs_after_code`, `policy.ward_agents_md_authoritative`)
//! expects subagents to update the first three before completing, so
//! fresh reads give the next subagent the most recent curated view.
//!
//! Push model: the LLM doesn't have to decide whether to query ctx.
//! Everything "what exists" pays for is in the prompt.

use std::path::Path;
use std::sync::Arc;

use zbot_stores_traits::MemoryFact;
use zbot_stores_traits::MemoryFactStore;

/// Byte caps per section — total preamble stays under ~5 KB.
const AGENTS_MD_CAP: usize = 2048;
const PRIMITIVES_CAP: usize = 2048;
const HANDOFF_SUMMARY_CAP: usize = 400;
const MAX_HANDOFFS: usize = 5;

/// Build the full `<ward_snapshot>` block for a subagent delegation.
///
/// Returns an empty string if the ward doesn't exist — the caller
/// just prepends nothing in that case. All file reads are non-fatal
/// (missing files are silently skipped).
pub async fn build(
    ward_id: &str,
    sid: &str,
    wards_dir: &Path,
    memory_store: Option<&Arc<dyn MemoryFactStore>>,
) -> String {
    let ward_root = wards_dir.join(ward_id);
    if !ward_root.exists() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("<ward_snapshot ward=\"");
    out.push_str(ward_id);
    out.push_str("\">\n");

    // 1. AGENTS.md — durable ward doctrine (the only filesystem read).
    if let Some(content) = read_file_capped(&ward_root.join("AGENTS.md"), AGENTS_MD_CAP) {
        if !content.trim().is_empty() {
            out.push_str("\n## Doctrine (AGENTS.md)\n\n");
            out.push_str(&content);
            if !content.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    // 2. Reusable primitives — queried live from memory_facts. Runtime
    //    AST hook populates this when code is written; agents cannot
    //    forget to update it (nothing to forget).
    if let Some(store) = memory_store {
        let primitives = store
            .list_primitives_for_ward(ward_id)
            .await
            .unwrap_or_default();
        if !primitives.is_empty() {
            let section = render_primitives(&primitives, PRIMITIVES_CAP);
            out.push_str("\n## Primitives (import these — don't duplicate)\n\n");
            out.push_str(&section);
            if !section.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    // 3. Prior step handoffs from this session (runtime memory).
    if let Some(store) = memory_store {
        match store.list_recent_state_handoffs(sid, MAX_HANDOFFS).await {
            Ok(handoffs) if !handoffs.is_empty() => {
                out.push_str("\n## Prior steps this session\n\n");
                // Oldest-first matches reading order.
                for fact in handoffs.iter().rev() {
                    let exec_id = extract_exec_id(&fact.key);
                    let agent_id = extract_owner_agent(&fact.source_summary);
                    let summary = summarize_handoff(&fact.content, HANDOFF_SUMMARY_CAP);
                    out.push_str(&format!(
                        "- **[{}, {}]** — {}\n",
                        exec_id, agent_id, summary
                    ));
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    session_id = %sid,
                    error = %e,
                    "Failed to fetch state handoffs for ward_snapshot — skipping"
                );
            }
        }
    }

    out.push_str("</ward_snapshot>");
    out
}

/// Render primitives grouped by file path, capped at the byte budget.
///
/// Input: `Vec<MemoryFact>` where each fact's `key` is
/// `primitive.<relative_path>.<symbol>` and `content` is
/// `signature\nsummary` (or just `signature`). Output is markdown
/// grouped by file:
///
/// ```text
/// ## core/valuation.py
/// - `calc_wacc(equity, debt, cost_of_equity, cost_of_debt, tax_rate) -> float`
///   Weighted average cost of capital.
/// ```
fn render_primitives(facts: &[MemoryFact], cap: usize) -> String {
    use std::collections::BTreeMap;
    let mut by_file: BTreeMap<&str, Vec<&MemoryFact>> = BTreeMap::new();
    for f in facts {
        // Key: primitive.<path>.<symbol> → take the middle part.
        let body = f.key.strip_prefix("primitive.").unwrap_or(&f.key);
        let file = match body.rfind('.') {
            Some(idx) => &body[..idx],
            None => body,
        };
        by_file.entry(file).or_default().push(f);
    }

    let mut out = String::new();
    for (file, entries) in by_file {
        out.push_str(&format!("### {}\n", file));
        for f in entries {
            let (sig, summary) = match f.content.split_once('\n') {
                Some((s, sum)) => (s, sum),
                None => (f.content.as_str(), ""),
            };
            out.push_str(&format!("- `{}`", sig));
            if !summary.is_empty() {
                out.push_str(&format!(" — {}", summary));
            }
            out.push('\n');
        }
        if out.len() > cap {
            // Truncate at a UTF-8 boundary + pointer; prevents bloat when
            // wards accumulate hundreds of primitives.
            let mut end = cap;
            while end > 0 && !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push_str("\n\n[…truncated; query memory_facts directly for the rest]");
            break;
        }
    }
    out
}

/// Prepend a freshly-built ward snapshot to the existing task text.
///
/// Called from spawn.rs to wrap the task before it reaches the subagent.
pub async fn prepend_to_task(
    ward_id: &str,
    sid: &str,
    wards_dir: &Path,
    memory_store: Option<&Arc<dyn MemoryFactStore>>,
    task: &str,
) -> String {
    let snapshot = build(ward_id, sid, wards_dir, memory_store).await;
    if snapshot.is_empty() {
        return task.to_string();
    }
    format!("{}\n\n{}", snapshot, task)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_file_capped(path: &Path, cap: usize) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.len() <= cap {
        return Some(content);
    }
    // Truncate at a UTF-8 char boundary, append a pointer line.
    let mut end = cap;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!(
        "{}\n\n[…truncated at {}B, full file at {}]",
        &content[..end],
        cap,
        path.display()
    ))
}

/// Extract the execution id from a ctx key of the form
/// `ctx.<sid>.state.<exec_id>`.
fn extract_exec_id(key: &str) -> String {
    key.rsplit_once("state.")
        .map(|(_, id)| id.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Parse the agent id out of `source_summary="owner=subagent:<agent_id>"`.
fn extract_owner_agent(source_summary: &Option<String>) -> String {
    source_summary
        .as_deref()
        .and_then(|s| s.strip_prefix("owner=subagent:"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            source_summary
                .as_deref()
                .and_then(|s| s.strip_prefix("owner="))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        })
}

/// Reduce a handoff's markdown body to a single-paragraph summary
/// suitable for a bullet line. Prefers the `## What I did` section
/// if present; otherwise takes the first non-frontmatter paragraph.
fn summarize_handoff(content: &str, cap: usize) -> String {
    // Skip YAML frontmatter if present
    let body = strip_frontmatter(content);

    // Prefer the "What I did" section if present
    let narrative = if let Some(idx) = body.find("## What I did") {
        let after = &body[idx + "## What I did".len()..];
        next_paragraph(after)
    } else if let Some(idx) = body.find("## Handoff for next agents") {
        let after = &body[idx + "## Handoff for next agents".len()..];
        next_paragraph(after)
    } else {
        next_paragraph(body)
    };

    let trimmed = narrative.trim();
    if trimmed.len() <= cap {
        trimmed.replace('\n', " ")
    } else {
        let mut end = cap;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &trimmed[..end]).replace('\n', " ")
    }
}

/// Strip leading `---\n…\n---\n` frontmatter if present. Otherwise
/// returns the original slice.
fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let after = &rest[end + 4..];
            return after.trim_start_matches('\n');
        }
    }
    trimmed
}

/// Take the text up to the next blank line (end of paragraph).
fn next_paragraph(s: &str) -> &str {
    let s = s.trim_start_matches(['\n', ' ']);
    if let Some(end) = s.find("\n\n") {
        &s[..end]
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Tests (no MemoryRepository — test the helpers in isolation)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[tokio::test]
    async fn test_build_empty_ward_returns_empty_string() {
        let dir = TempDir::new().unwrap();
        let out = build("does-not-exist", "sess-1", dir.path(), None).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_build_with_agents_md_only() {
        let dir = TempDir::new().unwrap();
        let wards = dir.path();
        write(
            wards,
            "stock-analysis/AGENTS.md",
            "# stock-analysis\n\nPurpose: value companies.",
        );
        let out = build("stock-analysis", "sess-1", wards, None).await;
        assert!(out.starts_with("<ward_snapshot ward=\"stock-analysis\">"));
        assert!(out.ends_with("</ward_snapshot>"));
        assert!(out.contains("## Doctrine (AGENTS.md)"));
        assert!(out.contains("Purpose: value companies"));
        // Memory-bank reads are gone — nothing references those files.
        assert!(!out.contains("memory-bank"));
    }

    #[tokio::test]
    async fn test_build_skips_empty_agents_md() {
        let dir = TempDir::new().unwrap();
        let wards = dir.path();
        write(wards, "w/AGENTS.md", "   \n");
        // No memory_store → no primitives or handoffs. An effectively
        // empty AGENTS.md produces a snapshot with no sections.
        let out = build("w", "sess-1", wards, None).await;
        assert!(out.starts_with("<ward_snapshot"));
        assert!(!out.contains("## Doctrine"));
    }

    #[test]
    fn test_render_primitives_groups_by_file() {
        use zbot_stores_sqlite::MemoryFact;
        fn mk(key: &str, content: &str) -> MemoryFact {
            MemoryFact {
                id: String::new(),
                session_id: None,
                agent_id: "__ward__".into(),
                scope: "global".into(),
                category: "primitive".into(),
                key: key.into(),
                content: content.into(),
                confidence: 1.0,
                mention_count: 1,
                source_summary: None,
                embedding: None,
                ward_id: "w".into(),
                contradicted_by: None,
                created_at: "t".into(),
                updated_at: "t".into(),
                expires_at: None,
                valid_from: None,
                valid_until: None,
                superseded_by: None,
                pinned: false,
                epistemic_class: None,
                source_episode_id: None,
                source_ref: None,
            }
        }
        let facts = vec![
            mk(
                "primitive.core/valuation.py.calc_wacc",
                "calc_wacc(equity, debt) -> float\nWeighted average cost of capital.",
            ),
            mk(
                "primitive.core/valuation.py.dcf_valuation",
                "dcf_valuation(base_fcf, wacc) -> dict",
            ),
            mk(
                "primitive.analysis/rel_val.py.get_multiples",
                "get_multiples(ticker) -> dict\nPeer valuation multiples.",
            ),
        ];
        let out = super::render_primitives(&facts, 10_000);
        // Grouped by file (alphabetical): analysis first, then core.
        assert!(out.contains("### analysis/rel_val.py"));
        assert!(out.contains("### core/valuation.py"));
        // Signature + summary format.
        assert!(out.contains("`calc_wacc(equity, debt) -> float`"));
        assert!(out.contains("Weighted average cost of capital."));
        // Entry with no summary renders as just `sig`.
        assert!(out.contains("`dcf_valuation(base_fcf, wacc) -> dict`"));
        // File ordering: alphabetical.
        let a_pos = out.find("analysis/rel_val.py").unwrap();
        let c_pos = out.find("core/valuation.py").unwrap();
        assert!(a_pos < c_pos);
    }

    #[test]
    fn test_read_file_capped_truncates_with_pointer() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("big.md");
        std::fs::write(&path, "x".repeat(5000)).unwrap();
        let out = read_file_capped(&path, 1024).unwrap();
        assert!(out.len() <= 1024 + 200); // cap + pointer line
        assert!(out.contains("[…truncated at 1024B"));
    }

    #[test]
    fn test_read_file_capped_utf8_boundary() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("emoji.md");
        let s = "🦀".repeat(500); // 2000 bytes
        std::fs::write(&path, &s).unwrap();
        let out = read_file_capped(&path, 100).unwrap();
        // Truncation must not panic on UTF-8 boundary
        assert!(out.contains("[…truncated"));
    }

    #[test]
    fn test_extract_exec_id() {
        assert_eq!(extract_exec_id("ctx.sess-abc.state.exec-xyz"), "exec-xyz");
        assert_eq!(
            extract_exec_id("ctx.sess.state.exec-1-with-dashes"),
            "exec-1-with-dashes"
        );
        assert_eq!(extract_exec_id("random_key"), "unknown");
    }

    #[test]
    fn test_extract_owner_agent_subagent() {
        let src = Some("owner=subagent:code-agent".to_string());
        assert_eq!(extract_owner_agent(&src), "code-agent");
    }

    #[test]
    fn test_extract_owner_agent_root() {
        let src = Some("owner=root".to_string());
        assert_eq!(extract_owner_agent(&src), "root");
    }

    #[test]
    fn test_extract_owner_agent_none() {
        assert_eq!(extract_owner_agent(&None), "unknown");
    }

    #[test]
    fn test_strip_frontmatter() {
        let s = "---\nkey: val\nother: thing\n---\n\nbody text here";
        assert_eq!(strip_frontmatter(s), "body text here");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        assert_eq!(strip_frontmatter("just body"), "just body");
    }

    #[test]
    fn test_summarize_handoff_prefers_what_i_did() {
        let content = "---\nk: v\n---\n\n## What I did\n\nWrote models/foo.py and calibrated DCF\n\n## Handoff for next agents\n\n- Use foo()";
        let out = summarize_handoff(content, 500);
        assert!(out.contains("Wrote models/foo.py"));
        assert!(!out.contains("Use foo()"));
    }

    #[test]
    fn test_summarize_handoff_falls_back_to_handoff_section() {
        let content = "## Handoff for next agents\n\nData at data/x.json\n\nmore";
        let out = summarize_handoff(content, 500);
        assert!(out.contains("Data at data/x.json"));
    }

    #[test]
    fn test_summarize_handoff_truncates_long_narrative() {
        let content = "## What I did\n\n".to_string() + &"word ".repeat(200);
        let out = summarize_handoff(&content, 100);
        assert!(out.len() <= 105);
        assert!(out.ends_with("…"));
    }

    #[tokio::test]
    async fn test_prepend_to_task_wraps_task_below_snapshot() {
        let dir = TempDir::new().unwrap();
        let wards = dir.path();
        write(wards, "w/AGENTS.md", "Ward purpose.");
        let out = prepend_to_task("w", "sess-1", wards, None, "Do the thing.").await;
        assert!(out.starts_with("<ward_snapshot"));
        assert!(out.ends_with("Do the thing."));
        assert!(out.contains("</ward_snapshot>\n\nDo the thing."));
    }

    #[tokio::test]
    async fn test_prepend_to_task_empty_ward_passes_task_through() {
        let dir = TempDir::new().unwrap();
        let out = prepend_to_task(
            "nonexistent-ward",
            "sess-1",
            dir.path(),
            None,
            "Original task.",
        )
        .await;
        assert_eq!(out, "Original task.");
    }
}
