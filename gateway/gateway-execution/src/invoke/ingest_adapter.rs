//! # Ingestion Adapter
//!
//! Bridges the trait-routed `KgEpisodeStore` + [`IngestionQueue`] +
//! `KnowledgeGraphStore` to [`agent_tools::IngestionAccess`].
//! Wired into the agent tool registry so the `ingest` tool can both
//! (a) enqueue text chunks for background LLM extraction, and
//! (b) bulk-upsert structured entities and relationships synchronously.
//!
//! Phase B2: backend-agnostic. Both paths now go through trait surfaces
//! so all backends share one code path.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

use agent_tools::{
    EvidenceRecord, IngestionAccess, StructuredCounts, StructuredEntity, StructuredRelationship,
};
use chrono::Utc;
use knowledge_graph::{Entity, EntityType, Relationship, RelationshipType};
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_traits::KgEpisodeStore;

use crate::ingest::{
    chunker::{chunk_text, ChunkOptions},
    IngestionQueue,
};

/// Property names whose values select graph scope or governance behavior.
/// These values must come from trusted runtime/configuration context, never
/// from model-produced graph candidates or caller-supplied structured data.
pub(crate) const GRAPH_CONTROL_PROPERTY_KEYS: [&str; 10] = [
    "ward_id",
    "ontology_id",
    "taxonomy_id",
    "epistemic_class",
    "parent_cluster_id",
    "layer",
    "confidence",
    "governance_ontology_ids",
    "governance_taxonomy_scheme_ids",
    "governance_record_kind",
];

/// Adapter that implements [`IngestionAccess`] for both text and structured
/// ingestion paths.
pub struct IngestionAdapter {
    queue: Arc<IngestionQueue>,
    episode_store: Arc<dyn KgEpisodeStore>,
    kg_store: Arc<dyn KnowledgeGraphStore>,
}

impl IngestionAdapter {
    pub fn new(
        queue: Arc<IngestionQueue>,
        episode_store: Arc<dyn KgEpisodeStore>,
        kg_store: Arc<dyn KnowledgeGraphStore>,
    ) -> Self {
        Self {
            queue,
            episode_store,
            kg_store,
        }
    }
}

#[async_trait]
impl IngestionAccess for IngestionAdapter {
    async fn record_evidence(&self, record: EvidenceRecord) -> std::result::Result<(), String> {
        let payload = serde_json::to_string(&record)
            .map_err(|e| format!("serialize evidence record: {e}"))?;
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());
        let episode_id = self
            .episode_store
            .upsert_pending(
                "evidence_intake",
                &record.evidence_id,
                &content_hash,
                record.session_id.as_deref(),
                &record.agent_id,
            )
            .await?;
        self.episode_store
            .set_payload(&episode_id, &payload)
            .await?;
        self.episode_store.mark_done(&episode_id).await?;
        Ok(())
    }

    async fn enqueue(
        &self,
        source_id: &str,
        source_type: &str,
        text: &str,
        session_id: Option<&str>,
        agent_id: &str,
    ) -> std::result::Result<(String, usize), String> {
        let chunks = chunk_text(text, ChunkOptions::default());
        let mut enqueued = 0usize;
        for chunk in &chunks {
            let source_ref = format!("{}#chunk-{}", source_id, chunk.index);
            let mut hasher = Sha256::new();
            hasher.update(chunk.text.as_bytes());
            let content_hash = format!("{:x}", hasher.finalize());
            let episode_id = self
                .episode_store
                .upsert_pending(
                    source_type,
                    &source_ref,
                    &content_hash,
                    session_id,
                    agent_id,
                )
                .await?;
            self.episode_store
                .set_payload(&episode_id, &chunk.text)
                .await?;
            enqueued += 1;
        }
        self.queue.notify();
        Ok((source_id.to_string(), enqueued))
    }

    async fn ingest_structured(
        &self,
        agent_id: &str,
        ward_id: Option<String>,
        entities: Vec<StructuredEntity>,
        relationships: Vec<StructuredRelationship>,
    ) -> std::result::Result<StructuredCounts, String> {
        let entity_count = entities.len();
        let relationship_count = relationships.len();
        let knowledge = build_knowledge(agent_id, ward_id.as_deref(), entities, relationships);
        self.kg_store
            .store_knowledge(agent_id, knowledge)
            .await
            .map_err(|e| format!("store_knowledge: {e}"))?;

        Ok(StructuredCounts {
            entities_upserted: entity_count,
            relationships_upserted: relationship_count,
        })
    }
}

/// Map the generic agent-tools shapes onto `zbot_stores::ExtractedKnowledge`.
/// Returns the trait-side type so the result can be passed straight to
/// `KnowledgeGraphStore::store_knowledge`.
fn build_knowledge(
    agent_id: &str,
    ward_id: Option<&str>,
    entities: Vec<StructuredEntity>,
    relationships: Vec<StructuredRelationship>,
) -> zbot_stores::ExtractedKnowledge {
    let now = Utc::now();

    let kg_entities: Vec<Entity> = entities
        .into_iter()
        .map(|e| {
            let props = trusted_graph_properties(e.properties, ward_id);
            Entity {
                id: e.id,
                agent_id: agent_id.to_string(),
                entity_type: EntityType::from_str(&e.entity_type),
                name: e.name,
                properties: props,
                first_seen_at: now,
                last_seen_at: now,
                mention_count: 1,
                name_embedding: None,
            }
        })
        .collect();

    let kg_relationships: Vec<Relationship> = relationships
        .into_iter()
        .map(|r| {
            let props = trusted_graph_properties(r.properties, ward_id);
            Relationship {
                id: format!("rel-{}", uuid::Uuid::new_v4()),
                agent_id: agent_id.to_string(),
                source_entity_id: r.from,
                target_entity_id: r.to,
                relationship_type: RelationshipType::from_str(&r.rel_type),
                properties: props,
                first_seen_at: now,
                last_seen_at: now,
                mention_count: 1,
            }
        })
        .collect();

    zbot_stores::ExtractedKnowledge {
        entities: kg_entities,
        relationships: kg_relationships,
    }
}

fn trusted_graph_properties(
    properties: serde_json::Map<String, serde_json::Value>,
    ward_id: Option<&str>,
) -> HashMap<String, serde_json::Value> {
    let mut trusted: HashMap<String, serde_json::Value> = properties
        .into_iter()
        .filter(|(key, _)| !GRAPH_CONTROL_PROPERTY_KEYS.contains(&key.as_str()))
        .collect();
    if let Some(ward_id) = ward_id.map(str::trim).filter(|ward| !ward.is_empty()) {
        trusted.insert("ward_id".to_string(), serde_json::json!(ward_id));
    }
    trusted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ExecutionError;
    use crate::ingest::extractor::Extractor;
    use gateway_services::VaultPaths;
    use zbot_engram_adapter::{AdapterConfig, EngramKnowledgeGraphStore};
    use zbot_stores_sqlite::kg::storage::GraphStorage;
    use zbot_stores_sqlite::{
        GatewayKgEpisodeStore, KgEpisodeRepository, KnowledgeDatabase, SqliteKgStore,
    };

    /// Minimal no-op extractor — lets IngestionQueue::start spawn cleanly
    /// without needing a provider/LLM. Tests never exercise the worker loop.
    struct NoopExtractor;

    #[async_trait]
    impl Extractor for NoopExtractor {
        async fn process(
            &self,
            _episode_id: &str,
            _chunk_text: &str,
            _kg_store: &Arc<dyn zbot_stores::KnowledgeGraphStore>,
        ) -> Result<(), ExecutionError> {
            Ok(())
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        episode_repo: Arc<KgEpisodeRepository>,
        graph: Arc<GraphStorage>,
        adapter: IngestionAdapter,
    }

    fn setup() -> Harness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let episode_repo = Arc::new(KgEpisodeRepository::new(db.clone()));
        let episode_store: Arc<dyn KgEpisodeStore> =
            Arc::new(GatewayKgEpisodeStore::new(episode_repo.clone()));
        let graph = Arc::new(GraphStorage::new(db).expect("graph"));
        let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(SqliteKgStore::new(graph.clone()));
        // 0 workers — spawns the dispatcher only. notify() is a no-op,
        // no workers try to claim-and-process anything we enqueue. Keeps
        // tests deterministic: the rows we insert stay in `pending`.
        let queue = Arc::new(IngestionQueue::start(
            0,
            episode_store.clone(),
            kg_store.clone(),
            Arc::new(NoopExtractor),
        ));
        let adapter = IngestionAdapter::new(queue, episode_store, kg_store);
        Harness {
            _tmp: tmp,
            episode_repo,
            graph,
            adapter,
        }
    }

    // --- build_knowledge: pure helper ---

    #[test]
    fn build_knowledge_maps_entity_and_relationship_fields() {
        let entity = StructuredEntity {
            id: "alice".into(),
            name: "Alice".into(),
            entity_type: "person".into(),
            properties: serde_json::json!({
                "role": "author",
                "ward_id": "attacker-ward",
                "ontology_id": "attacker-ontology",
                "governance_ontology_ids": ["attacker-ontology"]
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        let rel = StructuredRelationship {
            rel_type: "uses".into(),
            from: "alice".into(),
            to: "rust".into(),
            properties: serde_json::json!({
                "since": "2026",
                "ward_id": "attacker-ward",
                "governance_record_kind": "attacker"
            })
            .as_object()
            .unwrap()
            .clone(),
        };

        let knowledge = build_knowledge("agent-x", Some("trusted-ward"), vec![entity], vec![rel]);

        assert_eq!(knowledge.entities.len(), 1);
        let e = &knowledge.entities[0];
        assert_eq!(e.id, "alice");
        assert_eq!(e.name, "Alice");
        assert_eq!(e.agent_id, "agent-x");
        assert_eq!(e.mention_count, 1);
        assert_eq!(e.properties.get("role"), Some(&serde_json::json!("author")));
        assert_eq!(
            e.properties.get("ward_id"),
            Some(&serde_json::json!("trusted-ward"))
        );
        assert!(!e.properties.contains_key("ontology_id"));
        assert!(!e.properties.contains_key("governance_ontology_ids"));

        assert_eq!(knowledge.relationships.len(), 1);
        let r = &knowledge.relationships[0];
        assert!(r.id.starts_with("rel-"));
        assert_eq!(r.agent_id, "agent-x");
        assert_eq!(
            r.properties.get("ward_id"),
            Some(&serde_json::json!("trusted-ward"))
        );
        assert!(!r.properties.contains_key("governance_record_kind"));
        assert_eq!(r.source_entity_id, "alice");
        assert_eq!(r.target_entity_id, "rust");
        assert_eq!(r.mention_count, 1);
        assert_eq!(r.properties.get("since"), Some(&serde_json::json!("2026")));
    }

    #[test]
    fn build_knowledge_empty_inputs_produce_empty_outputs() {
        let knowledge = build_knowledge("agent-x", None, vec![], vec![]);
        assert!(knowledge.entities.is_empty());
        assert!(knowledge.relationships.is_empty());
    }

    // --- IngestionAdapter::enqueue ---

    #[tokio::test]
    async fn enqueue_chunks_text_and_persists_one_episode_per_chunk() {
        let h = setup();
        let text = "a".repeat(3000); // Forces at least 2 chunks at default chunk size.
        let (id, count) = h
            .adapter
            .enqueue("src-1", "document", &text, None, "agent-1")
            .await
            .expect("enqueue");

        assert_eq!(id, "src-1");
        assert!(count >= 1, "at least one chunk enqueued");

        // Verify `pending` rows were written via the global pending counter.
        let pending_rows = h
            .episode_repo
            .count_pending_global()
            .expect("count pending");
        assert_eq!(
            pending_rows as usize, count,
            "pending episode count matches enqueued chunk count"
        );
    }

    #[tokio::test]
    async fn record_evidence_persists_completed_episode_payload() {
        let h = setup();
        let record = EvidenceRecord {
            evidence_id: "root:memory:domain:valuation.aapl".into(),
            action: "memory_write".into(),
            source_id: "valuation.aapl".into(),
            source_type: "memory_fact:domain".into(),
            session_id: Some("sess-1".into()),
            ward_id: Some("ward-1".into()),
            agent_id: "root".into(),
            retention_policy: "durable".into(),
            ontology_labels: vec!["financial_metric".into()],
            taxonomy_labels: vec!["skos:finance".into()],
        };

        h.adapter
            .record_evidence(record.clone())
            .await
            .expect("record evidence");

        let episodes = h
            .episode_repo
            .list_by_session("sess-1")
            .expect("list episodes");
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].source_type, "evidence_intake");
        assert_eq!(episodes[0].status, "done");
        let episode_id = &episodes[0].id;
        let payload = h
            .episode_repo
            .get_payload(episode_id)
            .expect("payload")
            .expect("payload present");
        let stored: EvidenceRecord = serde_json::from_str(&payload).expect("evidence payload");
        assert_eq!(stored, record);
    }

    #[tokio::test]
    async fn enqueue_empty_text_returns_zero_chunks() {
        let h = setup();
        let (id, count) = h
            .adapter
            .enqueue("src-empty", "document", "", None, "agent-1")
            .await
            .expect("enqueue");
        assert_eq!(id, "src-empty");
        assert_eq!(count, 0, "empty text produces no chunks");
    }

    // --- IngestionAdapter::ingest_structured ---

    #[tokio::test]
    async fn ingest_structured_upserts_entities_and_returns_counts() {
        let h = setup();
        let entities = vec![
            StructuredEntity {
                id: "e1".into(),
                name: "EntityOne".into(),
                entity_type: "concept".into(),
                properties: serde_json::json!({
                    "ward_id": "attacker-ward",
                    "governance_record_kind": "attacker",
                    "display_name": "Entity One"
                })
                .as_object()
                .unwrap()
                .clone(),
            },
            StructuredEntity {
                id: "e2".into(),
                name: "EntityTwo".into(),
                entity_type: "concept".into(),
                properties: serde_json::Map::new(),
            },
        ];
        let relationships = vec![StructuredRelationship {
            rel_type: "related_to".into(),
            from: "e1".into(),
            to: "e2".into(),
            properties: serde_json::Map::new(),
        }];

        let counts = h
            .adapter
            .ingest_structured(
                "agent-x",
                Some("ward-1".to_string()),
                entities,
                relationships,
            )
            .await
            .expect("ingest_structured");

        assert_eq!(counts.entities_upserted, 2);
        assert_eq!(counts.relationships_upserted, 1);

        // The entities should actually have landed in the graph.
        let stored = h.graph.get_entity_by_name("agent-x", "EntityOne").unwrap();
        let stored = stored.expect("EntityOne should be retrievable");
        assert_eq!(
            stored.properties.get("ward_id"),
            Some(&serde_json::json!("ward-1"))
        );
        assert!(!stored.properties.contains_key("governance_record_kind"));
        assert_eq!(
            stored.properties.get("display_name"),
            Some(&serde_json::json!("Entity One"))
        );
    }

    #[tokio::test]
    async fn ingest_structured_empty_batches_are_noops() {
        let h = setup();
        let counts = h
            .adapter
            .ingest_structured("agent-x", None, vec![], vec![])
            .await
            .expect("ingest_structured");
        assert_eq!(counts.entities_upserted, 0);
        assert_eq!(counts.relationships_upserted, 0);
    }

    #[tokio::test]
    async fn ingest_structured_rejects_custom_predicate_without_partial_engram_writes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let episode_repo = Arc::new(KgEpisodeRepository::new(db));
        let episode_store: Arc<dyn KgEpisodeStore> =
            Arc::new(GatewayKgEpisodeStore::new(episode_repo));
        let engram = Arc::new(
            EngramKnowledgeGraphStore::open(AdapterConfig::engram_for_data_root(
                tmp.path(),
                "engram-ingest-test.db",
            ))
            .expect("Engram graph"),
        );
        let kg_store: Arc<dyn KnowledgeGraphStore> = engram.clone();
        let queue = Arc::new(IngestionQueue::start(
            0,
            episode_store.clone(),
            kg_store.clone(),
            Arc::new(NoopExtractor),
        ));
        let adapter = IngestionAdapter::new(queue, episode_store, kg_store);

        let error = adapter
            .ingest_structured(
                "agent-x",
                Some("trusted-ward".to_string()),
                vec![
                    StructuredEntity {
                        id: "person:alice".to_string(),
                        name: "Alice".to_string(),
                        entity_type: "person".to_string(),
                        properties: serde_json::Map::new(),
                    },
                    StructuredEntity {
                        id: "project:zbot".to_string(),
                        name: "ZBot".to_string(),
                        entity_type: "project".to_string(),
                        properties: serde_json::Map::new(),
                    },
                ],
                vec![StructuredRelationship {
                    rel_type: "unsupported_predicate".to_string(),
                    from: "person:alice".to_string(),
                    to: "project:zbot".to_string(),
                    properties: serde_json::Map::new(),
                }],
            )
            .await
            .expect_err("Engram must reject a custom predicate");

        assert!(error.contains("built-in ontology predicate"));
        assert_eq!(engram.count_all_entities().await.expect("entity count"), 0);
        assert_eq!(
            engram
                .count_all_relationships()
                .await
                .expect("relationship count"),
            0
        );
    }
}
