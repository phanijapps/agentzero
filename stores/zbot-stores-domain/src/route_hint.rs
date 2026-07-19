//! Durable route hints for memory items.
//!
//! A route hint tells an agent where to go after a search hit: which ward owns
//! the source and, when available, which file/artifact/session/execution backs
//! the item. It is metadata only; it must not affect ranking.

use serde::{Deserialize, Serialize};

/// Source family for a durable memory route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSourceKind {
    Fact,
    WikiArticle,
    Procedure,
    Episode,
    Artifact,
    WardFile,
    Graph,
    Goal,
    Belief,
}

/// Coordinates that let a future agent inspect the durable source behind a
/// recall/search hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteHint {
    pub ward_id: String,
    pub source_kind: RouteSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
}

impl RouteHint {
    pub fn new(ward_id: impl Into<String>, source_kind: RouteSourceKind) -> Self {
        Self {
            ward_id: ward_id.into(),
            source_kind,
            source_path: None,
            session_id: None,
            execution_id: None,
            artifact_id: None,
            memory_id: None,
        }
    }

    pub fn with_memory_id(mut self, memory_id: impl Into<String>) -> Self {
        self.memory_id = Some(memory_id.into());
        self
    }

    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    pub fn with_source_path(mut self, source_path: Option<String>) -> Self {
        self.source_path = source_path;
        self
    }

    pub fn with_execution_id(mut self, execution_id: Option<String>) -> Self {
        self.execution_id = execution_id;
        self
    }

    pub fn with_artifact_id(mut self, artifact_id: Option<String>) -> Self {
        self.artifact_id = artifact_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_hint_omits_absent_optional_fields() {
        let hint = RouteHint::new("lab", RouteSourceKind::Fact).with_memory_id("fact-1");
        let value = serde_json::to_value(hint).unwrap();
        assert_eq!(value["ward_id"], "lab");
        assert_eq!(value["source_kind"], "fact");
        assert_eq!(value["memory_id"], "fact-1");
        assert!(value.get("source_path").is_none());
        assert!(value.get("execution_id").is_none());
    }
}
