// ============================================================================
// EXECUTION GRAPH TOOL
// Runtime DAG workflow engine for orchestrating multi-step agent tasks.
// Stores graph state in session state. Works with delegate_to_agent for dispatch.
// ============================================================================

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext};

// ============================================================================
// DATA MODEL
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionGraph {
    pub id: String,
    pub nodes: HashMap<String, GraphNode>,
    pub status: GraphStatus,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GraphStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub agent: String,
    pub task: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "default_depend_mode")]
    pub depend_mode: DependMode,
    #[serde(default)]
    pub when: Option<Condition>,
    #[serde(default)]
    pub inputs: HashMap<String, InputRef>,
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default = "default_timeout_action")]
    pub on_timeout: TimeoutAction,
    #[serde(default)]
    pub status: NodeStatus,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub attempts: u32,
}

impl Default for GraphNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            agent: String::new(),
            task: String::new(),
            depends_on: Vec::new(),
            depend_mode: DependMode::All,
            when: None,
            inputs: HashMap::new(),
            retry: None,
            timeout_seconds: None,
            on_timeout: TimeoutAction::Skip,
            status: NodeStatus::Pending,
            result: None,
            error: None,
            attempts: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    #[default]
    Pending,
    Ready,
    Running,
    Completed,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DependMode {
    #[default]
    All,
    AnyCompleted,
    AnyOne,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    #[serde(rename = "ref")]
    pub ref_node: String,
    #[serde(default = "default_field")]
    pub field: String,
    pub operator: ConditionOp,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOp {
    Contains,
    NotContains,
    Equals,
    Gt,
    Lt,
    Regex,
    LlmEval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputRef {
    pub from: String,
    pub field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_retries")]
    pub max: u32,
    pub on_fail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    #[default]
    Skip,
    Fail,
}

fn default_depend_mode() -> DependMode {
    DependMode::All
}
fn default_timeout_action() -> TimeoutAction {
    TimeoutAction::Skip
}
fn default_field() -> String {
    "result".to_string()
}
fn default_max_retries() -> u32 {
    1
}

// ============================================================================
// READY NODE (returned to agent)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ReadyNode {
    pub id: String,
    pub agent: String,
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_eval_prompt: Option<String>,
}

// ============================================================================
// CONDITION EVALUATOR
// ============================================================================

impl Condition {
    /// Evaluate condition against completed node results.
    /// Returns Ok(true) if passed, Ok(false) if failed.
    /// Returns Err with prompt for llm_eval (agent evaluates).
    pub fn evaluate(
        &self,
        nodes: &HashMap<String, GraphNode>,
    ) -> std::result::Result<bool, String> {
        let node = nodes
            .get(&self.ref_node)
            .ok_or_else(|| format!("Referenced node '{}' not found", self.ref_node))?;

        let field_value = match self.field.as_str() {
            "result" => node.result.clone().unwrap_or_default(),
            "status" => serde_json::to_string(&node.status).unwrap_or_default(),
            "error" => node.error.clone().unwrap_or_default(),
            _ => return Err(format!("Unknown field: {}", self.field)),
        };

        match self.operator {
            ConditionOp::Contains => Ok(field_value.contains(&self.value)),
            ConditionOp::NotContains => Ok(!field_value.contains(&self.value)),
            ConditionOp::Equals => Ok(field_value == self.value),
            ConditionOp::Gt => {
                let a: f64 = field_value.parse().unwrap_or(0.0);
                let b: f64 = self.value.parse().unwrap_or(0.0);
                Ok(a > b)
            }
            ConditionOp::Lt => {
                let a: f64 = field_value.parse().unwrap_or(0.0);
                let b: f64 = self.value.parse().unwrap_or(0.0);
                Ok(a < b)
            }
            ConditionOp::Regex => {
                let re =
                    regex::Regex::new(&self.value).map_err(|e| format!("Invalid regex: {}", e))?;
                Ok(re.is_match(&field_value))
            }
            ConditionOp::LlmEval => Err(format!(
                "LLM_EVAL_REQUIRED: Evaluate this condition and call execute_next with the result. \
                 Question: {} | Value from node '{}': {}",
                self.value, self.ref_node, field_value
            )),
        }
    }
}

// ============================================================================
// GRAPH METHODS
// ============================================================================

impl ExecutionGraph {
    pub fn new(id: String, nodes: Vec<GraphNode>) -> Self {
        let mut node_map = HashMap::new();
        for node in nodes {
            node_map.insert(node.id.clone(), node);
        }
        Self {
            id,
            nodes: node_map,
            status: GraphStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Find nodes that are ready to execute (dependencies met, conditions passed).
    pub fn find_ready_nodes(&mut self) -> Vec<ReadyNode> {
        let mut ready = Vec::new();
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();

        for node_id in &node_ids {
            let node = self.nodes.get(node_id).unwrap().clone();
            if node.status != NodeStatus::Pending {
                continue;
            }

            // Check dependencies
            let deps_met = match node.depend_mode {
                DependMode::All => node.depends_on.iter().all(|dep| {
                    self.nodes.get(dep).is_some_and(|n| {
                        n.status == NodeStatus::Completed || n.status == NodeStatus::Skipped
                    })
                }),
                DependMode::AnyCompleted => {
                    node.depends_on.is_empty()
                        || node.depends_on.iter().any(|dep| {
                            self.nodes
                                .get(dep)
                                .is_some_and(|n| n.status == NodeStatus::Completed)
                        })
                }
                DependMode::AnyOne => {
                    node.depends_on.is_empty()
                        || node.depends_on.iter().any(|dep| {
                            self.nodes.get(dep).is_some_and(|n| {
                                matches!(
                                    n.status,
                                    NodeStatus::Completed
                                        | NodeStatus::Skipped
                                        | NodeStatus::Failed
                                )
                            })
                        })
                }
            };

            if !deps_met {
                continue;
            }

            // Check condition
            if let Some(condition) = &node.when {
                match condition.evaluate(&self.nodes) {
                    Ok(true) => {} // Condition passed, proceed
                    Ok(false) => {
                        // Condition failed — skip node
                        if let Some(n) = self.nodes.get_mut(node_id) {
                            n.status = NodeStatus::Skipped;
                        }
                        continue;
                    }
                    Err(prompt) => {
                        // LLM eval needed — include in ready with the prompt
                        ready.push(ReadyNode {
                            id: node_id.clone(),
                            agent: node.agent.clone(),
                            task: self.resolve_task(&node),
                            llm_eval_prompt: Some(prompt),
                        });
                        continue;
                    }
                }
            }

            // Node is ready — mark it
            if let Some(n) = self.nodes.get_mut(node_id) {
                n.status = NodeStatus::Ready;
            }
            ready.push(ReadyNode {
                id: node_id.clone(),
                agent: node.agent.clone(),
                task: self.resolve_task(&node),
                llm_eval_prompt: None,
            });
        }

        if !ready.is_empty() && self.status == GraphStatus::Pending {
            self.status = GraphStatus::Running;
        }

        // Check if graph is complete (no ready nodes and all terminal)
        if ready.is_empty() {
            let all_done = self.nodes.values().all(|n| {
                matches!(
                    n.status,
                    NodeStatus::Completed | NodeStatus::Skipped | NodeStatus::Failed
                )
            });
            if all_done {
                let any_failed = self.nodes.values().any(|n| n.status == NodeStatus::Failed);
                self.status = if any_failed {
                    GraphStatus::Failed
                } else {
                    GraphStatus::Completed
                };
            }
        }

        ready
    }

    /// Resolve input references in task description, substituting {param} with upstream results.
    fn resolve_task(&self, node: &GraphNode) -> String {
        let mut task = node.task.clone();
        for (param, input_ref) in &node.inputs {
            if let Some(upstream) = self.nodes.get(&input_ref.from) {
                let value = match input_ref.field.as_str() {
                    "result" => upstream.result.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                task = task.replace(&format!("{{{}}}", param), &value);
            }
        }
        task
    }

    /// Mark a node as completed with result.
    pub fn complete_node(&mut self, node_id: &str, result: String) -> bool {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.status = NodeStatus::Completed;
            node.result = Some(result);
            node.attempts += 1;
            true
        } else {
            false
        }
    }

    /// Mark a node as failed, with retry logic.
    pub fn fail_node(&mut self, node_id: &str, error: String) -> bool {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.attempts += 1;

            // Check retry policy
            if let Some(retry) = &node.retry
                && node.attempts < retry.max
            {
                node.status = NodeStatus::Pending; // will be re-evaluated next cycle
                node.error = Some(error);
                return true;
            }

            node.status = NodeStatus::Failed;
            node.error = Some(error);
            true
        } else {
            false
        }
    }

    /// Add a node to the graph dynamically.
    pub fn add_node(&mut self, node: GraphNode) -> bool {
        if self.nodes.contains_key(&node.id) {
            return false;
        }
        self.nodes.insert(node.id.clone(), node);
        true
    }
}

// ============================================================================
// EXECUTION GRAPH TOOL
// ============================================================================

pub struct ExecutionGraphTool;

impl ExecutionGraphTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ExecutionGraphTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ExecutionGraphTool {
    fn name(&self) -> &str {
        "execution_graph"
    }

    fn description(&self) -> &str {
        "Build and execute workflow DAGs for complex multi-step tasks. \
         Create a graph of nodes (agent tasks) with dependencies, conditions, \
         and result routing. Use with delegate_to_agent to dispatch ready nodes."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "execute_next", "status", "add_node"],
                    "description": "Action: create (build graph), execute_next (advance with results), status (check progress), add_node (inject node mid-execution)"
                },
                "graph_id": {
                    "type": "string",
                    "description": "Graph ID (required for execute_next, status, add_node)"
                },
                "nodes": {
                    "type": "array",
                    "description": "Node definitions (for create action)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Unique node identifier" },
                            "agent": { "type": "string", "description": "Agent ID to delegate to" },
                            "task": { "type": "string", "description": "Task description. Use {param} for input references." },
                            "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Upstream node IDs that must complete first" },
                            "depend_mode": { "type": "string", "enum": ["all", "any_completed", "any_one"], "description": "How to evaluate dependencies (default: all)" },
                            "when": {
                                "type": "object",
                                "description": "Conditional execution based on upstream results",
                                "properties": {
                                    "ref": { "type": "string", "description": "Upstream node ID to check" },
                                    "operator": { "type": "string", "enum": ["contains", "not_contains", "equals", "gt", "lt", "regex", "llm_eval"] },
                                    "value": { "type": "string", "description": "Value to compare against" },
                                    "field": { "type": "string", "description": "Field to check: result, status, error (default: result)" }
                                },
                                "required": ["ref", "operator", "value"]
                            },
                            "inputs": {
                                "type": "object",
                                "description": "Map param names to upstream node results. Use {param} in task.",
                                "additionalProperties": {
                                    "type": "object",
                                    "properties": {
                                        "from": { "type": "string" },
                                        "field": { "type": "string" }
                                    }
                                }
                            },
                            "retry": {
                                "type": "object",
                                "properties": {
                                    "max": { "type": "integer", "description": "Max attempts" },
                                    "on_fail": { "type": "string", "description": "Fallback node ID" }
                                }
                            },
                            "timeout_seconds": { "type": "integer" },
                            "on_timeout": { "type": "string", "enum": ["skip", "fail"] }
                        },
                        "required": ["id", "agent", "task"]
                    }
                },
                "completed": {
                    "type": "array",
                    "description": "Node completion results (for execute_next). Include result or error.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Node ID" },
                            "result": { "type": "string", "description": "Subagent result text" },
                            "error": { "type": "string", "description": "Error message if failed" }
                        },
                        "required": ["id"]
                    }
                },
                "node": {
                    "type": "object",
                    "description": "Single node definition (for add_node action)"
                }
            },
            "required": ["action"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Check for error markers from truncated/malformed tool calls
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AgentError::Tool(format!("{}: {}", error_type, message)));
        }

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'action' parameter".to_string()))?;

        match action {
            "create" => self.handle_create(ctx, &args).await,
            "execute_next" => self.handle_execute_next(ctx, &args).await,
            "status" => self.handle_status(ctx, &args).await,
            "add_node" => self.handle_add_node(ctx, &args).await,
            _ => Err(AgentError::Tool(format!(
                "Unknown action: '{}'. Valid: create, execute_next, status, add_node",
                action
            ))),
        }
    }
}

// ============================================================================
// VALIDATION HELPERS
// ============================================================================

fn validate_node_input_refs(node: &GraphNode, node_ids: &HashSet<&str>) -> Result<()> {
    for (param, input_ref) in &node.inputs {
        if !node_ids.contains(input_ref.from.as_str()) {
            return Err(AgentError::Tool(format!(
                "Node '{}' input '{}' references unknown node '{}'",
                node.id, param, input_ref.from
            )));
        }
    }
    Ok(())
}

// ============================================================================
// ACTION HANDLERS
// ============================================================================

impl ExecutionGraphTool {
    async fn handle_create(&self, ctx: Arc<dyn ToolContext>, args: &Value) -> Result<Value> {
        let nodes_val = args.get("nodes").ok_or_else(|| {
            AgentError::Tool("Missing 'nodes' array for create action".to_string())
        })?;

        let nodes: Vec<GraphNode> = serde_json::from_value(nodes_val.clone())
            .map_err(|e| AgentError::Tool(format!("Invalid node definitions: {}", e)))?;

        if nodes.is_empty() {
            return Err(AgentError::Tool("Nodes array cannot be empty".to_string()));
        }

        // Validate: no duplicate IDs
        let mut seen = HashSet::new();
        for node in &nodes {
            if !seen.insert(&node.id) {
                return Err(AgentError::Tool(format!(
                    "Duplicate node ID: '{}'",
                    node.id
                )));
            }
        }

        // Validate: all depends_on references exist
        let node_ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        for node in &nodes {
            for dep in &node.depends_on {
                if !node_ids.contains(dep.as_str()) {
                    return Err(AgentError::Tool(format!(
                        "Node '{}' depends on unknown node '{}'",
                        node.id, dep
                    )));
                }
            }
            // Validate condition references
            if let Some(cond) = &node.when
                && !node_ids.contains(cond.ref_node.as_str())
            {
                return Err(AgentError::Tool(format!(
                    "Node '{}' condition references unknown node '{}'",
                    node.id, cond.ref_node
                )));
            }
            validate_node_input_refs(node, &node_ids)?;
        }

        let graph_id = format!(
            "graph-{}",
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("0")
        );
        let mut graph = ExecutionGraph::new(graph_id.clone(), nodes);
        let ready = graph.find_ready_nodes();

        // Store graph in session state
        self.save_graph(&ctx, &graph_id, &graph)?;

        tracing::info!(graph_id = %graph_id, nodes = %graph.nodes.len(), ready = %ready.len(), "Execution graph created");

        Ok(json!({
            "graph_id": graph_id,
            "total_nodes": graph.nodes.len(),
            "ready_nodes": ready,
            "message": format!("{} node(s) ready to dispatch via delegate_to_agent", ready.len())
        }))
    }

    async fn handle_execute_next(&self, ctx: Arc<dyn ToolContext>, args: &Value) -> Result<Value> {
        let graph_id = args
            .get("graph_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'graph_id' for execute_next".to_string()))?;

        let mut graph = self.load_graph(&ctx, graph_id)?;

        // Process completed nodes
        if let Some(completed) = args.get("completed").and_then(|v| v.as_array()) {
            for entry in completed {
                let id = entry
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Tool("Completed entry missing 'id'".to_string()))?;

                if let Some(error) = entry.get("error").and_then(|v| v.as_str()) {
                    graph.fail_node(id, error.to_string());
                } else {
                    let result = entry
                        .get("result")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    graph.complete_node(id, result);
                }
            }
        }

        let ready = graph.find_ready_nodes();

        // Save updated graph
        self.save_graph(&ctx, graph_id, &graph)?;

        if graph.status == GraphStatus::Completed || graph.status == GraphStatus::Failed {
            let results: HashMap<String, Option<String>> = graph
                .nodes
                .iter()
                .filter(|(_, n)| n.status == NodeStatus::Completed)
                .map(|(id, n)| (id.clone(), n.result.clone()))
                .collect();

            Ok(json!({
                "graph_id": graph_id,
                "status": graph.status,
                "ready_nodes": [],
                "results": results,
                "message": format!("Graph {}", if graph.status == GraphStatus::Completed { "completed successfully" } else { "failed" })
            }))
        } else {
            Ok(json!({
                "graph_id": graph_id,
                "status": graph.status,
                "ready_nodes": ready,
                "message": format!("{} node(s) ready to dispatch", ready.len())
            }))
        }
    }

    async fn handle_status(&self, ctx: Arc<dyn ToolContext>, args: &Value) -> Result<Value> {
        let graph_id = args
            .get("graph_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'graph_id' for status".to_string()))?;

        let graph = self.load_graph(&ctx, graph_id)?;

        let summary: Vec<Value> = graph
            .nodes
            .values()
            .map(|n| {
                json!({
                    "id": n.id,
                    "agent": n.agent,
                    "status": n.status,
                    "result_preview": n.result.as_ref().map(|r| &r[..r.len().min(200)])
                })
            })
            .collect();

        let completed = graph
            .nodes
            .values()
            .filter(|n| n.status == NodeStatus::Completed)
            .count();
        let total = graph.nodes.len();

        Ok(json!({
            "graph_id": graph_id,
            "status": graph.status,
            "progress": format!("{}/{}", completed, total),
            "nodes": summary
        }))
    }

    async fn handle_add_node(&self, ctx: Arc<dyn ToolContext>, args: &Value) -> Result<Value> {
        let graph_id = args
            .get("graph_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'graph_id' for add_node".to_string()))?;

        let node_val = args
            .get("node")
            .ok_or_else(|| AgentError::Tool("Missing 'node' for add_node action".to_string()))?;

        let node: GraphNode = serde_json::from_value(node_val.clone())
            .map_err(|e| AgentError::Tool(format!("Invalid node definition: {}", e)))?;

        let mut graph = self.load_graph(&ctx, graph_id)?;

        let node_id = node.id.clone();
        if !graph.add_node(node) {
            return Err(AgentError::Tool(format!(
                "Node '{}' already exists in graph",
                node_id
            )));
        }

        self.save_graph(&ctx, graph_id, &graph)?;

        tracing::debug!(graph_id = %graph_id, node_id = %node_id, "Node added to execution graph");

        Ok(json!({
            "graph_id": graph_id,
            "added": node_id,
            "total_nodes": graph.nodes.len(),
            "message": format!("Node '{}' added to graph", node_id)
        }))
    }

    // Helpers

    fn load_graph(&self, ctx: &Arc<dyn ToolContext>, graph_id: &str) -> Result<ExecutionGraph> {
        let graph_val = ctx
            .get_state(&format!("app:graph:{}", graph_id))
            .ok_or_else(|| AgentError::Tool(format!("Graph '{}' not found", graph_id)))?;
        serde_json::from_value(graph_val)
            .map_err(|e| AgentError::Tool(format!("Failed to deserialize graph: {}", e)))
    }

    fn save_graph(
        &self,
        ctx: &Arc<dyn ToolContext>,
        graph_id: &str,
        graph: &ExecutionGraph,
    ) -> Result<()> {
        let graph_json = serde_json::to_value(graph)
            .map_err(|e| AgentError::Tool(format!("Failed to serialize graph: {}", e)))?;
        ctx.set_state(format!("app:graph:{}", graph_id), graph_json);
        Ok(())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_condition_contains() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            GraphNode {
                id: "a".to_string(),
                agent: "test".to_string(),
                task: "test".to_string(),
                status: NodeStatus::Completed,
                result: Some("The analysis shows positive results".to_string()),
                ..Default::default()
            },
        );

        let cond = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::Contains,
            value: "positive".to_string(),
        };
        assert!(cond.evaluate(&nodes).unwrap());

        let cond_neg = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::NotContains,
            value: "negative".to_string(),
        };
        assert!(cond_neg.evaluate(&nodes).unwrap());
    }

    #[test]
    fn test_condition_equals() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            GraphNode {
                id: "a".to_string(),
                status: NodeStatus::Completed,
                result: Some("42".to_string()),
                ..Default::default()
            },
        );

        let cond = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::Equals,
            value: "42".to_string(),
        };
        assert!(cond.evaluate(&nodes).unwrap());
    }

    #[test]
    fn test_condition_gt_lt() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            GraphNode {
                id: "a".to_string(),
                status: NodeStatus::Completed,
                result: Some("75.5".to_string()),
                ..Default::default()
            },
        );

        let cond_gt = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::Gt,
            value: "50".to_string(),
        };
        assert!(cond_gt.evaluate(&nodes).unwrap());

        let cond_lt = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::Lt,
            value: "100".to_string(),
        };
        assert!(cond_lt.evaluate(&nodes).unwrap());
    }

    #[test]
    fn test_condition_regex() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            GraphNode {
                id: "a".to_string(),
                status: NodeStatus::Completed,
                result: Some("Error: connection refused (code 503)".to_string()),
                ..Default::default()
            },
        );

        let cond = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::Regex,
            value: r"code \d{3}".to_string(),
        };
        assert!(cond.evaluate(&nodes).unwrap());
    }

    #[test]
    fn test_condition_llm_eval() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            GraphNode {
                id: "a".to_string(),
                status: NodeStatus::Completed,
                result: Some("complex analysis text".to_string()),
                ..Default::default()
            },
        );

        let cond = Condition {
            ref_node: "a".to_string(),
            field: "result".to_string(),
            operator: ConditionOp::LlmEval,
            value: "Does this analysis recommend proceeding?".to_string(),
        };
        let result = cond.evaluate(&nodes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("LLM_EVAL_REQUIRED"));
    }

    #[test]
    fn test_find_ready_nodes_no_deps() {
        let nodes = vec![
            GraphNode {
                id: "a".to_string(),
                agent: "research".to_string(),
                task: "research X".to_string(),
                ..Default::default()
            },
            GraphNode {
                id: "b".to_string(),
                agent: "research".to_string(),
                task: "research Y".to_string(),
                ..Default::default()
            },
        ];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        let ready = graph.find_ready_nodes();
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn test_find_ready_nodes_with_deps() {
        let nodes = vec![
            GraphNode {
                id: "a".to_string(),
                agent: "research".to_string(),
                task: "research X".to_string(),
                ..Default::default()
            },
            GraphNode {
                id: "b".to_string(),
                agent: "research".to_string(),
                task: "research Y".to_string(),
                ..Default::default()
            },
            GraphNode {
                id: "c".to_string(),
                agent: "writer".to_string(),
                task: "write report".to_string(),
                depends_on: vec!["a".to_string(), "b".to_string()],
                ..Default::default()
            },
        ];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        // Initially only a and b are ready
        let ready = graph.find_ready_nodes();
        assert_eq!(ready.len(), 2);
        assert!(ready.iter().any(|n| n.id == "a"));
        assert!(ready.iter().any(|n| n.id == "b"));

        // Complete A and B
        graph.complete_node("a", "result A".to_string());
        graph.complete_node("b", "result B".to_string());

        // Now C is ready
        let ready = graph.find_ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "c");
    }

    #[test]
    fn test_conditional_skip() {
        let nodes = vec![
            GraphNode {
                id: "a".to_string(),
                agent: "analyst".to_string(),
                task: "analyze".to_string(),
                status: NodeStatus::Completed,
                result: Some("negative outlook".to_string()),
                ..Default::default()
            },
            GraphNode {
                id: "b".to_string(),
                agent: "writer".to_string(),
                task: "write positive report".to_string(),
                depends_on: vec!["a".to_string()],
                when: Some(Condition {
                    ref_node: "a".to_string(),
                    field: "result".to_string(),
                    operator: ConditionOp::Contains,
                    value: "positive".to_string(),
                }),
                ..Default::default()
            },
        ];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        let ready = graph.find_ready_nodes();
        assert!(ready.is_empty());
        assert_eq!(graph.nodes.get("b").unwrap().status, NodeStatus::Skipped);
    }

    #[test]
    fn test_input_resolution() {
        let nodes = vec![
            GraphNode {
                id: "a".to_string(),
                agent: "research".to_string(),
                task: "research".to_string(),
                status: NodeStatus::Completed,
                result: Some("important findings".to_string()),
                ..Default::default()
            },
            GraphNode {
                id: "b".to_string(),
                agent: "writer".to_string(),
                task: "Write a report based on: {research_data}".to_string(),
                depends_on: vec!["a".to_string()],
                inputs: HashMap::from([(
                    "research_data".to_string(),
                    InputRef {
                        from: "a".to_string(),
                        field: "result".to_string(),
                    },
                )]),
                ..Default::default()
            },
        ];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);
        let ready = graph.find_ready_nodes();

        assert_eq!(ready.len(), 1);
        assert!(ready[0].task.contains("important findings"));
    }

    #[test]
    fn test_retry_on_failure() {
        let nodes = vec![GraphNode {
            id: "a".to_string(),
            agent: "worker".to_string(),
            task: "do work".to_string(),
            retry: Some(RetryPolicy {
                max: 3,
                on_fail: None,
            }),
            ..Default::default()
        }];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        // First find ready
        let ready = graph.find_ready_nodes();
        assert_eq!(ready.len(), 1);

        // Fail first attempt — should retry (reset to Pending)
        graph.fail_node("a", "timeout".to_string());
        assert_eq!(graph.nodes.get("a").unwrap().status, NodeStatus::Pending);
        assert_eq!(graph.nodes.get("a").unwrap().attempts, 1);

        // Fail second attempt — should retry
        graph.fail_node("a", "timeout again".to_string());
        assert_eq!(graph.nodes.get("a").unwrap().status, NodeStatus::Pending);
        assert_eq!(graph.nodes.get("a").unwrap().attempts, 2);

        // Fail third attempt — max reached, should be Failed
        graph.fail_node("a", "final failure".to_string());
        assert_eq!(graph.nodes.get("a").unwrap().status, NodeStatus::Failed);
        assert_eq!(graph.nodes.get("a").unwrap().attempts, 3);
    }

    #[test]
    fn test_graph_completion_status() {
        let nodes = vec![
            GraphNode {
                id: "a".to_string(),
                agent: "worker".to_string(),
                task: "work".to_string(),
                ..Default::default()
            },
            GraphNode {
                id: "b".to_string(),
                agent: "worker".to_string(),
                task: "work".to_string(),
                depends_on: vec!["a".to_string()],
                ..Default::default()
            },
        ];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        // Start
        let _ = graph.find_ready_nodes();
        assert_eq!(graph.status, GraphStatus::Running);

        // Complete a
        graph.complete_node("a", "done".to_string());
        let _ = graph.find_ready_nodes();

        // Complete b
        graph.complete_node("b", "done".to_string());
        let _ = graph.find_ready_nodes();
        assert_eq!(graph.status, GraphStatus::Completed);
    }

    #[test]
    fn test_graph_failed_status() {
        let nodes = vec![GraphNode {
            id: "a".to_string(),
            agent: "worker".to_string(),
            task: "work".to_string(),
            ..Default::default()
        }];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        let _ = graph.find_ready_nodes();
        graph.fail_node("a", "error".to_string());
        let _ = graph.find_ready_nodes();
        assert_eq!(graph.status, GraphStatus::Failed);
    }

    #[test]
    fn test_add_node() {
        let nodes = vec![GraphNode {
            id: "a".to_string(),
            agent: "worker".to_string(),
            task: "work".to_string(),
            ..Default::default()
        }];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        assert!(graph.add_node(GraphNode {
            id: "b".to_string(),
            agent: "writer".to_string(),
            task: "write".to_string(),
            depends_on: vec!["a".to_string()],
            ..Default::default()
        }));
        assert_eq!(graph.nodes.len(), 2);

        // Duplicate should fail
        assert!(!graph.add_node(GraphNode {
            id: "b".to_string(),
            ..Default::default()
        }));
    }

    #[test]
    fn test_depend_mode_any_completed() {
        let nodes = vec![
            GraphNode {
                id: "a".to_string(),
                agent: "worker".to_string(),
                task: "work".to_string(),
                status: NodeStatus::Completed,
                result: Some("done".to_string()),
                ..Default::default()
            },
            GraphNode {
                id: "b".to_string(),
                agent: "worker".to_string(),
                task: "work".to_string(),
                status: NodeStatus::Pending,
                ..Default::default()
            },
            GraphNode {
                id: "c".to_string(),
                agent: "merger".to_string(),
                task: "merge".to_string(),
                depends_on: vec!["a".to_string(), "b".to_string()],
                depend_mode: DependMode::AnyCompleted,
                ..Default::default()
            },
        ];
        let mut graph = ExecutionGraph::new("test".to_string(), nodes);

        let ready = graph.find_ready_nodes();
        // b is ready (no deps), and c is ready (any_completed, a is done)
        let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
        assert!(ready_ids.contains(&"b"));
        assert!(ready_ids.contains(&"c"));
    }

    #[test]
    fn test_execution_graph_tool_schema() {
        let tool = ExecutionGraphTool::new();
        assert_eq!(tool.name(), "execution_graph");
        let schema = tool.parameters_schema().unwrap();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("nodes").is_some());
        assert!(props.get("completed").is_some());
        assert!(props.get("graph_id").is_some());
    }
}
