// ============================================================================
// AGENT INDEXER MODULE
// Scans ~/Documents/zbot/agents/ directory for config.yaml files
// Parses agent configurations and builds metadata for indexing
// ============================================================================

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use agent_primitives::{Result, AgentError};

/// Parsed metadata from an agent's config.yaml
#[derive(Debug, Clone)]
pub struct AgentMetadata {
    /// Agent identifier (kebab-case, matches directory name)
    pub name: String,
    /// Human-readable display name
    pub display_name: String,
    /// Brief description of what this agent does
    pub description: String,
    /// Model name (e.g., 'gpt-4o', 'claude-3-5-sonnet-20241022')
    pub model: String,
    /// Provider ID (must exist in providers.json)
    pub provider_id: String,
    /// List of tool names enabled for this agent
    pub tools: Vec<String>,
    /// List of skill IDs to include
    pub skills: Vec<String>,
    /// List of MCP server IDs to include
    pub mcps: Vec<String>,
    /// Path to the config.yaml file
    pub file_path: PathBuf,
    /// Last modification time for staleness detection
    pub mtime: SystemTime,
}

/// Internal structure for parsing YAML config files
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentConfigYaml {
    name: String,
    #[serde(default)]
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    #[serde(rename = "providerId")]
    provider_id: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    skills: Option<Vec<String>>,
    #[serde(default)]
    mcps: Option<Vec<String>>,
}

/// Scan agents directory and return metadata for all agents
///
/// # Arguments
/// * `agents_dir` - Path to the agents directory (e.g., ~/Documents/zbot/agents/)
///
/// # Returns
/// * `Vec<AgentMetadata>` - List of all discovered agents with their metadata
///
/// # Errors
/// * `AgentError::Io` - If directory cannot be read
/// * `AgentError::Tool` - If directory does not exist
pub fn scan_agents_dir(agents_dir: &PathBuf) -> Result<Vec<AgentMetadata>> {
    // Check if directory exists
    if !agents_dir.exists() {
        tracing::debug!("Agents directory does not exist: {:?}", agents_dir);
        return Ok(Vec::new());
    }

    if !agents_dir.is_dir() {
        return Err(AgentError::Tool(format!(
            "Agents path is not a directory: {:?}",
            agents_dir
        )));
    }

    let mut agents = Vec::new();

    // Iterate subdirectories
    let entries = std::fs::read_dir(agents_dir).map_err(|e| {
        AgentError::Tool(format!(
            "Failed to read agents directory {:?}: {}",
            agents_dir, e
        ))
    })?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read directory entry: {}", e);
                continue;
            }
        };

        let agent_path = entry.path();

        // Skip non-directories
        if !agent_path.is_dir() {
            continue;
        }

        // Skip hidden directories (starting with .)
        if agent_path
            .file_name()
            .map(|n| n.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
        {
            continue;
        }

        // Look for config.yaml
        let config_file = agent_path.join("config.yaml");
        if !config_file.exists() {
            // Also try config.yml as an alternative
            let alt_config = agent_path.join("config.yml");
            if alt_config.exists() {
                if let Some(metadata) = parse_agent_config(&alt_config)? {
                    agents.push(metadata);
                }
            }
            continue;
        }

        // Parse and extract metadata
        match parse_agent_config(&config_file) {
            Ok(Some(metadata)) => agents.push(metadata),
            Ok(None) => {
                tracing::debug!(
                    "No metadata extracted from agent config: {:?}",
                    config_file
                );
            }
            Err(e) => {
                tracing::warn!("Failed to parse agent config {:?}: {}", config_file, e);
            }
        }
    }

    tracing::debug!("Scanned {} agent(s) from {:?}", agents.len(), agents_dir);
    Ok(agents)
}

/// Parse agent config.yaml and extract metadata
///
/// # Arguments
/// * `config_path` - Path to the config.yaml file
///
/// # Returns
/// * `Option<AgentMetadata>` - Parsed metadata, or None if parsing fails
///
/// # Errors
/// * `AgentError::Io` - If file cannot be read
/// * `AgentError::Tool` - If YAML parsing fails
pub fn parse_agent_config(config_path: &PathBuf) -> Result<Option<AgentMetadata>> {
    // Get file metadata for mtime
    let metadata = std::fs::metadata(config_path).map_err(|e| {
        AgentError::Tool(format!(
            "Failed to get metadata for {:?}: {}",
            config_path, e
        ))
    })?;

    let mtime = metadata.modified().map_err(|e| {
        AgentError::Tool(format!(
            "Failed to get modification time for {:?}: {}",
            config_path, e
        ))
    })?;

    // Read file content
    let content = std::fs::read_to_string(config_path).map_err(|e| {
        AgentError::Tool(format!("Failed to read {:?}: {}", config_path, e))
    })?;

    // Parse YAML
    let config: AgentConfigYaml = match serde_yaml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to parse YAML for {:?}: {}", config_path, e);
            return Ok(None);
        }
    };

    // Extract directory name as fallback for agent name
    let dir_name = config_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| config.name.clone());

    // Build metadata with defaults for missing fields
    let metadata = AgentMetadata {
        name: if config.name.is_empty() {
            dir_name
        } else {
            config.name
        },
        display_name: config
            .display_name
            .unwrap_or_else(|| "Unnamed Agent".to_string()),
        description: config.description.unwrap_or_default(),
        model: config.model.unwrap_or_default(),
        provider_id: config.provider_id.unwrap_or_default(),
        tools: config.tools.unwrap_or_default(),
        skills: config.skills.unwrap_or_default(),
        mcps: config.mcps.unwrap_or_default(),
        file_path: config_path.clone(),
        mtime,
    };

    Ok(Some(metadata))
}

/// Build a memory fact for semantic search
///
/// Creates a MemoryFact-compatible JSON structure for indexing
/// in the semantic search system.
///
/// # Arguments
/// * `agent` - Agent metadata to build fact from
///
/// # Returns
/// * `Value` - JSON structure with category, key, content, confidence, and scope
pub fn build_agent_memory_fact(agent: &AgentMetadata) -> Value {
    // Create content string combining name, description, skills, tools
    let skills_str = agent.skills.join(" ");
    let tools_str = agent.tools.join(" ");
    let content = format!(
        "{} {} {} {} {}",
        agent.name, agent.display_name, agent.description, skills_str, tools_str
    );

    json!({
        "category": "agent",
        "key": format!("agent:{}", agent.name),
        "content": content,
        "confidence": 1.0,
        "scope": "agent",
        "metadata": {
            "display_name": agent.display_name,
            "model": agent.model,
            "provider_id": agent.provider_id,
            "file_path": agent.file_path.to_string_lossy().to_string()
        }
    })
}

/// Build a knowledge graph entity for relationship tracking
///
/// Creates a knowledge graph entity structure for tracking
/// relationships between agents, skills, and MCPs.
///
/// # Arguments
/// * `agent` - Agent metadata to build entity from
///
/// # Returns
/// * `Value` - JSON structure with entity_type, name, and properties
pub fn build_agent_entity(agent: &AgentMetadata) -> Value {
    // Convert mtime to Unix timestamp
    let mtime_secs = agent
        .mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    json!({
        "entity_type": "agent",
        "name": agent.name,
        "properties": {
            "display_name": agent.display_name,
            "description": agent.description,
            "model": agent.model,
            "provider_id": agent.provider_id,
            "tools": agent.tools,
            "skills": agent.skills,
            "mcps": agent.mcps,
            "file_path": agent.file_path.to_string_lossy().to_string(),
            "mtime": mtime_secs
        }
    })
}

/// Check if an agent config has been modified since the given time
///
/// # Arguments
/// * `agent` - Agent metadata to check
/// * `since` - Time to compare against
///
/// # Returns
/// * `bool` - True if agent has been modified since the given time
pub fn is_agent_modified(agent: &AgentMetadata, since: SystemTime) -> bool {
    agent.mtime > since
}

/// Get the list of skills referenced by an agent
///
/// # Arguments
/// * `agent` - Agent metadata to extract skills from
///
/// # Returns
/// * `Vec<String>` - List of skill names referenced by this agent
pub fn get_agent_skill_refs(agent: &AgentMetadata) -> Vec<String> {
    agent.skills.clone()
}

/// Get the list of MCPs referenced by an agent
///
/// # Arguments
/// * `agent` - Agent metadata to extract MCPs from
///
/// # Returns
/// * `Vec<String>` - List of MCP server IDs referenced by this agent
pub fn get_agent_mcp_refs(agent: &AgentMetadata) -> Vec<String> {
    agent.mcps.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_agent_config(dir: &TempDir, name: &str, yaml_content: &str) -> PathBuf {
        let agent_dir = dir.path().join(name);
        std::fs::create_dir_all(&agent_dir).unwrap();

        let config_path = agent_dir.join("config.yaml");
        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(yaml_content.as_bytes()).unwrap();

        config_path
    }

    #[test]
    fn test_parse_agent_config_basic() {
        let dir = TempDir::new().unwrap();
        let yaml = r#"
name: test-agent
displayName: Test Agent
description: A test agent for unit testing
providerId: openai
model: gpt-4o
"#;
        let path = create_test_agent_config(&dir, "test-agent", yaml);

        let result = parse_agent_config(&path).unwrap();
        assert!(result.is_some());

        let agent = result.unwrap();
        assert_eq!(agent.name, "test-agent");
        assert_eq!(agent.display_name, "Test Agent");
        assert_eq!(agent.description, "A test agent for unit testing");
        assert_eq!(agent.provider_id, "openai");
        assert_eq!(agent.model, "gpt-4o");
    }

    #[test]
    fn test_parse_agent_config_with_skills() {
        let dir = TempDir::new().unwrap();
        let yaml = r#"
name: skillful-agent
displayName: Skillful Agent
description: An agent with skills
providerId: anthropic
model: claude-3-5-sonnet-20241022
skills:
  - coding
  - research
  - web-search
mcps:
  - filesystem
  - postgres
"#;
        let path = create_test_agent_config(&dir, "skillful-agent", yaml);

        let result = parse_agent_config(&path).unwrap();
        assert!(result.is_some());

        let agent = result.unwrap();
        assert_eq!(agent.skills.len(), 3);
        assert!(agent.skills.contains(&"coding".to_string()));
        assert!(agent.skills.contains(&"research".to_string()));
        assert_eq!(agent.mcps.len(), 2);
        assert!(agent.mcps.contains(&"filesystem".to_string()));
    }

    #[test]
    fn test_scan_agents_dir() {
        let dir = TempDir::new().unwrap();

        // Create multiple agent configs
        let yaml1 = r#"
name: agent-one
displayName: Agent One
description: First agent
providerId: openai
model: gpt-4o
"#;
        let yaml2 = r#"
name: agent-two
displayName: Agent Two
description: Second agent
providerId: anthropic
model: claude-3-5-sonnet-20241022
skills:
  - analysis
"#;

        create_test_agent_config(&dir, "agent-one", yaml1);
        create_test_agent_config(&dir, "agent-two", yaml2);

        // Create a hidden directory that should be skipped
        let hidden_dir = dir.path().join(".hidden");
        std::fs::create_dir_all(&hidden_dir).unwrap();
        let mut hidden_file = std::fs::File::create(hidden_dir.join("config.yaml")).unwrap();
        hidden_file
            .write_all(b"name: hidden-agent\ndescription: Should not appear\n")
            .unwrap();

        let agents = scan_agents_dir(&dir.path().to_path_buf()).unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents.iter().any(|a| a.name == "agent-one"));
        assert!(agents.iter().any(|a| a.name == "agent-two"));
        assert!(!agents.iter().any(|a| a.name == "hidden-agent"));
    }

    #[test]
    fn test_build_agent_memory_fact() {
        let agent = AgentMetadata {
            name: "test-agent".to_string(),
            display_name: "Test Agent".to_string(),
            description: "Does testing".to_string(),
            model: "gpt-4o".to_string(),
            provider_id: "openai".to_string(),
            tools: vec!["shell".to_string()],
            skills: vec!["coding".to_string()],
            mcps: vec![],
            file_path: PathBuf::from("/test/config.yaml"),
            mtime: SystemTime::UNIX_EPOCH,
        };

        let fact = build_agent_memory_fact(&agent);

        assert_eq!(fact["category"], "agent");
        assert_eq!(fact["key"], "agent:test-agent");
        assert_eq!(fact["confidence"], 1.0);
        assert_eq!(fact["scope"], "agent");
        assert!(fact["content"].as_str().unwrap().contains("test-agent"));
        assert!(fact["content"].as_str().unwrap().contains("coding"));
    }

    #[test]
    fn test_build_agent_entity() {
        let agent = AgentMetadata {
            name: "entity-agent".to_string(),
            display_name: "Entity Agent".to_string(),
            description: "For entity testing".to_string(),
            model: "claude-3".to_string(),
            provider_id: "anthropic".to_string(),
            tools: vec!["memory".to_string()],
            skills: vec!["research".to_string()],
            mcps: vec!["database".to_string()],
            file_path: PathBuf::from("/agents/entity-agent/config.yaml"),
            mtime: SystemTime::UNIX_EPOCH,
        };

        let entity = build_agent_entity(&agent);

        assert_eq!(entity["entity_type"], "agent");
        assert_eq!(entity["name"], "entity-agent");
        assert_eq!(entity["properties"]["display_name"], "Entity Agent");
        assert_eq!(entity["properties"]["model"], "claude-3");
        assert_eq!(entity["properties"]["provider_id"], "anthropic");
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let agents = scan_agents_dir(&dir.path().to_path_buf()).unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_directory() {
        let path = PathBuf::from("/nonexistent/path/that/does/not/exist");
        let agents = scan_agents_dir(&path).unwrap();
        assert!(agents.is_empty());
    }
}
