//! Rig-facing configuration resolved from current AgentZero settings.

use std::fmt;

use serde_json::{json, Value};

use crate::llm::LlmConfig;

const RESERVED_ADDITIONAL_PARAM_KEYS: &[&str] = &[
    "messages",
    "model",
    "stream",
    "tools",
    "tool_choice",
    "response_format",
    "temperature",
    "max_tokens",
    "thinking",
];

/// Invalid Rig-facing config.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RigConfigError {
    /// Provider params must be a JSON object.
    #[error("provider_params must be a JSON object")]
    ProviderParamsMustBeObject,
    /// Provider params tried to set a framework-owned request key.
    #[error("provider_params contains reserved request-control key: {0}")]
    ReservedProviderParam(String),
}

/// Provider/model settings needed to construct a Rig completion model.
#[derive(Clone, PartialEq)]
pub struct RigModelConfig {
    /// AgentZero provider identifier.
    pub provider_id: String,
    /// OpenAI-compatible API base URL owned by AgentZero config.
    pub base_url: String,
    /// API key or key reference already resolved by the existing config loader.
    pub api_key: String,
    /// Model identifier.
    pub model: String,
    /// Sampling temperature.
    pub temperature: f64,
    /// Maximum generated tokens.
    pub max_tokens: u64,
    /// Effective context window used by runtime middleware.
    pub context_window_tokens: u64,
    /// Whether reasoning/thinking should be requested.
    pub thinking_enabled: bool,
    /// Provider-specific request parameters for Rig's `additional_params`.
    pub provider_params: Option<Value>,
}

impl fmt::Debug for RigModelConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RigModelConfig")
            .field("provider_id", &self.provider_id)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("thinking_enabled", &self.thinking_enabled)
            .field(
                "provider_params",
                &self.provider_params.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl RigModelConfig {
    /// Resolve Rig-facing model settings from the existing LLM config.
    #[must_use]
    pub fn from_llm_config(config: &LlmConfig, context_window_tokens: u64) -> Self {
        Self {
            provider_id: config.provider_id.clone(),
            base_url: config.base_url.clone(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            temperature: config.temperature,
            max_tokens: u64::from(config.max_tokens),
            context_window_tokens,
            thinking_enabled: config.thinking_enabled,
            provider_params: config.provider_params.clone(),
        }
    }

    /// Additional params to apply to Rig completion requests.
    pub fn completion_additional_params(&self) -> Result<Option<Value>, RigConfigError> {
        let mut params = match &self.provider_params {
            Some(Value::Object(map)) => map.clone(),
            Some(_) => return Err(RigConfigError::ProviderParamsMustBeObject),
            None => serde_json::Map::new(),
        };
        for key in RESERVED_ADDITIONAL_PARAM_KEYS {
            if params.contains_key(*key) {
                return Err(RigConfigError::ReservedProviderParam((*key).to_string()));
            }
        }

        if self.thinking_enabled {
            params.insert("thinking".to_string(), json!({ "type": "enabled" }));
        }

        if params.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Value::Object(params)))
        }
    }
}

/// Agent settings needed to construct a Rig `AgentBuilder`.
#[derive(Clone, PartialEq)]
pub struct RigAgentConfig {
    /// Agent identifier.
    pub agent_id: String,
    /// Display name used as Rig agent name.
    pub name: String,
    /// Agent description.
    pub description: String,
    /// System instructions mapped to Rig preamble.
    pub instructions: String,
    /// Provider/model settings.
    pub model: RigModelConfig,
}

impl fmt::Debug for RigAgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RigAgentConfig")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("instructions", &"<redacted>")
            .field("model", &self.model)
            .finish()
    }
}

impl RigAgentConfig {
    /// Create a Rig-facing agent config from already-resolved runtime values.
    #[must_use]
    pub fn new(
        agent_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
        model: RigModelConfig,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: name.into(),
            description: description.into(),
            instructions: instructions.into(),
            model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_config_preserves_current_llm_settings() {
        let llm = LlmConfig::new(
            "https://llm.local/v1".into(),
            "sk-test".into(),
            "z-model".into(),
            "local-provider".into(),
        )
        .with_temperature(0.2)
        .with_max_tokens(321)
        .with_thinking(true)
        .with_provider_params(json!({"parallel_tool_calls": false}));

        let mapped = RigModelConfig::from_llm_config(&llm, 65_536);

        assert_eq!(mapped.provider_id, "local-provider");
        assert_eq!(mapped.base_url, "https://llm.local/v1");
        assert_eq!(mapped.api_key, "sk-test");
        assert_eq!(mapped.model, "z-model");
        assert_eq!(mapped.temperature, 0.2);
        assert_eq!(mapped.max_tokens, 321);
        assert_eq!(mapped.context_window_tokens, 65_536);
        assert!(mapped.thinking_enabled);
        assert_eq!(
            mapped.provider_params,
            Some(json!({"parallel_tool_calls": false}))
        );
    }

    #[test]
    fn completion_params_merge_provider_params_with_thinking() {
        let mapped = RigModelConfig {
            provider_id: "p".into(),
            base_url: "https://llm.local/v1".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            temperature: 0.7,
            max_tokens: 100,
            context_window_tokens: 1_000,
            thinking_enabled: true,
            provider_params: Some(json!({"parallel_tool_calls": false})),
        };

        assert_eq!(
            mapped.completion_additional_params(),
            Ok(Some(json!({
                "parallel_tool_calls": false,
                "thinking": {"type": "enabled"}
            })))
        );
    }

    #[test]
    fn completion_params_absent_when_no_extra_request_settings() {
        let mapped = RigModelConfig {
            provider_id: "p".into(),
            base_url: "https://llm.local/v1".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            temperature: 0.7,
            max_tokens: 100,
            context_window_tokens: 1_000,
            thinking_enabled: false,
            provider_params: None,
        };

        assert_eq!(mapped.completion_additional_params(), Ok(None));
    }

    #[test]
    fn completion_params_reject_reserved_request_control_keys() {
        let mapped = RigModelConfig {
            provider_id: "p".into(),
            base_url: "https://llm.local/v1".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            temperature: 0.7,
            max_tokens: 100,
            context_window_tokens: 1_000,
            thinking_enabled: true,
            provider_params: Some(json!({"tools": []})),
        };

        assert_eq!(
            mapped.completion_additional_params(),
            Err(RigConfigError::ReservedProviderParam("tools".to_string()))
        );
    }

    #[test]
    fn debug_redacts_secrets_and_instructions() {
        let mapped = RigAgentConfig::new(
            "agent-1",
            "Agent",
            "Description",
            "private instructions",
            RigModelConfig {
                provider_id: "p".into(),
                base_url: "https://llm.local/v1".into(),
                api_key: "sk-secret".into(),
                model: "m".into(),
                temperature: 0.7,
                max_tokens: 100,
                context_window_tokens: 1_000,
                thinking_enabled: false,
                provider_params: Some(json!({"secret": "value"})),
            },
        );

        let debug = format!("{mapped:?}");
        assert!(!debug.contains("sk-secret"));
        assert!(!debug.contains("private instructions"));
        assert!(!debug.contains("value"));
        assert!(debug.contains("<redacted>"));
    }
}
