//! Tests for the intent agent routing and injection rendering.
//! The old classifier-pipeline tests tested `prompt_typed` with mock LLMs —
//! that path is deleted. The agent flow is integration-tested through
//! the e2e suites.

use gateway_execution::middleware::intent::{
    format_intent_injection, ExecutionApproach, ExecutionStrategy, IntentAnalysis, WardAction,
    WardRecommendation, DEFAULT_INTENT_ANALYSIS_PROMPT,
};

fn analysis(approach: ExecutionApproach) -> IntentAnalysis {
    IntentAnalysis {
        primary_intent: "test-intent".to_string(),
        hidden_intents: vec!["implicit requirement".to_string()],
        solution_path: vec!["step one".to_string(), "step two".to_string()],
        recommended_skills: vec!["coding".to_string()],
        recommended_agents: vec!["research-agent".to_string()],
        recommended_procedures: vec![],
        recommended_capabilities: vec![],
        ward_recommendation: WardRecommendation {
            action: WardAction::UseExisting,
            ward_name: "financial-analysis".to_string(),
            subdirectory: None,
            structure: Default::default(),
            reason: "domain match".to_string(),
        },
        execution_strategy: ExecutionStrategy {
            approach,
            explanation: "test explanation".to_string(),
        },
        complexity: Some("L".to_string()),
        explanation: "because the task requires multi-agent research".to_string(),
        pinned_procedure: None,
    }
}

#[test]
fn graph_injection_includes_planner_and_ward() {
    let injection =
        format_intent_injection(&analysis(ExecutionApproach::Graph), Some("do the thing"));
    assert!(injection.contains("## Task Analysis"));
    assert!(injection.contains("Goal: test-intent"));
    assert!(injection.contains("Requirements (implicit):"));
    assert!(injection.contains("implicit requirement"));
    assert!(injection.contains("financial-analysis"));
    assert!(injection.contains("planner-agent") || injection.contains("Approach:"));
}

#[test]
fn simple_injection_includes_fast_path() {
    let injection =
        format_intent_injection(&analysis(ExecutionApproach::Simple), Some("quick question"));
    assert!(injection.contains("## Task Analysis"));
    assert!(injection.contains("Goal: test-intent"));
    assert!(injection.contains("Fast path"));
}

#[test]
fn rubric_names_research_triggers() {
    let prompt = DEFAULT_INTENT_ANALYSIS_PROMPT;
    assert!(prompt.contains("in-depth research"));
    assert!(prompt.contains("comparative analysis"));
    assert!(prompt.contains("ALWAYS graph"));
    assert!(prompt.contains("submit_intent"));
    assert!(prompt.contains("list_skills"));
    assert!(prompt.contains("search_procedures"));
}

#[test]
fn contract_has_new_fields() {
    // Proves the richer contract round-trips through serde
    let a = analysis(ExecutionApproach::Graph);
    let json = serde_json::to_string(&a).unwrap();
    assert!(json.contains("solution_path"));
    assert!(json.contains("recommended_procedures"));
    assert!(json.contains("complexity"));
    assert!(json.contains("explanation"));
    let back: IntentAnalysis = serde_json::from_str(&json).unwrap();
    assert_eq!(back.solution_path, a.solution_path);
    assert_eq!(back.complexity, a.complexity);
}
