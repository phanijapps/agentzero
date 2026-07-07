//! Knowledge graph and wiki mapping between AgentZero DTOs and Engram records.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use engram_domain::{
    Actor, ActorKind, AllowedUse, ChunkId, DeleteMode, DocumentId, EntityId as EngramEntityId,
    EntityKind, EntityRef, HierarchyMemberType, HierarchyMembership, HierarchyNode,
    HierarchyNodeId, HierarchyNodeKind, HierarchyNodeStatus, HierarchyRelation, KnowledgeChunk,
    KnowledgeChunkKind, KnowledgeEntity, KnowledgeRelationship, KnowledgeSource, Metadata, Policy,
    Provenance, RelationshipId as EngramRelationshipId, Retention, SourceDocument,
    SourceDocumentKind, SourceId, SourceLocation, Visibility,
};
use knowledge_graph::types::{Entity, EntityType, Relationship};
use serde_json::{json, Value};
use zbot_stores_domain::WikiArticle;

use crate::{
    error::{AdapterError, AdapterResult},
    scope::ScopeMapper,
};

pub const ZBOT_ENTITY_METADATA_KEY: &str = "zbotEntity";
pub const ZBOT_WIKI_METADATA_KEY: &str = "zbotWikiArticle";

/// Canonical Engram records emitted for a zbot wiki article.
#[derive(Debug, Clone)]
pub struct WikiKnowledgeRecords {
    pub source: KnowledgeSource,
    pub document: SourceDocument,
    pub chunk: KnowledgeChunk,
}

/// Map a zbot graph entity into Engram's source-grounded entity contract.
pub fn entity_to_knowledge_entity(
    entity: &Entity,
    mapper: &ScopeMapper,
) -> AdapterResult<KnowledgeEntity> {
    let ward_id = ward_id_from_properties(&entity.properties);
    let scope = mapper.ward_scope(&ward_id)?;
    let mut metadata = entity_metadata(entity)?;

    lift_string_property(&mut metadata, &entity.properties, "ward_id", "wardId");
    lift_string_property(
        &mut metadata,
        &entity.properties,
        "ontology_id",
        "ontologyId",
    );
    lift_string_property(
        &mut metadata,
        &entity.properties,
        "taxonomy_id",
        "taxonomyId",
    );
    lift_string_property(
        &mut metadata,
        &entity.properties,
        "epistemic_class",
        "epistemicClass",
    );
    lift_string_property(
        &mut metadata,
        &entity.properties,
        "parent_cluster_id",
        "parentClusterId",
    );
    lift_json_property(&mut metadata, &entity.properties, "layer", "layer");
    metadata.insert("agentId".to_string(), json!(entity.agent_id));
    metadata.insert("entityType".to_string(), json!(entity.entity_type.as_str()));
    metadata.insert("mentionCount".to_string(), json!(entity.mention_count));

    Ok(KnowledgeEntity {
        id: EngramEntityId::from(entity.id.clone()),
        graph_id: None,
        kind: entity_kind_for(&entity.entity_type),
        name: entity.name.clone(),
        aliases: aliases_from_properties(&entity.properties),
        scope,
        source_refs: Vec::new(),
        concept_refs: Vec::new(),
        provenance: provenance(
            &entity.agent_id,
            entity.first_seen_at,
            "agentzero.knowledge_graph_adapter",
            confidence_from_value(entity.properties.get("confidence")),
        ),
        created_at: entity.first_seen_at,
        updated_at: Some(entity.last_seen_at),
        metadata: Some(metadata),
    })
}

/// Reconstruct a zbot graph entity from an Engram knowledge entity.
pub fn knowledge_entity_to_entity(record: &KnowledgeEntity) -> AdapterResult<Entity> {
    if let Some(value) = record
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(ZBOT_ENTITY_METADATA_KEY))
    {
        let mut entity: Entity =
            serde_json::from_value(value.clone()).map_err(|error| AdapterError::Mapping {
                field: ZBOT_ENTITY_METADATA_KEY,
                reason: error.to_string(),
            })?;
        entity.name_embedding = None;
        return Ok(entity);
    }

    let metadata = record.metadata.as_ref();
    let mut properties = serde_json::Map::new();
    if let Some(ward_id) =
        metadata_string(metadata, "wardId").or_else(|| record.scope.workspace.clone())
    {
        properties.insert("ward_id".to_string(), json!(ward_id));
    }
    if let Some(ontology_id) = metadata_string(metadata, "ontologyId") {
        properties.insert("ontology_id".to_string(), json!(ontology_id));
    }
    if let Some(taxonomy_id) = metadata_string(metadata, "taxonomyId") {
        properties.insert("taxonomy_id".to_string(), json!(taxonomy_id));
    }
    if let Some(layer) = metadata.and_then(|m| m.get("layer")).cloned() {
        properties.insert("layer".to_string(), layer);
    }

    Ok(Entity {
        id: record.id.as_str().to_string(),
        agent_id: metadata_string(metadata, "agentId")
            .unwrap_or_else(|| record.provenance.actor.id.as_str().to_string()),
        entity_type: entity_type_for(&record.kind, metadata),
        name: record.name.clone(),
        properties: properties.into_iter().collect(),
        first_seen_at: record.created_at,
        last_seen_at: record.updated_at.unwrap_or(record.created_at),
        mention_count: metadata_i64(metadata, "mentionCount").unwrap_or(1),
        name_embedding: None,
    })
}

/// Map a zbot graph relationship into Engram's knowledge relationship contract.
pub fn relationship_to_knowledge_relationship(
    relationship: &Relationship,
    mapper: &ScopeMapper,
) -> AdapterResult<KnowledgeRelationship> {
    let ward_id = ward_id_from_properties(&relationship.properties);
    let scope = mapper.ward_scope(&ward_id)?;

    Ok(KnowledgeRelationship {
        id: EngramRelationshipId::from(relationship.id.clone()),
        graph_id: None,
        subject: EntityRef {
            id: Some(EngramEntityId::from(relationship.source_entity_id.clone())),
            kind: None,
            name: None,
            aliases: Vec::new(),
        },
        predicate: relationship.relationship_type.as_str().to_string(),
        object: EntityRef {
            id: Some(EngramEntityId::from(relationship.target_entity_id.clone())),
            kind: None,
            name: None,
            aliases: Vec::new(),
        },
        scope,
        evidence: Vec::new(),
        confidence: confidence_from_value(relationship.properties.get("confidence")),
        provenance: provenance(
            &relationship.agent_id,
            relationship.first_seen_at,
            "agentzero.knowledge_graph_adapter",
            confidence_from_value(relationship.properties.get("confidence")),
        ),
        created_at: relationship.first_seen_at,
        updated_at: Some(relationship.last_seen_at),
    })
}

/// Map a zbot wiki article into Engram source/document/chunk records.
pub fn wiki_article_to_knowledge_records(
    article: &WikiArticle,
    mapper: &ScopeMapper,
) -> AdapterResult<WikiKnowledgeRecords> {
    let scope = mapper.ward_scope(&article.ward_id)?;
    let created_at = parse_timestamp("created_at", &article.created_at)?;
    let updated_at = parse_timestamp("updated_at", &article.updated_at)?;
    let source_id = SourceId::from(format!("zbot-wiki-source:{}", article.ward_id));
    let document_id = DocumentId::from(format!("zbot-wiki-document:{}", article.id));
    let chunk_id = ChunkId::from(format!("zbot-wiki-chunk:{}", article.id));
    let content_hash = content_hash(&article.content);
    let metadata = wiki_metadata(article)?;
    let policy = durable_workspace_policy();
    let provenance = provenance(
        &article.agent_id,
        created_at,
        "agentzero.wiki_adapter",
        Some(1.0),
    );

    Ok(WikiKnowledgeRecords {
        source: KnowledgeSource {
            id: source_id.clone(),
            kind: engram_domain::SourceKind::Generated,
            scope: scope.clone(),
            name: format!("AgentZero ward wiki ({})", article.ward_id),
            uri: None,
            version: None,
            policy: policy.clone(),
            provenance: provenance.clone(),
            created_at,
            updated_at: Some(updated_at),
            metadata: Some(BTreeMap::from([
                ("agentId".to_string(), json!(article.agent_id)),
                ("wardId".to_string(), json!(article.ward_id)),
                ("sourceKind".to_string(), json!("zbot_wiki")),
            ])),
        },
        document: SourceDocument {
            id: document_id.clone(),
            source_id: source_id.clone(),
            kind: SourceDocumentKind::Markdown,
            uri: None,
            path: Some(article.title.clone()),
            title: Some(article.title.clone()),
            mime_type: Some("text/markdown".to_string()),
            language: None,
            version: Some(article.version.to_string()),
            content_hash: content_hash.clone(),
            provenance: provenance.clone(),
            policy: policy.clone(),
            created_at,
            updated_at: Some(updated_at),
            metadata: Some(BTreeMap::from([
                ("agentId".to_string(), json!(article.agent_id)),
                ("wardId".to_string(), json!(article.ward_id)),
                ("articleId".to_string(), json!(article.id)),
            ])),
        },
        chunk: KnowledgeChunk {
            id: chunk_id,
            document_id,
            source_id,
            kind: KnowledgeChunkKind::DocumentSection,
            text: article.content.clone(),
            summary: None,
            location: Some(SourceLocation {
                path: Some(article.title.clone()),
                start_line: None,
                end_line: None,
                start_offset: None,
                end_offset: None,
                anchor: Some(article.id.clone()),
            }),
            entities: Vec::new(),
            concepts: Vec::new(),
            embedding_refs: Vec::new(),
            content_hash,
            provenance,
            policy,
            created_at,
            updated_at: Some(updated_at),
            metadata: Some(metadata),
        },
    })
}

/// Build an Engram hierarchy node for a promoted aggregate graph entity.
pub fn aggregate_entity_to_hierarchy_node(
    entity: &Entity,
    mapper: &ScopeMapper,
    layer: i64,
    member_ids: &[zbot_stores::types::EntityId],
) -> AdapterResult<HierarchyNode> {
    let ward_id = ward_id_from_properties(&entity.properties);
    let scope = mapper.ward_scope(&ward_id)?;
    let now = Utc::now();
    let provenance = provenance(
        &entity.agent_id,
        entity.first_seen_at,
        "agentzero.hierarchy_adapter",
        Some(1.0),
    );
    let members = member_ids
        .iter()
        .enumerate()
        .map(|(index, member)| HierarchyMembership {
            id: format!("membership:{}:{}", entity.id, member.0),
            parent_id: HierarchyNodeId::from(entity.id.clone()),
            member_type: HierarchyMemberType::Entity,
            member_id: member.0.clone(),
            weight: None,
            rank: Some(index as u32),
            provenance: provenance.clone(),
            created_at: now,
        })
        .collect();

    Ok(HierarchyNode {
        id: HierarchyNodeId::from(entity.id.clone()),
        scope,
        kind: HierarchyNodeKind::Aggregate,
        layer: layer.max(0) as u32,
        name: entity.name.clone(),
        summary: entity
            .properties
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        parent_id: entity
            .properties
            .get("parent_cluster_id")
            .and_then(Value::as_str)
            .map(HierarchyNodeId::from),
        members,
        source_target_type: Some(engram_domain::RetrievalTargetType::Entity),
        source_target_id: Some(entity.id.clone()),
        embedding_refs: Vec::new(),
        status: HierarchyNodeStatus::Active,
        policy: durable_workspace_policy(),
        provenance,
        created_at: entity.first_seen_at,
        updated_at: Some(entity.last_seen_at),
        metadata: Some(BTreeMap::from([
            ("agentId".to_string(), json!(entity.agent_id)),
            ("wardId".to_string(), json!(ward_id)),
            ("entityId".to_string(), json!(entity.id)),
            ("memberCount".to_string(), json!(member_ids.len())),
        ])),
    })
}

/// Build an Engram hierarchy relation for an inter-cluster aggregate edge.
pub fn relationship_to_hierarchy_relation(
    relationship: &Relationship,
    mapper: &ScopeMapper,
) -> AdapterResult<HierarchyRelation> {
    let ward_id = ward_id_from_properties(&relationship.properties);
    let scope = mapper.ward_scope(&ward_id)?;
    let layer = relationship
        .properties
        .get("layer")
        .and_then(Value::as_i64)
        .map(|value| value.max(0) as u32);
    Ok(HierarchyRelation {
        id: relationship.id.clone(),
        scope,
        source_id: HierarchyNodeId::from(relationship.source_entity_id.clone()),
        target_id: HierarchyNodeId::from(relationship.target_entity_id.clone()),
        predicate: relationship.relationship_type.as_str().to_string(),
        layer,
        strength: confidence_from_value(relationship.properties.get("confidence")),
        is_inter_cluster: Some(true),
        evidence: Vec::new(),
        provenance: provenance(
            &relationship.agent_id,
            relationship.first_seen_at,
            "agentzero.hierarchy_adapter",
            confidence_from_value(relationship.properties.get("confidence")),
        ),
        created_at: relationship.first_seen_at,
    })
}

fn entity_metadata(entity: &Entity) -> AdapterResult<Metadata> {
    let mut sidecar = entity.clone();
    sidecar.name_embedding = None;
    let mut metadata = BTreeMap::new();
    metadata.insert(
        ZBOT_ENTITY_METADATA_KEY.to_string(),
        serde_json::to_value(sidecar).map_err(|error| AdapterError::Mapping {
            field: ZBOT_ENTITY_METADATA_KEY,
            reason: error.to_string(),
        })?,
    );
    Ok(metadata)
}

fn wiki_metadata(article: &WikiArticle) -> AdapterResult<Metadata> {
    let mut sidecar = article.clone();
    sidecar.embedding = None;
    Ok(BTreeMap::from([
        (
            ZBOT_WIKI_METADATA_KEY.to_string(),
            serde_json::to_value(sidecar).map_err(|error| AdapterError::Mapping {
                field: ZBOT_WIKI_METADATA_KEY,
                reason: error.to_string(),
            })?,
        ),
        ("agentId".to_string(), json!(article.agent_id)),
        ("wardId".to_string(), json!(article.ward_id)),
        ("articleId".to_string(), json!(article.id)),
        ("title".to_string(), json!(article.title)),
        ("tags".to_string(), json!(article.tags)),
        ("sourceFactIds".to_string(), json!(article.source_fact_ids)),
    ]))
}

fn ward_id_from_properties(properties: &std::collections::HashMap<String, Value>) -> String {
    properties
        .get("ward_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("__global__")
        .to_string()
}

fn entity_kind_for(entity_type: &EntityType) -> EntityKind {
    match entity_type {
        EntityType::Person => EntityKind::Person,
        EntityType::Organization => EntityKind::Organization,
        EntityType::Project => EntityKind::Project,
        EntityType::File => EntityKind::File,
        EntityType::Tool => EntityKind::Tool,
        EntityType::Concept | EntityType::Role | EntityType::Ward => EntityKind::Concept,
        EntityType::Document | EntityType::Artifact => EntityKind::Artifact,
        EntityType::Location | EntityType::Event | EntityType::TimePeriod => EntityKind::Unknown,
        EntityType::Custom(_) => EntityKind::Unknown,
    }
}

fn entity_type_for(kind: &EntityKind, metadata: Option<&Metadata>) -> EntityType {
    if let Some(entity_type) = metadata_string(metadata, "entityType") {
        return EntityType::from_str(&entity_type);
    }

    match kind {
        EntityKind::Person => EntityType::Person,
        EntityKind::Organization => EntityType::Organization,
        EntityKind::Project => EntityType::Project,
        EntityKind::Repository | EntityKind::File | EntityKind::Module => EntityType::File,
        EntityKind::Tool => EntityType::Tool,
        EntityKind::Artifact => EntityType::Artifact,
        EntityKind::Concept
        | EntityKind::ValueStream
        | EntityKind::Requirement
        | EntityKind::Task
        | EntityKind::Class
        | EntityKind::Function
        | EntityKind::Method
        | EntityKind::Variable
        | EntityKind::Api
        | EntityKind::Unknown => EntityType::Concept,
    }
}

fn aliases_from_properties(properties: &std::collections::HashMap<String, Value>) -> Vec<String> {
    match properties.get("aliases") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Some(Value::String(value)) if !value.trim().is_empty() => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn provenance(
    actor_id: &str,
    observed_at: DateTime<Utc>,
    method: &str,
    confidence: Option<f32>,
) -> Provenance {
    Provenance {
        source: method.to_string(),
        actor: Actor {
            id: actor_id.into(),
            kind: ActorKind::Agent,
            display_name: None,
            metadata: None,
        },
        observed_at,
        evidence: Vec::new(),
        derivations: Vec::new(),
        confidence,
        method: Some(method.to_string()),
    }
}

fn durable_workspace_policy() -> Policy {
    Policy {
        visibility: Visibility::Workspace,
        retention: Retention::Durable,
        sensitivity: None,
        allowed_uses: vec![AllowedUse::Retrieval, AllowedUse::Consolidation],
        expires_at: None,
        delete_mode: Some(DeleteMode::Archive),
    }
}

fn parse_timestamp(field: &'static str, value: &str) -> AdapterResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| AdapterError::Mapping {
            field,
            reason: error.to_string(),
        })
}

fn metadata_string(metadata: Option<&Metadata>, key: &str) -> Option<String> {
    metadata?.get(key)?.as_str().map(ToOwned::to_owned)
}

fn metadata_i64(metadata: Option<&Metadata>, key: &str) -> Option<i64> {
    metadata?.get(key)?.as_i64()
}

fn lift_string_property(
    metadata: &mut Metadata,
    properties: &std::collections::HashMap<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = properties.get(source_key).and_then(Value::as_str) {
        metadata.insert(target_key.to_string(), json!(value));
    }
}

fn lift_json_property(
    metadata: &mut Metadata,
    properties: &std::collections::HashMap<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = properties.get(source_key) {
        metadata.insert(target_key.to_string(), value.clone());
    }
}

fn confidence_from_value(value: Option<&Value>) -> Option<f32> {
    value
        .and_then(Value::as_f64)
        .filter(|confidence| (0.0..=1.0).contains(confidence))
        .map(|confidence| confidence as f32)
}

fn content_hash(content: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
