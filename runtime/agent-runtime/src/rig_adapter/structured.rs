//! Structured-output helpers that keep Rig types inside the runtime boundary.

use std::sync::Arc;

use rig::client::CompletionClient;
use rig::completion::TypedPrompt;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use super::client::LlmCompletionClient;
use crate::llm::LlmClient;

/// Run a Rig typed prompt over the existing AgentZero LLM client.
pub async fn prompt_typed<T>(
    llm_client: Arc<dyn LlmClient>,
    system_prompt: impl Into<String>,
    user_prompt: impl Into<String>,
) -> Result<T, String>
where
    T: JsonSchema + DeserializeOwned + Send + 'static,
{
    let model_id = llm_client.model().to_string();
    let system_prompt = system_prompt.into();
    let client = LlmCompletionClient::new(llm_client);
    let agent = client.agent(model_id).preamble(&system_prompt).build();

    agent
        .prompt_typed::<T>(user_prompt.into())
        .await
        .map_err(|e| e.to_string())
}
