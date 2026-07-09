// ============================================================================
// FILE SYSTEM CONTEXT TRAIT
// ============================================================================
// This module defines only the FileSystemContext trait and related runtime
// context wrappers. Concrete tool implementations live in `agent-tools`.

use std::path::PathBuf;
use std::sync::Arc;

/// Trait for providing file system context to tools
///
/// This allows the framework to be used with different directory structures
/// without depending on application-specific code like `AppDirs`.
pub trait FileSystemContext: Send + Sync {
    /// Get the conversation directory for a given conversation ID
    fn conversation_dir(&self, conversation_id: &str) -> Option<PathBuf>;

    /// Get the outputs directory
    fn outputs_dir(&self) -> Option<PathBuf>;

    /// Get the skills directory
    fn skills_dir(&self) -> Option<PathBuf>;

    /// Get the Python executable path
    fn python_executable(&self) -> Option<PathBuf>;
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

    fn python_executable(&self) -> Option<PathBuf> {
        None
    }
}

// ============================================================================
// TOOL CONTEXT WITH FILE SYSTEM
// ============================================================================

use super::context::ToolContext as BaseToolContext;

/// Extended tool context with file system access
pub struct ToolContextWithFs {
    /// Base tool context
    pub base: BaseToolContext,

    /// File system context
    pub fs: Arc<dyn FileSystemContext>,
}

impl ToolContextWithFs {
    /// Create a new tool context with file system
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self {
            base: BaseToolContext::new(),
            fs,
        }
    }

    /// Create with conversation ID
    #[must_use]
    pub fn with_conversation(fs: Arc<dyn FileSystemContext>, conversation_id: String) -> Self {
        Self {
            base: BaseToolContext::with_conversation(conversation_id),
            fs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fs_returns_none_for_every_path() {
        let fs = NoFileSystemContext;
        assert!(fs.conversation_dir("any-conv").is_none());
        assert!(fs.outputs_dir().is_none());
        assert!(fs.skills_dir().is_none());
        assert!(fs.python_executable().is_none());
    }

    #[test]
    fn no_fs_default_works_via_trait() {
        let fs: Arc<dyn FileSystemContext> = Arc::new(NoFileSystemContext);
        assert!(fs.conversation_dir("c1").is_none());
        assert!(fs.outputs_dir().is_none());
        assert!(fs.skills_dir().is_none());
        assert!(fs.python_executable().is_none());
    }

    #[test]
    fn no_fs_default_constructor() {
        // Exercise the derived Default impl
        let fs: NoFileSystemContext = NoFileSystemContext;
        let _ = fs.clone();
    }

    #[test]
    fn tool_context_with_fs_new_has_no_conversation() {
        let fs: Arc<dyn FileSystemContext> = Arc::new(NoFileSystemContext);
        let ctx = ToolContextWithFs::new(fs);
        assert!(ctx.base.conversation_id.is_none());
        assert!(ctx.fs.conversation_dir("c").is_none());
    }

    #[test]
    fn tool_context_with_fs_with_conversation_propagates() {
        let fs: Arc<dyn FileSystemContext> = Arc::new(NoFileSystemContext);
        let ctx = ToolContextWithFs::with_conversation(fs, "conv-42".to_string());
        assert_eq!(ctx.base.conversation_id.as_deref(), Some("conv-42"));
    }
}
