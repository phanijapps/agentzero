//! # File System Context
//!
//! File system abstraction for tool execution.

use std::path::PathBuf;

/// Trait for providing file system context to tools
///
/// This allows the framework to be used with different directory structures
/// without depending on application-specific code.
pub trait FileSystemContext: Send + Sync {
    /// Get the conversation directory for a given conversation ID
    fn conversation_dir(&self, conversation_id: &str) -> Option<PathBuf>;

    /// Get the outputs directory
    fn outputs_dir(&self) -> Option<PathBuf>;

    /// Get the (writable) skills directory.
    ///
    /// Most callers should prefer [`skills_dirs`](Self::skills_dirs), which
    /// also includes externally-installed roots like `$HOME/.agents/skills`.
    /// This method exists for create / update / delete operations that must
    /// target a single writable root.
    fn skills_dir(&self) -> Option<PathBuf>;

    /// Get every skills root in priority order (vault first, external
    /// roots after). Returns at least one path when `skills_dir()` is
    /// `Some`. Implementers can override to include additional read-only
    /// roots — the runtime `LoadSkillTool` walks the list and resolves a
    /// skill against the first root containing a matching `SKILL.md`.
    fn skills_dirs(&self) -> Vec<PathBuf> {
        self.skills_dir().into_iter().collect()
    }

    /// Get the agents directory
    fn agents_dir(&self) -> Option<PathBuf>;

    /// Get the agent data directory for a specific agent
    /// Returns the base directory for agent-specific data (e.g., wards/{agent-id}/)
    /// Note: Agent data is stored in wards to unify with session and ward storage.
    fn agent_data_dir(&self, agent_id: &str) -> Option<PathBuf> {
        self.ward_dir(agent_id)
    }

    /// Get the Python executable path
    fn python_executable(&self) -> Option<PathBuf>;

    /// Get the agent-specific node_modules directory
    /// Returns the directory for agent-specific npm packages (e.g., agents_data/{agent-id}/node_modules/)
    fn agent_node_modules_dir(&self, agent_id: &str) -> Option<PathBuf> {
        // Default implementation: node_modules inside agent_data_dir
        self.agent_data_dir(agent_id)
            .map(|p| p.join("node_modules"))
    }

    /// Get the vault/config root path
    /// This is the base path for all configuration
    fn vault_path(&self) -> Option<PathBuf> {
        None
    }

    /// Get the session code directory (where shell runs and code files live)
    /// Returns `{vault}/code/{session_id}/`
    fn session_code_dir(&self, session_id: &str) -> Option<PathBuf> {
        self.vault_path().map(|p| p.join("code").join(session_id))
    }

    /// Get the session data directory (for attachments, scratchpad, etc.)
    /// Returns `{vault}/wards/{session_id}/`
    /// Note: Session data is stored in wards to unify with agent and ward storage.
    fn session_data_dir(&self, session_id: &str) -> Option<PathBuf> {
        self.ward_dir(session_id)
    }

    /// Get the wards root directory.
    /// Returns `{vault}/wards/`
    fn wards_root_dir(&self) -> Option<PathBuf> {
        self.vault_path().map(|p| p.join("wards"))
    }

    /// Get a specific ward directory.
    /// Returns `{vault}/wards/{ward_id}/`
    fn ward_dir(&self, ward_id: &str) -> Option<PathBuf> {
        self.wards_root_dir().map(|p| p.join(ward_id))
    }

    /// Get the MCP servers configuration file path.
    /// Returns `{vault}/config/mcps.json`
    fn mcps_config(&self) -> Option<PathBuf> {
        None
    }
}

/// Default file system context that returns None for all paths
/// (for library-only usage without application integration)
#[derive(Debug, Clone, Default)]
pub struct NoFileSystemContext;

impl FileSystemContext for NoFileSystemContext {
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
        None
    }

    fn python_executable(&self) -> Option<PathBuf> {
        None
    }

    fn vault_path(&self) -> Option<PathBuf> {
        None
    }

    fn wards_root_dir(&self) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_file_system_context() {
        let ctx = NoFileSystemContext;
        assert!(ctx.conversation_dir("test").is_none());
        assert!(ctx.outputs_dir().is_none());
        assert!(ctx.skills_dir().is_none());
        assert!(ctx.python_executable().is_none());
    }
}
