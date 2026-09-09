//! `KnowledgeGraphStore` implementation backed by Engram knowledge/hierarchy records.

use agent_primitives::vec_math::cosine_f64_opt;
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::Utc;
use engram_hierarchy::HierarchyRepository;
use engram_knowledge::KnowledgeRepository;
use knowledge_graph::types::{
    Entity, EntityType, GraphStats, NeighborInfo, Relationship, RelationshipType, Subgraph,
};
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde_json::{json, Value};
use uuid::Uuid;
use zbot_stores::types::{
    Direction, EntityId, Neighbor, RelationshipId, ResolveOutcome, TraversalHit,
};
use zbot_stores::{
    ArchivableEntity, EmbeddingQueryIdentity, EntityNameEmbeddingHit, EntityWithEmbedding,
    ExtractedKnowledge, GraphView, HierarchySummary, InterClusterRelationHit, KgStats,
    KnowledgeGraphStore, LcaPath, ReindexReport, StoreError, StoreOutcome, StoreResult,
    VecIndexHealth,
};

use crate::{
    bootstrap::EngramProvider,
    capabilities::AdapterFeature,
    config::{AdapterConfig, ProviderMode},
    error::{AdapterError, AdapterResult},
    governance::{
        validate_relationship_against_builtin_ontology, GovernanceScope,
        GovernanceValidationFinding, ValidationMode,
    },
    mapping::knowledge::{
        aggregate_entity_to_hierarchy_node_with_governance,
        entity_to_knowledge_entity_with_governance, relationship_to_hierarchy_relation,
        relationship_to_knowledge_relationship,
    },
    scope::ScopeMapper,
};

const SIDECAR_COMPONENT: &str = "knowledge_graph_sidecar";
const DEFAULT_WARD_ID: &str = "__global__";
const RELATIONSHIP_GOVERNANCE_PROPERTY_KEYS: [&str; 3] = [
    "governance_ontology_ids",
    "governance_taxonomy_scheme_ids",
    "governance_record_kind",
];

/// Engram-backed implementation of AgentZero's knowledge graph store trait.
#[derive(Clone)]
pub struct EngramKnowledgeGraphStore {
    knowledge: Arc<dyn KnowledgeRepository>,
    hierarchy: Arc<dyn HierarchyRepository>,
    mapper: ScopeMapper,
    governance: crate::governance::GovernancePolicy,
    sidecar: KnowledgeGraphSidecar,
}

impl EngramKnowledgeGraphStore {
    /// Open an Engram-backed graph store for `ProviderMode::Engram`.
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "knowledge_graph",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        let provider = EngramProvider::open(config.clone())?;
        Self::from_provider(config, &provider)
    }

    /// Build a graph store from an already-bootstrapped Engram provider.
    pub fn from_provider(config: AdapterConfig, provider: &EngramProvider) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "knowledge_graph",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        provider.require_feature(AdapterFeature::KnowledgeGraph)?;
        provider.require_feature(AdapterFeature::Hierarchy)?;
        let mapper = config.scope_mapper()?;
        let knowledge = provider.knowledge()?;
        let hierarchy = provider.hierarchy()?;
        let sidecar = KnowledgeGraphSidecar::open(
            &config.compatibility_store_path("zbot-knowledge-graph.sqlite")?,
            embedding_identity_from_config(&config),
        )?;

        Ok(Self {
            knowledge,
            hierarchy,
            mapper,
            governance: config.governance.clone(),
            sidecar,
        })
    }

    async fn upsert_entity_record(
        &self,
        agent_id: &str,
        mut entity: Entity,
    ) -> StoreResult<EntityId> {
        self.admit_entity_write(agent_id, &mut entity)?;
        self.sidecar
            .claim_entity_owners(agent_id, std::iter::once(entity.id.as_str()))?;
        let id = EntityId(entity.id.clone());
        let embedding = entity.name_embedding.clone();
        self.knowledge
            .put_entity(
                entity_to_knowledge_entity_with_governance(
                    &entity,
                    &self.mapper,
                    Some(&self.governance),
                )
                .map_err(|error| StoreError::Backend(error.to_string()))?,
            )
            .await
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        let mut sidecar_entity = entity;
        sidecar_entity.name_embedding = None;
        self.sidecar
            .store_entity(&sidecar_entity, embedding.as_deref())?;
        Ok(id)
    }

    async fn upsert_relationship_record(
        &self,
        agent_id: &str,
        mut relationship: Relationship,
    ) -> StoreResult<RelationshipId> {
        self.admit_relationship_write(agent_id, &mut relationship)?;
        relationship = self.sidecar.canonicalize_relationship(relationship)?;
        // Deduplication merges durable sidecar properties. Apply the current
        // configured selection afterwards so stale or caller-supplied reserved
        // governance values cannot survive that merge.
        self.persist_relationship_governance_selection(&mut relationship)?;
        let id = RelationshipId(relationship.id.clone());
        let findings = self.validate_relationship(&relationship)?;
        self.knowledge
            .put_relationship(
                relationship_to_knowledge_relationship(&relationship, &self.mapper)
                    .map_err(|error| StoreError::Backend(error.to_string()))?,
            )
            .await
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        self.sidecar.store_relationship(&relationship)?;
        self.sidecar.replace_governance_findings(
            &relationship.id,
            &relationship.agent_id,
            &findings,
        )?;
        Ok(id)
    }

    /// Enforce the minimal contract that the zbot graph model can express at
    /// the adapter boundary. This is intentionally before either Engram or the
    /// compatibility sidecar is mutated, so every public graph-write path gets
    /// the same protection.
    fn admit_entity_write(&self, agent_id: &str, entity: &mut Entity) -> StoreResult<()> {
        validate_agent_id(agent_id)?;
        validate_record_id("entity", &entity.id)?;
        validate_name(&entity.name)?;
        validate_timestamps("entity", entity.first_seen_at, entity.last_seen_at)?;
        validate_mention_count("entity", entity.mention_count)?;

        entity.entity_type = canonical_builtin_entity_type(&entity.entity_type)?;
        entity.agent_id = agent_id.to_string();
        ensure_scope(&self.mapper, &mut entity.properties)?;
        Ok(())
    }

    fn admit_relationship_write(
        &self,
        agent_id: &str,
        relationship: &mut Relationship,
    ) -> StoreResult<()> {
        self.admit_relationship_shape(agent_id, relationship)?;
        self.ensure_relationship_endpoints(agent_id, relationship)
    }

    fn admit_relationship_shape(
        &self,
        agent_id: &str,
        relationship: &mut Relationship,
    ) -> StoreResult<()> {
        validate_agent_id(agent_id)?;
        validate_record_id("relationship", &relationship.id)?;
        validate_record_id("relationship source entity", &relationship.source_entity_id)?;
        validate_record_id("relationship target entity", &relationship.target_entity_id)?;
        if relationship.source_entity_id == relationship.target_entity_id {
            return Err(StoreError::Invalid(
                "relationship source and target must differ".to_string(),
            ));
        }
        validate_timestamps(
            "relationship",
            relationship.first_seen_at,
            relationship.last_seen_at,
        )?;
        validate_mention_count("relationship", relationship.mention_count)?;

        relationship.relationship_type =
            canonical_builtin_relationship_type(&relationship.relationship_type)?;
        relationship.agent_id = agent_id.to_string();
        ensure_scope(&self.mapper, &mut relationship.properties)?;
        Ok(())
    }

    fn ensure_relationship_endpoints_or_batch(
        &self,
        agent_id: &str,
        relationship: &Relationship,
        batch_entity_ids: &std::collections::HashSet<&str>,
    ) -> StoreResult<()> {
        for (role, id) in [
            ("source", relationship.source_entity_id.as_str()),
            ("target", relationship.target_entity_id.as_str()),
        ] {
            if batch_entity_ids.contains(id) {
                continue;
            }
            let entity = self
                .sidecar
                .get_entity(&EntityId(id.to_string()))?
                .ok_or_else(|| {
                    StoreError::Invalid(format!("relationship {role} entity does not exist: {id}"))
                })?;
            if entity.agent_id != agent_id {
                return Err(StoreError::Invalid(format!(
                    "relationship {role} entity belongs to another agent: {id}"
                )));
            }
        }
        Ok(())
    }

    fn ensure_relationship_endpoints(
        &self,
        agent_id: &str,
        relationship: &Relationship,
    ) -> StoreResult<()> {
        for (role, id) in [
            ("source", &relationship.source_entity_id),
            ("target", &relationship.target_entity_id),
        ] {
            let entity = self
                .sidecar
                .get_entity(&EntityId(id.clone()))?
                .ok_or_else(|| {
                    StoreError::Invalid(format!("relationship {role} entity does not exist: {id}"))
                })?;
            if entity.agent_id != agent_id {
                return Err(StoreError::Invalid(format!(
                    "relationship {role} entity belongs to a different agent"
                )));
            }
        }
        Ok(())
    }

    /// Persist the selected policy on the zbot relationship sidecar after
    /// canonicalization. Engram's relationship contract has no metadata field,
    /// so this compatibility record carries the authoritative selection.
    fn persist_relationship_governance_selection(
        &self,
        relationship: &mut Relationship,
    ) -> StoreResult<()> {
        for key in RELATIONSHIP_GOVERNANCE_PROPERTY_KEYS {
            relationship.properties.remove(key);
        }
        let ward_id = relationship_property_string(relationship, "ward_id")
            .or_else(|| {
                self.sidecar
                    .get_entity(&EntityId(relationship.source_entity_id.clone()))
                    .ok()
                    .flatten()
                    .and_then(|entity| property_string(&entity, "ward_id"))
            })
            .unwrap_or_else(|| DEFAULT_WARD_ID.to_string());
        let selection = self.governance.select(GovernanceScope {
            ward_id: Some(&ward_id),
            ..GovernanceScope::default()
        });
        if !selection.ontology_ids.is_empty() {
            relationship.properties.insert(
                "governance_ontology_ids".to_string(),
                json!(selection.ontology_ids),
            );
        }
        if !selection.taxonomy_scheme_ids.is_empty() {
            relationship.properties.insert(
                "governance_taxonomy_scheme_ids".to_string(),
                json!(selection.taxonomy_scheme_ids),
            );
        }
        if !selection.ontology_ids.is_empty() || !selection.taxonomy_scheme_ids.is_empty() {
            relationship
                .properties
                .insert("governance_record_kind".to_string(), json!("relationship"));
        }
        Ok(())
    }

    fn validate_relationship(
        &self,
        relationship: &Relationship,
    ) -> StoreResult<Vec<GovernanceValidationFinding>> {
        if self.governance.validation_mode == ValidationMode::Disabled {
            return Ok(Vec::new());
        }

        let ward_id = relationship_property_string(relationship, "ward_id")
            .or_else(|| {
                self.sidecar
                    .get_entity(&EntityId(relationship.source_entity_id.clone()))
                    .ok()
                    .flatten()
                    .and_then(|entity| property_string(&entity, "ward_id"))
            })
            .unwrap_or_else(|| DEFAULT_WARD_ID.to_string());
        let selection = self.governance.select(GovernanceScope {
            ward_id: Some(&ward_id),
            ..GovernanceScope::default()
        });
        if selection.ontology_ids.is_empty() {
            return Ok(Vec::new());
        }

        let source = self
            .sidecar
            .get_entity(&EntityId(relationship.source_entity_id.clone()))?;
        let target = self
            .sidecar
            .get_entity(&EntityId(relationship.target_entity_id.clone()))?;
        let source = source
            .as_ref()
            .map(|entity| (entity.id.as_str(), &entity.entity_type));
        let target = target
            .as_ref()
            .map(|entity| (entity.id.as_str(), &entity.entity_type));

        let mut findings = Vec::new();
        for ontology_id in selection.ontology_ids {
            findings.extend(validate_relationship_against_builtin_ontology(
                &relationship.id,
                &relationship.relationship_type,
                source,
                target,
                &ontology_id,
            ));
        }
        Ok(findings)
    }

    /// Additive governance diagnostics for later Observatory/API read models.
    pub fn list_governance_findings(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<GovernanceValidationFinding>> {
        self.sidecar.list_governance_findings(agent_id, limit)
    }
}

#[async_trait]
impl KnowledgeGraphStore for EngramKnowledgeGraphStore {
    async fn upsert_entity(&self, agent_id: &str, entity: Entity) -> StoreResult<EntityId> {
        self.upsert_entity_record(agent_id, entity).await
    }

    async fn get_entity(&self, id: &EntityId) -> StoreResult<Option<Entity>> {
        self.sidecar.get_entity(id)
    }

    async fn delete_entity(&self, id: &EntityId) -> StoreResult<()> {
        self.sidecar.delete_entity(id)
    }

    async fn bump_entity_mention(&self, id: &EntityId) -> StoreResult<()> {
        self.sidecar.bump_entity_mention(id)
    }

    async fn add_alias(&self, entity_id: &EntityId, surface: &str) -> StoreResult<()> {
        self.sidecar.add_alias(entity_id, surface)
    }

    async fn resolve_entity(
        &self,
        agent_id: &str,
        entity_type: &EntityType,
        name: &str,
        embedding: Option<&[f32]>,
    ) -> StoreResult<ResolveOutcome> {
        if let Some(entity) = self
            .sidecar
            .find_entity_by_name(agent_id, entity_type, name)?
        {
            return Ok(ResolveOutcome::Match(EntityId(entity.id)));
        }
        if let Some(id) = self.sidecar.resolve_alias(agent_id, entity_type, name)? {
            return Ok(ResolveOutcome::Match(id));
        }
        if let Some(embedding) = embedding {
            if let Some(hit) = self
                .sidecar
                .search_embedding(
                    agent_id,
                    Some(entity_type),
                    embedding,
                    Some(&self.sidecar.embedding_identity),
                    1,
                )?
                .into_iter()
                .next()
            {
                if hit.score >= 0.85 {
                    return Ok(ResolveOutcome::Match(EntityId(hit.entry.entity.id)));
                }
            }
        }
        Ok(ResolveOutcome::NoMatch)
    }

    async fn upsert_relationship(
        &self,
        agent_id: &str,
        relationship: Relationship,
    ) -> StoreResult<RelationshipId> {
        self.upsert_relationship_record(agent_id, relationship)
            .await
    }

    async fn delete_relationship(&self, id: &RelationshipId) -> StoreResult<()> {
        self.sidecar.delete_relationship(id)
    }

    async fn store_knowledge(
        &self,
        agent_id: &str,
        mut knowledge: ExtractedKnowledge,
    ) -> StoreResult<StoreOutcome> {
        let entity_count = knowledge.entities.len() as u64;
        let relationship_count = knowledge.relationships.len() as u64;

        // Preflight the complete batch before the first mutation. In
        // particular, a rejected custom predicate must not leave its otherwise
        // valid endpoint entities partially persisted.
        for entity in &mut knowledge.entities {
            self.admit_entity_write(agent_id, entity)?;
        }
        let batch_entity_ids: std::collections::HashSet<&str> = knowledge
            .entities
            .iter()
            .map(|entity| entity.id.as_str())
            .collect();
        for relationship in &mut knowledge.relationships {
            self.admit_relationship_shape(agent_id, relationship)?;
            self.ensure_relationship_endpoints_or_batch(agent_id, relationship, &batch_entity_ids)?;
        }
        self.sidecar.claim_entity_owners(
            agent_id,
            knowledge.entities.iter().map(|entity| entity.id.as_str()),
        )?;

        for entity in knowledge.entities {
            self.upsert_entity_record(agent_id, entity).await?;
        }
        for relationship in knowledge.relationships {
            self.upsert_relationship_record(agent_id, relationship)
                .await?;
        }
        Ok(StoreOutcome {
            entities_inserted: entity_count,
            entities_merged: 0,
            relationships_inserted: relationship_count,
        })
    }

    async fn get_neighbors(
        &self,
        id: &EntityId,
        direction: Direction,
        limit: usize,
    ) -> StoreResult<Vec<Neighbor>> {
        Ok(self
            .sidecar
            .neighbors(id, direction, limit)?
            .into_iter()
            .map(|row| Neighbor {
                entity_id: row.neighbor_id,
                relationship_id: RelationshipId(row.relationship.id),
                relationship_type: row.relationship.relationship_type.as_str().to_string(),
                direction: row.direction,
            })
            .collect())
    }

    async fn traverse(
        &self,
        seed: &EntityId,
        max_hops: usize,
        limit: usize,
    ) -> StoreResult<Vec<TraversalHit>> {
        let relationships = self
            .sidecar
            .list_relationship_entries(None, None, usize::MAX, 0)?;
        let entities = self
            .sidecar
            .list_entity_entries(None, None, usize::MAX, 0)?;
        let entity_mentions = entities
            .into_iter()
            .map(|entry| (entry.entity.id.clone(), entry.entity.mention_count))
            .collect::<HashMap<_, _>>();
        let mut outgoing: HashMap<String, Vec<&Relationship>> = HashMap::new();
        for relationship in &relationships {
            outgoing
                .entry(relationship.relationship.source_entity_id.clone())
                .or_default()
                .push(&relationship.relationship);
        }

        let mut queue = VecDeque::from([(seed.0.clone(), 0_usize, String::new())]);
        let mut seen = HashSet::from([seed.0.clone()]);
        let mut hits = Vec::new();
        while let Some((current, hop, path)) = queue.pop_front() {
            if hop >= max_hops || hits.len() >= limit {
                continue;
            }
            for relationship in outgoing.get(&current).into_iter().flatten() {
                if !seen.insert(relationship.target_entity_id.clone()) {
                    continue;
                }
                let next_path = append_path(&path, relationship.relationship_type.as_str());
                let next_hop = hop + 1;
                hits.push(TraversalHit {
                    entity_id: EntityId(relationship.target_entity_id.clone()),
                    hop: next_hop,
                    path: next_path.clone(),
                    mention_count: *entity_mentions
                        .get(&relationship.target_entity_id)
                        .unwrap_or(&1),
                });
                if hits.len() >= limit {
                    break;
                }
                queue.push_back((relationship.target_entity_id.clone(), next_hop, next_path));
            }
        }
        Ok(hits)
    }

    async fn search_entities_by_name(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        self.sidecar.search_entities_by_name(agent_id, query, limit)
    }

    async fn get_entity_by_normalized_name(
        &self,
        agent_id: &str,
        normalized_name: &str,
    ) -> StoreResult<Option<Entity>> {
        self.sidecar
            .get_entity_by_normalized_name(agent_id, normalized_name)
    }

    async fn search_entities_view(
        &self,
        agent_id: &str,
        query: &str,
        view: GraphView,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        self.sidecar
            .search_entities_view(agent_id, query, view, limit)
    }

    async fn search_entities_by_name_embedding(
        &self,
        agent_id: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> StoreResult<Vec<EntityNameEmbeddingHit>> {
        let _ = (agent_id, query_embedding, top_k);
        Ok(Vec::new())
    }

    async fn search_entities_by_name_embedding_with_identity(
        &self,
        agent_id: &str,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        top_k: usize,
    ) -> StoreResult<Vec<EntityNameEmbeddingHit>> {
        Ok(self
            .sidecar
            .search_embedding(agent_id, None, query_embedding, query_identity, top_k)?
            .into_iter()
            .map(|hit| EntityNameEmbeddingHit {
                confidence: confidence_for(&hit.entry.entity),
                id: hit.entry.entity.id.clone(),
                name: hit.entry.entity.name,
                entity_type: hit.entry.entity.entity_type.as_str().to_string(),
                distance: (2.0 * (1.0 - hit.score)).max(0.0) as f32,
            })
            .collect())
    }

    async fn reindex_embeddings(&self, _new_dim: usize) -> StoreResult<ReindexReport> {
        Ok(ReindexReport {
            tables_rebuilt: Vec::new(),
            rows_indexed: self.sidecar.count_entity_embeddings()? as u64,
        })
    }

    async fn stats(&self) -> StoreResult<KgStats> {
        Ok(KgStats {
            entity_count: self.sidecar.count_entities(None)? as u64,
            relationship_count: self.sidecar.count_relationships(None)? as u64,
            alias_count: self.sidecar.count_aliases()? as u64,
        })
    }

    async fn list_archivable_orphans(
        &self,
        min_age_hours: u32,
        limit: usize,
    ) -> StoreResult<Vec<ArchivableEntity>> {
        self.sidecar.list_archivable_orphans(min_age_hours, limit)
    }

    async fn mark_entity_archival(&self, id: &EntityId, reason: &str) -> StoreResult<()> {
        self.sidecar.mark_entity_archival(id, reason)
    }

    async fn graph_stats(&self, agent_id: &str) -> StoreResult<GraphStats> {
        self.sidecar.graph_stats(agent_id)
    }

    async fn list_entities(
        &self,
        agent_id: &str,
        entity_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<Entity>> {
        Ok(self
            .sidecar
            .list_entity_entries(Some(agent_id), entity_type, limit, offset)?
            .into_iter()
            .map(|entry| entry.entity)
            .collect())
    }

    async fn list_relationships(
        &self,
        agent_id: &str,
        relationship_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<Relationship>> {
        Ok(self
            .sidecar
            .list_relationship_entries(Some(agent_id), relationship_type, limit, offset)?
            .into_iter()
            .map(|entry| entry.relationship)
            .collect())
    }

    async fn get_neighbors_full(
        &self,
        agent_id: &str,
        entity_id: &str,
        direction: Direction,
        limit: usize,
    ) -> StoreResult<Vec<NeighborInfo>> {
        let id = EntityId(entity_id.to_string());
        let rows = self.sidecar.neighbors(&id, direction, limit)?;
        let mut hydrated = Vec::new();
        for row in rows {
            let Some(entity) = self.sidecar.get_entity(&row.neighbor_id)? else {
                continue;
            };
            if entity.agent_id != agent_id {
                continue;
            }
            hydrated.push(NeighborInfo {
                entity,
                relationship: row.relationship,
                direction: kg_direction(row.direction),
            });
        }
        Ok(hydrated)
    }

    async fn get_subgraph(
        &self,
        agent_id: &str,
        center_entity_id: &str,
        max_hops: usize,
    ) -> StoreResult<Subgraph> {
        self.sidecar.subgraph(agent_id, center_entity_id, max_hops)
    }

    async fn count_all_entities(&self) -> StoreResult<usize> {
        self.sidecar.count_entities(None)
    }

    async fn count_all_relationships(&self) -> StoreResult<usize> {
        self.sidecar.count_relationships(None)
    }

    async fn list_all_entities(
        &self,
        ward_id: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        self.sidecar.list_all_entities(ward_id, entity_type, limit)
    }

    async fn list_all_relationships(&self, limit: usize) -> StoreResult<Vec<Relationship>> {
        Ok(self
            .sidecar
            .list_relationship_entries(None, None, limit, 0)?
            .into_iter()
            .map(|entry| entry.relationship)
            .collect())
    }

    async fn vec_index_health(&self) -> StoreResult<VecIndexHealth> {
        let indexed_rows = self.sidecar.count_entity_embeddings()?;
        Ok(VecIndexHealth {
            tables_present: vec!["kg_name_index".to_string()],
            tables_missing: Vec::new(),
            indexed_rows,
        })
    }

    async fn connectivity_strength(
        &self,
        agent_id: &str,
        cluster_a: &[EntityId],
        cluster_b: &[EntityId],
    ) -> StoreResult<usize> {
        self.sidecar
            .connectivity_strength(agent_id, cluster_a, cluster_b)
    }

    async fn promote_cluster_to_aggregate(
        &self,
        agent_id: &str,
        layer: i64,
        members: &[EntityId],
        name: &str,
        description: &str,
        embedding: Option<Vec<f32>>,
    ) -> StoreResult<EntityId> {
        let ward_id = members
            .iter()
            .find_map(|member| {
                self.sidecar
                    .get_entity(member)
                    .ok()
                    .flatten()
                    .and_then(|entity| ward_id_for_entity(&entity))
            })
            .unwrap_or_else(|| DEFAULT_WARD_ID.to_string());
        let now = Utc::now();
        let mut entity = Entity::new(agent_id.to_string(), EntityType::Concept, name.to_string());
        entity.id = format!("aggregate_{}", Uuid::new_v4());
        entity.first_seen_at = now;
        entity.last_seen_at = now;
        entity.name_embedding = embedding;
        entity
            .properties
            .insert("ward_id".to_string(), json!(ward_id));
        entity.properties.insert("layer".to_string(), json!(layer));
        entity
            .properties
            .insert("description".to_string(), json!(description));
        entity
            .properties
            .insert("member_count".to_string(), json!(members.len()));
        entity.properties.insert(
            "member_ids".to_string(),
            json!(members
                .iter()
                .map(|member| member.0.clone())
                .collect::<Vec<_>>()),
        );
        entity
            .properties
            .insert("epistemic_class".to_string(), json!("current"));

        let id = self.upsert_entity_record(agent_id, entity.clone()).await?;
        self.sidecar.set_members_parent(members, &id)?;
        let node = aggregate_entity_to_hierarchy_node_with_governance(
            &entity,
            &self.mapper,
            layer,
            members,
            Some(&self.governance),
        )
        .map_err(|error| StoreError::Backend(error.to_string()))?;
        self.hierarchy
            .put_node(node)
            .await
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        Ok(id)
    }

    async fn write_inter_cluster_relation(
        &self,
        agent_id: &str,
        layer: i64,
        source_aggregate: &EntityId,
        target_aggregate: &EntityId,
        relationship_type: &str,
    ) -> StoreResult<RelationshipId> {
        let ward_id = self
            .sidecar
            .get_entity(source_aggregate)?
            .and_then(|entity| ward_id_for_entity(&entity))
            .unwrap_or_else(|| DEFAULT_WARD_ID.to_string());
        let mut relationship = Relationship::new(
            agent_id.to_string(),
            source_aggregate.0.clone(),
            target_aggregate.0.clone(),
            RelationshipType::RelatedTo,
        );
        relationship.id = format!("inter_{}", Uuid::new_v4());
        relationship
            .properties
            .insert("ward_id".to_string(), json!(ward_id));
        relationship
            .properties
            .insert("layer".to_string(), json!(layer));
        relationship
            .properties
            .insert("is_inter_cluster".to_string(), json!(true));
        relationship.properties.insert(
            "inter_cluster_relation_label".to_string(),
            json!(relationship_type.trim()),
        );
        relationship
            .properties
            .insert("epistemic_class".to_string(), json!("current"));
        relationship
            .properties
            .insert("confidence".to_string(), json!(1.0));

        let id = self
            .upsert_relationship_record(agent_id, relationship.clone())
            .await?;
        let relation = relationship_to_hierarchy_relation(&relationship, &self.mapper)
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        self.hierarchy
            .put_relation(relation)
            .await
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        Ok(id)
    }

    async fn list_entities_with_embeddings_at_layer(
        &self,
        agent_id: &str,
        layer: i64,
        limit: usize,
    ) -> StoreResult<Vec<EntityWithEmbedding>> {
        self.sidecar
            .list_entities_with_embeddings_at_layer(agent_id, layer, limit)
    }

    async fn compute_lca_path(
        &self,
        agent_id: &str,
        seed_entity_ids: &[EntityId],
    ) -> StoreResult<LcaPath> {
        self.sidecar.compute_lca_path(agent_id, seed_entity_ids)
    }

    async fn list_inter_cluster_relations(
        &self,
        agent_id: &str,
        entity_ids: &[EntityId],
    ) -> StoreResult<Vec<InterClusterRelationHit>> {
        self.sidecar
            .list_inter_cluster_relations(agent_id, entity_ids)
    }

    async fn hierarchy_summary(
        &self,
        agent_id: &str,
        top_n: usize,
    ) -> StoreResult<HierarchySummary> {
        self.sidecar.hierarchy_summary(agent_id, top_n)
    }
}

#[derive(Clone)]
struct KnowledgeGraphSidecar {
    connection: Arc<Mutex<Connection>>,
    embedding_identity: EmbeddingQueryIdentity,
}

#[derive(Debug, Clone)]
struct EntityEntry {
    entity: Entity,
    embedding: Option<Vec<f32>>,
    embedding_identity: Option<EmbeddingQueryIdentity>,
}

#[derive(Debug, Clone)]
struct RelationshipEntry {
    relationship: Relationship,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationshipDedupKey {
    agent_id: String,
    source_entity_id: String,
    target_entity_id: String,
    normalized_predicate: String,
    ward_id: String,
    visibility: String,
}

#[derive(Debug, Clone)]
struct NeighborRow {
    neighbor_id: EntityId,
    relationship: Relationship,
    direction: Direction,
}

#[derive(Debug, Clone)]
struct EmbeddingHit {
    entry: EntityEntry,
    score: f64,
}

impl KnowledgeGraphSidecar {
    fn open(path: &Path, embedding_identity: EmbeddingQueryIdentity) -> AdapterResult<Self> {
        let connection = Connection::open(path).map_err(|error| AdapterError::Storage {
            component: SIDECAR_COMPONENT,
            reason: error.to_string(),
        })?;
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;
                CREATE TABLE IF NOT EXISTS kg_entities (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    entity_type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    first_seen_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL,
                    mention_count INTEGER NOT NULL,
                    properties_json TEXT NOT NULL,
                    entity_json TEXT NOT NULL,
                    embedding_json TEXT,
                    embedding_identity_json TEXT,
                    archived INTEGER NOT NULL DEFAULT 0,
                    pruned INTEGER NOT NULL DEFAULT 0,
                    layer INTEGER NOT NULL DEFAULT 0,
                    parent_cluster_id TEXT,
                    compressed_into TEXT
                );
                CREATE TABLE IF NOT EXISTS kg_entity_owners (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL
                );
                INSERT OR IGNORE INTO kg_entity_owners (id, agent_id)
                    SELECT id, agent_id FROM kg_entities;
                CREATE INDEX IF NOT EXISTS idx_kg_entities_agent_type_name
                    ON kg_entities(agent_id, entity_type, name);
                CREATE INDEX IF NOT EXISTS idx_kg_entities_layer
                    ON kg_entities(agent_id, layer);
                CREATE INDEX IF NOT EXISTS idx_kg_entities_active_layer
                    ON kg_entities(agent_id, pruned, layer);
                CREATE INDEX IF NOT EXISTS idx_kg_entities_mentions
                    ON kg_entities(pruned, mention_count DESC, name);
                CREATE INDEX IF NOT EXISTS idx_kg_entities_type_mentions
                    ON kg_entities(entity_type, pruned, mention_count DESC, name);
                CREATE TABLE IF NOT EXISTS entity_aliases (
                    entity_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    entity_type TEXT NOT NULL,
                    surface TEXT NOT NULL,
                    PRIMARY KEY(entity_id, surface)
                );
                CREATE INDEX IF NOT EXISTS idx_entity_aliases_lookup
                    ON entity_aliases(agent_id, entity_type, surface);
                CREATE TABLE IF NOT EXISTS kg_relationships (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    source_entity_id TEXT NOT NULL,
                    target_entity_id TEXT NOT NULL,
                    relationship_type TEXT NOT NULL,
                    first_seen_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL,
                    mention_count INTEGER NOT NULL,
                    properties_json TEXT NOT NULL,
                    relationship_json TEXT NOT NULL,
                    layer INTEGER NOT NULL DEFAULT 0,
                    is_inter_cluster INTEGER NOT NULL DEFAULT 0,
                    confidence REAL,
                    archived INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS idx_kg_relationships_agent_type
                    ON kg_relationships(agent_id, relationship_type);
                CREATE INDEX IF NOT EXISTS idx_kg_relationships_source
                    ON kg_relationships(source_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_relationships_target
                    ON kg_relationships(target_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_relationships_mentions
                    ON kg_relationships(archived, mention_count DESC, id);
                CREATE INDEX IF NOT EXISTS idx_kg_relationships_active_inter_cluster
                    ON kg_relationships(agent_id, archived, is_inter_cluster);
                CREATE INDEX IF NOT EXISTS idx_kg_relationships_type_mentions
                    ON kg_relationships(relationship_type, archived, mention_count DESC, id);
                CREATE TABLE IF NOT EXISTS kg_governance_findings (
                    id TEXT PRIMARY KEY,
                    relationship_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    ontology_id TEXT NOT NULL,
                    code TEXT NOT NULL,
                    severity TEXT NOT NULL,
                    target_entity_id TEXT,
                    target_entity_type TEXT,
                    message TEXT NOT NULL,
                    finding_json TEXT NOT NULL,
                    detected_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_kg_governance_findings_agent
                    ON kg_governance_findings(agent_id, detected_at);
                CREATE INDEX IF NOT EXISTS idx_kg_governance_findings_relationship
                    ON kg_governance_findings(relationship_id);
                "#,
            )
            .map_err(|error| AdapterError::Storage {
                component: SIDECAR_COMPONENT,
                reason: error.to_string(),
            })?;
        ensure_optional_column(
            &connection,
            "kg_entities",
            "embedding_identity_json",
            "TEXT",
        )
        .map_err(|error| AdapterError::Storage {
            component: SIDECAR_COMPONENT,
            reason: error.to_string(),
        })?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            embedding_identity,
        })
    }

    /// Atomically reserve entity IDs for one agent. Bulk callers claim the
    /// entire batch before any Engram or graph-row mutation.
    fn claim_entity_owners<'a>(
        &self,
        agent_id: &str,
        entity_ids: impl IntoIterator<Item = &'a str>,
    ) -> StoreResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_backend)?;
        for entity_id in entity_ids {
            transaction
                .execute(
                    "INSERT INTO kg_entity_owners (id, agent_id) VALUES (?1, ?2) ON CONFLICT(id) DO NOTHING",
                    params![entity_id, agent_id],
                )
                .map_err(to_backend)?;
            let owner: String = transaction
                .query_row(
                    "SELECT agent_id FROM kg_entity_owners WHERE id = ?1",
                    params![entity_id],
                    |row| row.get(0),
                )
                .map_err(to_backend)?;
            if owner != agent_id {
                return Err(StoreError::Invalid(format!(
                    "entity id belongs to another agent: {entity_id}"
                )));
            }
        }
        transaction.commit().map_err(to_backend)
    }

    fn store_entity(&self, entity: &Entity, embedding: Option<&[f32]>) -> StoreResult<()> {
        self.store_entity_inner(entity, embedding, true)
    }

    fn replace_entity(&self, entity: &Entity, embedding: Option<&[f32]>) -> StoreResult<()> {
        self.store_entity_inner(entity, embedding, false)
    }

    fn store_entity_inner(
        &self,
        entity: &Entity,
        embedding: Option<&[f32]>,
        bump_existing: bool,
    ) -> StoreResult<()> {
        let existing = self.get_entity_entry(&EntityId(entity.id.clone()))?;
        let mut stored = entity.clone();
        let embedding_json = embedding
            .map(serde_json::to_string)
            .transpose()
            .map_err(to_backend)?;
        let embedding_identity_json = embedding.map(|_| encode_identity(&self.embedding_identity));
        if let Some(existing) = existing {
            stored.first_seen_at = existing.entity.first_seen_at;
            if bump_existing {
                stored.mention_count = stored
                    .mention_count
                    .max(existing.entity.mention_count.saturating_add(1));
            }
            if embedding.is_none() {
                stored.name_embedding = existing.embedding;
            }
        }
        let stored_embedding = stored.name_embedding.clone();
        stored.name_embedding = None;
        let entity_json = serde_json::to_string(&stored).map_err(to_backend)?;
        let properties_json = serde_json::to_string(&stored.properties).map_err(to_backend)?;
        let layer = property_i64(&stored, "layer").unwrap_or(0);
        let parent_cluster_id = property_string(&stored, "parent_cluster_id");
        self.lock()?
            .execute(
                r#"
                INSERT INTO kg_entities
                    (id, agent_id, entity_type, name, first_seen_at, last_seen_at,
                     mention_count, properties_json, entity_json, embedding_json,
                     embedding_identity_json, layer, parent_cluster_id)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(id) DO UPDATE SET
                    entity_type = excluded.entity_type,
                    name = excluded.name,
                    first_seen_at = excluded.first_seen_at,
                    last_seen_at = excluded.last_seen_at,
                    mention_count = excluded.mention_count,
                    properties_json = excluded.properties_json,
                    entity_json = excluded.entity_json,
                    embedding_json = COALESCE(excluded.embedding_json, kg_entities.embedding_json),
                    embedding_identity_json = COALESCE(excluded.embedding_identity_json, kg_entities.embedding_identity_json),
                    layer = excluded.layer,
                    parent_cluster_id = excluded.parent_cluster_id
                "#,
                params![
                    stored.id,
                    stored.agent_id,
                    stored.entity_type.as_str(),
                    stored.name,
                    stored.first_seen_at.to_rfc3339(),
                    stored.last_seen_at.to_rfc3339(),
                    stored.mention_count,
                    properties_json,
                    entity_json,
                    embedding_json.or_else(|| {
                        stored_embedding
                            .as_deref()
                            .map(serde_json::to_string)
                            .transpose()
                            .ok()
                            .flatten()
                    }),
                    embedding_identity_json,
                    layer,
                    parent_cluster_id,
                ],
            )
            .map_err(to_backend)?;
        Ok(())
    }

    fn get_entity(&self, id: &EntityId) -> StoreResult<Option<Entity>> {
        Ok(self.get_entity_entry(id)?.map(|entry| entry.entity))
    }

    fn get_entity_entry(&self, id: &EntityId) -> StoreResult<Option<EntityEntry>> {
        self.lock()?
            .query_row(
                "SELECT entity_json, embedding_json, embedding_identity_json FROM kg_entities WHERE id = ?1 AND pruned = 0",
                params![id.0],
                decode_entity_entry,
            )
            .optional()
            .map_err(to_backend)
    }

    fn delete_entity(&self, id: &EntityId) -> StoreResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM entity_aliases WHERE entity_id = ?1",
                params![id.0],
            )
            .map_err(to_backend)?;
        connection
            .execute(
                "DELETE FROM kg_relationships WHERE source_entity_id = ?1 OR target_entity_id = ?1",
                params![id.0],
            )
            .map_err(to_backend)?;
        connection
            .execute("DELETE FROM kg_entities WHERE id = ?1", params![id.0])
            .map_err(to_backend)?;
        Ok(())
    }

    fn bump_entity_mention(&self, id: &EntityId) -> StoreResult<()> {
        let Some(mut entry) = self.get_entity_entry(id)? else {
            return Err(StoreError::NotFound);
        };
        entry.entity.touch();
        self.replace_entity(&entry.entity, entry.embedding.as_deref())
    }

    fn add_alias(&self, entity_id: &EntityId, surface: &str) -> StoreResult<()> {
        let Some(entity) = self.get_entity(entity_id)? else {
            return Err(StoreError::NotFound);
        };
        self.lock()?
            .execute(
                "INSERT OR IGNORE INTO entity_aliases (entity_id, agent_id, entity_type, surface)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    entity_id.0,
                    entity.agent_id,
                    entity.entity_type.as_str(),
                    surface,
                ],
            )
            .map_err(to_backend)?;
        Ok(())
    }

    fn resolve_alias(
        &self,
        agent_id: &str,
        entity_type: &EntityType,
        surface: &str,
    ) -> StoreResult<Option<EntityId>> {
        self.lock()?
            .query_row(
                "SELECT entity_id FROM entity_aliases
                 WHERE agent_id = ?1 AND entity_type = ?2 AND lower(surface) = lower(?3)
                 LIMIT 1",
                params![agent_id, entity_type.as_str(), surface],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|id| id.map(EntityId))
            .map_err(to_backend)
    }

    fn find_entity_by_name(
        &self,
        agent_id: &str,
        entity_type: &EntityType,
        name: &str,
    ) -> StoreResult<Option<Entity>> {
        self.lock()?
            .query_row(
                "SELECT entity_json, embedding_json, embedding_identity_json FROM kg_entities
                 WHERE agent_id = ?1 AND entity_type = ?2 AND lower(name) = lower(?3)
                   AND pruned = 0
                 ORDER BY mention_count DESC LIMIT 1",
                params![agent_id, entity_type.as_str(), name],
                decode_entity_entry,
            )
            .optional()
            .map(|entry| entry.map(|entry| entry.entity))
            .map_err(to_backend)
    }

    fn store_relationship(&self, relationship: &Relationship) -> StoreResult<()> {
        let existing = self.get_relationship_entry(&RelationshipId(relationship.id.clone()))?;
        let mut stored = relationship.clone();
        if let Some(existing) = existing {
            stored.first_seen_at = existing.relationship.first_seen_at;
            stored.mention_count = stored
                .mention_count
                .max(existing.relationship.mention_count.saturating_add(1));
        }
        let properties_json = serde_json::to_string(&stored.properties).map_err(to_backend)?;
        let relationship_json = serde_json::to_string(&stored).map_err(to_backend)?;
        let layer = relationship_property_i64(&stored, "layer").unwrap_or(0);
        let is_inter_cluster =
            relationship_property_bool(&stored, "is_inter_cluster").unwrap_or(false);
        let confidence = relationship_property_f64(&stored, "confidence");
        self.lock()?
            .execute(
                r#"
                INSERT INTO kg_relationships
                    (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                     first_seen_at, last_seen_at, mention_count, properties_json,
                     relationship_json, layer, is_inter_cluster, confidence)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(id) DO UPDATE SET
                    agent_id = excluded.agent_id,
                    source_entity_id = excluded.source_entity_id,
                    target_entity_id = excluded.target_entity_id,
                    relationship_type = excluded.relationship_type,
                    first_seen_at = excluded.first_seen_at,
                    last_seen_at = excluded.last_seen_at,
                    mention_count = excluded.mention_count,
                    properties_json = excluded.properties_json,
                    relationship_json = excluded.relationship_json,
                    layer = excluded.layer,
                    is_inter_cluster = excluded.is_inter_cluster,
                    confidence = excluded.confidence
                "#,
                params![
                    stored.id,
                    stored.agent_id,
                    stored.source_entity_id,
                    stored.target_entity_id,
                    stored.relationship_type.as_str(),
                    stored.first_seen_at.to_rfc3339(),
                    stored.last_seen_at.to_rfc3339(),
                    stored.mention_count,
                    properties_json,
                    relationship_json,
                    layer,
                    is_inter_cluster,
                    confidence,
                ],
            )
            .map_err(to_backend)?;
        Ok(())
    }

    fn canonicalize_relationship(&self, relationship: Relationship) -> StoreResult<Relationship> {
        let key = self.relationship_dedup_key(&relationship)?;
        let Some(existing) = self.find_duplicate_relationship(&key, &relationship.id)? else {
            return Ok(relationship);
        };

        Ok(merge_duplicate_relationships(
            existing.relationship,
            relationship,
        ))
    }

    fn replace_governance_findings(
        &self,
        relationship_id: &str,
        agent_id: &str,
        findings: &[GovernanceValidationFinding],
    ) -> StoreResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_backend)?;
        transaction
            .execute(
                "DELETE FROM kg_governance_findings WHERE relationship_id = ?1",
                params![relationship_id],
            )
            .map_err(to_backend)?;
        for finding in findings {
            let finding_json = serde_json::to_string(finding).map_err(to_backend)?;
            transaction
                .execute(
                    r#"
                    INSERT INTO kg_governance_findings
                        (id, relationship_id, agent_id, ontology_id, code, severity,
                         target_entity_id, target_entity_type, message, finding_json, detected_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    ON CONFLICT(id) DO UPDATE SET
                        relationship_id = excluded.relationship_id,
                        agent_id = excluded.agent_id,
                        ontology_id = excluded.ontology_id,
                        code = excluded.code,
                        severity = excluded.severity,
                        target_entity_id = excluded.target_entity_id,
                        target_entity_type = excluded.target_entity_type,
                        message = excluded.message,
                        finding_json = excluded.finding_json,
                        detected_at = excluded.detected_at
                    "#,
                    params![
                        finding.id,
                        relationship_id,
                        agent_id,
                        finding.ontology_id,
                        finding.code,
                        format!("{:?}", finding.severity).to_lowercase(),
                        finding.target_entity_id,
                        finding.target_entity_type,
                        finding.message,
                        finding_json,
                        Utc::now().to_rfc3339(),
                    ],
                )
                .map_err(to_backend)?;
        }
        transaction.commit().map_err(to_backend)?;
        Ok(())
    }

    fn list_governance_findings(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<GovernanceValidationFinding>> {
        let limit = limit.max(1) as i64;
        let connection = self.lock()?;
        if let Some(agent_id) = agent_id {
            let mut statement = connection
                .prepare(
                    "SELECT finding_json FROM kg_governance_findings
                     WHERE agent_id = ?1 ORDER BY detected_at DESC, id LIMIT ?2",
                )
                .map_err(to_backend)?;
            let rows = statement
                .query_map(params![agent_id, limit], decode_governance_finding)
                .map_err(to_backend)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(to_backend)
        } else {
            let mut statement = connection
                .prepare(
                    "SELECT finding_json FROM kg_governance_findings
                     ORDER BY detected_at DESC, id LIMIT ?1",
                )
                .map_err(to_backend)?;
            let rows = statement
                .query_map(params![limit], decode_governance_finding)
                .map_err(to_backend)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(to_backend)
        }
    }

    fn get_relationship_entry(
        &self,
        id: &RelationshipId,
    ) -> StoreResult<Option<RelationshipEntry>> {
        self.lock()?
            .query_row(
                "SELECT relationship_json FROM kg_relationships WHERE id = ?1 AND archived = 0",
                params![id.0],
                decode_relationship_entry,
            )
            .optional()
            .map_err(to_backend)
    }

    fn delete_relationship(&self, id: &RelationshipId) -> StoreResult<()> {
        self.lock()?
            .execute("DELETE FROM kg_relationships WHERE id = ?1", params![id.0])
            .map_err(to_backend)?;
        Ok(())
    }

    fn neighbors(
        &self,
        id: &EntityId,
        direction: Direction,
        limit: usize,
    ) -> StoreResult<Vec<NeighborRow>> {
        let relationships = self.list_relationship_entries(None, None, usize::MAX, 0)?;
        let mut rows = Vec::new();
        for entry in relationships {
            let relationship = entry.relationship;
            let outgoing = relationship.source_entity_id == id.0;
            let incoming = relationship.target_entity_id == id.0;
            let include = match direction {
                Direction::Outgoing => outgoing,
                Direction::Incoming => incoming,
                Direction::Both => outgoing || incoming,
            };
            if !include {
                continue;
            }
            let row_direction = if outgoing {
                Direction::Outgoing
            } else {
                Direction::Incoming
            };
            let neighbor_id = if outgoing {
                EntityId(relationship.target_entity_id.clone())
            } else {
                EntityId(relationship.source_entity_id.clone())
            };
            rows.push(NeighborRow {
                neighbor_id,
                relationship,
                direction: row_direction,
            });
            if rows.len() >= limit {
                break;
            }
        }
        Ok(rows)
    }

    fn search_entities_by_name(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        let query = query.to_lowercase();
        let mut rows = self.list_entity_entries(Some(agent_id), None, usize::MAX, 0)?;
        rows.retain(|entry| entry.entity.name.to_lowercase().contains(&query));
        rows.sort_by(|left, right| {
            right
                .entity
                .mention_count
                .cmp(&left.entity.mention_count)
                .then_with(|| left.entity.name.cmp(&right.entity.name))
        });
        rows.truncate(limit.max(1));
        Ok(rows.into_iter().map(|entry| entry.entity).collect())
    }

    fn get_entity_by_normalized_name(
        &self,
        agent_id: &str,
        normalized_name: &str,
    ) -> StoreResult<Option<Entity>> {
        let connection = self.lock()?;
        let entity_json = connection
            .query_row(
                "SELECT entity_json FROM kg_entities
                 WHERE agent_id = ?1
                   AND pruned = 0
                   AND lower(trim(name)) = lower(trim(?2))
                 LIMIT 1",
                params![agent_id, normalized_name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(to_backend)?;
        entity_json
            .map(|json| serde_json::from_str(&json).map_err(to_backend))
            .transpose()
    }

    fn search_entities_view(
        &self,
        agent_id: &str,
        query: &str,
        view: GraphView,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        let query = query.to_lowercase();
        let mut rows = self.list_entity_entries(Some(agent_id), None, usize::MAX, 0)?;
        rows.retain(|entry| entry.entity.name.to_lowercase().contains(&query));
        match view {
            GraphView::Temporal => rows.sort_by(|left, right| {
                right
                    .entity
                    .last_seen_at
                    .cmp(&left.entity.last_seen_at)
                    .then_with(|| left.entity.name.cmp(&right.entity.name))
            }),
            GraphView::Entity => {
                let degrees = self.degrees()?;
                rows.sort_by(|left, right| {
                    degrees
                        .get(&right.entity.id)
                        .unwrap_or(&0)
                        .cmp(degrees.get(&left.entity.id).unwrap_or(&0))
                        .then_with(|| left.entity.name.cmp(&right.entity.name))
                });
            }
            GraphView::Semantic | GraphView::Hybrid => rows.sort_by(|left, right| {
                right
                    .entity
                    .mention_count
                    .cmp(&left.entity.mention_count)
                    .then_with(|| left.entity.name.cmp(&right.entity.name))
            }),
        }
        rows.truncate(limit.max(1));
        Ok(rows.into_iter().map(|entry| entry.entity).collect())
    }

    fn search_embedding(
        &self,
        agent_id: &str,
        entity_type: Option<&EntityType>,
        query_embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        limit: usize,
    ) -> StoreResult<Vec<EmbeddingHit>> {
        if !identity_compatible(
            &self.embedding_identity,
            query_identity,
            query_embedding.len(),
        ) {
            return Ok(Vec::new());
        }
        let mut hits = self
            .list_entity_entries(
                Some(agent_id),
                entity_type.map(EntityType::as_str),
                usize::MAX,
                0,
            )?
            .into_iter()
            .filter_map(|entry| {
                if !identity_compatible(
                    &self.embedding_identity,
                    entry.embedding_identity.as_ref(),
                    entry.embedding.as_ref()?.len(),
                ) {
                    return None;
                }
                let score = cosine_f64_opt(query_embedding, entry.embedding.as_deref()?)?;
                Some(EmbeddingHit { entry, score })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit.max(1));
        Ok(hits)
    }

    fn list_entity_entries(
        &self,
        agent_id: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<EntityEntry>> {
        let connection = self.lock()?;
        let mut conditions = vec!["pruned = 0".to_string()];
        let mut param_values: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(agent_id) = agent_id {
            conditions.push(format!("agent_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(agent_id.to_string()));
        }
        if let Some(entity_type) = entity_type {
            conditions.push(format!("entity_type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(entity_type.to_string()));
        }

        let mut sql = format!(
            "SELECT entity_json, embedding_json, embedding_identity_json FROM kg_entities
             WHERE {}
             ORDER BY mention_count DESC, name",
            conditions.join(" AND ")
        );
        if limit != usize::MAX {
            sql.push_str(&format!(
                " LIMIT ?{} OFFSET ?{}",
                param_values.len() + 1,
                param_values.len() + 2
            ));
            param_values.push(Box::new(limit.max(1) as i64));
            param_values.push(Box::new(offset as i64));
        }

        let params_refs: Vec<&dyn ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
        let mut statement = connection.prepare(&sql).map_err(to_backend)?;
        let rows = statement
            .query_map(params_refs.as_slice(), decode_entity_entry)
            .map_err(to_backend)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(to_backend)
    }

    fn list_relationship_entries(
        &self,
        agent_id: Option<&str>,
        relationship_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<RelationshipEntry>> {
        let mut conditions = vec!["archived = 0".to_string()];
        let mut param_values: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(agent_id) = agent_id {
            conditions.push(format!("agent_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(agent_id.to_string()));
        }
        if let Some(relationship_type) = relationship_type {
            conditions.push(format!("relationship_type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(relationship_type.to_string()));
        }

        let candidate_limit = offset.saturating_add(limit.max(1).saturating_mul(4));
        let mut sql = format!(
            "SELECT relationship_json FROM kg_relationships
             WHERE {}
             ORDER BY mention_count DESC, id",
            conditions.join(" AND ")
        );
        if limit != usize::MAX {
            sql.push_str(&format!(" LIMIT ?{}", param_values.len() + 1,));
            param_values.push(Box::new(candidate_limit as i64));
        }

        let rows = {
            let connection = self.lock()?;
            let params_refs: Vec<&dyn ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
            let mut statement = connection.prepare(&sql).map_err(to_backend)?;
            let rows = statement
                .query_map(params_refs.as_slice(), decode_relationship_entry)
                .map_err(to_backend)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(to_backend)?
        };
        let mut rows = self.deduplicate_relationship_entries(rows)?;
        rows.sort_by(|left, right| {
            right
                .relationship
                .mention_count
                .cmp(&left.relationship.mention_count)
                .then_with(|| left.relationship.id.cmp(&right.relationship.id))
        });
        Ok(rows.into_iter().skip(offset).take(limit.max(1)).collect())
    }

    fn load_all_entities(&self) -> StoreResult<Vec<EntityEntry>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT entity_json, embedding_json, embedding_identity_json FROM kg_entities
                 WHERE pruned = 0 ORDER BY mention_count DESC, name",
            )
            .map_err(to_backend)?;
        let rows = statement
            .query_map([], decode_entity_entry)
            .map_err(to_backend)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(to_backend)
    }

    fn load_all_relationships(&self) -> StoreResult<Vec<RelationshipEntry>> {
        let mut rows = self.load_all_relationship_rows()?;
        rows = self.deduplicate_relationship_entries(rows)?;
        rows.sort_by(|left, right| {
            right
                .relationship
                .mention_count
                .cmp(&left.relationship.mention_count)
                .then_with(|| left.relationship.id.cmp(&right.relationship.id))
        });
        Ok(rows)
    }

    fn load_all_relationship_rows(&self) -> StoreResult<Vec<RelationshipEntry>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT relationship_json FROM kg_relationships
                 WHERE archived = 0 ORDER BY mention_count DESC, id",
            )
            .map_err(to_backend)?;
        let rows = statement
            .query_map([], decode_relationship_entry)
            .map_err(to_backend)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(to_backend)
    }

    fn find_duplicate_relationship(
        &self,
        key: &RelationshipDedupKey,
        incoming_id: &str,
    ) -> StoreResult<Option<RelationshipEntry>> {
        for entry in self.load_all_relationship_rows()? {
            if entry.relationship.id == incoming_id {
                continue;
            }
            if self.relationship_dedup_key(&entry.relationship)? == *key {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    fn deduplicate_relationship_entries(
        &self,
        rows: Vec<RelationshipEntry>,
    ) -> StoreResult<Vec<RelationshipEntry>> {
        let mut deduped: Vec<(RelationshipDedupKey, RelationshipEntry)> = Vec::new();
        for entry in rows {
            let key = self.relationship_dedup_key(&entry.relationship)?;
            if let Some((_, existing)) = deduped.iter_mut().find(|(candidate, _)| *candidate == key)
            {
                existing.relationship = merge_duplicate_relationships(
                    existing.relationship.clone(),
                    entry.relationship,
                );
            } else {
                deduped.push((key, entry));
            }
        }
        Ok(deduped.into_iter().map(|(_, entry)| entry).collect())
    }

    fn relationship_dedup_key(
        &self,
        relationship: &Relationship,
    ) -> StoreResult<RelationshipDedupKey> {
        let ward_id = relationship_property_string(relationship, "ward_id")
            .or_else(|| {
                self.get_entity(&EntityId(relationship.source_entity_id.clone()))
                    .ok()
                    .flatten()
                    .and_then(|entity| property_string(&entity, "ward_id"))
            })
            .unwrap_or_else(|| DEFAULT_WARD_ID.to_string());
        Ok(RelationshipDedupKey {
            agent_id: relationship.agent_id.clone(),
            source_entity_id: relationship.source_entity_id.clone(),
            target_entity_id: relationship.target_entity_id.clone(),
            normalized_predicate: normalize_relationship_predicate(
                relationship.relationship_type.as_str(),
            ),
            ward_id: normalize_key_part(&ward_id),
            visibility: relationship_property_string(relationship, "visibility")
                .map(|value| normalize_key_part(&value))
                .unwrap_or_else(|| "workspace".to_string()),
        })
    }

    fn count_entities(&self, agent_id: Option<&str>) -> StoreResult<usize> {
        let connection = self.lock()?;
        let count = match agent_id {
            Some(agent_id) => connection.query_row(
                "SELECT COUNT(*) FROM kg_entities WHERE pruned = 0 AND agent_id = ?1",
                params![agent_id],
                |row| row.get::<_, i64>(0),
            ),
            None => connection.query_row(
                "SELECT COUNT(*) FROM kg_entities WHERE pruned = 0",
                [],
                |row| row.get::<_, i64>(0),
            ),
        }
        .map_err(to_backend)?;
        Ok(count.max(0) as usize)
    }

    fn count_relationships(&self, agent_id: Option<&str>) -> StoreResult<usize> {
        let connection = self.lock()?;
        let count = match agent_id {
            Some(agent_id) => connection.query_row(
                "SELECT COUNT(*) FROM kg_relationships WHERE archived = 0 AND agent_id = ?1",
                params![agent_id],
                |row| row.get::<_, i64>(0),
            ),
            None => connection.query_row(
                "SELECT COUNT(*) FROM kg_relationships WHERE archived = 0",
                [],
                |row| row.get::<_, i64>(0),
            ),
        }
        .map_err(to_backend)?;
        Ok(count.max(0) as usize)
    }

    fn count_aliases(&self) -> StoreResult<usize> {
        self.lock()?
            .query_row("SELECT COUNT(*) FROM entity_aliases", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count.max(0) as usize)
            .map_err(to_backend)
    }

    fn count_entity_embeddings(&self) -> StoreResult<usize> {
        self.lock()?
            .query_row(
                "SELECT COUNT(*) FROM kg_entities WHERE embedding_json IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(to_backend)
    }

    fn list_archivable_orphans(
        &self,
        min_age_hours: u32,
        limit: usize,
    ) -> StoreResult<Vec<ArchivableEntity>> {
        let cutoff = Utc::now() - chrono::Duration::hours(i64::from(min_age_hours));
        let connected = self.connected_entity_ids()?;
        let mut rows = self
            .load_all_entities()?
            .into_iter()
            .filter(|entry| {
                entry.entity.mention_count <= 1
                    && entry.entity.first_seen_at < cutoff
                    && !connected.contains(&entry.entity.id)
                    && property_string(&entry.entity, "epistemic_class").as_deref()
                        != Some("archival")
                    && confidence_for(&entry.entity) < 0.5
            })
            .map(|entry| ArchivableEntity {
                entity_id: EntityId(entry.entity.id),
                agent_id: entry.entity.agent_id,
                entity_type: entry.entity.entity_type.as_str().to_string(),
                name: entry.entity.name,
            })
            .collect::<Vec<_>>();
        rows.truncate(limit.max(1));
        Ok(rows)
    }

    fn mark_entity_archival(&self, id: &EntityId, reason: &str) -> StoreResult<()> {
        let Some(mut entry) = self.get_entity_entry(id)? else {
            return Err(StoreError::NotFound);
        };
        entry
            .entity
            .properties
            .insert("epistemic_class".to_string(), json!("archival"));
        entry
            .entity
            .properties
            .insert("compressed_into".to_string(), json!(reason));
        self.replace_entity(&entry.entity, entry.embedding.as_deref())
    }

    fn graph_stats(&self, agent_id: &str) -> StoreResult<GraphStats> {
        let entities = self.list_entity_entries(Some(agent_id), None, usize::MAX, 0)?;
        let relationships = self.list_relationship_entries(Some(agent_id), None, usize::MAX, 0)?;
        let mut entity_types = HashMap::new();
        for entry in &entities {
            *entity_types
                .entry(entry.entity.entity_type.as_str().to_string())
                .or_insert(0) += 1;
        }
        let mut relationship_types = HashMap::new();
        for entry in &relationships {
            *relationship_types
                .entry(entry.relationship.relationship_type.as_str().to_string())
                .or_insert(0) += 1;
        }
        let degrees = self.degrees()?;
        let mut most_connected_entities = entities
            .iter()
            .map(|entry| {
                (
                    entry.entity.name.clone(),
                    *degrees.get(&entry.entity.id).unwrap_or(&0),
                )
            })
            .collect::<Vec<_>>();
        most_connected_entities.sort_by_key(|entity| std::cmp::Reverse(entity.1));
        most_connected_entities.truncate(10);
        Ok(GraphStats {
            entity_count: entities.len(),
            relationship_count: relationships.len(),
            entity_types,
            relationship_types,
            most_connected_entities,
        })
    }

    fn subgraph(
        &self,
        agent_id: &str,
        center_entity_id: &str,
        max_hops: usize,
    ) -> StoreResult<Subgraph> {
        let mut visited = HashSet::from([center_entity_id.to_string()]);
        let mut queue = VecDeque::from([(center_entity_id.to_string(), 0_usize)]);
        let mut relationship_ids = HashSet::new();
        while let Some((current, hop)) = queue.pop_front() {
            if hop >= max_hops {
                continue;
            }
            for row in self.neighbors(&EntityId(current.clone()), Direction::Both, usize::MAX)? {
                relationship_ids.insert(row.relationship.id.clone());
                if visited.insert(row.neighbor_id.0.clone()) {
                    queue.push_back((row.neighbor_id.0, hop + 1));
                }
            }
        }

        let entities = self
            .load_all_entities()?
            .into_iter()
            .filter(|entry| entry.entity.agent_id == agent_id && visited.contains(&entry.entity.id))
            .map(|entry| entry.entity)
            .collect::<Vec<_>>();
        let relationships = self
            .load_all_relationships()?
            .into_iter()
            .filter(|entry| {
                entry.relationship.agent_id == agent_id
                    && relationship_ids.contains(&entry.relationship.id)
            })
            .map(|entry| entry.relationship)
            .collect::<Vec<_>>();
        Ok(Subgraph {
            entities,
            relationships,
            center: center_entity_id.to_string(),
            max_hops,
        })
    }

    fn list_all_entities(
        &self,
        ward_id: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<Entity>> {
        let connection = self.lock()?;
        let mut conditions = vec!["pruned = 0".to_string()];
        let mut param_values: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(ward_id) = ward_id {
            conditions.push(format!(
                "json_extract(properties_json, '$.ward_id') = ?{}",
                param_values.len() + 1
            ));
            param_values.push(Box::new(ward_id.to_string()));
        }
        if let Some(entity_type) = entity_type {
            conditions.push(format!("entity_type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(entity_type.to_string()));
        }

        let sql = format!(
            "SELECT entity_json, NULL, NULL FROM kg_entities
             WHERE {}
             ORDER BY mention_count DESC, name
             LIMIT ?{}",
            conditions.join(" AND "),
            param_values.len() + 1
        );
        param_values.push(Box::new(limit.max(1) as i64));
        let params_refs: Vec<&dyn ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
        let mut statement = connection.prepare(&sql).map_err(to_backend)?;
        let rows = statement
            .query_map(params_refs.as_slice(), decode_entity_entry)
            .map_err(to_backend)?;
        rows.map(|row| row.map(|entry| entry.entity))
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_backend)
    }

    fn connectivity_strength(
        &self,
        agent_id: &str,
        cluster_a: &[EntityId],
        cluster_b: &[EntityId],
    ) -> StoreResult<usize> {
        if cluster_a.is_empty() || cluster_b.is_empty() {
            return Ok(0);
        }
        let a = cluster_a
            .iter()
            .map(|id| id.0.as_str())
            .collect::<HashSet<_>>();
        let b = cluster_b
            .iter()
            .map(|id| id.0.as_str())
            .collect::<HashSet<_>>();
        Ok(self
            .load_all_relationships()?
            .into_iter()
            .filter(|entry| entry.relationship.agent_id == agent_id)
            .filter(|entry| {
                (a.contains(entry.relationship.source_entity_id.as_str())
                    && b.contains(entry.relationship.target_entity_id.as_str()))
                    || (b.contains(entry.relationship.source_entity_id.as_str())
                        && a.contains(entry.relationship.target_entity_id.as_str()))
            })
            .count())
    }

    fn set_members_parent(&self, members: &[EntityId], parent: &EntityId) -> StoreResult<()> {
        for member in members {
            let Some(mut entry) = self.get_entity_entry(member)? else {
                continue;
            };
            entry
                .entity
                .properties
                .insert("parent_cluster_id".to_string(), json!(parent.0));
            self.replace_entity(&entry.entity, entry.embedding.as_deref())?;
        }
        Ok(())
    }

    fn list_entities_with_embeddings_at_layer(
        &self,
        agent_id: &str,
        layer: i64,
        limit: usize,
    ) -> StoreResult<Vec<EntityWithEmbedding>> {
        let mut rows = self
            .load_all_entities()?
            .into_iter()
            .filter(|entry| {
                entry.entity.agent_id == agent_id
                    && property_i64(&entry.entity, "layer").unwrap_or(0) == layer
                    && entry.embedding.is_some()
            })
            .map(|entry| EntityWithEmbedding {
                id: EntityId(entry.entity.id),
                embedding: entry.embedding.unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        rows.truncate(limit.max(1));
        Ok(rows)
    }

    fn compute_lca_path(&self, agent_id: &str, seeds: &[EntityId]) -> StoreResult<LcaPath> {
        if seeds.is_empty() {
            return Ok(LcaPath::default());
        }
        if seeds.len() == 1 {
            let Some(entity) = self.get_entity(&seeds[0])? else {
                return Ok(LcaPath::default());
            };
            if entity.agent_id == agent_id
                && (property_i64(&entity, "layer").unwrap_or(0) > 0
                    || property_string(&entity, "parent_cluster_id").is_some())
            {
                return Ok(LcaPath {
                    lca: Some(seeds[0].clone()),
                    path_entities: Vec::new(),
                    max_layer: property_i64(&entity, "layer").unwrap_or(0),
                });
            }
            return Ok(LcaPath::default());
        }

        let chains = seeds
            .iter()
            .map(|seed| self.ancestor_chain(agent_id, seed))
            .collect::<StoreResult<Vec<_>>>()?;
        if chains.iter().any(Vec::is_empty) {
            return Ok(LcaPath::default());
        }

        let mut common = chains[0].clone();
        for chain in chains.iter().skip(1) {
            let set = chain
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<HashSet<_>>();
            common.retain(|(id, _)| set.contains(id));
        }
        let Some((lca, max_layer)) = common.into_iter().max_by_key(|(_, layer)| *layer) else {
            return Ok(LcaPath::default());
        };
        let mut path_entities = Vec::new();
        let mut seen = HashSet::new();
        for chain in chains {
            for (id, _) in chain {
                if seen.insert(id.clone()) {
                    path_entities.push(EntityId(id.clone()));
                }
                if id == lca {
                    break;
                }
            }
        }
        Ok(LcaPath {
            lca: Some(EntityId(lca)),
            path_entities,
            max_layer,
        })
    }

    fn ancestor_chain(&self, agent_id: &str, seed: &EntityId) -> StoreResult<Vec<(String, i64)>> {
        let mut chain = Vec::new();
        let mut current = seed.clone();
        let mut guard = 0;
        while guard < 64 {
            guard += 1;
            let Some(entity) = self.get_entity(&current)? else {
                break;
            };
            if entity.agent_id != agent_id {
                break;
            }
            let Some(parent) = property_string(&entity, "parent_cluster_id") else {
                break;
            };
            let Some(parent_entity) = self.get_entity(&EntityId(parent.clone()))? else {
                break;
            };
            chain.push((
                parent.clone(),
                property_i64(&parent_entity, "layer").unwrap_or(0),
            ));
            current = EntityId(parent);
        }
        Ok(chain)
    }

    fn list_inter_cluster_relations(
        &self,
        agent_id: &str,
        entity_ids: &[EntityId],
    ) -> StoreResult<Vec<InterClusterRelationHit>> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = entity_ids
            .iter()
            .map(|id| id.0.as_str())
            .collect::<HashSet<_>>();
        Ok(self
            .load_all_relationships()?
            .into_iter()
            .filter(|entry| {
                entry.relationship.agent_id == agent_id
                    && relationship_property_bool(&entry.relationship, "is_inter_cluster")
                        .unwrap_or(false)
                    && relationship_property_string(&entry.relationship, "epistemic_class")
                        .as_deref()
                        .unwrap_or("current")
                        == "current"
                    && ids.contains(entry.relationship.source_entity_id.as_str())
                    && ids.contains(entry.relationship.target_entity_id.as_str())
            })
            .map(|entry| InterClusterRelationHit {
                layer: relationship_property_i64(&entry.relationship, "layer").unwrap_or(0),
                id: entry.relationship.id,
                source_entity_id: entry.relationship.source_entity_id,
                target_entity_id: entry.relationship.target_entity_id,
                relationship_type: entry.relationship.relationship_type.as_str().to_string(),
            })
            .collect())
    }

    fn hierarchy_summary(&self, agent_id: &str, top_n: usize) -> StoreResult<HierarchySummary> {
        let connection = self.lock()?;
        let mut layer_statement = connection
            .prepare(
                "SELECT layer, COUNT(*) FROM kg_entities
                 WHERE agent_id = ?1 AND pruned = 0
                 GROUP BY layer
                 ORDER BY layer ASC",
            )
            .map_err(to_backend)?;
        let layer_counts = layer_statement
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(to_backend)?
            .map(|row| {
                row.map(|(layer, count)| (layer, count.max(0) as usize))
                    .map_err(to_backend)
            })
            .collect::<StoreResult<Vec<_>>>()?;

        let mut aggregate_statement = connection
            .prepare(
                "SELECT id, name, layer,
                        CASE json_type(properties_json, '$.member_count')
                            WHEN 'integer' THEN json_extract(properties_json, '$.member_count')
                            ELSE 0
                        END,
                        CASE json_type(properties_json, '$.description')
                            WHEN 'text' THEN json_extract(properties_json, '$.description')
                            ELSE ''
                        END
                 FROM kg_entities
                 WHERE agent_id = ?1
                   AND pruned = 0
                   AND layer > 0
                   AND CASE json_type(properties_json, '$.member_count')
                           WHEN 'integer' THEN json_extract(properties_json, '$.member_count')
                           ELSE 0
                       END > 0
                 ORDER BY CASE json_type(properties_json, '$.member_count')
                              WHEN 'integer' THEN json_extract(properties_json, '$.member_count')
                              ELSE 0
                          END DESC,
                          name ASC
                 LIMIT ?2",
            )
            .map_err(to_backend)?;
        let top_aggregates = aggregate_statement
            .query_map(params![agent_id, top_n as i64], |row| {
                Ok(zbot_stores::AggregateSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    layer: row.get(2)?,
                    member_count: row.get::<_, i64>(3)?.max(0) as usize,
                    description: row.get(4)?,
                })
            })
            .map_err(to_backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_backend)?;

        let inter_cluster_relations = connection
            .query_row(
                "SELECT COUNT(*) FROM kg_relationships
                 WHERE agent_id = ?1 AND archived = 0 AND is_inter_cluster = 1",
                params![agent_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(to_backend)?
            .max(0) as usize;
        Ok(HierarchySummary {
            layer_counts,
            inter_cluster_relations,
            top_aggregates,
        })
    }

    fn connected_entity_ids(&self) -> StoreResult<HashSet<String>> {
        let mut ids = HashSet::new();
        for entry in self.load_all_relationships()? {
            ids.insert(entry.relationship.source_entity_id);
            ids.insert(entry.relationship.target_entity_id);
        }
        Ok(ids)
    }

    fn degrees(&self) -> StoreResult<HashMap<String, usize>> {
        let mut degrees = HashMap::new();
        for entry in self.load_all_relationships()? {
            *degrees
                .entry(entry.relationship.source_entity_id.clone())
                .or_insert(0) += 1;
            *degrees
                .entry(entry.relationship.target_entity_id.clone())
                .or_insert(0) += 1;
        }
        Ok(degrees)
    }

    fn lock(&self) -> StoreResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StoreError::Backend("knowledge graph sidecar lock poisoned".to_string()))
    }
}

fn decode_entity_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntityEntry> {
    let entity_json: String = row.get(0)?;
    let embedding_json: Option<String> = row.get(1)?;
    let embedding_identity_json: Option<String> = row.get(2)?;
    let entity = serde_json::from_str::<Entity>(&entity_json).map_err(json_sql_error(0))?;
    let embedding = embedding_json
        .map(|json| serde_json::from_str::<Vec<f32>>(&json).map_err(json_sql_error(1)))
        .transpose()?;
    let embedding_identity = embedding_identity_json
        .map(|json| decode_identity_json(&json).map_err(json_string_sql_error(2)))
        .transpose()?;
    Ok(EntityEntry {
        entity,
        embedding,
        embedding_identity,
    })
}

fn decode_relationship_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RelationshipEntry> {
    let relationship_json: String = row.get(0)?;
    let relationship =
        serde_json::from_str::<Relationship>(&relationship_json).map_err(json_sql_error(0))?;
    Ok(RelationshipEntry { relationship })
}

fn decode_governance_finding(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<GovernanceValidationFinding> {
    let finding_json: String = row.get(0)?;
    serde_json::from_str::<GovernanceValidationFinding>(&finding_json).map_err(json_sql_error(0))
}

fn merge_duplicate_relationships(
    mut canonical: Relationship,
    incoming: Relationship,
) -> Relationship {
    canonical.first_seen_at = canonical.first_seen_at.min(incoming.first_seen_at);
    canonical.last_seen_at = canonical.last_seen_at.max(incoming.last_seen_at);
    canonical.mention_count = canonical
        .mention_count
        .saturating_add(incoming.mention_count.max(1));
    merge_relationship_properties(&mut canonical.properties, incoming.properties);
    canonical
}

fn merge_relationship_properties(
    canonical: &mut HashMap<String, Value>,
    incoming: HashMap<String, Value>,
) {
    for (key, incoming_value) in incoming {
        match canonical.get_mut(&key) {
            Some(existing_value) if is_evidence_property(&key) => {
                merge_json_array_values(existing_value, incoming_value);
            }
            Some(_) => {}
            None => {
                canonical.insert(key, incoming_value);
            }
        }
    }
}

fn is_evidence_property(key: &str) -> bool {
    matches!(
        key,
        "evidence" | "evidence_ids" | "evidenceIds" | "source_refs" | "sourceRefs" | "contexts"
    )
}

fn merge_json_array_values(existing: &mut Value, incoming: Value) {
    let mut values = value_as_vec(existing.take());
    values.extend(value_as_vec(incoming));

    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.to_string()));
    *existing = Value::Array(values);
}

fn value_as_vec(value: Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values,
        Value::Null => Vec::new(),
        value => vec![value],
    }
}

fn normalize_relationship_predicate(predicate: &str) -> String {
    let compact = predicate
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>();
    let normalized = RelationshipType::from_str(&compact).as_str().to_string();
    normalize_key_part(&normalized)
}

fn normalize_key_part(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_separator = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.extend(ch.to_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            normalized.push('_');
            previous_separator = true;
        }
    }
    let normalized = normalized.trim_matches('_').to_string();
    if normalized.is_empty() {
        "default".to_string()
    } else {
        normalized
    }
}

fn json_sql_error(column: usize) -> impl FnOnce(serde_json::Error) -> rusqlite::Error {
    move |error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    }
}

fn json_string_sql_error(column: usize) -> impl FnOnce(String) -> rusqlite::Error {
    move |error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    }
}

fn to_backend(error: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(error.to_string())
}

fn embedding_identity_from_config(config: &AdapterConfig) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: config.embedding_provider.provider_type.clone(),
        model: config.embedding_provider.model.clone(),
        dimensions: config.embedding_provider.dimensions,
        prompt_profile: config.embedding_provider.prompt_profile.clone(),
        normalization: config.embedding_provider.normalization.clone(),
    }
}

fn encode_identity(identity: &EmbeddingQueryIdentity) -> String {
    serde_json::json!({
        "providerType": identity.provider_type,
        "model": identity.model,
        "dimensions": identity.dimensions,
        "promptProfile": identity.prompt_profile,
        "normalization": identity.normalization })
    .to_string()
}

fn decode_identity_json(json: &str) -> Result<EmbeddingQueryIdentity, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let dimensions = value
        .get("dimensions")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "missing embedding identity dimensions".to_string())?;
    Ok(EmbeddingQueryIdentity {
        provider_type: value
            .get("providerType")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        model: value
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        dimensions: dimensions as u32,
        prompt_profile: value
            .get("promptProfile")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("query")
            .to_string(),
        normalization: value
            .get("normalization")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn identity_compatible(
    expected: &EmbeddingQueryIdentity,
    actual: Option<&EmbeddingQueryIdentity>,
    vector_dimensions: usize,
) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    expected.provider_type == actual.provider_type
        && expected.model == actual.model
        && expected.dimensions == actual.dimensions
        && expected.dimensions as usize == vector_dimensions
        && expected.prompt_profile == actual.prompt_profile
        && expected.normalization == actual.normalization
}

fn ensure_optional_column(
    connection: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if names.iter().any(|name| name == column) {
        return Ok(());
    }
    connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
        [],
    )?;
    Ok(())
}

fn validate_agent_id(agent_id: &str) -> StoreResult<()> {
    if agent_id.trim().is_empty() {
        return Err(StoreError::Invalid(
            "graph write requires a non-empty agent id for scope and provenance".to_string(),
        ));
    }
    Ok(())
}

fn validate_record_id(kind: &str, id: &str) -> StoreResult<()> {
    if id.trim().is_empty() {
        return Err(StoreError::Invalid(format!("{kind} id must not be blank")));
    }
    Ok(())
}

fn validate_name(name: &str) -> StoreResult<()> {
    if name.trim().is_empty() {
        return Err(StoreError::Invalid(
            "entity name must not be blank".to_string(),
        ));
    }
    Ok(())
}

fn validate_timestamps(
    kind: &str,
    first_seen_at: chrono::DateTime<Utc>,
    last_seen_at: chrono::DateTime<Utc>,
) -> StoreResult<()> {
    if last_seen_at < first_seen_at {
        return Err(StoreError::Invalid(format!(
            "{kind} last_seen_at must not precede first_seen_at"
        )));
    }
    Ok(())
}

fn validate_mention_count(kind: &str, mention_count: i64) -> StoreResult<()> {
    if mention_count < 1 {
        return Err(StoreError::Invalid(format!(
            "{kind} mention_count must be positive"
        )));
    }
    Ok(())
}

fn canonical_builtin_entity_type(entity_type: &EntityType) -> StoreResult<EntityType> {
    let canonical = EntityType::from_str(entity_type.as_str());
    if matches!(canonical, EntityType::Custom(_)) {
        return Err(StoreError::Invalid(format!(
            "entity type must be a built-in ontology class: {}",
            entity_type.as_str()
        )));
    }
    Ok(canonical)
}

fn canonical_builtin_relationship_type(
    relationship_type: &RelationshipType,
) -> StoreResult<RelationshipType> {
    let canonical = RelationshipType::from_str(relationship_type.as_str());
    if matches!(canonical, RelationshipType::Custom(_)) {
        return Err(StoreError::Invalid(format!(
            "relationship predicate must be a built-in ontology predicate: {}",
            relationship_type.as_str()
        )));
    }
    Ok(canonical)
}

fn ensure_scope(mapper: &ScopeMapper, properties: &mut HashMap<String, Value>) -> StoreResult<()> {
    let ward_id = match properties.get("ward_id") {
        Some(Value::String(value)) if !value.trim().is_empty() => value.trim().to_string(),
        Some(_) => {
            return Err(StoreError::Invalid(
                "ward_id must be a non-empty string when supplied".to_string(),
            ));
        }
        None => DEFAULT_WARD_ID.to_string(),
    };
    mapper
        .ward_scope(&ward_id)
        .map_err(|error| StoreError::Invalid(format!("invalid graph scope: {error}")))?;
    properties.insert("ward_id".to_string(), json!(ward_id));
    Ok(())
}

fn append_path(path: &str, relationship_type: &str) -> String {
    if path.is_empty() {
        relationship_type.to_string()
    } else {
        format!("{path},{relationship_type}")
    }
}

fn kg_direction(direction: Direction) -> knowledge_graph::types::Direction {
    match direction {
        Direction::Outgoing => knowledge_graph::types::Direction::Outgoing,
        Direction::Incoming => knowledge_graph::types::Direction::Incoming,
        Direction::Both => knowledge_graph::types::Direction::Both,
    }
}

fn ward_id_for_entity(entity: &Entity) -> Option<String> {
    property_string(entity, "ward_id")
}

fn property_string(entity: &Entity, key: &str) -> Option<String> {
    entity.properties.get(key)?.as_str().map(ToOwned::to_owned)
}

fn property_i64(entity: &Entity, key: &str) -> Option<i64> {
    entity.properties.get(key)?.as_i64()
}

fn relationship_property_string(relationship: &Relationship, key: &str) -> Option<String> {
    relationship
        .properties
        .get(key)?
        .as_str()
        .map(ToOwned::to_owned)
}

fn relationship_property_i64(relationship: &Relationship, key: &str) -> Option<i64> {
    relationship.properties.get(key)?.as_i64()
}

fn relationship_property_f64(relationship: &Relationship, key: &str) -> Option<f64> {
    relationship.properties.get(key)?.as_f64()
}

fn relationship_property_bool(relationship: &Relationship, key: &str) -> Option<bool> {
    relationship.properties.get(key)?.as_bool()
}

fn confidence_for(entity: &Entity) -> f64 {
    entity
        .properties
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
}
