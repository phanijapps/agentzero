//! Resolved execution inputs, independent of the selected execution loop.

use std::{collections::HashSet, sync::Arc};

use super::{ExecutorConfig, RecallHook};
use crate::steering::{SteeringHandle, SteeringQueue};
use crate::{llm::LlmClient, mcp::McpManager, middleware::MiddlewarePipeline, tools::ToolRegistry};
mod mcp_tools;

/// Session setup data. This type does not execute model or tool turns.
pub struct PreparedExecution {
    /// Adapter construction metadata, separate from shared execution policy.
    pub rig_config: Option<crate::rig_adapter::RigAgentConfig>,
    pub config: ExecutorConfig,
    pub llm_client: Arc<dyn LlmClient>,
    pub tool_registry: Arc<ToolRegistry>,
    pub mcp_manager: Arc<McpManager>,
    pub middleware_pipeline: Arc<MiddlewarePipeline>,
    pub recall: Option<(RecallHook, u32, HashSet<String>)>,
    pub steering_queue: Option<SteeringQueue>,
    mcp_tools: Vec<Arc<dyn agent_primitives::Tool>>,
}

impl PreparedExecution {
    pub fn new(
        config: ExecutorConfig,
        llm_client: Arc<dyn LlmClient>,
        tool_registry: Arc<ToolRegistry>,
        mcp_manager: Arc<McpManager>,
        middleware_pipeline: Arc<MiddlewarePipeline>,
    ) -> Self {
        Self {
            rig_config: None,
            config,
            llm_client,
            tool_registry,
            mcp_manager,
            middleware_pipeline,
            recall: None,
            steering_queue: None,
            mcp_tools: Vec::new(),
        }
    }

    pub fn config(&self) -> &ExecutorConfig {
        &self.config
    }

    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    pub fn model_visible_tools(&self) -> Vec<Arc<dyn agent_primitives::Tool>> {
        if !self.config.tools_enabled {
            return Vec::new();
        }
        self.tool_registry
            .get_all()
            .iter()
            .chain(self.mcp_tools.iter())
            .filter(|tool| !self.config.model_hidden_tools.contains(tool.name()))
            .cloned()
            .collect()
    }

    /// Discover only configured servers and freeze exact dispatch bindings.
    /// On an ambiguous namespace, remove any previously prepared inventory.
    pub async fn resolve_mcp_tools(&mut self) -> Result<(), super::ExecutorError> {
        self.mcp_tools.clear();
        self.mcp_tools = mcp_tools::resolve(self).await?;
        Ok(())
    }

    pub fn set_recall_hook(
        &mut self,
        hook: RecallHook,
        every_n_turns: u32,
        initial_keys: HashSet<String>,
    ) {
        self.recall = Some((hook, every_n_turns, initial_keys));
    }

    pub fn enable_steering(&mut self) -> SteeringHandle {
        let (queue, handle) = SteeringQueue::new();
        self.steering_queue = Some(queue);
        handle
    }
}
