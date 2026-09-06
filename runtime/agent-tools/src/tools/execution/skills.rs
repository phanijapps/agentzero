// ============================================================================
// LOAD SKILL TOOL
// ============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use agent_primitives::FileSystemContext;
use agent_primitives::{Result, Tool, ToolContext};

use crate::tools::guards::has_placeholder_specs;

const MAX_SKILL_SUMMARY_CHARS: usize = 720;
const MAX_SKILL_SECTION_CHARS: usize = 520;
const MAX_SKILL_SECTIONS: usize = 6;

// ============================================================================
// SKILL STATE TYPES
// ============================================================================

/// Entry for a loaded skill in the skill graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    /// Tool call ID when the skill was loaded
    pub tool_call_id: String,
    /// Timestamp when the skill was loaded (millis since epoch)
    pub loaded_at: i64,
    /// Resources (files) loaded within this skill
    pub resources: Vec<ResourceEntry>,
}

/// Entry for a resource file loaded within a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEntry {
    /// Relative path within the skill directory
    pub path: String,
    /// Tool call ID when this resource was loaded
    pub tool_call_id: String,
}

/// Skill graph: maps skill name -> skill entry
pub type SkillGraph = HashMap<String, SkillEntry>;

// ============================================================================
// SKILL STATE HELPERS
// ============================================================================

/// Track a skill being loaded in the context state
fn track_skill_load(ctx: &Arc<dyn ToolContext>, skill_name: &str, tool_call_id: &str) {
    // Get or create the skill graph
    let mut graph: SkillGraph = ctx
        .get_state("skill:graph")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    // Add/update this skill entry
    graph.insert(
        skill_name.to_string(),
        SkillEntry {
            tool_call_id: tool_call_id.to_string(),
            loaded_at: chrono::Utc::now().timestamp_millis(),
            resources: vec![],
        },
    );

    ctx.set_state("skill:graph".to_string(), json!(graph));

    // Update the loaded_skills list
    let mut loaded: Vec<String> = ctx
        .get_state("skill:loaded_skills")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    if !loaded.contains(&skill_name.to_string()) {
        loaded.push(skill_name.to_string());
        ctx.set_state("skill:loaded_skills".to_string(), json!(loaded));
    }
}

/// Track a resource file being loaded within a skill
fn track_resource_load(
    ctx: &Arc<dyn ToolContext>,
    skill_name: &str,
    resource_path: &str,
    tool_call_id: &str,
) {
    let mut graph: SkillGraph = ctx
        .get_state("skill:graph")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    if let Some(entry) = graph.get_mut(skill_name) {
        // Add resource to existing skill entry
        entry.resources.push(ResourceEntry {
            path: resource_path.to_string(),
            tool_call_id: tool_call_id.to_string(),
        });
        ctx.set_state("skill:graph".to_string(), json!(graph));
    } else {
        // Skill not in graph yet (unusual case - resource loaded before main skill)
        // Create a placeholder entry with just this resource
        graph.insert(
            skill_name.to_string(),
            SkillEntry {
                tool_call_id: String::new(), // Unknown - skill wasn't loaded via load_skill
                loaded_at: chrono::Utc::now().timestamp_millis(),
                resources: vec![ResourceEntry {
                    path: resource_path.to_string(),
                    tool_call_id: tool_call_id.to_string(),
                }],
            },
        );
        ctx.set_state("skill:graph".to_string(), json!(graph));

        // Also add to loaded_skills list
        let mut loaded: Vec<String> = ctx
            .get_state("skill:loaded_skills")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        if !loaded.contains(&skill_name.to_string()) {
            loaded.push(skill_name.to_string());
            ctx.set_state("skill:loaded_skills".to_string(), json!(loaded));
        }
    }
}

// ============================================================================
// LOAD SKILL TOOL
// ============================================================================

/// Tool for loading skills and skill files
pub struct LoadSkillTool {
    /// File system context
    fs: Arc<dyn FileSystemContext>,
}

impl LoadSkillTool {
    /// Create a new load skill tool with file system context
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self { fs }
    }

    /// Parse skill file path
    /// Returns (skill_name, relative_path, is_explicit)
    /// Supports formats:
    /// - "@skill:skill-name/path/to/file" -> ("skill-name", "path/to/file", true)
    /// - "@skill:skill-name" -> ("skill-name", "SKILL.md", true) - loads SKILL.md explicitly
    /// - "@skill:FILENAME.md" -> ("", "FILENAME.md", false) (uses current skill)
    /// - "path/to/file" -> ("", "path/to/file", false) (uses current skill)
    fn parse_skill_path(&self, file_path: &str) -> (String, String, bool) {
        const FILE_EXTENSIONS: &[&str] = &[
            ".md", ".txt", ".json", ".yaml", ".yml", ".toml", ".html", ".css", ".js", ".ts", ".py",
            ".rs", ".pdf", ".png", ".jpg", ".jpeg", ".gif",
        ];

        if let Some(path) = file_path.strip_prefix("@skill:") {
            // Skip "@skill:"
            if path.contains('/') {
                let parts: Vec<&str> = path.splitn(2, '/').collect();
                return (parts[0].to_string(), parts[1].to_string(), true);
            }
            // @skill:skill-name or @skill:FILENAME.md
            // Check if it has a file extension
            let has_file_extension = FILE_EXTENSIONS.iter().any(|ext| path.ends_with(ext));
            if has_file_extension {
                // It's a filename, use current skill
                return (String::new(), path.to_string(), false);
            }
            // It's a skill name, load SKILL.md
            return (path.to_string(), "SKILL.md".to_string(), true);
        }
        // Relative path - will use current skill from context
        (String::new(), file_path.to_string(), false)
    }
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &str {
        "load_skill"
    }

    fn description(&self) -> &str {
        "Load a skill or files from a skill's directory. Use 'skill' parameter to load SKILL.md, or 'file' parameter with @skill: prefix (e.g., '@skill:rust-development' loads that skill's SKILL.md, '@skill:rust-development/REFERENCE.md' loads a specific file)."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Name of the skill to load (loads SKILL.md)"
                },
                "file": {
                    "type": "string",
                    "description": "Path to file within skill directory. Use '@skill:' prefix. Examples: '@skill:rust-development' (loads SKILL.md), '@skill:rust-development/REFERENCE.md', '@skill:assets/config.json' (after loading skill)"
                }
            },
            "required": []
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        if has_placeholder_specs(ctx.as_ref()) {
            return Ok(crate::tools::guards::placeholder_specs_redirect(
                "Delegate to a planning subagent to fill them; the skills each spec needs are listed in the spec file.",
            ));
        }

        // Check if loading main skill file or specific file
        let has_skill = args.get("skill").and_then(|v| v.as_str()).is_some();
        let has_file = args.get("file").and_then(|v| v.as_str()).is_some();

        if has_skill && !has_file {
            // Load main SKILL.md
            self.load_main_skill(ctx, args).await
        } else if has_file {
            // Load specific file from skill directory
            self.load_skill_file(ctx, args).await
        } else {
            Err(agent_primitives::AgentError::Tool(
                "Either 'skill' or 'file' parameter must be provided".to_string(),
            ))
        }
    }
}

impl LoadSkillTool {
    fn canonical_skill_name(skill_name: &str) -> &str {
        match skill_name {
            "planning-highlevel" => "spec-builder",
            "planning-decompose" => "plan-composer",
            _ => skill_name,
        }
    }

    fn missing_skill_hint(skill_name: &str) -> &'static str {
        match skill_name {
            "planning-highlevel" => {
                " Legacy planner skill alias: use load_skill(skill=\"spec-builder\") or install the alias."
            }
            "planning-decompose" => {
                " Legacy planner skill alias: use load_skill(skill=\"plan-composer\") or install the alias."
            }
            _ => {
                " Retry with an installed skill name or an explicit @skill:<name>/SKILL.md file handle."
            }
        }
    }

    /// Resolve a skill name to its on-disk directory. Walks every root
    /// returned by `FileSystemContext::skills_dirs()` in priority order
    /// (vault first, then `$HOME/.agents/skills` etc.) and returns the
    /// first root whose `<root>/<name>/SKILL.md` exists.
    fn resolve_skill_dir(&self, skill_name: &str) -> Result<std::path::PathBuf> {
        let roots = self.fs.skills_dirs();
        let canonical_skill_name = Self::canonical_skill_name(skill_name);
        if std::path::Path::new(canonical_skill_name)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(agent_primitives::AgentError::Tool(
                "Invalid skill name: must remain inside a configured skills root".into(),
            ));
        }
        if roots.is_empty() {
            return Err(agent_primitives::AgentError::Tool(
                "Skills directory not configured".to_string(),
            ));
        }
        for root in &roots {
            let candidate = root.join(canonical_skill_name);
            if candidate.join("SKILL.md").exists() {
                let root = root.canonicalize()?;
                let candidate = candidate.canonicalize()?;
                if !candidate.starts_with(&root) {
                    return Err(agent_primitives::AgentError::Tool(
                        "Invalid skill path: outside configured skills root".into(),
                    ));
                }
                return Ok(candidate);
            }
        }
        let searched: Vec<String> = roots
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        Err(agent_primitives::AgentError::Tool(format!(
            "Skill '{}' not found in any configured skills root (searched: {}).{}",
            skill_name,
            searched.join(", "),
            Self::missing_skill_hint(skill_name)
        )))
    }

    /// Load the main SKILL.md file for a skill
    async fn load_main_skill(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let skill_name = args.get("skill").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'skill' parameter".to_string())
        })?;
        let canonical_skill_name = Self::canonical_skill_name(skill_name);

        let skill_dir = self.resolve_skill_dir(skill_name)?;
        let skill_file = confined_skill_file(&skill_dir, "SKILL.md")?;

        let content = std::fs::read_to_string(&skill_file).map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to read skill file: {}", e))
        })?;

        // Parse YAML frontmatter
        let (metadata, instructions) = self.parse_skill_frontmatter(&content)?;

        // Store current skill in state for subsequent convenience file loads
        ctx.set_state(
            "skill:current_skill".to_string(),
            json!(canonical_skill_name),
        );

        // Track this skill load in the skill graph
        let tool_call_id = ctx.function_call_id();
        track_skill_load(&ctx, skill_name, &tool_call_id);

        // List available resource files in the skill directory
        let resources = list_skill_resources(&skill_dir, skill_name);

        Ok(build_skill_packet(
            skill_name,
            &metadata,
            &instructions,
            resources,
        ))
    }

    /// Load a specific file from a skill's directory
    async fn load_skill_file(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let file_path = args.get("file").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'file' parameter".to_string())
        })?;

        // Parse path and get skill name
        let (skill_name_from_path, relative_path, is_explicit) = self.parse_skill_path(file_path);

        // Get skill name - use explicit skill from path, or fall back to context
        let skill_name = if !skill_name_from_path.is_empty() {
            skill_name_from_path
        } else {
            ctx.get_state("skill:current_skill")
                .ok_or_else(|| agent_primitives::AgentError::Tool(
                    "No skill context. Either use @skill:skill-name/path format or load a skill first using the 'skill' parameter.".to_string()
                ))?
                .as_str()
                .ok_or_else(|| agent_primitives::AgentError::Tool("Invalid skill state".to_string()))?
                .to_string()
        };

        // Walk all configured roots so resource files in `~/.agents/skills`
        // resolve too — same priority order as the main SKILL.md loader.
        let skill_dir = self.resolve_skill_dir(&skill_name)?;
        let full_path = confined_skill_file(&skill_dir, &relative_path)?;

        // Check for binary file
        if is_binary_file(&relative_path) {
            return Ok(json!({
                "skill": skill_name,
                "path": relative_path,
                "content": null,
                "is_binary": true,
                "message": "Binary file - content not displayed"
            }));
        }

        // Read file content
        let content = std::fs::read_to_string(&full_path).map_err(|e| {
            agent_primitives::AgentError::Tool(format!("Failed to read skill file: {}", e))
        })?;

        // Get the tool call ID for tracking
        let tool_call_id = ctx.function_call_id();

        // Update current skill in state if we explicitly loaded a different skill's SKILL.md
        if is_explicit && relative_path == "SKILL.md" {
            ctx.set_state("skill:current_skill".to_string(), json!(skill_name));
            // Track as a skill load
            track_skill_load(&ctx, &skill_name, &tool_call_id);
        } else {
            // Track as a resource load under the parent skill
            track_resource_load(&ctx, &skill_name, &relative_path, &tool_call_id);
        }

        Ok(json!({
            "skill": skill_name,
            "path": relative_path,
            "content": content,
            "is_binary": false
        }))
    }

    fn parse_skill_frontmatter(&self, content: &str) -> Result<(Value, String)> {
        // Simple parser for YAML frontmatter between --- delimiters
        let parts: Vec<&str> = content.splitn(3, "---").collect();

        if parts.len() >= 3 {
            let yaml_content = parts[1].trim();
            let instructions = parts[2].trim().to_string();

            let metadata: Value = serde_yaml::from_str(yaml_content).map_err(|e| {
                agent_primitives::AgentError::Tool(format!("Failed to parse skill YAML: {}", e))
            })?;

            Ok((metadata, instructions))
        } else {
            // No frontmatter, return empty metadata
            Ok((json!({}), content.to_string()))
        }
    }
}

fn confined_skill_file(skill_dir: &std::path::Path, relative: &str) -> Result<std::path::PathBuf> {
    let path = skill_dir.join(relative).canonicalize().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            agent_primitives::AgentError::Tool(format!("Skill file not found: {relative}"))
        } else {
            agent_primitives::AgentError::Io(error)
        }
    })?;
    if !path.starts_with(skill_dir) {
        return Err(agent_primitives::AgentError::Tool(
            "Invalid path: cannot access files outside skill directory".into(),
        ));
    }
    Ok(path)
}

fn build_skill_packet(
    skill_name: &str,
    metadata: &Value,
    instructions: &str,
    resources: Vec<Value>,
) -> Value {
    let sections = summarize_sections(skill_name, instructions);
    let summary = metadata
        .get("description")
        .and_then(Value::as_str)
        .map(|description| truncate_text(description, MAX_SKILL_SUMMARY_CHARS))
        .unwrap_or_else(|| summarize_instructions(instructions));
    let token_estimate = estimate_tokens(&summary)
        + sections
            .iter()
            .map(|section| {
                section
                    .get("token_estimate")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .sum::<u64>() as usize;

    json!({
        "name": skill_name,
        "metadata": metadata,
        "packet": {
            "summary": summary,
            "sections": sections,
            "resource_uri": format!("zbot://skills/{skill_name}/SKILL.md"),
            "full_body_resource_uri": format!("zbot://skills/{skill_name}/sections/full"),
            "token_estimate": token_estimate,
            "render_policy": "summary",
            "full_body_read": "Use load_skill(file=\"@skill:<skill>/SKILL.md\") only when the full body is explicitly needed."
        },
        "resources": resources,
    })
}

fn summarize_sections(skill_name: &str, instructions: &str) -> Vec<Value> {
    let mut sections = Vec::new();
    let mut current_title = "overview".to_string();
    let mut current_body = String::new();

    for line in instructions.lines() {
        if let Some(title) = markdown_heading(line) {
            push_section(skill_name, &mut sections, &current_title, &current_body);
            if sections.len() >= MAX_SKILL_SECTIONS {
                return sections;
            }
            current_title = title.to_string();
            current_body.clear();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    push_section(skill_name, &mut sections, &current_title, &current_body);
    sections
}

fn push_section(skill_name: &str, sections: &mut Vec<Value>, title: &str, body: &str) {
    let summary = summarize_instructions(body);
    if summary.trim().is_empty() {
        return;
    }
    let slug = slugify(title);
    sections.push(json!({
        "title": title,
        "summary": truncate_text(&summary, MAX_SKILL_SECTION_CHARS),
        "resource_uri": format!("zbot://skills/{skill_name}/sections/{slug}"),
        "token_estimate": estimate_tokens(&summary),
    }));
}

fn markdown_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let title = trimmed
        .strip_prefix("### ")
        .or_else(|| trimmed.strip_prefix("## "))
        .or_else(|| trimmed.strip_prefix("# "))?;
    let title = title.trim();
    (!title.is_empty()).then_some(title)
}

fn summarize_instructions(instructions: &str) -> String {
    instructions
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("---"))
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
        .pipe(|summary| truncate_text(&summary, MAX_SKILL_SUMMARY_CHARS))
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

fn slugify(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

/// List resource files in a skill directory (excluding SKILL.md).
///
/// Returns a list of objects with filename and the load_skill command to use.
fn list_skill_resources(skill_dir: &std::path::Path, skill_name: &str) -> Vec<Value> {
    let mut resources = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                // Skip SKILL.md itself
                if name.eq_ignore_ascii_case("SKILL.md") {
                    continue;
                }
                resources.push(json!({
                    "file": name,
                    "load_with": format!("load_skill(file=\"{}\")", name),
                }));
            }
        }
    }
    // Also check subdirectories one level deep
    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && let Ok(sub_entries) = std::fs::read_dir(&path)
            {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file()
                        && let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
                        && let Some(file_name) = sub_path.file_name().and_then(|n| n.to_str())
                    {
                        let rel_path = format!("{}/{}", dir_name, file_name);
                        resources.push(json!({
                                        "file": rel_path.clone(),
                                        "load_with": format!("load_skill(file=\"@skill:{}/{}\")", skill_name, rel_path),
                                    }));
                    }
                }
            }
        }
    }
    resources
}

/// Check if a file is binary based on its extension
fn is_binary_file(filename: &str) -> bool {
    const BINARY_EXTENSIONS: &[&str] = &[
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "zip", "tar", "gz", "rar", "7z", "exe",
        "dll", "so", "dylib", "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "mp3", "mp4",
        "wav", "avi", "mov", "mkv", "ttf", "otf", "woff", "woff2",
    ];

    if let Some(ext) = filename.rsplit('.').next() {
        BINARY_EXTENSIONS.contains(&ext.to_lowercase().as_str())
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::FileSystemContext;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Test FileSystemContext that returns a configurable list of skills
    /// roots. Mirrors what `GatewayFileSystem` does in production but
    /// without dragging in a real vault.
    struct TestFs {
        roots: Vec<PathBuf>,
    }

    impl FileSystemContext for TestFs {
        fn conversation_dir(&self, _conversation_id: &str) -> Option<PathBuf> {
            None
        }
        fn outputs_dir(&self) -> Option<PathBuf> {
            None
        }
        fn skills_dir(&self) -> Option<PathBuf> {
            self.roots.first().cloned()
        }
        fn skills_dirs(&self) -> Vec<PathBuf> {
            self.roots.clone()
        }
        fn agents_dir(&self) -> Option<PathBuf> {
            None
        }
        fn python_executable(&self) -> Option<PathBuf> {
            None
        }
    }

    fn write_skill(root: &std::path::Path, name: &str, body: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {body}\n---\n\n{body}\n"),
        )
        .unwrap();
    }

    fn make_tool(roots: Vec<PathBuf>) -> LoadSkillTool {
        LoadSkillTool::new(Arc::new(TestFs { roots }))
    }

    #[test]
    fn resolve_finds_skill_in_vault_only() {
        let vault = TempDir::new().unwrap();
        write_skill(vault.path(), "alpha", "vault skill");
        let tool = make_tool(vec![vault.path().to_path_buf()]);

        let dir = tool.resolve_skill_dir("alpha").expect("resolve ok");
        assert_eq!(dir, vault.path().join("alpha"));
    }

    #[test]
    fn resolve_finds_skill_in_agent_root() {
        let vault = TempDir::new().unwrap();
        let agent = TempDir::new().unwrap();
        write_skill(agent.path(), "managed", "agent skill");
        let tool = make_tool(vec![vault.path().to_path_buf(), agent.path().to_path_buf()]);

        let dir = tool.resolve_skill_dir("managed").expect("resolve ok");
        assert_eq!(
            dir,
            agent.path().join("managed"),
            "agent-root skill must be found when vault has nothing matching"
        );
    }

    #[test]
    fn resolve_vault_wins_on_collision() {
        let vault = TempDir::new().unwrap();
        let agent = TempDir::new().unwrap();
        write_skill(vault.path(), "shared", "vault copy");
        write_skill(agent.path(), "shared", "agent copy");
        let tool = make_tool(vec![vault.path().to_path_buf(), agent.path().to_path_buf()]);

        let dir = tool.resolve_skill_dir("shared").expect("resolve ok");
        assert_eq!(
            dir,
            vault.path().join("shared"),
            "vault root takes priority over the agent root"
        );
    }

    #[test]
    fn resolve_supports_legacy_planner_skill_aliases() {
        let vault = TempDir::new().unwrap();
        write_skill(vault.path(), "spec-builder", "first pass planner");
        write_skill(vault.path(), "plan-composer", "second pass planner");
        let tool = make_tool(vec![vault.path().to_path_buf()]);

        let highlevel = tool
            .resolve_skill_dir("planning-highlevel")
            .expect("legacy highlevel alias resolves");
        let decompose = tool
            .resolve_skill_dir("planning-decompose")
            .expect("legacy decompose alias resolves");

        assert_eq!(highlevel, vault.path().join("spec-builder"));
        assert_eq!(decompose, vault.path().join("plan-composer"));
    }

    #[test]
    fn missing_legacy_planner_skill_error_names_recovery_skill() {
        let vault = TempDir::new().unwrap();
        let tool = make_tool(vec![vault.path().to_path_buf()]);

        let err = tool
            .resolve_skill_dir("planning-highlevel")
            .expect_err("missing legacy alias should explain recovery");
        let msg = format!("{err}");

        assert!(msg.contains("spec-builder"), "got: {msg}");
    }

    #[test]
    fn resolve_returns_clear_error_when_missing_in_all_roots() {
        let vault = TempDir::new().unwrap();
        let agent = TempDir::new().unwrap();
        let tool = make_tool(vec![vault.path().to_path_buf(), agent.path().to_path_buf()]);

        let err = tool.resolve_skill_dir("ghost").expect_err("must error");
        let msg = format!("{err}");
        assert!(msg.contains("not found"), "got: {msg}");
        // Both roots should be mentioned so the user knows where we looked.
        assert!(
            msg.contains(vault.path().to_string_lossy().as_ref())
                && msg.contains(agent.path().to_string_lossy().as_ref()),
            "error must list every searched root, got: {msg}"
        );
    }

    #[test]
    fn resolve_tolerates_missing_root_directory() {
        let vault = TempDir::new().unwrap();
        write_skill(vault.path(), "alpha", "vault skill");
        let nonexistent = vault.path().join("not-a-real-path");
        let tool = make_tool(vec![vault.path().to_path_buf(), nonexistent]);

        let dir = tool.resolve_skill_dir("alpha").expect("resolve ok");
        assert_eq!(dir, vault.path().join("alpha"));
    }

    #[test]
    fn resolve_errors_when_no_roots_configured() {
        let tool = make_tool(vec![]);
        let err = tool.resolve_skill_dir("alpha").expect_err("must error");
        assert!(format!("{err}").contains("not configured"), "got: {err}");
    }

    #[test]
    fn skill_packet_is_bounded_and_handle_based() {
        let instructions = r#"
# Overview
Use this skill for focused Rust work.

## Build
Run cargo check before tests.

## Review
Look for ownership and lifetime issues.
"#;
        let packet = build_skill_packet(
            "rust",
            &json!({"description": "Rust development workflow"}),
            instructions,
            vec![json!({"file": "REFERENCE.md", "load_with": "load_skill(file=\"REFERENCE.md\")"})],
        );

        assert_eq!(packet["name"], "rust");
        assert!(packet.get("instructions").is_none());
        assert_eq!(packet["packet"]["summary"], "Rust development workflow");
        assert_eq!(
            packet["packet"]["resource_uri"],
            "zbot://skills/rust/SKILL.md"
        );
        assert!(packet["packet"]["token_estimate"].as_u64().unwrap() > 0);
        let sections = packet["packet"]["sections"].as_array().unwrap();
        assert!(
            sections
                .iter()
                .any(|section| section["resource_uri"] == "zbot://skills/rust/sections/build")
        );
    }

    #[test]
    fn skill_packet_truncates_large_sections() {
        let body = format!("# Big\n{}", "a".repeat(MAX_SKILL_SECTION_CHARS + 200));
        let packet = build_skill_packet("big", &json!({}), &body, vec![]);
        let summary = packet["packet"]["sections"][0]["summary"].as_str().unwrap();

        assert!(summary.len() <= MAX_SKILL_SECTION_CHARS + 3);
        assert!(summary.ends_with("..."));
        assert!(packet.get("instructions").is_none());
    }
}
