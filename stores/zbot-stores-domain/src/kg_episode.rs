//! `KgEpisode` and `EpisodeSource` — provenance tracking for KG extractions.

use serde::{Deserialize, Serialize};

/// The source system that produced an episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpisodeSource {
    ToolResult,
    WardFile,
    Session,
    Distillation,
    UserInput,
}

impl EpisodeSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolResult => "tool_result",
            Self::WardFile => "ward_file",
            Self::Session => "session",
            Self::Distillation => "distillation",
            Self::UserInput => "user_input",
        }
    }
}

/// A provenance record: one extraction event from one source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgEpisode {
    pub id: String,
    pub source_type: String,
    pub source_ref: String,
    pub content_hash: String,
    pub session_id: Option<String>,
    pub agent_id: String,
    pub status: String,
    pub retry_count: u32,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}
