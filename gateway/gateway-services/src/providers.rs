// ============================================================================
// PROVIDERS SERVICE
// LLM provider management for the gateway
// ============================================================================

use crate::paths::SharedVaultPaths;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

static PROVIDER_MUTATION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
pub const OLLAMA_CLOUD_PENDING_MARKER: &str = ".ollama-cloud-commissioning-pending";

pub fn provider_mutation_lock() -> &'static tokio::sync::Mutex<()> {
    PROVIDER_MUTATION_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn ollama_cloud_commissioning_pending(paths: &crate::paths::VaultPaths) -> bool {
    match std::fs::symlink_metadata(paths.config_dir().join(OLLAMA_CLOUD_PENDING_MARKER)) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(rename = "apiKey")]
    #[serde(default)]
    pub api_key: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub models: Vec<String>,
    #[serde(rename = "embeddingModels", skip_serializing_if = "Option::is_none")]
    pub embedding_models: Option<Vec<String>>,
    #[serde(
        rename = "embeddingDimensions",
        skip_serializing_if = "Option::is_none"
    )]
    pub embedding_dimensions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(rename = "isDefault", default)]
    pub is_default: bool,
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Maximum concurrent LLM requests for this provider (default: 3).
    /// Set lower for rate-limited providers (e.g., 1 for free tiers).
    #[serde(
        rename = "maxConcurrentRequests",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_concurrent_requests: Option<u32>,
    /// Context window size in tokens. Overrides the hardcoded model lookup.
    /// Set this when using models not in the built-in lookup table.
    #[serde(rename = "contextWindow", skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Default model for this provider. Used when creating root or specialist agents
    /// that don't specify a model. Falls back to `models[0]` if not set.
    #[serde(rename = "defaultModel", skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Rate limiting configuration for this provider.
    #[serde(
        rename = "rateLimits",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub rate_limits: Option<RateLimits>,
    /// Enriched model configurations with capabilities and limits.
    #[serde(
        rename = "modelConfigs",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub model_configs: Option<HashMap<String, ModelConfig>>,
}

impl Provider {
    /// Get the default model for this provider.
    /// Priority: explicit `defaultModel` → first entry in `models` → `"gpt-4o"`.
    pub fn default_model(&self) -> &str {
        self.default_model
            .as_deref()
            .or_else(|| self.models.first().map(|s| s.as_str()))
            .unwrap_or("gpt-4o")
    }

    /// Get effective max_output for a model from model_configs.
    pub fn effective_max_output(&self, model_id: &str) -> Option<u64> {
        self.model_configs
            .as_ref()
            .and_then(|configs| configs.get(model_id))
            .and_then(|c| c.max_output)
    }

    /// Get effective max_input for a model from model_configs.
    pub fn effective_max_input(&self, model_id: &str) -> Option<u64> {
        self.model_configs
            .as_ref()
            .and_then(|configs| configs.get(model_id))
            .and_then(|c| c.max_input)
    }

    /// Get effective rate limits. Falls back to defaults if not set.
    pub fn effective_rate_limits(&self) -> RateLimits {
        self.rate_limits.clone().unwrap_or_default()
    }
}

fn validate_fixed_provider_origin(provider: &Provider) -> Result<(), String> {
    if provider.id.as_deref() == Some("provider-ollama-cloud")
        && provider.base_url != "https://ollama.com/v1"
    {
        return Err("Ollama Cloud endpoint must be https://ollama.com/v1".to_string());
    }
    Ok(())
}

fn scrub_secret(value: &str, secret: &str) -> String {
    if secret.is_empty() {
        value.to_string()
    } else {
        value.replace(secret, "[redacted]")
    }
}

/// Per-provider rate limit configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimits {
    /// Maximum requests per minute. Default: 60.
    #[serde(rename = "requestsPerMinute", default = "default_rpm")]
    pub requests_per_minute: u32,
    /// Maximum concurrent requests. Default: 3.
    #[serde(rename = "concurrentRequests", default = "default_concurrent")]
    pub concurrent_requests: u32,
}

fn default_rpm() -> u32 {
    30
}
fn default_concurrent() -> u32 {
    2
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            requests_per_minute: default_rpm(),
            concurrent_requests: default_concurrent(),
        }
    }
}

/// Per-model configuration with capabilities and token limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model capabilities (text, tools, vision, etc.)
    #[serde(default)]
    pub capabilities: crate::models::ModelCapabilities,
    /// Maximum input tokens.
    #[serde(rename = "maxInput", skip_serializing_if = "Option::is_none")]
    pub max_input: Option<u64>,
    /// Maximum output tokens.
    #[serde(rename = "maxOutput", skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Data source: "registry", "discovered", or "user".
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "registry".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderTestResult {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

// ============================================================================
// Service
// ============================================================================

pub struct ProviderService {
    paths: SharedVaultPaths,
    cache: RwLock<Option<Vec<Provider>>>,
}

impl ProviderService {
    pub fn new(paths: SharedVaultPaths) -> Self {
        Self {
            paths,
            cache: RwLock::new(None),
        }
    }

    /// Get the config file path.
    fn config_path(&self) -> PathBuf {
        self.paths.providers()
    }

    /// Read all providers from config file (bypasses cache).
    fn read_providers_from_disk(&self) -> Result<Vec<Provider>, String> {
        if !self.config_path().exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(self.config_path())
            .map_err(|e| format!("Failed to read providers config: {}", e))?;

        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse providers config: {}", e))
    }

    /// Write providers to config file and update cache.
    fn write_providers(&self, providers: &[Provider]) -> Result<(), String> {
        let content = serde_json::to_string_pretty(providers)
            .map_err(|e| format!("Failed to serialize providers: {}", e))?;

        fs::write(self.config_path(), content)
            .map_err(|e| format!("Failed to write providers config: {}", e))?;

        // Update cache with the data we just wrote
        if let Ok(mut cache) = self.cache.write() {
            *cache = Some(providers.to_vec());
        }

        Ok(())
    }

    /// Invalidate the cache, forcing next read to go to disk.
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            *cache = None;
        }
    }

    /// List all providers (cached).
    pub fn list(&self) -> Result<Vec<Provider>, String> {
        // Check cache first
        if let Ok(cache) = self.cache.read() {
            if let Some(providers) = cache.as_ref() {
                return Ok(providers.clone());
            }
        }

        // Cache miss: read from disk
        let providers = self.read_providers_from_disk()?;

        // Update cache
        if let Ok(mut cache) = self.cache.write() {
            *cache = Some(providers.clone());
        }

        Ok(providers)
    }

    /// Get a single provider by ID
    pub fn get(&self, id: &str) -> Result<Provider, String> {
        let providers = self.list()?;
        providers
            .into_iter()
            .find(|p| p.id.as_deref() == Some(id))
            .ok_or_else(|| format!("Provider not found: {}", id))
            .and_then(|provider| {
                validate_fixed_provider_origin(&provider)?;
                Ok(provider)
            })
    }

    /// Create a new provider
    pub fn create(&self, mut provider: Provider) -> Result<Provider, String> {
        let mut providers = self.list()?;

        // Generate ID if not provided
        let provider_id = provider.id.clone().unwrap_or_else(|| {
            format!(
                "provider-{}",
                provider.name.to_lowercase().replace(' ', "-")
            )
        });

        // Check for duplicate ID
        if providers
            .iter()
            .any(|p| p.id.as_deref() == Some(provider_id.as_str()))
        {
            return Err(format!("Provider with ID {} already exists", provider_id));
        }

        provider.id = Some(provider_id);
        if provider.api_key.trim().is_empty() {
            return Err("Provider API key is required".to_string());
        }
        validate_fixed_provider_origin(&provider)?;
        provider.created_at = Some(chrono::Utc::now().to_rfc3339());

        providers.push(provider.clone());
        self.write_providers(&providers)?;

        Ok(provider)
    }

    /// Update an existing provider
    pub fn update(&self, id: &str, mut provider: Provider) -> Result<Provider, String> {
        let mut providers = self.list()?;

        let index = providers
            .iter()
            .position(|p| p.id.as_deref() == Some(id))
            .ok_or_else(|| format!("Provider not found: {}", id))?;

        // Provider identity is selected by the route/storage lookup, never by
        // mutable request data. This keeps identity-bound security policy intact.
        provider.id = providers[index].id.clone();
        if provider.api_key.trim().is_empty() {
            provider.api_key = providers[index].api_key.clone();
        }
        if provider.created_at.is_none() {
            provider.created_at = providers[index].created_at.clone();
        }
        validate_fixed_provider_origin(&provider)?;

        providers[index] = provider.clone();
        self.write_providers(&providers)?;

        Ok(provider)
    }

    /// Delete a provider
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut providers = self.list()?;

        let initial_len = providers.len();
        providers.retain(|p| p.id.as_deref() != Some(id));

        if providers.len() == initial_len {
            return Err(format!("Provider not found: {}", id));
        }

        self.write_providers(&providers)?;
        Ok(())
    }

    /// Set a provider as the default (unsets all others)
    pub fn set_default(&self, id: &str) -> Result<Provider, String> {
        let mut providers = self.list()?;

        let mut found = false;
        for provider in providers.iter_mut() {
            if provider.id.as_deref() == Some(id) {
                provider.is_default = true;
                found = true;
            } else {
                provider.is_default = false;
            }
        }

        if !found {
            return Err(format!("Provider not found: {}", id));
        }

        self.write_providers(&providers)?;

        // Return the updated provider
        self.get(id)
    }

    /// Test a provider connection
    pub async fn test(&self, provider: &Provider) -> ProviderTestResult {
        // Verification never forwards a user-supplied bearer credential to a
        // redirect target. Canonical Ollama Cloud receives the additional
        // fixed-origin validation below.
        let client_builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none());
        if provider.id.as_deref() == Some("provider-ollama-cloud") {
            if let Err(error) = validate_fixed_provider_origin(provider) {
                return ProviderTestResult {
                    success: false,
                    message: error,
                    models: None,
                };
            }
        }
        let client = client_builder.build();

        let client = match client {
            Ok(c) => c,
            Err(e) => {
                return ProviderTestResult {
                    success: false,
                    message: format!("Failed to create HTTP client: {}", e),
                    models: None,
                }
            }
        };

        let models_url = format!("{}/models", provider.base_url.trim_end_matches('/'));

        let response = client
            .get(&models_url)
            .header("Authorization", format!("Bearer {}", provider.api_key))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                let mut stream = resp.bytes_stream();
                let mut body = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(error) => {
                            return ProviderTestResult {
                                success: false,
                                message: scrub_secret(
                                    &format!("Connection failed: {error}"),
                                    &provider.api_key,
                                ),
                                models: None,
                            }
                        }
                    };
                    if body.len().saturating_add(chunk.len()) > 1_048_576 {
                        return ProviderTestResult {
                            success: false,
                            message: "Provider response exceeded the 1 MiB limit".to_string(),
                            models: None,
                        };
                    }
                    body.extend_from_slice(&chunk);
                }
                if status.is_success() {
                    match serde_json::from_slice::<serde_json::Value>(&body) {
                        Ok(json) => {
                            // Try to extract models from OpenAI-style response
                            let models: Vec<String> = json
                                .get("data")
                                .and_then(|d| d.as_array())
                                .map(|arr: &Vec<serde_json::Value>| {
                                    arr.iter()
                                        .filter_map(|m: &serde_json::Value| {
                                            m.get("id")
                                                .and_then(|id| id.as_str())
                                                .filter(|id| id.chars().count() <= 160)
                                                .map(|id| scrub_secret(id, &provider.api_key))
                                        })
                                        .take(1_000)
                                        .collect()
                                })
                                .unwrap_or_default();

                            if !models.is_empty() {
                                ProviderTestResult {
                                    success: true,
                                    message: scrub_secret(
                                        &format!(
                                            "Successfully connected to {}. Found {} models.",
                                            provider.name,
                                            models.len()
                                        ),
                                        &provider.api_key,
                                    ),
                                    models: Some(models),
                                }
                            } else {
                                ProviderTestResult {
                                    success: true,
                                    message: scrub_secret(
                                        &format!(
                                            "Connected to {}. Could not auto-detect models.",
                                            provider.name
                                        ),
                                        &provider.api_key,
                                    ),
                                    models: None,
                                }
                            }
                        }
                        Err(_) => ProviderTestResult {
                            success: true,
                            message: scrub_secret(
                                &format!(
                                    "Connected to {}. Response format not recognized.",
                                    provider.name
                                ),
                                &provider.api_key,
                            ),
                            models: None,
                        },
                    }
                } else {
                    let error_text =
                        scrub_secret(&String::from_utf8_lossy(&body), &provider.api_key);
                    ProviderTestResult {
                        success: false,
                        message: format!("HTTP {}: {}", status, error_text),
                        models: None,
                    }
                }
            }
            Err(e) => ProviderTestResult {
                success: false,
                message: scrub_secret(&format!("Connection failed: {}", e), &provider.api_key),
                models: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ollama_cloud(base_url: &str) -> Provider {
        serde_json::from_value(serde_json::json!({
            "id": "provider-ollama-cloud",
            "name": "Ollama Cloud",
            "description": "Ollama Cloud API",
            "apiKey": "sentinel-secret",
            "baseUrl": base_url,
            "models": ["glm-5.2:cloud"],
            "isDefault": false
        }))
        .unwrap()
    }

    #[test]
    fn ollama_cloud_origin_is_fixed() {
        assert!(validate_fixed_provider_origin(&ollama_cloud("https://ollama.com/v1")).is_ok());
        assert!(
            validate_fixed_provider_origin(&ollama_cloud("http://localhost:11434/v1")).is_err()
        );
        assert!(validate_fixed_provider_origin(&ollama_cloud("https://example.com/v1")).is_err());
    }

    #[test]
    fn secret_scrubbing_handles_empty_and_repeated_values() {
        assert_eq!(scrub_secret("abc abc", "abc"), "[redacted] [redacted]");
        assert_eq!(scrub_secret("unchanged", ""), "unchanged");
    }

    #[test]
    fn update_cannot_rename_ollama_cloud_to_bypass_origin_policy() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir(vault.path().join("config")).unwrap();
        let paths = std::sync::Arc::new(crate::paths::VaultPaths::new(vault.path().to_path_buf()));
        let service = ProviderService::new(paths);
        service
            .create(ollama_cloud("https://ollama.com/v1"))
            .unwrap();
        let mut attack = ollama_cloud("https://attacker.example/v1");
        attack.id = Some("provider-renamed".to_string());

        assert!(service.update("provider-ollama-cloud", attack).is_err());
        let stored = service.get("provider-ollama-cloud").unwrap();
        assert_eq!(stored.base_url, "https://ollama.com/v1");
        assert_eq!(stored.api_key, "sentinel-secret");
    }

    #[test]
    fn correcting_a_hand_edited_cloud_origin_retains_a_blank_omitted_key() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir(vault.path().join("config")).unwrap();
        let paths = std::sync::Arc::new(crate::paths::VaultPaths::new(vault.path().to_path_buf()));
        std::fs::write(
            paths.providers(),
            serde_json::to_vec(&vec![ollama_cloud("https://attacker.example/v1")]).unwrap(),
        )
        .unwrap();
        let service = ProviderService::new(paths);
        let mut correction = ollama_cloud("https://ollama.com/v1");
        correction.api_key.clear();

        let updated = service.update("provider-ollama-cloud", correction).unwrap();
        assert_eq!(updated.api_key, "sentinel-secret");
        assert_eq!(updated.base_url, "https://ollama.com/v1");
    }

    #[test]
    fn create_rejects_a_blank_api_key() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir(vault.path().join("config")).unwrap();
        let paths = std::sync::Arc::new(crate::paths::VaultPaths::new(vault.path().to_path_buf()));
        let service = ProviderService::new(paths);
        let mut provider = ollama_cloud("https://ollama.com/v1");
        provider.api_key = "   ".to_string();

        assert_eq!(
            service.create(provider).unwrap_err(),
            "Provider API key is required"
        );
    }

    #[tokio::test]
    async fn provider_mutation_lock_rejects_a_concurrent_owner() {
        let owner = provider_mutation_lock().lock().await;
        assert!(provider_mutation_lock().try_lock().is_err());
        drop(owner);
        assert!(provider_mutation_lock().try_lock().is_ok());
    }
}
