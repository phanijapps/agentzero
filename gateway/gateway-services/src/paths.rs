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
use std::path::PathBuf;
use std::sync::Arc;

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

    /// Path to `config/mcps.json`
    pub fn mcps(&self) -> PathBuf {
        self.vault_dir.join("config").join("mcps.json")
    }

    /// Path to `config/connectors.json`
    pub fn connectors(&self) -> PathBuf {
        self.vault_dir.join("config").join("connectors.json")
    }

    /// Path to `config/cron_jobs.json`
    pub fn cron_jobs(&self) -> PathBuf {
        self.vault_dir.join("config").join("cron_jobs.json")
    }

    /// Path to `config/INSTRUCTIONS.md`
    pub fn instructions(&self) -> PathBuf {
        self.vault_dir.join("config").join("INSTRUCTIONS.md")
    }

    /// Path to `config/distillation_prompt.md`
    pub fn distillation_prompt(&self) -> PathBuf {
        self.vault_dir.join("config").join("distillation_prompt.md")
    }

    /// Path to `config/intent_analysis_prompt.md`
    pub fn intent_analysis_prompt(&self) -> PathBuf {
        self.vault_dir
            .join("config")
            .join("intent_analysis_prompt.md")
    }

    /// Path to the config directory
    pub fn config_dir(&self) -> PathBuf {
        self.vault_dir.join("config")
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

    /// Ensure all required directories exist.
    ///
    /// Creates:
    /// - config/
    /// - config/wards/ (language configs for ward indexing)
    /// - data/
    /// - logs/
    /// - agents/
    /// - skills/
    /// - plugins/
    /// - wards/ (with scratch subdirectory)
    pub fn ensure_dirs_exist(&self) -> io::Result<()> {
        let dirs = [
            self.config_dir(),
            self.ward_lang_configs_dir(),
            self.data_dir(),
            self.traces_dir(),
            self.logs_dir(),
            self.agents_dir(),
            self.skills_dir(),
            self.plugins_dir(),
            self.wards_dir(),
            self.temp_dir(),
        ];

        for dir in &dirs {
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
                tracing::debug!("Created directory: {:?}", dir);
            }
        }

        // Ensure scratch ward exists
        let scratch_dir = self.ward_dir("scratch");
        if !scratch_dir.exists() {
            std::fs::create_dir_all(&scratch_dir)?;
            tracing::debug!("Created scratch ward: {:?}", scratch_dir);
        }

        Ok(())
    }
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
        assert_eq!(paths.mcps(), dir.path().join("config").join("mcps.json"));
        assert_eq!(
            paths.connectors(),
            dir.path().join("config").join("connectors.json")
        );
        assert_eq!(
            paths.cron_jobs(),
            dir.path().join("config").join("cron_jobs.json")
        );
        assert_eq!(
            paths.instructions(),
            dir.path().join("config").join("INSTRUCTIONS.md")
        );
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
    fn test_ensure_dirs_exist() {
        let dir = tempdir().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());

        paths.ensure_dirs_exist().unwrap();

        assert!(dir.path().join("config").exists());
        assert!(dir.path().join("data").exists());
        assert!(dir.path().join("logs").exists());
        assert!(dir.path().join("agents").exists());
        assert!(dir.path().join("skills").exists());
        assert!(dir.path().join("plugins").exists());
        assert!(dir.path().join("wards").exists());
        assert!(dir.path().join("wards").join("scratch").exists());
    }
}
