//! Construct Rig directly from resolved runtime inputs, never another executor.

use super::{engine::RigAgentEngine, model::LlmCompletionModel, RigAgentConfig, RigToolAdapter};
use crate::{engine::PreparedExecution, tools::ToolContext};
use std::sync::Arc;

#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod live_context_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod progress_tests;
#[cfg(test)]
mod result_tests;
#[cfg(test)]
mod snapshot_tests;

/// Build the Rig loop with the actor-filtered inventory and effective prompt.
/// The engine owns the configured MCP sessions for its execution lifetime.
pub fn build_engine(
    prepared: PreparedExecution,
    mut rig_config: RigAgentConfig,
) -> RigAgentEngine<LlmCompletionModel> {
    let tools = prepared
        .model_visible_tools()
        .into_iter()
        .map(RigToolAdapter::boxed)
        .collect();
    let mut cfg = prepared.config;
    let restored = crate::engine::snapshot::restore(&mut cfg.initial_state);
    rig_config.agent_id = cfg.agent_id.clone();
    rig_config.model.model = cfg.model.clone();
    rig_config.model.provider_id = cfg.provider_id.clone();
    // Gateway resolves shards, capability instructions and session context after
    // loading the agent YAML. Those instructions are authoritative at execution.
    rig_config.instructions = cfg.system_instruction.clone().unwrap_or_default();
    let shared = Arc::new(ToolContext::full_with_state(
        cfg.agent_id,
        cfg.conversation_id,
        cfg.skills,
        cfg.initial_state,
    ));
    let policy = Arc::new(super::context_policy::ContextPolicy::new(
        super::context_policy::ContextPolicyConfig {
            provider_id: cfg.provider_id,
            model: cfg.model.clone(),
            system_instruction: cfg.system_instruction,
            input_budget: cfg.context_window_tokens,
            progress: super::progress_policy::ProgressConfig {
                turn_budget: cfg.turn_budget,
                max_turns: cfg.max_turns,
                complexity: cfg.complexity,
                input_budget: cfg.context_window_tokens,
                warn_pct: cfg.compaction_warn_pct,
            },
        },
        prepared.middleware_pipeline,
        shared.clone(),
        super::context_inputs::ContextInputs::new(
            prepared.recall,
            prepared.steering_queue,
            cfg.transform_context,
        ),
        restored,
    ));
    let model = LlmCompletionModel::new(prepared.llm_client, cfg.model)
        .with_single_action_mode(cfg.single_action_mode)
        .with_context_policy(policy.clone());
    RigAgentEngine::with_tool_hooks(
        rig_config,
        model,
        tools,
        shared,
        cfg.before_tool_call,
        cfg.after_tool_call,
    )
    .with_execution_turn_limit(cfg.max_turns)
    .with_context_policy(policy)
    .with_result_context(crate::ToolResultContextConfig {
        max_tool_result_chars: cfg.max_tool_result_chars,
        offload_large_results: cfg.offload_large_results,
        offload_threshold_chars: cfg.offload_threshold_chars,
        offload_dir: cfg.offload_dir,
    })
    .with_mcp_session(prepared.mcp_manager)
}

#[cfg(test)]
mod tests {
    use super::super::RigModelConfig;
    use super::*;
    use crate::{
        engine::{AgentEngine, ExecutorConfig},
        llm::{
            ChatMessage, ChatResponse, LlmClient, LlmConfig, LlmError, StreamCallback, StreamChunk,
        },
        mcp::McpManager,
        middleware::MiddlewarePipeline,
        tools::ToolRegistry,
    };
    use serde_json::{json, Value};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingProvider {
        requests: Mutex<Vec<(Vec<ChatMessage>, Option<Value>)>>,
        terminal_reply: bool,
    }

    #[async_trait::async_trait]
    impl LlmClient for RecordingProvider {
        fn model(&self) -> &str {
            "test"
        }
        fn provider(&self) -> &str {
            "test"
        }
        async fn chat(
            &self,
            _: Vec<ChatMessage>,
            _: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            unreachable!("Rig uses streaming")
        }
        async fn chat_stream(
            &self,
            messages: Vec<ChatMessage>,
            tools: Option<Value>,
            callback: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            self.requests.lock().unwrap().push((messages, tools));
            if self.terminal_reply && self.requests.lock().unwrap().len() == 1 {
                return Ok(ChatResponse {
                    content: String::new(),
                    tool_calls: Some(vec![crate::types::ToolCall {
                        id: "terminal-call".into(),
                        name: "respond".into(),
                        arguments: json!({"message":"terminal answer"}),
                    }]),
                    reasoning: None,
                    usage: Some(crate::llm::TokenUsage {
                        prompt_tokens: 7,
                        completion_tokens: 4,
                        total_tokens: 11,
                        cached_prompt_tokens: None,
                    }),
                });
            }
            callback(StreamChunk::Token("done".into()));
            Ok(ChatResponse {
                content: "done".into(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
    }

    struct NamedTool(&'static str);
    #[async_trait::async_trait]
    impl agent_primitives::Tool for NamedTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "factory test"
        }
        fn parameters_schema(&self) -> Option<Value> {
            Some(json!({"type":"object","properties":{}}))
        }
        async fn execute(
            &self,
            _: Arc<dyn agent_primitives::ToolContext>,
            _: Value,
        ) -> Result<Value, agent_primitives::error::AgentError> {
            Ok(json!("ok"))
        }
    }

    #[tokio::test]
    async fn direct_factory_uses_effective_prompt_and_filters_hidden_tools() {
        assert_factory_policy(true).await;
        assert_factory_policy(false).await;
    }

    #[tokio::test]
    async fn successful_respond_ends_rig_without_another_provider_request() {
        assert_respond_boundary(false).await;
    }

    #[tokio::test]
    async fn denied_respond_allows_the_model_to_recover() {
        assert_respond_boundary(true).await;
    }

    async fn assert_respond_boundary(deny: bool) {
        let provider = Arc::new(RecordingProvider {
            terminal_reply: true,
            ..Default::default()
        });
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(crate::tools::RespondTool::new()));
        let mut config = ExecutorConfig::new("agent".into(), "test".into(), "test".into());
        if deny {
            config.before_tool_call = Some(Arc::new(|_, _| crate::ToolCallDecision::Block {
                reason: "fixture denial".into(),
            }));
        }
        let prepared = PreparedExecution::new(
            config,
            provider.clone(),
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        );
        let rig_config = RigAgentConfig::new(
            "agent",
            "Agent",
            "test",
            "",
            RigModelConfig::from_llm_config(
                &LlmConfig::new(
                    "http://unused".into(),
                    String::new(),
                    "test".into(),
                    "test".into(),
                ),
                8192,
            ),
        );
        let engine = build_engine(prepared, rig_config);
        let mut events = Vec::new();
        engine
            .execute_stream("answer", &[], &mut |event| events.push(event))
            .await
            .unwrap();
        assert_eq!(
            provider.requests.lock().unwrap().len(),
            if deny { 2 } else { 1 }
        );
        assert!(events.iter().any(|event| matches!(
            event,
            crate::StreamEvent::TokenUpdate {
                tokens_in: 7,
                tokens_out: 4,
                ..
            }
        )));
        assert_eq!(
            events.iter().any(|event| matches!(event,
                crate::StreamEvent::ActionRespond { message, .. } if message == "terminal answer"
            )),
            !deny
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::StreamEvent::Done { .. }))
                .count(),
            1
        );
    }

    async fn assert_factory_policy(tools_enabled: bool) {
        let provider = Arc::new(RecordingProvider::default());
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(NamedTool("visible")));
        registry.register(Arc::new(NamedTool("internal_only")));
        let mut config = ExecutorConfig::new("agent".into(), "test".into(), "test".into());
        config.system_instruction = Some("effective resolved session instructions".into());
        config.tools_enabled = tools_enabled;
        config.model_hidden_tools.insert("internal_only".into());
        let prepared = PreparedExecution::new(
            config,
            provider.clone(),
            Arc::new(registry),
            Arc::new(McpManager::new()),
            Arc::new(MiddlewarePipeline::new()),
        );
        let rig_config = RigAgentConfig::new(
            "agent",
            "Agent",
            "test",
            "stale raw YAML instructions",
            RigModelConfig::from_llm_config(
                &LlmConfig::new(
                    "http://unused".into(),
                    String::new(),
                    "test".into(),
                    "test".into(),
                ),
                8192,
            ),
        );
        let engine = build_engine(prepared, rig_config);
        assert_eq!(engine.execute("hello", &[]).await.unwrap(), "done");
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let (messages, tools) = &requests[0];
        let text = messages
            .iter()
            .map(ChatMessage::text_content)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("effective resolved session instructions"));
        assert!(!text.contains("stale raw YAML instructions"));
        if tools_enabled {
            let tools = tools.as_ref().unwrap().as_array().unwrap();
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0]["function"]["name"], "visible");
        } else {
            assert!(tools
                .as_ref()
                .is_none_or(|value| value.as_array().is_some_and(Vec::is_empty)));
        }
    }
}
