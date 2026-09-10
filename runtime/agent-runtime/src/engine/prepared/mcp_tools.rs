//! Frozen, configured-server-only MCP tool bindings for one execution.
use super::PreparedExecution;
use crate::{
    mcp::McpClient,
    tool_schema::{harden_tool_schema, normalize_mcp_parameters, normalize_tool_name},
};
use agent_primitives::{error::AgentError, Tool, ToolContext};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

struct BoundMcpTool {
    advertised_name: String,
    raw_name: String,
    description: String,
    parameters: Value,
    client: Arc<dyn McpClient>,
}

#[async_trait::async_trait]
impl Tool for BoundMcpTool {
    fn name(&self) -> &str {
        &self.advertised_name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(self.parameters.clone())
    }
    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> Result<Value, AgentError> {
        self.client
            .call_tool(&self.raw_name, args)
            .await
            .map_err(|e| AgentError::Mcp(e.to_string()))
    }
}

pub(super) async fn resolve(
    prepared: &PreparedExecution,
) -> Result<Vec<Arc<dyn Tool>>, crate::engine::ExecutorError> {
    if !prepared.config.tools_enabled {
        return Ok(Vec::new());
    }
    let mut names: HashSet<String> = prepared
        .tool_registry
        .get_all()
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect();
    let mut servers = HashSet::new();
    let mut result: Vec<Arc<dyn Tool>> = Vec::new();
    for server_id in &prepared.config.mcps {
        if !servers.insert(server_id) {
            continue;
        }
        let Some(client) = prepared.mcp_manager.get_client(server_id).await else {
            continue;
        };
        let definitions = match client.list_tools().await {
            Ok(tools) => tools,
            Err(_) => {
                prepared.mcp_manager.mark_startup_failed(server_id).await;
                continue;
            }
        };
        for definition in definitions {
            if definition.name.is_empty() {
                continue;
            }
            let advertised_name = format!(
                "{}__{}",
                normalize_tool_name(server_id),
                normalize_tool_name(&definition.name)
            );
            if !names.insert(advertised_name.clone()) {
                return Err(crate::engine::ExecutorError::ConfigError(
                    "Ambiguous MCP tool namespace".into(),
                ));
            }
            if prepared
                .config
                .model_hidden_tools
                .contains(&advertised_name)
            {
                continue;
            }
            result.push(Arc::new(BoundMcpTool {
                advertised_name,
                raw_name: definition.name,
                description: definition.description,
                parameters: harden_tool_schema(normalize_mcp_parameters(definition.parameters)),
                client: client.clone(),
            }));
        }
    }
    Ok(result)
}
