//! Intent injection — render the routed decision for the root system prompt.
//!
//! The runtime enforces what it can (tool filtering, capability wiring,
//! planning gate). This renders only what the model must be told: the goal,
//! implicit requirements, one approach directive, compact resource
//! candidates, and — when deterministic — the pinned procedure invocation.

use super::contract::{ExecutionApproach, IntentAnalysis, WardAction};

/// Render the "## Task Analysis" advisory appended to root instructions.
pub fn format_intent_injection(
    analysis: &IntentAnalysis,
    original_message: Option<&str>,
) -> String {
    let mut out = String::from("\n\n## Task Analysis\n\n");

    if let Some(msg) = original_message {
        out.push_str(&format!("**Original Request:** {}\n", msg));
    }
    out.push_str(&format!("**Goal:** {}\n", analysis.primary_intent));

    if !analysis.hidden_intents.is_empty() {
        out.push_str("\n**Requirements (implicit):**\n");
        for h in &analysis.hidden_intents {
            out.push_str(&format!("- {}\n", h));
        }
    }

    // Pinned procedure: a deterministic macro match routes here — the model
    // invokes it directly, in the procedure's home ward (which may differ
    // from the session's ward; `run_procedure` resolves by name globally).
    if let Some(procedure) = &analysis.pinned_procedure {
        let ward_note = procedure
            .ward_id
            .as_deref()
            .map(|ward| format!(" (home ward: `{ward}`)"))
            .unwrap_or_default();
        out.push_str(&format!(
            "\n**Proven procedure matched this request**{ward_note}. Call \
             `run_procedure(name=\"{}\")` directly — do not replan it.\n",
            procedure.name
        ));
        return out;
    }

    if analysis.execution_strategy.approach == ExecutionApproach::Simple {
        out.push_str(
            "\n**Fast path:** This is a simple one-shot task. Work in the root \
             execution — use memory, graph, direct tools, agents, and relevant \
             skills as the task requires, then call `respond` when the answer \
             is ready.\n",
        );
        // Soft ward note: when the classifier matched an existing ward, work
        // products belong there (read-only answers may skip it).
        if analysis.ward_recommendation.action == WardAction::UseExisting
            && analysis.ward_recommendation.reason != "Trivial message"
        {
            out.push_str(&format!(
                "\n**Ward:** File-producing work belongs in the existing `{}` ward.\n",
                analysis.ward_recommendation.ward_name
            ));
        }
        append_resources(&mut out, analysis);
        return out;
    }

    // Graph posture, warm ward: delegate the WHOLE task to the ward-agent in
    // one call; it plans and executes internally.
    if analysis.ward_recommendation.action == WardAction::UseExisting {
        let ward = analysis.ward_recommendation.ward_name.as_str();
        let mut ward_task = original_message
            .map(str::to_string)
            .unwrap_or_else(|| analysis.primary_intent.clone());
        for h in &analysis.hidden_intents {
            ward_task.push_str(&format!("\n- also: {}", h));
        }
        let assignment = analysis
            .recommended_capabilities
            .iter()
            .find(|assignment| assignment.agent_id == format!("ward:{ward}"));
        let capability_args = assignment.map_or_else(String::new, |assignment| {
            format!(
                ", skills={}, mcps={}",
                serde_json::to_string(&assignment.skills).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&assignment.mcps).unwrap_or_else(|_| "[]".to_string()),
            )
        });
        out.push_str(&format!(
            "\n**Required action:** This task belongs to the existing `{ward}` ward.\n\
             1. Delegate the ENTIRE task to the ward-agent in ONE call and wait \
             for its result:\n\
             ```\n\
             delegate_to_agent(agent_id=\"ward:{ward}\", task=\"{ward_task}\", wait_for_result=true{capability_args})\n\
             ```\n\
             The `ward:{ward}` agent plans and executes the whole task internally and returns \
             a finished result. Do NOT call `ward(action=\"use\")`. Do NOT delegate to \
             `planner-agent`. Do NOT plan or manage steps yourself. When the ward-agent \
             returns, synthesize its result and call `respond`.\n"
        ));
        return out;
    }

    // Graph posture, cold ward: establish the workspace first — the runtime's
    // planning gate blocks everything else until the ward exists.
    let wr = &analysis.ward_recommendation;
    out.push_str(&format!(
        "\n**Required workspace:** Your first tool call MUST be \
         `ward(action=\"{}\", name=\"{}\")`. The ward name `{}` is mandatory — \
         do not rename it to a task-specific alternative. Reason: {}\n",
        if wr.action == WardAction::UseExisting {
            "use"
        } else {
            "create"
        },
        wr.ward_name,
        wr.ward_name,
        wr.reason
    ));
    if let Some(sub) = &wr.subdirectory {
        out.push_str(&format!(
            "  Place task-specific work under subdirectory `{}/` within that ward.\n",
            sub
        ));
    }
    append_resources(&mut out, analysis);
    out.push_str("\n**Ward Rule:** All file-producing work happens inside the ward. Enter it before delegating. Read AGENTS.md to know what exists — reuse before creating.\n");
    out.push_str(&format!(
        "\n**Approach:** Complex task requiring multi-step execution. The planner \
         (started automatically after ward entry) returns a structured execution \
         plan; execute it by delegating each step briefing to its assigned agent \
         with `mode=\"step_executor\"`. Do NOT delegate to `planner-agent` yourself \
         — the system starts it after the ward exists.\n\nPlanner context:\n{}\n",
        format_planner_task(analysis, original_message)
    ));
    out
}

/// Compact candidate listing — what the model may load/delegate. Retrieved,
/// not judged; the tool catalog and actor filtering remain authoritative.
fn append_resources(out: &mut String, analysis: &IntentAnalysis) {
    if !analysis.recommended_skills.is_empty() || !analysis.recommended_agents.is_empty() {
        out.push_str("\n**Suggested resources:**\n");
        for skill in &analysis.recommended_skills {
            out.push_str(&format!("- skill: `{}` (load with load_skill)\n", skill));
        }
        for agent in &analysis.recommended_agents {
            out.push_str(&format!(
                "- agent: `{}` (delegate with delegate_to_agent)\n",
                agent
            ));
        }
    }
}

/// The planner's stable task context. Bootstrap stores the same content in
/// the cold-graph planning gate; WardTool appends the active ward when it
/// consumes that gate.
#[must_use]
pub fn format_planner_task(analysis: &IntentAnalysis, original_message: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(msg) = original_message {
        out.push_str(&format!("Original request: {}\n", msg));
    }
    out.push_str(&format!("Intent: {}\n", analysis.primary_intent));
    let wr = &analysis.ward_recommendation;
    out.push_str(&format!("Ward: {} ({})", wr.ward_name, wr.action));
    if let Some(sub) = &wr.subdirectory {
        out.push_str(&format!("; subdirectory: {}", sub));
    }
    out.push_str(".\n");
    if !analysis.hidden_intents.is_empty() {
        out.push_str("Hidden requirements:\n");
        for requirement in &analysis.hidden_intents {
            out.push_str(&format!("- {}\n", requirement));
        }
    }
    out
}
