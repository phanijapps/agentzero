//! LLM-backed implementation of `PairwiseVerifier`.
//!
//! Conservative — returns `false` (don't merge) on any LLM failure so a
//! flaky network never triggers an incorrect entity merge.

use std::sync::Arc;

use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use knowledge_graph::Entity;
use serde::Deserialize;

use crate::sleep::compactor::PairwiseVerifier;
use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

#[derive(Debug, Deserialize)]
struct VerifierResponse {
    same_entity: bool,
}

/// LLM-backed pairwise verifier. Defaults to deny on any failure so a
/// flaky network or bad response never causes a wrong merge.
pub struct LlmPairwiseVerifier {
    client: CachedLlmClient,
}

impl LlmPairwiseVerifier {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            client: CachedLlmClient::new(factory, LlmClientConfig::new(0.0, 128)),
        }
    }
}

#[async_trait]
impl PairwiseVerifier for LlmPairwiseVerifier {
    async fn should_merge(&self, a: &Entity, b: &Entity) -> bool {
        let client = match self.client.get().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "verifier: build_client failed; default deny");
                return false;
            }
        };
        let prompt = format!(
            "You are an entity-resolution adjudicator. Two entities of the same \
             type were proposed for merge by a similarity threshold. Decide \
             whether they refer to the same real-world thing.\n\n\
             Return ONLY JSON: {{\"same_entity\": bool, \"confidence\": 0.0-1.0}}\n\n\
             Entity A: name={:?}  type={:?}\n\
             Entity B: name={:?}  type={:?}",
            a.name, a.entity_type, b.name, b.entity_type,
        );
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = match client.chat(messages, None).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "verifier: LLM call failed; default deny");
                return false;
            }
        };
        match parse_llm_json::<VerifierResponse>(&response.content) {
            Ok(r) => r.same_entity,
            Err(e) => {
                tracing::warn!(error = %e, "verifier: parse failed; default deny");
                false
            }
        }
    }
}
