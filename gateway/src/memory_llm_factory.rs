//! Production implementation of `gateway_memory::MemoryLlmFactory` wired to
//! `gateway_services::ProviderService`.
//!
//! Lives in the gateway crate (not `gateway-memory`) because it depends on
//! `ProviderService`, which itself depends on `gateway-memory`. Constructing
//! one of these per process avoids the six copy-pasted `build_client`
//! methods that previously lived inside each sleep-time LLM impl.

use std::sync::Arc;

use agent_runtime::llm::LlmClient;
use async_trait::async_trait;
use gateway_memory::{LlmClientConfig, MemoryLlmFactory};
use gateway_services::ProviderService;

/// Builds OpenAI-compatible LLM clients using the default configured
/// provider from `ProviderService`. Falls back to the first listed
/// provider when none is marked default.
pub struct ProviderServiceLlmFactory {
    provider_service: Arc<ProviderService>,
}

impl ProviderServiceLlmFactory {
    pub fn new(provider_service: Arc<ProviderService>) -> Self {
        Self { provider_service }
    }
}

#[async_trait]
impl MemoryLlmFactory for ProviderServiceLlmFactory {
    async fn build_client(&self, config: LlmClientConfig) -> Result<Arc<dyn LlmClient>, String> {
        let providers = self
            .provider_service
            .list()
            .map_err(|e| format!("list providers: {e}"))?;
        let provider = gateway_services::select_provider(&providers, None)
            .ok_or_else(|| "no providers configured".to_string())?;
        gateway_services::provider_client(
            provider,
            provider.default_model(),
            config.temperature,
            config.max_tokens,
        )
    }
}
