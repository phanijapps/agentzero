//! # Skills Endpoints
//!
//! CRUD operations for skills.

use crate::services::skills::Skill;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

/// Skill response for API.
#[derive(Debug, Serialize, Deserialize)]
pub struct SkillResponse {
    pub id: String,
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
    pub category: String,
    pub instructions: String,
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl From<Skill> for SkillResponse {
    fn from(skill: Skill) -> Self {
        Self {
            id: skill.id,
            name: skill.name,
            display_name: skill.display_name,
            description: skill.description,
            category: skill.category,
            instructions: skill.instructions,
            created_at: skill.created_at,
        }
    }
}

/// Create skill request.
#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub instructions: Option<String>,
}

/// Update skill request.
#[derive(Debug, Deserialize)]
pub struct UpdateSkillRequest {
    pub name: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub instructions: Option<String>,
}

/// GET /api/skills - List all skills.
pub async fn list_skills(State(state): State<AppState>) -> Json<Vec<SkillResponse>> {
    match state.skills.list().await {
        Ok(skills) => Json(skills.into_iter().map(SkillResponse::from).collect()),
        Err(e) => {
            tracing::error!("Failed to list skills: {}", e);
            Json(vec![])
        }
    }
}

/// POST /api/skills - Create a new skill.
pub async fn create_skill(
    State(state): State<AppState>,
    Json(request): Json<CreateSkillRequest>,
) -> Result<Json<SkillResponse>, StatusCode> {
    let skill = Skill {
        id: String::new(),
        name: request.name.clone(),
        display_name: request.display_name.unwrap_or_else(|| request.name.clone()),
        description: request.description.unwrap_or_default(),
        category: request.category.unwrap_or_else(|| "general".to_string()),
        instructions: request
            .instructions
            .unwrap_or_else(|| "You are a helpful skill.".to_string()),
        created_at: None,
        // Created skills always land in the vault; SkillService::create
        // overrides this anyway, but we set it explicitly so the struct
        // initializer is complete.
        source: gateway_services::SkillSource::Vault,
    };

    match state.skills.create(skill).await {
        Ok(created) => Ok(Json(SkillResponse::from(created))),
        Err(e) => {
            tracing::error!("Failed to create skill: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /api/skills/:id - Get a skill by ID.
pub async fn get_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SkillResponse>, StatusCode> {
    match state.skills.get(&id).await {
        Ok(skill) => Ok(Json(SkillResponse::from(skill))),
        Err(e) => {
            tracing::warn!("Skill not found: {} - {}", id, e);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

/// PUT /api/skills/:id - Update a skill.
pub async fn update_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateSkillRequest>,
) -> Result<Json<SkillResponse>, StatusCode> {
    let existing = match state.skills.get(&id).await {
        Ok(s) => s,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };

    let updated = Skill {
        id: existing.id,
        name: request.name.unwrap_or(existing.name),
        display_name: request.display_name.unwrap_or(existing.display_name),
        description: request.description.unwrap_or(existing.description),
        category: request.category.unwrap_or(existing.category),
        instructions: request.instructions.unwrap_or(existing.instructions),
        created_at: existing.created_at,
        source: existing.source,
    };

    match state.skills.update(&id, updated).await {
        Ok(skill) => Ok(Json(SkillResponse::from(skill))),
        Err(e) => {
            tracing::error!("Failed to update skill: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// DELETE /api/skills/:id - Delete a skill.
pub async fn delete_skill(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    match state.skills.delete(&id).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::warn!("Failed to delete skill: {} - {}", id, e);
            StatusCode::NOT_FOUND
        }
    }
}
