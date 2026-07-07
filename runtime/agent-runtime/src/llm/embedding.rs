// ============================================================================
// EMBEDDING CLIENT TRAIT
// Abstract interface for embedding providers
// ============================================================================

use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// Errors from embedding operations
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    /// Error from the HTTP client
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Error from a local model
    #[error("Model error: {0}")]
    ModelError(String),

    /// Error parsing response
    #[error("Parse error: {0}")]
    ParseError(String),

    /// API returned an error
    #[error("API error: {0}")]
    ApiError(String),

    /// Configuration error
    #[error("Config error: {0}")]
    ConfigError(String),
}

/// Trait for embedding providers.
///
/// Implementations can wrap remote APIs (`OpenAI`, Ollama, Voyage) or
/// local ONNX models (fastembed). The trait is object-safe so it can
/// be stored behind `Arc<dyn EmbeddingClient>`.
#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    /// Generate embeddings for one or more texts.
    ///
    /// Returns one vector per input text, each with `dimensions()` floats.
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Return the dimensionality of this model's output.
    fn dimensions(&self) -> usize;

    /// Return the model name for logging and cache keys.
    ///
    /// Returns `String` (not `&str`) so wrapper clients like
    /// `LiveEmbeddingClient` can forward the current backend's name without
    /// fighting the borrow checker over temporary `Arc` guards.
    fn model_name(&self) -> String;

    /// Return the provider family for vector-space compatibility checks.
    fn provider_type(&self) -> String {
        "unknown".to_string()
    }

    /// Return the prompt profile used to create the embedding.
    fn prompt_profile(&self) -> String {
        "query".to_string()
    }

    /// Return the normalization policy used by this embedding backend.
    fn normalization(&self) -> Option<String> {
        None
    }
}

/// Configuration for the embedding system.
///
/// Determines which backend to use and how to connect to it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingConfig {
    /// Which provider to use.
    pub provider: EmbeddingProviderType,

    /// Model identifier (e.g. "all-MiniLM-L6-v2", "nomic-embed-text").
    pub model: String,

    /// Output dimensionality.
    pub dimensions: usize,

    /// Maximum texts per `embed()` call (default 32).
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Enable SHA-256 hash dedup cache (default true).
    #[serde(default = "default_cache_enabled")]
    pub cache_enabled: bool,

    /// Idle timeout in seconds before unloading the local model from RAM.
    /// Default: 600 (10 minutes). Set to 0 to never unload (keep in RAM permanently).
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
}

/// Which embedding backend to use.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmbeddingProviderType {
    /// Local ONNX model via fastembed — no API calls, works offline.
    Local,

    /// Use an existing LLM provider's embedding endpoint.
    /// The `provider_id` references a Provider in providers.json.
    Provider {
        /// The provider ID whose `base_url` and `api_key` will be used.
        provider_id: String,
    },
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProviderType::Local,
            model: "all-MiniLM-L6-v2".to_string(),
            dimensions: 384,
            batch_size: default_batch_size(),
            cache_enabled: default_cache_enabled(),
            idle_timeout_secs: default_idle_timeout(),
        }
    }
}

const fn default_batch_size() -> usize {
    32
}

const fn default_cache_enabled() -> bool {
    true
}

const fn default_idle_timeout() -> u64 {
    600 // 10 minutes — long enough to cover multi-delegation sessions
}

/// Compute SHA-256 hash of text content for embedding cache lookups.
#[must_use]
pub fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.model, "all-MiniLM-L6-v2");
        assert_eq!(config.dimensions, 384);
        assert_eq!(config.batch_size, 32);
        assert!(config.cache_enabled);
        assert_eq!(config.idle_timeout_secs, 600);
        assert!(matches!(config.provider, EmbeddingProviderType::Local));
    }

    #[test]
    fn test_config_serialization() {
        let config = EmbeddingConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: EmbeddingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, config.model);
        assert_eq!(parsed.dimensions, config.dimensions);
    }

    #[test]
    fn test_provider_config_serialization() {
        let config = EmbeddingConfig {
            provider: EmbeddingProviderType::Provider {
                provider_id: "ollama".to_string(),
            },
            model: "nomic-embed-text".to_string(),
            dimensions: 768,
            batch_size: 16,
            cache_enabled: true,
            idle_timeout_secs: 600,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("provider_id"));
        let parsed: EmbeddingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.dimensions, 768);
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_different_inputs() {
        let h1 = content_hash("hello");
        let h2 = content_hash("world");
        assert_ne!(h1, h2);
    }
}
