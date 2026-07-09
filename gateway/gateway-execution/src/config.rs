//! # Execution Configuration
//!
//! Configuration types for agent execution.

use agent_primitives::FileSystemContext;
use execution_state::TriggerSource;
use gateway_events::HookContext;
use serde_json::Value;
use std::path::PathBuf;

// ============================================================================
// FILE SYSTEM CONTEXT
// ============================================================================

/// File system context for gateway execution.
///
/// Provides paths to the agent tools based on the vault directory structure.
#[derive(Debug, Clone)]
pub struct GatewayFileSystem {
    /// Base vault/config directory
    vault_dir: PathBuf,
}

impl GatewayFileSystem {
    /// Create a new gateway file system context.
    pub fn new(vault_dir: PathBuf) -> Self {
        Self { vault_dir }
    }
}

impl FileSystemContext for GatewayFileSystem {
    fn conversation_dir(&self, conversation_id: &str) -> Option<PathBuf> {
        Some(self.vault_dir.join("conversations").join(conversation_id))
    }

    fn outputs_dir(&self) -> Option<PathBuf> {
        Some(self.vault_dir.join("outputs"))
    }

    fn skills_dir(&self) -> Option<PathBuf> {
        Some(self.vault_dir.join("skills"))
    }

    /// Skills load from the vault first (writable, user-owned) and then
    /// from `$HOME/.agents/skills/` (read-only, externally installed).
    /// Mirrors `gateway_services::VaultPaths::skills_dirs()` so the runtime
    /// loader sees the same roots the indexer does.
    fn skills_dirs(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.vault_dir.join("skills")];
        if let Some(agent_root) = dirs::home_dir().map(|h| h.join(".agents").join("skills")) {
            roots.push(agent_root);
        }
        roots
    }

    fn agents_dir(&self) -> Option<PathBuf> {
        Some(self.vault_dir.join("agents"))
    }

    fn agent_data_dir(&self, agent_id: &str) -> Option<PathBuf> {
        // Agent data is stored in wards/{agent_id}/
        Some(self.vault_dir.join("wards").join(agent_id))
    }

    fn python_executable(&self) -> Option<PathBuf> {
        // Use system Python - could be made configurable
        None
    }

    fn vault_path(&self) -> Option<PathBuf> {
        Some(self.vault_dir.clone())
    }

    fn session_code_dir(&self, session_id: &str) -> Option<PathBuf> {
        Some(self.vault_dir.join("code").join(session_id))
    }

    fn session_data_dir(&self, session_id: &str) -> Option<PathBuf> {
        // Session data is stored in wards/{session_id}/
        Some(self.vault_dir.join("wards").join(session_id))
    }

    fn wards_root_dir(&self) -> Option<PathBuf> {
        Some(self.vault_dir.join("wards"))
    }

    fn ward_dir(&self, ward_id: &str) -> Option<PathBuf> {
        Some(self.vault_dir.join("wards").join(ward_id))
    }

    fn mcps_config(&self) -> Option<PathBuf> {
        Some(self.vault_dir.join("config").join("mcps.json"))
    }
}

// ============================================================================
// EXECUTION CONFIG
// ============================================================================

/// Configuration for agent execution.
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// Agent ID to execute
    pub agent_id: String,
    /// Conversation ID for tracking (legacy, used for message persistence)
    pub conversation_id: String,
    /// Configuration directory (vault path)
    pub config_dir: PathBuf,
    /// Maximum iterations before prompting for continuation
    pub max_iterations: u32,
    /// Optional hook context for routing responses
    pub hook_context: Option<HookContext>,
    /// Optional session ID for continuing an existing session.
    /// - None: create a new session
    /// - Some(id): continue the existing session with this ID
    pub session_id: Option<String>,
    /// Optional connector IDs to route the final response to.
    /// - None/empty: response goes to WebSocket subscribers only (default)
    /// - Some([...]): response also dispatched to listed connectors
    pub respond_to: Option<Vec<String>>,
    /// Thread ID for conversation threading with external connectors.
    pub thread_id: Option<String>,
    /// Connector ID that triggered this session.
    pub connector_id: Option<String>,
    /// Trigger source for this execution (web, cli, connector, etc.)
    pub source: TriggerSource,
    /// Metadata from the request (e.g., plugin context, sender info)
    pub metadata: Option<Value>,
    /// Execution mode: "fast"/"chat" skips intent analysis pipeline and uses a lean prompt;
    /// "deep"/"research" runs the full pipeline (intent analysis, planning, delegation, wards).
    /// Memory injection runs in BOTH modes — mode only gates the pipeline depth.
    ///
    /// Any other value (including "deep"/"research") uses the research behavior.
    pub mode: Option<String>,
}

/// Session execution mode — split from "fast_mode" to decouple memory injection
/// from pipeline depth. See [`ExecutionConfig::mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Chat mode: skip intent analysis / planning / delegation / ward transitions.
    /// Memory injection and skills still run. Lean prompt, higher temperature.
    #[serde(alias = "fast")]
    Chat,
    /// Research mode: full pipeline (intent analysis, planning, delegation, wards).
    /// Full system prompt.
    #[serde(alias = "deep")]
    #[default]
    Research,
}

impl SessionMode {
    /// Parse from the legacy string-mode representation used on the wire.
    /// "fast"/"chat" → Chat; anything else (including None/"deep"/"research") → Research.
    pub fn from_mode_string(mode: Option<&str>) -> Self {
        match mode {
            Some("fast") | Some("chat") => Self::Chat,
            _ => Self::Research,
        }
    }
}

impl ExecutionConfig {
    /// Create a new execution config.
    pub fn new(agent_id: String, conversation_id: String, config_dir: PathBuf) -> Self {
        Self {
            agent_id,
            conversation_id,
            config_dir,
            max_iterations: 1000,
            hook_context: None,
            session_id: None,
            respond_to: None,
            thread_id: None,
            connector_id: None,
            source: TriggerSource::default(),
            metadata: None,
            mode: None,
        }
    }

    /// Set the hook context for routing responses.
    #[must_use]
    pub fn with_hook_context(mut self, hook_context: HookContext) -> Self {
        self.hook_context = Some(hook_context);
        self
    }

    /// Set the session ID to continue an existing session.
    #[must_use]
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Set the connector IDs to route the final response to.
    #[must_use]
    pub fn with_respond_to(mut self, connector_ids: Vec<String>) -> Self {
        self.respond_to = Some(connector_ids);
        self
    }

    /// Set the thread ID for conversation threading.
    #[must_use]
    pub fn with_thread_id(mut self, thread_id: String) -> Self {
        self.thread_id = Some(thread_id);
        self
    }

    /// Set the connector ID that triggered this session.
    #[must_use]
    pub fn with_connector_id(mut self, connector_id: String) -> Self {
        self.connector_id = Some(connector_id);
        self
    }

    /// Set the trigger source for this execution.
    #[must_use]
    pub fn with_source(mut self, source: TriggerSource) -> Self {
        self.source = source;
        self
    }

    /// Set the metadata from the request.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set the execution mode ("fast" or "deep").
    #[must_use]
    pub fn with_mode(mut self, mode: String) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Returns the typed session mode (memory-safe successor to `is_fast_mode`).
    pub fn session_mode(&self) -> SessionMode {
        SessionMode::from_mode_string(self.mode.as_deref())
    }

    /// Returns true if this execution is in chat mode (skip pipeline, keep memory).
    pub fn is_chat_mode(&self) -> bool {
        matches!(self.session_mode(), SessionMode::Chat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn execution_config_new_defaults() {
        let config = ExecutionConfig::new(
            "root".to_string(),
            "conv-123".to_string(),
            PathBuf::from("/tmp"),
        );

        assert_eq!(config.agent_id, "root");
        assert_eq!(config.conversation_id, "conv-123");
        assert_eq!(config.source, TriggerSource::Web); // default
        assert!(config.metadata.is_none());
        assert!(config.session_id.is_none());
        assert!(config.respond_to.is_none());
    }

    #[test]
    fn execution_config_with_source() {
        let config = ExecutionConfig::new(
            "root".to_string(),
            "conv-123".to_string(),
            PathBuf::from("/tmp"),
        )
        .with_source(TriggerSource::Connector);

        assert_eq!(config.source, TriggerSource::Connector);
    }

    #[test]
    fn execution_config_with_metadata() {
        let metadata = serde_json::json!({
            "thread_id": "C123:1234567890.123456",
            "sender": "U12345",
        });

        let config = ExecutionConfig::new(
            "root".to_string(),
            "conv-123".to_string(),
            PathBuf::from("/tmp"),
        )
        .with_metadata(metadata.clone());

        assert_eq!(config.metadata, Some(metadata));
    }

    #[test]
    fn execution_config_builder_chain() {
        let metadata = serde_json::json!({"key": "value"});
        let config = ExecutionConfig::new(
            "root".to_string(),
            "conv-123".to_string(),
            PathBuf::from("/tmp"),
        )
        .with_source(TriggerSource::Connector)
        .with_metadata(metadata.clone())
        .with_session_id("sess-456".to_string())
        .with_thread_id("thread-789".to_string())
        .with_connector_id("slack".to_string())
        .with_respond_to(vec!["slack".to_string()]);

        assert_eq!(config.source, TriggerSource::Connector);
        assert_eq!(config.metadata, Some(metadata));
        assert_eq!(config.session_id, Some("sess-456".to_string()));
        assert_eq!(config.thread_id, Some("thread-789".to_string()));
        assert_eq!(config.connector_id, Some("slack".to_string()));
        assert_eq!(config.respond_to, Some(vec!["slack".to_string()]));
    }

    #[test]
    fn session_mode_parses_chat_aliases() {
        assert_eq!(
            SessionMode::from_mode_string(Some("fast")),
            SessionMode::Chat
        );
        assert_eq!(
            SessionMode::from_mode_string(Some("chat")),
            SessionMode::Chat
        );
    }

    #[test]
    fn session_mode_parses_research_aliases() {
        assert_eq!(
            SessionMode::from_mode_string(Some("deep")),
            SessionMode::Research
        );
        assert_eq!(
            SessionMode::from_mode_string(Some("research")),
            SessionMode::Research
        );
        assert_eq!(SessionMode::from_mode_string(None), SessionMode::Research);
        assert_eq!(
            SessionMode::from_mode_string(Some("bogus")),
            SessionMode::Research
        );
    }

    #[test]
    fn session_mode_serde_aliases() {
        // Wire compat: legacy "fast"/"deep" values must still deserialize.
        let chat: SessionMode = serde_json::from_str("\"fast\"").unwrap();
        assert_eq!(chat, SessionMode::Chat);
        let research: SessionMode = serde_json::from_str("\"deep\"").unwrap();
        assert_eq!(research, SessionMode::Research);
        // New canonical values also work.
        let chat2: SessionMode = serde_json::from_str("\"chat\"").unwrap();
        assert_eq!(chat2, SessionMode::Chat);
        let research2: SessionMode = serde_json::from_str("\"research\"").unwrap();
        assert_eq!(research2, SessionMode::Research);
    }

    #[test]
    fn execution_config_is_chat_mode_tracks_mode_string() {
        let base = ExecutionConfig::new("root".to_string(), "c".to_string(), PathBuf::from("/tmp"));
        assert!(!base.is_chat_mode());
        assert_eq!(base.session_mode(), SessionMode::Research);

        let chat = base.clone().with_mode("fast".to_string());
        assert!(chat.is_chat_mode());
        assert_eq!(chat.session_mode(), SessionMode::Chat);

        let chat_new = base.clone().with_mode("chat".to_string());
        assert!(chat_new.is_chat_mode());

        let research = base.with_mode("deep".to_string());
        assert!(!research.is_chat_mode());
    }

    #[test]
    fn execution_config_with_hook_context_stores_it() {
        use gateway_events::HookType;
        let ctx = HookContext {
            hook_type: HookType::Cli,
            source_id: "cli-1".into(),
            channel_id: None,
            metadata: std::collections::HashMap::new(),
            created_at: chrono::Utc::now(),
        };
        let cfg = ExecutionConfig::new("root".into(), "c".into(), PathBuf::from("/tmp"))
            .with_hook_context(ctx);
        assert!(cfg.hook_context.is_some());
    }

    // --- GatewayFileSystem: FileSystemContext impl ---
    //
    // One assertion per method. All deterministic: every path resolves to a
    // pre-known subdir under the vault, except `python_executable` which is
    // hardcoded None.

    #[test]
    fn gateway_fs_resolves_every_vault_subdir() {
        let vault = PathBuf::from("/v");
        let fs = GatewayFileSystem::new(vault.clone());

        assert_eq!(fs.vault_path(), Some(vault.clone()));
        assert_eq!(fs.outputs_dir(), Some(vault.join("outputs")));
        assert_eq!(fs.skills_dir(), Some(vault.join("skills")));
        assert_eq!(fs.agents_dir(), Some(vault.join("agents")));
        assert_eq!(fs.wards_root_dir(), Some(vault.join("wards")));
        assert_eq!(
            fs.mcps_config(),
            Some(vault.join("config").join("mcps.json"))
        );
        assert_eq!(
            fs.conversation_dir("c1"),
            Some(vault.join("conversations").join("c1"))
        );
        assert_eq!(
            fs.session_code_dir("s1"),
            Some(vault.join("code").join("s1"))
        );
        assert_eq!(
            fs.session_data_dir("s1"),
            Some(vault.join("wards").join("s1"))
        );
        assert_eq!(
            fs.agent_data_dir("agent-a"),
            Some(vault.join("wards").join("agent-a"))
        );
        assert_eq!(fs.ward_dir("w1"), Some(vault.join("wards").join("w1")));
    }

    #[test]
    fn gateway_fs_python_executable_is_none() {
        // Documented as "use system Python — could be made configurable."
        // If this ever returns Some(...), the behaviour change should be
        // flagged by this test flipping.
        let fs = GatewayFileSystem::new(PathBuf::from("/v"));
        assert!(fs.python_executable().is_none());
    }
}
