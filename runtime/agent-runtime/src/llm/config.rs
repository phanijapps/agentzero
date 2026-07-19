// ============================================================================
// LLM CONFIGURATION
// Configuration for LLM clients
// ============================================================================

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for an LLM client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// API base URL
    pub base_url: String,

    /// API key for authentication
    pub api_key: String,

    /// Model identifier
    pub model: String,

    /// Provider identifier
    pub provider_id: String,

    /// Temperature for generation (0.0 - 1.0)
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    /// Maximum tokens to generate
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    /// Enable reasoning/thinking
    #[serde(default)]
    pub thinking_enabled: bool,

    /// Provider-specific request parameters.
    #[serde(default, skip_serializing)]
    pub provider_params: Option<Value>,
}

const fn default_temperature() -> f64 {
    0.7
}

const fn default_max_tokens() -> u32 {
    2000
}

impl LlmConfig {
    /// Create a new LLM config
    #[must_use]
    pub fn new(base_url: String, api_key: String, model: String, provider_id: String) -> Self {
        Self {
            base_url,
            api_key,
            model,
            provider_id,
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            thinking_enabled: false,
            provider_params: None,
        }
    }

    /// Create config with temperature
    #[must_use]
    pub const fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }

    /// Create config with max tokens
    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Create config with thinking enabled
    #[must_use]
    pub const fn with_thinking(mut self, thinking_enabled: bool) -> Self {
        self.thinking_enabled = thinking_enabled;
        self
    }

    /// Create config with provider-specific request parameters.
    #[must_use]
    pub fn with_provider_params(mut self, provider_params: Value) -> Self {
        self.provider_params = Some(provider_params);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = LlmConfig::new(
            "https://api.openai.com".to_string(),
            "sk-test".to_string(),
            "gpt-4".to_string(),
            "openai".to_string(),
        )
        .with_temperature(0.5)
        .with_max_tokens(1000)
        .with_thinking(true);

        assert_eq!(config.temperature, 0.5);
        assert_eq!(config.max_tokens, 1000);
        assert!(config.thinking_enabled);
        assert_eq!(config.provider_params, None);
    }
}
