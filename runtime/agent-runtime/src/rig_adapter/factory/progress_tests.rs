use super::*;
use crate::llm::{StreamCallback, TokenUsage};
use crate::{
    AgentEngine, ChatMessage, ChatResponse, ExecutorConfig, LlmClient, LlmConfig, LlmError,
    McpManager, MiddlewarePipeline, ToolRegistry,
};
use serde_json::{json, Value};
use std::sync::Mutex;

struct Provider {
    requests: Mutex<Vec<Vec<ChatMessage>>>,
    turns: usize,
    usage: Vec<Option<u32>>,
    name: &'static str,
}
#[async_trait::async_trait]
impl LlmClient for Provider {
    fn model(&self) -> &str {
        "fixture"
    }
    fn provider(&self) -> &str {
        "fixture"
    }
    async fn chat(&self, _: Vec<ChatMessage>, _: Option<Value>) -> Result<ChatResponse, LlmError> {
        unreachable!()
    }
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        _: Option<Value>,
        _: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(messages);
        let n = requests.len();
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: (n <= self.turns).then(|| {
                vec![crate::ToolCall::new(
                    format!("call-{n}"),
                    self.name.into(),
                    json!({"path":"same-path","content":n}),
                )]
            }),
            reasoning: None,
            usage: self
                .usage
                .get(n - 1)
                .copied()
                .flatten()
                .map(|prompt_tokens| TokenUsage {
                    prompt_tokens,
                    completion_tokens: 1,
                    total_tokens: prompt_tokens + 1,
                    cached_prompt_tokens: None,
                }),
        })
    }
}
struct Effect {
    name: &'static str,
    fail: bool,
}
#[async_trait::async_trait]
impl agent_primitives::Tool for Effect {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "progress fixture"
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({"type":"object","properties":{}}))
    }
    async fn execute(
        &self,
        _: Arc<dyn agent_primitives::ToolContext>,
        _: Value,
    ) -> Result<Value, agent_primitives::error::AgentError> {
        if self.fail {
            Err(agent_primitives::error::AgentError::Tool(
                "same failure".into(),
            ))
        } else {
            Ok(json!("ok"))
        }
    }
}
async fn run(
    cfg: ExecutorConfig,
    turns: usize,
    usage: Vec<Option<u32>>,
    name: &'static str,
    fail: bool,
) -> (Result<String, crate::ExecutorError>, Arc<Provider>) {
    let provider = Arc::new(Provider {
        requests: Mutex::new(Vec::new()),
        turns,
        usage,
        name,
    });
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Effect { name, fail }));
    let prepared = PreparedExecution::new(
        cfg,
        provider.clone(),
        Arc::new(registry),
        Arc::new(McpManager::new()),
        Arc::new(MiddlewarePipeline::new()),
    );
    let rig = RigAgentConfig::new(
        "actor",
        "Actor",
        "fixture",
        "",
        super::super::RigModelConfig::from_llm_config(
            &LlmConfig::new(
                "http://unused".into(),
                String::new(),
                "fixture".into(),
                "fixture".into(),
            ),
            8192,
        ),
    );
    (
        build_engine(prepared, rig).execute("local", &[]).await,
        provider,
    )
}
fn config() -> ExecutorConfig {
    ExecutorConfig::new("actor".into(), "fixture".into(), "fixture".into())
}
fn count(messages: &[ChatMessage], needle: &str) -> usize {
    messages
        .iter()
        .filter(|m| m.text_content().contains(needle))
        .count()
}

#[tokio::test]
async fn planless_calls_receive_one_planning_advisory_after_five_tools() {
    let (result, provider) = run(config(), 7, vec![], "effect", false).await;
    result.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(count(&requests[4], "without creating a plan"), 0);
    assert_eq!(count(&requests[5], "without creating a plan"), 1);
    assert_eq!(count(&requests[7], "without creating a plan"), 1);
}

#[tokio::test]
async fn turn_and_context_nudges_use_real_requests_without_dummy_iterations() {
    let mut cfg = config();
    cfg.turn_budget = 2;
    cfg.max_turns = 4;
    cfg.context_window_tokens = 5000;
    let (result, provider) = run(cfg, 2, vec![Some(4900), None, None], "effect", false).await;
    result.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(count(&requests[1], "Wrap up your current work"), 1);
    assert_eq!(count(&requests[1], "Context is getting full"), 1);
    assert_eq!(count(&requests[2], "Context is getting full"), 1);
}

#[tokio::test]
async fn cumulative_usage_does_not_trigger_context_warning() {
    let mut cfg = config();
    cfg.context_window_tokens = 5000;
    let (result, provider) = run(cfg, 3, vec![Some(2000); 4], "effect", false).await;
    result.unwrap();
    assert!(provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|m| count(m, "Context is getting full") == 0));
}

#[tokio::test]
async fn repeated_errors_and_same_path_writes_trip_existing_safety_valve() {
    for (name, fail) in [("effect", true), ("write_file", false)] {
        let (result, provider) = run(config(), 20, vec![], name, fail).await;
        assert!(
            matches!(
                result,
                Err(crate::ExecutorError::MaxIterationsNeedsIntervention { .. })
            ),
            "{name}: {result:?}"
        );
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 11);
        assert_eq!(count(&requests[10], "repeating similar actions"), 1);
    }
}

#[tokio::test]
async fn complexity_soft_and_urgent_budgets_remain_advisory() {
    let mut cfg = config();
    cfg.complexity = Some("S".into());
    let (result, provider) = run(cfg, 15, vec![], "effect", false).await;
    result.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 16);
    assert_eq!(count(&requests[11], "Wrap up or simplify"), 1);
    assert_eq!(count(&requests[14], "Respond NOW"), 1);
}
