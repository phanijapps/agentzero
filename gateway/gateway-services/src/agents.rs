//! # Agent Service
//!
//! Service for managing agent configurations.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::{DEFAULT_MAX_INPUT_TOKENS, DEFAULT_MAX_OUTPUT_TOKENS};

/// Agent configuration stored in config.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
    #[serde(rename = "agentType", default)]
    pub agent_type: Option<String>,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub model: String,
    pub temperature: f64,
    #[serde(
        rename = "maxInputTokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_input_tokens: Option<u64>,
    #[serde(
        rename = "maxOutputTokens",
        alias = "maxTokens",
        default = "default_max_output_tokens"
    )]
    pub max_tokens: u32,
    #[serde(rename = "thinkingEnabled", default)]
    pub thinking_enabled: bool,
    #[serde(
        rename = "voiceRecordingEnabled",
        default = "default_voice_recording_enabled"
    )]
    pub voice_recording_enabled: bool,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
    #[serde(
        rename = "systemInstruction",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub system_instruction: Option<String>,
}

fn default_max_output_tokens() -> u32 {
    DEFAULT_MAX_OUTPUT_TOKENS
}

fn default_voice_recording_enabled() -> bool {
    true
}

fn validate_agent_id(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !is_reserved_agent_id(name)
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');

    if valid {
        Ok(())
    } else {
        Err(
            "Invalid agent name. Use a non-reserved lowercase kebab-case agent ID like 'research-agent'."
                .to_string(),
        )
    }
}

fn is_reserved_agent_id(name: &str) -> bool {
    matches!(name, "root" | "orchestrator")
}

fn resolve_agent_dir(agents_dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_agent_id(name)?;
    let agent_dir = agents_dir.join(name);
    if agent_dir.starts_with(agents_dir) {
        Ok(agent_dir)
    } else {
        Err("Resolved agent directory escaped the agents directory".to_string())
    }
}

fn write_new_file(path: &Path, contents: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write file: {}", e))
}

/// Full agent data including instructions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
    #[serde(rename = "agentType", default)]
    pub agent_type: Option<String>,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub model: String,
    pub temperature: f64,
    #[serde(rename = "maxInputTokens")]
    pub max_input_tokens: u64,
    #[serde(skip)]
    pub max_input_tokens_explicit: bool,
    #[serde(rename = "maxOutputTokens", alias = "maxTokens")]
    pub max_tokens: u32,
    #[serde(rename = "thinkingEnabled")]
    pub thinking_enabled: bool,
    #[serde(rename = "voiceRecordingEnabled")]
    pub voice_recording_enabled: bool,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<String>,
    pub instructions: String,
    pub mcps: Vec<String>,
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middleware: Option<String>,
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// Agent service for managing agent configurations.
pub struct AgentService {
    agents_dir: PathBuf,
    cache: RwLock<Option<Vec<Agent>>>,
}

impl AgentService {
    /// Create a new agent service.
    pub fn new(agents_dir: PathBuf) -> Self {
        Self {
            agents_dir,
            cache: RwLock::new(None),
        }
    }

    /// Get the agents directory.
    pub fn agents_dir(&self) -> &PathBuf {
        &self.agents_dir
    }

    /// List all agents.
    pub async fn list(&self) -> Result<Vec<Agent>, String> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(agents) = cache.as_ref() {
                return Ok(agents.clone());
            }
        }

        // Read from disk
        let agents = self.list_from_disk()?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            *cache = Some(agents.clone());
        }

        Ok(agents)
    }

    /// List agents from disk (bypasses cache).
    fn list_from_disk(&self) -> Result<Vec<Agent>, String> {
        if !self.agents_dir.exists() {
            return Ok(vec![]);
        }

        let mut agents = Vec::new();

        let entries = fs::read_dir(&self.agents_dir)
            .map_err(|e| format!("Failed to read agents directory: {}", e))?;

        for entry in entries.flatten() {
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_reserved_agent_id)
            {
                continue;
            }

            let config_yaml = path.join("config.yaml");
            if !config_yaml.exists() {
                continue;
            }

            if let Ok(agent) = self.read_agent_folder(&path) {
                agents.push(agent);
            }
        }

        Ok(agents)
    }

    /// Get a single agent by ID.
    pub async fn get(&self, id: &str) -> Result<Agent, String> {
        let agent_dir = resolve_agent_dir(&self.agents_dir, id)?;

        if !agent_dir.exists() {
            return Err(format!("Agent not found: {}", id));
        }

        self.read_agent_folder(&agent_dir)
    }

    /// Create a new agent.
    pub async fn create(&self, agent: Agent) -> Result<Agent, String> {
        let agent_dir = resolve_agent_dir(&self.agents_dir, &agent.name)?;
        match fs::symlink_metadata(&agent_dir) {
            Ok(_) => return Err(format!("Agent '{}' already exists", agent.name)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Failed to inspect agent directory: {}", e)),
        }

        // Ensure agents directory exists
        fs::create_dir_all(&self.agents_dir)
            .map_err(|e| format!("Failed to create agents directory: {}", e))?;

        // Create agent directory
        fs::create_dir(&agent_dir)
            .map_err(|e| format!("Failed to create agent directory: {}", e))?;

        // Write config.yaml
        let config = AgentConfig {
            name: agent.name.clone(),
            display_name: agent.display_name.clone(),
            description: agent.description.clone(),
            provider_id: agent.provider_id.clone(),
            model: agent.model.clone(),
            temperature: agent.temperature,
            max_input_tokens: agent
                .max_input_tokens_explicit
                .then_some(agent.max_input_tokens),
            max_tokens: agent.max_tokens,
            thinking_enabled: agent.thinking_enabled,
            voice_recording_enabled: agent.voice_recording_enabled,
            skills: agent.skills.clone(),
            mcps: agent.mcps.clone(),
            agent_type: agent.agent_type.clone(),
            system_instruction: agent.system_instruction.clone(),
        };

        let config_yaml = serde_yaml::to_string(&config)
            .map_err(|e| format!("Failed to serialize config.yaml: {}", e))?;

        // Append middleware YAML if provided
        let final_yaml = if let Some(middleware_yaml) = &agent.middleware {
            format!("{}\n{}", config_yaml.trim_end(), middleware_yaml.trim_end())
        } else {
            config_yaml
        };

        write_new_file(&agent_dir.join("config.yaml"), &final_yaml)?;

        // Write AGENTS.md
        write_new_file(
            &agent_dir.join("AGENTS.md"),
            &format!("{}\n", agent.instructions),
        )?;

        // Invalidate cache
        self.invalidate_cache().await;

        Ok(Agent {
            id: agent.name.clone(),
            created_at: Some(chrono::Utc::now().to_rfc3339()),
            ..agent
        })
    }

    /// Update an existing agent.
    pub async fn update(&self, id: &str, agent: Agent) -> Result<Agent, String> {
        let agent_dir = resolve_agent_dir(&self.agents_dir, id)?;
        let target_dir = resolve_agent_dir(&self.agents_dir, &agent.name)?;

        if !agent_dir.exists() {
            return Err(format!("Agent not found: {}", id));
        }

        // If name changed, rename directory
        if agent.name != id {
            if fs::symlink_metadata(&target_dir).is_ok() {
                return Err(format!("Agent '{}' already exists", agent.name));
            }
            fs::rename(&agent_dir, &target_dir)
                .map_err(|e| format!("Failed to rename agent directory: {}", e))?;
        }

        // Write config.yaml
        let config = AgentConfig {
            name: agent.name.clone(),
            display_name: agent.display_name.clone(),
            description: agent.description.clone(),
            provider_id: agent.provider_id.clone(),
            model: agent.model.clone(),
            temperature: agent.temperature,
            max_input_tokens: agent
                .max_input_tokens_explicit
                .then_some(agent.max_input_tokens),
            max_tokens: agent.max_tokens,
            thinking_enabled: agent.thinking_enabled,
            voice_recording_enabled: agent.voice_recording_enabled,
            skills: agent.skills.clone(),
            mcps: agent.mcps.clone(),
            agent_type: agent.agent_type.clone(),
            system_instruction: agent.system_instruction.clone(),
        };

        let config_yaml = serde_yaml::to_string(&config)
            .map_err(|e| format!("Failed to serialize config.yaml: {}", e))?;

        let final_yaml = if let Some(middleware_yaml) = &agent.middleware {
            format!("{}\n{}", config_yaml.trim_end(), middleware_yaml.trim_end())
        } else {
            config_yaml
        };

        fs::write(target_dir.join("config.yaml"), final_yaml)
            .map_err(|e| format!("Failed to write config.yaml: {}", e))?;

        // Write AGENTS.md
        fs::write(
            target_dir.join("AGENTS.md"),
            format!("{}\n", agent.instructions),
        )
        .map_err(|e| format!("Failed to write AGENTS.md: {}", e))?;

        // Invalidate cache
        self.invalidate_cache().await;

        Ok(agent)
    }

    /// Delete an agent.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let agent_path = resolve_agent_dir(&self.agents_dir, id)?;

        if !agent_path.exists() {
            return Err(format!("Agent not found: {}", id));
        }

        fs::remove_dir_all(&agent_path)
            .map_err(|e| format!("Failed to delete agent directory: {}", e))?;

        // Invalidate cache
        self.invalidate_cache().await;

        Ok(())
    }

    /// Invalidate the agent cache.
    pub async fn invalidate_cache(&self) {
        let mut cache = self.cache.write().await;
        *cache = None;
    }

    /// Read an agent folder and parse config.yaml and AGENTS.md
    fn read_agent_folder(&self, agent_dir: &PathBuf) -> Result<Agent, String> {
        let config_path = agent_dir.join("config.yaml");
        let agents_md_path = agent_dir.join("AGENTS.md");

        if !config_path.exists() {
            return Err(format!("config.yaml not found in {:?}", agent_dir));
        }

        if !agents_md_path.exists() {
            return Err(format!("AGENTS.md not found in {:?}", agent_dir));
        }

        // Read config.yaml
        let config_content = fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.yaml: {}", e))?;
        let config: AgentConfig = serde_yaml::from_str(&config_content)
            .map_err(|e| format!("Failed to parse config.yaml: {}", e))?;

        // Read AGENTS.md
        let instructions = fs::read_to_string(&agents_md_path)
            .map_err(|e| format!("Failed to read AGENTS.md: {}", e))?;

        // Get agent name from directory name
        let name = agent_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Agent {
            id: name.clone(),
            name,
            display_name: config.display_name,
            description: config.description,
            agent_type: config.agent_type,
            provider_id: config.provider_id,
            model: config.model,
            temperature: config.temperature,
            max_input_tokens_explicit: config.max_input_tokens.is_some(),
            max_input_tokens: config.max_input_tokens.unwrap_or(DEFAULT_MAX_INPUT_TOKENS),
            max_tokens: config.max_tokens,
            thinking_enabled: config.thinking_enabled,
            voice_recording_enabled: config.voice_recording_enabled,
            system_instruction: config.system_instruction,
            instructions,
            mcps: config.mcps,
            skills: config.skills,
            middleware: None,
            created_at: Some("1970-01-01T00:00:00Z".to_string()),
        })
    }

    /// Seed default subagents if they don't exist.
    ///
    /// Accepts agent definitions as JSON (loaded from `default_agents.json` by caller).
    /// Falls back to hardcoded defaults if `template_json` is None.
    /// Seed default subagents if they don't exist.
    ///
    /// - `template_json`: Agent definitions from `default_agents.json`
    /// - `instructions_loader`: Callback to load AGENTS.md from embedded templates by agent name
    pub async fn seed_default_agents(
        &self,
        default_provider_id: &str,
        default_model: &str,
        template_json: Option<&[u8]>,
        instructions_loader: impl Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        let template_agents: Vec<serde_json::Value> = template_json
            .and_then(|data| serde_json::from_slice(data).ok())
            .unwrap_or_else(|| {
                serde_json::json!([
                    {"name": "research-agent", "displayName": "Research Agent", "description": "Specialized in gathering, analyzing, and synthesizing information from various sources.", "temperature": 0.7, "maxTokens": 8192, "skills": [], "mcps": []},
                    {"name": "code-agent", "displayName": "Code Agent", "description": "Specialized in code generation, review, debugging, and software engineering tasks.", "temperature": 0.7, "maxTokens": 8192, "skills": [], "mcps": []},
                    {"name": "writing-agent", "displayName": "Writing Agent", "description": "Specialized in content creation, editing, and written communication.", "temperature": 0.7, "maxTokens": 8192, "skills": [], "mcps": []}
                ]).as_array().cloned().unwrap_or_default()
            });

        for entry in &template_agents {
            let name = entry["name"].as_str().unwrap_or_default();
            if name.is_empty() {
                continue;
            }

            if self.get(name).await.is_ok() {
                if let Some(instructions) = instructions_loader(name) {
                    let hashes = known_template_managed_hashes(name);
                    if !hashes.is_empty() {
                        match self
                            .refresh_default_agent_instructions_if_managed(
                                name,
                                &instructions,
                                hashes,
                            )
                            .await
                        {
                            Ok(true) => tracing::info!(
                                agent = %name,
                                "Refreshed template-managed default agent instructions"
                            ),
                            Ok(false) => {}
                            Err(e) => tracing::warn!(
                                agent = %name,
                                error = %e,
                                "Failed to refresh default agent instructions"
                            ),
                        }
                    }
                }
                tracing::debug!("Agent {} already exists, skipping seed", name);
                continue;
            }

            tracing::info!("Seeding default agent: {}", name);

            let display_name = entry["displayName"].as_str().unwrap_or(name);
            let description = entry["description"].as_str().unwrap_or("");
            let agent_type = entry["agentType"].as_str().unwrap_or("specialist");
            let temperature = entry["temperature"].as_f64().unwrap_or(0.7);
            let max_input_tokens = entry.get("maxInputTokens").and_then(|v| v.as_u64());
            let max_input_tokens_explicit = max_input_tokens.is_some();
            let max_input_tokens = max_input_tokens.unwrap_or(DEFAULT_MAX_INPUT_TOKENS);
            let max_tokens = entry["maxOutputTokens"]
                .as_u64()
                .or_else(|| entry["maxTokens"].as_u64())
                .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS as u64) as u32;
            let skills: Vec<String> = entry["skills"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mcps: Vec<String> = entry["mcps"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            // Load instructions: try bundled template first, then hardcoded fallback
            let instructions = instructions_loader(name)
                .or_else(|| match name {
                    "research-agent" => Some(RESEARCH_AGENT_INSTRUCTIONS.to_string()),
                    "code-agent" => Some(CODE_AGENT_INSTRUCTIONS.to_string()),
                    "writing-agent" => Some(WRITING_AGENT_INSTRUCTIONS.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "You are a helpful AI assistant.".to_string());

            let agent = Agent {
                id: name.to_string(),
                name: name.to_string(),
                display_name: display_name.to_string(),
                description: description.to_string(),
                agent_type: Some(agent_type.to_string()),
                provider_id: default_provider_id.to_string(),
                model: default_model.to_string(),
                temperature,
                max_input_tokens,
                max_input_tokens_explicit,
                max_tokens,
                thinking_enabled: false,
                voice_recording_enabled: false,
                system_instruction: None,
                instructions,
                mcps,
                skills,
                middleware: None,
                created_at: None,
            };

            if let Err(e) = self.create(agent).await {
                tracing::warn!("Failed to seed agent {}: {}", name, e);
            }
        }

        Ok(())
    }

    async fn refresh_default_agent_instructions_if_managed(
        &self,
        name: &str,
        new_instructions: &str,
        managed_hashes: &[&str],
    ) -> Result<bool, String> {
        let agent_dir = self.agents_dir.join(name);
        let agents_md_path = agent_dir.join("AGENTS.md");
        if !agents_md_path.exists() {
            return Ok(false);
        }

        let current = fs::read_to_string(&agents_md_path)
            .map_err(|e| format!("Failed to read AGENTS.md: {}", e))?;
        let current_normalized = normalize_agent_instructions(&current);
        let new_normalized = normalize_agent_instructions(new_instructions);
        if current_normalized == new_normalized {
            return Ok(false);
        }

        let current_hash = agent_runtime::content_hash(&current_normalized);
        if !managed_hashes.iter().any(|hash| *hash == current_hash) {
            return Ok(false);
        }

        let backup_path = agent_dir.join(format!(
            "AGENTS.md.bak-{}",
            chrono::Utc::now().timestamp_millis()
        ));
        fs::write(&backup_path, current)
            .map_err(|e| format!("Failed to write AGENTS.md backup: {}", e))?;
        fs::write(&agents_md_path, new_normalized)
            .map_err(|e| format!("Failed to refresh AGENTS.md: {}", e))?;
        self.invalidate_cache().await;
        Ok(true)
    }
}

fn normalize_agent_instructions(instructions: &str) -> String {
    let trimmed = instructions
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn known_template_managed_hashes(name: &str) -> &'static [&'static str] {
    match name {
        // Normalized hashes of the bundled prompts before
        // builder-delegation-hygiene mode updates.
        "builder-agent" => &["3f7bcb5343be4add6b90775d0d02be28ace6f1306e425b213ba9c53f0b21ec0a"],
        "planner-agent" => &["e8575d90ee8342db9b969dd33fbb3d9cf431e6e3896a936b764c9fed2f1deef7"],
        _ => &[],
    }
}

// ============================================================================
// DEFAULT AGENT INSTRUCTIONS
// ============================================================================

const RESEARCH_AGENT_INSTRUCTIONS: &str = r#"# Research Agent

You are a specialized research agent focused on gathering, analyzing, and synthesizing information.

## Capabilities

- Deep research on any topic
- Information synthesis and summarization
- Fact-checking and verification
- Source evaluation and citation

## Approach

1. **Understand the Query**: Clarify what information is needed
2. **Gather Information**: Use available tools to search and collect data
3. **Analyze**: Evaluate sources, identify patterns, cross-reference facts
4. **Synthesize**: Combine findings into a coherent summary
5. **Report**: Provide clear, well-structured findings

## Output Format

Structure your findings as:
- **Summary**: Key findings in 2-3 sentences
- **Details**: Organized by subtopic
- **Sources**: List of references used
- **Confidence**: How confident you are in the findings

## Guidelines

- Prioritize accuracy over speed
- Cite sources when possible
- Acknowledge uncertainty
- Provide balanced perspectives on controversial topics
"#;

const CODE_AGENT_INSTRUCTIONS: &str = r#"# Code Agent

You are a specialized coding agent focused on software development tasks.

## Capabilities

- Code generation in any language
- Code review and analysis
- Debugging and troubleshooting
- Architecture design
- Best practices and optimization

## Approach

1. **Understand Requirements**: Clarify what needs to be built
2. **Design**: Plan the structure before coding
3. **Implement**: Write clean, maintainable code
4. **Test**: Verify the code works correctly
5. **Document**: Add appropriate comments and documentation

## Output Format

When providing code:
- Use proper syntax highlighting
- Include comments explaining complex logic
- Provide usage examples when helpful
- Note any dependencies or prerequisites

## Guidelines

- Follow language-specific best practices
- Write readable, maintainable code
- Consider edge cases and error handling
- Suggest improvements when reviewing existing code
- Keep security in mind
"#;

const WRITING_AGENT_INSTRUCTIONS: &str = r#"# Writing Agent

You are a specialized writing agent focused on content creation and communication.

## Capabilities

- Content creation (articles, blog posts, documentation)
- Editing and proofreading
- Tone and style adaptation
- Summarization and expansion
- Translation and localization support

## Approach

1. **Understand the Brief**: Clarify purpose, audience, tone
2. **Outline**: Structure the content logically
3. **Draft**: Write the initial content
4. **Refine**: Edit for clarity, flow, and impact
5. **Polish**: Final proofreading and formatting

## Output Format

- Use clear headings and structure
- Match the requested tone and style
- Include formatting appropriate for the medium
- Provide word count if requested

## Guidelines

- Adapt tone to audience (professional, casual, technical)
- Be concise without sacrificing clarity
- Use active voice when possible
- Vary sentence structure for readability
- Fact-check any claims made
"#;

/// Create a shared agent service.
pub fn shared_agent_service(agents_dir: PathBuf) -> Arc<AgentService> {
    Arc::new(AgentService::new(agents_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent(name: &str) -> Agent {
        Agent {
            id: String::new(),
            name: name.to_string(),
            display_name: "Test Agent".to_string(),
            description: "Test agent".to_string(),
            agent_type: Some("llm".to_string()),
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            temperature: 0.7,
            max_input_tokens: DEFAULT_MAX_INPUT_TOKENS,
            max_input_tokens_explicit: false,
            max_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            thinking_enabled: false,
            voice_recording_enabled: false,
            system_instruction: None,
            instructions: "Test instructions".to_string(),
            mcps: vec![],
            skills: vec![],
            middleware: None,
            created_at: None,
        }
    }

    #[tokio::test]
    async fn create_rejects_invalid_agent_name_before_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());

        let err = service
            .create(sample_agent("../escape"))
            .await
            .expect_err("invalid name should be rejected");

        assert!(err.contains("Invalid agent name"));
        assert!(!dir.path().join("escape").exists());
    }

    #[tokio::test]
    async fn create_rejects_reserved_root_agent_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());

        let err = service
            .create(sample_agent("root"))
            .await
            .expect_err("root should be rejected");

        assert!(err.contains("Invalid agent name"));
        assert!(!dir.path().join("root").exists());
    }

    #[tokio::test]
    async fn get_update_delete_reject_path_like_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());

        let get_err = service.get("../escape").await.expect_err("get rejects");
        let update_err = service
            .update("../escape", sample_agent("safe-agent"))
            .await
            .expect_err("update rejects");
        let delete_err = service
            .delete("../escape")
            .await
            .expect_err("delete rejects");

        assert!(get_err.contains("Invalid agent name"));
        assert!(update_err.contains("Invalid agent name"));
        assert!(delete_err.contains("Invalid agent name"));
    }

    #[tokio::test]
    async fn list_filters_reserved_agent_folders() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());
        service
            .create(sample_agent("normal-agent"))
            .await
            .expect("normal agent");
        let root_dir = dir.path().join("root");
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::write(
            root_dir.join("config.yaml"),
            "name: root\n\
             displayName: Evil Root\n\
             description: Root shadow\n\
             providerId: provider\n\
             model: model\n\
             temperature: 0.7\n\
             maxTokens: 8192\n\
             thinkingEnabled: false\n\
             voiceRecordingEnabled: false\n\
             skills: []\n\
             mcps: []\n",
        )
        .unwrap();
        std::fs::write(root_dir.join("AGENTS.md"), "malicious root\n").unwrap();

        let agents = service.list().await.expect("list");

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "normal-agent");
    }

    #[tokio::test]
    async fn create_rejects_existing_agent_without_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing_dir = dir.path().join("reviewer-agent");
        std::fs::create_dir_all(&existing_dir).unwrap();
        std::fs::write(existing_dir.join("config.yaml"), "existing: true\n").unwrap();
        std::fs::write(existing_dir.join("AGENTS.md"), "existing instructions\n").unwrap();
        let service = AgentService::new(dir.path().to_path_buf());

        let err = service
            .create(sample_agent("reviewer-agent"))
            .await
            .expect_err("existing agent should be rejected");

        assert!(err.contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(existing_dir.join("config.yaml")).unwrap(),
            "existing: true\n"
        );
        assert_eq!(
            std::fs::read_to_string(existing_dir.join("AGENTS.md")).unwrap(),
            "existing instructions\n"
        );
    }

    #[tokio::test]
    async fn update_rejects_rename_to_existing_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());
        service
            .create(sample_agent("source-agent"))
            .await
            .expect("source");
        service
            .create(sample_agent("target-agent"))
            .await
            .expect("target");
        let mut updated = sample_agent("target-agent");
        updated.display_name = "Replacement".to_string();

        let err = service
            .update("source-agent", updated)
            .await
            .expect_err("rename to existing should fail");

        assert!(err.contains("already exists"));
        let target_config =
            std::fs::read_to_string(dir.path().join("target-agent").join("config.yaml")).unwrap();
        assert!(target_config.contains("displayName: Test Agent"));
        assert!(!target_config.contains("Replacement"));
    }

    #[tokio::test]
    async fn seed_default_agents_creates_reviewer_agent_from_template() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());
        let templates = br#"[
            {
                "name": "reviewer-agent",
                "displayName": "Reviewer",
                "description": "Read-only reviewer",
                "agentType": "specialist",
                "temperature": 0.1,
                "maxTokens": 16384,
                "skills": [],
                "mcps": []
            }
        ]"#;

        service
            .seed_default_agents("provider", "model", Some(templates), |name| {
                (name == "reviewer-agent").then(|| {
                    "You are a read-only reviewer.\nRESULT: APPROVED\nRESULT: DEFECTS".to_string()
                })
            })
            .await
            .expect("seed defaults");

        let reviewer = service.get("reviewer-agent").await.expect("reviewer");
        assert_eq!(reviewer.name, "reviewer-agent");
        assert_eq!(reviewer.display_name, "Reviewer");
        assert!(!reviewer.max_input_tokens_explicit);
        assert!(reviewer.instructions.contains("read-only reviewer"));
    }

    #[test]
    fn agent_config_reads_legacy_max_tokens_as_output_tokens() {
        let yaml = r#"
name: test-agent
displayName: Test
description: Test agent
providerId: provider
model: model
temperature: 0.7
maxInputTokens: 123456
maxTokens: 7777
thinkingEnabled: false
voiceRecordingEnabled: false
skills: []
mcps: []
"#;

        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.max_input_tokens, Some(123456));
        assert_eq!(config.max_tokens, 7777);
    }

    #[test]
    fn agent_config_defaults_to_simplified_token_limits() {
        let yaml = r#"
name: test-agent
displayName: Test
description: Test agent
providerId: provider
model: model
temperature: 0.7
thinkingEnabled: false
voiceRecordingEnabled: false
skills: []
mcps: []
"#;

        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.max_input_tokens, None);
        assert_eq!(config.max_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
    }

    #[tokio::test]
    async fn loaded_agent_tracks_absent_max_input_tokens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent_dir = dir.path().join("legacy-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("config.yaml"),
            "name: legacy-agent\n\
             displayName: Legacy\n\
             description: Legacy\n\
             providerId: provider\n\
             model: model\n\
             temperature: 0.7\n\
             maxTokens: 8192\n\
             thinkingEnabled: false\n\
             voiceRecordingEnabled: false\n\
             skills: []\n\
             mcps: []\n",
        )
        .unwrap();
        std::fs::write(agent_dir.join("AGENTS.md"), "Legacy instructions\n").unwrap();

        let service = AgentService::new(dir.path().to_path_buf());
        let agent = service.get("legacy-agent").await.expect("agent");

        assert_eq!(agent.max_input_tokens, DEFAULT_MAX_INPUT_TOKENS);
        assert!(!agent.max_input_tokens_explicit);
    }

    #[tokio::test]
    async fn loaded_agent_preserves_explicit_default_max_input_tokens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent_dir = dir.path().join("explicit-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("config.yaml"),
            format!(
                "name: explicit-agent\n\
                 displayName: Explicit\n\
                 description: Explicit\n\
                 providerId: provider\n\
                 model: model\n\
                 temperature: 0.7\n\
                 maxInputTokens: {}\n\
                 maxTokens: 8192\n\
                 thinkingEnabled: false\n\
                 voiceRecordingEnabled: false\n\
                 skills: []\n\
                 mcps: []\n",
                DEFAULT_MAX_INPUT_TOKENS
            ),
        )
        .unwrap();
        std::fs::write(agent_dir.join("AGENTS.md"), "Explicit instructions\n").unwrap();

        let service = AgentService::new(dir.path().to_path_buf());
        let agent = service.get("explicit-agent").await.expect("agent");

        assert_eq!(agent.max_input_tokens, DEFAULT_MAX_INPUT_TOKENS);
        assert!(agent.max_input_tokens_explicit);
    }

    #[tokio::test]
    async fn refresh_default_agent_updates_known_template_managed_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());
        let agent_dir = dir.path().join("builder-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("config.yaml"), "name: builder-agent\ndisplayName: Builder\ndescription: Builder\nproviderId: provider\nmodel: model\ntemperature: 0.1\nmaxTokens: 8192\nthinkingEnabled: false\nvoiceRecordingEnabled: false\nskills: []\nmcps: []\n").unwrap();
        let old = "old template\n\n";
        std::fs::write(agent_dir.join("AGENTS.md"), old).unwrap();
        let old_hash = agent_runtime::content_hash(&normalize_agent_instructions(old));

        let refreshed = service
            .refresh_default_agent_instructions_if_managed(
                "builder-agent",
                "new template",
                &[old_hash.as_str()],
            )
            .await
            .expect("refresh");

        assert!(refreshed);
        assert_eq!(
            std::fs::read_to_string(agent_dir.join("AGENTS.md")).unwrap(),
            "new template\n"
        );
        let backups = std::fs::read_dir(&agent_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.md.bak-")
            })
            .count();
        assert_eq!(backups, 1);
    }

    #[tokio::test]
    async fn refresh_default_agent_preserves_customized_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());
        let agent_dir = dir.path().join("builder-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let custom = "old template\ncustom line\n";
        std::fs::write(agent_dir.join("AGENTS.md"), custom).unwrap();

        let refreshed = service
            .refresh_default_agent_instructions_if_managed(
                "builder-agent",
                "new template",
                &["not-the-custom-hash"],
            )
            .await
            .expect("refresh");

        assert!(!refreshed);
        assert_eq!(
            std::fs::read_to_string(agent_dir.join("AGENTS.md")).unwrap(),
            custom
        );
    }

    #[tokio::test]
    async fn refresh_default_agent_is_idempotent_after_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = AgentService::new(dir.path().to_path_buf());
        let agent_dir = dir.path().join("builder-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let old = "old template\n";
        std::fs::write(agent_dir.join("AGENTS.md"), old).unwrap();
        let old_hash = agent_runtime::content_hash(&normalize_agent_instructions(old));

        assert!(service
            .refresh_default_agent_instructions_if_managed(
                "builder-agent",
                "new template",
                &[old_hash.as_str()],
            )
            .await
            .unwrap());
        assert!(!service
            .refresh_default_agent_instructions_if_managed(
                "builder-agent",
                "new template",
                &[old_hash.as_str()],
            )
            .await
            .unwrap());
    }
}
