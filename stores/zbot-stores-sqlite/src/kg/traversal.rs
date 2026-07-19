//! # Graph Traversal
//!
//! Bounded BFS traversal over the knowledge graph using recursive CTEs.
//!
//! Provides [`GraphTraversal`] trait and a SQLite implementation
//! ([`SqliteGraphTraversal`]) that uses `WITH RECURSIVE` for efficient
//! multi-hop graph walks. Cycle detection is built into the CTE via a
//! `visited` accumulator, and `max_hops` bounds recursion depth — safe
//! for resource-constrained targets like Raspberry Pi 4.

use super::storage::GraphStorage;
use rusqlite::params;
use std::collections::HashMap;
use std::sync::Arc;

/// A node discovered during graph traversal.
#[derive(Debug, Clone)]
pub struct TraversalNode {
    /// ID of the discovered entity
    pub entity_id: String,
    /// Human-readable name
    pub entity_name: String,
    /// Entity type as stored in the DB (e.g. "person", "tool")
    pub entity_type: String,
    /// How many hops from the seed entity
    pub hop_distance: u8,
    /// Comma-separated relationship types along the shortest path
    pub path: String,
    /// Relevance score: `hop_decay ^ hop_distance`
    pub relevance: f64,
    /// Number of times the entity has been mentioned
    pub mention_count: i64,
}

// Conversion to the trait-level `TraversalHit` type. Hosted here (the
// owner of `TraversalNode`) instead of in `zbot-stores` to avoid that
// crate having to depend on `zbot-stores-sqlite`. Relocated from
// `zbot-stores/src/types.rs` during Slice D6b.
impl From<TraversalNode> for zbot_stores::types::TraversalHit {
    fn from(n: TraversalNode) -> Self {
        zbot_stores::types::TraversalHit {
            entity_id: zbot_stores::types::EntityId(n.entity_id),
            hop: n.hop_distance as usize,
            path: n.path,
            mention_count: n.mention_count,
        }
    }
}

/// Trait for multi-hop graph traversal.
#[async_trait::async_trait]
pub trait GraphTraversal: Send + Sync {
    /// BFS from a single entity, return all entities within `max_hops`.
    async fn traverse(
        &self,
        entity_id: &str,
        max_hops: u8,
        limit: usize,
    ) -> Result<Vec<TraversalNode>, String>;

    /// Find entities connected to any of the given entity **names** within
    /// `max_hops`. Names are matched case-insensitively. Results are
    /// deduplicated by `entity_id`, keeping the shortest hop distance.
    async fn connected_entities(
        &self,
        names: &[&str],
        max_hops: u8,
        limit: usize,
    ) -> Result<Vec<TraversalNode>, String>;
}

/// SQLite-backed graph traversal using `WITH RECURSIVE`.
pub struct SqliteGraphTraversal {
    storage: Arc<GraphStorage>,
    hop_decay: f64,
}

impl SqliteGraphTraversal {
    /// Create a new traversal engine.
    ///
    /// * `storage`  – shared handle to the graph database
    /// * `hop_decay` – multiplier applied per hop (e.g. 0.7 means hop-2
    ///   relevance = 0.7^2 = 0.49)
    pub fn new(storage: Arc<GraphStorage>, hop_decay: f64) -> Self {
        Self { storage, hop_decay }
    }

    /// Core BFS query using a recursive CTE.
    ///
    /// Returns `(entity_id, name, entity_type, min_hop, path, mention_count)`.
    fn traverse_inner(
        &self,
        conn: &rusqlite::Connection,
        entity_id: &str,
        max_hops: u8,
        limit: usize,
    ) -> Result<Vec<TraversalNode>, String> {
        let sql = r#"
            WITH RECURSIVE graph_walk(entity_id, hop, path, visited) AS (
                -- Seed: start entity
                SELECT ?1, 0, '', ?1
                UNION ALL
                -- Walk: follow relationships in either direction, avoiding cycles
                SELECT
                    CASE WHEN r.source_entity_id = gw.entity_id
                         THEN r.target_entity_id
                         ELSE r.source_entity_id
                    END,
                    gw.hop + 1,
                    CASE WHEN gw.path = '' THEN r.relationship_type
                         ELSE gw.path || ',' || r.relationship_type
                    END,
                    gw.visited || ',' ||
                        CASE WHEN r.source_entity_id = gw.entity_id
                             THEN r.target_entity_id
                             ELSE r.source_entity_id
                        END
                FROM graph_walk gw
                JOIN kg_relationships r
                    ON (r.source_entity_id = gw.entity_id
                        OR r.target_entity_id = gw.entity_id)
                WHERE gw.hop < ?2
                  AND gw.visited NOT LIKE
                      '%' ||
                      CASE WHEN r.source_entity_id = gw.entity_id
                           THEN r.target_entity_id
                           ELSE r.source_entity_id
                      END || '%'
            )
            SELECT DISTINCT
                gw.entity_id,
                e.name,
                e.entity_type,
                MIN(gw.hop)       AS min_hop,
                gw.path,
                e.mention_count
            FROM graph_walk gw
            JOIN kg_entities e ON e.id = gw.entity_id
            WHERE gw.entity_id != ?1
            GROUP BY gw.entity_id
            ORDER BY min_hop ASC, e.mention_count DESC
            LIMIT ?3
        "#;

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| format!("prepare traverse: {e}"))?;

        let rows = stmt
            .query_map(params![entity_id, max_hops as i64, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| format!("query traverse: {e}"))?;

        let decay = self.hop_decay;
        let mut nodes = Vec::new();
        for row in rows {
            let (id, name, etype, hop, path, mentions) =
                row.map_err(|e| format!("row traverse: {e}"))?;
            let hop_u8 = hop as u8;
            nodes.push(TraversalNode {
                entity_id: id,
                entity_name: name,
                entity_type: etype,
                hop_distance: hop_u8,
                path,
                relevance: decay.powi(hop_u8 as i32),
                mention_count: mentions,
            });
        }
        Ok(nodes)
    }
}

#[async_trait::async_trait]
impl GraphTraversal for SqliteGraphTraversal {
    async fn traverse(
        &self,
        entity_id: &str,
        max_hops: u8,
        limit: usize,
    ) -> Result<Vec<TraversalNode>, String> {
        self.storage
            .db
            .with_connection(|conn| {
                self.traverse_inner(conn, entity_id, max_hops, limit)
                    .map_err(|e| {
                        rusqlite::Error::InvalidColumnType(0, e, rusqlite::types::Type::Null)
                    })
            })
            .map_err(|e| format!("traverse: {e}"))
    }

    async fn connected_entities(
        &self,
        names: &[&str],
        max_hops: u8,
        limit: usize,
    ) -> Result<Vec<TraversalNode>, String> {
        self.storage
            .db
            .with_connection(|conn| {
                // Resolve each name to an entity ID (case-insensitive)
                let mut seed_ids: Vec<String> = Vec::new();
                for name in names {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM kg_entities WHERE name COLLATE NOCASE = ?1 LIMIT 1",
                    )?;

                    let mut rows = stmt.query_map(params![*name], |row| row.get::<_, String>(0))?;

                    if let Some(Ok(id)) = rows.next() {
                        seed_ids.push(id);
                    }
                }

                // BFS from each seed, merge keeping shortest hop per entity
                let mut best: HashMap<String, TraversalNode> = HashMap::new();
                for seed_id in &seed_ids {
                    let nodes = self
                        .traverse_inner(conn, seed_id, max_hops, limit)
                        .map_err(|e| {
                            rusqlite::Error::InvalidColumnType(0, e, rusqlite::types::Type::Null)
                        })?;
                    for node in nodes {
                        // Also skip if the node is one of the other seed entities
                        if seed_ids.contains(&node.entity_id) {
                            continue;
                        }
                        best.entry(node.entity_id.clone())
                            .and_modify(|existing| {
                                if node.hop_distance < existing.hop_distance {
                                    *existing = node.clone();
                                }
                            })
                            .or_insert(node);
                    }
                }

                let mut results: Vec<TraversalNode> = best.into_values().collect();
                results.sort_by(|a, b| {
                    a.hop_distance
                        .cmp(&b.hop_distance)
                        .then_with(|| b.mention_count.cmp(&a.mention_count))
                });
                results.truncate(limit);
                Ok(results)
            })
            .map_err(|e| format!("connected_entities: {e}"))
    }
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge_graph::types::{
        Entity, EntityType, ExtractedKnowledge, Relationship, RelationshipType,
    };
    use tempfile::tempdir;

    /// Spin up an in-memory-ish test storage and return it wrapped in Arc.
    fn create_test_storage() -> Arc<GraphStorage> {
        let dir = tempdir().unwrap();
        let tmp_path = dir.keep();
        let paths = Arc::new(gateway_services::VaultPaths::new(tmp_path));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(crate::KnowledgeDatabase::new(paths).unwrap());
        Arc::new(GraphStorage::new(db).unwrap())
    }

    /// Build a small graph: Alice --uses--> Rust --part_of--> Systems
    async fn seed_graph(storage: &GraphStorage) -> (String, String, String) {
        let alice = Entity::new("agent1".into(), EntityType::Person, "Alice".into());
        let rust = Entity::new("agent1".into(), EntityType::Tool, "Rust".into());
        let systems = Entity::new("agent1".into(), EntityType::Concept, "Systems".into());

        let alice_id = alice.id.clone();
        let rust_id = rust.id.clone();
        let systems_id = systems.id.clone();

        let rel_uses = Relationship::new(
            "agent1".into(),
            alice_id.clone(),
            rust_id.clone(),
            RelationshipType::Uses,
        );
        let rel_part_of = Relationship::new(
            "agent1".into(),
            rust_id.clone(),
            systems_id.clone(),
            RelationshipType::PartOf,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![alice, rust, systems],
            relationships: vec![rel_uses, rel_part_of],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        // IDs may have been remapped by dedup; look them up by name
        let alice_actual = storage
            .get_entity_by_name("agent1", "Alice")
            .unwrap()
            .unwrap()
            .id;
        let rust_actual = storage
            .get_entity_by_name("agent1", "Rust")
            .unwrap()
            .unwrap()
            .id;
        let systems_actual = storage
            .get_entity_by_name("agent1", "Systems")
            .unwrap()
            .unwrap()
            .id;

        (alice_actual, rust_actual, systems_actual)
    }

    #[tokio::test]
    async fn test_traverse_2_hops() {
        let storage = create_test_storage();
        let (alice_id, _rust_id, _systems_id) = seed_graph(&storage).await;

        let traversal = SqliteGraphTraversal::new(storage, 0.7);
        let results = traversal.traverse(&alice_id, 2, 20).await.unwrap();

        // Should find both Rust (hop 1) and Systems (hop 2)
        assert_eq!(results.len(), 2, "expected 2 nodes, got: {results:?}");

        let hop1: Vec<&TraversalNode> = results.iter().filter(|n| n.hop_distance == 1).collect();
        let hop2: Vec<&TraversalNode> = results.iter().filter(|n| n.hop_distance == 2).collect();

        assert_eq!(hop1.len(), 1, "expected 1 node at hop 1");
        assert_eq!(hop1[0].entity_name, "Rust");
        assert!((hop1[0].relevance - 0.7).abs() < 1e-9);

        assert_eq!(hop2.len(), 1, "expected 1 node at hop 2");
        assert_eq!(hop2[0].entity_name, "Systems");
        assert!((hop2[0].relevance - 0.49).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_traverse_max_hops_1() {
        let storage = create_test_storage();
        let (alice_id, _rust_id, _systems_id) = seed_graph(&storage).await;

        let traversal = SqliteGraphTraversal::new(storage, 0.7);
        let results = traversal.traverse(&alice_id, 1, 20).await.unwrap();

        // Only Rust should be found (hop 1); Systems is at hop 2
        assert_eq!(results.len(), 1, "expected 1 node, got: {results:?}");
        assert_eq!(results[0].entity_name, "Rust");
        assert_eq!(results[0].hop_distance, 1);
    }

    #[tokio::test]
    async fn test_connected_entities_by_name() {
        let storage = create_test_storage();
        let (_alice_id, _rust_id, _systems_id) = seed_graph(&storage).await;

        let traversal = SqliteGraphTraversal::new(storage, 0.7);
        let results = traversal
            .connected_entities(&["Alice"], 2, 20)
            .await
            .unwrap();

        // Same as traversing from Alice's ID — should find Rust and Systems
        assert_eq!(results.len(), 2, "expected 2 nodes, got: {results:?}");

        let names: Vec<&str> = results.iter().map(|n| n.entity_name.as_str()).collect();
        assert!(names.contains(&"Rust"), "Rust missing from {names:?}");
        assert!(names.contains(&"Systems"), "Systems missing from {names:?}");
    }

    #[tokio::test]
    async fn test_connected_entities_case_insensitive() {
        let storage = create_test_storage();
        seed_graph(&storage).await;

        let traversal = SqliteGraphTraversal::new(storage, 0.7);
        // Use lowercase "alice" — should still resolve
        let results = traversal
            .connected_entities(&["alice"], 2, 20)
            .await
            .unwrap();

        assert_eq!(
            results.len(),
            2,
            "case-insensitive lookup failed: {results:?}"
        );
    }

    #[tokio::test]
    async fn test_traverse_cycle_detection() {
        // Build a cycle: A -> B -> C -> A
        let storage = create_test_storage();

        let a = Entity::new("agent1".into(), EntityType::Concept, "NodeA".into());
        let b = Entity::new("agent1".into(), EntityType::Concept, "NodeB".into());
        let c = Entity::new("agent1".into(), EntityType::Concept, "NodeC".into());

        let rel_ab = Relationship::new(
            "agent1".into(),
            a.id.clone(),
            b.id.clone(),
            RelationshipType::RelatedTo,
        );
        let rel_bc = Relationship::new(
            "agent1".into(),
            b.id.clone(),
            c.id.clone(),
            RelationshipType::RelatedTo,
        );
        let rel_ca = Relationship::new(
            "agent1".into(),
            c.id.clone(),
            a.id.clone(),
            RelationshipType::RelatedTo,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![a, b, c],
            relationships: vec![rel_ab, rel_bc, rel_ca],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        let a_id = storage
            .get_entity_by_name("agent1", "NodeA")
            .unwrap()
            .unwrap()
            .id;

        let traversal = SqliteGraphTraversal::new(storage, 0.7);
        // Even with high max_hops, cycle detection should keep results finite
        let results = traversal.traverse(&a_id, 10, 100).await.unwrap();

        // Should find B and C, no duplicates, no infinite loop
        assert_eq!(
            results.len(),
            2,
            "cycle graph: expected 2 nodes, got: {results:?}"
        );
    }
}
