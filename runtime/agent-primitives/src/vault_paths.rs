//! # Vault Paths
//!
//! Centralized path management for the z-Bot vault.
//!
//! Provides XDG-inspired directory structure:
//! - Config files go in `config/` subdirectory
//! - Data files go in `data/` subdirectory
//! - Agent/session data goes in `wards/` subdirectory (each agent/session is a ward)
//! - Other directories (logs, agents, skills) remain at root level

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::WardArchetypeId;

const LEGACY_MCP_SERVERS_FILE: &str = "mcps.json";
const LEGACY_SCHEDULES_FILE: &str = "cron_jobs.json";
const LEGACY_MCP_OAUTH_PENDING_FILE: &str = "mcp_oauth_pending.json";
const LEGACY_MCP_OAUTH_TOKENS_FILE: &str = "mcp_oauth_tokens.json";
const LEGACY_DISTILLATION_PROMPT_FILE: &str = "distillation_prompt.md";
const LEGACY_INTENT_ANALYSIS_PROMPT_FILE: &str = "intent_analysis_prompt.md";
const LEGACY_RECALL_CONFIG_FILE: &str = "recall_config.json";
const LEGACY_SEEDED_DEFAULTS_FILE: &str = "seeded_defaults.json";
const RETIRED_OKF_PROMPT: &str = "okf_tooling.md";

/// Result of applying the non-destructive vault layout migration.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LayoutMigration {
    /// Legacy files copied into a previously absent canonical path.
    pub copied: Vec<PathBuf>,
    /// Both paths existed, so the canonical file was retained unchanged.
    pub conflicts: Vec<(PathBuf, PathBuf)>,
}

/// Centralized path management for the z-Bot vault.
///
/// Provides a consistent interface for accessing all vault paths,
/// following XDG-inspired conventions:
/// - Config files → `config/` subdirectory
/// - Data files → `data/` subdirectory
/// - Agent/session data → `wards/{id}/` subdirectory
#[derive(Debug, Clone)]
pub struct VaultPaths {
    /// Base vault directory (e.g., ~/Documents/zbot)
    vault_dir: PathBuf,
}

impl VaultPaths {
    /// Create a new VaultPaths instance.
    pub fn new(vault_dir: PathBuf) -> Self {
        Self { vault_dir }
    }

    // =========================================================================
    // Config paths (config/ subdirectory)
    // =========================================================================

    /// Path to `config/settings.json`
    pub fn settings(&self) -> PathBuf {
        self.vault_dir.join("config").join("settings.json")
    }

    /// Path to `config/providers.json`
    pub fn providers(&self) -> PathBuf {
        self.vault_dir.join("config").join("providers.json")
    }

    /// Path to `config/mcp-servers.json`.
    pub fn mcps(&self) -> PathBuf {
        self.config_dir().join("mcp-servers.json")
    }

    /// Path to `config/connectors.json`
    pub fn connectors(&self) -> PathBuf {
        self.vault_dir.join("config").join("connectors.json")
    }

    /// Path to `config/schedules.json`.
    pub fn cron_jobs(&self) -> PathBuf {
        self.config_dir().join("schedules.json")
    }

    /// Path to `config/agent/INSTRUCTIONS.md`.
    pub fn instructions(&self) -> PathBuf {
        self.agent_contracts_dir().join("INSTRUCTIONS.md")
    }

    /// Path to `config/agent/SOUL.md`.
    pub fn soul(&self) -> PathBuf {
        self.agent_contracts_dir().join("SOUL.md")
    }

    /// Path to `config/agent/OS.md`.
    pub fn os(&self) -> PathBuf {
        self.agent_contracts_dir().join("OS.md")
    }

    /// Path to `config/agent/chat-instructions.md`.
    pub fn chat_instructions(&self) -> PathBuf {
        self.agent_contracts_dir().join("chat-instructions.md")
    }

    /// Path to `config/agent-prompts/`.
    pub fn agent_prompts_dir(&self) -> PathBuf {
        self.config_dir().join("agent-prompts")
    }

    /// Directory containing user-editable templates seeded by z-Bot.
    pub fn templates_dir(&self) -> PathBuf {
        self.config_dir().join("templates")
    }

    /// Canonical user-editable Ward Layout Contract template.
    pub fn ward_layout_template(&self) -> PathBuf {
        self.templates_dir().join("ward-conf.yaml")
    }

    /// Canonical user-editable template used to scaffold Ward `AGENTS.md`.
    pub fn ward_agent_template(&self) -> PathBuf {
        self.templates_dir().join("ward-agent.md")
    }

    /// Canonical directory containing complete Ward Layout archetype bundles.
    pub fn ward_archetype_registry_dir(&self) -> PathBuf {
        self.templates_dir().join("wards")
    }

    /// Canonical bundle directory for one closed Ward Layout archetype.
    pub fn ward_archetype_bundle(&self, archetype: WardArchetypeId) -> PathBuf {
        self.ward_archetype_registry_dir().join(archetype.as_str())
    }

    /// Path to `config/auth/mcp/tokens.json`.
    pub fn mcp_oauth_tokens(&self) -> PathBuf {
        self.config_dir()
            .join("auth")
            .join("mcp")
            .join("tokens.json")
    }

    /// Path to `config/auth/mcp/pending.json`.
    pub fn mcp_oauth_pending(&self) -> PathBuf {
        self.config_dir()
            .join("auth")
            .join("mcp")
            .join("pending.json")
    }

    /// Path to `config/distillation-prompt.md`.
    pub fn distillation_prompt(&self) -> PathBuf {
        self.config_dir().join("distillation-prompt.md")
    }

    /// Path to `config/intent-analysis-prompt.md`.
    pub fn intent_analysis_prompt(&self) -> PathBuf {
        self.config_dir().join("intent-analysis-prompt.md")
    }

    /// Path to `config/recall-config.json`.
    pub fn recall_config(&self) -> PathBuf {
        self.config_dir().join("recall-config.json")
    }

    /// Path to `config/seeded-defaults.json`.
    pub fn seeded_defaults(&self) -> PathBuf {
        self.config_dir().join("seeded-defaults.json")
    }

    /// Path to the config directory
    pub fn config_dir(&self) -> PathBuf {
        self.vault_dir.join("config")
    }

    /// Directory containing the exact-case root agent contracts.
    pub fn agent_contracts_dir(&self) -> PathBuf {
        self.config_dir().join("agent")
    }

    // =========================================================================
    // Data paths (data/ subdirectory)
    // =========================================================================

    /// Path to `data/conversations.db`
    pub fn conversations_db(&self) -> PathBuf {
        self.vault_dir.join("data").join("conversations.db")
    }

    /// Path to `data/knowledge.db` — long-term memory + graph + vec0 indexes.
    pub fn knowledge_db(&self) -> PathBuf {
        self.vault_dir.join("data").join("knowledge.db")
    }

    /// Path to the data directory
    pub fn data_dir(&self) -> PathBuf {
        self.vault_dir.join("data")
    }

    /// Path to `data/traces` — per-session `.jsonl.gz` execution trace files.
    pub fn traces_dir(&self) -> PathBuf {
        self.vault_dir.join("data").join("traces")
    }

    // =========================================================================
    // Root-level directories
    // =========================================================================

    /// Path to logs directory
    pub fn logs_dir(&self) -> PathBuf {
        self.vault_dir.join("logs")
    }

    /// Path to agents directory (agent definitions)
    pub fn agents_dir(&self) -> PathBuf {
        self.vault_dir.join("agents")
    }

    /// Path to the vault-owned skills directory.
    pub fn skills_dir(&self) -> PathBuf {
        self.vault_dir.join("skills")
    }

    /// Path to the system-wide agent-installed skills directory
    /// (`$HOME/.agents/skills`). Online services and CLI installers drop
    /// skills here. Not tied to any single vault.
    ///
    /// Returns `/.agents/skills` if `$HOME` cannot be resolved — that path
    /// won't exist on a real system, so the indexer treats it as empty.
    pub fn agent_skills_dir() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".agents").join("skills"))
            .unwrap_or_else(|| PathBuf::from("/.agents/skills"))
    }

    /// Skills roots in priority order: vault first (user-owned, mutable),
    /// then `$HOME/.agents/skills` (managed, read-only). When the same
    /// skill name appears in both, the loader keeps the vault copy.
    pub fn skills_dirs(&self) -> Vec<PathBuf> {
        vec![self.skills_dir(), Self::agent_skills_dir()]
    }

    /// Path to wards directory (contains agent data, session data, and scratch ward)
    pub fn wards_dir(&self) -> PathBuf {
        self.vault_dir.join("wards")
    }

    /// Path to plugins directory (contains STDIO plugins)
    pub fn plugins_dir(&self) -> PathBuf {
        self.vault_dir.join("plugins")
    }

    /// Path to the vault-owned temp directory.
    ///
    /// Convention: ephemeral working files, agent scratch output,
    /// download caches — anything safe to wipe periodically. The
    /// `/api/cleanup/vault-temp` endpoint is bounded to this path.
    pub fn temp_dir(&self) -> PathBuf {
        self.vault_dir.join("temp")
    }

    /// Path to a specific ward (also used for agent data and session data).
    /// Returns `wards/{ward_id}/`
    pub fn ward_dir(&self, ward_id: &str) -> PathBuf {
        self.vault_dir.join("wards").join(ward_id)
    }

    /// Independently activated Ward Layout Contract snapshot for one ward.
    pub fn ward_layout_snapshot(&self, ward_id: &str) -> PathBuf {
        self.ward_dir(ward_id).join("ward-conf.yaml")
    }

    /// Protected z-Bot control-plane state for one ward.
    pub fn ward_layout_control_dir(&self, ward_id: &str) -> PathBuf {
        self.ward_dir(ward_id).join(".zbot").join("ward-layout")
    }

    /// Path to `config/wards/` — language config directory for ward indexing
    pub fn ward_lang_configs_dir(&self) -> PathBuf {
        self.vault_dir.join("config").join("wards")
    }

    /// Get the base vault directory
    pub fn vault_dir(&self) -> &PathBuf {
        &self.vault_dir
    }

    // =========================================================================
    // Directory initialization
    // =========================================================================

    /// Ensure the required vault roots exist.
    ///
    /// Creates:
    /// - config/
    /// - data/
    /// - logs/
    /// - wards/
    ///
    /// Optional integrations and runtime directories are created by their
    /// owning service when they are actually used.
    pub fn ensure_dirs_exist(&self) -> io::Result<()> {
        let dirs = [
            self.config_dir(),
            self.data_dir(),
            self.logs_dir(),
            self.wards_dir(),
        ];

        for dir in &dirs {
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
                tracing::debug!("Created directory: {:?}", dir);
            }
        }

        Ok(())
    }

    /// Create an optional directory at the moment its owning service needs it.
    pub fn ensure_optional_dir(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    /// Copy legacy layout files into their canonical locations without deleting
    /// the source. Canonical files always win if both paths exist.
    pub fn migrate_legacy_layout(&self) -> io::Result<LayoutMigration> {
        self.ensure_dirs_exist()?;
        let mut migration = LayoutMigration::default();
        let config = self.config_dir();
        let copies = [
            (config.join(LEGACY_MCP_SERVERS_FILE), self.mcps()),
            (config.join(LEGACY_SCHEDULES_FILE), self.cron_jobs()),
            (
                config.join(LEGACY_MCP_OAUTH_PENDING_FILE),
                self.mcp_oauth_pending(),
            ),
            (
                config.join(LEGACY_MCP_OAUTH_TOKENS_FILE),
                self.mcp_oauth_tokens(),
            ),
            (
                config.join(LEGACY_DISTILLATION_PROMPT_FILE),
                self.distillation_prompt(),
            ),
            (
                config.join(LEGACY_INTENT_ANALYSIS_PROMPT_FILE),
                self.intent_analysis_prompt(),
            ),
            (config.join(LEGACY_RECALL_CONFIG_FILE), self.recall_config()),
            (
                config.join(LEGACY_SEEDED_DEFAULTS_FILE),
                self.seeded_defaults(),
            ),
            (config.join("SOUL.md"), self.soul()),
            (config.join("INSTRUCTIONS.md"), self.instructions()),
            (config.join("OS.md"), self.os()),
        ];

        for (legacy, canonical) in copies {
            copy_if_canonical_absent(&legacy, &canonical, &mut migration)?;
        }

        self.migrate_legacy_prompts(&mut migration)?;

        Ok(migration)
    }

    fn migrate_legacy_prompts(&self, migration: &mut LayoutMigration) -> io::Result<()> {
        let legacy_prompts = self.config_dir().join("shards");
        if !legacy_prompts.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(legacy_prompts)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if filename == RETIRED_OKF_PROMPT {
                continue;
            }
            let Some(canonical_name) = canonical_prompt_filename(&filename) else {
                tracing::warn!(
                    legacy = %entry.path().display(),
                    "skipping legacy prompt with a non-canonical filename"
                );
                continue;
            };
            copy_if_canonical_absent(
                &entry.path(),
                &self.agent_prompts_dir().join(canonical_name),
                migration,
            )?;
        }
        Ok(())
    }
}

fn canonical_prompt_filename(filename: &str) -> Option<String> {
    let stem = filename.strip_suffix(".md")?;
    if stem.is_empty()
        || !stem.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        return None;
    }
    let canonical = stem.replace('_', "-");
    (!canonical.starts_with('-') && !canonical.ends_with('-')).then(|| format!("{canonical}.md"))
}

fn copy_if_canonical_absent(
    legacy: &Path,
    canonical: &Path,
    migration: &mut LayoutMigration,
) -> io::Result<()> {
    if !legacy.exists() {
        return Ok(());
    }
    if canonical.exists() {
        migration
            .conflicts
            .push((legacy.to_path_buf(), canonical.to_path_buf()));
        tracing::warn!(
            legacy = %legacy.display(),
            canonical = %canonical.display(),
            "vault layout conflict: retaining canonical file and legacy rollback copy"
        );
        return Ok(());
    }
    if let Some(parent) = canonical.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(legacy, canonical)?;
    migration.copied.push(canonical.to_path_buf());
    tracing::info!(
        legacy = %legacy.display(),
        canonical = %canonical.display(),
        "copied legacy vault layout file to canonical path"
    );
    Ok(())
}

/// Reference-counted VaultPaths for sharing across services.
pub type SharedVaultPaths = Arc<VaultPaths>;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_config_paths() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        assert_eq!(
            paths.settings(),
            dir.path().join("config").join("settings.json")
        );
        assert_eq!(
            paths.providers(),
            dir.path().join("config").join("providers.json")
        );
        assert_eq!(
            paths.mcps(),
            dir.path().join("config").join("mcp-servers.json")
        );
        assert_eq!(
            paths.connectors(),
            dir.path().join("config").join("connectors.json")
        );
        assert_eq!(
            paths.cron_jobs(),
            dir.path().join("config").join("schedules.json")
        );
        assert_eq!(
            paths.instructions(),
            dir.path()
                .join("config")
                .join("agent")
                .join("INSTRUCTIONS.md")
        );
        assert_eq!(
            paths.distillation_prompt(),
            dir.path().join("config").join("distillation-prompt.md")
        );
        assert_eq!(
            paths.intent_analysis_prompt(),
            dir.path().join("config").join("intent-analysis-prompt.md")
        );
        assert_eq!(
            paths.recall_config(),
            dir.path().join("config").join("recall-config.json")
        );
        assert_eq!(
            paths.seeded_defaults(),
            dir.path().join("config").join("seeded-defaults.json")
        );
        assert_eq!(
            paths.ward_layout_template(),
            dir.path()
                .join("config")
                .join("templates")
                .join("ward-conf.yaml")
        );
        // STUB: AC1 — Ward doctrine has one canonical user-editable template path.
        assert_eq!(
            paths.ward_agent_template(),
            dir.path()
                .join("config")
                .join("templates")
                .join("ward-agent.md")
        );
    }

    // STUB: AC4
    #[test]
    fn ward_archetype_bundle_is_confined() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        let registry = dir.path().join("config").join("templates").join("wards");
        assert_eq!(paths.ward_archetype_registry_dir(), registry);

        for archetype in WardArchetypeId::ALL {
            let bundle = paths.ward_archetype_bundle(archetype);
            assert_eq!(bundle.parent(), Some(registry.as_path()));
            assert_eq!(
                bundle.file_name().and_then(|value| value.to_str()),
                Some(archetype.as_str())
            );
        }
    }

    #[test]
    fn test_data_paths() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        assert_eq!(
            paths.conversations_db(),
            dir.path().join("data").join("conversations.db")
        );
    }

    #[test]
    fn test_directory_paths() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        assert_eq!(paths.logs_dir(), dir.path().join("logs"));
        assert_eq!(paths.agents_dir(), dir.path().join("agents"));
        assert_eq!(paths.skills_dir(), dir.path().join("skills"));
        assert_eq!(paths.wards_dir(), dir.path().join("wards"));
        assert_eq!(paths.plugins_dir(), dir.path().join("plugins"));
    }

    #[test]
    fn test_ward_dir() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        assert_eq!(
            paths.ward_dir("scratch"),
            dir.path().join("wards").join("scratch")
        );
        assert_eq!(
            paths.ward_dir("root"),
            dir.path().join("wards").join("root")
        );
        assert_eq!(
            paths.ward_layout_snapshot("root"),
            dir.path().join("wards").join("root").join("ward-conf.yaml")
        );
        assert_eq!(
            paths.ward_layout_control_dir("root"),
            dir.path()
                .join("wards")
                .join("root")
                .join(".zbot")
                .join("ward-layout")
        );
    }

    #[test]
    fn test_skills_dirs_returns_two_in_priority_order() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        let roots = paths.skills_dirs();
        assert_eq!(roots.len(), 2, "expected vault + agent dir");
        assert_eq!(roots[0], paths.skills_dir(), "vault must come first");
        assert_eq!(
            roots[1],
            VaultPaths::agent_skills_dir(),
            "agent skills dir must come second"
        );
    }

    #[test]
    fn test_agent_skills_dir_is_under_home() {
        // Either we get $HOME/.agents/skills, or — in a sandbox without HOME —
        // the documented fallback. Both end in `.agents/skills` so the
        // contract is stable for callers.
        let p = VaultPaths::agent_skills_dir();
        let s = p.to_string_lossy();
        assert!(
            s.ends_with(".agents/skills"),
            "agent_skills_dir must end with .agents/skills, got {}",
            s
        );
    }

    #[test]
    fn ensure_dirs_creates_required_roots_but_not_optional_roots() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        paths.ensure_dirs_exist().unwrap();

        assert!(dir.path().join("config").exists());
        assert!(dir.path().join("data").exists());
        assert!(dir.path().join("logs").exists());
        assert!(dir.path().join("wards").exists());
        assert!(!dir.path().join("agents").exists());
        assert!(!dir.path().join("skills").exists());
        assert!(!dir.path().join("plugins").exists());
        assert!(!dir.path().join("temp").exists());
        assert!(!dir.path().join("config").join("wards").exists());
        assert!(!dir.path().join("wards").join("scratch").exists());
    }

    #[test]
    fn legacy_configuration_is_copied_to_canonical_paths_without_deleting_sources() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join(LEGACY_MCP_SERVERS_FILE), "[]").unwrap();
        std::fs::write(config.join(LEGACY_SCHEDULES_FILE), "[]").unwrap();
        std::fs::write(config.join(LEGACY_MCP_OAUTH_PENDING_FILE), "{}").unwrap();
        std::fs::write(config.join(LEGACY_MCP_OAUTH_TOKENS_FILE), "{}").unwrap();
        std::fs::write(config.join(LEGACY_DISTILLATION_PROMPT_FILE), "distill").unwrap();
        std::fs::write(config.join(LEGACY_INTENT_ANALYSIS_PROMPT_FILE), "intent").unwrap();
        std::fs::write(config.join(LEGACY_RECALL_CONFIG_FILE), "{\"max_facts\": 5}").unwrap();
        std::fs::write(config.join(LEGACY_SEEDED_DEFAULTS_FILE), "{}").unwrap();
        std::fs::write(config.join("SOUL.md"), "legacy soul").unwrap();
        std::fs::write(config.join("INSTRUCTIONS.md"), "legacy instructions").unwrap();
        std::fs::write(config.join("OS.md"), "legacy os").unwrap();
        std::fs::create_dir_all(config.join("shards")).unwrap();
        std::fs::write(
            config.join("shards").join("memory_learning.md"),
            "legacy prompt",
        )
        .unwrap();
        std::fs::write(
            config.join("shards").join("custom_rules.md"),
            "custom prompt",
        )
        .unwrap();
        std::fs::write(config.join("shards").join(RETIRED_OKF_PROMPT), "retired").unwrap();

        let migration = paths.migrate_legacy_layout().unwrap();

        assert_eq!(std::fs::read_to_string(paths.mcps()).unwrap(), "[]");
        assert_eq!(std::fs::read_to_string(paths.cron_jobs()).unwrap(), "[]");
        assert_eq!(
            std::fs::read_to_string(paths.mcp_oauth_pending()).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(paths.mcp_oauth_tokens()).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(paths.distillation_prompt()).unwrap(),
            "distill"
        );
        assert_eq!(
            std::fs::read_to_string(paths.intent_analysis_prompt()).unwrap(),
            "intent"
        );
        assert_eq!(
            std::fs::read_to_string(paths.recall_config()).unwrap(),
            "{\"max_facts\": 5}"
        );
        assert_eq!(
            std::fs::read_to_string(paths.seeded_defaults()).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(paths.soul()).unwrap(),
            "legacy soul"
        );
        assert_eq!(
            std::fs::read_to_string(paths.instructions()).unwrap(),
            "legacy instructions"
        );
        assert_eq!(std::fs::read_to_string(paths.os()).unwrap(), "legacy os");
        assert_eq!(
            std::fs::read_to_string(paths.agent_prompts_dir().join("memory-learning.md")).unwrap(),
            "legacy prompt"
        );
        assert_eq!(
            std::fs::read_to_string(paths.agent_prompts_dir().join("custom-rules.md")).unwrap(),
            "custom prompt"
        );
        assert!(!paths.agent_prompts_dir().join("okf-tooling.md").exists());
        assert!(config.join(LEGACY_MCP_SERVERS_FILE).exists());
        assert!(config.join(LEGACY_SCHEDULES_FILE).exists());
        assert!(config.join("SOUL.md").exists());
        assert_eq!(migration.copied.len(), 13);
        assert!(migration.conflicts.is_empty());
    }

    #[test]
    fn canonical_configuration_wins_without_overwriting_legacy_source() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join(LEGACY_MCP_SERVERS_FILE), "legacy").unwrap();
        std::fs::write(paths.mcps(), "canonical").unwrap();

        let migration = paths.migrate_legacy_layout().unwrap();

        assert_eq!(std::fs::read_to_string(paths.mcps()).unwrap(), "canonical");
        assert_eq!(
            std::fs::read_to_string(config.join(LEGACY_MCP_SERVERS_FILE)).unwrap(),
            "legacy"
        );
        assert!(migration.copied.is_empty());
        assert_eq!(migration.conflicts.len(), 1);
    }
}
