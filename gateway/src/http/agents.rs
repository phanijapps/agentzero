//! # Agent Endpoints
//!
//! CRUD operations for agents.

use crate::services::agents::Agent;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use gateway_services::models::{DEFAULT_MAX_INPUT_TOKENS, DEFAULT_MAX_OUTPUT_TOKENS};
use serde::{Deserialize, Serialize};

/// Agent response (full view for API).
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentResponse {
    pub id: String,
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub model: String,
    pub temperature: f64,
    #[serde(rename = "maxInputTokens")]
    pub max_input_tokens: u64,
    #[serde(rename = "maxInputTokensExplicit")]
    pub max_input_tokens_explicit: bool,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: u32,
    #[serde(rename = "maxTokens")]
    pub max_tokens: u32,
    #[serde(rename = "thinkingEnabled")]
    pub thinking_enabled: bool,
    #[serde(rename = "voiceRecordingEnabled")]
    pub voice_recording_enabled: bool,
    pub instructions: String,
    pub mcps: Vec<String>,
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middleware: Option<String>,
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl From<Agent> for AgentResponse {
    fn from(agent: Agent) -> Self {
        Self {
            id: agent.id,
            name: agent.name,
            display_name: agent.display_name,
            description: agent.description,
            provider_id: agent.provider_id,
            model: agent.model,
            temperature: agent.temperature,
            max_input_tokens: agent.max_input_tokens,
            max_input_tokens_explicit: agent.max_input_tokens_explicit,
            max_output_tokens: agent.max_tokens,
            max_tokens: agent.max_tokens,
            thinking_enabled: agent.thinking_enabled,
            voice_recording_enabled: agent.voice_recording_enabled,
            instructions: agent.instructions,
            mcps: agent.mcps,
            skills: agent.skills,
            middleware: agent.middleware,
            created_at: agent.created_at,
        }
    }
}

/// Create agent request.
#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub model: String,
    pub temperature: Option<f64>,
    #[serde(rename = "maxInputTokens")]
    pub max_input_tokens: Option<u64>,
    #[serde(rename = "maxInputTokensExplicit")]
    pub max_input_tokens_explicit: Option<bool>,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u32>,
    #[serde(rename = "maxTokens")]
    pub legacy_max_tokens: Option<u32>,
    pub instructions: Option<String>,
    pub mcps: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
}

impl CreateAgentRequest {
    fn effective_max_output_tokens(&self) -> Option<u32> {
        self.max_output_tokens.or(self.legacy_max_tokens)
    }
}

/// Update agent request.
#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "providerId")]
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    #[serde(rename = "maxInputTokens")]
    pub max_input_tokens: Option<u64>,
    #[serde(rename = "maxInputTokensExplicit")]
    pub max_input_tokens_explicit: Option<bool>,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u32>,
    #[serde(rename = "maxTokens")]
    pub legacy_max_tokens: Option<u32>,
    #[serde(rename = "thinkingEnabled")]
    pub thinking_enabled: Option<bool>,
    #[serde(rename = "voiceRecordingEnabled")]
    pub voice_recording_enabled: Option<bool>,
    pub instructions: Option<String>,
    pub mcps: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub middleware: Option<String>,
}

impl UpdateAgentRequest {
    fn effective_max_output_tokens(&self) -> Option<u32> {
        self.max_output_tokens.or(self.legacy_max_tokens)
    }
}

fn resolve_updated_max_input_tokens(
    existing_tokens: u64,
    existing_explicit: bool,
    requested_tokens: Option<u64>,
    requested_explicit: Option<bool>,
) -> (u64, bool) {
    match requested_explicit {
        Some(false) => (DEFAULT_MAX_INPUT_TOKENS, false),
        Some(true) => (requested_tokens.unwrap_or(existing_tokens), true),
        None => match requested_tokens {
            Some(tokens) if tokens != existing_tokens => (tokens, true),
            Some(tokens) => (tokens, existing_explicit),
            None => (existing_tokens, existing_explicit),
        },
    }
}

/// GET /api/agents - List all agents.
pub async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentResponse>> {
    match state.agents.list().await {
        Ok(agents) => Json(agents.into_iter().map(AgentResponse::from).collect()),
        Err(e) => {
            tracing::error!("Failed to list agents: {}", e);
            Json(vec![])
        }
    }
}

/// POST /api/agents - Create a new agent.
pub async fn create_agent(
    State(state): State<AppState>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<AgentResponse>, StatusCode> {
    let max_output_tokens = request.effective_max_output_tokens();
    let max_input_tokens_explicit = request
        .max_input_tokens_explicit
        .unwrap_or_else(|| request.max_input_tokens.is_some());
    let max_input_tokens = if max_input_tokens_explicit {
        request.max_input_tokens.unwrap_or(DEFAULT_MAX_INPUT_TOKENS)
    } else {
        DEFAULT_MAX_INPUT_TOKENS
    };
    let agent = Agent {
        id: String::new(),
        name: request.name.clone(),
        display_name: request.display_name.unwrap_or_else(|| request.name.clone()),
        description: request.description.unwrap_or_default(),
        agent_type: Some("llm".to_string()),
        provider_id: request.provider_id,
        model: request.model,
        temperature: request.temperature.unwrap_or(0.7),
        max_input_tokens,
        max_input_tokens_explicit,
        max_tokens: max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
        thinking_enabled: false,
        voice_recording_enabled: true,
        system_instruction: None,
        instructions: request
            .instructions
            .unwrap_or_else(|| "You are a helpful AI assistant.".to_string()),
        mcps: request.mcps.unwrap_or_default(),
        skills: request.skills.unwrap_or_default(),
        middleware: None,
        created_at: None,
    };

    match state.agents.create(agent).await {
        Ok(created) => Ok(Json(AgentResponse::from(created))),
        Err(e) => {
            tracing::error!("Failed to create agent: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /api/agents/:id - Get an agent by ID.
pub async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentResponse>, StatusCode> {
    match state.agents.get(&id).await {
        Ok(agent) => Ok(Json(AgentResponse::from(agent))),
        Err(e) => {
            tracing::warn!("Agent not found: {} - {}", id, e);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

/// PUT /api/agents/:id - Update an agent.
pub async fn update_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateAgentRequest>,
) -> Result<Json<AgentResponse>, StatusCode> {
    // Get existing agent
    let existing = match state.agents.get(&id).await {
        Ok(a) => a,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };
    let max_output_tokens = request.effective_max_output_tokens();
    let (max_input_tokens, max_input_tokens_explicit) = resolve_updated_max_input_tokens(
        existing.max_input_tokens,
        existing.max_input_tokens_explicit,
        request.max_input_tokens,
        request.max_input_tokens_explicit,
    );

    // Merge updates
    let updated = Agent {
        id: existing.id,
        name: request.name.unwrap_or(existing.name),
        display_name: request.display_name.unwrap_or(existing.display_name),
        description: request.description.unwrap_or(existing.description),
        agent_type: existing.agent_type,
        provider_id: request.provider_id.unwrap_or(existing.provider_id),
        model: request.model.unwrap_or(existing.model),
        temperature: request.temperature.unwrap_or(existing.temperature),
        max_input_tokens,
        max_input_tokens_explicit,
        max_tokens: max_output_tokens.unwrap_or(existing.max_tokens),
        thinking_enabled: request
            .thinking_enabled
            .unwrap_or(existing.thinking_enabled),
        voice_recording_enabled: request
            .voice_recording_enabled
            .unwrap_or(existing.voice_recording_enabled),
        system_instruction: existing.system_instruction,
        instructions: request.instructions.unwrap_or(existing.instructions),
        mcps: request.mcps.unwrap_or(existing.mcps),
        skills: request.skills.unwrap_or(existing.skills),
        middleware: request.middleware.or(existing.middleware),
        created_at: existing.created_at,
    };

    match state.agents.update(&id, updated).await {
        Ok(agent) => Ok(Json(AgentResponse::from(agent))),
        Err(e) => {
            tracing::error!("Failed to update agent: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// DELETE /api/agents/:id - Delete an agent.
pub async fn delete_agent(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    match state.agents.delete(&id).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::warn!("Failed to delete agent: {} - {}", id, e);
            StatusCode::NOT_FOUND
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_agent_request_accepts_canonical_and_legacy_output_tokens() {
        let request: UpdateAgentRequest = serde_json::from_value(serde_json::json!({
            "displayName": "Reviewer",
            "providerId": "provider-ollama",
            "model": "kimi-k2.6:cloud",
            "maxInputTokens": 200000,
            "maxInputTokensExplicit": true,
            "maxOutputTokens": 16384,
            "maxTokens": 16384
        }))
        .expect("agent update payload with legacy maxTokens should parse");

        assert_eq!(request.max_input_tokens, Some(200000));
        assert_eq!(request.max_input_tokens_explicit, Some(true));
        assert_eq!(request.max_output_tokens, Some(16384));
        assert_eq!(request.legacy_max_tokens, Some(16384));
        assert_eq!(request.effective_max_output_tokens(), Some(16384));
    }

    #[test]
    fn create_agent_request_accepts_legacy_output_tokens() {
        let request: CreateAgentRequest = serde_json::from_value(serde_json::json!({
            "name": "reviewer-agent",
            "providerId": "provider-ollama",
            "model": "kimi-k2.6:cloud",
            "maxTokens": 12000
        }))
        .expect("legacy agent create payload should parse");

        assert_eq!(request.max_output_tokens, None);
        assert_eq!(request.legacy_max_tokens, Some(12000));
        assert_eq!(request.effective_max_output_tokens(), Some(12000));
    }

    #[test]
    fn update_roundtrip_preserves_inherited_max_input_tokens() {
        let (tokens, explicit) =
            resolve_updated_max_input_tokens(DEFAULT_MAX_INPUT_TOKENS, false, Some(200000), None);

        assert_eq!(tokens, DEFAULT_MAX_INPUT_TOKENS);
        assert!(!explicit);
    }

    #[test]
    fn update_can_explicitly_set_default_max_input_tokens() {
        let (tokens, explicit) = resolve_updated_max_input_tokens(
            DEFAULT_MAX_INPUT_TOKENS,
            false,
            Some(200000),
            Some(true),
        );

        assert_eq!(tokens, DEFAULT_MAX_INPUT_TOKENS);
        assert!(explicit);
    }

    #[test]
    fn update_can_clear_explicit_max_input_tokens() {
        let (tokens, explicit) =
            resolve_updated_max_input_tokens(64_000, true, Some(64_000), Some(false));

        assert_eq!(tokens, DEFAULT_MAX_INPUT_TOKENS);
        assert!(!explicit);
    }
}
