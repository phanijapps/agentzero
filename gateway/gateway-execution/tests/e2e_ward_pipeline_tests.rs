//! End-to-end integration tests for the ward execution pipeline.
//!
//! Tests the infrastructure that keeps breaking:
//! 1. Ward scaffolding scoped to recommended skills
//! 2. Subagent context construction (lean, not bloated)
//! 3. Callback structured result detection
//! 4. Intent injection SDLC pattern

use tempfile::TempDir;

#[test]
fn planning_templates_are_agent_aware_template_directed_and_lint_is_explicit() {
    let composer = include_str!("../../templates/skills/plan-composer/SKILL.md");
    let planner = include_str!("../../templates/agents/planner-agent.md");
    let builder = include_str!("../../templates/agents/builder-agent.md");
    let ward_agent = include_str!("../../templates/ward-agent.md");

    for instructions in [composer, planner] {
        assert!(instructions.contains("recommended agent"));
        assert!(instructions.contains("live agent catalog"));
        assert!(instructions.contains("capabilities"));
        assert!(instructions.contains("replan"));
    }
    for instructions in [composer, planner, builder] {
        assert!(instructions.contains("Active Ward Template"));
        assert!(!instructions.contains("<ward-lint-report>"));
        assert!(!instructions.contains("surface lint nudges"));
    }
    assert!(builder.contains("explicit lint action"));
    assert!(composer.contains("ward(action=\"create_concept\""));
    assert!(composer.contains("`ok: true` and `data.valid: true`"));
    assert!(composer.contains("stale"));
    // Ward-agent doctrine is scoped to its guard-permitted surface: concept
    // actions are named as root-only, conformance lint belongs to the
    // planner (ward-slim P4 audience split).
    assert!(ward_agent.contains("root-only actions"));
    assert!(ward_agent.contains("delegated planner"));
    assert!(!ward_agent.contains("ward(action=\"lint\""));
    for instructions in [composer, ward_agent] {
        assert!(!instructions.contains("src/"));
        assert!(!instructions.contains("data/"));
        assert!(!instructions.contains("reports/"));
        assert!(!instructions.contains("output/"));
        assert!(!instructions.contains("generate navigation"));
        assert!(!instructions.contains("generate backlinks"));
        assert!(!instructions.contains("synchronize task"));
    }
}

// STUB: AC3, AC4, AC5, AC8
#[test]
fn planner_contract_requires_declared_refinement_artifacts_before_returning_steps() {
    let composer = include_str!("../../templates/skills/plan-composer/SKILL.md");
    let planner = include_str!("../../templates/agents/planner-agent.md");
    let spec_builder = include_str!("../../templates/skills/spec-builder/SKILL.md");

    for instructions in [composer, planner, spec_builder] {
        assert!(instructions.contains("concrete refinement slug"));
        assert!(instructions.contains("matching declared"));
        assert!(instructions.contains("before returning execution steps"));
        assert!(instructions.contains("template digest"));
    }
    assert!(planner.contains("successful ward lint"));
    assert!(planner.contains("required specification and plan"));
    assert!(planner.contains("optional repeatable task"));
    assert!(planner.contains("never create placeholder tasks"));
    assert!(spec_builder.contains("does not need to say spec"));
    assert!(composer.contains("no task index"));
    for instructions in [composer, planner, spec_builder] {
        assert!(instructions.contains("role_not_declared"));
        assert!(instructions.contains("absent"));
        assert!(instructions.contains("Never invent a fallback"));
    }
}

// ============================================================================
// 1. WARD SCAFFOLDING — SCOPED TO RECOMMENDED SKILLS
// ============================================================================

/// Scaffolding should only create directories from RECOMMENDED skills,
/// not all skills on disk. Bug: life-os dirs appeared in financial-analysis ward.
#[test]
fn test_scaffolding_scoped_to_recommended_skills() {
    let dir = TempDir::new().unwrap();
    let skills_dir = dir.path().join("skills");

    // Create coding skill with ward_setup
    let coding_dir = skills_dir.join("coding");
    std::fs::create_dir_all(&coding_dir).unwrap();
    std::fs::write(
        coding_dir.join("SKILL.md"),
        r#"---
name: coding
description: Code stuff
ward_setup:
  directories:
    - core/
    - output/
    - specs/
---
Instructions here
"#,
    )
    .unwrap();

    // Create life-os skill with ward_setup (should NOT apply to coding wards)
    let lifeos_dir = skills_dir.join("life-os");
    std::fs::create_dir_all(&lifeos_dir).unwrap();
    std::fs::write(
        lifeos_dir.join("SKILL.md"),
        r#"---
name: life-os
description: Life stuff
ward_setup:
  directories:
    - daily/
    - weekly/
    - projects/
    - areas/
---
Instructions here
"#,
    )
    .unwrap();

    // Scaffold a ward with ONLY coding skill recommended
    let ward_dir = dir.path().join("wards").join("financial-analysis");
    std::fs::create_dir_all(&ward_dir).unwrap();

    // Simulate scoped scaffolding (only from coding skill)
    let setups = gateway_execution::invoke::collect_ward_setups_for_skills(
        &skills_dir,
        &["coding".to_string()],
    );
    gateway_execution::middleware::ward_scaffold::scaffold_ward(
        &ward_dir,
        "financial-analysis",
        &setups,
    );

    // Coding dirs should exist
    assert!(ward_dir.join("core").is_dir(), "core/ should be created");
    assert!(
        ward_dir.join("output").is_dir(),
        "output/ should be created"
    );
    assert!(ward_dir.join("specs").is_dir(), "specs/ should be created");

    // Life-os dirs should NOT exist
    assert!(
        !ward_dir.join("daily").exists(),
        "daily/ should NOT be created — wrong skill"
    );
    assert!(
        !ward_dir.join("weekly").exists(),
        "weekly/ should NOT be created — wrong skill"
    );
    assert!(
        !ward_dir.join("projects").exists(),
        "projects/ should NOT be created — wrong skill"
    );
    assert!(
        !ward_dir.join("areas").exists(),
        "areas/ should NOT be created — wrong skill"
    );
}

/// Scaffolding with life-os skill should create life-os dirs, not coding dirs.
#[test]
fn test_scaffolding_lifeos_skill_creates_lifeos_dirs() {
    let dir = TempDir::new().unwrap();
    let skills_dir = dir.path().join("skills");

    let lifeos_dir = skills_dir.join("life-os");
    std::fs::create_dir_all(&lifeos_dir).unwrap();
    std::fs::write(
        lifeos_dir.join("SKILL.md"),
        r#"---
name: life-os
description: Life stuff
ward_setup:
  directories:
    - daily/
    - weekly/
    - projects/
---
Instructions here
"#,
    )
    .unwrap();

    let ward_dir = dir.path().join("wards").join("personal-life");
    std::fs::create_dir_all(&ward_dir).unwrap();

    let setups = gateway_execution::invoke::collect_ward_setups_for_skills(
        &skills_dir,
        &["life-os".to_string()],
    );
    gateway_execution::middleware::ward_scaffold::scaffold_ward(
        &ward_dir,
        "personal-life",
        &setups,
    );

    assert!(ward_dir.join("daily").is_dir());
    assert!(ward_dir.join("weekly").is_dir());
    assert!(ward_dir.join("projects").is_dir());
    assert!(
        !ward_dir.join("core").exists(),
        "core/ should NOT exist — coding skill not recommended"
    );
}

// ============================================================================
// 2. SUBAGENT CONTEXT — LEAN, NOT BLOATED
// ============================================================================

/// Executor subagent rules should be under 300 bytes.
#[test]
fn test_subagent_rules_are_lean() {
    let rules = gateway_execution::invoke::setup::subagent_rules(
        gateway_execution::invoke::setup::SubagentRole::Executor,
        gateway_execution::delegation::DelegationMode::DirectArtifact,
    );
    let byte_count = rules.len();
    assert!(
        byte_count < 320,
        "Executor rules should be under 300 bytes, got {} bytes:\n{}",
        byte_count,
        rules
    );
    assert!(!rules.contains("AGENTS.md + memory-bank/core_docs.md"));
}

/// Reviewer rules should include RESULT format.
#[test]
fn test_reviewer_rules_include_result_format() {
    let rules = gateway_execution::invoke::setup::subagent_rules(
        gateway_execution::invoke::setup::SubagentRole::Reviewer,
        gateway_execution::delegation::DelegationMode::WardBackedBuild,
    );
    assert!(
        rules.contains("RESULT: APPROVED"),
        "Reviewer rules must mention RESULT: APPROVED"
    );
    assert!(
        rules.contains("RESULT: DEFECTS"),
        "Reviewer rules must mention RESULT: DEFECTS"
    );
}

#[test]
fn executor_rules_are_mode_specific() {
    use gateway_execution::delegation::DelegationMode;
    use gateway_execution::invoke::setup::{subagent_rules, SubagentRole};

    let direct = subagent_rules(SubagentRole::Executor, DelegationMode::DirectArtifact);
    assert!(direct.contains("direct_artifact"));
    assert!(direct.contains("Create the exact requested output files first"));
    assert!(!direct.contains("read AGENTS.md + memory-bank/core_docs.md"));

    let backed = subagent_rules(SubagentRole::Executor, DelegationMode::WardBackedBuild);
    assert!(backed.contains("ward_backed_build"));
    assert!(backed.contains("Read the supplied ward_snapshot"));

    let step = subagent_rules(SubagentRole::Executor, DelegationMode::StepExecutor);
    assert!(step.contains("step_executor"));
    assert!(step.contains("Execute the delegated step spec exactly"));
}

/// Role detection should identify review tasks.
#[test]
fn test_role_detection() {
    use gateway_execution::invoke::setup::{detect_subagent_role, SubagentRole};

    assert_eq!(
        detect_subagent_role("code-agent", "Build the data pipeline"),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role("code-agent", "Review code against specs"),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role("data-analyst", "Validate output quality"),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role("data-analyst", "Run the analysis script"),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role("code-agent", "Evaluate the implementation"),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role(
            "research-agent",
            "Find 10 homes, extract fields, and verify source URLs"
        ),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role("builder-agent", "Write parser and verify output"),
        SubagentRole::Executor
    );
    assert_eq!(
        detect_subagent_role("reviewer-agent", "Review generated output"),
        SubagentRole::Reviewer
    );
    assert_eq!(
        detect_subagent_role(
            "code-agent",
            "Perform a read-only review without changing files"
        ),
        SubagentRole::Reviewer
    );
}

// ============================================================================
// 3. CALLBACK STRUCTURED RESULT DETECTION
// ============================================================================

/// Callback should detect APPROVED and add action hint.
#[test]
fn test_callback_detects_approved() {
    let msg = gateway_execution::delegation::format_callback_message(
        "code-agent",
        "Code looks good. All tests pass.\n\nRESULT: APPROVED",
        "conv-123",
        None,
    );
    assert!(msg.contains("APPROVED"), "Should contain APPROVED");
    assert!(
        msg.contains("Proceed to the next node"),
        "Should suggest proceeding"
    );
}

/// Callback should detect DEFECTS and include defect list.
#[test]
fn test_callback_detects_defects() {
    let msg = gateway_execution::delegation::format_callback_message(
        "data-analyst",
        "Found issues.\n\nRESULT: DEFECTS\n- output.json: RSI value is -5 (severity: high)\n- data.csv: Only 10 rows (severity: medium)",
        "conv-123",
        None,
    );
    assert!(msg.contains("DEFECTS found"), "Should mention DEFECTS");
    assert!(
        msg.contains("RSI value is -5"),
        "Should include defect details"
    );
    assert!(
        msg.contains("Re-delegate to coding agent"),
        "Should suggest re-delegation"
    );
}

/// Callback without RESULT marker should not add action hints.
#[test]
fn test_callback_without_result_no_action() {
    let msg = gateway_execution::delegation::format_callback_message(
        "code-agent",
        "Here is the analysis of the data.\nIt shows interesting patterns.",
        "conv-123",
        None,
    );
    assert!(
        !msg.contains("Action:"),
        "Should not contain Action hint for plain responses"
    );
}

// ============================================================================
// 4. INTENT INJECTION — SDLC PATTERN
// ============================================================================

/// Graph approach should inject SDLC pattern.
#[test]
fn test_intent_injection_sdlc_for_graph() {
    use gateway_execution::middleware::intent_analysis::*;

    let analysis = IntentAnalysis {
        primary_intent: "stock analysis".to_string(),
        hidden_intents: vec!["fetch options data".to_string()],
        recommended_skills: vec!["coding".to_string()],
        recommended_agents: vec!["code-agent".to_string()],
        recommended_capabilities: vec![],
        ward_recommendation: WardRecommendation {
            action: WardAction::CreateNew,
            ward_name: "financial-analysis".to_string(),
            subdirectory: Some("stocks/amd".to_string()),
            structure: Default::default(),
            reason: "domain match".to_string(),
        },
        execution_strategy: ExecutionStrategy {
            approach: ExecutionApproach::Graph,
            graph: None,
            explanation: "Complex analysis".to_string(),
        },
        rewritten_prompt: String::new(),
        procedure_recommendation: None,
    };

    let injection = format_intent_injection(&analysis, None, None);

    // Graph approach should route to planner-agent
    assert!(
        injection.contains("## Task Analysis"),
        "Graph approach should include task analysis"
    );
    assert!(injection.contains("Goal:"), "Should include the goal");
    assert!(
        injection.contains("planner-agent"),
        "Should route to planner for graph tasks"
    );
    assert!(
        injection.contains("Ward Rule:"),
        "Should include ward discipline"
    );
}

/// Simple approach should NOT inject SDLC pattern.
#[test]
fn test_intent_injection_no_sdlc_for_simple() {
    use gateway_execution::middleware::intent_analysis::*;

    let analysis = IntentAnalysis {
        primary_intent: "greeting".to_string(),
        hidden_intents: vec![],
        recommended_skills: vec![],
        recommended_agents: vec![],
        recommended_capabilities: vec![],
        ward_recommendation: WardRecommendation {
            action: WardAction::UseExisting,
            ward_name: "scratch".to_string(),
            subdirectory: None,
            structure: Default::default(),
            reason: "simple".to_string(),
        },
        execution_strategy: ExecutionStrategy {
            approach: ExecutionApproach::Simple,
            graph: None,
            explanation: "Quick question".to_string(),
        },
        rewritten_prompt: String::new(),
        procedure_recommendation: None,
    };

    let injection = format_intent_injection(&analysis, None, None);

    assert!(
        !injection.contains("SDLC Pattern"),
        "Simple approach should NOT include SDLC"
    );
    assert!(
        !injection.contains("tasks.json"),
        "Simple approach should NOT mention tasks.json"
    );
    assert!(
        injection.contains("**Fast path:**"),
        "Simple approach should explicitly route through the direct fast path"
    );
    assert!(
        !injection.contains("delegate_to_agent(agent_id="),
        "Simple approach should not render an executable delegation example"
    );
    assert!(
        !injection.contains("**Required workspace:**"),
        "Simple approach should not force ward entry"
    );
}

/// Ward rules should not have hardcoded domain examples.
#[test]
fn test_ward_rules_domain_agnostic() {
    use gateway_execution::middleware::intent_analysis::*;

    let analysis = IntentAnalysis {
        primary_intent: "test".to_string(),
        hidden_intents: vec![],
        recommended_skills: vec![],
        recommended_agents: vec![],
        recommended_capabilities: vec![],
        ward_recommendation: WardRecommendation {
            action: WardAction::CreateNew,
            ward_name: "test".to_string(),
            subdirectory: None,
            structure: Default::default(),
            reason: "test".to_string(),
        },
        execution_strategy: ExecutionStrategy {
            approach: ExecutionApproach::Simple,
            graph: None,
            explanation: "test".to_string(),
        },
        rewritten_prompt: String::new(),
        procedure_recommendation: None,
    };

    let injection = format_intent_injection(&analysis, None, None);

    // Should NOT have financial domain terms
    assert!(
        !injection.contains("SPY"),
        "Ward rules should not mention SPY"
    );
    assert!(
        !injection.contains("ohlcv"),
        "Ward rules should not mention ohlcv"
    );
    assert!(
        !injection.contains("RSI"),
        "Ward rules should not mention RSI"
    );
    assert!(
        !injection.contains("yfinance"),
        "Ward rules should not mention yfinance"
    );
}
