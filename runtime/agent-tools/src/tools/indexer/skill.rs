// ============================================================================
// SKILL INDEXER MODULE
// Scans skills directory for SKILL.md files
// Parses skill frontmatter and builds metadata for indexing
// ============================================================================

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use agent_primitives::{Result, AgentError};

/// Parsed metadata from a skill's SKILL.md file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill identifier (directory name)
    pub name: String,
    /// Brief description of what this skill does
    pub description: String,
    /// Keywords that trigger this skill
    #[serde(default)]
    pub trigger_keywords: Vec<String>,
    /// Domain hints for classification
    #[serde(default)]
    pub domain_hints: Vec<String>,
    /// Tools required by this skill
    #[serde(default)]
    pub tools: Vec<String>,
    /// Model preference for this skill
    #[serde(default)]
    pub model: Option<String>,
    /// Path to the SKILL.md file
    #[serde(skip)]
    #[serde(default = "PathBuf::new")]
    pub file_path: PathBuf,
    /// Last modification time for staleness detection
    #[serde(skip)]
    #[serde(default = "default_mtime")]
    pub mtime: SystemTime,
}

fn default_mtime() -> SystemTime {
    SystemTime::UNIX_EPOCH
}

/// Internal structure for parsing YAML frontmatter
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    #[serde(rename = "trigger_keywords")]
    trigger_keywords: Option<Vec<String>>,
    #[serde(default)]
    #[serde(rename = "domain_hints")]
    domain_hints: Option<Vec<String>>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    model: Option<String>,
}

/// Scan skills directory and return metadata for all skills
///
/// # Arguments
/// * `skills_dir` - Path to the skills directory
///
/// # Returns
/// * `Vec<SkillMetadata>` - List of all discovered skills with their metadata
///
/// # Errors
/// * `AgentError::Tool` - If directory does not exist or cannot be read
pub fn scan_skills_dir(skills_dir: &PathBuf) -> Result<Vec<SkillMetadata>> {
    // Check if directory exists
    if !skills_dir.exists() {
        tracing::debug!("Skills directory does not exist: {:?}", skills_dir);
        return Ok(Vec::new());
    }

    if !skills_dir.is_dir() {
        return Err(AgentError::Tool(format!(
            "Skills path is not a directory: {:?}",
            skills_dir
        )));
    }

    let mut skills = Vec::new();

    // Iterate subdirectories
    let entries = std::fs::read_dir(skills_dir).map_err(|e| {
        AgentError::Tool(format!(
            "Failed to read skills directory {:?}: {}",
            skills_dir, e
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

        let skill_path = entry.path();

        // Skip non-directories
        if !skill_path.is_dir() {
            continue;
        }

        // Skip hidden directories (starting with .)
        if skill_path
            .file_name()
            .map(|n| n.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
        {
            continue;
        }

        // Look for SKILL.md
        let skill_file = skill_path.join("SKILL.md");
        if !skill_file.exists() {
            continue;
        }

        // Parse and extract metadata
        match parse_skill_md(&skill_file) {
            Ok(Some(metadata)) => skills.push(metadata),
            Ok(None) => {
                tracing::debug!("No metadata extracted from skill file: {:?}", skill_file);
            }
            Err(e) => {
                tracing::warn!("Failed to parse skill file {:?}: {}", skill_file, e);
            }
        }
    }

    tracing::debug!("Scanned {} skill(s) from {:?}", skills.len(), skills_dir);
    Ok(skills)
}

/// Parse skill SKILL.md and extract metadata
///
/// # Arguments
/// * `skill_path` - Path to the SKILL.md file
///
/// # Returns
/// * `Option<SkillMetadata>` - Parsed metadata, or None if parsing fails
///
/// # Errors
/// * `AgentError::Io` - If file cannot be read
pub fn parse_skill_md(skill_path: &PathBuf) -> Result<Option<SkillMetadata>> {
    // Get file metadata for mtime
    let metadata = std::fs::metadata(skill_path).map_err(|e| {
        AgentError::Tool(format!(
            "Failed to get metadata for {:?}: {}",
            skill_path, e
        ))
    })?;

    let mtime = metadata.modified().map_err(|e| {
        AgentError::Tool(format!(
            "Failed to get modification time for {:?}: {}",
            skill_path, e
        ))
    })?;

    // Read file content
    let content = std::fs::read_to_string(skill_path).map_err(|e| {
        AgentError::Tool(format!("Failed to read {:?}: {}", skill_path, e))
    })?;

    // Extract directory name as skill name
    let dir_name = skill_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Parse frontmatter
    let (frontmatter, _) = parse_frontmatter(&content);

    // Build metadata
    let metadata = SkillMetadata {
        name: frontmatter.name.unwrap_or(dir_name),
        description: frontmatter.description.unwrap_or_default(),
        trigger_keywords: frontmatter.trigger_keywords.unwrap_or_default(),
        domain_hints: frontmatter.domain_hints.unwrap_or_default(),
        tools: frontmatter.tools.unwrap_or_default(),
        model: frontmatter.model,
        file_path: skill_path.clone(),
        mtime,
    };

    Ok(Some(metadata))
}

/// Parse YAML frontmatter from skill content
///
/// # Arguments
/// * `content` - Raw SKILL.md content
///
/// # Returns
/// * `(SkillFrontmatter, &str)` - Parsed frontmatter and remaining content
fn parse_frontmatter(content: &str) -> (SkillFrontmatter, &str) {
    if !content.starts_with("---") {
        return (SkillFrontmatter::default(), content);
    }

    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return (SkillFrontmatter::default(), content);
    }

    let yaml_content = parts[1].trim();
    let remaining = parts[2].trim();

    let frontmatter: SkillFrontmatter = match serde_yaml::from_str(yaml_content) {
        Ok(fm) => fm,
        Err(e) => {
            tracing::debug!("Failed to parse frontmatter: {}", e);
            SkillFrontmatter::default()
        }
    };

    (frontmatter, remaining)
}

/// Build a memory fact for semantic search
///
/// Creates a MemoryFact-compatible JSON structure for indexing
/// in the semantic search system.
///
/// # Arguments
/// * `skill` - Skill metadata to build fact from
///
/// # Returns
/// * `Value` - JSON structure with category, key, content, confidence, and scope
pub fn build_skill_memory_fact(skill: &SkillMetadata) -> Value {
    // Create content string combining all searchable fields
    let mut content_parts: Vec<&str> = vec![&skill.name, &skill.description];

    // Add keywords
    for keyword in &skill.trigger_keywords {
        content_parts.push(keyword);
    }

    // Add domain hints
    for hint in &skill.domain_hints {
        content_parts.push(hint);
    }

    let content = content_parts.join(" ");

    json!({
        "category": "skill",
        "key": format!("skill:{}", skill.name),
        "content": content,
        "confidence": 1.0,
        "scope": "agent",
        "metadata": {
            "description": skill.description,
            "trigger_keywords": skill.trigger_keywords,
            "domain_hints": skill.domain_hints,
            "tools": skill.tools,
            "model": skill.model,
            "file_path": skill.file_path.to_string_lossy().to_string()
        }
    })
}

/// Build a knowledge graph entity for relationship tracking
///
/// Creates a knowledge graph entity structure for tracking
/// relationships between skills, agents, and tools.
///
/// # Arguments
/// * `skill` - Skill metadata to build entity from
///
/// # Returns
/// * `Value` - JSON structure with entity_type, name, and properties
pub fn build_skill_entity(skill: &SkillMetadata) -> Value {
    // Convert mtime to Unix timestamp
    let mtime_secs = skill
        .mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    json!({
        "entity_type": "skill",
        "name": skill.name,
        "properties": {
            "description": skill.description,
            "trigger_keywords": skill.trigger_keywords,
            "domain_hints": skill.domain_hints,
            "tools": skill.tools,
            "model": skill.model,
            "file_path": skill.file_path.to_string_lossy().to_string(),
            "mtime": mtime_secs
        }
    })
}

/// Check if a skill file has been modified since the given time
///
/// # Arguments
/// * `skill` - Skill metadata to check
/// * `since` - Time to compare against
///
/// # Returns
/// * `bool` - True if skill has been modified since the given time
pub fn is_skill_modified(skill: &SkillMetadata, since: SystemTime) -> bool {
    skill.mtime > since
}

/// Get the list of tools referenced by a skill
///
/// # Arguments
/// * `skill` - Skill metadata to extract tools from
///
/// # Returns
/// * `Vec<String>` - List of tool names referenced by this skill
pub fn get_skill_tool_refs(skill: &SkillMetadata) -> Vec<String> {
    skill.tools.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_skill_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let skill_dir = dir.path().join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();

        let skill_path = skill_dir.join("SKILL.md");
        let mut file = std::fs::File::create(&skill_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        skill_path
    }

    #[test]
    fn test_parse_skill_md_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        let content = r#"---
name: test-skill
description: A test skill for unit testing
trigger_keywords:
  - test
  - example
domain_hints:
  - testing
tools:
  - read
  - write
model: claude-opus-4-5-20251101
---

# Test Skill

This is the skill body.
"#;
        let path = create_test_skill_file(&dir, "test-skill", content);

        let result = parse_skill_md(&path).unwrap();
        assert!(result.is_some());

        let skill = result.unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill for unit testing");
        assert_eq!(skill.trigger_keywords.len(), 2);
        assert!(skill.trigger_keywords.contains(&"test".to_string()));
        assert_eq!(skill.domain_hints.len(), 1);
        assert_eq!(skill.tools.len(), 2);
        assert_eq!(skill.model, Some("claude-opus-4-5-20251101".to_string()));
    }

    #[test]
    fn test_parse_skill_md_without_frontmatter() {
        let dir = TempDir::new().unwrap();
        let content = r#"# Plain Skill

This skill has no frontmatter.
Just plain markdown content.
"#;
        let path = create_test_skill_file(&dir, "plain-skill", content);

        let result = parse_skill_md(&path).unwrap();
        assert!(result.is_some());

        let skill = result.unwrap();
        assert_eq!(skill.name, "plain-skill"); // Uses directory name
        assert_eq!(skill.description, "");
        assert!(skill.trigger_keywords.is_empty());
    }

    #[test]
    fn test_scan_skills_dir() {
        let dir = TempDir::new().unwrap();

        // Create multiple skill files
        let content1 = r#"---
name: skill-one
description: First skill
---
# Skill One
"#;
        let content2 = r#"---
name: skill-two
description: Second skill
tools:
  - shell
---
# Skill Two
"#;

        create_test_skill_file(&dir, "skill-one", content1);
        create_test_skill_file(&dir, "skill-two", content2);

        // Create a hidden directory that should be skipped
        let hidden_dir = dir.path().join(".hidden");
        std::fs::create_dir_all(&hidden_dir).unwrap();
        let mut hidden_file = std::fs::File::create(hidden_dir.join("SKILL.md")).unwrap();
        hidden_file
            .write_all(b"---\nname: hidden-skill\n---\nHidden content\n")
            .unwrap();

        let skills = scan_skills_dir(&dir.path().to_path_buf()).unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills.iter().any(|s| s.name == "skill-one"));
        assert!(skills.iter().any(|s| s.name == "skill-two"));
        assert!(!skills.iter().any(|s| s.name == "hidden-skill"));
    }

    #[test]
    fn test_build_skill_memory_fact() {
        let skill = SkillMetadata {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            trigger_keywords: vec!["test".to_string(), "example".to_string()],
            domain_hints: vec!["testing".to_string()],
            tools: vec!["read".to_string()],
            model: Some("claude-3".to_string()),
            file_path: PathBuf::from("/test/SKILL.md"),
            mtime: SystemTime::UNIX_EPOCH,
        };

        let fact = build_skill_memory_fact(&skill);

        assert_eq!(fact["category"], "skill");
        assert_eq!(fact["key"], "skill:test-skill");
        assert_eq!(fact["confidence"], 1.0);
        assert_eq!(fact["scope"], "agent");
        assert!(fact["content"].as_str().unwrap().contains("test-skill"));
        assert!(fact["content"].as_str().unwrap().contains("test"));
        assert!(fact["content"].as_str().unwrap().contains("testing"));
    }

    #[test]
    fn test_build_skill_entity() {
        let skill = SkillMetadata {
            name: "entity-skill".to_string(),
            description: "For entity testing".to_string(),
            trigger_keywords: vec!["entity".to_string()],
            domain_hints: vec!["code".to_string()],
            tools: vec!["shell".to_string(), "memory".to_string()],
            model: None,
            file_path: PathBuf::from("/skills/entity-skill/SKILL.md"),
            mtime: SystemTime::UNIX_EPOCH,
        };

        let entity = build_skill_entity(&skill);

        assert_eq!(entity["entity_type"], "skill");
        assert_eq!(entity["name"], "entity-skill");
        assert_eq!(entity["properties"]["description"], "For entity testing");
        assert_eq!(entity["properties"]["tools"], json!(["shell", "memory"]));
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let skills = scan_skills_dir(&dir.path().to_path_buf()).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_directory() {
        let path = PathBuf::from("/nonexistent/path/that/does/not/exist");
        let skills = scan_skills_dir(&path).unwrap();
        assert!(skills.is_empty());
    }
}
