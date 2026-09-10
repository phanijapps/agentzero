//! # Settings Service
//!
//! Service for managing application settings including tool and logging configuration.

use crate::logging::LogSettings;
use agent_primitives::vault_paths::SharedVaultPaths;
use agent_tools::ToolSettings;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

use crate::models::{DEFAULT_MAX_INPUT_TOKENS, DEFAULT_MAX_OUTPUT_TOKENS};

// Re-export so `gateway_services::settings::MemorySettings` keeps working;
// `lib.rs` continues to surface it as `gateway_services::MemorySettings`.
pub use gateway_memory::MemorySettings;

/// Application settings.
///
/// Stored in `{data_dir}/settings.json` and persisted across restarts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Tool settings (enable/disable optional tools)
    #[serde(default)]
    pub tools: ToolSettings,

    /// Logging configuration (file logging, rotation, etc.)
    #[serde(default)]
    pub logs: LogSettings,

    /// Execution settings (concurrency, delegation limits, etc.)
    #[serde(default)]
    pub execution: ExecutionSettings,

    /// Network / discovery configuration. New top-level block; absent in
    /// pre-v0.X settings.json files, in which case the default
    /// (`exposeToLan: true`) applies.
    #[serde(default)]
    pub network: discovery::DiscoveryConfig,

    /// Durable first-run commissioning choices. This deliberately records
    /// portable semantic intent only; provisioning stays outside the gateway.
    #[serde(default)]
    pub commissioning: CommissioningSettings,

    /// Display-only work-surface presentation preferences.
    #[serde(default)]
    pub presentation: PresentationSettings,
}

/// Live presentation settings persisted in `settings.json`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSettings {
    /// Save validated display-only surfaces and restore them on session load.
    #[serde(default)]
    pub persist_surfaces: bool,
}

/// Durable readiness state for the Agent Commissioning flow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommissioningState {
    #[default]
    NotStarted,
    InProgress,
    NeedsAttention,
    Complete,
}

/// Portable semantic choices consumed by a later semantic provider integration.
/// No Engram identifier, crate type, or storage path belongs in this type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticProfile {
    pub version: u32,
    pub base_pack_ids: Vec<String>,
    pub domain_pack_ids: Vec<String>,
    pub provisioning: SemanticProvisioning,
}

impl Default for SemanticProfile {
    fn default() -> Self {
        Self {
            version: 1,
            base_pack_ids: vec!["zbot.base:v1".to_string(), "zbot.general:v1".to_string()],
            domain_pack_ids: Vec::new(),
            provisioning: SemanticProvisioning::Deferred,
        }
    }
}

/// Commissioning owns the selection; semantic provisioning is intentionally
/// deferred until the provider integration consumes this profile.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SemanticProvisioning {
    #[default]
    Deferred,
}

/// User-owned configuration collected during commissioning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommissioningSettings {
    pub version: u32,
    pub state: CommissioningState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_focus: Option<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Personal details the user elects to keep in local z-Bot data. These are
    /// deliberately separate from the agent's working instructions, so they
    /// are not copied into `SOUL.md` or sent as model context by commissioning.
    #[serde(default)]
    pub user_profile: UserProfile,
    #[serde(default = "default_commissioning_autonomy")]
    pub autonomy: String,
    #[serde(default = "default_commissioning_privacy")]
    pub privacy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub semantic_profile: SemanticProfile,
}

/// A locally persisted, user-owned profile collected during commissioning.
///
/// This is configuration data, not semantic memory. It has no backend or
/// provider dependency and is intentionally omitted from status responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub interests: Vec<String>,
    #[serde(default)]
    pub hobbies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_of_birth: Option<String>,
}

impl Default for CommissioningSettings {
    fn default() -> Self {
        Self {
            version: 1,
            state: CommissioningState::NotStarted,
            primary_focus: None,
            domains: Vec::new(),
            profile: None,
            user_profile: UserProfile::default(),
            autonomy: default_commissioning_autonomy(),
            privacy: default_commissioning_privacy(),
            provider_id: None,
            model: None,
            semantic_profile: SemanticProfile::default(),
        }
    }
}

fn default_commissioning_autonomy() -> String {
    "guided".to_string()
}

fn default_commissioning_privacy() -> String {
    "local_preferred".to_string()
}

/// Execution settings for controlling agent concurrency and delegation behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSettings {
    /// Maximum number of subagents that can run in parallel across all sessions.
    /// Default: 2. Set lower for resource-constrained environments.
    #[serde(default = "default_max_parallel_agents")]
    pub max_parallel_agents: u32,
    /// Whether the first-time setup wizard has been completed.
    /// Default: false. Set to true after the wizard finishes.
    #[serde(default)]
    pub setup_complete: bool,
    /// The user-chosen name for the root agent (e.g., "Brahmi", "Jarvis").
    /// Used in SOUL.md and displayed in the UI.
    #[serde(default)]
    pub agent_name: Option<String>,
    /// Disable streaming (SSE) for subagents — use non-streaming requests instead.
    /// Default: true. Subagents run in background, nobody watches their output in
    /// real-time. Non-streaming is more reliable (no mid-stream decode errors).
    #[serde(default = "default_true")]
    pub subagent_non_streaming: bool,
    /// Root agent (orchestrator) configuration.
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    /// Distillation model configuration (provider/model override).
    #[serde(default)]
    pub distillation: DistillationConfig,
    /// Ward-curator LLM configuration (provider/model override).
    #[serde(default)]
    pub curator: CuratorConfig,
    /// Intent-analysis LLM configuration (provider/model override).
    #[serde(default)]
    pub intent_analysis: IntentAnalysisConfig,
    /// Multimodal model configuration (vision analysis fallback).
    #[serde(default)]
    pub multimodal: MultimodalConfig,
    /// Persistent chat session configuration.
    #[serde(default)]
    pub chat: ChatConfig,
    /// Wiki / Obsidian vault ward configuration.
    #[serde(default)]
    pub wiki: WikiConfig,
    /// Background memory worker configuration.
    #[serde(default)]
    pub memory: MemorySettings,
    /// Experimental UI feature flags. Free-form bag persisted verbatim so
    /// we can gate beta surfaces without schema churn.
    #[serde(default)]
    pub feature_flags: std::collections::HashMap<String, bool>,
}

/// Root agent (orchestrator) configuration.
/// Stored in settings.json, NOT in agents/root/config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestratorConfig {
    /// Provider ID for the orchestrator. None = use default provider.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Model for the orchestrator. None = use provider's default model.
    #[serde(default)]
    pub model: Option<String>,
    /// Temperature (0.0 - 2.0). Default: 0.7.
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    /// Maximum input tokens. None = inherit from provider/model limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
    /// Maximum output tokens. Default: 32000.
    #[serde(
        rename = "maxOutputTokens",
        alias = "maxTokens",
        default = "default_max_output_tokens"
    )]
    pub max_tokens: u32,
    /// Enable extended thinking/reasoning. Default: true.
    /// Orchestrator reasons before delegating — improves planning quality.
    #[serde(default = "default_true")]
    pub thinking_enabled: bool,
}

fn default_max_parallel_agents() -> u32 {
    2
}
fn default_true() -> bool {
    true
}
fn default_temperature() -> f64 {
    0.7
}
fn default_max_input_tokens() -> u64 {
    DEFAULT_MAX_INPUT_TOKENS
}
fn default_max_output_tokens() -> u32 {
    DEFAULT_MAX_OUTPUT_TOKENS
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            provider_id: None,
            model: None,
            temperature: default_temperature(),
            max_input_tokens: None,
            max_tokens: default_max_output_tokens(),
            thinking_enabled: true,
        }
    }
}

impl OrchestratorConfig {
    pub fn effective_max_input_tokens(&self) -> u64 {
        self.max_input_tokens.unwrap_or(DEFAULT_MAX_INPUT_TOKENS)
    }

    pub fn max_input_tokens_explicit(&self) -> bool {
        self.max_input_tokens.is_some()
    }
}

/// Distillation model configuration.
/// Controls which provider/model is used for session distillation.
/// Both fields default to None, inheriting from orchestrator config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct DistillationConfig {
    /// Provider ID override. None = inherit from orchestrator config.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Model override. None = inherit from orchestrator config.
    #[serde(default)]
    pub model: Option<String>,
    /// Max input tokens override. None = inherit from orchestrator config.
    #[serde(
        default,
        rename = "maxInputTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_input_tokens: Option<u64>,
    /// Max output tokens override. None = inherit from orchestrator config.
    #[serde(
        default,
        rename = "maxOutputTokens",
        alias = "maxTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens: Option<u32>,
}

/// Ward-curator LLM configuration (provider/model override).
/// Used by `POST /api/curator/consolidate` for the Phase C consolidation
/// LLM call. Mirrors `DistillationConfig`'s shape — empty values inherit
/// the orchestrator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuratorConfig {
    /// Provider ID override. None = inherit from orchestrator config.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Model override. None = inherit from orchestrator config.
    #[serde(default)]
    pub model: Option<String>,
    /// Max input tokens override. None = inherit from orchestrator config.
    #[serde(
        default,
        rename = "maxInputTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_input_tokens: Option<u64>,
    /// Max output tokens override. None = inherit from orchestrator config.
    #[serde(
        default,
        rename = "maxOutputTokens",
        alias = "maxTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens: Option<u32>,
}

/// Intent-analysis LLM configuration (provider/model override).
/// Used by `analyze_intent` at root-session pre-flight — the
/// highest-frequency LLM call in the system (every root prompt), so
/// routing this to a cheaper/faster model has the biggest recurring cost
/// impact. Quality risk: bad classification mis-routes the rest of the
/// session, so verify before pinning to a weaker model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentAnalysisConfig {
    /// Provider ID override. None = inherit from orchestrator config.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Model override. None = inherit from orchestrator config.
    #[serde(default)]
    pub model: Option<String>,
    /// Max input tokens override. None = inherit from orchestrator config.
    #[serde(
        default,
        rename = "maxInputTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_input_tokens: Option<u64>,
    /// Max output tokens override. None = inherit from orchestrator config.
    #[serde(
        default,
        rename = "maxOutputTokens",
        alias = "maxTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens: Option<u32>,
}

/// Default multimodal model configuration.
/// Used by the multimodal_analyze tool as a universal vision fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultimodalConfig {
    pub provider_id: Option<String>,
    pub model: Option<String>,
    #[serde(default = "default_multimodal_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u64,
    #[serde(
        rename = "maxOutputTokens",
        alias = "maxTokens",
        default = "default_max_output_tokens"
    )]
    pub max_tokens: u32,
}

fn default_multimodal_temperature() -> f64 {
    0.3
}
impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            provider_id: None,
            model: None,
            temperature: default_multimodal_temperature(),
            max_input_tokens: default_max_input_tokens(),
            max_tokens: default_max_output_tokens(),
        }
    }
}

/// Configuration for the persistent chat session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    /// The permanent session ID for chat mode. Created on first /chat visit.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The conversation ID for WebSocket routing.
    #[serde(default)]
    pub conversation_id: Option<String>,
}

/// Configuration for the wiki / Obsidian vault ward.
///
/// The wiki ward is auto-created at startup and seeded with the canonical
/// Obsidian vault layout. Producer skills (book-reader, research archetypes)
/// write into their origin ward; the `wiki` skill then promotes content into
/// this ward. The name is configurable so multiple vaults (work/personal/
/// client) are a settings change away.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiConfig {
    /// Ward name used as the Obsidian vault. Default: "wiki".
    #[serde(default = "default_wiki_ward_name")]
    pub ward_name: String,
}

fn default_wiki_ward_name() -> String {
    "wiki".to_string()
}

impl Default for WikiConfig {
    fn default() -> Self {
        Self {
            ward_name: default_wiki_ward_name(),
        }
    }
}

impl Default for ExecutionSettings {
    fn default() -> Self {
        Self {
            max_parallel_agents: default_max_parallel_agents(),
            setup_complete: false,
            agent_name: None,
            subagent_non_streaming: true,
            orchestrator: OrchestratorConfig::default(),
            distillation: DistillationConfig::default(),
            curator: CuratorConfig::default(),
            intent_analysis: IntentAnalysisConfig::default(),
            multimodal: MultimodalConfig::default(),
            chat: ChatConfig::default(),
            wiki: WikiConfig::default(),
            memory: MemorySettings::default(),
            feature_flags: std::collections::HashMap::new(),
        }
    }
}

/// Service for managing application settings.
pub struct SettingsService {
    paths: SharedVaultPaths,
    cache: RwLock<Option<AppSettings>>,
}

impl SettingsService {
    /// Create a new settings service.
    pub fn new(paths: SharedVaultPaths) -> Self {
        Self {
            paths,
            cache: RwLock::new(None),
        }
    }

    /// Create a settings service from a vault root.
    /// Used for early initialization before shared paths are available.
    pub fn from_vault_dir(vault_dir: PathBuf) -> Self {
        Self {
            paths: std::sync::Arc::new(agent_primitives::vault_paths::VaultPaths::new(vault_dir)),
            cache: RwLock::new(None),
        }
    }

    /// Get the config file path.
    fn config_path(&self) -> PathBuf {
        self.paths.settings()
    }

    /// Invalidate the cache, forcing next read to go to disk.
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            *cache = None;
        }
    }

    /// Load settings from disk (bypasses cache).
    fn load_from_disk(&self) -> Result<AppSettings, String> {
        if !self.config_path().exists() {
            return Ok(AppSettings::default());
        }

        let content = fs::read_to_string(self.config_path())
            .map_err(|e| format!("Failed to read settings.json: {}", e))?;

        serde_json::from_str(&content).map_err(|e| format!("Failed to parse settings.json: {}", e))
    }

    /// Load settings (cached).
    pub fn load(&self) -> Result<AppSettings, String> {
        // Check cache first
        if let Ok(cache) = self.cache.read() {
            if let Some(settings) = cache.as_ref() {
                return Ok(settings.clone());
            }
        }

        // Cache miss: read from disk
        let settings = self.load_from_disk()?;

        // Update cache
        if let Ok(mut cache) = self.cache.write() {
            *cache = Some(settings.clone());
        }

        Ok(settings)
    }

    /// Save settings to disk and update cache.
    ///
    /// Merges the typed `AppSettings` into the on-disk JSON object rather
    /// than overwriting the file wholesale. Any top-level keys present on
    /// disk but not modeled in `AppSettings` are preserved. This exists
    /// because other services (notably `EmbeddingService::persist_settings`)
    /// share the same file and write their own top-level sections — a
    /// typed overwrite here would strip them silently on every save.
    pub fn save(&self, settings: &AppSettings) -> Result<(), String> {
        let path = self.config_path();
        let parent = path
            .parent()
            .ok_or_else(|| "settings.json has no parent directory".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;

        // Load existing file as a free-form Value so unknown top-level
        // keys survive the round-trip.
        let mut merged: serde_json::Value = if path.exists() {
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read settings.json: {}", e))?;
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let typed_value = serde_json::to_value(settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        if let (Some(merged_obj), Some(typed_obj)) =
            (merged.as_object_mut(), typed_value.as_object())
        {
            for (key, value) in typed_obj {
                merged_obj.insert(key.clone(), value.clone());
            }
        } else {
            // On-disk file wasn't an object (corrupted / empty). Replace.
            merged = typed_value;
        }

        let content = serde_json::to_string_pretty(&merged)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        // Atomic write: temp + rename. Prevents a crash mid-write from
        // leaving a half-written settings.json, and keeps concurrent
        // readers safe from torn content.
        let tmp = parent.join("settings.json.tmp");
        fs::write(&tmp, content.as_bytes())
            .map_err(|e| format!("Failed to write settings.json tmp: {}", e))?;
        fs::rename(&tmp, &path).map_err(|e| format!("Failed to rename settings.json: {}", e))?;

        if let Ok(mut cache) = self.cache.write() {
            *cache = Some(settings.clone());
        }

        Ok(())
    }

    /// Get tool settings.
    pub fn get_tool_settings(&self) -> Result<ToolSettings, String> {
        let settings = self.load()?;
        Ok(settings.tools)
    }

    /// Update tool settings.
    pub fn update_tool_settings(&self, tool_settings: ToolSettings) -> Result<(), String> {
        let mut settings = self.load().unwrap_or_default();
        settings.tools = tool_settings;
        self.save(&settings)
    }

    /// Get log settings.
    pub fn get_log_settings(&self) -> Result<LogSettings, String> {
        let settings = self.load()?;
        Ok(settings.logs)
    }

    /// Update log settings.
    ///
    /// Note: Changes to log settings require a daemon restart to take effect.
    pub fn update_log_settings(&self, log_settings: LogSettings) -> Result<(), String> {
        // Validate before saving
        log_settings.validate()?;

        let mut settings = self.load().unwrap_or_default();
        settings.logs = log_settings;
        self.save(&settings)
    }

    /// Get execution settings.
    pub fn get_execution_settings(&self) -> Result<ExecutionSettings, String> {
        let settings = self.load()?;
        Ok(settings.execution)
    }

    /// Update execution settings.
    ///
    /// Note: Changes to max_parallel_agents require a daemon restart to take effect.
    pub fn update_execution_settings(
        &self,
        execution_settings: ExecutionSettings,
    ) -> Result<(), String> {
        if execution_settings.max_parallel_agents == 0 {
            return Err("max_parallel_agents must be at least 1".to_string());
        }
        let mut settings = self.load().unwrap_or_default();
        settings.execution = execution_settings;
        self.save(&settings)
    }

    pub fn get_presentation_settings(&self) -> Result<PresentationSettings, String> {
        Ok(self.load()?.presentation)
    }

    pub fn update_presentation_settings(
        &self,
        presentation: PresentationSettings,
    ) -> Result<(), String> {
        let mut settings = self.load().unwrap_or_default();
        settings.presentation = presentation;
        self.save(&settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_settings() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        let settings = service.load().unwrap();
        assert!(!settings.tools.file_tools);
        assert!(settings.tools.offload_large_results);
        // Logging is enabled by default (quiet mode)
        assert!(settings.logs.enabled);
        assert_eq!(settings.commissioning.state, CommissioningState::NotStarted);
        assert_eq!(
            settings.commissioning.semantic_profile.base_pack_ids,
            ["zbot.base:v1", "zbot.general:v1"]
        );
        // STUB: AC3/AC6 — persistence is an explicit typed opt-in.
        let serialized = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            serialized["presentation"]["persistSurfaces"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn presentation_settings_round_trip_preserves_unrelated_settings() {
        // STUB: AC3/AC6 — typed presentation updates keep other config intact.
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());
        let config_dir = dir.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("settings.json"),
            r#"{
              "presentation": {"persistSurfaces": true},
              "thirdParty": {"keep": "yes"}
            }"#,
        )
        .unwrap();

        let loaded = service.load().unwrap();
        let serialized = serde_json::to_value(&loaded).unwrap();
        assert_eq!(
            serialized["presentation"]["persistSurfaces"].as_bool(),
            Some(true)
        );
        service.save(&loaded).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(config_dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(raw["thirdParty"]["keep"].as_str(), Some("yes"));
    }

    #[test]
    fn commissioning_settings_round_trip_without_semantic_provider_details() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());
        let mut settings = AppSettings::default();
        settings.commissioning.state = CommissioningState::Complete;
        settings.commissioning.primary_focus = Some("research_learn".to_string());
        settings.commissioning.domains = vec!["learning".to_string()];
        settings.commissioning.user_profile = UserProfile {
            name: Some("Ada".to_string()),
            interests: vec!["Learning".to_string()],
            hobbies: vec!["Reading".to_string()],
            date_of_birth: Some("1990-01-01".to_string()),
        };
        settings.commissioning.semantic_profile.domain_pack_ids =
            vec!["zbot.learning:v1".to_string()];

        service.save(&settings).unwrap();
        service.invalidate_cache();
        let loaded = service.load().unwrap();

        assert_eq!(loaded.commissioning, settings.commissioning);
        let json = serde_json::to_string(&loaded.commissioning).unwrap();
        assert!(!json.contains("engram"));
        assert!(!json.contains("storagePath"));
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        let mut settings = AppSettings::default();
        settings.tools.file_tools = true;
        settings.tools.offload_large_results = false;

        service.save(&settings).unwrap();

        let loaded = service.load().unwrap();
        assert!(loaded.tools.file_tools);
        assert!(!loaded.tools.offload_large_results);
    }

    #[test]
    fn test_log_settings_crud() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        // Default: logging enabled with stdout suppressed
        let log_settings = service.get_log_settings().unwrap();
        assert!(log_settings.enabled);

        // Update: enable logging
        let mut new_log_settings = LogSettings::enabled();
        new_log_settings.max_files = 14;
        new_log_settings.level = "debug".to_string();

        service
            .update_log_settings(new_log_settings.clone())
            .unwrap();

        // Verify
        let loaded = service.get_log_settings().unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.max_files, 14);
        assert_eq!(loaded.level, "debug");
    }

    #[test]
    fn test_log_settings_validation() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        // Invalid log level should fail
        let invalid_settings = LogSettings {
            level: "invalid".to_string(),
            ..LogSettings::default()
        };

        let result = service.update_log_settings(invalid_settings);
        assert!(result.is_err());
    }

    #[test]
    fn test_settings_json_format() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        let mut settings = AppSettings::default();
        settings.tools.file_tools = true;
        settings.logs.enabled = true;
        settings.logs.max_files = 30;

        service.save(&settings).unwrap();

        // Read raw JSON to verify camelCase format
        let json_path = dir.path().join("config").join("settings.json");
        let json_content = fs::read_to_string(json_path).unwrap();

        assert!(json_content.contains("maxFiles"));
        assert!(json_content.contains("suppressStdout"));
    }

    #[test]
    fn test_distillation_config_defaults() {
        let config = DistillationConfig::default();
        assert!(config.provider_id.is_none());
        assert!(config.model.is_none());
    }

    #[test]
    fn test_distillation_config_in_execution_settings() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        let mut settings = AppSettings::default();
        settings.execution.distillation = DistillationConfig {
            provider_id: Some("ollama".to_string()),
            model: Some("llama3".to_string()),
            ..Default::default()
        };
        service.save(&settings).unwrap();

        service.invalidate_cache();
        let loaded = service.get_execution_settings().unwrap();
        assert_eq!(loaded.distillation.provider_id.as_deref(), Some("ollama"));
        assert_eq!(loaded.distillation.model.as_deref(), Some("llama3"));
    }

    #[test]
    fn orchestrator_reads_legacy_max_tokens_as_output_tokens() {
        let json = r#"{
            "providerId": "p",
            "model": "m",
            "temperature": 0.7,
            "maxInputTokens": 123456,
            "maxTokens": 7777,
            "thinkingEnabled": true
        }"#;

        let config: OrchestratorConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.max_input_tokens, Some(123456));
        assert!(config.max_input_tokens_explicit());
        assert_eq!(config.max_tokens, 7777);
    }

    #[test]
    fn orchestrator_defaults_to_simplified_token_limits() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.max_input_tokens, None);
        assert_eq!(
            config.effective_max_input_tokens(),
            DEFAULT_MAX_INPUT_TOKENS
        );
        assert!(!config.max_input_tokens_explicit());
        assert_eq!(config.max_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn test_distillation_config_absent_in_json() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        let json = r#"{ "execution": { "maxParallelAgents": 3 } }"#;
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("settings.json"), json).unwrap();

        service.invalidate_cache();
        let loaded = service.get_execution_settings().unwrap();
        assert!(loaded.distillation.provider_id.is_none());
        assert!(loaded.distillation.model.is_none());
    }

    /// Regression: saving typed AppSettings must preserve top-level keys
    /// that the typed struct doesn't model. The `embeddings` section is
    /// written by EmbeddingService; other services must not strip it.
    #[test]
    fn save_preserves_unknown_top_level_keys() {
        let dir = tempdir().unwrap();
        let service = SettingsService::from_vault_dir(dir.path().to_path_buf());

        let initial_json = r#"{
  "tools": { "fileTools": true, "offloadLargeResults": true },
  "embeddings": {
    "backend": "ollama",
    "dimensions": 1024,
    "ollama": { "base_url": "http://localhost:11434", "model": "snowflake-arctic-embed" }
  },
  "thirdParty": { "anything": "preserved" }
}"#;
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("settings.json"), initial_json).unwrap();

        // Save via typed API — this is what update_tool_settings, the
        // wizard, etc. call. Before the fix this wiped the file wholesale.
        service.invalidate_cache();
        let mut settings = service.load().unwrap();
        settings.tools.file_tools = true;
        service.save(&settings).unwrap();

        // Reread raw so we can assert unknown keys survived.
        let raw = fs::read_to_string(config_dir.join("settings.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed["embeddings"]["backend"].as_str(),
            Some("ollama"),
            "embeddings.backend must survive typed save"
        );
        assert_eq!(
            parsed["embeddings"]["dimensions"].as_u64(),
            Some(1024),
            "embeddings.dimensions must survive typed save"
        );
        assert_eq!(
            parsed["embeddings"]["ollama"]["model"].as_str(),
            Some("snowflake-arctic-embed"),
            "embeddings.ollama.model must survive typed save"
        );
        assert_eq!(
            parsed["thirdParty"]["anything"].as_str(),
            Some("preserved"),
            "arbitrary unknown keys must survive typed save"
        );
        assert_eq!(
            parsed["tools"]["fileTools"].as_bool(),
            Some(true),
            "typed field update must still take effect"
        );
    }
}

#[cfg(test)]
mod memory_settings_tests {
    use super::*;

    #[test]
    fn default_conflict_resolver_interval_is_24() {
        let m = MemorySettings::default();
        assert_eq!(m.conflict_resolver_interval_hours, 24);
    }

    #[test]
    fn memory_settings_deserializes_partial() {
        let json = r#"{"conflictResolverIntervalHours": 6}"#;
        let m: MemorySettings = serde_json::from_str(json).unwrap();
        assert_eq!(m.conflict_resolver_interval_hours, 6);
    }
}

#[cfg(test)]
mod network_settings_tests {
    use super::*;

    #[test]
    fn defaults_have_expose_to_lan_true() {
        let s = AppSettings::default();
        assert!(s.network.expose_to_lan);
        assert_eq!(s.network.advanced.http_port, 18791);
    }

    #[test]
    fn old_settings_without_network_block_still_parses() {
        let json = r#"{
            "tools": {},
            "logs": {},
            "execution": {}
        }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert!(s.network.expose_to_lan);
    }

    #[test]
    fn explicit_off_round_trips() {
        let json = r#"{ "network": { "exposeToLan": false } }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert!(!s.network.expose_to_lan);
    }
}
