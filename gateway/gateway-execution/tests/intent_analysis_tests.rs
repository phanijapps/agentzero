//! E2E integration tests for the intent analysis enrichment pipeline.
//!
//! These tests verify the full flow:
//!   analyze_intent -> IntentAnalysis -> format_intent_injection

use agent_runtime::{ChatMessage, ChatResponse, LlmClient, LlmError, StreamCallback};
use async_trait::async_trait;
use gateway_execution::middleware::intent_analysis::{
    analyze_intent, format_intent_injection, ExecutionApproach, WardAction,
    DEFAULT_INTENT_ANALYSIS_PROMPT,
};
use serde_json::Value;
use zbot_stores::MemoryFactStore;

// ===========================================================================
// Mock LLM clients
// ===========================================================================

struct MockLlmClient {
    response: String,
}

#[async_trait]
impl LlmClient for MockLlmClient {
    fn model(&self) -> &str {
        "mock"
    }
    fn provider(&self) -> &str {
        "mock"
    }
    async fn chat(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        Ok(ChatResponse {
            content: self.response.clone(),
            tool_calls: None,
            reasoning: None,
            usage: None,
        })
    }
    async fn chat_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Value>,
        _callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        Err(LlmError::ApiError(
            "chat_stream not used by typed intent analysis".into(),
        ))
    }
}

struct FailingLlmClient;

#[async_trait]
impl LlmClient for FailingLlmClient {
    fn model(&self) -> &str {
        "failing-mock"
    }
    fn provider(&self) -> &str {
        "mock"
    }
    async fn chat(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        Err(LlmError::ApiError("Service unavailable".into()))
    }
    async fn chat_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Value>,
        _callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        Err(LlmError::ApiError("Service unavailable".into()))
    }
}

// ===========================================================================
// Mock fact store
// ===========================================================================

struct MockFactStore;

#[async_trait]
impl MemoryFactStore for MockFactStore {
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
        Ok(serde_json::json!({"status": "ok"}))
    }

    async fn recall_facts(
        &self,
        _agent_id: &str,
        _query: &str,
        _limit: usize,
    ) -> Result<Value, String> {
        Ok(serde_json::json!({"results": []}))
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn complex_analysis_json() -> String {
    serde_json::json!({
        "primary_intent": "financial_analysis",
        "hidden_intents": [
            "Compare historical performance across asset classes",
            "Identify tax-loss harvesting opportunities",
            "Generate risk-adjusted return projections"
        ],
        "recommended_skills": ["web-search", "code-exec", "file-write"],
        "recommended_agents": ["researcher", "analyst"],
        "ward_recommendation": {
            "action": "create_new",
            "ward_name": "financial-analysis",
            "subdirectory": "portfolio-review",
            "reason": "New domain for financial work"
        },
        "execution_strategy": {
            "approach": "graph",
            "graph": {
                "nodes": [
                    {"id": "A", "task": "Research current financial data", "agent": "researcher", "skills": ["web-search"]},
                    {"id": "B", "task": "Evaluate investment options", "agent": "analyst", "skills": ["code-exec"]},
                    {"id": "C", "task": "Synthesize findings", "agent": "analyst", "skills": ["file-write"]},
                    {"id": "D", "task": "Quality verification", "agent": "root", "skills": []},
                    {"id": "E", "task": "Fix gaps", "agent": "analyst", "skills": ["web-search", "code-exec"]}
                ],
                "edges": [
                    {"from": "A", "to": "B"},
                    {"from": "B", "to": "C"},
                    {"from": "C", "to": "D"},
                    {"from": "D", "conditions": [
                        {"when": "all checks pass", "to": "END"},
                        {"when": "gaps or errors found", "to": "E"}
                    ]},
                    {"from": "E", "to": "D"}
                ]
            },
            "explanation": "Research feeds into analysis, then synthesis. Quality gate loops back capped at 2 cycles."
        }
    })
    .to_string()
}

// ===========================================================================
// Tests
// ===========================================================================

/// Full happy-path: analyze_intent -> format_intent_injection.
#[tokio::test]
async fn test_full_enrichment_flow() {
    let mock = MockLlmClient {
        response: complex_analysis_json(),
    };
    let fact_store = MockFactStore;

    let analysis = analyze_intent(
        std::sync::Arc::new(mock),
        "Analyze my investment portfolio",
        &fact_store,
        None,
        DEFAULT_INTENT_ANALYSIS_PROMPT,
        &[],
        None,
        &[],
    )
    .await
    .expect("analyze_intent should succeed with valid JSON");

    assert_eq!(analysis.primary_intent, "financial_analysis");
    assert_eq!(analysis.hidden_intents.len(), 3);
    assert!(analysis.hidden_intents[0].contains("historical performance"));
    assert_eq!(
        analysis.recommended_skills,
        vec!["web-search", "code-exec", "file-write"]
    );
    assert_eq!(analysis.recommended_agents, vec!["researcher", "analyst"]);
    assert_eq!(
        analysis.execution_strategy.approach,
        ExecutionApproach::Graph
    );

    let graph = analysis
        .execution_strategy
        .graph
        .as_ref()
        .expect("graph should be present");
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.edges.len(), 5);

    // Verify injection formatting
    let injection = format_intent_injection(&analysis, None, None);
    assert!(injection.contains("## Task Analysis"));
    assert!(injection.contains("financial-analysis"));
    assert!(injection.contains("planner-agent"));
}

/// LLM call failure should degrade to a simple fallback analysis.
#[tokio::test]
async fn test_graceful_degradation_on_llm_failure() {
    let client = FailingLlmClient;
    let fact_store = MockFactStore;

    let result = analyze_intent(
        std::sync::Arc::new(client),
        "Create a dashboard for monitoring server metrics",
        &fact_store,
        None,
        DEFAULT_INTENT_ANALYSIS_PROMPT,
        &[],
        None,
        &[],
    )
    .await;

    let analysis = result.expect("LLM failure should fall back, not abort execution");
    assert_eq!(
        analysis.execution_strategy.approach,
        ExecutionApproach::Simple
    );
    assert_eq!(analysis.ward_recommendation.action, WardAction::CreateNew);
    assert_eq!(analysis.ward_recommendation.ward_name, "general");
}

/// Malformed LLM output should degrade to a simple fallback analysis.
#[tokio::test]
async fn test_graceful_degradation_on_malformed_json() {
    let mock = MockLlmClient {
        response: "I'm not sure what you mean.".to_string(),
    };
    let fact_store = MockFactStore;

    let result = analyze_intent(
        std::sync::Arc::new(mock),
        "Do something",
        &fact_store,
        None,
        DEFAULT_INTENT_ANALYSIS_PROMPT,
        &[],
        None,
        &[],
    )
    .await;

    let analysis = result.expect("malformed output should fall back, not abort execution");
    assert_eq!(
        analysis.execution_strategy.approach,
        ExecutionApproach::Simple
    );
    assert_eq!(analysis.ward_recommendation.action, WardAction::CreateNew);
    assert_eq!(analysis.ward_recommendation.ward_name, "general");
}

/// Simple strategy without a graph should parse correctly.
#[tokio::test]
async fn test_simple_request_no_graph() {
    let simple_json = serde_json::json!({
        "primary_intent": "greeting",
        "hidden_intents": [],
        "recommended_skills": [],
        "recommended_agents": [],
        "ward_recommendation": {
            "action": "use_existing",
            "ward_name": "scratch",
            "subdirectory": null,
            "reason": "Simple greeting needs no dedicated ward"
        },
        "execution_strategy": {
            "approach": "simple",
            "explanation": "Simple greeting, no orchestration needed"
        }
    })
    .to_string();

    let mock = MockLlmClient {
        response: simple_json,
    };
    let fact_store = MockFactStore;

    let analysis = analyze_intent(
        std::sync::Arc::new(mock),
        "What is the weather forecast for this weekend",
        &fact_store,
        None,
        DEFAULT_INTENT_ANALYSIS_PROMPT,
        &[],
        None,
        &[],
    )
    .await
    .expect("should parse simple intent");

    assert_eq!(analysis.primary_intent, "greeting");
    assert_eq!(
        analysis.execution_strategy.approach,
        ExecutionApproach::Simple
    );
    assert!(analysis.execution_strategy.graph.is_none());
}

/// Verify skills and agents are correctly parsed from complex analysis.
#[tokio::test]
async fn test_skills_recommended() {
    let mock = MockLlmClient {
        response: complex_analysis_json(),
    };
    let fact_store = MockFactStore;

    let analysis = analyze_intent(
        std::sync::Arc::new(mock),
        "Analyze my portfolio",
        &fact_store,
        None,
        DEFAULT_INTENT_ANALYSIS_PROMPT,
        &[],
        None,
        &[],
    )
    .await
    .expect("should succeed");

    assert_eq!(
        analysis.recommended_skills,
        vec!["web-search", "code-exec", "file-write"]
    );
    assert_eq!(analysis.recommended_agents, vec!["researcher", "analyst"]);
    assert_eq!(
        analysis.execution_strategy.approach,
        ExecutionApproach::Graph
    );
}
