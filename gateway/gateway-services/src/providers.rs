// ============================================================================
// PROVIDERS SERVICE
// LLM provider management for the gateway
// ============================================================================

use crate::paths::SharedVaultPaths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

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

        // Preserve ID and created_at
        if provider.id.is_none() {
            provider.id = providers[index].id.clone();
        }
        if provider.created_at.is_none() {
            provider.created_at = providers[index].created_at.clone();
        }

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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build();

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
                if resp.status().is_success() {
                    match resp.json::<serde_json::Value>().await {
                        Ok(json) => {
                            // Try to extract models from OpenAI-style response
                            let models: Vec<String> = json
                                .get("data")
                                .and_then(|d| d.as_array())
                                .map(|arr: &Vec<serde_json::Value>| {
                                    arr.iter()
                                        .filter_map(|m: &serde_json::Value| {
                                            m.get("id").and_then(|id| id.as_str()).map(String::from)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            if !models.is_empty() {
                                ProviderTestResult {
                                    success: true,
                                    message: format!(
                                        "Successfully connected to {}. Found {} models.",
                                        provider.name,
                                        models.len()
                                    ),
                                    models: Some(models),
                                }
                            } else {
                                ProviderTestResult {
                                    success: true,
                                    message: format!(
                                        "Connected to {}. Could not auto-detect models.",
                                        provider.name
                                    ),
                                    models: None,
                                }
                            }
                        }
                        Err(_) => ProviderTestResult {
                            success: true,
                            message: format!(
                                "Connected to {}. Response format not recognized.",
                                provider.name
                            ),
                            models: None,
                        },
                    }
                } else {
                    let status = resp.status();
                    let error_text: String = resp.text().await.unwrap_or_default();
                    ProviderTestResult {
                        success: false,
                        message: format!("HTTP {}: {}", status, error_text),
                        models: None,
                    }
                }
            }
            Err(e) => ProviderTestResult {
                success: false,
                message: format!("Connection failed: {}", e),
                models: None,
            },
        }
    }
}
