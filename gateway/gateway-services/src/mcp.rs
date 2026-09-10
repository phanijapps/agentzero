//! # MCP Service
//!
//! Service for managing MCP server configurations.

use agent_primitives::vault_paths::SharedVaultPaths;
use agent_runtime::McpServerConfig;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// MCP service for loading and managing MCP server configurations.
pub struct McpService {
    paths: SharedVaultPaths,
    cache: RwLock<Option<Vec<McpServerConfig>>>,
    oauth_lock: Mutex<()>,
}

/// Failure reason when replacing an MCP configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpUpdateError {
    /// The requested existing MCP record is absent.
    NotFound { id: String },
    /// A different MCP record already owns the requested replacement ID.
    DuplicateId { id: String },
    /// Reading, persisting, or securely clearing state failed.
    Internal(String),
}

impl std::fmt::Display for McpUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { id } => write!(f, "MCP server not found: {id}"),
            Self::DuplicateId { id } => write!(f, "MCP server with ID '{id}' already exists"),
            Self::Internal(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for McpUpdateError {}

/// Summary of an MCP server for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerSummary {
    /// Server ID
    pub id: String,
    /// Server name
    pub name: String,
    /// Server description
    pub description: String,
    /// Transport type (stdio, http, sse, streamable-http)
    #[serde(rename = "type")]
    pub transport_type: String,
    /// Whether the server is enabled
    pub enabled: bool,
    /// Non-secret auth status for UI/listing surfaces.
    #[serde(rename = "authStatus", skip_serializing_if = "Option::is_none")]
    pub auth_status: Option<String>,
}

/// Safe reason why a requested dynamic MCP could not be mounted.
///
/// Deliberately excludes the requested identifier and underlying error text so
/// callers can make an audit record without exposing untrusted values or
/// configuration details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCapabilityRejection {
    UnknownId,
    Deleted,
    Disabled,
    OAuthUnavailable,
}

impl McpCapabilityRejection {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownId => "unknown_id",
            Self::Deleted => "deleted",
            Self::Disabled => "disabled",
            Self::OAuthUnavailable => "oauth_unavailable",
        }
    }
}

/// Result of resolving untrusted dynamic MCP IDs into safe runtime IDs.
///
/// `effective_ids` contains canonical configured IDs only. Rejections are
/// countable categories rather than echoes of the raw requested values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpCapabilityResolution {
    /// Canonical IDs validated either against current configuration or the
    /// host-owned catalog snapshot used to create the assignment.
    pub canonical_requested_ids: Vec<String>,
    pub effective_ids: Vec<String>,
    pub rejections: Vec<McpCapabilityRejection>,
}

/// Non-secret OAuth connection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthStatus {
    /// Server has no OAuth metadata.
    NotConfigured,
    /// Server has OAuth metadata but no usable token.
    NotConnected,
    /// Server has a non-expired access token.
    Connected,
    /// Token material exists but cannot be used.
    ReauthRequired,
}

impl McpOAuthStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::NotConnected => "not_connected",
            Self::Connected => "connected",
            Self::ReauthRequired => "reauth_required",
        }
    }
}

/// Secret OAuth token record. Never serialize this through config/list/get
/// endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthTokenRecord {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub resource: String,
    pub token_endpoint: String,
}

/// Pending OAuth state. Contains PKCE verifier and must be secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthPendingRecord {
    pub mcp_id: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub resource: String,
    pub expires_at_unix: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub token_endpoint: String,
}

impl McpService {
    /// Create a new MCP service.
    ///
    /// The paths should contain the z-Bot configuration directory
    /// (e.g., ~/Documents/zbot). The service will look for mcp-servers.json
    /// in the config subdirectory.
    pub fn new(paths: SharedVaultPaths) -> Self {
        Self {
            paths,
            cache: RwLock::new(None),
            oauth_lock: Mutex::new(()),
        }
    }

    /// Get the config file path.
    pub fn config_path(&self) -> PathBuf {
        self.paths.mcps()
    }

    /// Secret token-store path. Kept outside `mcp-servers.json`.
    pub fn oauth_tokens_path(&self) -> PathBuf {
        self.paths.mcp_oauth_tokens()
    }

    /// Secret pending-state path. Kept outside `mcp-servers.json`.
    pub fn oauth_pending_path(&self) -> PathBuf {
        self.paths.mcp_oauth_pending()
    }

    /// Invalidate the cache, forcing next read to go to disk.
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            *cache = None;
        }
    }

    /// Read configs from disk (bypasses cache).
    fn list_from_disk(&self) -> Result<Vec<McpServerConfig>, String> {
        if !self.config_path().exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(self.config_path())
            .map_err(|e| format!("Failed to read mcp-servers.json: {}", e))?;

        let configs: Vec<McpServerConfig> = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse mcp-servers.json: {}", e))?;

        validate_oauth_configs(&configs)?;

        Ok(configs)
    }

    /// List all MCP server configurations (cached).
    pub fn list(&self) -> Result<Vec<McpServerConfig>, String> {
        // Check cache first
        if let Ok(cache) = self.cache.read() {
            if let Some(configs) = cache.as_ref() {
                return Ok(configs.clone());
            }
        }

        // Cache miss: read from disk
        let configs = self.list_from_disk()?;

        // Update cache
        if let Ok(mut cache) = self.cache.write() {
            *cache = Some(configs.clone());
        }

        Ok(configs)
    }

    /// List MCP server summaries (lightweight for UI).
    pub fn list_summaries(&self) -> Result<Vec<McpServerSummary>, String> {
        let configs = self.list()?;
        Ok(configs
            .into_iter()
            .map(|c| self.config_to_summary(&c))
            .collect())
    }

    /// Get a specific MCP server configuration by ID.
    pub fn get(&self, id: &str) -> Result<McpServerConfig, String> {
        let configs = self.list()?;

        configs
            .into_iter()
            .find(|c| c.id() == id)
            .ok_or_else(|| format!("MCP server not found: {}", id))
    }

    /// Get multiple MCP server configurations by ID or display name.
    ///
    /// Returns only the configs that exist and are enabled. ID matches are the
    /// canonical path, but name matches keep older/manual agent configs working.
    /// Missing or disabled configs are skipped with a warning.
    pub fn get_multiple(&self, ids: &[String]) -> Vec<McpServerConfig> {
        let Ok(configs) = self.list() else {
            return vec![];
        };

        let mut matched = Vec::new();
        for requested_id in ids {
            // Exact canonical identity always wins. Only use a display-name
            // alias when no record owns the requested canonical ID; this
            // preserves legacy static aliases without allowing an aliasing
            // server to join an exact dynamic assignment.
            if let Some(exact) = configs.iter().find(|config| config.id() == *requested_id) {
                if exact.enabled()
                    && !matched
                        .iter()
                        .any(|config: &McpServerConfig| config.id() == exact.id())
                {
                    matched.push(exact.clone());
                }
                continue;
            }

            for alias in configs
                .iter()
                .filter(|config| config.enabled() && config.name() == requested_id)
            {
                if !matched
                    .iter()
                    .any(|config: &McpServerConfig| config.id() == alias.id())
                {
                    matched.push(alias.clone());
                }
            }
        }

        for requested_id in ids {
            if !matched.iter().any(|c| mcp_ref_matches(c, requested_id)) {
                tracing::warn!(
                    mcp_ref = %requested_id,
                    "Configured MCP reference did not match an enabled server by ID or name"
                );
            }
        }

        matched
    }

    /// Get enabled MCP configs for runtime startup, injecting bearer tokens for
    /// connected OAuth servers without persisting those headers to `mcp-servers.json`.
    pub fn get_multiple_for_runtime(&self, ids: &[String]) -> Vec<McpServerConfig> {
        self.get_multiple(ids)
            .into_iter()
            .filter_map(|config| match self.runtime_auth_status(&config) {
                McpOAuthStatus::NotConfigured | McpOAuthStatus::Connected => {
                    Some(self.with_runtime_auth(config))
                }
                status => {
                    tracing::warn!(
                        mcp_id = %config.id(),
                        auth_status = status.as_str(),
                        "Skipping OAuth MCP server because it is not connected"
                    );
                    None
                }
            })
            .collect()
    }

    /// Resolve canonical dynamic MCP IDs for an executor without exposing
    /// configuration secrets or invalid requested IDs to the caller.
    ///
    /// Unlike `get_multiple_for_runtime`, this accepts only exact canonical
    /// IDs. Display-name aliases are retained exclusively for legacy static
    /// agent configuration compatibility.
    pub fn resolve_dynamic_runtime_ids(
        &self,
        requested_ids: &[String],
    ) -> Result<McpCapabilityResolution, String> {
        self.resolve_dynamic_runtime_ids_with_catalog(requested_ids, &[])
    }

    /// Resolve dynamic IDs while retaining provenance from the safe catalog
    /// snapshot that produced the assignment. A missing ID present in that
    /// snapshot is `deleted`; a value absent from both sources is untrusted and
    /// remains `unknown_id`.
    pub fn resolve_dynamic_runtime_ids_with_catalog(
        &self,
        requested_ids: &[String],
        known_catalog_ids: &[String],
    ) -> Result<McpCapabilityResolution, String> {
        let configs = self.list()?;
        let mut result = McpCapabilityResolution::default();
        let known_catalog_ids = known_catalog_ids.iter().collect::<HashSet<_>>();
        let mut seen_requested = HashSet::new();

        for requested_id in requested_ids {
            if !seen_requested.insert(requested_id) {
                continue;
            }
            let Some(config) = configs.iter().find(|config| config.id() == *requested_id) else {
                if known_catalog_ids.contains(requested_id) {
                    result
                        .canonical_requested_ids
                        .push(requested_id.to_string());
                    result.rejections.push(McpCapabilityRejection::Deleted);
                } else {
                    result.rejections.push(McpCapabilityRejection::UnknownId);
                }
                continue;
            };
            let canonical_id = config.id();
            result.canonical_requested_ids.push(canonical_id.clone());
            if !config.enabled() {
                result.rejections.push(McpCapabilityRejection::Disabled);
                continue;
            }
            if !matches!(
                self.runtime_auth_status(config),
                McpOAuthStatus::NotConfigured | McpOAuthStatus::Connected
            ) {
                result
                    .rejections
                    .push(McpCapabilityRejection::OAuthUnavailable);
                continue;
            }

            if !result.effective_ids.contains(&canonical_id) {
                result.effective_ids.push(canonical_id);
            }
        }

        Ok(result)
    }

    /// Get one MCP config for connection testing/runtime use, injecting OAuth
    /// bearer headers only when the stored token is connected and resource-bound
    /// to the current config.
    pub fn get_for_runtime(&self, id: &str) -> Result<McpServerConfig, String> {
        let config = self.get(id)?;
        match self.runtime_auth_status(&config) {
            McpOAuthStatus::NotConfigured | McpOAuthStatus::Connected => {
                Ok(self.with_runtime_auth(config))
            }
            status => Err(format!(
                "MCP server requires OAuth authentication before use (status: {})",
                status.as_str()
            )),
        }
    }

    /// Save or replace OAuth token material for one MCP server.
    pub fn save_oauth_token(
        &self,
        mcp_id: &str,
        record: McpOAuthTokenRecord,
    ) -> Result<(), String> {
        let _guard = self
            .oauth_lock
            .lock()
            .map_err(|_| "OAuth secret store lock poisoned".to_string())?;
        let mut records = self.read_secret_map::<McpOAuthTokenRecord>(&self.oauth_tokens_path())?;
        records.insert(mcp_id.to_string(), record);
        self.write_secret_map(&self.oauth_tokens_path(), &records)
    }

    /// Read OAuth token material for one MCP server.
    pub fn get_oauth_token(&self, mcp_id: &str) -> Result<Option<McpOAuthTokenRecord>, String> {
        let records = self.read_secret_map::<McpOAuthTokenRecord>(&self.oauth_tokens_path())?;
        Ok(records.get(mcp_id).cloned())
    }

    /// Save or replace pending OAuth state.
    pub fn save_oauth_pending(
        &self,
        state: &str,
        record: McpOAuthPendingRecord,
    ) -> Result<(), String> {
        let _guard = self
            .oauth_lock
            .lock()
            .map_err(|_| "OAuth secret store lock poisoned".to_string())?;
        let mut records =
            self.read_secret_map::<McpOAuthPendingRecord>(&self.oauth_pending_path())?;
        records.insert(state.to_string(), record);
        self.write_secret_map(&self.oauth_pending_path(), &records)
    }

    /// Remove pending OAuth state by state ID.
    pub fn remove_oauth_pending(
        &self,
        state: &str,
    ) -> Result<Option<McpOAuthPendingRecord>, String> {
        let _guard = self
            .oauth_lock
            .lock()
            .map_err(|_| "OAuth secret store lock poisoned".to_string())?;
        let mut records =
            self.read_secret_map::<McpOAuthPendingRecord>(&self.oauth_pending_path())?;
        let removed = records.remove(state);
        self.write_secret_map(&self.oauth_pending_path(), &records)?;
        Ok(removed)
    }

    /// Remove OAuth token and pending state for one MCP server.
    pub fn disconnect_oauth(&self, mcp_id: &str) -> Result<(), String> {
        let _guard = self
            .oauth_lock
            .lock()
            .map_err(|_| "OAuth secret store lock poisoned".to_string())?;
        let mut tokens = self.read_secret_map::<McpOAuthTokenRecord>(&self.oauth_tokens_path())?;
        tokens.remove(mcp_id);
        self.write_secret_map(&self.oauth_tokens_path(), &tokens)?;

        let mut pending =
            self.read_secret_map::<McpOAuthPendingRecord>(&self.oauth_pending_path())?;
        pending.retain(|_, record| record.mcp_id != mcp_id);
        self.write_secret_map(&self.oauth_pending_path(), &pending)
    }

    /// Report non-secret OAuth status for one MCP server.
    pub fn oauth_status(&self, mcp_id: &str) -> McpOAuthStatus {
        let Ok(config) = self.get(mcp_id) else {
            return McpOAuthStatus::NotConfigured;
        };
        if !config.is_oauth() {
            return McpOAuthStatus::NotConfigured;
        }

        let records = match self.read_secret_map::<McpOAuthTokenRecord>(&self.oauth_tokens_path()) {
            Ok(records) => records,
            Err(_) if self.oauth_tokens_path().exists() => return McpOAuthStatus::ReauthRequired,
            Err(_) => return McpOAuthStatus::NotConnected,
        };

        let Some(record) = records.get(mcp_id) else {
            return McpOAuthStatus::NotConnected;
        };
        if record.access_token.trim().is_empty() {
            return McpOAuthStatus::ReauthRequired;
        }
        if let Some(current_resource) = config_resource_url(&config) {
            if record.resource != current_resource {
                return McpOAuthStatus::ReauthRequired;
            }
        }
        if record
            .expires_at_unix
            .is_some_and(|expires_at| expires_at <= Utc::now().timestamp())
        {
            return McpOAuthStatus::ReauthRequired;
        }
        McpOAuthStatus::Connected
    }

    fn read_secret_map<T>(&self, path: &PathBuf) -> Result<HashMap<String, T>, String>
    where
        T: for<'de> Deserialize<'de>,
    {
        if !path.exists() {
            return Ok(HashMap::new());
        }
        ensure_secret_permissions(path)?;
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read OAuth secret store: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse OAuth secret store: {}", e))
    }

    fn write_secret_map<T>(
        &self,
        path: &PathBuf,
        records: &HashMap<String, T>,
    ) -> Result<(), String>
    where
        T: Serialize,
    {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create OAuth secret directory: {}", e))?;
        }
        if path.exists() {
            ensure_secret_permissions(path)?;
        }
        let content = serde_json::to_vec_pretty(records)
            .map_err(|e| format!("Failed to serialize OAuth secret store: {}", e))?;
        write_secret_file_atomic(path, &content)
    }

    /// Save MCP server configurations to disk and update cache.
    pub fn save(&self, configs: &[McpServerConfig]) -> Result<(), String> {
        validate_oauth_configs(configs)?;

        // Ensure parent directory exists
        if let Some(parent) = self.config_path().parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        let content = serde_json::to_string_pretty(configs)
            .map_err(|e| format!("Failed to serialize mcp-servers.json: {}", e))?;

        fs::write(self.config_path(), content)
            .map_err(|e| format!("Failed to write mcp-servers.json: {}", e))?;

        // Update cache with the data we just wrote
        if let Ok(mut cache) = self.cache.write() {
            *cache = Some(configs.to_vec());
        }

        Ok(())
    }

    /// Add a new MCP server configuration.
    pub fn add(&self, config: McpServerConfig) -> Result<(), String> {
        let mut configs = self.list().unwrap_or_default();

        // Check for duplicate ID
        let new_id = config.id();
        if configs.iter().any(|c| c.id() == new_id) {
            return Err(format!("MCP server with ID '{}' already exists", new_id));
        }

        configs.push(config);
        self.save(&configs)
    }

    /// Update an existing MCP server configuration.
    pub fn update(&self, id: &str, config: McpServerConfig) -> Result<(), McpUpdateError> {
        let mut configs = self.list().map_err(McpUpdateError::Internal)?;

        let index = configs
            .iter()
            .position(|c| c.id() == id)
            .ok_or_else(|| McpUpdateError::NotFound { id: id.to_string() })?;

        let replacement_id = config.id();
        if configs
            .iter()
            .enumerate()
            .any(|(other_index, existing)| other_index != index && existing.id() == replacement_id)
        {
            return Err(McpUpdateError::DuplicateId {
                id: replacement_id.to_string(),
            });
        }

        if oauth_identity_changed(&configs[index], &config) {
            self.disconnect_oauth(id)
                .map_err(McpUpdateError::Internal)?;
        }
        configs[index] = config;
        self.save(&configs).map_err(McpUpdateError::Internal)
    }

    /// Delete an MCP server configuration.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut configs = self.list()?;

        let index = configs
            .iter()
            .position(|c| c.id() == id)
            .ok_or_else(|| format!("MCP server not found: {}", id))?;

        configs.remove(index);
        self.save(&configs)
    }

    /// Convert a config to a summary.
    fn config_to_summary(&self, config: &McpServerConfig) -> McpServerSummary {
        match config {
            McpServerConfig::Stdio {
                id,
                name,
                description,
                enabled,
                ..
            } => McpServerSummary {
                id: id.clone().unwrap_or_else(|| name.clone()),
                name: name.clone(),
                description: description.clone(),
                transport_type: "stdio".to_string(),
                enabled: *enabled,
                auth_status: None,
            },
            McpServerConfig::Http {
                id,
                name,
                description,
                enabled,
                ..
            } => McpServerSummary {
                id: id.clone().unwrap_or_else(|| name.clone()),
                name: name.clone(),
                description: description.clone(),
                transport_type: "http".to_string(),
                enabled: *enabled,
                auth_status: self.auth_status_for_config(config),
            },
            McpServerConfig::Sse {
                id,
                name,
                description,
                enabled,
                ..
            } => McpServerSummary {
                id: id.clone().unwrap_or_else(|| name.clone()),
                name: name.clone(),
                description: description.clone(),
                transport_type: "sse".to_string(),
                enabled: *enabled,
                auth_status: self.auth_status_for_config(config),
            },
            McpServerConfig::StreamableHttp {
                id,
                name,
                description,
                enabled,
                ..
            } => McpServerSummary {
                id: id.clone().unwrap_or_else(|| name.clone()),
                name: name.clone(),
                description: description.clone(),
                transport_type: "streamable-http".to_string(),
                enabled: *enabled,
                auth_status: self.auth_status_for_config(config),
            },
        }
    }

    fn auth_status_for_config(&self, config: &McpServerConfig) -> Option<String> {
        config
            .is_oauth()
            .then(|| self.oauth_status(&config.id()).as_str().to_string())
    }

    fn with_runtime_auth(&self, mut config: McpServerConfig) -> McpServerConfig {
        if !config.is_oauth() || self.oauth_status(&config.id()) != McpOAuthStatus::Connected {
            return config;
        }
        let Ok(Some(token)) = self.get_oauth_token(&config.id()) else {
            return config;
        };
        let Some(resource) = config_resource_url(&config) else {
            return config;
        };
        if token.resource != resource {
            return config;
        }
        insert_runtime_authorization_header(&mut config, token.access_token);
        config
    }

    fn runtime_auth_status(&self, config: &McpServerConfig) -> McpOAuthStatus {
        if config.is_oauth() {
            self.oauth_status(&config.id())
        } else {
            McpOAuthStatus::NotConfigured
        }
    }
}

fn validate_oauth_configs(configs: &[McpServerConfig]) -> Result<(), String> {
    for config in configs {
        if config.is_oauth() && config.has_authorization_header() {
            return Err(format!(
                "OAuth MCP server '{}' cannot persist Authorization headers; connect with OAuth instead",
                config.id()
            ));
        }
    }
    Ok(())
}

fn oauth_identity_changed(old: &McpServerConfig, new: &McpServerConfig) -> bool {
    old.is_oauth()
        && (old.id() != new.id()
            || config_resource_url(old) != config_resource_url(new)
            || old.auth() != new.auth())
}

fn config_resource_url(config: &McpServerConfig) -> Option<String> {
    match config {
        McpServerConfig::Http { url, .. }
        | McpServerConfig::Sse { url, .. }
        | McpServerConfig::StreamableHttp { url, .. } => Some(url.clone()),
        McpServerConfig::Stdio { .. } => None,
    }
}

fn insert_runtime_authorization_header(config: &mut McpServerConfig, access_token: String) {
    let headers = match config {
        McpServerConfig::Stdio { .. } => return,
        McpServerConfig::Http { headers, .. }
        | McpServerConfig::Sse { headers, .. }
        | McpServerConfig::StreamableHttp { headers, .. } => headers,
    };
    let headers = headers.get_or_insert_with(HashMap::new);
    headers.insert(
        "Authorization".to_string(),
        format!("Bearer {access_token}"),
    );
}

#[cfg(unix)]
fn ensure_secret_permissions(path: &PathBuf) -> Result<(), String> {
    let mode = fs::metadata(path)
        .map_err(|e| format!("Failed to inspect OAuth secret store permissions: {}", e))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "OAuth secret store has unsafe permissions {:o}; expected 600",
            mode
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_secret_permissions(_path: &PathBuf) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn write_secret_file_atomic(path: &PathBuf, content: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "tmp-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("Failed to create OAuth secret temp file: {}", e))?;
    file.write_all(content)
        .map_err(|e| format!("Failed to write OAuth secret temp file: {}", e))?;
    file.sync_all()
        .map_err(|e| format!("Failed to sync OAuth secret temp file: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to replace OAuth secret store: {}", e)
    })?;
    ensure_secret_permissions(path)
}

#[cfg(not(unix))]
fn write_secret_file_atomic(path: &PathBuf, content: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "tmp-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")
    ));
    fs::write(&tmp, content)
        .map_err(|e| format!("Failed to write OAuth secret temp file: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to replace OAuth secret store: {}", e)
    })
}

fn mcp_ref_matches(config: &McpServerConfig, requested: &str) -> bool {
    config.id() == requested || config.name() == requested
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::vault_paths::VaultPaths;
    use agent_runtime::{McpAuthConfig, McpAuthType};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    fn service() -> (tempfile::TempDir, McpService) {
        let dir = tempdir().unwrap();
        let paths = Arc::new(VaultPaths::new(dir.path().to_path_buf()));
        let service = McpService::new(paths);
        (dir, service)
    }

    fn stdio(id: Option<&str>, name: &str, enabled: bool) -> McpServerConfig {
        McpServerConfig::Stdio {
            id: id.map(str::to_string),
            name: name.to_string(),
            description: "desc".to_string(),
            command: "npx".to_string(),
            args: vec!["server".to_string()],
            env: None,
            enabled,
            validated: None,
        }
    }

    fn oauth_streamable(id: &str, enabled: bool) -> McpServerConfig {
        McpServerConfig::StreamableHttp {
            id: Some(id.to_string()),
            name: id.to_string(),
            description: "oauth".to_string(),
            url: "https://example.com/mcp".to_string(),
            headers: None,
            auth: Some(McpAuthConfig {
                auth_type: McpAuthType::OAuth2,
                client_id: None,
                scopes: vec![],
            }),
            enabled,
            validated: None,
        }
    }

    #[test]
    fn dynamic_runtime_resolution_requires_enabled_canonical_ready_ids() {
        let (_dir, service) = service();
        service
            .save(&[
                stdio(Some("ready"), "Ready", true),
                stdio(Some("disabled"), "Disabled", false),
                oauth_streamable("oauth", true),
            ])
            .unwrap();

        let resolution = service
            .resolve_dynamic_runtime_ids(&[
                "ready".to_string(),
                "Ready".to_string(),
                "disabled".to_string(),
                "oauth".to_string(),
                "missing".to_string(),
                "ready".to_string(),
            ])
            .unwrap();

        assert_eq!(resolution.effective_ids, vec!["ready"]);
        assert_eq!(
            resolution.canonical_requested_ids,
            vec!["ready", "disabled", "oauth"]
        );
        assert_eq!(
            resolution.rejections,
            vec![
                McpCapabilityRejection::UnknownId,
                McpCapabilityRejection::Disabled,
                McpCapabilityRejection::OAuthUnavailable,
                McpCapabilityRejection::UnknownId,
            ]
        );
    }

    #[test]
    fn dynamic_runtime_resolution_distinguishes_deleted_from_unknown_ids() {
        let (_dir, service) = service();

        let resolution = service
            .resolve_dynamic_runtime_ids_with_catalog(
                &["deleted-mcp".to_string(), "never-known".to_string()],
                &["deleted-mcp".to_string()],
            )
            .unwrap();

        assert_eq!(resolution.canonical_requested_ids, vec!["deleted-mcp"]);
        assert!(resolution.effective_ids.is_empty());
        assert_eq!(
            resolution.rejections,
            vec![
                McpCapabilityRejection::Deleted,
                McpCapabilityRejection::UnknownId,
            ]
        );
    }

    fn token_record(access_token: &str, expires_at_unix: Option<i64>) -> McpOAuthTokenRecord {
        McpOAuthTokenRecord {
            access_token: access_token.to_string(),
            refresh_token: None,
            expires_at_unix,
            client_id: None,
            client_secret: None,
            resource: "https://example.com/mcp".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
        }
    }

    #[test]
    fn get_multiple_matches_enabled_server_by_id() {
        let (_dir, service) = service();
        service
            .save(&[
                stdio(Some("brave-search"), "Brave Search", true),
                stdio(Some("time"), "Time", true),
            ])
            .unwrap();

        let found = service.get_multiple(&["brave-search".to_string()]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "brave-search");
    }

    #[test]
    fn get_multiple_matches_enabled_server_by_display_name() {
        let (_dir, service) = service();
        service
            .save(&[stdio(Some("brave-search"), "Brave Search", true)])
            .unwrap();

        let found = service.get_multiple(&["Brave Search".to_string()]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "brave-search");
    }

    #[test]
    fn get_multiple_exact_id_wins_over_colliding_display_name() {
        let (_dir, service) = service();
        service
            .save(&[
                stdio(Some("trusted"), "Trusted", true),
                stdio(Some("alias"), "trusted", true),
            ])
            .unwrap();

        let found = service.get_multiple(&["trusted".to_string()]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "trusted");
    }

    #[test]
    fn get_multiple_skips_disabled_name_match() {
        let (_dir, service) = service();
        service
            .save(&[stdio(Some("brave-search"), "Brave Search", false)])
            .unwrap();

        let found = service.get_multiple(&["Brave Search".to_string()]);

        assert!(found.is_empty());
    }

    #[test]
    fn update_rejects_replacement_id_owned_by_another_mcp_without_mutation() {
        let (_dir, service) = service();
        service
            .save(&[
                stdio(Some("first"), "First", true),
                stdio(Some("second"), "Second", true),
            ])
            .unwrap();

        let result = service.update("first", stdio(Some("second"), "First", true));

        assert_eq!(
            result,
            Err(McpUpdateError::DuplicateId {
                id: "second".to_string()
            })
        );
        assert_eq!(service.get("first").unwrap().name(), "First");
        assert_eq!(service.get("second").unwrap().name(), "Second");
    }

    #[test]
    fn oauth_token_store_is_separate_from_mcps_config() {
        let (_dir, service) = service();
        service.save(&[oauth_streamable("rh", false)]).unwrap();

        service
            .save_oauth_token(
                "rh",
                McpOAuthTokenRecord {
                    access_token: "access-secret".to_string(),
                    refresh_token: Some("refresh-secret".to_string()),
                    expires_at_unix: Some(Utc::now().timestamp() + 3600),
                    client_id: Some("client".to_string()),
                    client_secret: Some("client-secret".to_string()),
                    resource: "https://example.com/mcp".to_string(),
                    token_endpoint: "https://example.com/token".to_string(),
                },
            )
            .unwrap();

        let mcps = fs::read_to_string(service.config_path()).unwrap();
        assert!(!mcps.contains("access-secret"));
        assert!(!mcps.contains("refresh-secret"));
        assert!(!mcps.contains("client-secret"));

        let tokens = fs::read_to_string(service.oauth_tokens_path()).unwrap();
        assert!(tokens.contains("access-secret"));
        assert_eq!(service.oauth_status("rh"), McpOAuthStatus::Connected);
    }

    #[test]
    fn oauth_disconnect_removes_one_server_only() {
        let (_dir, service) = service();
        service
            .save(&[
                oauth_streamable("one", false),
                oauth_streamable("two", false),
            ])
            .unwrap();

        for id in ["one", "two"] {
            service
                .save_oauth_token(
                    id,
                    token_record(&format!("{id}-access"), Some(Utc::now().timestamp() + 3600)),
                )
                .unwrap();
            service
                .save_oauth_pending(
                    &format!("{id}-state"),
                    McpOAuthPendingRecord {
                        mcp_id: id.to_string(),
                        code_verifier: "verifier".to_string(),
                        redirect_uri: "http://localhost/callback".to_string(),
                        resource: "https://example.com/mcp".to_string(),
                        expires_at_unix: Utc::now().timestamp() + 300,
                        client_id: Some("client".to_string()),
                        client_secret: None,
                        token_endpoint: "https://example.com/token".to_string(),
                    },
                )
                .unwrap();
        }

        service.disconnect_oauth("one").unwrap();

        assert_eq!(service.oauth_status("one"), McpOAuthStatus::NotConnected);
        assert_eq!(service.oauth_status("two"), McpOAuthStatus::Connected);
        assert!(service.remove_oauth_pending("one-state").unwrap().is_none());
        assert!(service.remove_oauth_pending("two-state").unwrap().is_some());
    }

    #[test]
    fn changing_oauth_mcp_id_clears_old_identity_oauth_state() {
        let (_dir, service) = service();
        service
            .save(&[
                oauth_streamable("old", false),
                oauth_streamable("other", false),
            ])
            .unwrap();

        for id in ["old", "other"] {
            service
                .save_oauth_token(
                    id,
                    token_record(&format!("{id}-access"), Some(Utc::now().timestamp() + 3600)),
                )
                .unwrap();
            service
                .save_oauth_pending(
                    &format!("{id}-state"),
                    McpOAuthPendingRecord {
                        mcp_id: id.to_string(),
                        code_verifier: "verifier".to_string(),
                        redirect_uri: "http://localhost/callback".to_string(),
                        resource: "https://example.com/mcp".to_string(),
                        expires_at_unix: Utc::now().timestamp() + 300,
                        client_id: Some("client".to_string()),
                        client_secret: None,
                        token_endpoint: "https://example.com/token".to_string(),
                    },
                )
                .unwrap();
        }

        service
            .update("old", oauth_streamable("renamed", false))
            .unwrap();

        let tokens = service
            .read_secret_map::<McpOAuthTokenRecord>(&service.oauth_tokens_path())
            .unwrap();
        assert!(!tokens.contains_key("old"));
        assert!(tokens.contains_key("other"));
        assert!(service.get("old").is_err());
        assert!(service.get("renamed").is_ok());
        assert!(service.remove_oauth_pending("old-state").unwrap().is_none());
        assert!(service
            .remove_oauth_pending("other-state")
            .unwrap()
            .is_some());
    }

    #[test]
    fn oauth_status_distinguishes_not_connected_connected_and_reauth() {
        let (_dir, service) = service();
        service.save(&[oauth_streamable("rh", false)]).unwrap();
        assert_eq!(service.oauth_status("rh"), McpOAuthStatus::NotConnected);

        service
            .save_oauth_token(
                "rh",
                token_record("access", Some(Utc::now().timestamp() + 3600)),
            )
            .unwrap();
        assert_eq!(service.oauth_status("rh"), McpOAuthStatus::Connected);

        service
            .save_oauth_token(
                "rh",
                token_record("access", Some(Utc::now().timestamp() - 1)),
            )
            .unwrap();
        assert_eq!(service.oauth_status("rh"), McpOAuthStatus::ReauthRequired);
    }

    #[test]
    fn runtime_config_injects_authorization_only_for_connected_oauth() {
        let (_dir, service) = service();
        service.save(&[oauth_streamable("rh", true)]).unwrap();
        service
            .save_oauth_token(
                "rh",
                token_record("access", Some(Utc::now().timestamp() + 3600)),
            )
            .unwrap();

        let configs = service.get_multiple_for_runtime(&["rh".to_string()]);

        assert_eq!(configs.len(), 1);
        assert!(configs[0].has_authorization_header());
        let persisted = fs::read_to_string(service.config_path()).unwrap();
        assert!(!persisted.contains("Authorization"));
        assert!(!persisted.contains("access"));
    }

    #[test]
    fn runtime_config_does_not_inject_expired_oauth_token() {
        let (_dir, service) = service();
        service.save(&[oauth_streamable("rh", true)]).unwrap();
        service
            .save_oauth_token(
                "rh",
                token_record("access", Some(Utc::now().timestamp() - 1)),
            )
            .unwrap();

        let configs = service.get_multiple_for_runtime(&["rh".to_string()]);

        assert!(configs.is_empty());
        assert_eq!(service.oauth_status("rh"), McpOAuthStatus::ReauthRequired);
        let err = service.get_for_runtime("rh").unwrap_err();
        assert!(err.contains("requires OAuth authentication"));
    }

    #[test]
    fn runtime_config_skips_unconnected_oauth_server() {
        let (_dir, service) = service();
        service.save(&[oauth_streamable("rh", true)]).unwrap();

        let configs = service.get_multiple_for_runtime(&["rh".to_string()]);

        assert!(configs.is_empty());
        let err = service.get_for_runtime("rh").unwrap_err();
        assert!(err.contains("status: not_connected"));
    }

    #[test]
    fn hand_edited_oauth_authorization_header_fails_closed_on_load() {
        let (_dir, service) = service();
        if let Some(parent) = service.config_path().parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            service.config_path(),
            r#"[{
              "type": "streamable-http",
              "id": "bad",
              "name": "Bad",
              "description": "bad",
              "url": "https://example.com/mcp",
              "headers": { "Authorization": "Bearer persisted" },
              "auth": { "type": "oauth2" },
              "enabled": true
            }]"#,
        )
        .unwrap();

        let err = service.list().unwrap_err();

        assert!(err.contains("cannot persist Authorization headers"));
    }

    #[test]
    fn runtime_config_does_not_inject_token_after_resource_change() {
        let (_dir, service) = service();
        service.save(&[oauth_streamable("rh", true)]).unwrap();
        service
            .save_oauth_token(
                "rh",
                token_record("access", Some(Utc::now().timestamp() + 3600)),
            )
            .unwrap();

        let changed = McpServerConfig::StreamableHttp {
            id: Some("rh".to_string()),
            name: "rh".to_string(),
            description: "oauth".to_string(),
            url: "https://attacker.example/mcp".to_string(),
            headers: None,
            auth: Some(McpAuthConfig {
                auth_type: McpAuthType::OAuth2,
                client_id: None,
                scopes: vec![],
            }),
            enabled: true,
            validated: None,
        };
        service.update("rh", changed).unwrap();

        let err = service.get_for_runtime("rh").unwrap_err();

        assert!(err.contains("requires OAuth authentication"));
        assert_eq!(service.oauth_status("rh"), McpOAuthStatus::NotConnected);
    }

    #[cfg(unix)]
    #[test]
    fn oauth_secret_store_rejects_broad_permissions() {
        let (_dir, service) = service();
        if let Some(parent) = service.oauth_tokens_path().parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let path = service.oauth_tokens_path();
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o644)
            .open(&path)
            .unwrap();
        file.write_all(b"{}").unwrap();
        drop(file);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let err = service
            .save_oauth_token("rh", token_record("access", None))
            .unwrap_err();

        assert!(err.contains("unsafe permissions"));
    }
}
