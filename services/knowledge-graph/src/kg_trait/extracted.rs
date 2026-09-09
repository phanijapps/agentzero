use crate::types::{Entity, Relationship};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedKnowledge {
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

impl From<ExtractedKnowledge> for crate::types::ExtractedKnowledge {
    fn from(value: ExtractedKnowledge) -> Self {
        crate::types::ExtractedKnowledge {
            entities: value.entities,
            relationships: value.relationships,
        }
    }
}
