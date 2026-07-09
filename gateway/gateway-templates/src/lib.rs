//! # Gateway Templates
//!
//! System prompt assembly for AgentZero agents.
//!
//! Assembly order:
//! 1. `config/SOUL.md` — agent identity/personality (created from starter if missing)
//! 2. `config/INSTRUCTIONS.md` — execution rules (created from starter if missing)
//! 3. `config/OS.md` — platform-specific commands (auto-generated for current OS if missing)
//! 4. Shards — `config/shards/{name}.md` overrides embedded defaults; extra files included too

use gateway_services::VaultPaths;
use rust_embed::RustEmbed;
use std::path::Path;
use std::sync::Arc;

/// Embedded template files.
#[derive(RustEmbed)]
#[folder = "../templates/"]
pub struct Templates;

/// Required shards — loaded from config/shards/ if present, otherwise from embedded defaults.
const REQUIRED_SHARDS: &[&str] = &[
    "first_turn_protocol",
    "tooling_skills",
    "memory_learning",
    "planning_autonomy",
];

// =========================================================================
// Public API
// =========================================================================

/// Load system prompt using VaultPaths.
///
/// Assembly: SOUL.md + INSTRUCTIONS.md + OS.md + shards
pub fn load_system_prompt_from_paths(paths: &Arc<VaultPaths>) -> String {
    let config_dir = paths.vault_dir().join("config");
    assemble_prompt(&config_dir, paths.vault_dir())
}

/// Load system prompt (legacy path-based).
pub fn load_system_prompt(data_dir: &Path) -> String {
    let config_dir = data_dir.join("config");
    assemble_prompt(&config_dir, data_dir)
}

/// Load a lean system prompt for fast chat mode using VaultPaths.
///
/// Assembly: SOUL.md + chat_instructions.md + OS.md + chat_protocol + tooling_skills
pub fn load_chat_prompt_from_paths(paths: &Arc<VaultPaths>) -> String {
    let config_dir = paths.vault_dir().join("config");
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
/// Includes only: SOUL.md + chat_instructions.md + OS.md + chat_protocol + tooling_skills.
/// Skips: INSTRUCTIONS.md, first_turn_protocol, memory_learning, planning_autonomy shards.
fn assemble_chat_prompt(config_dir: &Path, vault_dir: &Path) -> String {
    std::fs::create_dir_all(config_dir).ok();

    let mut parts: Vec<String> = Vec::new();

    // 1. SOUL.md — same identity
    let soul = load_or_create_config(config_dir, "SOUL.md", "soul_starter.md");
    if !soul.is_empty() {
        parts.push(soul);
    }

    // 2. Chat-specific instructions (instead of full INSTRUCTIONS.md)
    let chat_instructions =
        load_or_create_config(config_dir, "chat_instructions.md", "chat_instructions.md");
    if !chat_instructions.is_empty() {
        parts.push(chat_instructions);
    }

    // 3. OS.md — platform-specific commands
    let os_md = load_or_create_os(config_dir);
    if !os_md.is_empty() {
        parts.push(os_md);
    }

    // 4. Minimal shards: chat_protocol + tooling_skills only
    let shards_dir = config_dir.join("shards");
    std::fs::create_dir_all(&shards_dir).ok();
    for name in &["chat_protocol", "tooling_skills"] {
        let user_path = shards_dir.join(format!("{}.md", name));
        if user_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_path) {
                if !content.trim().is_empty() {
                    parts.push(content);
                }
            }
        } else if let Some(embedded) = Templates::get(&format!("shards/{}.md", name)) {
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
    std::fs::create_dir_all(config_dir).ok();

    let mut parts: Vec<String> = Vec::new();

    // 1. SOUL.md — identity/personality
    let soul = load_or_create_config(config_dir, "SOUL.md", "soul_starter.md");
    if !soul.is_empty() {
        parts.push(soul);
    }

    // 2. INSTRUCTIONS.md — execution rules
    let instructions =
        load_or_create_config(config_dir, "INSTRUCTIONS.md", "instructions_starter.md");
    if !instructions.is_empty() {
        parts.push(instructions);
    }

    // 3. OS.md — platform-specific commands
    let os_md = load_or_create_os(config_dir);
    if !os_md.is_empty() {
        parts.push(os_md);
    }

    // 4. Shards — config/shards/ overrides embedded, plus extra user shards
    let shards = load_shards(config_dir);
    if !shards.is_empty() {
        parts.push("# --- SYSTEM SHARDS ---".to_string());
        parts.push(shards);
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

/// Load a config file, creating from embedded starter if missing.
fn load_or_create_config(config_dir: &Path, filename: &str, starter_name: &str) -> String {
    let path = config_dir.join(filename);

    if !path.exists() {
        // Create from embedded starter
        if let Some(starter) = Templates::get(starter_name) {
            let content = String::from_utf8_lossy(&starter.data).to_string();
            if let Err(e) = std::fs::write(&path, &content) {
                tracing::warn!("Failed to create {}: {}", filename, e);
            } else {
                tracing::info!("Created {} from {}", filename, starter_name);
            }
            return content;
        }
        return String::new();
    }

    std::fs::read_to_string(&path)
        .map(|c| c.trim().to_string())
        .unwrap_or_default()
}

/// Load or auto-generate OS.md for the current platform.
fn load_or_create_os(config_dir: &Path) -> String {
    std::fs::create_dir_all(config_dir).ok();
    let path = config_dir.join("OS.md");

    if path.exists() {
        return std::fs::read_to_string(&path)
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
        if let Err(e) = std::fs::write(&path, &content) {
            tracing::warn!("Failed to create OS.md: {}", e);
        } else {
            tracing::info!("Auto-generated OS.md for {}", std::env::consts::OS);
        }
        content
    } else {
        String::new()
    }
}

/// Load shards: config/shards/ overrides embedded, extra user files included.
fn load_shards(config_dir: &Path) -> String {
    let user_shards_dir = config_dir.join("shards");
    std::fs::create_dir_all(&user_shards_dir).ok();

    let mut loaded: Vec<String> = Vec::new();
    let mut loaded_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Load required shards (user override > embedded default)
    for name in REQUIRED_SHARDS {
        let user_path = user_shards_dir.join(format!("{}.md", name));
        let content = if user_path.exists() {
            tracing::debug!("Loading shard '{}' from user config", name);
            std::fs::read_to_string(&user_path).ok()
        } else {
            // Write embedded default to disk so user can see and customize it
            let embedded_path = format!("shards/{}.md", name);
            let embedded = Templates::get(&embedded_path)
                .map(|file| String::from_utf8_lossy(&file.data).to_string());
            if let Some(ref content) = embedded {
                if let Err(e) = std::fs::write(&user_path, content) {
                    tracing::debug!("Failed to write default shard {}: {}", name, e);
                } else {
                    tracing::info!("Created default shard: config/shards/{}.md", name);
                }
            }
            embedded
        };

        if let Some(c) = content {
            if !c.trim().is_empty() {
                loaded.push(c);
            }
        }
        loaded_names.insert(name.to_string());
    }

    // Scan for extra user shards (any .md not in REQUIRED_SHARDS)
    if let Ok(entries) = std::fs::read_dir(&user_shards_dir) {
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
                    tracing::info!("Loading extra shard: {:?}", entry.file_name());
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

        // Should have created SOUL.md, INSTRUCTIONS.md, OS.md
        assert!(config_dir.join("SOUL.md").exists());
        assert!(config_dir.join("INSTRUCTIONS.md").exists());
        assert!(config_dir.join("OS.md").exists());
        assert!(config_dir.join("shards").is_dir());

        // Prompt should contain content from all three
        assert!(prompt.contains("Jaffa")); // from SOUL
        assert!(prompt.contains("execution_mode")); // from INSTRUCTIONS
        assert!(prompt.contains("PLATFORM")); // from OS
        assert!(prompt.contains("SYSTEM SHARDS")); // separator
        assert!(prompt.contains("MEMORY & LEARNING")); // from shard
    }

    #[test]
    fn test_user_override_shard() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        let shards_dir = config_dir.join("shards");
        std::fs::create_dir_all(&shards_dir).unwrap();

        // User overrides memory_learning shard
        std::fs::write(
            shards_dir.join("memory_learning.md"),
            "CUSTOM MEMORY RULES\nMy custom memory shard.",
        )
        .unwrap();

        let prompt = assemble_prompt(&config_dir, dir.path());

        // Should contain the custom shard, not the embedded default
        assert!(prompt.contains("CUSTOM MEMORY RULES"));
        assert!(!prompt.contains("Ward Memory")); // embedded default content
    }

    #[test]
    fn test_extra_user_shard_included() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        let shards_dir = config_dir.join("shards");
        std::fs::create_dir_all(&shards_dir).unwrap();

        // User adds a custom shard
        std::fs::write(
            shards_dir.join("my_rules.md"),
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

        let os_content = load_or_create_os(&config_dir);

        assert!(os_content.contains("PLATFORM"));
        assert!(config_dir.join("OS.md").exists());

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
        std::fs::create_dir_all(&config_dir).unwrap();

        std::fs::write(config_dir.join("SOUL.md"), "I am a custom soul.").unwrap();

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
    fn test_load_shard_fallback_to_embedded() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");

        let shards = load_shards(&config_dir);

        // Should load all required shards from embedded
        assert!(shards.contains("TOOLING & SKILLS"));
        assert!(shards.contains("MEMORY & LEARNING"));
        assert!(shards.contains("delegation_rules")); // from planning_autonomy shard
    }
}
