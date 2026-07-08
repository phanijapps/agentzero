//! # Knowledge Graph Types
//!
//! Core data structures for entities and relationships.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Entity type classification
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntityType {
    /// Person (e.g., "John Doe")
    Person,
    /// Organization (e.g., "Google")
    Organization,
    /// Location (e.g., "San Francisco")
    Location,
    /// Concept/Topic (e.g., "machine learning")
    Concept,
    /// Tool/Technology (e.g., "React")
    Tool,
    /// Project (e.g., "Project X")
    Project,
    /// File (e.g., "main.rs", "config.toml")
    File,
    /// Event (e.g., "World War II", "Product Launch")
    Event,
    /// Time period (e.g., "1945", "Renaissance")
    TimePeriod,
    /// Document (e.g., "Declaration of Independence")
    Document,
    /// Role/Title (e.g., "CEO", "President")
    Role,
    /// Artifact (e.g., "Mona Lisa", "Constitution")
    Artifact,
    /// Ward (internal AgentZero concept)
    Ward,
    /// Custom entity type
    Custom(String),
}

// Custom serialization to serialize as string instead of {Custom: "value"} format
impl Serialize for EntityType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EntityType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(EntityType::from_str(&s))
    }
}

impl EntityType {
    /// Built-in entity type IDs that zbot owns before custom extensions.
    pub const BUILTIN_IDS: &'static [&'static str] = &[
        "person",
        "organization",
        "location",
        "concept",
        "tool",
        "project",
        "file",
        "event",
        "time_period",
        "document",
        "role",
        "artifact",
        "ward",
    ];

    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "person" => EntityType::Person,
            "organization" | "org" => EntityType::Organization,
            "location" | "place" => EntityType::Location,
            "concept" | "topic" => EntityType::Concept,
            "tool" | "technology" => EntityType::Tool,
            "project" => EntityType::Project,
            "file" => EntityType::File,
            "event" => EntityType::Event,
            "timeperiod" | "time_period" | "time period" | "year" | "era" => EntityType::TimePeriod,
            "document" | "doc" => EntityType::Document,
            "role" | "title" => EntityType::Role,
            "artifact" => EntityType::Artifact,
            "ward" => EntityType::Ward,
            "company" => EntityType::Organization,
            other => EntityType::Custom(other.to_string()),
        }
    }

    /// Convert to string
    pub fn as_str(&self) -> &str {
        match self {
            EntityType::Person => "person",
            EntityType::Organization => "organization",
            EntityType::Location => "location",
            EntityType::Concept => "concept",
            EntityType::Tool => "tool",
            EntityType::Project => "project",
            EntityType::File => "file",
            EntityType::Event => "event",
            EntityType::TimePeriod => "time_period",
            EntityType::Document => "document",
            EntityType::Role => "role",
            EntityType::Artifact => "artifact",
            EntityType::Ward => "ward",
            EntityType::Custom(s) => s,
        }
    }
}

/// Relationship type classification
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RelationshipType {
    /// Works for/at
    WorksFor,
    /// Located in
    LocatedIn,
    /// Related to (general)
    RelatedTo,
    /// Created/owns
    Created,
    /// Uses/depends on
    Uses,
    /// Part of
    PartOf,
    /// Mentions
    Mentions,
    // --- Temporal ---
    /// Occurs before another event/period
    Before,
    /// Occurs after another event/period
    After,
    /// Occurs during another event/period
    During,
    /// Occurs at the same time as another event
    ConcurrentWith,
    /// Is succeeded by another event/person/role
    SucceededBy,
    /// Is preceded by another event/person/role
    PrecededBy,
    // --- Role-based ---
    /// Is president of an organization/country
    PresidentOf,
    /// Is founder of an organization
    FounderOf,
    /// Is a member of a group/organization
    MemberOf,
    /// Is author of a document/work
    AuthorOf,
    /// Held a role/title
    HeldRole,
    /// Is/was employed by an organization
    EmployedBy,
    // --- Spatial ---
    /// Event held at a location
    HeldAt,
    /// Born in a location
    BornIn,
    /// Died in a location
    DiedIn,
    // --- Causal ---
    /// Caused another event/outcome
    Caused,
    /// Enabled another event/outcome
    Enabled,
    /// Prevented another event/outcome
    Prevented,
    /// Was triggered by another event
    TriggeredBy,
    // --- Hierarchical ---
    /// Contains another entity
    Contains,
    /// Is an instance of a concept/type
    InstanceOf,
    /// Is a subtype of a concept/type
    SubtypeOf,
    /// Custom relationship type
    Custom(String),
}

// Custom serialization to serialize as string instead of {Custom: "value"} format
impl Serialize for RelationshipType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelationshipType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(RelationshipType::from_str(&s))
    }
}

impl RelationshipType {
    /// Built-in relationship type IDs that zbot owns before custom extensions.
    pub const BUILTIN_IDS: &'static [&'static str] = &[
        "works_for",
        "located_in",
        "related_to",
        "created",
        "uses",
        "part_of",
        "mentions",
        "before",
        "after",
        "during",
        "concurrent_with",
        "succeeded_by",
        "preceded_by",
        "president_of",
        "founder_of",
        "member_of",
        "author_of",
        "held_role",
        "employed_by",
        "held_at",
        "born_in",
        "died_in",
        "caused",
        "enabled",
        "prevented",
        "triggered_by",
        "contains",
        "instance_of",
        "subtype_of",
    ];

    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().replace('_', "").as_str() {
            "worksfor" => RelationshipType::WorksFor,
            "locatedin" => RelationshipType::LocatedIn,
            "relatedto" => RelationshipType::RelatedTo,
            "created" => RelationshipType::Created,
            "uses" => RelationshipType::Uses,
            "partof" => RelationshipType::PartOf,
            "mentions" => RelationshipType::Mentions,
            // Temporal
            "before" => RelationshipType::Before,
            "after" => RelationshipType::After,
            "during" => RelationshipType::During,
            "concurrentwith" => RelationshipType::ConcurrentWith,
            "succeededby" => RelationshipType::SucceededBy,
            "precededby" => RelationshipType::PrecededBy,
            // Role-based
            "presidentof" => RelationshipType::PresidentOf,
            "founderof" => RelationshipType::FounderOf,
            "memberof" => RelationshipType::MemberOf,
            "authorof" => RelationshipType::AuthorOf,
            "heldrole" => RelationshipType::HeldRole,
            "employedby" => RelationshipType::EmployedBy,
            // Spatial
            "heldat" => RelationshipType::HeldAt,
            "bornin" => RelationshipType::BornIn,
            "diedin" => RelationshipType::DiedIn,
            // Causal
            "caused" => RelationshipType::Caused,
            "enabled" => RelationshipType::Enabled,
            "prevented" => RelationshipType::Prevented,
            "triggeredby" => RelationshipType::TriggeredBy,
            // Hierarchical
            "contains" => RelationshipType::Contains,
            "instanceof" => RelationshipType::InstanceOf,
            "subtypeof" => RelationshipType::SubtypeOf,
            other => RelationshipType::Custom(other.to_string()),
        }
    }

    /// Convert to string
    pub fn as_str(&self) -> &str {
        match self {
            RelationshipType::WorksFor => "works_for",
            RelationshipType::LocatedIn => "located_in",
            RelationshipType::RelatedTo => "related_to",
            RelationshipType::Created => "created",
            RelationshipType::Uses => "uses",
            RelationshipType::PartOf => "part_of",
            RelationshipType::Mentions => "mentions",
            RelationshipType::Before => "before",
            RelationshipType::After => "after",
            RelationshipType::During => "during",
            RelationshipType::ConcurrentWith => "concurrent_with",
            RelationshipType::SucceededBy => "succeeded_by",
            RelationshipType::PrecededBy => "preceded_by",
            RelationshipType::PresidentOf => "president_of",
            RelationshipType::FounderOf => "founder_of",
            RelationshipType::MemberOf => "member_of",
            RelationshipType::AuthorOf => "author_of",
            RelationshipType::HeldRole => "held_role",
            RelationshipType::EmployedBy => "employed_by",
            RelationshipType::HeldAt => "held_at",
            RelationshipType::BornIn => "born_in",
            RelationshipType::DiedIn => "died_in",
            RelationshipType::Caused => "caused",
            RelationshipType::Enabled => "enabled",
            RelationshipType::Prevented => "prevented",
            RelationshipType::TriggeredBy => "triggered_by",
            RelationshipType::Contains => "contains",
            RelationshipType::InstanceOf => "instance_of",
            RelationshipType::SubtypeOf => "subtype_of",
            RelationshipType::Custom(s) => s,
        }
    }
}

/// Knowledge graph entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique entity ID
    pub id: String,
    /// Agent ID this entity belongs to
    pub agent_id: String,
    /// Entity type
    pub entity_type: EntityType,
    /// Entity name (e.g., "John", "Google")
    pub name: String,
    /// Additional properties (aliases, descriptions, etc.)
    pub properties: HashMap<String, serde_json::Value>,
    /// First time this entity was seen
    pub first_seen_at: DateTime<Utc>,
    /// Last time this entity was referenced
    pub last_seen_at: DateTime<Utc>,
    /// Number of times this entity appears
    pub mention_count: i64,
    /// Optional name embedding used for stage 2 (ANN) resolver.
    /// When set, this vector is written to `kg_name_index` on store so that
    /// subsequent resolves can match this entity by semantic similarity even
    /// if the surface-form alias lookup misses.
    #[serde(default)]
    pub name_embedding: Option<Vec<f32>>,
}

/// Knowledge graph relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Unique relationship ID
    pub id: String,
    /// Agent ID this relationship belongs to
    pub agent_id: String,
    /// Source entity ID
    pub source_entity_id: String,
    /// Target entity ID
    pub target_entity_id: String,
    /// Relationship type
    pub relationship_type: RelationshipType,
    /// Additional properties (confidence, context, etc.)
    pub properties: HashMap<String, serde_json::Value>,
    /// First time this relationship was seen
    pub first_seen_at: DateTime<Utc>,
    /// Last time this relationship was referenced
    pub last_seen_at: DateTime<Utc>,
    /// Number of times this relationship appears
    pub mention_count: i64,
}

impl Entity {
    /// Create a new entity
    pub fn new(agent_id: String, entity_type: EntityType, name: String) -> Self {
        let now = Utc::now();
        let id = format!("entity_{}_{}", agent_id, uuid::Uuid::new_v4());

        Self {
            id,
            agent_id,
            entity_type,
            name,
            properties: HashMap::new(),
            first_seen_at: now,
            last_seen_at: now,
            mention_count: 1,
            name_embedding: None,
        }
    }

    /// Update the last seen time and increment mention count
    pub fn touch(&mut self) {
        self.last_seen_at = Utc::now();
        self.mention_count += 1;
    }

    /// Add a property
    pub fn with_property(mut self, key: String, value: serde_json::Value) -> Self {
        self.properties.insert(key, value);
        self
    }
}

impl Relationship {
    /// Create a new relationship
    pub fn new(
        agent_id: String,
        source_entity_id: String,
        target_entity_id: String,
        relationship_type: RelationshipType,
    ) -> Self {
        let now = Utc::now();
        let id = format!(
            "rel_{}_{}_{}_{}",
            agent_id,
            source_entity_id,
            target_entity_id,
            uuid::Uuid::new_v4()
        );

        Self {
            id,
            agent_id,
            source_entity_id,
            target_entity_id,
            relationship_type,
            properties: HashMap::new(),
            first_seen_at: now,
            last_seen_at: now,
            mention_count: 1,
        }
    }

    /// Update the last seen time and increment mention count
    pub fn touch(&mut self) {
        self.last_seen_at = Utc::now();
        self.mention_count += 1;
    }

    /// Add a property
    pub fn with_property(mut self, key: String, value: serde_json::Value) -> Self {
        self.properties.insert(key, value);
        self
    }
}

/// Extracted entities and relationships from a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedKnowledge {
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

/// Direction for neighbor queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Direction {
    /// Entity → Other (outgoing edges)
    Outgoing,
    /// Other → Entity (incoming edges)
    Incoming,
    /// Either direction
    #[default]
    Both,
}

/// Information about a neighboring entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeighborInfo {
    /// The neighboring entity
    pub entity: Entity,
    /// The relationship connecting to the neighbor
    pub relationship: Relationship,
    /// Direction of the relationship from the source entity's perspective
    pub direction: Direction,
}

/// Entity with its connections (incoming and outgoing)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityWithConnections {
    /// The central entity
    pub entity: Entity,
    /// Outgoing relationships: Entity → Other
    pub outgoing: Vec<(Relationship, Entity)>,
    /// Incoming relationships: Other → Entity
    pub incoming: Vec<(Relationship, Entity)>,
}

/// Graph statistics for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    /// Total number of entities
    pub entity_count: usize,
    /// Total number of relationships
    pub relationship_count: usize,
    /// Entity counts by type
    pub entity_types: std::collections::HashMap<String, usize>,
    /// Relationship counts by type
    pub relationship_types: std::collections::HashMap<String, usize>,
    /// Top entities by connection count (entity_name, connection_count)
    pub most_connected_entities: Vec<(String, usize)>,
}

/// Subgraph extracted around a center entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subgraph {
    /// All entities in the subgraph
    pub entities: Vec<Entity>,
    /// All relationships in the subgraph
    pub relationships: Vec<Relationship>,
    /// ID of the center entity
    pub center: String,
    /// Maximum hops from center
    pub max_hops: usize,
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_entity_type_from_str() {
        assert_eq!(EntityType::from_str("person"), EntityType::Person);
        assert_eq!(EntityType::from_str("PERSON"), EntityType::Person);
        assert_eq!(EntityType::from_str("Person"), EntityType::Person);
        assert_eq!(
            EntityType::from_str("organization"),
            EntityType::Organization
        );
        assert_eq!(EntityType::from_str("org"), EntityType::Organization);
        assert_eq!(EntityType::from_str("location"), EntityType::Location);
        assert_eq!(EntityType::from_str("concept"), EntityType::Concept);
        assert_eq!(EntityType::from_str("tool"), EntityType::Tool);
        assert_eq!(EntityType::from_str("project"), EntityType::Project);
        assert_eq!(EntityType::from_str("file"), EntityType::File);
        assert_eq!(EntityType::from_str("File"), EntityType::File);
        assert_eq!(
            EntityType::from_str("custom_type"),
            EntityType::Custom("custom_type".to_string())
        );
    }

    #[test]
    fn test_entity_type_as_str() {
        assert_eq!(EntityType::Person.as_str(), "person");
        assert_eq!(EntityType::Organization.as_str(), "organization");
        assert_eq!(EntityType::Location.as_str(), "location");
        assert_eq!(EntityType::Concept.as_str(), "concept");
        assert_eq!(EntityType::Tool.as_str(), "tool");
        assert_eq!(EntityType::Project.as_str(), "project");
        assert_eq!(EntityType::File.as_str(), "file");
        assert_eq!(EntityType::Custom("custom".to_string()).as_str(), "custom");
    }

    #[test]
    fn test_relationship_type_from_str() {
        assert_eq!(
            RelationshipType::from_str("works_for"),
            RelationshipType::WorksFor
        );
        assert_eq!(
            RelationshipType::from_str("worksfor"),
            RelationshipType::WorksFor
        );
        assert_eq!(
            RelationshipType::from_str("located_in"),
            RelationshipType::LocatedIn
        );
        assert_eq!(
            RelationshipType::from_str("related_to"),
            RelationshipType::RelatedTo
        );
        assert_eq!(
            RelationshipType::from_str("created"),
            RelationshipType::Created
        );
        assert_eq!(RelationshipType::from_str("uses"), RelationshipType::Uses);
        assert_eq!(
            RelationshipType::from_str("part_of"),
            RelationshipType::PartOf
        );
        assert_eq!(
            RelationshipType::from_str("mentions"),
            RelationshipType::Mentions
        );
        assert_eq!(
            RelationshipType::from_str("custom_rel"),
            RelationshipType::Custom("customrel".to_string())
        );
    }

    #[test]
    fn test_relationship_type_as_str() {
        assert_eq!(RelationshipType::WorksFor.as_str(), "works_for");
        assert_eq!(RelationshipType::LocatedIn.as_str(), "located_in");
        assert_eq!(RelationshipType::RelatedTo.as_str(), "related_to");
        assert_eq!(RelationshipType::Created.as_str(), "created");
        assert_eq!(RelationshipType::Uses.as_str(), "uses");
        assert_eq!(RelationshipType::PartOf.as_str(), "part_of");
        assert_eq!(RelationshipType::Mentions.as_str(), "mentions");
        assert_eq!(
            RelationshipType::Custom("custom".to_string()).as_str(),
            "custom"
        );
    }

    #[test]
    fn test_entity_new() {
        let entity = Entity::new(
            "agent-123".to_string(),
            EntityType::Person,
            "John Smith".to_string(),
        );

        assert_eq!(entity.agent_id, "agent-123");
        assert_eq!(entity.name, "John Smith");
        assert!(matches!(entity.entity_type, EntityType::Person));
        assert_eq!(entity.mention_count, 1);
        assert!(entity.properties.is_empty());
        assert!(entity.id.starts_with("entity_agent-123_"));
    }

    #[test]
    fn test_entity_touch() {
        let mut entity = Entity::new(
            "agent-123".to_string(),
            EntityType::Person,
            "John Smith".to_string(),
        );

        let original_count = entity.mention_count;
        entity.touch();

        assert_eq!(entity.mention_count, original_count + 1);
        assert!(entity.last_seen_at > entity.first_seen_at);
    }

    #[test]
    fn test_entity_with_property() {
        let entity = Entity::new(
            "agent-123".to_string(),
            EntityType::Person,
            "John Smith".to_string(),
        )
        .with_property("email".to_string(), json!("john@example.com"))
        .with_property("role".to_string(), json!("Engineer"));

        assert_eq!(entity.properties.len(), 2);
        assert_eq!(
            entity.properties.get("email"),
            Some(&json!("john@example.com"))
        );
        assert_eq!(entity.properties.get("role"), Some(&json!("Engineer")));
    }

    #[test]
    fn test_entity_serialization() {
        let entity = Entity::new(
            "agent-123".to_string(),
            EntityType::Organization,
            "Acme Corp".to_string(),
        );

        let json_str = serde_json::to_string(&entity).unwrap();
        let parsed: Entity = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, entity.id);
        assert_eq!(parsed.name, entity.name);
        assert_eq!(parsed.agent_id, entity.agent_id);
    }

    #[test]
    fn test_relationship_new() {
        let relationship = Relationship::new(
            "agent-123".to_string(),
            "entity-1".to_string(),
            "entity-2".to_string(),
            RelationshipType::WorksFor,
        );

        assert_eq!(relationship.agent_id, "agent-123");
        assert_eq!(relationship.source_entity_id, "entity-1");
        assert_eq!(relationship.target_entity_id, "entity-2");
        assert!(matches!(
            relationship.relationship_type,
            RelationshipType::WorksFor
        ));
        assert_eq!(relationship.mention_count, 1);
        assert!(relationship.properties.is_empty());
    }

    #[test]
    fn test_relationship_touch() {
        let mut relationship = Relationship::new(
            "agent-123".to_string(),
            "entity-1".to_string(),
            "entity-2".to_string(),
            RelationshipType::Uses,
        );

        let original_count = relationship.mention_count;
        relationship.touch();

        assert_eq!(relationship.mention_count, original_count + 1);
        assert!(relationship.last_seen_at > relationship.first_seen_at);
    }

    #[test]
    fn test_relationship_with_property() {
        let relationship = Relationship::new(
            "agent-123".to_string(),
            "entity-1".to_string(),
            "entity-2".to_string(),
            RelationshipType::RelatedTo,
        )
        .with_property("confidence".to_string(), json!(0.9))
        .with_property("context".to_string(), json!("project meeting"));

        assert_eq!(relationship.properties.len(), 2);
        assert_eq!(relationship.properties.get("confidence"), Some(&json!(0.9)));
    }

    #[test]
    fn test_relationship_serialization() {
        let relationship = Relationship::new(
            "agent-123".to_string(),
            "source".to_string(),
            "target".to_string(),
            RelationshipType::PartOf,
        );

        let json_str = serde_json::to_string(&relationship).unwrap();
        let parsed: Relationship = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, relationship.id);
        assert_eq!(parsed.source_entity_id, "source");
        assert_eq!(parsed.target_entity_id, "target");
    }

    #[test]
    fn entity_type_roundtrip_new_variants() {
        assert_eq!(EntityType::from_str("event"), EntityType::Event);
        assert_eq!(EntityType::from_str("time_period"), EntityType::TimePeriod);
        assert_eq!(EntityType::from_str("year"), EntityType::TimePeriod);
        assert_eq!(EntityType::from_str("document"), EntityType::Document);
        assert_eq!(EntityType::from_str("role"), EntityType::Role);
        assert_eq!(EntityType::from_str("artifact"), EntityType::Artifact);
        assert_eq!(EntityType::from_str("ward"), EntityType::Ward);
        assert_eq!(EntityType::from_str("company"), EntityType::Organization);

        // as_str round-trip
        assert_eq!(EntityType::Event.as_str(), "event");
        assert_eq!(EntityType::TimePeriod.as_str(), "time_period");
        assert_eq!(EntityType::Document.as_str(), "document");
        assert_eq!(EntityType::Role.as_str(), "role");
        assert_eq!(EntityType::Artifact.as_str(), "artifact");
        assert_eq!(EntityType::Ward.as_str(), "ward");

        // Confirm none fall through to Custom
        for s in [
            "event",
            "time_period",
            "document",
            "role",
            "artifact",
            "ward",
        ] {
            assert!(
                !matches!(EntityType::from_str(s), EntityType::Custom(_)),
                "'{}' unexpectedly parsed as Custom",
                s
            );
        }
    }

    #[test]
    fn relationship_type_roundtrip_temporal() {
        assert_eq!(
            RelationshipType::from_str("before"),
            RelationshipType::Before
        );
        assert_eq!(RelationshipType::from_str("after"), RelationshipType::After);
        assert_eq!(
            RelationshipType::from_str("during"),
            RelationshipType::During
        );
        assert_eq!(
            RelationshipType::from_str("concurrent_with"),
            RelationshipType::ConcurrentWith
        );
        assert_eq!(
            RelationshipType::from_str("succeeded_by"),
            RelationshipType::SucceededBy
        );
        assert_eq!(
            RelationshipType::from_str("preceded_by"),
            RelationshipType::PrecededBy
        );

        assert_eq!(RelationshipType::ConcurrentWith.as_str(), "concurrent_with");
        assert_eq!(RelationshipType::SucceededBy.as_str(), "succeeded_by");
    }

    #[test]
    fn relationship_type_roundtrip_role_based() {
        assert_eq!(
            RelationshipType::from_str("president_of"),
            RelationshipType::PresidentOf
        );
        assert_eq!(
            RelationshipType::from_str("founder_of"),
            RelationshipType::FounderOf
        );
        assert_eq!(
            RelationshipType::from_str("member_of"),
            RelationshipType::MemberOf
        );
        assert_eq!(
            RelationshipType::from_str("author_of"),
            RelationshipType::AuthorOf
        );
        assert_eq!(
            RelationshipType::from_str("held_role"),
            RelationshipType::HeldRole
        );
        assert_eq!(
            RelationshipType::from_str("employed_by"),
            RelationshipType::EmployedBy
        );

        assert_eq!(RelationshipType::PresidentOf.as_str(), "president_of");
        assert_eq!(RelationshipType::AuthorOf.as_str(), "author_of");
    }

    #[test]
    fn relationship_type_case_insensitive() {
        assert_eq!(
            RelationshipType::from_str("PresidentOf"),
            RelationshipType::PresidentOf
        );
        assert_eq!(
            RelationshipType::from_str("president_of"),
            RelationshipType::PresidentOf
        );
        assert_eq!(
            RelationshipType::from_str("PRESIDENT_OF"),
            RelationshipType::PresidentOf
        );
        assert_eq!(
            RelationshipType::from_str("FounderOf"),
            RelationshipType::FounderOf
        );
        assert_eq!(
            RelationshipType::from_str("TRIGGERED_BY"),
            RelationshipType::TriggeredBy
        );
    }

    #[test]
    fn test_extracted_knowledge_empty() {
        let knowledge = ExtractedKnowledge {
            entities: vec![],
            relationships: vec![],
        };

        assert!(knowledge.entities.is_empty());
        assert!(knowledge.relationships.is_empty());
    }

    #[test]
    fn test_extracted_knowledge_with_data() {
        let entity = Entity::new(
            "agent-1".to_string(),
            EntityType::Person,
            "Alice".to_string(),
        );

        let relationship = Relationship::new(
            "agent-1".to_string(),
            entity.id.clone(),
            "org-1".to_string(),
            RelationshipType::WorksFor,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![entity],
            relationships: vec![relationship],
        };

        assert_eq!(knowledge.entities.len(), 1);
        assert_eq!(knowledge.relationships.len(), 1);
        assert_eq!(knowledge.entities[0].name, "Alice");
    }
}
