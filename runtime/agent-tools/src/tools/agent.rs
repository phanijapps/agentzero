// ============================================================================
// AGENT TOOLS
// Tools for managing and discovering AI agents
// ============================================================================

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt;

use agent_primitives::FileSystemContext;
use agent_primitives::{Result, Tool, ToolContext};

fn validate_agent_id(name: &str) -> Result<()> {
    const RESERVED_AGENT_IDS: &[&str] = &["root", "orchestrator"];
    let valid = !name.is_empty()
        && name.len() <= 64
        && !RESERVED_AGENT_IDS.contains(&name)
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');

    if valid {
        Ok(())
    } else {
        Err(agent_primitives::AgentError::Tool(
            "Invalid agent name. Use a non-reserved lowercase kebab-case agent ID like 'research-agent'."
                .to_string(),
        ))
    }
}

fn resolve_agent_dir(agents_dir: &Path, name: &str) -> Result<PathBuf> {
    validate_agent_id(name)?;
    let agent_dir = agents_dir.join(name);
    if agent_dir.starts_with(agents_dir) {
        Ok(agent_dir)
    } else {
        Err(agent_primitives::AgentError::Tool(
            "Resolved agent directory escaped the agents directory".to_string(),
        ))
    }
}

// ============================================================================
// LIST AGENTS TOOL
// ============================================================================

/// Tool for discovering available agents to delegate to.
///
/// This tool reads from a cached agent list stored in the ToolContext state.
/// The list is populated by the execution runner when creating the executor.
pub struct ListAgentsTool;

impl ListAgentsTool {
    /// Create a new list agents tool
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListAgentsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn description(&self) -> &str {
        "List available agents you can delegate tasks to using delegate_to_agent. \
         Returns agent IDs, names, and descriptions to help you choose the right agent for a task."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, _args: Value) -> Result<Value> {
        // Read cached agent list from context state
        let agents: Value = match ctx.get_state("available_agents") {
            Some(v) => v.clone(),
            None => json!([]),
        };

        // Get current agent ID to exclude from list
        let current_agent_id: String = match ctx.get_state("agent_id") {
            Some(v) => v.as_str().unwrap_or("").to_string(),
            None => String::new(),
        };

        // Filter out current agent
        let mut agent_list: Vec<Value> = Vec::new();
        if let Some(arr) = agents.as_array() {
            for agent in arr {
                let agent_id = match agent.get("id") {
                    Some(v) => v.as_str().unwrap_or(""),
                    None => "",
                };
                if agent_id != current_agent_id {
                    agent_list.push(agent.clone());
                }
            }
        }

        if agent_list.is_empty() {
            return Ok(json!({
                "agents": [],
                "message": "No other agents available for delegation."
            }));
        }

        Ok(json!({
            "agents": agent_list,
            "count": agent_list.len(),
            "message": format!("Found {} agent(s) available for delegation. Use delegate_to_agent with the agent's id.", agent_list.len())
        }))
    }
}

// ============================================================================
// CREATE AGENT TOOL
// ============================================================================

/// Tool for creating new AI agents
pub struct CreateAgentTool {
    /// File system context
    fs: Arc<dyn FileSystemContext>,
}

impl CreateAgentTool {
    /// Create a new create agent tool with file system context
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for CreateAgentTool {
    fn name(&self) -> &str {
        "create_agent"
    }

    fn description(&self) -> &str {
        "Create a new AI agent with the specified configuration. The agent will be saved to the agents directory."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Agent identifier (kebab-case, e.g., 'my-agent')"
                },
                "displayName": {
                    "type": "string",
                    "description": "Human-readable display name (e.g., 'My Agent')"
                },
                "description": {
                    "type": "string",
                    "description": "Brief description of what this agent does"
                },
                "providerId": {
                    "type": "string",
                    "description": "Provider ID (must exist in providers.json)"
                },
                "model": {
                    "type": "string",
                    "description": "Model name (e.g., 'gpt-4o', 'claude-3-5-sonnet-20241022')"
                },
                "temperature": {
                    "type": "number",
                    "description": "Temperature (0.0-2.0, default 0.7)",
                    "default": 0.7
                },
                "maxTokens": {
                    "type": "integer",
                    "description": "Legacy maximum output tokens for response (default 32000)",
                    "default": 32000
                },
                "maxInputTokens": {
                    "type": "integer",
                    "description": "Optional explicit maximum input/context tokens. Omit to inherit provider/model limits."
                },
                "maxOutputTokens": {
                    "type": "integer",
                    "description": "Maximum output tokens for response (default 32000)",
                    "default": 32000
                },
                "thinkingEnabled": {
                    "type": "boolean",
                    "description": "Enable extended thinking (for supported models)",
                    "default": false
                },
                "instructions": {
                    "type": "string",
                    "description": "System instructions for the agent"
                },
                "skills": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of skill IDs to include",
                    "default": []
                },
                "mcps": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of MCP server IDs to include",
                    "default": []
                }
            },
            "required": ["name", "displayName", "description", "providerId", "model", "instructions"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'name' parameter".to_string())
        })?;

        let display_name = args
            .get("displayName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'displayName' parameter".to_string())
            })?;

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'description' parameter".to_string())
            })?;

        let provider_id = args
            .get("providerId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'providerId' parameter".to_string())
            })?;

        let model = args.get("model").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'model' parameter".to_string())
        })?;

        let instructions = args
            .get("instructions")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_primitives::AgentError::Tool("Missing 'instructions' parameter".to_string())
            })?;

        let temperature = args
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7);

        let max_input_tokens = args.get("maxInputTokens").and_then(|v| v.as_u64());

        let max_tokens = args
            .get("maxOutputTokens")
            .or_else(|| args.get("maxTokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(32_000) as u32;

        let thinking_enabled = args
            .get("thinkingEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let skills: Vec<String> = args
            .get("skills")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let mcps: Vec<String> = args
            .get("mcps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // Get agents directory from file system context
        let agents_dir = self.fs.agents_dir().ok_or_else(|| {
            agent_primitives::AgentError::Tool("Agents directory not configured".to_string())
        })?;

        // Create agent directory without overwriting existing agents or symlinks.
        let agent_dir = resolve_agent_dir(&agents_dir, name)?;
        match tokio::fs::symlink_metadata(&agent_dir).await {
            Ok(_) => {
                return Err(agent_primitives::AgentError::Tool(format!(
                    "Agent '{}' already exists",
                    name
                )));
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(agent_primitives::AgentError::Tool(format!(
                    "Failed to inspect agent directory: {}",
                    e
                )));
            }
        }
        tokio::fs::create_dir_all(&agents_dir).await.map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to create agents directory: {}", e))
        })?;
        tokio::fs::create_dir(&agent_dir).await.map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to create agent directory: {}", e))
        })?;

        // Create config.yaml. Omit maxInputTokens when absent so provider/model
        // context limits can remain inherited.
        let mut config = Map::new();
        config.insert("name".to_string(), json!(name));
        config.insert("displayName".to_string(), json!(display_name));
        config.insert("description".to_string(), json!(description));
        config.insert("providerId".to_string(), json!(provider_id));
        config.insert("model".to_string(), json!(model));
        config.insert("temperature".to_string(), json!(temperature));
        if let Some(max_input_tokens) = max_input_tokens {
            config.insert("maxInputTokens".to_string(), json!(max_input_tokens));
        }
        config.insert("maxOutputTokens".to_string(), json!(max_tokens));
        config.insert("thinkingEnabled".to_string(), json!(thinking_enabled));
        config.insert("skills".to_string(), json!(skills));
        config.insert("mcps".to_string(), json!(mcps));

        let config_yaml = serde_yaml::to_string(&Value::Object(config)).map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to serialize config: {}", e))
        })?;

        write_new_file(&agent_dir.join("config.yaml"), config_yaml.as_bytes()).await?;

        // Create AGENTS.md
        let agents_md = format!("{}\n", instructions);
        write_new_file(&agent_dir.join("AGENTS.md"), agents_md.as_bytes()).await?;

        tracing::info!("Created agent '{}' at {:?}", name, agent_dir);

        let mut response = Map::new();
        response.insert("name".to_string(), json!(name));
        response.insert("displayName".to_string(), json!(display_name));
        response.insert("description".to_string(), json!(description));
        response.insert("providerId".to_string(), json!(provider_id));
        response.insert("model".to_string(), json!(model));
        response.insert("temperature".to_string(), json!(temperature));
        response.insert(
            "maxInputTokensExplicit".to_string(),
            json!(max_input_tokens.is_some()),
        );
        if let Some(max_input_tokens) = max_input_tokens {
            response.insert("maxInputTokens".to_string(), json!(max_input_tokens));
        }
        response.insert("maxOutputTokens".to_string(), json!(max_tokens));
        response.insert("maxTokens".to_string(), json!(max_tokens));
        response.insert("thinkingEnabled".to_string(), json!(thinking_enabled));
        response.insert("skills".to_string(), json!(skills));
        response.insert("mcps".to_string(), json!(mcps));
        response.insert(
            "location".to_string(),
            json!(agent_dir.to_string_lossy().to_string()),
        );
        response.insert(
            "message".to_string(),
            json!(format!("Agent '{}' created successfully!", name)),
        );

        Ok(Value::Object(response))
    }
}

async fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|e| agent_primitives::AgentError::Tool(format!("Failed to create file: {}", e)))?;
    file.write_all(bytes)
        .await
        .map_err(|e| agent_primitives::AgentError::Tool(format!("Failed to write file: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    use agent_primitives::types::Content;
    use agent_primitives::{CallbackContext, EventActions, ReadonlyContext};

    struct TestFs {
        agents_dir: PathBuf,
    }

    impl FileSystemContext for TestFs {
        fn conversation_dir(&self, _conversation_id: &str) -> Option<PathBuf> {
            None
        }

        fn outputs_dir(&self) -> Option<PathBuf> {
            None
        }

        fn skills_dir(&self) -> Option<PathBuf> {
            None
        }

        fn agents_dir(&self) -> Option<PathBuf> {
            Some(self.agents_dir.clone())
        }

        fn python_executable(&self) -> Option<PathBuf> {
            None
        }
    }

    struct TestCtx;

    impl ReadonlyContext for TestCtx {
        fn invocation_id(&self) -> &str {
            "test-invocation"
        }

        fn agent_name(&self) -> &str {
            "test-agent"
        }

        fn user_id(&self) -> &str {
            "test-user"
        }

        fn app_name(&self) -> &str {
            "test-app"
        }

        fn session_id(&self) -> &str {
            "test-session"
        }

        fn branch(&self) -> &str {
            "test"
        }

        fn user_content(&self) -> &Content {
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for TestCtx {
        fn get_state(&self, _key: &str) -> Option<Value> {
            None
        }

        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for TestCtx {
        fn function_call_id(&self) -> String {
            "test-call".to_string()
        }

        fn actions(&self) -> EventActions {
            EventActions::default()
        }

        fn set_actions(&self, _actions: EventActions) {}
    }

    fn create_args(name: &str) -> Value {
        json!({
            "name": name,
            "displayName": "Test Agent",
            "description": "Test description",
            "providerId": "provider",
            "model": "model",
            "instructions": "Test instructions",
        })
    }

    async fn run_create_agent(args: Value) -> (Value, Value) {
        let temp = tempfile::tempdir().expect("tempdir");
        let agents_dir = temp.path().join("agents");
        let tool = CreateAgentTool::new(Arc::new(TestFs {
            agents_dir: agents_dir.clone(),
        }));
        let name = args["name"].as_str().expect("name").to_string();

        let result = tool
            .execute(Arc::new(TestCtx), args)
            .await
            .expect("create agent");
        let config_yaml = tokio::fs::read_to_string(agents_dir.join(name).join("config.yaml"))
            .await
            .expect("config yaml");
        let config: Value = serde_yaml::from_str(&config_yaml).expect("parse config");

        (result, config)
    }

    #[tokio::test]
    async fn create_agent_omits_absent_max_input_tokens() {
        let (result, config) = run_create_agent(create_args("inherited-agent")).await;

        assert!(config.get("maxInputTokens").is_none());
        assert_eq!(result.get("maxInputTokens"), None);
        assert_eq!(result["maxInputTokensExplicit"], false);
    }

    #[tokio::test]
    async fn create_agent_preserves_explicit_default_max_input_tokens() {
        let mut args = create_args("explicit-agent");
        args["maxInputTokens"] = json!(200_000);

        let (result, config) = run_create_agent(args).await;

        assert_eq!(config["maxInputTokens"], 200_000);
        assert_eq!(result["maxInputTokens"], 200_000);
        assert_eq!(result["maxInputTokensExplicit"], true);
    }

    #[tokio::test]
    async fn create_agent_rejects_path_traversal_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let agents_dir = temp.path().join("agents");
        let tool = CreateAgentTool::new(Arc::new(TestFs {
            agents_dir: agents_dir.clone(),
        }));
        let mut args = create_args("../escape");
        args["name"] = json!("../escape");

        let err = tool
            .execute(Arc::new(TestCtx), args)
            .await
            .expect_err("path traversal should be rejected");

        assert!(err.to_string().contains("Invalid agent name"));
        assert!(!temp.path().join("escape").exists());
    }

    #[tokio::test]
    async fn create_agent_rejects_reserved_root_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let agents_dir = temp.path().join("agents");
        let tool = CreateAgentTool::new(Arc::new(TestFs {
            agents_dir: agents_dir.clone(),
        }));

        let err = tool
            .execute(Arc::new(TestCtx), create_args("root"))
            .await
            .expect_err("root should be rejected");

        assert!(err.to_string().contains("Invalid agent name"));
        assert!(!agents_dir.join("root").exists());
    }

    #[tokio::test]
    async fn create_agent_rejects_existing_agent_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let agents_dir = temp.path().join("agents");
        let existing_dir = agents_dir.join("reviewer-agent");
        tokio::fs::create_dir_all(&existing_dir)
            .await
            .expect("existing dir");
        tokio::fs::write(existing_dir.join("config.yaml"), "existing: true\n")
            .await
            .expect("existing config");
        tokio::fs::write(existing_dir.join("AGENTS.md"), "existing instructions\n")
            .await
            .expect("existing instructions");
        let tool = CreateAgentTool::new(Arc::new(TestFs {
            agents_dir: agents_dir.clone(),
        }));

        let err = tool
            .execute(Arc::new(TestCtx), create_args("reviewer-agent"))
            .await
            .expect_err("existing agent should be rejected");

        assert!(err.to_string().contains("already exists"));
        let config = tokio::fs::read_to_string(existing_dir.join("config.yaml"))
            .await
            .expect("config");
        let instructions = tokio::fs::read_to_string(existing_dir.join("AGENTS.md"))
            .await
            .expect("instructions");
        assert_eq!(config, "existing: true\n");
        assert_eq!(instructions, "existing instructions\n");
    }

    #[test]
    fn create_agent_schema_does_not_default_max_input_tokens() {
        let tool = CreateAgentTool::new(Arc::new(TestFs {
            agents_dir: PathBuf::from("/tmp/agents"),
        }));
        let schema = tool.parameters_schema().expect("schema");
        let max_input = &schema["properties"]["maxInputTokens"];

        assert!(max_input.get("default").is_none());
        assert!(
            max_input["description"]
                .as_str()
                .expect("description")
                .contains("Omit to inherit provider/model limits")
        );
    }
}
