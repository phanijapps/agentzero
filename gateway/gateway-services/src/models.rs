// ============================================================================
// MODEL REGISTRY
// Compatibility model metadata registry for the gateway.
// Provider config and agent/settings fields own token limits now.
// ============================================================================

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_MAX_INPUT_TOKENS: u64 = 200_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 32_000;

// ============================================================================
// Types
// ============================================================================

/// Profile for a single model — capabilities, context window, embedding spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub name: String,
    /// Canonical vendor label for display/grouping (e.g., "openai", "zhipu").
    /// Not a Provider.id lookup key — purely informational.
    pub provider: String,
    pub capabilities: ModelCapabilities,
    pub context: ContextWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingSpec>,
}

/// Boolean capability flags for a model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub thinking: bool,
    #[serde(default)]
    pub embeddings: bool,
    #[serde(default)]
    pub voice: bool,
    #[serde(default)]
    pub image_generation: bool,
    #[serde(default)]
    pub video_generation: bool,
}

/// Input/output token limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindow {
    pub input: u64,
    /// Max output tokens. None means same as input.
    pub output: Option<u64>,
}

impl ContextWindow {
    /// Resolve output token limit. Returns explicit output if set, otherwise input.
    pub fn resolved_output(&self) -> u64 {
        self.output.unwrap_or(self.input)
    }
}

/// Embedding model parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSpec {
    pub dimensions: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dimensions: Option<u32>,
}

/// Named capability for programmatic checks via `has_capability()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Tools,
    Vision,
    Thinking,
    Embeddings,
    Voice,
    ImageGeneration,
    VideoGeneration,
}

impl ModelCapabilities {
    /// Check whether a specific capability is enabled.
    pub fn has(&self, cap: Capability) -> bool {
        match cap {
            Capability::Tools => self.tools,
            Capability::Vision => self.vision,
            Capability::Thinking => self.thinking,
            Capability::Embeddings => self.embeddings,
            Capability::Voice => self.voice,
            Capability::ImageGeneration => self.image_generation,
            Capability::VideoGeneration => self.video_generation,
        }
    }
}

// ============================================================================
// Registry
// ============================================================================

/// Model capabilities registry with fallback-only resolution.
pub struct ModelRegistry {
    models: HashMap<String, ModelProfile>,
    /// Pre-built fallback returned for unknown model IDs.
    fallback: ModelProfile,
}

impl ModelRegistry {
    /// Load an empty compatibility registry.
    pub fn load() -> Self {
        let fallback = ModelProfile {
            name: "Unknown Model".to_string(),
            provider: "unknown".to_string(),
            capabilities: ModelCapabilities {
                tools: true,
                vision: false,
                thinking: false,
                embeddings: false,
                voice: false,
                image_generation: false,
                video_generation: false,
            },
            context: ContextWindow {
                input: DEFAULT_MAX_INPUT_TOKENS,
                output: Some(DEFAULT_MAX_OUTPUT_TOKENS as u64),
            },
            embedding: None,
        };

        let models = HashMap::new();

        tracing::info!("Model registry disabled; using fallback profile for custom models");

        Self { models, fallback }
    }

    /// Get a model profile by ID. Returns the fallback profile for unknown models.
    pub fn get(&self, model_id: &str) -> &ModelProfile {
        self.models.get(model_id).unwrap_or(&self.fallback)
    }

    /// Check whether a model has a specific capability.
    pub fn has_capability(&self, model_id: &str, cap: Capability) -> bool {
        self.get(model_id).capabilities.has(cap)
    }

    /// Get context window for a model.
    pub fn context_window(&self, model_id: &str) -> &ContextWindow {
        &self.get(model_id).context
    }

    /// List all known models (for API/UI). Sorted by model ID.
    pub fn list(&self) -> Vec<(&str, &ModelProfile)> {
        let mut entries: Vec<_> = self.models.iter().map(|(k, v)| (k.as_str(), v)).collect();
        entries.sort_by_key(|(k, _)| *k);
        entries
    }

    /// Check if a model is known (exists in registry, not the fallback).
    pub fn is_known(&self, model_id: &str) -> bool {
        self.models.contains_key(model_id)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_unknown_model_returns_fallback() {
        let registry = ModelRegistry::load();
        let profile = registry.get("nonexistent-model");
        assert_eq!(profile.provider, "unknown");
        assert!(profile.capabilities.tools);
        assert!(!profile.capabilities.vision);
        assert_eq!(profile.context.input, 200_000);
    }

    #[test]
    fn fallback_profile_uses_200k_in_32k_out_with_tools() {
        let registry = ModelRegistry::load();
        let profile = registry.get("some-unknown-model-id");
        assert_eq!(profile.context.input, DEFAULT_MAX_INPUT_TOKENS);
        assert_eq!(
            profile.context.output,
            Some(DEFAULT_MAX_OUTPUT_TOKENS as u64)
        );
        assert!(profile.capabilities.tools);
        assert!(!profile.capabilities.vision);
        assert!(!profile.capabilities.thinking);
        assert!(!profile.capabilities.embeddings);
    }

    #[test]
    fn test_has_capability() {
        let registry = ModelRegistry::load();
        assert!(registry.has_capability("custom-model", Capability::Tools));
        assert!(!registry.has_capability("custom-model", Capability::Vision));
        assert!(!registry.has_capability("custom-model", Capability::Thinking));
        assert!(!registry.has_capability("custom-model", Capability::Embeddings));
    }

    #[test]
    fn test_context_window() {
        let registry = ModelRegistry::load();
        let ctx = registry.context_window("custom-model");
        assert_eq!(ctx.input, DEFAULT_MAX_INPUT_TOKENS);
        assert_eq!(ctx.output, Some(DEFAULT_MAX_OUTPUT_TOKENS as u64));
        assert_eq!(ctx.resolved_output(), DEFAULT_MAX_OUTPUT_TOKENS as u64);
    }

    #[test]
    fn test_is_known() {
        let registry = ModelRegistry::load();
        assert!(!registry.is_known("gpt-4o"));
        assert!(!registry.is_known("nonexistent"));
    }

    #[test]
    fn test_registry_lists_no_bundled_models() {
        let registry = ModelRegistry::load();
        assert_eq!(registry.list().len(), 0);
        let profile = registry.get("anything");
        assert_eq!(profile.provider, "unknown");
    }
}
