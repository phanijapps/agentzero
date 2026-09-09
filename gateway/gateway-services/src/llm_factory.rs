//! Shared provider selection and LLM client construction.
//!
//! One place for the "target → default → first" provider pick and the
//! OpenAI-compatible client build, instead of per-crate hand-rolled copies
//! drifting apart (temperature defaults, fallback order, error strings).

use std::sync::Arc;

use agent_runtime::llm::{openai::OpenAiClient, LlmClient, LlmConfig};

use crate::providers::Provider;

/// Pick a provider from a list: explicit `target` id first (when present
/// and resolvable), then the provider marked default, then the first
/// listed provider. Returns `None` when the list is empty.
pub fn select_provider<'a>(
    providers: &'a [Provider],
    target: Option<&str>,
) -> Option<&'a Provider> {
    target
        .and_then(|tid| providers.iter().find(|p| p.id.as_deref() == Some(tid)))
        .or_else(|| providers.iter().find(|p| p.is_default))
        .or_else(|| providers.first())
}

/// Build an OpenAI-compatible client for an already-selected provider.
pub fn provider_client(
    provider: &Provider,
    model: &str,
    temperature: f64,
    max_tokens: u32,
) -> Result<Arc<dyn LlmClient>, String> {
    let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());
    let config = LlmConfig::new(
        provider.base_url.clone(),
        provider.api_key.clone(),
        model.to_string(),
        provider_id,
    )
    .with_temperature(temperature)
    .with_max_tokens(max_tokens);
    let client = OpenAiClient::new(config).map_err(|e| format!("build client: {e}"))?;
    Ok(Arc::new(client) as Arc<dyn LlmClient>)
}
