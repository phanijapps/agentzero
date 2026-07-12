//! # Gateway Templates
//!
//! System prompt assembly for AgentZero agents.
//!
//! Assembly order:
//! 1. `config/agent/SOUL.md` — agent identity/personality (created from starter if missing)
//! 2. `config/agent/INSTRUCTIONS.md` — execution rules (created from starter if missing)
//! 3. `config/agent/OS.md` — platform-specific commands (auto-generated for current OS if missing)
//! 4. Prompts — `config/agent-prompts/{name}.md` overrides embedded defaults; extra files included too

use gateway_services::VaultPaths;
use rust_embed::RustEmbed;
use std::path::Path;
use std::sync::Arc;

/// Embedded template files.
#[derive(RustEmbed)]
#[folder = "../templates/"]
pub struct Templates;

/// Required prompts — canonical filename, then embedded template stem.
const REQUIRED_PROMPTS: &[(&str, &str)] = &[
    ("first-turn-protocol", "first_turn_protocol"),
    ("tooling-skills", "tooling_skills"),
    ("memory-learning", "memory_learning"),
    ("planning-autonomy", "planning_autonomy"),
];

// =========================================================================
// Public API
// =========================================================================

/// Load system prompt using VaultPaths.
///
/// Assembly: SOUL.md + INSTRUCTIONS.md + OS.md + shards
pub fn load_system_prompt_from_paths(paths: &Arc<VaultPaths>) -> String {
    let config_dir = paths.config_dir();
    assemble_prompt(&config_dir, paths.vault_dir())
}

/// Load system prompt (legacy path-based).
pub fn load_system_prompt(data_dir: &Path) -> String {
    let paths = VaultPaths::new(data_dir.to_path_buf());
    let _ = paths.migrate_legacy_layout();
    assemble_prompt(&paths.config_dir(), data_dir)
}

/// Load a lean system prompt for fast chat mode using VaultPaths.
///
/// Assembly: SOUL.md + chat-instructions.md + OS.md + chat-protocol + tooling-skills
pub fn load_chat_prompt_from_paths(paths: &Arc<VaultPaths>) -> String {
    let config_dir = paths.config_dir();
    assemble_chat_prompt(&config_dir, paths.vault_dir())
}

/// Get the embedded default system prompt (fallback).
pub fn default_system_prompt() -> String {
    Templates::get("system_prompt.md")
        .map(|file| String::from_utf8_lossy(&file.data).to_string())
        .unwrap_or_else(|| "You are a helpful AI assistant.".to_string())
}

// =========================================================================
// Assembly
// =========================================================================

/// Assemble a lean system prompt for fast chat mode.
///
/// Includes only: SOUL.md + chat-instructions.md + OS.md + chat-protocol + tooling-skills.
/// Skips: INSTRUCTIONS.md, first-turn-protocol, memory-learning, planning-autonomy prompts.
fn assemble_chat_prompt(config_dir: &Path, vault_dir: &Path) -> String {
    let agent_dir = config_dir.join("agent");
    let prompts_dir = config_dir.join("agent-prompts");
    std::fs::create_dir_all(&agent_dir).ok();

    let mut parts: Vec<String> = Vec::new();

    // 1. SOUL.md — same identity
    let soul = load_or_create_file(&agent_dir.join("SOUL.md"), "soul_starter.md");
    if !soul.is_empty() {
        parts.push(soul);
    }

    // 2. Chat-specific instructions (instead of full INSTRUCTIONS.md)
    let chat_instructions = load_or_create_file(
        &agent_dir.join("chat-instructions.md"),
        "chat_instructions.md",
    );
    if !chat_instructions.is_empty() {
        parts.push(chat_instructions);
    }

    // 3. OS.md — platform-specific commands
    let os_md = load_or_create_os(&agent_dir.join("OS.md"));
    if !os_md.is_empty() {
        parts.push(os_md);
    }

    // 4. Minimal prompts: chat-protocol + tooling-skills only
    std::fs::create_dir_all(&prompts_dir).ok();
    for (filename, embedded_name) in &[
        ("chat-protocol", "chat_protocol"),
        ("tooling-skills", "tooling_skills"),
    ] {
        let user_path = prompts_dir.join(format!("{filename}.md"));
        if user_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_path) {
                if !content.trim().is_empty() {
                    parts.push(content);
                }
            }
        } else if let Some(embedded) = Templates::get(&format!("shards/{embedded_name}.md")) {
            let content = String::from_utf8_lossy(&embedded.data).to_string();
            let _ = std::fs::write(&user_path, &content);
            parts.push(content);
        }
    }

    // 5. Runtime environment info
    parts.push(runtime_info(vault_dir));

    let result = parts.join("\n\n");

    if result.trim().is_empty() {
        tracing::warn!("Fast chat prompt is empty, using embedded default");
        return default_system_prompt();
    }

    tracing::info!(
        chars = result.len(),
        "Assembled fast chat prompt from config"
    );
    result
}

/// Assemble the full system prompt from config files and shards.
fn assemble_prompt(config_dir: &Path, vault_dir: &Path) -> String {
    let agent_dir = config_dir.join("agent");
    let prompts_dir = config_dir.join("agent-prompts");
    std::fs::create_dir_all(&agent_dir).ok();

    let mut parts: Vec<String> = Vec::new();

    // 1. SOUL.md — identity/personality
    let soul = load_or_create_file(&agent_dir.join("SOUL.md"), "soul_starter.md");
    if !soul.is_empty() {
        parts.push(soul);
    }

    // 2. INSTRUCTIONS.md — execution rules
    let instructions = load_or_create_file(
        &agent_dir.join("INSTRUCTIONS.md"),
        "instructions_starter.md",
    );
    if !instructions.is_empty() {
        parts.push(instructions);
    }

    // 3. OS.md — platform-specific commands
    let os_md = load_or_create_os(&agent_dir.join("OS.md"));
    if !os_md.is_empty() {
        parts.push(os_md);
    }

    // 4. Prompts — config/agent-prompts/ overrides embedded defaults.
    let prompts = load_prompts(&prompts_dir);
    if !prompts.is_empty() {
        parts.push("# --- SYSTEM PROMPTS ---".to_string());
        parts.push(prompts);
    }

    // 5. Runtime environment info
    parts.push(runtime_info(vault_dir));

    let result = parts.join("\n\n");

    if result.trim().is_empty() {
        tracing::warn!("Assembled prompt is empty, using embedded default");
        return default_system_prompt();
    }

    tracing::info!(chars = result.len(), "Assembled system prompt from config");
    result
}

/// Load a canonical agent file, creating it from an embedded starter if missing.
fn load_or_create_file(path: &Path, starter_name: &str) -> String {
    if !path.exists() {
        // Create from embedded starter
        if let Some(starter) = Templates::get(starter_name) {
            let content = String::from_utf8_lossy(&starter.data).to_string();
            if let Err(e) = std::fs::write(path, &content) {
                tracing::warn!(path = %path.display(), "Failed to create starter file: {}", e);
            } else {
                tracing::info!(path = %path.display(), "Created starter file from {}", starter_name);
            }
            return content;
        }
        return String::new();
    }

    std::fs::read_to_string(path)
        .map(|c| c.trim().to_string())
        .unwrap_or_default()
}

/// Load or auto-generate OS.md for the current platform.
fn load_or_create_os(path: &Path) -> String {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if path.exists() {
        return std::fs::read_to_string(path)
            .map(|c| c.trim().to_string())
            .unwrap_or_default();
    }

    // Auto-generate for current platform
    let template_name = match std::env::consts::OS {
        "windows" => "os_windows.md",
        "macos" => "os_macos.md",
        "linux" => "os_linux.md",
        _ => "os_linux.md", // default to Linux
    };

    if let Some(template) = Templates::get(template_name) {
        let content = String::from_utf8_lossy(&template.data).to_string();
        if let Err(e) = std::fs::write(path, &content) {
            tracing::warn!("Failed to create OS.md: {}", e);
        } else {
            tracing::info!("Auto-generated OS.md for {}", std::env::consts::OS);
        }
        content
    } else {
        String::new()
    }
}

/// Load prompts: config/agent-prompts/ overrides embedded, extra user files included.
fn load_prompts(user_prompts_dir: &Path) -> String {
    std::fs::create_dir_all(user_prompts_dir).ok();

    let mut loaded: Vec<String> = Vec::new();
    let mut loaded_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Load required prompts (user override > embedded default)
    for (filename, embedded_name) in REQUIRED_PROMPTS {
        let user_path = user_prompts_dir.join(format!("{filename}.md"));
        let content = if user_path.exists() {
            tracing::debug!("Loading prompt '{}' from user config", filename);
            std::fs::read_to_string(&user_path).ok()
        } else {
            // Write embedded default to disk so user can see and customize it
            let embedded_path = format!("shards/{embedded_name}.md");
            let embedded = Templates::get(&embedded_path)
                .map(|file| String::from_utf8_lossy(&file.data).to_string());
            if let Some(ref content) = embedded {
                if let Err(e) = std::fs::write(&user_path, content) {
                    tracing::debug!("Failed to write default prompt {}: {}", filename, e);
                } else {
                    tracing::info!(
                        "Created default prompt: config/agent-prompts/{}.md",
                        filename
                    );
                }
            }
            embedded
        };

        if let Some(c) = content {
            if !c.trim().is_empty() {
                loaded.push(c);
            }
        }
        loaded_names.insert(filename.to_string());
    }

    // Scan for extra user prompts (any .md not in REQUIRED_PROMPTS)
    if let Ok(entries) = std::fs::read_dir(user_prompts_dir) {
        let mut extras: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.ends_with(".md") && !loaded_names.contains(name.trim_end_matches(".md"))
            })
            .collect();
        extras.sort_by_key(|e| e.file_name());

        for entry in extras {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if !content.trim().is_empty() {
                    tracing::info!("Loading extra prompt: {:?}", entry.file_name());
                    loaded.push(content);
                }
            }
        }
    }

    loaded.join("\n\n")
}

/// Minimal runtime info (vault path, venv status).
fn runtime_info(vault_dir: &Path) -> String {
    let mut lines = vec![format!("VAULT: {}", vault_dir.display())];

    let venv_dir = vault_dir.join("venv");
    let python_path = if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    };
    if python_path.exists() {
        lines.push(format!("PYTHON VENV: {} (ready)", venv_dir.display()));
    }

    lines.join("\n")
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_system_prompt_contains_expected_content() {
        let prompt = default_system_prompt();
        assert!(prompt.contains("Jaffa"));
        assert!(prompt.contains("CORE IDENTITY"));
    }

    #[test]
    fn test_assemble_creates_missing_config_files() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");

        let prompt = assemble_prompt(&config_dir, dir.path());

        let agent_dir = config_dir.join("agent");
        // Should have created the exact-case agent contracts and canonical prompts.
        assert!(agent_dir.join("SOUL.md").exists());
        assert!(agent_dir.join("INSTRUCTIONS.md").exists());
        assert!(agent_dir.join("OS.md").exists());
        assert!(config_dir.join("agent-prompts").is_dir());

        // Prompt should contain content from all three
        assert!(prompt.contains("Jaffa")); // from SOUL
        assert!(prompt.contains("execution_mode")); // from INSTRUCTIONS
        assert!(prompt.contains("PLATFORM")); // from OS
        assert!(prompt.contains("SYSTEM PROMPTS")); // separator
        assert!(prompt.contains("MEMORY & LEARNING")); // from prompt
    }

    #[test]
    fn test_user_override_prompt() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        let prompts_dir = config_dir.join("agent-prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // User overrides memory-learning prompt
        std::fs::write(
            prompts_dir.join("memory-learning.md"),
            "CUSTOM MEMORY RULES\nMy custom memory prompt.",
        )
        .unwrap();

        let prompt = assemble_prompt(&config_dir, dir.path());

        // Should contain the custom prompt, not the embedded default
        assert!(prompt.contains("CUSTOM MEMORY RULES"));
        assert!(!prompt.contains("Ward Memory")); // embedded default content
    }

    #[test]
    fn test_extra_user_prompt_included() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        let prompts_dir = config_dir.join("agent-prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // User adds a custom prompt
        std::fs::write(
            prompts_dir.join("my-rules.md"),
            "MY CUSTOM RULES\nAlways use TypeScript.",
        )
        .unwrap();

        let prompt = assemble_prompt(&config_dir, dir.path());

        assert!(prompt.contains("MY CUSTOM RULES"));
        assert!(prompt.contains("Always use TypeScript"));
    }

    #[test]
    fn test_os_md_auto_generated_for_platform() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");

        let os_path = config_dir.join("agent").join("OS.md");
        let os_content = load_or_create_os(&os_path);

        assert!(os_content.contains("PLATFORM"));
        assert!(os_path.exists());

        // Should match current OS
        if cfg!(windows) {
            assert!(os_content.contains("PowerShell"));
        } else if cfg!(target_os = "macos") {
            assert!(os_content.contains("zsh"));
        } else {
            assert!(os_content.contains("bash"));
        }
    }

    #[test]
    fn test_existing_config_not_overwritten() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        let agent_dir = config_dir.join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();

        std::fs::write(agent_dir.join("SOUL.md"), "I am a custom soul.").unwrap();

        let prompt = assemble_prompt(&config_dir, dir.path());

        assert!(prompt.contains("I am a custom soul."));
        assert!(!prompt.contains("Jaffa")); // starter content NOT injected
    }

    #[test]
    fn test_load_system_prompt_legacy() {
        let dir = TempDir::new().unwrap();
        let prompt = load_system_prompt(dir.path());
        assert!(!prompt.trim().is_empty());
        assert!(prompt.contains("Jaffa"));
    }

    #[test]
    fn test_load_prompt_fallback_to_embedded() {
        let dir = TempDir::new().unwrap();
        let prompts_dir = dir.path().join("config").join("agent-prompts");

        let prompts = load_prompts(&prompts_dir);

        // Should load all required prompts from embedded.
        assert!(prompts.contains("TOOLING & SKILLS"));
        assert!(prompts.contains("MEMORY & LEARNING"));
        assert!(prompts.contains("delegation_rules")); // from planning-autonomy prompt
    }
}
