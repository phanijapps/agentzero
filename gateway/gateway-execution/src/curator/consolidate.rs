//! LLM-driven ward consolidation. Spec: §3 of
//! `docs/architecture/future-state/2026-05-23-ward-curator-spec.md`.
//!
//! Pipeline:
//! 1. `WardCurator::build_candidates()` — flatten the sidecar.
//! 2. Either accept the caller's `plan` (tests / replay) or ask the LLM to
//!    produce one (`ask_llm_for_plan`).
//! 3. Re-key procedures: every Merge/Absorb action moves all procedures of
//!    each `from` ward to point at the `into` ward.
//! 4. Hand the plan to `WardCurator::apply_consolidation` for the
//!    deterministic file + sidecar mutation.

use std::sync::Arc;

use agent_runtime::{ChatMessage, LlmClient};
use gateway_services::{
    ConsolidateRequest, ConsolidationAction, ConsolidationPlan, ConsolidationReport, WardCandidate,
    WardCurator,
};
use zbot_stores_traits::ProcedureStore;

/// System prompt — the curator-agent's behavioural contract. Mirrors
/// Hermes's `CURATOR_REVIEW_PROMPT`, adapted for wards.
const CONSOLIDATION_SYSTEM_PROMPT: &str = r#"You are the **ward curator**. You see a table of agent-authored z-Bot wards. Cluster wards whose Purpose/Scope materially overlap and decide one action per cluster.

Available actions:
- `merge` — combine ≥2 sibling wards into a NEW umbrella ward. Specify `from` (the siblings), `into` (the new umbrella name — must NOT already exist), `purpose` (the combined Purpose/Scope, one paragraph), and a `reason`.
- `absorb` — same shape but `into` is an EXISTING umbrella. Specify `from`, `into`, `reason`.
- `archive` — standalone retire of a one-off ward. Specify `ward`, `reason`.

Emit exactly one YAML object inside a fenced code block:

```yaml
consolidations:
  - action: merge
    from: [travel-rome, travel-paris]
    into: travel-planning
    purpose: "Trip planning across all destinations — itineraries, transit, lodging."
    reason: "both target city itineraries; merging into a single travel-planning ward"
  - action: archive
    ward: orphan-x
    reason: "no activity in 47d and no procedures"
```

Hard rules — these are NOT negotiable:
1. Only act on wards explicitly listed in the candidate table.
2. Never act on a candidate where the table doesn't say `state: active` or `state: stale`. (Archived wards are already done.)
3. `use_count == 0` is NOT a consolidation signal — if a ward has never been used, prefer `archive` over `merge`/`absorb`.
4. Every ward you reference in `from` (for merge or absorb) must have ≥1 use in the recent past (it's in the table, so it's an active candidate).
5. Archive only — no delete. Sources of a merge/absorb are auto-archived; you do not need to enumerate them separately.
6. Respect the cap on total consolidations stated in the user message.

If nothing should change, emit an empty plan:

```yaml
consolidations: []
```

Output the YAML block AND NOTHING ELSE — no prose, no explanation."#;

/// Drive a full consolidation pass: build candidates → (optionally) call LLM
/// → re-key procedures → apply. Pure orchestration; all heavy lifting is in
/// `WardCurator` and the trait objects passed in.
pub async fn consolidate_wards(
    curator: &WardCurator,
    llm: &dyn LlmClient,
    procedure_store: Option<&Arc<dyn ProcedureStore>>,
    req: &ConsolidateRequest,
) -> Result<ConsolidationReport, String> {
    let plan = match &req.plan {
        Some(p) => p.clone(),
        None => {
            let candidates = curator.build_candidates();
            if candidates.is_empty() {
                ConsolidationPlan::default()
            } else {
                ask_llm_for_plan(llm, &candidates, req.max_consolidations).await?
            }
        }
    };
    let plan = cap_plan(plan, req.max_consolidations);

    // Re-key procedures first so a failed apply leaves them already pointing
    // at the (possibly soon-to-be-created) umbrella — never at a stale
    // ward_id. Dry-runs skip this — they mutate nothing.
    if !req.dry_run {
        if let Some(store) = procedure_store {
            rekey_procedures_for_plan(store.as_ref(), &plan).await?;
        }
    }

    curator.apply_consolidation(&plan, req.dry_run)
}

async fn ask_llm_for_plan(
    llm: &dyn LlmClient,
    candidates: &[WardCandidate],
    max: usize,
) -> Result<ConsolidationPlan, String> {
    let table = render_table(candidates);
    let user_msg = format!(
        "Cap total consolidations at {max}. The candidate table below is your full universe — never reference a ward not listed here.\n\n{table}\n\nEmit the YAML plan now."
    );
    let messages = vec![
        ChatMessage::system(CONSOLIDATION_SYSTEM_PROMPT.to_string()),
        ChatMessage::user(user_msg),
    ];
    let resp = llm
        .chat(messages, None)
        .await
        .map_err(|e| format!("ward-curator LLM call failed: {e}"))?;
    parse_plan_response(&resp.content)
}

/// Public so tests can exercise prompt-parsing without an LLM.
pub fn parse_plan_response(content: &str) -> Result<ConsolidationPlan, String> {
    let yaml =
        extract_yaml_block(content).ok_or_else(|| "no YAML block in LLM response".to_string())?;
    let plan: ConsolidationPlan =
        serde_yaml::from_str(yaml).map_err(|e| format!("parse plan YAML: {e}"))?;
    Ok(plan)
}

fn render_table(candidates: &[WardCandidate]) -> String {
    let mut out =
        String::from("| name | purpose | use_count | age_days | state |\n|---|---|---|---|---|\n");
    for c in candidates {
        let state = format!("{:?}", c.state).to_lowercase();
        // Strip newlines + pipes from the purpose so the markdown table stays valid.
        let purpose = c.purpose.replace('\n', " ").replace('|', "\\|");
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            c.name, purpose, c.use_count, c.age_days, state
        ));
    }
    out
}

/// Find the contents of the first ```yaml ... ``` (or plain ``` ... ```) fence.
/// Returns the inner text, trimmed of the leading newline after the opening
/// fence. Returns `None` if no fence is found.
fn extract_yaml_block(text: &str) -> Option<&str> {
    let (open_idx, open_len) = if let Some(i) = text.find("```yaml") {
        (i, "```yaml".len())
    } else if let Some(i) = text.find("```") {
        (i, "```".len())
    } else {
        return None;
    };
    let after_open = &text[open_idx + open_len..];
    // Skip past the rest of the opening fence line.
    let inner_start = match after_open.find('\n') {
        Some(i) => i + 1,
        None => 0,
    };
    let inner = &after_open[inner_start..];
    let close_idx = inner.find("```")?;
    Some(&inner[..close_idx])
}

fn cap_plan(mut plan: ConsolidationPlan, max: usize) -> ConsolidationPlan {
    if plan.consolidations.len() > max {
        plan.consolidations.truncate(max);
    }
    plan
}

async fn rekey_procedures_for_plan(
    store: &dyn ProcedureStore,
    plan: &ConsolidationPlan,
) -> Result<(), String> {
    for action in &plan.consolidations {
        let (from_wards, into) = match action {
            ConsolidationAction::Merge { from, into, .. }
            | ConsolidationAction::Absorb { from, into, .. } => (from, into),
            ConsolidationAction::Archive { .. } => continue,
        };
        for f in from_wards {
            rekey_one_ward(store, f, into).await?;
        }
    }
    Ok(())
}

/// Move every procedure currently keyed by `from` to point at `into`.
/// Re-upserts each row with its `ward_id` field rewritten. Errors are
/// returned to the caller so a partial re-key can be detected.
async fn rekey_one_ward(store: &dyn ProcedureStore, from: &str, into: &str) -> Result<(), String> {
    // 1000 is a soft cap that's higher than any realistic ward's procedure
    // count today; if a ward ever exceeds it, the spec's v2 KG re-keying
    // notes already commit to revisiting this.
    let procs = store
        .list_by_ward(from, 1000)
        .await
        .map_err(|e| format!("list_by_ward({from}): {e}"))?;
    for mut value in procs {
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "ward_id".to_string(),
                serde_json::Value::String(into.to_string()),
            );
        } else {
            return Err(format!(
                "procedure row for ward '{from}' was not a JSON object; skipping rekey"
            ));
        }
        store
            .upsert_procedure(value, None)
            .await
            .map_err(|e| format!("upsert_procedure (rekey {from} -> {into}): {e}"))?;
    }
    Ok(())
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::WardState;

    #[test]
    fn extract_yaml_block_handles_fenced_yaml() {
        let resp = "Here is the plan:\n```yaml\nconsolidations:\n  - action: archive\n    ward: x\n    reason: stale\n```\nbye";
        let inner = extract_yaml_block(resp).unwrap();
        assert!(inner.contains("action: archive"));
        assert!(inner.contains("ward: x"));
    }

    #[test]
    fn extract_yaml_block_handles_plain_fence() {
        let resp = "```\nconsolidations: []\n```";
        let inner = extract_yaml_block(resp).unwrap();
        assert!(inner.contains("consolidations: []"));
    }

    #[test]
    fn extract_yaml_block_returns_none_on_unfenced_text() {
        assert!(extract_yaml_block("just prose, no fence").is_none());
    }

    #[test]
    fn parse_plan_response_round_trips_merge() {
        let resp = "```yaml\nconsolidations:\n  - action: merge\n    from: [a, b]\n    into: umb\n    purpose: \"u\"\n    reason: \"r\"\n```";
        let plan = parse_plan_response(resp).unwrap();
        assert_eq!(plan.consolidations.len(), 1);
        match &plan.consolidations[0] {
            ConsolidationAction::Merge { from, into, .. } => {
                assert_eq!(from, &vec!["a".to_string(), "b".to_string()]);
                assert_eq!(into, "umb");
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn parse_plan_response_handles_empty_plan() {
        let resp = "```yaml\nconsolidations: []\n```";
        let plan = parse_plan_response(resp).unwrap();
        assert!(plan.consolidations.is_empty());
    }

    #[test]
    fn parse_plan_response_errors_on_invalid_yaml() {
        // Properly fenced (so extract_yaml_block succeeds) but the inner
        // content is broken YAML — parse step should surface that.
        let err =
            parse_plan_response("```yaml\nconsolidations:\n  - action: merge\n    from: [\n```")
                .unwrap_err();
        assert!(err.contains("parse plan YAML"), "unexpected error: {err}");
    }

    #[test]
    fn cap_plan_truncates_to_max() {
        let plan = ConsolidationPlan {
            consolidations: vec![
                ConsolidationAction::Archive {
                    ward: "a".into(),
                    reason: "".into(),
                },
                ConsolidationAction::Archive {
                    ward: "b".into(),
                    reason: "".into(),
                },
                ConsolidationAction::Archive {
                    ward: "c".into(),
                    reason: "".into(),
                },
            ],
        };
        let capped = cap_plan(plan, 2);
        assert_eq!(capped.consolidations.len(), 2);
    }

    #[test]
    fn render_table_escapes_pipes_and_newlines_in_purpose() {
        let candidates = vec![WardCandidate {
            name: "alpha".to_string(),
            purpose: "Has a | pipe\nand a newline".to_string(),
            use_count: 1,
            state: WardState::Active,
            last_used_at: None,
            age_days: 3,
        }];
        let table = render_table(&candidates);
        assert!(table.contains("Has a \\| pipe and a newline"));
        // Markdown table still has exactly one data row.
        let data_lines = table.lines().filter(|l| l.starts_with("| alpha")).count();
        assert_eq!(data_lines, 1);
    }
}
