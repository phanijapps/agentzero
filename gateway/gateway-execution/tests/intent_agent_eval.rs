//! Integration test for the intent agent using the real Ollama provider.
//! Run: cargo test -p gateway-execution --features test-stubs --test intent_agent_eval -- --nocapture --ignored

use agent_primitives::vault_paths::SharedVaultPaths;
use agent_primitives::vault_paths::VaultPaths;
use gateway_execution::middleware::intent::agent::{run_intent_agent, IntentAgentDeps};
use gateway_services::providers::Provider;
use serde_json::Value;
use std::sync::Arc;
use zbot_stores::MemoryFactStore;

// ---------------------------------------------------------------------------
// Mock fact store — returns static indexed resources
// ---------------------------------------------------------------------------

struct MockFactStore;

#[async_trait::async_trait]
impl MemoryFactStore for MockFactStore {
    async fn save_fact(
        &self,
        _a: &str,
        _b: &str,
        _c: &str,
        _d: &str,
        _e: f64,
        _f: Option<&str>,
        _g: Option<chrono::DateTime<chrono::Utc>>,
    ) -> zbot_stores_traits::StoreResult<Value> {
        Ok(serde_json::json!({}))
    }

    async fn recall_facts(
        &self,
        _agent: &str,
        query: &str,
        _limit: usize,
    ) -> zbot_stores_traits::StoreResult<Value> {
        // Return static results so MemorySearchTool has data
        let _q = query.to_lowercase();
        let mut results = Vec::new();

        // Always return something useful regardless of query
        results.push(serde_json::json!({
            "key": "skill:web-search",
            "content": "Search the web for current information on any topic",
            "category": "skill" }));
        results.push(serde_json::json!({
            "key": "skill:coding",
            "content": "Write and execute code for data analysis",
            "category": "skill" }));
        results.push(serde_json::json!({
            "key": "agent:research-agent",
            "content": "Web search and information gathering specialist",
            "category": "agent" }));
        results.push(serde_json::json!({
            "key": "agent:writing-agent",
            "content": "Creates formatted documents and reports",
            "category": "agent" }));
        results.push(serde_json::json!({
            "key": "ward:financial-analysis",
            "content": "Financial analysis and market research ward",
            "category": "ward" }));
        Ok(serde_json::json!({ "results": results }))
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

fn ollama_provider() -> Provider {
    Provider {
        id: Some("provider-ollama".to_string()),
        name: "Ollama".to_string(),
        description: "local".to_string(),
        api_key: "ollama".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        models: vec!["deepseek-v4-flash:cloud".to_string()],
        embedding_models: None,
        embedding_dimensions: None,
        verified: Some(true),
        is_default: true,
        created_at: None,
        max_concurrent_requests: None,
        context_window: Some(32_768),
        default_model: Some("deepseek-v4-flash:cloud".to_string()),
        rate_limits: None,
        model_configs: None,
    }
}

fn deps(tmp: &tempfile::TempDir) -> IntentAgentDeps {
    let paths: SharedVaultPaths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();

    IntentAgentDeps {
        fact_store: Arc::new(MockFactStore),
        procedure_store: None,
        paths,
        provider: ollama_provider(),
        model: "deepseek-v4-flash:cloud".to_string(),
        max_tokens: 2000,
    }
}

#[tokio::test]
#[ignore = "requires Ollama on localhost:11404"]
async fn intent_agent_comprehensive_research() {
    let tmp = tempfile::tempdir().unwrap();
    let deps = deps(&tmp);

    let result = run_intent_agent(
        &deps,
        "Perform comprehensive analysis of renewable energy sector trends and provide investment insights",
    )
    .await;

    match result {
        Some(analysis) => {
            println!("✓ primary_intent: {}", analysis.primary_intent);
            println!("  approach: {:?}", analysis.execution_strategy.approach);
            println!("  complexity: {:?}", analysis.complexity);
            println!("  solution_path: {:?}", analysis.solution_path);
            println!("  explanation: {}", analysis.explanation);
            assert!(!analysis.primary_intent.is_empty(), "primary_intent empty");
        }
        None => panic!("intent agent returned None"),
    }
}

#[tokio::test]
#[ignore = "requires Ollama on localhost:11434"]
async fn intent_agent_simple_question() {
    let tmp = tempfile::tempdir().unwrap();
    let deps = deps(&tmp);

    let result = run_intent_agent(&deps, "what is 2+2").await;
    match result {
        Some(a) => println!(
            "✓ simple: {} → {:?}",
            a.primary_intent, a.execution_strategy.approach
        ),
        None => println!("✓ returned None (acceptable for simple)"),
    }
}

#[tokio::test]
#[ignore = "requires Ollama on localhost:11434"]
async fn debug_raw_model_output() {
    let tmp = tempfile::tempdir().unwrap();
    let deps = deps(&tmp);

    let llm_config = agent_runtime::LlmConfig::new(
        deps.provider.base_url.clone(),
        deps.provider.api_key.clone(),
        deps.model.clone(),
        "provider-ollama".to_string(),
    )
    .with_max_tokens(2000);
    let client: Arc<dyn agent_runtime::llm::LlmClient> =
        Arc::new(agent_runtime::OpenAiClient::new(llm_config).unwrap());

    // Plain chat call — no response_format, no tools
    let msgs = vec![agent_runtime::ChatMessage::system(
        "You are an intent analyzer. Available resources:\n- [skill] web-search: Search the web\n- [agent] research-agent: Web research specialist\n\nRespond with ONLY a JSON object with these fields: primary_intent, hidden_intents, solution_path, recommended_skills, recommended_agents, recommended_procedures, recommended_capabilities, ward_recommendation (object with action, ward_name, reason), execution_strategy (object with approach, explanation), complexity, explanation. No markdown, no prose, just JSON.".to_string(),
    ), agent_runtime::ChatMessage::user(
        "Perform comprehensive analysis of renewable energy sector trends".to_string(),
    )];

    let response = client.chat(msgs, None).await.unwrap();
    println!("RAW_CONTENT: [{}]", response.content);
    println!(
        "RAW_REASONING: [{}]",
        response.reasoning.unwrap_or_default()
    );
    println!("CONTENT_LEN: {}", response.content.len());
}
