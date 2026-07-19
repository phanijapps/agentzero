//! # Graph Storage
//!
//! SQLite storage for knowledge graph entities and relationships.

use crate::KnowledgeDatabase;
use knowledge_graph::error::{GraphError, GraphResult};
use knowledge_graph::types::{
    Direction, Entity, EntityType, ExtractedKnowledge, NeighborInfo, Relationship, RelationshipType,
};
use rusqlite::{params, Connection};
use std::sync::Arc;

/// Row tuple from `search_entities_by_name_embedding`:
/// `(entity_id, name, entity_type, distance, confidence)`. Aliased to
/// keep the trait wrapper signature within clippy's `type_complexity`
/// budget. `confidence` was added in MEM-001 Part B-1.
type EntityNameAnnHit = (String, String, String, f32, f64);

/// Row returned by `traverse_weighted_sync` (MEM-001 Part B-2). The
/// caller dedupes to one entry per `entity_id` — multiple paths from
/// the seed produce multiple rows, the trait wrapper picks the
/// shortest hop and ties on the highest edge-confidence product.
#[derive(Debug, Clone)]
pub struct TraverseWeightedRow {
    pub entity_id: String,
    pub name: String,
    pub entity_type: String,
    pub hop: usize,
    pub path: String,
    pub entity_confidence: f64,
    pub edge_confidence_product: f64,
}

/// Row tuple from `list_inter_cluster_relations`:
/// `(id, source_entity_id, target_entity_id, relationship_type, layer)`.
/// Aliased to keep the trait wrapper signature within clippy's
/// `type_complexity` budget.
type InterClusterRelationRow = (String, String, String, String, i64);

/// Row tuple from `hierarchy_summary`'s top-aggregates query:
/// `(id, name, layer, member_count, description)`. Same aliasing
/// rationale as `InterClusterRelationRow`.
type AggregateSummaryRow = (String, String, i64, usize, String);

/// Return tuple from `hierarchy_summary`:
/// `(layer_counts, inter_cluster_total, top_aggregates)`. Trait
/// wrapper maps it into the public `HierarchySummary`.
type HierarchySummaryRow = (Vec<(i64, usize)>, usize, Vec<AggregateSummaryRow>);

/// Build a comma-separated list of positional placeholders for SQL
/// `IN ()` clauses: `placeholder_list(2, 3)` → `"?2,?3,?4"`. Callers
/// are responsible for binding the exact same number of parameters in
/// the same positions.
fn placeholder_list(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Convert a `GraphError` into a `rusqlite::Error` so that closures passed to
/// `KnowledgeDatabase::with_connection` (which require `rusqlite::Result`) can
/// propagate our higher-level errors. `with_connection` stringifies the result
/// anyway, so the round-trip fidelity is acceptable.
fn graph_to_rusqlite(e: GraphError) -> rusqlite::Error {
    match e {
        GraphError::Database(db) => db,
        other => {
            rusqlite::Error::InvalidColumnType(0, format!("{other:?}"), rusqlite::types::Type::Null)
        }
    }
}

/// Tuple shape returned by the entity-row `parse_entity` closure used by
/// `list_entities` / `find_entities_by_*`. Centralized so the row-shaped
/// result can be passed to `build_entity_from_row` without re-listing fields.
type EntityRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
);

/// Convert an entity-shaped row tuple into a domain `Entity`. Extracted so
/// per-row construction does not nest inside the SQL-building functions and
/// keeps their cognitive complexity below the threshold.
fn build_entity_from_row(row: EntityRow) -> Entity {
    let (
        id,
        agent_id,
        entity_type_str,
        name,
        properties_json,
        first_seen_at,
        last_seen_at,
        mention_count,
    ) = row;
    let entity_type = EntityType::from_str(&entity_type_str);
    let properties = properties_json
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    Entity {
        id,
        agent_id,
        entity_type,
        name,
        properties,
        first_seen_at: chrono::DateTime::parse_from_rfc3339(&first_seen_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        last_seen_at: chrono::DateTime::parse_from_rfc3339(&last_seen_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        mention_count,
        name_embedding: None,
    }
}

/// Tuple shape returned by the relationship-row `parse_relationship` closure
/// used by `get_relationships` / `list_relationships` / `list_all_relationships`.
type RelationshipRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
);

/// Convert a relationship-shaped row tuple into a domain `Relationship`.
fn build_relationship_from_row(row: RelationshipRow) -> Relationship {
    let (
        id,
        agent_id,
        source_entity_id,
        target_entity_id,
        rel_type_str,
        properties_json,
        first_seen_at,
        last_seen_at,
        mention_count,
    ) = row;
    let relationship_type = RelationshipType::from_str(&rel_type_str);
    let properties = properties_json
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    Relationship {
        id,
        agent_id,
        source_entity_id,
        target_entity_id,
        relationship_type,
        properties,
        first_seen_at: chrono::DateTime::parse_from_rfc3339(&first_seen_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        last_seen_at: chrono::DateTime::parse_from_rfc3339(&last_seen_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        mention_count,
    }
}

/// SQLite-backed storage for the knowledge graph, sharing the pooled
/// `KnowledgeDatabase` connection for `knowledge.db` (schema v22).
pub struct GraphStorage {
    pub(crate) db: Arc<KnowledgeDatabase>,
}

impl GraphStorage {
    /// Create a new graph storage backed by the shared `KnowledgeDatabase` pool.
    ///
    /// Schema v22 is already initialized by `KnowledgeDatabase::new`, so this
    /// constructor is effectively a no-op wrapper.
    pub fn new(db: Arc<KnowledgeDatabase>) -> GraphResult<Self> {
        Ok(Self { db })
    }

    /// Borrow the underlying knowledge database pool. Used by
    /// `zbot-stores-sqlite` to drive impl-specific maintenance routines
    /// (e.g. `reindex_embeddings`) that need raw SQL access without
    /// fighting the existing `pub(crate)` field visibility.
    pub fn knowledge_db(&self) -> &Arc<KnowledgeDatabase> {
        &self.db
    }

    /// Store extracted knowledge (entities and relationships)
    pub fn store_knowledge(
        &self,
        agent_id: &str,
        knowledge: ExtractedKnowledge,
    ) -> GraphResult<()> {
        self.db
            .with_connection(|conn| {
                // `with_connection` provides `&Connection` (not `&mut`), so we
                // use `unchecked_transaction` which is available on shared refs.
                // All entity + relationship inserts are wrapped in a single
                // transaction so a partial failure cannot leave the graph in an
                // inconsistent state (trait contract: atomic all-or-nothing).
                (|| -> GraphResult<()> {
                    let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;

                    // Store entities and build ID mapping (new_id → actual_id).
                    // `Transaction` auto-derefs to `&Connection`, so helpers accept `&tx`.
                    let mut entity_id_map: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
                    for entity in knowledge.entities {
                        let original_id = entity.id.clone();
                        let actual_id = store_entity(&tx, agent_id, entity)?;
                        entity_id_map.insert(original_id, actual_id);
                    }

                    for mut relationship in knowledge.relationships {
                        if let Some(mapped) = entity_id_map.get(&relationship.source_entity_id) {
                            relationship.source_entity_id = mapped.clone();
                        }
                        if let Some(mapped) = entity_id_map.get(&relationship.target_entity_id) {
                            relationship.target_entity_id = mapped.clone();
                        }
                        store_relationship(&tx, agent_id, relationship)?;
                    }

                    tx.commit().map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Get all entities for an agent (includes __global__ entities)
    pub fn get_entities(&self, agent_id: &str) -> GraphResult<Vec<Entity>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Entity>> {
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities WHERE agent_id = ?1 OR agent_id = '__global__'"
        ).map_err(GraphError::Database)?;

        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .map_err(GraphError::Database)?;

        let mut entities = Vec::new();
        for row in rows {
            let (
                id,
                agent_id,
                entity_type_str,
                name,
                properties_json,
                first_seen_at,
                last_seen_at,
                mention_count,
            ) = row?;

            let entity_type = EntityType::from_str(&entity_type_str);
            let properties = if let Some(json) = properties_json {
                serde_json::from_str(&json).unwrap_or_default()
            } else {
                Default::default()
            };

            entities.push(Entity {
                id,
                agent_id,
                entity_type,
                name,
                properties,
                first_seen_at: chrono::DateTime::parse_from_rfc3339(&first_seen_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                last_seen_at: chrono::DateTime::parse_from_rfc3339(&last_seen_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                mention_count,
                name_embedding: None,
            });
        }

        Ok(entities)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Get all relationships for an agent (includes __global__ relationships)
    pub fn get_relationships(&self, agent_id: &str) -> GraphResult<Vec<Relationship>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Relationship>> {

        let mut stmt = conn.prepare(
            "SELECT id, agent_id, source_entity_id, target_entity_id, relationship_type, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_relationships WHERE agent_id = ?1 OR agent_id = '__global__'"
        ).map_err(GraphError::Database)?;

        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(GraphError::Database)?;

        let relationships = rows
            .map(|row| row.map(build_relationship_from_row))
            .collect::<Result<Vec<Relationship>, _>>()?;

        Ok(relationships)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Search entities by name (includes __global__ entities)
    pub fn search_entities(&self, agent_id: &str, query: &str) -> GraphResult<Vec<Entity>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Entity>> {

        let mut stmt = conn.prepare(
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities
             WHERE (agent_id = ?1 OR agent_id = '__global__') AND name LIKE ?2
             ORDER BY mention_count DESC"
        ).map_err(GraphError::Database)?;

        let pattern = format!("%{}%", query);
        let rows = stmt
            .query_map(params![agent_id, pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .map_err(GraphError::Database)?;

        let mut entities = Vec::new();
        for row in rows {
            let (
                id,
                agent_id,
                entity_type_str,
                name,
                properties_json,
                first_seen_at,
                last_seen_at,
                mention_count,
            ) = row?;

            let entity_type = EntityType::from_str(&entity_type_str);
            let properties = if let Some(json) = properties_json {
                serde_json::from_str(&json).unwrap_or_default()
            } else {
                Default::default()
            };

            entities.push(Entity {
                id,
                agent_id,
                entity_type,
                name,
                properties,
                first_seen_at: chrono::DateTime::parse_from_rfc3339(&first_seen_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                last_seen_at: chrono::DateTime::parse_from_rfc3339(&last_seen_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                mention_count,
                name_embedding: None,
            });
        }

        Ok(entities)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Search entities ordered by a caller-provided clause.
    ///
    /// `order_clause` is whitelisted to prevent SQL injection; any value
    /// outside the whitelist falls back to `mention_count DESC`.
    pub fn search_entities_order_by(
        &self,
        agent_id: &str,
        query: &str,
        order_clause: &str,
        limit: usize,
    ) -> GraphResult<Vec<Entity>> {
        // Whitelist to prevent SQL injection — only accept known safe clauses.
        let safe_order = match order_clause {
            "last_seen_at DESC" | "mention_count DESC" | "first_seen_at DESC" => order_clause,
            _ => "mention_count DESC",
        };

        let sql = format!(
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities
             WHERE (agent_id = ?1 OR agent_id = '__global__') AND name LIKE ?2
             ORDER BY {}
             LIMIT ?3",
            safe_order
        );

        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Entity>> {
                    let mut stmt = conn.prepare(&sql).map_err(GraphError::Database)?;

                    let pattern = format!("%{}%", query);
                    let rows = stmt
                        .query_map(params![agent_id, pattern, limit as i64], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, String>(6)?,
                                row.get::<_, i64>(7)?,
                            ))
                        })
                        .map_err(GraphError::Database)?;

                    let mut entities = Vec::new();
                    for row in rows {
                        let (
                            id,
                            agent_id,
                            entity_type_str,
                            name,
                            properties_json,
                            first_seen_at,
                            last_seen_at,
                            mention_count,
                        ) = row?;

                        let entity_type = EntityType::from_str(&entity_type_str);
                        let properties = if let Some(json) = properties_json {
                            serde_json::from_str(&json).unwrap_or_default()
                        } else {
                            Default::default()
                        };

                        entities.push(Entity {
                            id,
                            agent_id,
                            entity_type,
                            name,
                            properties,
                            first_seen_at: chrono::DateTime::parse_from_rfc3339(&first_seen_at)
                                .map(|dt| dt.with_timezone(&chrono::Utc))
                                .unwrap_or_else(|_| chrono::Utc::now()),
                            last_seen_at: chrono::DateTime::parse_from_rfc3339(&last_seen_at)
                                .map(|dt| dt.with_timezone(&chrono::Utc))
                                .unwrap_or_else(|_| chrono::Utc::now()),
                            mention_count,
                            name_embedding: None,
                        });
                    }

                    Ok(entities)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Count relationships referencing a particular entity (source OR target).
    pub fn count_relationships_for(&self, entity_id: &str) -> GraphResult<i64> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<i64> {
                    let count: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) FROM kg_relationships
                 WHERE source_entity_id = ?1 OR target_entity_id = ?1",
                            params![entity_id],
                            |row| row.get(0),
                        )
                        .map_err(GraphError::Database)?;
                    Ok(count)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Find an existing entity by agent_id + name (case-insensitive), returning its ID.
    pub fn find_entity_by_name(&self, agent_id: &str, name: &str) -> GraphResult<Option<String>> {
        self.db
            .with_connection(|conn| {
                find_entity_by_name(conn, agent_id, name).map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// ANN search over `kg_name_index` (sqlite-vec virtual table) for the
    /// nearest entity names to `query_embedding`.
    ///
    /// Returns rows of `(entity_id, name, entity_type, distance, confidence)`.
    /// `distance` is the L2-squared distance from the vec0 index — for
    /// L2-normalized vectors, cosine similarity ≈ `1 - distance / 2`. The
    /// query embedding MUST be L2-normalized by the caller. Results are
    /// filtered to the caller's agent plus `__global__` and ordered by
    /// ascending distance. `confidence` is the current `kg_entities.confidence`
    /// for downstream weighting (MEM-001 Part B-1).
    pub fn search_entities_by_name_embedding(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        agent_id: &str,
    ) -> GraphResult<Vec<EntityNameAnnHit>> {
        if query_embedding.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }
        let embedding_json = serde_json::to_string(query_embedding)
            .map_err(|e| GraphError::Other(format!("serialize query embedding: {e}")))?;

        // vec0 KNN queries require a bare `k = ?` or `LIMIT ?` on the virtual
        // table itself — extra predicates/joins aren't accepted at prepare
        // time. Pull top-K by distance, then filter against `kg_entities`.
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<EntityNameAnnHit>> {
                    let mut ann_stmt = conn
                        .prepare(
                            "SELECT entity_id, distance \
                             FROM kg_name_index \
                             WHERE name_embedding MATCH ?1 \
                             ORDER BY distance \
                             LIMIT ?2",
                        )
                        .map_err(GraphError::Database)?;
                    let hits = ann_stmt
                        .query_map(params![embedding_json, top_k as i64], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?))
                        })
                        .map_err(GraphError::Database)?;

                    // MEM-001 Part B-1: pull `confidence` alongside the
                    // existing name + entity_type so recall can weight
                    // hits by entity confidence and skip below threshold.
                    let mut lookup_stmt = conn
                        .prepare(
                            "SELECT name, entity_type, confidence FROM kg_entities \
                             WHERE id = ?1 \
                               AND (agent_id = ?2 OR agent_id = '__global__')",
                        )
                        .map_err(GraphError::Database)?;

                    let mut out = Vec::new();
                    for row in hits {
                        let (entity_id, dist) = row.map_err(GraphError::Database)?;
                        let mut rows = lookup_stmt
                            .query(params![entity_id, agent_id])
                            .map_err(GraphError::Database)?;
                        if let Some(r) = rows.next().map_err(GraphError::Database)? {
                            let name: String = r.get(0).map_err(GraphError::Database)?;
                            let etype: String = r.get(1).map_err(GraphError::Database)?;
                            let confidence: f64 = r.get(2).map_err(GraphError::Database)?;
                            out.push((entity_id, name, etype, dist, confidence));
                        }
                    }
                    Ok(out)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Increment mention count and update last_seen for an existing entity.
    pub fn bump_entity_mention(&self, entity_id: &str) -> GraphResult<()> {
        self.db
            .with_connection(|conn| bump_entity_mention(conn, entity_id).map_err(graph_to_rusqlite))
            .map_err(GraphError::Other)
    }

    /// Store (upsert) a single entity for an agent, returning its canonical ID.
    ///
    /// Delegates to the private `store_entity` function which handles
    /// name-normalisation, deduplication, and alias seeding.
    pub fn upsert_entity(&self, agent_id: &str, entity: Entity) -> GraphResult<String> {
        let agent_id = agent_id.to_string();
        self.db
            .with_connection(|conn| {
                store_entity(conn, &agent_id, entity).map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Store a relationship, merging properties if one with the same
    /// (source, target, type) already exists. Returns the relationship ID.
    pub fn upsert_relationship(
        &self,
        agent_id: &str,
        relationship: Relationship,
    ) -> GraphResult<String> {
        let src = relationship.source_entity_id.clone();
        let tgt = relationship.target_entity_id.clone();
        let rel_type_str = relationship.relationship_type.as_str().to_string();
        let agent_id = agent_id.to_string();
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<String> {
                    store_relationship(conn, &agent_id, relationship)?;
                    // After INSERT ... ON CONFLICT DO UPDATE the id column is
                    // never overwritten; query by the UNIQUE key to get the id.
                    conn.query_row(
                        "SELECT id FROM kg_relationships \
                         WHERE source_entity_id = ?1 \
                           AND target_entity_id = ?2 \
                           AND relationship_type = ?3 \
                         LIMIT 1",
                        rusqlite::params![src, tgt, rel_type_str],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(GraphError::Database)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Delete a single relationship by its ID.
    pub fn delete_relationship(&self, id: &str) -> GraphResult<()> {
        self.db
            .with_connection(|conn| {
                conn.execute(
                    "DELETE FROM kg_relationships WHERE id = ?1",
                    rusqlite::params![id],
                )
                .map(|_| ())
                .map_err(|e| graph_to_rusqlite(GraphError::Database(e)))
            })
            .map_err(GraphError::Other)
    }

    /// Fetch a single entity by its ID. Returns `None` if not found.
    pub fn get_entity_by_id(&self, entity_id: &str) -> GraphResult<Option<Entity>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Option<Entity>> {
                    let mut stmt = conn
                        .prepare(
                            "SELECT id, agent_id, entity_type, name, properties, \
                             first_seen_at, last_seen_at, mention_count \
                             FROM kg_entities WHERE id = ?1 LIMIT 1",
                        )
                        .map_err(GraphError::Database)?;

                    let mut rows = stmt
                        .query_map(params![entity_id], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, String>(6)?,
                                row.get::<_, i64>(7)?,
                            ))
                        })
                        .map_err(GraphError::Database)?;

                    match rows.next() {
                        Some(row) => Ok(Some(build_entity_from_row(row?))),
                        None => Ok(None),
                    }
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Delete a single entity by its ID. Also removes any associated
    /// relationships and alias entries to keep the graph consistent.
    ///
    /// All four DELETEs (aliases, name_index, relationships, entity) are
    /// executed inside a single transaction so a crash between statements
    /// cannot leave the graph in a partially-deleted state.
    pub fn delete_entity_by_id(&self, entity_id: &str) -> GraphResult<()> {
        self.db
            .with_connection(|conn| {
                // `with_connection` provides `&Connection` (not `&mut`), so we
                // use `unchecked_transaction` which is available on shared refs.
                (|| -> GraphResult<()> {
                    let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;
                    tx.execute(
                        "DELETE FROM kg_aliases WHERE entity_id = ?1",
                        params![entity_id],
                    )
                    .map_err(GraphError::Database)?;
                    tx.execute(
                        "DELETE FROM kg_name_index WHERE entity_id = ?1",
                        params![entity_id],
                    )
                    .map_err(GraphError::Database)?;
                    tx.execute(
                        "DELETE FROM kg_relationships \
                         WHERE source_entity_id = ?1 OR target_entity_id = ?1",
                        params![entity_id],
                    )
                    .map_err(GraphError::Database)?;
                    tx.execute("DELETE FROM kg_entities WHERE id = ?1", params![entity_id])
                        .map_err(GraphError::Database)?;
                    tx.commit().map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Delete all data for an agent
    pub fn delete_agent_data(&self, agent_id: &str) -> GraphResult<usize> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<usize> {
                    // Delete relationships
                    let rel_count = conn
                        .execute(
                            "DELETE FROM kg_relationships WHERE agent_id = ?1",
                            params![agent_id],
                        )
                        .map_err(GraphError::Database)?;

                    // Delete entities
                    let ent_count = conn
                        .execute(
                            "DELETE FROM kg_entities WHERE agent_id = ?1",
                            params![agent_id],
                        )
                        .map_err(GraphError::Database)?;

                    Ok(rel_count + ent_count)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    // ===== NEW READ METHODS (Phase 1: Graph Repository Layer) =====

    /// List entities for an agent with optional type filter and pagination
    pub fn list_entities(
        &self,
        agent_id: &str,
        entity_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GraphResult<Vec<Entity>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Entity>> {

        // Build query and params based on whether type filter is provided
        let sql = if entity_type.is_some() {
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities
             WHERE (agent_id = ?1 OR agent_id = '__global__') AND entity_type = ?2
             ORDER BY mention_count DESC
             LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities
             WHERE agent_id = ?1 OR agent_id = '__global__'
             ORDER BY mention_count DESC
             LIMIT ?2 OFFSET ?3"
        };

        let mut stmt = conn.prepare(sql).map_err(GraphError::Database)?;

        // Helper to parse entity from row
        let parse_entity = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        };

        let rows = if let Some(type_filter) = entity_type {
            stmt.query_map(
                params![agent_id, type_filter, limit as i64, offset as i64],
                parse_entity,
            )
            .map_err(GraphError::Database)?
        } else {
            stmt.query_map(params![agent_id, limit as i64, offset as i64], parse_entity)
                .map_err(GraphError::Database)?
        };

        let entities = rows
            .map(|row_result| row_result.map(build_entity_from_row))
            .collect::<Result<Vec<Entity>, _>>()?;

        Ok(entities)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// List relationships for an agent with optional type filter and pagination
    pub fn list_relationships(
        &self,
        agent_id: &str,
        relationship_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GraphResult<Vec<Relationship>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Relationship>> {

        // Build query based on whether type filter is provided
        let sql = if relationship_type.is_some() {
            "SELECT id, agent_id, source_entity_id, target_entity_id, relationship_type, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_relationships
             WHERE (agent_id = ?1 OR agent_id = '__global__') AND relationship_type = ?2
             ORDER BY mention_count DESC
             LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT id, agent_id, source_entity_id, target_entity_id, relationship_type, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_relationships
             WHERE agent_id = ?1 OR agent_id = '__global__'
             ORDER BY mention_count DESC
             LIMIT ?2 OFFSET ?3"
        };

        let mut stmt = conn.prepare(sql).map_err(GraphError::Database)?;

        // Helper to parse relationship from row
        let parse_relationship = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
            ))
        };

        let rows = if let Some(type_filter) = relationship_type {
            stmt.query_map(
                params![agent_id, type_filter, limit as i64, offset as i64],
                parse_relationship,
            )
            .map_err(GraphError::Database)?
        } else {
            stmt.query_map(
                params![agent_id, limit as i64, offset as i64],
                parse_relationship,
            )
            .map_err(GraphError::Database)?
        };

        let relationships = rows
            .map(|row_result| row_result.map(build_relationship_from_row))
            .collect::<Result<Vec<Relationship>, _>>()?;

        Ok(relationships)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Get entity by name (case-insensitive)
    pub fn get_entity_by_name(&self, agent_id: &str, name: &str) -> GraphResult<Option<Entity>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Option<Entity>> {

        let mut stmt = conn.prepare(
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities
             WHERE (agent_id = ?1 OR agent_id = '__global__') AND name = ?2 COLLATE NOCASE
             LIMIT 1"
        ).map_err(GraphError::Database)?;

        let lower_name = name.to_lowercase();
        let mut rows = stmt
            .query_map(params![agent_id, lower_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .map_err(GraphError::Database)?;

        if let Some(row) = rows.next() {
            let (
                id,
                agent_id,
                entity_type_str,
                name,
                properties_json,
                first_seen_at,
                last_seen_at,
                mention_count,
            ) = row?;

            let entity_type = EntityType::from_str(&entity_type_str);
            let properties = if let Some(json) = properties_json {
                serde_json::from_str(&json).unwrap_or_default()
            } else {
                Default::default()
            };

            Ok(Some(Entity {
                id,
                agent_id,
                entity_type,
                name,
                properties,
                first_seen_at: chrono::DateTime::parse_from_rfc3339(&first_seen_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                last_seen_at: chrono::DateTime::parse_from_rfc3339(&last_seen_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                mention_count,
                name_embedding: None,
            }))
        } else {
            Ok(None)
        }
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Get neighbors of an entity (1-hop)
    pub fn get_neighbors(
        &self,
        _agent_id: &str,
        entity_id: &str,
        direction: Direction,
        limit: usize,
    ) -> GraphResult<Vec<NeighborInfo>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<NeighborInfo>> {

        let mut neighbors = Vec::new();

        // Get outgoing neighbors (Entity → Other)
        if direction == Direction::Outgoing || direction == Direction::Both {
            let mut stmt = conn.prepare(
                "SELECT e.id, e.agent_id, e.entity_type, e.name, e.properties, e.first_seen_at, e.last_seen_at, e.mention_count,
                        r.id, r.agent_id, r.source_entity_id, r.target_entity_id, r.relationship_type, r.properties, r.first_seen_at, r.last_seen_at, r.mention_count
                 FROM kg_entities e
                 INNER JOIN kg_relationships r ON r.target_entity_id = e.id
                 WHERE r.source_entity_id = ?1
                 ORDER BY r.mention_count DESC
                 LIMIT ?2"
            ).map_err(GraphError::Database)?;

            let rows = stmt
                .query_map(params![entity_id, limit as i64], |row| {
                    Ok((
                        // Entity fields
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        // Relationship fields
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, String>(14)?,
                        row.get::<_, String>(15)?,
                        row.get::<_, i64>(16)?,
                    ))
                })
                .map_err(GraphError::Database)?;

            for row in rows {
                let (
                    e_id,
                    e_agent_id,
                    e_type_str,
                    e_name,
                    e_props_json,
                    e_first,
                    e_last,
                    e_mentions,
                    r_id,
                    r_agent_id,
                    r_source,
                    r_target,
                    r_type_str,
                    r_props_json,
                    r_first,
                    r_last,
                    r_mentions,
                ) = row?;

                let entity = Entity {
                    id: e_id,
                    agent_id: e_agent_id,
                    entity_type: EntityType::from_str(&e_type_str),
                    name: e_name,
                    properties: e_props_json
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default(),
                    first_seen_at: chrono::DateTime::parse_from_rfc3339(&e_first)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    last_seen_at: chrono::DateTime::parse_from_rfc3339(&e_last)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    mention_count: e_mentions,
                    name_embedding: None,
                };

                let relationship = Relationship {
                    id: r_id,
                    agent_id: r_agent_id,
                    source_entity_id: r_source,
                    target_entity_id: r_target,
                    relationship_type: RelationshipType::from_str(&r_type_str),
                    properties: r_props_json
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default(),
                    first_seen_at: chrono::DateTime::parse_from_rfc3339(&r_first)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    last_seen_at: chrono::DateTime::parse_from_rfc3339(&r_last)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    mention_count: r_mentions,
                };

                neighbors.push(NeighborInfo {
                    entity,
                    relationship,
                    direction: Direction::Outgoing,
                });
            }
        }

        // Get incoming neighbors (Other → Entity)
        if direction == Direction::Incoming || direction == Direction::Both {
            let mut stmt = conn.prepare(
                "SELECT e.id, e.agent_id, e.entity_type, e.name, e.properties, e.first_seen_at, e.last_seen_at, e.mention_count,
                        r.id, r.agent_id, r.source_entity_id, r.target_entity_id, r.relationship_type, r.properties, r.first_seen_at, r.last_seen_at, r.mention_count
                 FROM kg_entities e
                 INNER JOIN kg_relationships r ON r.source_entity_id = e.id
                 WHERE r.target_entity_id = ?1
                 ORDER BY r.mention_count DESC
                 LIMIT ?2"
            ).map_err(GraphError::Database)?;

            let rows = stmt
                .query_map(params![entity_id, limit as i64], |row| {
                    Ok((
                        // Entity fields
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        // Relationship fields
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, String>(14)?,
                        row.get::<_, String>(15)?,
                        row.get::<_, i64>(16)?,
                    ))
                })
                .map_err(GraphError::Database)?;

            for row in rows {
                let (
                    e_id,
                    e_agent_id,
                    e_type_str,
                    e_name,
                    e_props_json,
                    e_first,
                    e_last,
                    e_mentions,
                    r_id,
                    r_agent_id,
                    r_source,
                    r_target,
                    r_type_str,
                    r_props_json,
                    r_first,
                    r_last,
                    r_mentions,
                ) = row?;

                let entity = Entity {
                    id: e_id,
                    agent_id: e_agent_id,
                    entity_type: EntityType::from_str(&e_type_str),
                    name: e_name,
                    properties: e_props_json
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default(),
                    first_seen_at: chrono::DateTime::parse_from_rfc3339(&e_first)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    last_seen_at: chrono::DateTime::parse_from_rfc3339(&e_last)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    mention_count: e_mentions,
                    name_embedding: None,
                };

                let relationship = Relationship {
                    id: r_id,
                    agent_id: r_agent_id,
                    source_entity_id: r_source,
                    target_entity_id: r_target,
                    relationship_type: RelationshipType::from_str(&r_type_str),
                    properties: r_props_json
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default(),
                    first_seen_at: chrono::DateTime::parse_from_rfc3339(&r_first)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    last_seen_at: chrono::DateTime::parse_from_rfc3339(&r_last)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    mention_count: r_mentions,
                };

                neighbors.push(NeighborInfo {
                    entity,
                    relationship,
                    direction: Direction::Incoming,
                });
            }
        }

        Ok(neighbors)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// BFS traversal from a seed entity up to `max_hops` with a result cap.
    ///
    /// Runs the same recursive CTE used by [`SqliteGraphTraversal`], callable
    /// from a synchronous `spawn_blocking` closure in `zbot-stores-sqlite`.
    pub fn traverse_sync(
        &self,
        entity_id: &str,
        max_hops: u8,
        limit: usize,
    ) -> GraphResult<Vec<super::traversal::TraversalNode>> {
        let hop_decay = 0.7_f64;
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<super::traversal::TraversalNode>> {
                    let sql = r#"
                        WITH RECURSIVE graph_walk(entity_id, hop, path, visited) AS (
                            SELECT ?1, 0, '', ?1
                            UNION ALL
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
                            MIN(gw.hop) AS min_hop,
                            gw.path,
                            e.mention_count
                        FROM graph_walk gw
                        JOIN kg_entities e ON e.id = gw.entity_id
                        WHERE gw.entity_id != ?1
                        GROUP BY gw.entity_id
                        ORDER BY min_hop ASC, e.mention_count DESC
                        LIMIT ?3
                    "#;
                    let mut stmt = conn.prepare(sql).map_err(GraphError::Database)?;
                    let rows = stmt
                        .query_map(
                            rusqlite::params![entity_id, max_hops as i64, limit as i64],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, i64>(3)?,
                                    row.get::<_, String>(4)?,
                                    row.get::<_, i64>(5)?,
                                ))
                            },
                        )
                        .map_err(GraphError::Database)?;
                    let mut nodes = Vec::new();
                    for row in rows {
                        let (id, _name, _etype, hop, path, mentions) =
                            row.map_err(GraphError::Database)?;
                        let hop_u8 = hop as u8;
                        nodes.push(super::traversal::TraversalNode {
                            entity_id: id,
                            entity_name: _name,
                            entity_type: _etype,
                            hop_distance: hop_u8,
                            path,
                            relevance: hop_decay.powi(hop_u8 as i32),
                            mention_count: mentions,
                        });
                    }
                    Ok(nodes)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// MEM-001 Part B-2 — confidence-aware BFS traversal for recall.
    ///
    /// Mirrors `traverse_sync`'s recursive CTE but:
    /// 1. Filters edges where `confidence < min_edge_confidence` at the CTE
    ///    level — contradicted / decayed relationships don't pollute recall.
    /// 2. Carries an `edge_confidence_product` accumulator along each path
    ///    (multiplicative across edges).
    /// 3. Joins `kg_entities.confidence` so recall can weight the final
    ///    score by the target entity's confidence too.
    /// 4. Agent-scopes both relationships and entities (`__global__` is
    ///    treated as cross-agent and included).
    ///
    /// Returns one row per path; the caller dedupes to one entry per
    /// `entity_id` (shortest hop wins; tie-break by highest edge product).
    /// The seed itself is excluded.
    pub fn traverse_weighted_sync(
        &self,
        entity_id: &str,
        agent_id: &str,
        max_hops: u8,
        min_edge_confidence: f64,
        limit: usize,
    ) -> GraphResult<Vec<TraverseWeightedRow>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<TraverseWeightedRow>> {
                    let sql = r#"
                        WITH RECURSIVE graph_walk(entity_id, hop, path, visited, edge_conf_product) AS (
                            SELECT ?1, 0, '', ?1, 1.0
                            UNION ALL
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
                                    END,
                                gw.edge_conf_product * r.confidence
                            FROM graph_walk gw
                            JOIN kg_relationships r
                                ON (r.source_entity_id = gw.entity_id
                                    OR r.target_entity_id = gw.entity_id)
                            WHERE gw.hop < ?2
                              AND r.confidence >= ?3
                              AND (r.agent_id = ?4 OR r.agent_id = '__global__')
                              AND r.epistemic_class != 'archival'
                              AND gw.visited NOT LIKE
                                  '%' ||
                                  CASE WHEN r.source_entity_id = gw.entity_id
                                       THEN r.target_entity_id
                                       ELSE r.source_entity_id
                                  END || '%'
                        )
                        SELECT
                            gw.entity_id,
                            e.name,
                            e.entity_type,
                            gw.hop,
                            gw.path,
                            e.confidence,
                            gw.edge_conf_product
                        FROM graph_walk gw
                        JOIN kg_entities e ON e.id = gw.entity_id
                        WHERE gw.entity_id != ?1
                          AND (e.agent_id = ?4 OR e.agent_id = '__global__')
                          AND e.epistemic_class != 'archival'
                        ORDER BY gw.hop ASC, gw.edge_conf_product DESC
                        LIMIT ?5
                    "#;
                    let mut stmt = conn.prepare(sql).map_err(GraphError::Database)?;
                    let rows = stmt
                        .query_map(
                            rusqlite::params![
                                entity_id,
                                max_hops as i64,
                                min_edge_confidence,
                                agent_id,
                                limit as i64
                            ],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, i64>(3)?,
                                    row.get::<_, String>(4)?,
                                    row.get::<_, f64>(5)?,
                                    row.get::<_, f64>(6)?,
                                ))
                            },
                        )
                        .map_err(GraphError::Database)?;
                    let mut out = Vec::new();
                    for row in rows {
                        let (id, name, etype, hop, path, entity_conf, edge_conf_prod) =
                            row.map_err(GraphError::Database)?;
                        out.push(TraverseWeightedRow {
                            entity_id: id,
                            name,
                            entity_type: etype,
                            hop: hop as usize,
                            path,
                            entity_confidence: entity_conf,
                            edge_confidence_product: edge_conf_prod,
                        });
                    }
                    Ok(out)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Search entities by name with a result limit (LIKE match, case-insensitive).
    pub fn search_by_name(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> GraphResult<Vec<Entity>> {
        self.search_entities_order_by(agent_id, query, "mention_count DESC", limit)
    }

    /// Count entities for an agent
    pub fn count_entities(&self, agent_id: &str) -> GraphResult<usize> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<usize> {
                    let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_entities WHERE agent_id = ?1 OR agent_id = '__global__'",
                params![agent_id],
                |row| row.get(0),
            )
            .map_err(GraphError::Database)?;

                    Ok(count as usize)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Count relationships for an agent
    pub fn count_relationships(&self, agent_id: &str) -> GraphResult<usize> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<usize> {
                    let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM kg_relationships WHERE agent_id = ?1 OR agent_id = '__global__'",
            params![agent_id],
            |row| row.get(0),
        ).map_err(GraphError::Database)?;

                    Ok(count as usize)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Count all entities across all agents.
    pub fn count_all_entities(&self) -> GraphResult<usize> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<usize> {
                    let count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM kg_entities", [], |row| row.get(0))
                        .map_err(GraphError::Database)?;
                    Ok(count as usize)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Count all relationships across all agents.
    pub fn count_all_relationships(&self) -> GraphResult<usize> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<usize> {
                    let count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM kg_relationships", [], |row| {
                            row.get(0)
                        })
                        .map_err(GraphError::Database)?;
                    Ok(count as usize)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Return aggregate counts for entities, relationships, and aliases across
    /// all agents. Used by `KnowledgeGraphStore::stats`.
    pub fn stats(&self) -> GraphResult<(u64, u64, u64)> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<(u64, u64, u64)> {
                    let entity_count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM kg_entities", [], |r| r.get(0))
                        .map_err(GraphError::Database)?;
                    let relationship_count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM kg_relationships", [], |r| r.get(0))
                        .map_err(GraphError::Database)?;
                    let alias_count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM kg_aliases", [], |r| r.get(0))
                        .map_err(GraphError::Database)?;
                    Ok((
                        entity_count as u64,
                        relationship_count as u64,
                        alias_count as u64,
                    ))
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// List all relationships across all agents with pagination.
    ///
    /// Used by the Observatory "All Agents" mode to render edges.
    pub fn list_all_relationships(&self, limit: usize) -> GraphResult<Vec<Relationship>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Relationship>> {

        let mut stmt = conn.prepare(
            "SELECT id, agent_id, source_entity_id, target_entity_id, relationship_type, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_relationships
             ORDER BY mention_count DESC
             LIMIT ?1"
        ).map_err(GraphError::Database)?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(GraphError::Database)?;

        let relationships = rows
            .map(|row| row.map(build_relationship_from_row))
            .collect::<Result<Vec<Relationship>, _>>()?;

        Ok(relationships)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Hierarchical-memory health snapshot — layer counts + total
    /// inter-cluster edge count + top-N aggregates by member size.
    /// Used by `GET /api/hierarchy/stats` (Observatory pill + slideover).
    ///
    /// One DB session, three queries: count-by-layer, total
    /// inter-cluster relations, top-N aggregates ordered by
    /// `member_count` parsed from `properties` JSON. Dual-matches
    /// `agent_id` against `__global__` (same idiom as the other
    /// hierarchy reads — base entities live under that sentinel).
    ///
    /// Returns a triple to keep the trait-wrapper signature simple;
    /// the caller maps it to the public `HierarchySummary` shape.
    pub fn hierarchy_summary(
        &self,
        agent_id: &str,
        top_n: usize,
    ) -> GraphResult<HierarchySummaryRow> {
        let agent_id_for_db = agent_id.to_string();
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<HierarchySummaryRow> {
                    let mut layer_stmt = conn
                        .prepare(
                            "SELECT layer, COUNT(*) FROM kg_entities \
                             WHERE (agent_id = ?1 OR agent_id = '__global__') \
                               AND epistemic_class = 'current' \
                             GROUP BY layer ORDER BY layer ASC",
                        )
                        .map_err(GraphError::Database)?;
                    let layer_counts: Vec<(i64, usize)> = layer_stmt
                        .query_map(params![agent_id_for_db], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as usize))
                        })
                        .map_err(GraphError::Database)?
                        .filter_map(Result::ok)
                        .collect();

                    let inter_cluster: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) FROM kg_relationships \
                             WHERE (agent_id = ?1 OR agent_id = '__global__') \
                               AND is_inter_cluster = 1 \
                               AND epistemic_class = 'current'",
                            params![agent_id_for_db],
                            |row| row.get(0),
                        )
                        .map_err(GraphError::Database)?;

                    let top_aggregates: Vec<AggregateSummaryRow> = if top_n == 0 {
                        Vec::new()
                    } else {
                        let mut agg_stmt = conn
                            .prepare(
                                "SELECT id, name, layer, properties \
                                 FROM kg_entities \
                                 WHERE (agent_id = ?1 OR agent_id = '__global__') \
                                   AND layer > 0 \
                                   AND epistemic_class = 'current' \
                                 ORDER BY CAST(json_extract(properties, '$.member_count') AS INTEGER) DESC \
                                 LIMIT ?2",
                            )
                            .map_err(GraphError::Database)?;
                        let mapped = agg_stmt
                            .query_map(params![agent_id_for_db, top_n as i64], |row| {
                                let id: String = row.get(0)?;
                                let name: String = row.get(1)?;
                                let layer: i64 = row.get(2)?;
                                let props_json: Option<String> = row.get(3)?;
                                let (member_count, description) = match props_json {
                                    Some(s) => {
                                        let v: serde_json::Value = serde_json::from_str(&s)
                                            .unwrap_or(serde_json::json!({}));
                                        let mc = v
                                            .get("member_count")
                                            .and_then(|n| n.as_u64())
                                            .unwrap_or(1)
                                            as usize;
                                        let desc = v
                                            .get("description")
                                            .and_then(|d| d.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        (mc, desc)
                                    }
                                    None => (1, String::new()),
                                };
                                Ok((id, name, layer, member_count, description))
                            })
                            .map_err(GraphError::Database)?;
                        let collected: Vec<AggregateSummaryRow> =
                            mapped.filter_map(Result::ok).collect();
                        drop(agg_stmt);
                        collected
                    };

                    Ok((layer_counts, inter_cluster as usize, top_aggregates))
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// List inter-cluster relations whose BOTH endpoints sit in
    /// `entity_ids` (Phase H-4 follow-up).
    ///
    /// Used by recall step 5c after `compute_lca_path` to surface the
    /// synthesised edges along the LCA path. Filters
    /// `is_inter_cluster = 1` AND `epistemic_class = 'current'` AND
    /// `agent_id` match. Empty input short-circuits to an empty
    /// result (`IN ()` would be a SQL syntax error).
    pub fn list_inter_cluster_relations(
        &self,
        agent_id: &str,
        entity_ids: &[String],
    ) -> GraphResult<Vec<InterClusterRelationRow>> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }

        let n = entity_ids.len();
        let src_in = placeholder_list(2, n);
        let tgt_in = placeholder_list(2 + n, n);
        // Dual-match `agent_id` against the queried agent OR `__global__`
        // because base entities (and the relationships among them) are
        // stored under `agent_id = '__global__'` by the cross-agent
        // dedup convention. Without this, a daemon configured for
        // `agent_id = 'root'` sees zero hierarchy data even when the
        // graph is full. Same idiom as `search_entities_by_name_embedding`.
        let sql = format!(
            "SELECT id, source_entity_id, target_entity_id, relationship_type, layer \
             FROM kg_relationships \
             WHERE (agent_id = ?1 OR agent_id = '__global__') \
               AND is_inter_cluster = 1 \
               AND epistemic_class = 'current' \
               AND source_entity_id IN ({src_in}) \
               AND target_entity_id IN ({tgt_in})"
        );

        // Bind: agent_id, then the id list twice (once per IN).
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(1 + 2 * n);
        params.push(Box::new(agent_id.to_string()));
        for id in entity_ids {
            params.push(Box::new(id.clone()));
        }
        for id in entity_ids {
            params.push(Box::new(id.clone()));
        }

        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<InterClusterRelationRow>> {
                    let mut stmt = conn.prepare(&sql).map_err(GraphError::Database)?;
                    let rows = stmt
                        .query_map(
                            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, String>(3)?,
                                    row.get::<_, i64>(4)?,
                                ))
                            },
                        )
                        .map_err(GraphError::Database)?;
                    Ok(rows.filter_map(|r| r.ok()).collect())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Count relationships bridging two entity clusters (Phase H-3c).
    ///
    /// Used by the future `HierarchyBuilder` sleep worker to decide
    /// whether a cluster pair is "connected enough" to justify
    /// synthesising an aggregate inter-cluster relation at the next
    /// hierarchy layer (LeanRAG's λ > τ gate).
    ///
    /// Counts both directions (A→B and B→A). Only `epistemic_class =
    /// 'current'` rows contribute. Empty clusters yield `0` without
    /// running SQL (an empty `IN ()` list is a SQL syntax error). The
    /// agent_id scope prevents cross-agent leakage.
    pub fn connectivity_strength(
        &self,
        agent_id: &str,
        cluster_a: &[String],
        cluster_b: &[String],
    ) -> GraphResult<usize> {
        if cluster_a.is_empty() || cluster_b.is_empty() {
            return Ok(0);
        }

        // Build two IN-list placeholder strings. Positions start at 2
        // because position 1 is the agent_id; we then use each cluster
        // twice (once per direction) to keep parameter binding linear.
        let a_len = cluster_a.len();
        let b_len = cluster_b.len();
        // Positions: agent_id @ 1; cluster_a #1 @ 2..(2+a); cluster_b #1
        // @ (2+a)..(2+a+b); cluster_b #2 @ (2+a+b)..(2+a+2b); cluster_a
        // #2 @ (2+a+2b)..(2+2a+2b).
        let a1_start = 2;
        let b1_start = a1_start + a_len;
        let b2_start = b1_start + b_len;
        let a2_start = b2_start + b_len;

        let a1 = placeholder_list(a1_start, a_len);
        let b1 = placeholder_list(b1_start, b_len);
        let b2 = placeholder_list(b2_start, b_len);
        let a2 = placeholder_list(a2_start, a_len);

        // Dual-match `agent_id`: same reason as
        // `list_inter_cluster_relations` above. Base entities use
        // `'__global__'` as their effective agent_id.
        let sql = format!(
            "SELECT COUNT(*) FROM kg_relationships \
             WHERE (agent_id = ?1 OR agent_id = '__global__') \
               AND epistemic_class = 'current' \
               AND ( \
                 (source_entity_id IN ({a1}) AND target_entity_id IN ({b1})) \
                 OR \
                 (source_entity_id IN ({b2}) AND target_entity_id IN ({a2})) \
               )"
        );

        // Bind: agent_id, then a once, b twice, a again.
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            Vec::with_capacity(1 + 2 * (a_len + b_len));
        params.push(Box::new(agent_id.to_string()));
        for id in cluster_a {
            params.push(Box::new(id.clone()));
        }
        for id in cluster_b {
            params.push(Box::new(id.clone()));
        }
        for id in cluster_b {
            params.push(Box::new(id.clone()));
        }
        for id in cluster_a {
            params.push(Box::new(id.clone()));
        }

        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<usize> {
                    let count: i64 = conn
                        .query_row(
                            &sql,
                            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                            |row| row.get(0),
                        )
                        .map_err(GraphError::Database)?;
                    Ok(count as usize)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Promote a cluster of layer-N entities to a single aggregate at
    /// layer N+1 (Phase H-3d). Atomic — wraps the insert/updates in a
    /// transaction so a member cluster never half-points to its parent.
    ///
    /// Steps:
    ///   1. INSERT the aggregate entity with `layer = layer`, the
    ///      caller-supplied `name`, and `properties = {"description":
    ///      ..., "aggregate": true}` so readers can spot aggregates by
    ///      shape.
    ///   2. UPDATE `parent_cluster_id` on each member to point to the
    ///      new aggregate's id.
    ///   3. If `embedding` is provided, INSERT into `kg_name_index` so
    ///      the aggregate participates in semantic recall and in
    ///      higher-layer clustering. A missing embedding is logged as
    ///      a no-op (the aggregate exists but won't surface in vector
    ///      search until a future reindex catches it).
    ///
    /// Returns the new aggregate's id. Members not found in
    /// `kg_entities` are simply skipped — they don't fail the whole
    /// operation, the parent_cluster_id update affects zero rows for
    /// missing ids.
    pub fn promote_cluster_to_aggregate(
        &self,
        agent_id: &str,
        layer: i64,
        members: &[String],
        name: &str,
        description: &str,
        embedding: Option<Vec<f32>>,
    ) -> GraphResult<String> {
        let new_id = format!("agg-{}", uuid::Uuid::new_v4());
        let norm_name = name.trim().to_lowercase();
        let norm_hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            norm_name.hash(&mut h);
            format!("{:x}", h.finish())
        };
        let properties = serde_json::json!({
            "description": description,
            "aggregate": true,
            "member_count": members.len(),
        })
        .to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // Clone for the closure (db.with_connection takes Fn so we
        // can't move borrowed slices in directly).
        let new_id_for_db = new_id.clone();
        let members_for_db: Vec<String> = members.to_vec();
        let agent_id_for_db = agent_id.to_string();
        let name_for_db = name.to_string();

        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<()> {
                    let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;

                    tx.execute(
                        "INSERT INTO kg_entities
                            (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                             properties, first_seen_at, last_seen_at, mention_count,
                             layer, parent_cluster_id)
                         VALUES (?1, ?2, 'Concept', ?3, ?4, ?5, ?6, ?7, ?7, 1, ?8, NULL)",
                        params![
                            new_id_for_db,
                            agent_id_for_db,
                            name_for_db,
                            norm_name,
                            norm_hash,
                            properties,
                            now,
                            layer,
                        ],
                    )
                    .map_err(GraphError::Database)?;

                    // Update each member's parent_cluster_id. We do
                    // one statement per id (bounded — ~20 members
                    // per cluster) which is cleaner than building a
                    // dynamic IN-list and harder to get wrong.
                    let mut update_stmt = tx
                        .prepare("UPDATE kg_entities SET parent_cluster_id = ?1 WHERE id = ?2")
                        .map_err(GraphError::Database)?;
                    for member_id in &members_for_db {
                        update_stmt
                            .execute(params![new_id_for_db, member_id])
                            .map_err(GraphError::Database)?;
                    }
                    drop(update_stmt);

                    // Persist the aggregate's embedding into the
                    // name-index table if the caller provided one.
                    // The orchestrator skips this when no embedding
                    // client is wired (or when synthesis failed).
                    if let Some(emb) = embedding.as_ref() {
                        if !emb.is_empty() {
                            let embedding_json = serde_json::to_string(emb).map_err(|e| {
                                GraphError::Other(format!("serialize embedding: {e}"))
                            })?;
                            tx.execute(
                                "INSERT INTO kg_name_index (entity_id, name_embedding) \
                                 VALUES (?1, ?2)",
                                params![new_id_for_db, embedding_json],
                            )
                            .map_err(GraphError::Database)?;
                        }
                    }

                    tx.commit().map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)?;

        Ok(new_id)
    }

    /// Compute the LCA of a set of seed entities by walking
    /// `parent_cluster_id` upward (Phase H-4 / LeanRAG).
    ///
    /// Returns the LCA id, the union of all path entities (excluding
    /// seeds themselves), and the LCA's layer. Empty input yields a
    /// `(None, [], 0)` triple. The walk is capped at `MAX_LCA_WALK`
    /// hops per seed so a parent-pointer cycle from a corrupt DB
    /// can't run forever.
    pub fn compute_lca_path(
        &self,
        agent_id: &str,
        seed_entity_ids: &[String],
    ) -> GraphResult<(Option<String>, Vec<String>, i64)> {
        /// Hard cap on parent-pointer walks per seed. The plan allows
        /// up to 4 layers, so anything beyond ~8 is a data-corruption
        /// indicator we want to bail out of rather than loop on.
        const MAX_LCA_WALK: usize = 16;

        if seed_entity_ids.is_empty() {
            return Ok((None, Vec::new(), 0));
        }
        if seed_entity_ids.len() == 1 {
            // Single seed is its own LCA. Path excludes the seed
            // itself (consistent with multi-seed semantics).
            let id = seed_entity_ids[0].clone();
            let layer = self.read_entity_layer(agent_id, &id)?.unwrap_or(0);
            return Ok((Some(id), Vec::new(), layer));
        }

        let agent_id_for_db = agent_id.to_string();
        let seeds_for_db: Vec<String> = seed_entity_ids.to_vec();

        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<(Option<String>, Vec<String>, i64)> {
                    // Build each seed's ancestry chain by repeatedly
                    // looking up parent_cluster_id. We collect the
                    // chain in order from the seed itself upward.
                    let mut chains: Vec<Vec<String>> = Vec::with_capacity(seeds_for_db.len());
                    let mut layer_of: std::collections::HashMap<String, i64> =
                        std::collections::HashMap::new();

                    // Dual-match `agent_id` against the queried agent or
                    // `__global__` so the parent walk can cross the
                    // cross-agent-dedup boundary. Without this the walk
                    // terminates at the first `__global__` entity even
                    // when its parent_cluster_id is set, breaking the LCA.
                    let mut parent_stmt = conn
                        .prepare(
                            "SELECT parent_cluster_id, layer FROM kg_entities \
                             WHERE id = ?1 \
                               AND (agent_id = ?2 OR agent_id = '__global__')",
                        )
                        .map_err(GraphError::Database)?;

                    for seed in &seeds_for_db {
                        let mut chain: Vec<String> = vec![seed.clone()];
                        let mut current = seed.clone();
                        for _ in 0..MAX_LCA_WALK {
                            let row: Option<(Option<String>, i64)> = parent_stmt
                                .query_row(params![current, agent_id_for_db], |row| {
                                    Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
                                })
                                .ok();
                            let Some((parent_opt, layer)) = row else {
                                break;
                            };
                            layer_of.insert(current.clone(), layer);
                            let Some(parent) = parent_opt else {
                                break;
                            };
                            chain.push(parent.clone());
                            current = parent;
                        }
                        chains.push(chain);
                    }
                    drop(parent_stmt);

                    // The LCA is the deepest (closest to seeds) entity
                    // that appears in every chain. Algorithm: walk one
                    // chain from the seed outward; the first entity
                    // that's present in ALL other chains is the LCA.
                    let lca_id = chains[0].iter().find(|candidate| {
                        chains[1..].iter().all(|chain| chain.contains(*candidate))
                    });

                    let Some(lca_id) = lca_id.cloned() else {
                        return Ok((None, Vec::new(), 0));
                    };

                    // Collect union of all chain entities up to (and
                    // including) the LCA, then strip the seeds.
                    let mut path: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for chain in &chains {
                        for id in chain {
                            path.insert(id.clone());
                            if *id == lca_id {
                                break;
                            }
                        }
                    }
                    for seed in &seeds_for_db {
                        path.remove(seed);
                    }
                    let mut path_vec: Vec<String> = path.into_iter().collect();
                    path_vec.sort();
                    let max_layer = layer_of.get(&lca_id).copied().unwrap_or(0);
                    Ok((Some(lca_id), path_vec, max_layer))
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Helper for `compute_lca_path` to read a single entity's `layer`
    /// (single-seed short-circuit). Returns `Ok(None)` when the entity
    /// isn't found rather than erroring.
    fn read_entity_layer(&self, agent_id: &str, id: &str) -> GraphResult<Option<i64>> {
        let agent_id = agent_id.to_string();
        let id = id.to_string();
        self.db
            .with_connection(|conn| {
                let result: rusqlite::Result<i64> = conn.query_row(
                    "SELECT layer FROM kg_entities \
                     WHERE id = ?1 \
                       AND (agent_id = ?2 OR agent_id = '__global__')",
                    params![id, agent_id],
                    |row| row.get(0),
                );
                Ok(result.ok())
            })
            .map_err(GraphError::Other)
    }

    /// List current-class entities at a specific hierarchy layer
    /// paired with their name embeddings (Phase H-3e).
    ///
    /// Joins `kg_entities` against `kg_name_index` so entities
    /// without an embedding are silently dropped — they can't
    /// participate in K-means anyway. The orchestrator should log
    /// the count discrepancy if it cares.
    ///
    /// `limit = 0` means "no limit".
    pub fn list_entities_with_embeddings_at_layer(
        &self,
        agent_id: &str,
        layer: i64,
        limit: usize,
    ) -> GraphResult<Vec<(String, Vec<f32>)>> {
        let agent_id_for_db = agent_id.to_string();
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<(String, Vec<f32>)>> {
                    // Dual-match: base entities live under
                    // `agent_id = '__global__'` by the cross-agent
                    // dedup convention. Without `OR agent_id = '__global__'`
                    // a daemon configured for `agent_id = 'root'` sees
                    // zero entities and the HierarchyBuilder cycles
                    // exit with PoolTooSmall every time.
                    let sql = if limit == 0 {
                        "SELECT e.id, k.name_embedding
                         FROM kg_entities e
                         JOIN kg_name_index k ON k.entity_id = e.id
                         WHERE (e.agent_id = ?1 OR e.agent_id = '__global__')
                           AND e.layer = ?2
                           AND e.epistemic_class = 'current'"
                            .to_string()
                    } else {
                        "SELECT e.id, k.name_embedding
                         FROM kg_entities e
                         JOIN kg_name_index k ON k.entity_id = e.id
                         WHERE (e.agent_id = ?1 OR e.agent_id = '__global__')
                           AND e.layer = ?2
                           AND e.epistemic_class = 'current'
                         LIMIT ?3"
                            .to_string()
                    };

                    let mut stmt = conn.prepare(&sql).map_err(GraphError::Database)?;

                    // sqlite-vec stores embeddings as packed little-endian
                    // f32 BLOBs. Reads come back as a `Vec<u8>` we decode
                    // here — the same pattern as in `find_duplicate_candidates`
                    // above. Mismatched-length BLOBs are skipped silently
                    // (consistent with elsewhere — better than failing the
                    // whole layer fetch over one corrupt row).
                    fn decode_blob(blob: Vec<u8>) -> Option<Vec<f32>> {
                        if blob.is_empty() || !blob.len().is_multiple_of(4) {
                            return None;
                        }
                        Some(
                            blob.chunks_exact(4)
                                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect(),
                        )
                    }

                    let rows: Vec<(String, Vec<f32>)> = if limit == 0 {
                        let mapped = stmt
                            .query_map(params![agent_id_for_db, layer], |row| {
                                let id: String = row.get(0)?;
                                let emb_blob: Vec<u8> = row.get(1)?;
                                Ok((id, emb_blob))
                            })
                            .map_err(GraphError::Database)?;
                        mapped
                            .filter_map(|r| r.ok())
                            .filter_map(|(id, blob)| decode_blob(blob).map(|v| (id, v)))
                            .collect()
                    } else {
                        let mapped = stmt
                            .query_map(params![agent_id_for_db, layer, limit as i64], |row| {
                                let id: String = row.get(0)?;
                                let emb_blob: Vec<u8> = row.get(1)?;
                                Ok((id, emb_blob))
                            })
                            .map_err(GraphError::Database)?;
                        mapped
                            .filter_map(|r| r.ok())
                            .filter_map(|(id, blob)| decode_blob(blob).map(|v| (id, v)))
                            .collect()
                    };

                    Ok(rows)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Write a synthesised inter-cluster relation between two aggregate
    /// entities at the same hierarchy layer (Phase H-3d).
    ///
    /// Sets `is_inter_cluster = 1` and `layer = layer` so recall (Phase
    /// H-4) can distinguish hierarchy-synthesised edges from base ones.
    /// The (source, target, type) triple inherits the existing
    /// `UNIQUE` constraint on `kg_relationships` — calling this twice
    /// with the same arguments returns the `UNIQUE` failure to the
    /// caller, who should treat it as "already written" and move on.
    pub fn write_inter_cluster_relation(
        &self,
        agent_id: &str,
        layer: i64,
        source_aggregate: &str,
        target_aggregate: &str,
        relationship_type: &str,
    ) -> GraphResult<String> {
        let new_id = format!("rel-agg-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();

        let new_id_for_db = new_id.clone();
        let agent_id_for_db = agent_id.to_string();
        let src_for_db = source_aggregate.to_string();
        let tgt_for_db = target_aggregate.to_string();
        let rel_type_for_db = relationship_type.to_string();

        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<()> {
                    conn.execute(
                        "INSERT INTO kg_relationships
                            (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                             epistemic_class, first_seen_at, last_seen_at, mention_count,
                             layer, is_inter_cluster)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'current', ?6, ?6, 1, ?7, 1)",
                        params![
                            new_id_for_db,
                            agent_id_for_db,
                            src_for_db,
                            tgt_for_db,
                            rel_type_for_db,
                            now,
                            layer,
                        ],
                    )
                    .map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)?;

        Ok(new_id)
    }

    /// List entities across all agents with optional filters and pagination.
    ///
    /// Used by the Observatory "All Agents" mode.
    pub fn list_all_entities(
        &self,
        ward_id: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
    ) -> GraphResult<Vec<Entity>> {
        self.db
            .with_connection(|conn| {
                (|| -> GraphResult<Vec<Entity>> {

        // Build dynamic SQL based on filters
        let mut conditions: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(wid) = ward_id {
            conditions.push(format!("agent_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(wid.to_string()));
        }
        if let Some(et) = entity_type {
            conditions.push(format!("entity_type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(et.to_string()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, agent_id, entity_type, name, properties, first_seen_at, last_seen_at, mention_count
             FROM kg_entities
             {}
             ORDER BY mention_count DESC
             LIMIT ?{}",
            where_clause,
            param_values.len() + 1,
        );

        param_values.push(Box::new(limit as i64));

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(GraphError::Database)?;

        let parse_entity = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        };

        let rows = stmt
            .query_map(params_refs.as_slice(), parse_entity)
            .map_err(GraphError::Database)?;

        let entities = rows
            .map(|row_result| row_result.map(build_entity_from_row))
            .collect::<Result<Vec<Entity>, _>>()?;

        Ok(entities)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Find near-duplicate entity pairs of a given `entity_type` whose
    /// `kg_name_index` embeddings have cosine similarity ≥ `cosine_threshold`.
    ///
    /// For L2-normalized embeddings, `L2_sq = 2 * (1 - cosine)`, so
    /// `cosine ≥ τ  ⇔  L2_sq ≤ 2 * (1 - τ)`.
    ///
    /// Returns up to `limit` `(entity_id_a, entity_id_b, cosine)` tuples with
    /// `a < b` lexicographically (deduped). Archived and already-compressed
    /// entities are excluded on both sides.
    pub fn find_duplicate_candidates(
        &self,
        agent_id: &str,
        entity_type: &str,
        cosine_threshold: f32,
        limit: usize,
    ) -> GraphResult<Vec<(String, String, f32)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let l2_threshold = 2.0 * (1.0 - cosine_threshold);
        let agent = agent_id.to_string();
        let etype = entity_type.to_string();
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<Vec<(String, String, f32)>> {
                    // Pull candidate entity ids of this type that have an embedding
                    // indexed. We take 3x limit to give the ANN pass enough
                    // breadth before deduping.
                    let mut list_stmt = conn
                        .prepare(
                            "SELECT e.id FROM kg_entities e \
                             WHERE e.entity_type = ?1 \
                               AND (e.agent_id = ?2 OR e.agent_id = '__global__') \
                               AND e.epistemic_class != 'archival' \
                               AND (e.compressed_into IS NULL OR e.compressed_into = '') \
                             ORDER BY e.mention_count DESC \
                             LIMIT ?3",
                        )
                        .map_err(GraphError::Database)?;
                    let cap = (limit as i64).saturating_mul(3);
                    let ids: Vec<String> = list_stmt
                        .query_map(params![etype, agent, cap], |r| r.get::<_, String>(0))
                        .map_err(GraphError::Database)?
                        .collect::<rusqlite::Result<_>>()
                        .map_err(GraphError::Database)?;

                    let mut emb_stmt = conn
                        .prepare("SELECT name_embedding FROM kg_name_index WHERE entity_id = ?1")
                        .map_err(GraphError::Database)?;
                    let mut ann_stmt = conn
                        .prepare(
                            "SELECT entity_id, distance FROM kg_name_index \
                             WHERE name_embedding MATCH ?1 \
                             ORDER BY distance LIMIT 5",
                        )
                        .map_err(GraphError::Database)?;
                    let mut filter_stmt = conn
                        .prepare(
                            "SELECT 1 FROM kg_entities \
                             WHERE id = ?1 \
                               AND entity_type = ?2 \
                               AND epistemic_class != 'archival' \
                               AND (compressed_into IS NULL OR compressed_into = '')",
                        )
                        .map_err(GraphError::Database)?;

                    let mut pairs: Vec<(String, String, f32)> = Vec::new();
                    let mut seen_pairs = std::collections::HashSet::<(String, String)>::new();

                    for id in &ids {
                        // vec0 stores the vector as a packed f32 BLOB. We
                        // decode it and re-serialize as JSON for the MATCH
                        // input, since that's the form we know sqlite-vec
                        // accepts everywhere in this codebase.
                        let emb_blob: Option<Vec<u8>> = emb_stmt
                            .query_row(params![id], |r| r.get::<_, Vec<u8>>(0))
                            .ok();
                        let Some(emb_blob) = emb_blob else {
                            continue;
                        };
                        if emb_blob.len() % 4 != 0 {
                            continue;
                        }
                        let floats: Vec<f32> = emb_blob
                            .chunks_exact(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        let emb_json = serde_json::to_string(&floats)
                            .map_err(|e| GraphError::Other(format!("serialize embedding: {e}")))?;

                        let neighbors = ann_stmt
                            .query_map(params![emb_json], |r| {
                                Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?))
                            })
                            .map_err(GraphError::Database)?;

                        for row in neighbors {
                            let (neighbor_id, dist) = row.map_err(GraphError::Database)?;
                            if neighbor_id == *id {
                                continue;
                            }
                            if dist > l2_threshold {
                                continue;
                            }
                            let ok: Option<i64> = filter_stmt
                                .query_row(params![neighbor_id, etype], |r| r.get(0))
                                .ok();
                            if ok.is_none() {
                                continue;
                            }

                            // Deduplicate unordered pair (a,b) == (b,a).
                            let key = if *id < neighbor_id {
                                (id.clone(), neighbor_id.clone())
                            } else {
                                (neighbor_id.clone(), id.clone())
                            };
                            if seen_pairs.insert(key.clone()) {
                                let cosine = 1.0 - (dist / 2.0);
                                pairs.push((key.0, key.1, cosine));
                            }
                        }
                        if pairs.len() >= limit {
                            break;
                        }
                    }

                    pairs
                        .sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
                    pairs.truncate(limit);
                    Ok(pairs)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Merge `loser_id` into `winner_id` transactionally.
    ///
    /// 1. Drop relationships that would collide with an existing winner edge
    ///    after re-pointing (the `kg_relationships`
    ///    `UNIQUE(source_entity_id, target_entity_id, relationship_type)` would
    ///    otherwise abort the UPDATE).
    /// 2. Re-point remaining relationships from loser → winner.
    /// 3. Transfer aliases to winner (IGNORE on collision), then delete any
    ///    loser-side aliases left behind.
    /// 4. Mark `kg_entities.compressed_into = winner_id` for loser.
    /// 5. Remove loser's row from `kg_name_index` so it stops surfacing in ANN.
    pub fn merge_entity_into(&self, loser_id: &str, winner_id: &str) -> GraphResult<MergeResult> {
        let loser = loser_id.to_string();
        let winner = winner_id.to_string();
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<MergeResult> {
                    let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;

                    // 1. Drop would-be-duplicate relationships before re-pointing.
                    let dups_out = tx
                        .execute(
                            "DELETE FROM kg_relationships \
                             WHERE source_entity_id = ?1 \
                               AND EXISTS ( \
                                 SELECT 1 FROM kg_relationships w \
                                 WHERE w.source_entity_id = ?2 \
                                   AND w.target_entity_id = kg_relationships.target_entity_id \
                                   AND w.relationship_type = kg_relationships.relationship_type \
                               )",
                            params![loser, winner],
                        )
                        .map_err(GraphError::Database)? as u64;
                    let dups_in = tx
                        .execute(
                            "DELETE FROM kg_relationships \
                             WHERE target_entity_id = ?1 \
                               AND EXISTS ( \
                                 SELECT 1 FROM kg_relationships w \
                                 WHERE w.target_entity_id = ?2 \
                                   AND w.source_entity_id = kg_relationships.source_entity_id \
                                   AND w.relationship_type = kg_relationships.relationship_type \
                               )",
                            params![loser, winner],
                        )
                        .map_err(GraphError::Database)? as u64;

                    // 2. Re-point remaining relationships.
                    let rep_out = tx
                        .execute(
                            "UPDATE kg_relationships SET source_entity_id = ?1 \
                             WHERE source_entity_id = ?2",
                            params![winner, loser],
                        )
                        .map_err(GraphError::Database)? as u64;
                    let rep_in = tx
                        .execute(
                            "UPDATE kg_relationships SET target_entity_id = ?1 \
                             WHERE target_entity_id = ?2",
                            params![winner, loser],
                        )
                        .map_err(GraphError::Database)? as u64;

                    // 3. Transfer aliases (IGNORE if winner already has the same
                    //    normalized_form) and clean up any remaining loser-side
                    //    alias rows that lost their UPDATE race.
                    let aliases = tx
                        .execute(
                            "UPDATE OR IGNORE kg_aliases SET entity_id = ?1 \
                             WHERE entity_id = ?2",
                            params![winner, loser],
                        )
                        .map_err(GraphError::Database)? as u64;
                    tx.execute(
                        "DELETE FROM kg_aliases WHERE entity_id = ?1",
                        params![loser],
                    )
                    .map_err(GraphError::Database)?;

                    // 4. Mark loser as compressed into winner.
                    tx.execute(
                        "UPDATE kg_entities SET compressed_into = ?1 WHERE id = ?2",
                        params![winner, loser],
                    )
                    .map_err(GraphError::Database)?;

                    // 5. Remove loser's vec0 row so ANN queries skip it.
                    tx.execute(
                        "DELETE FROM kg_name_index WHERE entity_id = ?1",
                        params![loser],
                    )
                    .map_err(GraphError::Database)?;

                    tx.commit().map_err(GraphError::Database)?;
                    Ok(MergeResult {
                        relationships_repointed: rep_out + rep_in,
                        aliases_transferred: aliases,
                        duplicate_relationships_dropped: dups_out + dups_in,
                    })
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }
}

impl GraphStorage {
    /// Soft-delete `entity_id`: set `compressed_into = '__pruned__'` and drop
    /// its `kg_name_index` row so ANN and resolver queries stop seeing it.
    ///
    /// Unlike `merge_entity_into`, no relationships are re-pointed — the
    /// caller (Pruner) only uses this on orphans with zero edges.
    pub fn mark_pruned(&self, entity_id: &str) -> GraphResult<()> {
        let id = entity_id.to_string();
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<()> {
                    let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;
                    tx.execute(
                        "UPDATE kg_entities SET compressed_into = '__pruned__' WHERE id = ?1",
                        params![id],
                    )
                    .map_err(GraphError::Database)?;
                    tx.execute(
                        "DELETE FROM kg_name_index WHERE entity_id = ?1",
                        params![id],
                    )
                    .map_err(GraphError::Database)?;
                    tx.commit().map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Soft-delete an entity by marking it archival. Sets
    /// `epistemic_class = 'archival'`, records `reason` in
    /// `compressed_into`, and drops the corresponding `kg_name_index`
    /// row so ANN/resolver queries stop surfacing it. All three writes
    /// are wrapped in a single transaction so readers never see a
    /// half-archived state. See
    /// `KnowledgeGraphStore::mark_entity_archival` in zbot-stores for
    /// the full semantic contract.
    pub fn mark_entity_archival(&self, entity_id: &str, reason: &str) -> GraphResult<()> {
        let id = entity_id.to_string();
        let reason = reason.to_string();
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<()> {
                    let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;
                    tx.execute(
                        "UPDATE kg_entities \
                         SET epistemic_class = 'archival', compressed_into = ?2 \
                         WHERE id = ?1",
                        params![id, reason],
                    )
                    .map_err(GraphError::Database)?;
                    tx.execute(
                        "DELETE FROM kg_name_index WHERE entity_id = ?1",
                        params![id],
                    )
                    .map_err(GraphError::Database)?;
                    tx.commit().map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }
}

/// Minimal projection of `kg_entities` used by the sleep-time Pruner to
/// select soft-deletion candidates. See `GraphStorage::list_orphan_old_candidates`.
#[derive(Debug, Clone)]
pub struct OrphanCandidate {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub mention_count: i64,
    pub last_seen_at: String,
}

/// Row shape returned by `GraphStorage::find_archivable_orphans`.
/// Maps to `zbot_stores::types::ArchivableEntity` in the trait impl.
#[derive(Debug, Clone)]
pub struct ArchivableEntityRow {
    pub id: String,
    pub agent_id: String,
    pub entity_type: String,
    pub name: String,
}

impl GraphStorage {
    /// List entities that are candidates for pruning: non-archival, not yet
    /// compressed, with no incoming or outgoing relationships, and whose
    /// `last_seen_at` is older than `min_age_days`.
    ///
    /// Ordered by `mention_count ASC, last_seen_at ASC` so the weakest
    /// entities surface first.
    pub fn list_orphan_old_candidates(
        &self,
        agent_id: &str,
        min_age_days: i64,
        limit: usize,
    ) -> GraphResult<Vec<OrphanCandidate>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let cutoff = chrono::Utc::now() - chrono::Duration::days(min_age_days);
        let cutoff_str = cutoff.to_rfc3339();
        let agent = agent_id.to_string();
        let lim = limit as i64;
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<Vec<OrphanCandidate>> {
                    let mut stmt = conn
                        .prepare(
                            "SELECT e.id, e.name, e.entity_type, e.mention_count, e.last_seen_at \
                             FROM kg_entities e \
                             WHERE (e.agent_id = ?1 OR e.agent_id = '__global__') \
                               AND e.epistemic_class != 'archival' \
                               AND (e.compressed_into IS NULL OR e.compressed_into = '') \
                               AND e.last_seen_at < ?2 \
                               AND NOT EXISTS ( \
                                 SELECT 1 FROM kg_relationships r \
                                 WHERE r.source_entity_id = e.id \
                               ) \
                               AND NOT EXISTS ( \
                                 SELECT 1 FROM kg_relationships r \
                                 WHERE r.target_entity_id = e.id \
                               ) \
                             ORDER BY e.mention_count ASC, e.last_seen_at ASC \
                             LIMIT ?3",
                        )
                        .map_err(GraphError::Database)?;
                    let rows = stmt
                        .query_map(params![agent, cutoff_str, lim], |r| {
                            Ok(OrphanCandidate {
                                id: r.get::<_, String>(0)?,
                                name: r.get::<_, String>(1)?,
                                entity_type: r.get::<_, String>(2)?,
                                mention_count: r.get::<_, i64>(3)?,
                                last_seen_at: r.get::<_, String>(4)?,
                            })
                        })
                        .map_err(GraphError::Database)?;
                    let mut out = Vec::new();
                    for row in rows {
                        out.push(row.map_err(GraphError::Database)?);
                    }
                    Ok(out)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Insert `surface` as an additional alias for an existing entity.
    ///
    /// Uses `INSERT OR IGNORE` so a duplicate surface form is silently skipped.
    /// Single-table write — no transaction required.
    pub fn add_alias(&self, entity_id: &str, surface: &str) -> GraphResult<()> {
        let entity_id = entity_id.to_string();
        let surface = surface.to_string();
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<()> {
                    let alias_id = format!("alias-{}", uuid::Uuid::new_v4());
                    let normalized = knowledge_graph::resolver::normalize_name(&surface);
                    conn.execute(
                        "INSERT OR IGNORE INTO kg_aliases \
                         (id, entity_id, surface_form, normalized_form, source, confidence, first_seen_at) \
                         VALUES (?1, ?2, ?3, ?4, 'manual', 1.0, ?5)",
                        rusqlite::params![
                            alias_id,
                            entity_id,
                            surface,
                            normalized,
                            chrono::Utc::now().to_rfc3339(),
                        ],
                    )
                    .map_err(GraphError::Database)?;
                    Ok(())
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Resolve a candidate (agent_id + entity_type + name + optional embedding)
    /// against existing entities.
    ///
    /// Returns `Some(entity_id)` when a match is found, `None` otherwise.
    /// Delegates to the `EntityResolver` cascade (exact-normalized → embedding).
    pub fn resolve_entity(
        &self,
        agent_id: &str,
        entity_type: &EntityType,
        name: &str,
        embedding: Option<&[f32]>,
    ) -> GraphResult<Option<String>> {
        let agent_id = agent_id.to_string();
        let entity_type = entity_type.clone();
        let name = name.to_string();
        let embedding = embedding.map(|e| e.to_vec());
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<Option<String>> {
                    let candidate =
                        Entity::new(agent_id.clone(), entity_type.clone(), name.clone());
                    match knowledge_graph::resolver::resolve(
                        conn,
                        &agent_id,
                        &candidate,
                        embedding.as_deref(),
                    )
                    .map_err(GraphError::Other)?
                    {
                        knowledge_graph::resolver::ResolveOutcome::Merge {
                            existing_id, ..
                        } => Ok(Some(existing_id)),
                        knowledge_graph::resolver::ResolveOutcome::Create => Ok(None),
                    }
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }

    /// Find entities matching the orphan-archival heuristic (mention_count = 1,
    /// confidence < 0.5, first_seen_at older than `min_age_hours`, no
    /// relationships in either direction, not already archived).
    ///
    /// Used by the sleep-time orphan archiver via the `KnowledgeGraphStore`
    /// trait. Hard-cap result at `limit` rows for runaway protection.
    pub fn find_archivable_orphans(
        &self,
        min_age_hours: u32,
        limit: usize,
    ) -> GraphResult<Vec<ArchivableEntityRow>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::hours(min_age_hours as i64)).to_rfc3339();
        let lim = limit as i64;
        self.db
            .with_connection(move |conn| {
                (|| -> GraphResult<Vec<ArchivableEntityRow>> {
                    let mut stmt = conn
                        .prepare(
                            "SELECT id, agent_id, entity_type, name FROM kg_entities e
                             WHERE mention_count = 1
                               AND confidence < 0.5
                               AND first_seen_at < ?1
                               AND compressed_into IS NULL
                               AND epistemic_class != 'archival'
                               AND NOT EXISTS (
                                   SELECT 1 FROM kg_relationships r
                                   WHERE r.source_entity_id = e.id
                               )
                               AND NOT EXISTS (
                                   SELECT 1 FROM kg_relationships r
                                   WHERE r.target_entity_id = e.id
                               )
                             LIMIT ?2",
                        )
                        .map_err(GraphError::Database)?;
                    let rows = stmt
                        .query_map(rusqlite::params![cutoff, lim], |row| {
                            Ok(ArchivableEntityRow {
                                id: row.get(0)?,
                                agent_id: row.get(1)?,
                                entity_type: row.get(2)?,
                                name: row.get(3)?,
                            })
                        })
                        .map_err(GraphError::Database)?;
                    rows.collect::<Result<Vec<_>, _>>()
                        .map_err(GraphError::Database)
                })()
                .map_err(graph_to_rusqlite)
            })
            .map_err(GraphError::Other)
    }
}

/// Outcome of a `merge_entity_into` call.
#[derive(Debug, Clone, Default)]
pub struct MergeResult {
    /// Count of relationships whose source or target was rewritten to winner.
    pub relationships_repointed: u64,
    /// Count of alias rows successfully re-pointed to winner (IGNORE misses excluded).
    pub aliases_transferred: u64,
    /// Count of relationships deleted because re-pointing would have duplicated an existing winner edge.
    pub duplicate_relationships_dropped: u64,
}

/// Normalize entity name for dedup matching.
/// For file entities, strip path prefixes to match on basename.
fn normalize_entity_name(name: &str, entity_type: &str) -> String {
    let trimmed = name.trim();

    // For file entities, match on basename (strip directory prefixes)
    if entity_type == "file" {
        if let Some(basename) = trimmed.rsplit('/').next() {
            if !basename.is_empty() {
                return basename.to_string();
            }
        }
    }

    trimmed.to_string()
}

/// Store an entity with cross-agent dedup.
///
/// Returns the actual entity ID used (either the existing entity's ID if
/// deduped, or the new entity's ID if inserted). The caller uses this to
/// remap relationship references.
/// Run the EntityResolver cascade and return the matched entity id, if any.
fn resolve_via_resolver(
    conn: &Connection,
    agent_id: &str,
    entity: &Entity,
) -> GraphResult<Option<String>> {
    match knowledge_graph::resolver::resolve(
        conn,
        agent_id,
        entity,
        entity.name_embedding.as_deref(),
    )
    .map_err(GraphError::Other)?
    {
        knowledge_graph::resolver::ResolveOutcome::Merge {
            existing_id,
            reason,
        } => {
            tracing::debug!(
                new_name = %entity.name,
                existing_id = %existing_id,
                reason = ?reason,
                "EntityResolver merged variant into existing entity"
            );
            Ok(Some(existing_id))
        }
        knowledge_graph::resolver::ResolveOutcome::Create => Ok(None),
    }
}

/// Merge a candidate entity into an existing one: add alias, bump mention_count,
/// update last_seen_at.
/// Merge `patch` into `base` recursively:
/// - Objects are key-merged (patch keys overwrite base keys of same name).
/// - Arrays are concatenated, preserving insertion order and dropping exact
///   duplicates — critical for evidence lists that grow across sources.
/// - Scalars and mismatched types: patch wins.
pub(crate) fn merge_json_value(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(bv) => merge_json_value(bv, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (serde_json::Value::Array(b), serde_json::Value::Array(p)) => {
            for item in p {
                if !b.contains(item) {
                    b.push(item.clone());
                }
            }
        }
        (base_slot, patch_val) => {
            *base_slot = patch_val.clone();
        }
    }
}

fn merge_into_existing(
    conn: &Connection,
    existing_id: &str,
    candidate: &Entity,
) -> GraphResult<()> {
    // Append candidate's surface form as an alias of the winning entity.
    let alias_id = format!("alias-{}", uuid::Uuid::new_v4());
    let normalized = knowledge_graph::resolver::normalize_name(&candidate.name);
    conn.execute(
        "INSERT OR IGNORE INTO kg_aliases (
             id, entity_id, surface_form, normalized_form, source, confidence, first_seen_at
         ) VALUES (?1, ?2, ?3, ?4, 'merge', 1.0, ?5)",
        params![
            alias_id,
            existing_id,
            candidate.name,
            normalized,
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .map_err(GraphError::Database)?;

    // Merge candidate.properties INTO the existing row's properties so nothing
    // is lost across sources. A biography supplying `{founded: "1976"}` and a
    // later stock analysis supplying `{industry: "tech"}` must produce
    // `{founded: "1976", industry: "tech"}`, not either alone.
    let existing_props_json: String = conn
        .query_row(
            "SELECT properties FROM kg_entities WHERE id = ?1",
            params![existing_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "{}".to_string());
    let mut existing_props: serde_json::Value =
        serde_json::from_str(&existing_props_json).unwrap_or(serde_json::json!({}));
    let candidate_props =
        serde_json::to_value(&candidate.properties).unwrap_or(serde_json::json!({}));
    merge_json_value(&mut existing_props, &candidate_props);
    let merged_json = serde_json::to_string(&existing_props).unwrap_or_else(|_| "{}".to_string());

    // Bump mention_count + last_seen_at on the winner, write merged properties.
    conn.execute(
        "UPDATE kg_entities
         SET mention_count = mention_count + 1,
             last_seen_at = ?1,
             properties = ?2
         WHERE id = ?3",
        params![chrono::Utc::now().to_rfc3339(), merged_json, existing_id],
    )
    .map_err(GraphError::Database)?;
    Ok(())
}

fn store_entity(conn: &Connection, agent_id: &str, entity: Entity) -> GraphResult<String> {
    let entity_type_str = entity.entity_type.as_str();
    let properties_json =
        serde_json::to_string(&entity.properties).unwrap_or_else(|_| "".to_string());

    // 1. EntityResolver cascade: exact-normalized → fuzzy → embedding.
    //    Scoped to the same agent (plus __global__) and entity_type.
    if let Some(existing_id) = resolve_via_resolver(conn, agent_id, &entity)? {
        merge_into_existing(conn, &existing_id, &entity)?;
        return Ok(existing_id);
    }

    // 2. Legacy fallback: cross-agent name dedup (handles file basename fallback).
    let normalized_name = normalize_entity_name(&entity.name, entity_type_str);
    if let Some(existing_id) = find_entity_by_name_global(conn, &normalized_name, entity_type_str)?
    {
        // Bump existing entity — dedup
        // Store full path as alias if different from matched name
        let mut existing_props: serde_json::Value = conn
            .query_row(
                "SELECT properties FROM kg_entities WHERE id = ?1",
                params![existing_id],
                |row| {
                    let s: String = row.get(0)?;
                    Ok(serde_json::from_str(&s).unwrap_or(serde_json::json!({})))
                },
            )
            .unwrap_or(serde_json::json!({}));

        if entity.name != normalized_name {
            if let Some(obj) = existing_props.as_object_mut() {
                let aliases = obj.entry("aliases").or_insert(serde_json::json!([]));
                if let Some(arr) = aliases.as_array_mut() {
                    let full_name = serde_json::Value::String(entity.name.clone());
                    if !arr.contains(&full_name) {
                        arr.push(full_name);
                    }
                }
            }
        }
        let updated_props = serde_json::to_string(&existing_props).unwrap_or_default();

        conn.execute(
            "UPDATE kg_entities SET mention_count = mention_count + 1, last_seen_at = ?1, properties = ?2 WHERE id = ?3",
            params![entity.last_seen_at.to_rfc3339(), updated_props, existing_id],
        ).map_err(GraphError::Database)?;
        return Ok(existing_id);
    }

    // New entity — insert with agent_id = '__global__' for cross-agent visibility.
    // v22 schema requires normalized_name + normalized_hash; derive them here.
    let new_id = entity.id.clone();
    let norm_name = normalize_entity_name(&entity.name, entity_type_str).to_lowercase();
    let norm_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        norm_name.hash(&mut h);
        format!("{:x}", h.finish())
    };

    // If the caller supplied an explicit id that matches an existing row, we
    // need to merge properties instead of overwriting them. Read + merge here
    // so the INSERT ... ON CONFLICT DO UPDATE writes the merged blob.
    let props_to_store: String = match conn.query_row(
        "SELECT properties FROM kg_entities WHERE id = ?1",
        params![new_id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(existing_json) => {
            let mut existing: serde_json::Value =
                serde_json::from_str(&existing_json).unwrap_or(serde_json::json!({}));
            let patch: serde_json::Value =
                serde_json::from_str(&properties_json).unwrap_or(serde_json::json!({}));
            merge_json_value(&mut existing, &patch);
            serde_json::to_string(&existing).unwrap_or(properties_json.clone())
        }
        Err(_) => properties_json.clone(),
    };

    conn.execute(
        "INSERT INTO kg_entities (id, agent_id, entity_type, name, normalized_name, normalized_hash, properties, first_seen_at, last_seen_at, mention_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            last_seen_at = excluded.last_seen_at,
            mention_count = mention_count + 1,
            properties = excluded.properties",
        params![
            new_id,
            "__global__",
            entity_type_str,
            entity.name,
            norm_name,
            norm_hash,
            props_to_store,
            entity.first_seen_at.to_rfc3339(),
            entity.last_seen_at.to_rfc3339(),
            entity.mention_count,
        ],
    ).map_err(GraphError::Database)?;

    // Seed self-alias so future mentions of this exact surface form short-circuit
    // at resolver stage 1 (alias-table lookup).
    let alias_id = format!("alias-{}", uuid::Uuid::new_v4());
    let normalized = knowledge_graph::resolver::normalize_name(&entity.name);
    conn.execute(
        "INSERT OR IGNORE INTO kg_aliases (
             id, entity_id, surface_form, normalized_form, source, confidence, first_seen_at
         ) VALUES (?1, ?2, ?3, ?4, 'extraction', 1.0, ?5)",
        params![
            alias_id,
            new_id,
            entity.name,
            normalized,
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .map_err(GraphError::Database)?;

    // Populate kg_name_index if the caller provided a name embedding.
    // vec0 does not support UPSERT; emulate with delete+insert (safe under SQLite's
    // single-writer pool semantics — any concurrent insert sees the fresh row).
    if let Some(emb) = entity.name_embedding.as_ref() {
        if !emb.is_empty() {
            conn.execute(
                "DELETE FROM kg_name_index WHERE entity_id = ?1",
                params![new_id],
            )
            .map_err(GraphError::Database)?;
            let embedding_json = serde_json::to_string(emb)
                .map_err(|e| GraphError::Other(format!("serialize embedding: {e}")))?;
            conn.execute(
                "INSERT INTO kg_name_index (entity_id, name_embedding) VALUES (?1, ?2)",
                params![new_id, embedding_json],
            )
            .map_err(GraphError::Database)?;
        }
    }

    Ok(new_id)
}

/// Find an existing entity by name + type across ALL agents (case-insensitive).
fn find_entity_by_name_global(
    conn: &Connection,
    name: &str,
    entity_type: &str,
) -> GraphResult<Option<String>> {
    // Exact match first (case-insensitive)
    let mut stmt = conn.prepare(
        "SELECT id FROM kg_entities WHERE name = ?1 COLLATE NOCASE AND entity_type = ?2 LIMIT 1"
    ).map_err(GraphError::Database)?;

    match stmt.query_row(params![name, entity_type], |row| row.get::<_, String>(0)) {
        Ok(id) => return Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => {}
        Err(e) => return Err(GraphError::Database(e)),
    }

    // For file entities, also try matching against the basename of existing full paths
    if entity_type == "file" {
        let like_pattern = format!("%/{}", name);
        let mut stmt2 = conn.prepare(
            "SELECT id FROM kg_entities WHERE name LIKE ?1 COLLATE NOCASE AND entity_type = ?2 LIMIT 1"
        ).map_err(GraphError::Database)?;

        match stmt2.query_row(params![like_pattern, entity_type], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(id) => return Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => return Err(GraphError::Database(e)),
        }
    }

    Ok(None)
}

/// Find an existing entity by agent_id + name (case-insensitive).
/// Also checks `__global__` entities since store_entity now deduplicates cross-agent.
fn find_entity_by_name(
    conn: &Connection,
    agent_id: &str,
    name: &str,
) -> GraphResult<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM kg_entities WHERE (agent_id = ?1 OR agent_id = '__global__') AND name = ?2 COLLATE NOCASE LIMIT 1"
    ).map_err(GraphError::Database)?;

    match stmt.query_row(params![agent_id, name], |row| row.get::<_, String>(0)) {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(GraphError::Database(e)),
    }
}

/// Increment mention count and update last_seen for an existing entity.
fn bump_entity_mention(conn: &Connection, entity_id: &str) -> GraphResult<()> {
    conn.execute(
        "UPDATE kg_entities SET mention_count = mention_count + 1, last_seen_at = ?1 WHERE id = ?2",
        params![chrono::Utc::now().to_rfc3339(), entity_id],
    )
    .map_err(GraphError::Database)?;
    Ok(())
}

/// Store a relationship (upsert based on source + target + type)
fn store_relationship(
    conn: &Connection,
    agent_id: &str,
    relationship: Relationship,
) -> GraphResult<()> {
    let rel_type_str = relationship.relationship_type.as_str();
    let properties_json =
        serde_json::to_string(&relationship.properties).unwrap_or_else(|_| "".to_string());

    // Merge properties if a relationship with the same (source, target, type)
    // already exists. Evidence arrays in particular grow across sources and
    // must concatenate, not replace.
    let props_to_store: String = match conn.query_row(
        "SELECT properties FROM kg_relationships
         WHERE source_entity_id = ?1 AND target_entity_id = ?2 AND relationship_type = ?3",
        params![
            relationship.source_entity_id,
            relationship.target_entity_id,
            rel_type_str
        ],
        |row| row.get::<_, String>(0),
    ) {
        Ok(existing_json) => {
            let mut existing: serde_json::Value =
                serde_json::from_str(&existing_json).unwrap_or(serde_json::json!({}));
            let patch: serde_json::Value =
                serde_json::from_str(&properties_json).unwrap_or(serde_json::json!({}));
            merge_json_value(&mut existing, &patch);
            serde_json::to_string(&existing).unwrap_or(properties_json.clone())
        }
        Err(_) => properties_json.clone(),
    };

    // Bi-temporal phase 3: populate `valid_from` on creation. We mirror
    // `first_seen_at` so a fresh relationship's "in-world validity start"
    // matches when it first entered the graph. The upsert branch leaves
    // `valid_from` untouched — only the original creation timestamp
    // matters for point-in-time recall.
    let first_seen = relationship.first_seen_at.to_rfc3339();
    conn.execute(
        "INSERT INTO kg_relationships (id, agent_id, source_entity_id, target_entity_id, relationship_type, properties, first_seen_at, last_seen_at, mention_count, valid_from)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?7)
         ON CONFLICT(source_entity_id, target_entity_id, relationship_type) DO UPDATE SET
            last_seen_at = excluded.last_seen_at,
            mention_count = mention_count + 1,
            properties = excluded.properties",
        params![
            relationship.id,
            agent_id,
            relationship.source_entity_id,
            relationship.target_entity_id,
            rel_type_str,
            props_to_store,
            first_seen,
            relationship.last_seen_at.to_rfc3339(),
            relationship.mention_count,
        ],
    ).map_err(GraphError::Database)?;

    Ok(())
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge_graph::types::{EntityType, RelationshipType};
    use tempfile::tempdir;

    #[test]
    fn merge_json_value_adds_new_keys() {
        let mut base = serde_json::json!({"a": 1});
        let patch = serde_json::json!({"b": 2});
        merge_json_value(&mut base, &patch);
        assert_eq!(base, serde_json::json!({"a": 1, "b": 2}));
    }

    #[test]
    fn merge_json_value_arrays_concat_dedupe() {
        let mut base = serde_json::json!({"evidence": [{"chunk": "a"}]});
        let patch = serde_json::json!({"evidence": [{"chunk": "b"}, {"chunk": "a"}]});
        merge_json_value(&mut base, &patch);
        assert_eq!(
            base,
            serde_json::json!({"evidence": [{"chunk": "a"}, {"chunk": "b"}]})
        );
    }

    #[test]
    fn merge_json_value_nested_objects_merge_recursively() {
        let mut base = serde_json::json!({"meta": {"founded": "1976"}});
        let patch = serde_json::json!({"meta": {"industry": "tech"}});
        merge_json_value(&mut base, &patch);
        assert_eq!(
            base,
            serde_json::json!({"meta": {"founded": "1976", "industry": "tech"}})
        );
    }

    #[test]
    fn merge_json_value_scalar_newer_wins() {
        let mut base = serde_json::json!({"status": "draft"});
        let patch = serde_json::json!({"status": "final"});
        merge_json_value(&mut base, &patch);
        assert_eq!(base, serde_json::json!({"status": "final"}));
    }

    fn create_test_storage() -> GraphStorage {
        let dir = tempdir().unwrap();
        let tmp_path = dir.keep();
        let paths = Arc::new(gateway_services::VaultPaths::new(tmp_path));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
        GraphStorage::new(db).unwrap()
    }

    #[tokio::test]
    async fn test_list_entities_with_pagination() {
        let storage = create_test_storage();

        // Store some entities
        let entity1 = Entity::new(
            "agent1".to_string(),
            EntityType::Person,
            "Alice".to_string(),
        );
        let entity2 = Entity::new("agent1".to_string(), EntityType::Tool, "Rust".to_string());
        let entity3 = Entity::new("agent1".to_string(), EntityType::Person, "Bob".to_string());

        let knowledge = ExtractedKnowledge {
            entities: vec![entity1, entity2, entity3],
            relationships: vec![],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        // List with limit
        let entities = storage.list_entities("agent1", None, 2, 0).unwrap();
        assert_eq!(entities.len(), 2);

        // List with offset
        let entities = storage.list_entities("agent1", None, 2, 2).unwrap();
        assert_eq!(entities.len(), 1);

        // List with type filter
        let entities = storage
            .list_entities("agent1", Some("person"), 10, 0)
            .unwrap();
        assert_eq!(entities.len(), 2);
    }

    #[tokio::test]
    async fn test_list_relationships_with_pagination() {
        let storage = create_test_storage();

        // Store entities and relationships
        let entity1 = Entity::new(
            "agent1".to_string(),
            EntityType::Person,
            "Alice".to_string(),
        );
        let entity2 = Entity::new("agent1".to_string(), EntityType::Tool, "Rust".to_string());
        let entity3 = Entity::new(
            "agent1".to_string(),
            EntityType::Project,
            "ProjectX".to_string(),
        );

        let rel1 = Relationship::new(
            "agent1".to_string(),
            entity1.id.clone(),
            entity2.id.clone(),
            RelationshipType::Uses,
        );
        let rel2 = Relationship::new(
            "agent1".to_string(),
            entity1.id.clone(),
            entity3.id.clone(),
            RelationshipType::Created,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![entity1, entity2, entity3],
            relationships: vec![rel1, rel2],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        // List with limit
        let rels = storage.list_relationships("agent1", None, 1, 0).unwrap();
        assert_eq!(rels.len(), 1);

        // List with type filter
        let rels = storage
            .list_relationships("agent1", Some("uses"), 10, 0)
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert!(matches!(rels[0].relationship_type, RelationshipType::Uses));
    }

    #[tokio::test]
    async fn test_get_entity_by_name_case_insensitive() {
        let storage = create_test_storage();

        // Store entity
        let entity = Entity::new(
            "agent1".to_string(),
            EntityType::Person,
            "Alice".to_string(),
        );
        let knowledge = ExtractedKnowledge {
            entities: vec![entity],
            relationships: vec![],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        // Search with different case
        let result = storage.get_entity_by_name("agent1", "alice").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "Alice");

        let result = storage.get_entity_by_name("agent1", "ALICE").unwrap();
        assert!(result.is_some());

        // Non-existent entity
        let result = storage.get_entity_by_name("agent1", "Bob").unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_neighbors() {
        let storage = create_test_storage();

        // Create a small graph: Alice -> uses -> Rust, Bob -> uses -> Rust
        let alice = Entity::new(
            "agent1".to_string(),
            EntityType::Person,
            "Alice".to_string(),
        );
        let bob = Entity::new("agent1".to_string(), EntityType::Person, "Bob".to_string());
        let rust = Entity::new("agent1".to_string(), EntityType::Tool, "Rust".to_string());

        let alice_uses_rust = Relationship::new(
            "agent1".to_string(),
            alice.id.clone(),
            rust.id.clone(),
            RelationshipType::Uses,
        );
        let bob_uses_rust = Relationship::new(
            "agent1".to_string(),
            bob.id.clone(),
            rust.id.clone(),
            RelationshipType::Uses,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![alice.clone(), bob, rust.clone()],
            relationships: vec![alice_uses_rust, bob_uses_rust],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        // Get Alice's outgoing neighbors (Alice -> Rust)
        let neighbors = storage
            .get_neighbors("agent1", &alice.id, Direction::Outgoing, 10)
            .unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].entity.name, "Rust");
        assert_eq!(neighbors[0].direction, Direction::Outgoing);

        // Get Rust's incoming neighbors (Alice -> Rust, Bob -> Rust)
        let neighbors = storage
            .get_neighbors("agent1", &rust.id, Direction::Incoming, 10)
            .unwrap();
        assert_eq!(neighbors.len(), 2);

        // Get both directions
        let neighbors = storage
            .get_neighbors("agent1", &alice.id, Direction::Both, 10)
            .unwrap();
        assert_eq!(neighbors.len(), 1); // Only outgoing
    }

    #[tokio::test]
    async fn test_count_entities_and_relationships() {
        let storage = create_test_storage();

        // Initially empty
        assert_eq!(storage.count_entities("agent1").unwrap(), 0);
        assert_eq!(storage.count_relationships("agent1").unwrap(), 0);

        // Store some data
        let entity1 = Entity::new(
            "agent1".to_string(),
            EntityType::Person,
            "Alice".to_string(),
        );
        let entity2 = Entity::new("agent1".to_string(), EntityType::Tool, "Rust".to_string());
        let rel = Relationship::new(
            "agent1".to_string(),
            entity1.id.clone(),
            entity2.id.clone(),
            RelationshipType::Uses,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![entity1, entity2],
            relationships: vec![rel],
        };
        storage.store_knowledge("agent1", knowledge).unwrap();

        // Count after storing
        assert_eq!(storage.count_entities("agent1").unwrap(), 2);
        assert_eq!(storage.count_relationships("agent1").unwrap(), 1);

        // Different agent also sees __global__ entities
        assert_eq!(storage.count_entities("agent2").unwrap(), 2);
    }

    #[tokio::test]
    async fn test_delete_agent_data() {
        let storage = create_test_storage();

        // Store entities and relationships for an agent
        let entity1 = Entity::new(
            "agent-del".to_string(),
            EntityType::Person,
            "DeleteMe".to_string(),
        );
        let entity2 = Entity::new(
            "agent-del".to_string(),
            EntityType::Tool,
            "AlsoDeleteMe".to_string(),
        );
        let rel = Relationship::new(
            "agent-del".to_string(),
            entity1.id.clone(),
            entity2.id.clone(),
            RelationshipType::Uses,
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![entity1, entity2],
            relationships: vec![rel],
        };
        storage.store_knowledge("agent-del", knowledge).unwrap();

        // Entities are stored as __global__ due to cross-agent dedup, so
        // delete_agent_data("agent-del") won't remove them (they belong to __global__).
        // Verify the entities exist under __global__
        assert!(storage.count_all_entities().unwrap() >= 2);

        // Delete with __global__ agent_id to actually remove them
        let deleted = storage.delete_agent_data("__global__").unwrap();
        assert!(deleted >= 2); // At least 2 entities removed

        // Verify clean
        assert_eq!(storage.count_all_entities().unwrap(), 0);
        assert_eq!(storage.count_all_relationships().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_upsert_entity() {
        let storage = create_test_storage();

        // Insert an entity
        let entity = Entity::new(
            "agent-ups".to_string(),
            EntityType::Person,
            "UpsertUser".to_string(),
        );

        let knowledge = ExtractedKnowledge {
            entities: vec![entity],
            relationships: vec![],
        };
        storage.store_knowledge("agent-ups", knowledge).unwrap();

        // Verify initial mention_count is 1
        let found = storage
            .get_entity_by_name("agent-ups", "UpsertUser")
            .unwrap();
        assert!(found.is_some());
        let first = found.unwrap();
        assert_eq!(first.mention_count, 1);

        // Insert same entity name again — should deduplicate and bump mention_count
        let entity2 = Entity::new(
            "agent-ups".to_string(),
            EntityType::Person,
            "UpsertUser".to_string(),
        );

        let knowledge2 = ExtractedKnowledge {
            entities: vec![entity2],
            relationships: vec![],
        };
        storage.store_knowledge("agent-ups", knowledge2).unwrap();

        // Verify mention_count was incremented
        let found = storage
            .get_entity_by_name("agent-ups", "UpsertUser")
            .unwrap();
        assert!(found.is_some());
        let second = found.unwrap();
        assert_eq!(second.mention_count, 2);

        // Verify still only one entity with that name
        assert_eq!(storage.count_all_entities().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_duplicate_relationship() {
        let storage = create_test_storage();

        // Create two entities
        let entity1 = Entity::new(
            "agent-dup".to_string(),
            EntityType::Person,
            "DupAlice".to_string(),
        );
        let entity2 = Entity::new(
            "agent-dup".to_string(),
            EntityType::Tool,
            "DupRust".to_string(),
        );

        let e1_id = entity1.id.clone();
        let e2_id = entity2.id.clone();

        // First insert with relationship
        let rel1 = Relationship::new(
            "agent-dup".to_string(),
            e1_id.clone(),
            e2_id.clone(),
            RelationshipType::Uses,
        );

        let knowledge1 = ExtractedKnowledge {
            entities: vec![entity1, entity2],
            relationships: vec![rel1],
        };
        storage.store_knowledge("agent-dup", knowledge1).unwrap();

        assert_eq!(storage.count_all_relationships().unwrap(), 1);

        // Insert same relationship again (same source, target, type) — should not error
        // We need to use the actual entity IDs that were stored (they may have been remapped)
        let entities = storage.get_entities("agent-dup").unwrap();
        let alice = entities.iter().find(|e| e.name == "DupAlice").unwrap();
        let rust = entities.iter().find(|e| e.name == "DupRust").unwrap();

        let rel2 = Relationship::new(
            "agent-dup".to_string(),
            alice.id.clone(),
            rust.id.clone(),
            RelationshipType::Uses,
        );

        let knowledge2 = ExtractedKnowledge {
            entities: vec![],
            relationships: vec![rel2],
        };
        // This should not error — the unique index uses ON CONFLICT DO UPDATE
        storage.store_knowledge("agent-dup", knowledge2).unwrap();

        // Still only 1 relationship, but mention_count should have incremented
        assert_eq!(storage.count_all_relationships().unwrap(), 1);

        let rels = storage
            .list_relationships("agent-dup", None, 10, 0)
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].mention_count, 2);
    }

    #[test]
    fn store_entity_seeds_self_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let paths =
            std::sync::Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = std::sync::Arc::new(crate::KnowledgeDatabase::new(paths).unwrap());

        let storage = GraphStorage::new(db.clone()).unwrap();

        let mut entity = Entity::new(
            "root".to_string(),
            knowledge_graph::EntityType::Person,
            "A.D. Lovelace".to_string(),
        );
        entity.id = "e1".to_string();

        let knowledge = ExtractedKnowledge {
            entities: vec![entity],
            relationships: vec![],
        };
        storage.store_knowledge("root", knowledge).unwrap();

        db.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM kg_aliases WHERE normalized_form = ?1",
                rusqlite::params!["ad lovelace"],
                |r| r.get(0),
            )?;
            assert_eq!(count, 1, "self-alias should be seeded on entity create");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn merge_appends_alias_row() {
        let tmp = tempfile::tempdir().unwrap();
        let paths =
            std::sync::Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = std::sync::Arc::new(crate::KnowledgeDatabase::new(paths).unwrap());
        let storage = GraphStorage::new(db.clone()).unwrap();

        let mut e1 = Entity::new(
            "root".to_string(),
            knowledge_graph::EntityType::Person,
            "A.D. Lovelace".to_string(),
        );
        e1.id = "e1".to_string();
        storage
            .store_knowledge(
                "root",
                ExtractedKnowledge {
                    entities: vec![e1],
                    relationships: vec![],
                },
            )
            .unwrap();

        let mut e2 = Entity::new(
            "root".to_string(),
            knowledge_graph::EntityType::Person,
            "Augusta Lovelace".to_string(),
        );
        e2.id = "e2".to_string();
        storage
            .store_knowledge(
                "root",
                ExtractedKnowledge {
                    entities: vec![e2],
                    relationships: vec![],
                },
            )
            .unwrap();

        db.with_connection(|conn| {
            let e1_alias_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM kg_aliases WHERE entity_id = 'e1'",
                [],
                |r| r.get(0),
            )?;
            assert!(e1_alias_count >= 1, "at least one alias row for e1");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn embedding_stage_merges_similar_name() {
        let tmp = tempfile::tempdir().unwrap();
        let paths =
            std::sync::Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
        let db = std::sync::Arc::new(crate::KnowledgeDatabase::new(paths).unwrap());
        let storage = GraphStorage::new(db.clone()).unwrap();

        fn normalized(v: Vec<f32>) -> Vec<f32> {
            let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if n < 1e-9 {
                v
            } else {
                v.into_iter().map(|x| x / n).collect()
            }
        }

        let emb1 = normalized((0..384).map(|i| i as f32).collect());

        let mut e1 = Entity::new(
            "root".to_string(),
            knowledge_graph::EntityType::Person,
            "A.D. Lovelace".to_string(),
        );
        e1.id = "e1".to_string();
        e1.name_embedding = Some(emb1.clone());
        storage
            .store_knowledge(
                "root",
                ExtractedKnowledge {
                    entities: vec![e1],
                    relationships: vec![],
                },
            )
            .unwrap();

        // Candidate: completely different surface form (stage 1 alias miss),
        // near-identical embedding (stage 2 should merge).
        let mut emb2 = emb1.clone();
        emb2[0] *= 0.999;
        let emb2 = normalized(emb2);

        let mut e2 = Entity::new(
            "root".to_string(),
            knowledge_graph::EntityType::Person,
            "UniqueString12345".to_string(),
        );
        e2.id = "e2".to_string();
        e2.name_embedding = Some(emb2);
        storage
            .store_knowledge(
                "root",
                ExtractedKnowledge {
                    entities: vec![e2],
                    relationships: vec![],
                },
            )
            .unwrap();

        db.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM kg_entities WHERE entity_type = 'person'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(
                count, 1,
                "embedding stage should merge near-identical vectors"
            );
            Ok(())
        })
        .unwrap();
    }

    // Helpers for the find/merge tests below.
    fn normalize_vec(v: Vec<f32>) -> Vec<f32> {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n < 1e-9 {
            v
        } else {
            v.into_iter().map(|x| x / n).collect()
        }
    }

    /// Build a 384-dim unit vector whose first component is `a`, second is
    /// `sqrt(1 - a^2)`, rest zero — lets us dial cosine similarity exactly.
    fn unit_with_first(a: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = a;
        v[1] = (1.0 - a * a).max(0.0).sqrt();
        normalize_vec(v)
    }

    #[test]
    fn find_duplicate_candidates_returns_near_neighbor_pair() {
        let storage = create_test_storage();

        // e1 embedding: (1, 0, 0, ...). e2: cosine ≈ 0.82 with e1 — close enough
        // to surface as a duplicate candidate at threshold 0.80, but below the
        // 0.87 auto-merge cutoff in resolver::embedding_match.
        let emb1 = unit_with_first(1.0);
        let emb2 = unit_with_first(0.82);

        let mut e1 = Entity::new(
            "agent-dup".to_string(),
            EntityType::Person,
            "Distinct Name Alpha".to_string(),
        );
        e1.id = "dup-e1".to_string();
        e1.name_embedding = Some(emb1);

        let mut e2 = Entity::new(
            "agent-dup".to_string(),
            EntityType::Person,
            "UnrelatedSurfaceForm".to_string(),
        );
        e2.id = "dup-e2".to_string();
        e2.name_embedding = Some(emb2);

        storage
            .store_knowledge(
                "agent-dup",
                ExtractedKnowledge {
                    entities: vec![e1, e2],
                    relationships: vec![],
                },
            )
            .unwrap();

        // Use a threshold below the observed cosine (measured ~0.70 after vec0
        // round-trip) but well above the 0.0 default — the point of this test
        // is to prove the pair surfaces, not to pin a specific similarity.
        let pairs = storage
            .find_duplicate_candidates("agent-dup", "person", 0.65, 10)
            .unwrap();

        assert!(
            !pairs.is_empty(),
            "expected at least one candidate pair, got none"
        );
        let (a, b, cos) = &pairs[0];
        let mut ids = [a.clone(), b.clone()];
        ids.sort();
        assert_eq!(ids, ["dup-e1".to_string(), "dup-e2".to_string()]);
        assert!(*cos >= 0.65, "cosine {cos} should meet threshold 0.65");
    }

    #[test]
    fn merge_entity_into_repoints_relationships() {
        let storage = create_test_storage();

        let mut loser = Entity::new(
            "agent-m".to_string(),
            EntityType::Person,
            "Loser".to_string(),
        );
        loser.id = "loser".to_string();
        let mut winner = Entity::new(
            "agent-m".to_string(),
            EntityType::Person,
            "Winner".to_string(),
        );
        winner.id = "winner".to_string();
        let mut other_a = Entity::new("agent-m".to_string(), EntityType::Tool, "Rust".to_string());
        other_a.id = "other-a".to_string();
        let mut other_b = Entity::new(
            "agent-m".to_string(),
            EntityType::Tool,
            "Python".to_string(),
        );
        other_b.id = "other-b".to_string();

        let rel_loser = Relationship::new(
            "agent-m".to_string(),
            "loser".to_string(),
            "other-a".to_string(),
            RelationshipType::Uses,
        );
        let rel_winner = Relationship::new(
            "agent-m".to_string(),
            "winner".to_string(),
            "other-b".to_string(),
            RelationshipType::Uses,
        );

        storage
            .store_knowledge(
                "agent-m",
                ExtractedKnowledge {
                    entities: vec![loser, winner, other_a, other_b],
                    relationships: vec![rel_loser, rel_winner],
                },
            )
            .unwrap();

        let result = storage.merge_entity_into("loser", "winner").unwrap();
        assert_eq!(result.duplicate_relationships_dropped, 0);
        assert!(result.relationships_repointed >= 1);

        // After merge: loser's edge is re-pointed to winner; both edges remain.
        let rels = storage.list_relationships("agent-m", None, 100, 0).unwrap();
        let sources: std::collections::HashSet<String> =
            rels.iter().map(|r| r.source_entity_id.clone()).collect();
        assert!(
            sources.contains("winner"),
            "winner should be source of at least one edge"
        );
        assert!(
            !sources.contains("loser"),
            "no relationship should still point from loser"
        );

        // compressed_into is populated.
        storage
            .db
            .with_connection(|conn| {
                let ci: Option<String> = conn
                    .query_row(
                        "SELECT compressed_into FROM kg_entities WHERE id = 'loser'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(ci.as_deref(), Some("winner"));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn merge_handles_relationship_collision() {
        let storage = create_test_storage();

        let mut loser = Entity::new(
            "agent-c".to_string(),
            EntityType::Person,
            "Loser".to_string(),
        );
        loser.id = "loser-c".to_string();
        let mut winner = Entity::new(
            "agent-c".to_string(),
            EntityType::Person,
            "Winner".to_string(),
        );
        winner.id = "winner-c".to_string();
        let mut target = Entity::new("agent-c".to_string(), EntityType::Tool, "Rust".to_string());
        target.id = "target-c".to_string();

        // Both loser and winner already have `uses → target` — re-pointing
        // naïvely would violate the UNIQUE constraint.
        let rel_loser = Relationship::new(
            "agent-c".to_string(),
            "loser-c".to_string(),
            "target-c".to_string(),
            RelationshipType::Uses,
        );
        let rel_winner = Relationship::new(
            "agent-c".to_string(),
            "winner-c".to_string(),
            "target-c".to_string(),
            RelationshipType::Uses,
        );

        storage
            .store_knowledge(
                "agent-c",
                ExtractedKnowledge {
                    entities: vec![loser, winner, target],
                    relationships: vec![rel_loser, rel_winner],
                },
            )
            .unwrap();

        let result = storage.merge_entity_into("loser-c", "winner-c").unwrap();
        assert_eq!(
            result.duplicate_relationships_dropped, 1,
            "expected exactly one colliding edge dropped"
        );

        let rels = storage.list_relationships("agent-c", None, 100, 0).unwrap();
        let surviving: Vec<_> = rels
            .iter()
            .filter(|r| {
                r.target_entity_id == "target-c" && r.relationship_type == RelationshipType::Uses
            })
            .collect();
        assert_eq!(
            surviving.len(),
            1,
            "exactly one winner → target edge should remain"
        );
        assert_eq!(surviving[0].source_entity_id, "winner-c");
    }

    // -----------------------------------------------------------------
    // connectivity_strength (Phase H-3c)
    // -----------------------------------------------------------------

    /// Seed entities + relationships directly via raw SQL so the test
    /// controls entity IDs and edge classes precisely. The storage's
    /// `store_knowledge` path normalises and assigns IDs, which makes
    /// it awkward for tests that need stable cross-cluster references.
    fn seed_entity_raw(storage: &GraphStorage, id: &str, agent_id: &str) {
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         first_seen_at, last_seen_at)
                     VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, datetime('now'), datetime('now'))",
                    rusqlite::params![id, agent_id],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn seed_relationship_raw(
        storage: &GraphStorage,
        id: &str,
        agent_id: &str,
        src: &str,
        tgt: &str,
        epistemic_class: &str,
    ) {
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_relationships
                        (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                         epistemic_class, first_seen_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, 'relates_to', ?5,
                             datetime('now'), datetime('now'))",
                    rusqlite::params![id, agent_id, src, tgt, epistemic_class],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn connectivity_strength_returns_zero_for_empty_clusters() {
        let storage = create_test_storage();
        let result = storage
            .connectivity_strength("agent-cs", &[], &["a".into()])
            .unwrap();
        assert_eq!(result, 0, "empty cluster A → 0");
        let result = storage
            .connectivity_strength("agent-cs", &["a".into()], &[])
            .unwrap();
        assert_eq!(result, 0, "empty cluster B → 0");
        let result = storage.connectivity_strength("agent-cs", &[], &[]).unwrap();
        assert_eq!(result, 0, "both empty → 0");
    }

    #[test]
    fn connectivity_strength_counts_both_directions() {
        let storage = create_test_storage();
        // Cluster A: e1, e2. Cluster B: e3, e4.
        for id in ["e1", "e2", "e3", "e4"] {
            seed_entity_raw(&storage, id, "agent-cs");
        }
        // Two A→B edges and one B→A edge.
        seed_relationship_raw(&storage, "r1", "agent-cs", "e1", "e3", "current");
        seed_relationship_raw(&storage, "r2", "agent-cs", "e2", "e4", "current");
        seed_relationship_raw(&storage, "r3", "agent-cs", "e4", "e1", "current");

        let cluster_a = vec!["e1".to_string(), "e2".to_string()];
        let cluster_b = vec!["e3".to_string(), "e4".to_string()];

        let result = storage
            .connectivity_strength("agent-cs", &cluster_a, &cluster_b)
            .unwrap();
        assert_eq!(result, 3, "two A→B + one B→A must sum to 3, got {result}");
    }

    #[test]
    fn connectivity_strength_ignores_intra_cluster_edges() {
        let storage = create_test_storage();
        for id in ["e1", "e2", "e3", "e4"] {
            seed_entity_raw(&storage, id, "agent-cs");
        }
        // Within-cluster edges (e1 → e2, e3 → e4) must NOT count.
        seed_relationship_raw(&storage, "r1", "agent-cs", "e1", "e2", "current");
        seed_relationship_raw(&storage, "r2", "agent-cs", "e3", "e4", "current");
        // One legitimate cross-cluster edge.
        seed_relationship_raw(&storage, "r3", "agent-cs", "e1", "e3", "current");

        let cluster_a = vec!["e1".to_string(), "e2".to_string()];
        let cluster_b = vec!["e3".to_string(), "e4".to_string()];

        let result = storage
            .connectivity_strength("agent-cs", &cluster_a, &cluster_b)
            .unwrap();
        assert_eq!(result, 1, "only the cross-cluster edge counts");
    }

    #[test]
    fn connectivity_strength_excludes_archival_edges() {
        let storage = create_test_storage();
        for id in ["e1", "e2", "e3"] {
            seed_entity_raw(&storage, id, "agent-cs");
        }
        // Two cross-cluster edges with distinct endpoints (the UNIQUE
        // constraint on (src, tgt, type) means we can't have two
        // edges on the same triplet — use different endpoints for the
        // current vs archived edge).
        seed_relationship_raw(&storage, "r-current", "agent-cs", "e1", "e2", "current");
        seed_relationship_raw(&storage, "r-archived", "agent-cs", "e1", "e3", "archival");

        let cluster_a = vec!["e1".to_string()];
        let cluster_b = vec!["e2".to_string(), "e3".to_string()];

        let result = storage
            .connectivity_strength("agent-cs", &cluster_a, &cluster_b)
            .unwrap();
        assert_eq!(
            result, 1,
            "only the current-class edge contributes; archival is filtered"
        );
    }

    #[test]
    fn connectivity_strength_scopes_by_agent_id() {
        let storage = create_test_storage();
        // Same logical entity IDs under two agents; relationships under
        // a different agent must not leak into the queried agent's count.
        for id in ["e1", "e2"] {
            seed_entity_raw(&storage, id, "agent-a");
            seed_entity_raw(&storage, &format!("{id}b"), "agent-b");
        }
        // agent-a has one cross-cluster edge.
        seed_relationship_raw(&storage, "ra", "agent-a", "e1", "e2", "current");
        // agent-b has two edges that would match if the agent filter
        // weren't applied. Both endpoints share names with cluster A/B,
        // but live under a different agent.
        seed_relationship_raw(&storage, "rb1", "agent-b", "e1b", "e2b", "current");
        seed_relationship_raw(&storage, "rb2", "agent-b", "e2b", "e1b", "current");

        let cluster_a = vec!["e1".to_string()];
        let cluster_b = vec!["e2".to_string()];

        let result = storage
            .connectivity_strength("agent-a", &cluster_a, &cluster_b)
            .unwrap();
        assert_eq!(result, 1, "agent-b edges must not leak into agent-a count");

        // And the reverse — agent-b's count is independent of agent-a's.
        let cluster_a_b = vec!["e1b".to_string()];
        let cluster_b_b = vec!["e2b".to_string()];
        let result = storage
            .connectivity_strength("agent-b", &cluster_a_b, &cluster_b_b)
            .unwrap();
        assert_eq!(result, 2, "agent-b sees its own edges in both directions");
    }

    #[test]
    fn connectivity_strength_zero_when_clusters_unconnected() {
        let storage = create_test_storage();
        for id in ["e1", "e2", "e3", "e4"] {
            seed_entity_raw(&storage, id, "agent-cs");
        }
        // Edges only within each cluster.
        seed_relationship_raw(&storage, "r1", "agent-cs", "e1", "e2", "current");
        seed_relationship_raw(&storage, "r2", "agent-cs", "e3", "e4", "current");

        let cluster_a = vec!["e1".to_string(), "e2".to_string()];
        let cluster_b = vec!["e3".to_string(), "e4".to_string()];

        let result = storage
            .connectivity_strength("agent-cs", &cluster_a, &cluster_b)
            .unwrap();
        assert_eq!(result, 0, "disconnected clusters → 0");
    }

    // -----------------------------------------------------------------
    // promote_cluster_to_aggregate + write_inter_cluster_relation (H-3d)
    // -----------------------------------------------------------------

    /// Read a single column from a single row by id. Convenience for
    /// the H-3d tests below.
    fn read_entity_field<T: rusqlite::types::FromSql>(
        storage: &GraphStorage,
        id: &str,
        column: &str,
    ) -> Option<T> {
        let id_owned = id.to_string();
        let column_owned = column.to_string();
        let sql = format!("SELECT {column_owned} FROM kg_entities WHERE id = ?1");
        storage
            .db
            .with_connection(|conn| {
                let value: rusqlite::Result<T> =
                    conn.query_row(&sql, rusqlite::params![id_owned], |row| row.get(0));
                Ok(value.ok())
            })
            .unwrap()
    }

    #[test]
    fn promote_cluster_creates_layer_n_plus_1_entity() {
        let storage = create_test_storage();
        for id in ["m1", "m2", "m3"] {
            seed_entity_raw(&storage, id, "agent-h");
        }
        let members = vec!["m1".to_string(), "m2".to_string(), "m3".to_string()];

        let agg_id = storage
            .promote_cluster_to_aggregate(
                "agent-h",
                1,
                &members,
                "test aggregate",
                "Three concepts grouped together for testing.",
                None,
            )
            .unwrap();
        assert!(
            agg_id.starts_with("agg-"),
            "aggregate id should have agg- prefix, got {agg_id}"
        );

        let layer: Option<i64> = read_entity_field(&storage, &agg_id, "layer");
        assert_eq!(layer, Some(1), "aggregate must live at layer 1");

        let parent: Option<String> = read_entity_field(&storage, &agg_id, "parent_cluster_id");
        assert!(parent.is_none(), "fresh aggregate has no parent");

        let name: Option<String> = read_entity_field(&storage, &agg_id, "name");
        assert_eq!(name, Some("test aggregate".to_string()));

        // Description is stored in properties JSON.
        let props: Option<String> = read_entity_field(&storage, &agg_id, "properties");
        let props_json: serde_json::Value =
            serde_json::from_str(&props.expect("properties present")).unwrap();
        assert_eq!(props_json["aggregate"], serde_json::json!(true));
        assert_eq!(
            props_json["description"],
            serde_json::json!("Three concepts grouped together for testing.")
        );
        assert_eq!(props_json["member_count"], serde_json::json!(3));
    }

    #[test]
    fn promote_cluster_updates_member_parent_pointers() {
        let storage = create_test_storage();
        for id in ["m1", "m2"] {
            seed_entity_raw(&storage, id, "agent-h");
        }

        let agg_id = storage
            .promote_cluster_to_aggregate(
                "agent-h",
                1,
                &["m1".to_string(), "m2".to_string()],
                "agg",
                "desc",
                None,
            )
            .unwrap();

        for member in ["m1", "m2"] {
            let parent: Option<String> = read_entity_field(&storage, member, "parent_cluster_id");
            assert_eq!(
                parent.as_deref(),
                Some(agg_id.as_str()),
                "{member} should point to the new aggregate"
            );
        }
    }

    #[test]
    fn promote_cluster_persists_embedding_when_provided() {
        let storage = create_test_storage();
        seed_entity_raw(&storage, "m1", "agent-h");

        // The kg_name_index vec0 table is declared with FLOAT[384] in
        // the default-dimension init path. Padded fixture vector so
        // the dimension check passes (test doesn't care about the
        // actual values — only that the row exists).
        let mut embedding = vec![0.0_f32; 384];
        embedding[0] = 0.1;
        embedding[1] = 0.2;
        let agg_id = storage
            .promote_cluster_to_aggregate(
                "agent-h",
                1,
                &["m1".to_string()],
                "agg",
                "desc",
                Some(embedding.clone()),
            )
            .unwrap();

        // kg_name_index should have a row for the aggregate's id.
        let agg_id_for_db = agg_id.clone();
        let row_count: i64 = storage
            .db
            .with_connection(|conn| {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM kg_name_index WHERE entity_id = ?1",
                        rusqlite::params![agg_id_for_db],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                Ok(count)
            })
            .unwrap();
        assert_eq!(
            row_count, 1,
            "kg_name_index should have a row for the aggregate"
        );
    }

    #[test]
    fn promote_cluster_skips_embedding_when_none() {
        let storage = create_test_storage();
        seed_entity_raw(&storage, "m1", "agent-h");

        let agg_id = storage
            .promote_cluster_to_aggregate("agent-h", 1, &["m1".to_string()], "agg", "desc", None)
            .unwrap();

        let agg_id_for_db = agg_id.clone();
        let row_count: i64 = storage
            .db
            .with_connection(|conn| {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM kg_name_index WHERE entity_id = ?1",
                        rusqlite::params![agg_id_for_db],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                Ok(count)
            })
            .unwrap();
        assert_eq!(row_count, 0, "no embedding → no kg_name_index row");
    }

    #[test]
    fn promote_cluster_is_atomic_under_missing_members() {
        // A member id that doesn't exist must not fail the whole
        // operation. The aggregate still gets created; the missing
        // member's UPDATE simply affects zero rows.
        let storage = create_test_storage();
        seed_entity_raw(&storage, "m1", "agent-h");

        let agg_id = storage
            .promote_cluster_to_aggregate(
                "agent-h",
                1,
                &["m1".to_string(), "ghost".to_string()],
                "agg",
                "desc",
                None,
            )
            .unwrap();

        let layer: Option<i64> = read_entity_field(&storage, &agg_id, "layer");
        assert_eq!(layer, Some(1));

        let m1_parent: Option<String> = read_entity_field(&storage, "m1", "parent_cluster_id");
        assert_eq!(m1_parent, Some(agg_id));
    }

    #[test]
    fn write_inter_cluster_relation_sets_flag_and_layer() {
        let storage = create_test_storage();
        seed_entity_raw(&storage, "agg-a", "agent-h");
        seed_entity_raw(&storage, "agg-b", "agent-h");

        let rel_id = storage
            .write_inter_cluster_relation("agent-h", 2, "agg-a", "agg-b", "encompasses")
            .unwrap();
        assert!(
            rel_id.starts_with("rel-agg-"),
            "inter-cluster relation id should have rel-agg- prefix, got {rel_id}"
        );

        let rel_id_for_db = rel_id.clone();
        let (layer, is_inter, src, tgt, rtype): (i64, i64, String, String, String) = storage
            .db
            .with_connection(|conn| {
                let row = conn
                    .query_row(
                        "SELECT layer, is_inter_cluster, source_entity_id, target_entity_id,
                                relationship_type
                         FROM kg_relationships WHERE id = ?1",
                        rusqlite::params![rel_id_for_db],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                            ))
                        },
                    )
                    .unwrap();
                Ok(row)
            })
            .unwrap();
        assert_eq!(layer, 2);
        assert_eq!(is_inter, 1);
        assert_eq!(src, "agg-a");
        assert_eq!(tgt, "agg-b");
        assert_eq!(rtype, "encompasses");
    }

    /// Helper for H-3e: insert an entity at a non-zero layer with an
    /// embedding row in kg_name_index. Used to seed layer-N fixtures
    /// for the list-by-layer tests below.
    fn seed_entity_at_layer(
        storage: &GraphStorage,
        id: &str,
        agent_id: &str,
        layer: i64,
        embedding: Vec<f32>,
    ) {
        let id = id.to_string();
        let agent_id = agent_id.to_string();
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         first_seen_at, last_seen_at, layer)
                     VALUES (?1, ?2, 'Concept', ?1, ?1, ?1,
                             datetime('now'), datetime('now'), ?3)",
                    rusqlite::params![id, agent_id, layer],
                )?;
                let embedding_json = serde_json::to_string(&embedding).unwrap();
                conn.execute(
                    "INSERT INTO kg_name_index (entity_id, name_embedding) VALUES (?1, ?2)",
                    rusqlite::params![id, embedding_json],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn padded_embedding(seed: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = seed;
        v[1] = seed * 0.5;
        v
    }

    #[test]
    fn list_entities_with_embeddings_at_layer_returns_only_matching_layer() {
        let storage = create_test_storage();
        seed_entity_at_layer(&storage, "l0a", "agent-h", 0, padded_embedding(1.0));
        seed_entity_at_layer(&storage, "l0b", "agent-h", 0, padded_embedding(2.0));
        seed_entity_at_layer(&storage, "l1a", "agent-h", 1, padded_embedding(3.0));

        let layer0 = storage
            .list_entities_with_embeddings_at_layer("agent-h", 0, 0)
            .unwrap();
        let mut layer0_ids: Vec<_> = layer0.iter().map(|(id, _)| id.clone()).collect();
        layer0_ids.sort();
        assert_eq!(layer0_ids, vec!["l0a".to_string(), "l0b".to_string()]);

        let layer1 = storage
            .list_entities_with_embeddings_at_layer("agent-h", 1, 0)
            .unwrap();
        assert_eq!(layer1.len(), 1);
        assert_eq!(layer1[0].0, "l1a");
        assert!(
            layer1[0].1.len() == 384,
            "embedding round-trip preserves dimension"
        );
    }

    #[test]
    fn list_entities_with_embeddings_at_layer_scopes_by_agent_id() {
        let storage = create_test_storage();
        seed_entity_at_layer(&storage, "agA-e1", "agent-a", 0, padded_embedding(1.0));
        seed_entity_at_layer(&storage, "agB-e1", "agent-b", 0, padded_embedding(2.0));

        let rows = storage
            .list_entities_with_embeddings_at_layer("agent-a", 0, 0)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "agA-e1");
    }

    /// Regression: base entities live under `agent_id = '__global__'` by
    /// the cross-agent dedup convention in this codebase. The hierarchy
    /// reads must dual-match so a daemon configured for `agent_id = 'root'`
    /// can see them — without this, the builder hits PoolTooSmall on
    /// every cycle even when the graph is full.
    #[test]
    fn list_entities_with_embeddings_at_layer_picks_up_global_rows() {
        let storage = create_test_storage();
        // Seed under __global__ (the prod convention).
        seed_entity_at_layer(&storage, "g-e1", "__global__", 0, padded_embedding(1.0));
        seed_entity_at_layer(&storage, "g-e2", "__global__", 0, padded_embedding(2.0));
        // And one under a different agent_id to be sure cross-agent
        // leak doesn't happen (the dual-match is `?1 OR '__global__'`,
        // not "everything").
        seed_entity_at_layer(
            &storage,
            "other-e1",
            "agent-other",
            0,
            padded_embedding(3.0),
        );

        let rows = storage
            .list_entities_with_embeddings_at_layer("root", 0, 0)
            .unwrap();
        let mut ids: Vec<_> = rows.iter().map(|(id, _)| id.clone()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["g-e1".to_string(), "g-e2".to_string()],
            "dual-match must pull __global__ rows but not other agents'"
        );
    }

    /// Same regression for `connectivity_strength`: cross-cluster edges
    /// stored under `__global__` must be counted when the queried agent
    /// is `root`.
    #[test]
    fn connectivity_strength_picks_up_global_relationships() {
        let storage = create_test_storage();
        seed_entity_raw(&storage, "g-a", "__global__");
        seed_entity_raw(&storage, "g-b", "__global__");
        seed_relationship_raw(&storage, "r-global", "__global__", "g-a", "g-b", "current");

        let result = storage
            .connectivity_strength("root", &["g-a".to_string()], &["g-b".to_string()])
            .unwrap();
        assert_eq!(result, 1, "edge under __global__ must count for root");
    }

    /// Same regression for `compute_lca_path`: the parent-pointer walk
    /// must cross the __global__ boundary so a layer-0 entity under
    /// __global__ can reach a layer-1 aggregate.
    #[test]
    fn lca_walk_traverses_global_parent_pointers() {
        let storage = create_test_storage();
        seed_hier_entity(&storage, "agg-top", "__global__", 1, None);
        seed_hier_entity(&storage, "g-a", "__global__", 0, Some("agg-top"));
        seed_hier_entity(&storage, "g-b", "__global__", 0, Some("agg-top"));

        let (lca, path, max_layer) = storage
            .compute_lca_path("root", &["g-a".to_string(), "g-b".to_string()])
            .unwrap();
        assert_eq!(lca.as_deref(), Some("agg-top"));
        assert_eq!(path, vec!["agg-top".to_string()]);
        assert_eq!(max_layer, 1);
    }

    // -----------------------------------------------------------------
    // hierarchy_summary (Observatory stats endpoint)
    // -----------------------------------------------------------------

    /// Seed an aggregate entity at a given layer with a JSON
    /// `properties` blob carrying `member_count` + `description`.
    /// Used by the summary tests below to control the top-N ordering.
    fn seed_aggregate(
        storage: &GraphStorage,
        id: &str,
        agent_id: &str,
        name: &str,
        layer: i64,
        member_count: usize,
        description: &str,
    ) {
        let id = id.to_string();
        let agent_id = agent_id.to_string();
        let name = name.to_string();
        let description = description.to_string();
        let props = serde_json::json!({
            "aggregate": true,
            "member_count": member_count,
            "description": description,
        })
        .to_string();
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         properties, first_seen_at, last_seen_at, layer)
                     VALUES (?1, ?2, 'Concept', ?3, ?3, ?3,
                             ?4, datetime('now'), datetime('now'), ?5)",
                    rusqlite::params![id, agent_id, name, props, layer],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn hierarchy_summary_groups_by_layer_and_orders_aggregates_by_member_count() {
        let storage = create_test_storage();
        // Layer 0 base entities (dual-match: stored under __global__).
        for i in 0..5 {
            seed_entity_raw(&storage, &format!("e{i}"), "__global__");
        }
        // Three aggregates at layer 1, two at layer 2.
        seed_aggregate(&storage, "a1", "root", "small-cluster", 1, 5, "small");
        seed_aggregate(
            &storage,
            "a2",
            "root",
            "huge-cluster",
            1,
            50,
            "huge — should be ranked first",
        );
        seed_aggregate(&storage, "a3", "root", "medium-cluster", 1, 20, "medium");
        seed_aggregate(&storage, "a4", "root", "top-l2-a", 2, 30, "top layer");
        seed_aggregate(&storage, "a5", "root", "top-l2-b", 2, 25, "top layer");

        // One inter-cluster edge between two layer-1 aggregates.
        seed_relationship_raw(&storage, "ic1", "root", "a1", "a2", "current");
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE kg_relationships SET is_inter_cluster = 1, layer = 1 \
                     WHERE id = ?1",
                    rusqlite::params!["ic1"],
                )?;
                Ok(())
            })
            .unwrap();

        let (layer_counts, inter_cluster, top_aggs) =
            storage.hierarchy_summary("root", 10).unwrap();

        // Layer counts: layer 0 = 5 (under __global__, dual-matched);
        // layer 1 = 3; layer 2 = 2.
        assert_eq!(
            layer_counts,
            vec![(0, 5), (1, 3), (2, 2)],
            "layer_counts must group correctly + include __global__ rows"
        );

        // Total inter-cluster edges = 1.
        assert_eq!(inter_cluster, 1);

        // Top aggregates ordered by member_count DESC. Layer 0
        // entities are excluded (layer > 0 filter).
        let names: Vec<&str> = top_aggs.iter().map(|t| t.1.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "huge-cluster",
                "top-l2-a",
                "top-l2-b",
                "medium-cluster",
                "small-cluster",
            ],
            "aggregates must order by member_count DESC across layers"
        );

        // First entry has the right shape end-to-end.
        let huge = &top_aggs[0];
        assert_eq!(huge.0, "a2");
        assert_eq!(huge.1, "huge-cluster");
        assert_eq!(huge.2, 1, "layer");
        assert_eq!(huge.3, 50, "member_count");
        assert!(huge.4.contains("ranked first"), "description preserved");
    }

    #[test]
    fn hierarchy_summary_top_n_zero_skips_aggregate_query() {
        let storage = create_test_storage();
        seed_aggregate(&storage, "a1", "root", "agg-1", 1, 10, "");
        seed_aggregate(&storage, "a2", "root", "agg-2", 1, 20, "");

        let (_layers, _ic, top) = storage.hierarchy_summary("root", 0).unwrap();
        assert!(top.is_empty(), "top_n=0 must return zero aggregate rows");
    }

    #[test]
    fn hierarchy_summary_top_n_caps_results() {
        let storage = create_test_storage();
        for i in 0..5 {
            seed_aggregate(
                &storage,
                &format!("a{i}"),
                "root",
                &format!("agg-{i}"),
                1,
                (5 - i) * 10,
                "",
            );
        }
        let (_layers, _ic, top) = storage.hierarchy_summary("root", 2).unwrap();
        assert_eq!(top.len(), 2, "top_n must cap the result set");
        // Highest member_count first.
        assert_eq!(top[0].1, "agg-0");
        assert_eq!(top[1].1, "agg-1");
    }

    #[test]
    fn hierarchy_summary_empty_db_returns_empty_layer_list() {
        let storage = create_test_storage();
        let (layers, inter_cluster, top) = storage.hierarchy_summary("root", 10).unwrap();
        assert!(layers.is_empty());
        assert_eq!(inter_cluster, 0);
        assert!(top.is_empty());
    }

    #[test]
    fn list_entities_with_embeddings_at_layer_skips_entities_without_embedding() {
        let storage = create_test_storage();
        seed_entity_at_layer(&storage, "with-emb", "agent-h", 0, padded_embedding(1.0));
        // Insert an entity WITHOUT a kg_name_index row.
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         first_seen_at, last_seen_at, layer)
                     VALUES ('no-emb', 'agent-h', 'Concept', 'x', 'x', 'x',
                             datetime('now'), datetime('now'), 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let rows = storage
            .list_entities_with_embeddings_at_layer("agent-h", 0, 0)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "entity without kg_name_index row must be skipped"
        );
        assert_eq!(rows[0].0, "with-emb");
    }

    #[test]
    fn list_entities_with_embeddings_at_layer_respects_limit() {
        let storage = create_test_storage();
        for i in 0..5 {
            seed_entity_at_layer(
                &storage,
                &format!("e{i}"),
                "agent-h",
                0,
                padded_embedding(i as f32),
            );
        }

        let unbounded = storage
            .list_entities_with_embeddings_at_layer("agent-h", 0, 0)
            .unwrap();
        assert_eq!(unbounded.len(), 5, "limit=0 means no limit");

        let bounded = storage
            .list_entities_with_embeddings_at_layer("agent-h", 0, 2)
            .unwrap();
        assert_eq!(bounded.len(), 2);
    }

    #[test]
    fn list_entities_with_embeddings_at_layer_excludes_archival_entities() {
        let storage = create_test_storage();
        seed_entity_at_layer(&storage, "current", "agent-h", 0, padded_embedding(1.0));
        // Same shape as seed_entity_at_layer but with epistemic_class='archival'.
        let archival_emb = serde_json::to_string(&padded_embedding(2.0)).unwrap();
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, first_seen_at, last_seen_at, layer)
                     VALUES ('archived', 'agent-h', 'Concept', 'x', 'x', 'x',
                             'archival', datetime('now'), datetime('now'), 0)",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO kg_name_index (entity_id, name_embedding) VALUES (?1, ?2)",
                    rusqlite::params!["archived", archival_emb],
                )?;
                Ok(())
            })
            .unwrap();

        let rows = storage
            .list_entities_with_embeddings_at_layer("agent-h", 0, 0)
            .unwrap();
        assert_eq!(rows.len(), 1, "only current-class entities count");
        assert_eq!(rows[0].0, "current");
    }

    // -----------------------------------------------------------------
    // compute_lca_path (Phase H-4a)
    // -----------------------------------------------------------------

    /// Seed an entity at `layer` with the given `parent_cluster_id`
    /// (None = top-level). Used by the LCA tests below.
    fn seed_hier_entity(
        storage: &GraphStorage,
        id: &str,
        agent_id: &str,
        layer: i64,
        parent: Option<&str>,
    ) {
        let id = id.to_string();
        let agent_id = agent_id.to_string();
        let parent = parent.map(String::from);
        storage
            .db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         first_seen_at, last_seen_at, layer, parent_cluster_id)
                     VALUES (?1, ?2, 'Concept', ?1, ?1, ?1,
                             datetime('now'), datetime('now'), ?3, ?4)",
                    rusqlite::params![id, agent_id, layer, parent],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn lca_empty_seeds_yields_empty_result() {
        let storage = create_test_storage();
        let (lca, path, max_layer) = storage.compute_lca_path("agent-lca", &[]).unwrap();
        assert!(lca.is_none());
        assert!(path.is_empty());
        assert_eq!(max_layer, 0);
    }

    #[test]
    fn lca_single_seed_is_its_own_lca() {
        let storage = create_test_storage();
        seed_hier_entity(&storage, "e1", "agent-lca", 0, None);
        let (lca, path, max_layer) = storage
            .compute_lca_path("agent-lca", &["e1".to_string()])
            .unwrap();
        assert_eq!(lca.as_deref(), Some("e1"));
        assert!(
            path.is_empty(),
            "path excludes the seed; single-seed path must be empty"
        );
        assert_eq!(max_layer, 0);
    }

    #[test]
    fn lca_seeds_with_no_parents_have_no_common_ancestor() {
        let storage = create_test_storage();
        seed_hier_entity(&storage, "a", "agent-lca", 0, None);
        seed_hier_entity(&storage, "b", "agent-lca", 0, None);
        let (lca, path, max_layer) = storage
            .compute_lca_path("agent-lca", &["a".to_string(), "b".to_string()])
            .unwrap();
        assert!(lca.is_none(), "two unparented seeds share no LCA");
        assert!(path.is_empty());
        assert_eq!(max_layer, 0);
    }

    #[test]
    fn lca_two_seeds_under_same_parent_resolves_at_layer_1() {
        let storage = create_test_storage();
        // agg-1 (layer 1) parent of both a, b (layer 0)
        seed_hier_entity(&storage, "agg-1", "agent-lca", 1, None);
        seed_hier_entity(&storage, "a", "agent-lca", 0, Some("agg-1"));
        seed_hier_entity(&storage, "b", "agent-lca", 0, Some("agg-1"));

        let (lca, path, max_layer) = storage
            .compute_lca_path("agent-lca", &["a".to_string(), "b".to_string()])
            .unwrap();
        assert_eq!(lca.as_deref(), Some("agg-1"));
        assert_eq!(
            path,
            vec!["agg-1".to_string()],
            "path contains just the LCA (seeds excluded)"
        );
        assert_eq!(max_layer, 1);
    }

    #[test]
    fn lca_three_seeds_across_two_layers_resolves_at_layer_2() {
        let storage = create_test_storage();
        // Hierarchy:
        //   top (L2)
        //     ├── mid-1 (L1)
        //     │     ├── a (L0)
        //     │     └── b (L0)
        //     └── mid-2 (L1)
        //           └── c (L0)
        // LCA(a, c) = top; LCA(a, b) = mid-1.
        seed_hier_entity(&storage, "top", "agent-lca", 2, None);
        seed_hier_entity(&storage, "mid-1", "agent-lca", 1, Some("top"));
        seed_hier_entity(&storage, "mid-2", "agent-lca", 1, Some("top"));
        seed_hier_entity(&storage, "a", "agent-lca", 0, Some("mid-1"));
        seed_hier_entity(&storage, "b", "agent-lca", 0, Some("mid-1"));
        seed_hier_entity(&storage, "c", "agent-lca", 0, Some("mid-2"));

        // a, b → mid-1
        let (lca, path, max_layer) = storage
            .compute_lca_path("agent-lca", &["a".to_string(), "b".to_string()])
            .unwrap();
        assert_eq!(lca.as_deref(), Some("mid-1"));
        assert_eq!(path, vec!["mid-1".to_string()]);
        assert_eq!(max_layer, 1);

        // a, c → top, path also contains both mids
        let (lca, mut path, max_layer) = storage
            .compute_lca_path("agent-lca", &["a".to_string(), "c".to_string()])
            .unwrap();
        assert_eq!(lca.as_deref(), Some("top"));
        path.sort();
        assert_eq!(
            path,
            vec!["mid-1".to_string(), "mid-2".to_string(), "top".to_string()]
        );
        assert_eq!(max_layer, 2);
    }

    #[test]
    fn lca_walk_terminates_on_corrupt_parent_cycle() {
        // Two seeds, one with a cycle in its parent chain. The walk
        // must bail out at MAX_LCA_WALK rather than hang. The cycle
        // means the seed eventually re-visits an ancestor but never
        // hits the other seed's chain — result: no LCA, no panic.
        let storage = create_test_storage();
        seed_hier_entity(&storage, "x", "agent-lca", 1, Some("y"));
        seed_hier_entity(&storage, "y", "agent-lca", 2, Some("x")); // cycle x→y→x
        seed_hier_entity(&storage, "z", "agent-lca", 0, None);

        let (lca, path, _) = storage
            .compute_lca_path("agent-lca", &["x".to_string(), "z".to_string()])
            .unwrap();
        assert!(lca.is_none(), "z has no parent — no common ancestor");
        assert!(path.is_empty());
    }

    #[test]
    fn lca_scopes_by_agent_id() {
        let storage = create_test_storage();
        // Agent A: a, b → agg-A
        seed_hier_entity(&storage, "agg-A", "agent-A", 1, None);
        seed_hier_entity(&storage, "a", "agent-A", 0, Some("agg-A"));
        seed_hier_entity(&storage, "b", "agent-A", 0, Some("agg-A"));
        // Agent B has its own hierarchy with no overlap.
        seed_hier_entity(&storage, "agg-B", "agent-B", 1, None);
        seed_hier_entity(&storage, "p", "agent-B", 0, Some("agg-B"));

        let (lca_a, _, _) = storage
            .compute_lca_path("agent-A", &["a".to_string(), "b".to_string()])
            .unwrap();
        assert_eq!(lca_a.as_deref(), Some("agg-A"));

        // Querying agent-A with an agent-B entity returns no LCA —
        // the parent walk filters by agent_id, so p's parent pointer
        // is invisible under agent-A.
        let (lca_cross, _, _) = storage
            .compute_lca_path("agent-A", &["a".to_string(), "p".to_string()])
            .unwrap();
        assert!(
            lca_cross.is_none(),
            "cross-agent seeds must not share an LCA"
        );
    }

    // -----------------------------------------------------------------
    // list_inter_cluster_relations (Phase H-4 follow-up)
    // -----------------------------------------------------------------

    #[test]
    fn list_inter_cluster_relations_empty_input_yields_empty() {
        let storage = create_test_storage();
        let out = storage
            .list_inter_cluster_relations("agent-h", &[])
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn list_inter_cluster_relations_returns_only_marked_rows_in_set() {
        let storage = create_test_storage();
        for id in ["agg-a", "agg-b", "agg-c"] {
            seed_entity_raw(&storage, id, "agent-h");
        }
        // One inter-cluster edge (the kind we want).
        storage
            .write_inter_cluster_relation("agent-h", 2, "agg-a", "agg-b", "encompasses")
            .unwrap();
        // One ordinary base edge — must NOT come back.
        seed_relationship_raw(&storage, "r-base", "agent-h", "agg-a", "agg-c", "current");

        let ids = vec![
            "agg-a".to_string(),
            "agg-b".to_string(),
            "agg-c".to_string(),
        ];
        let out = storage
            .list_inter_cluster_relations("agent-h", &ids)
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, "agg-a"); // source
        assert_eq!(out[0].2, "agg-b"); // target
        assert_eq!(out[0].3, "encompasses");
        assert_eq!(out[0].4, 2); // layer
    }

    #[test]
    fn list_inter_cluster_relations_excludes_edges_outside_id_set() {
        let storage = create_test_storage();
        for id in ["agg-a", "agg-b", "agg-c"] {
            seed_entity_raw(&storage, id, "agent-h");
        }
        // Edge between agg-a and agg-c — but agg-c is not in our query set.
        storage
            .write_inter_cluster_relation("agent-h", 2, "agg-a", "agg-c", "encompasses")
            .unwrap();

        let ids = vec!["agg-a".to_string(), "agg-b".to_string()];
        let out = storage
            .list_inter_cluster_relations("agent-h", &ids)
            .unwrap();
        assert!(
            out.is_empty(),
            "edge whose target is outside the queried set must not surface"
        );
    }

    #[test]
    fn list_inter_cluster_relations_scopes_by_agent_id() {
        let storage = create_test_storage();
        for id in ["agg-a", "agg-b"] {
            seed_entity_raw(&storage, id, "agent-a");
            seed_entity_raw(&storage, &format!("{id}-b"), "agent-b");
        }
        storage
            .write_inter_cluster_relation("agent-a", 2, "agg-a", "agg-b", "shares")
            .unwrap();
        storage
            .write_inter_cluster_relation("agent-b", 2, "agg-a-b", "agg-b-b", "shares")
            .unwrap();

        let ids = vec!["agg-a".to_string(), "agg-b".to_string()];
        let out = storage
            .list_inter_cluster_relations("agent-a", &ids)
            .unwrap();
        assert_eq!(out.len(), 1, "agent-b's edge must not leak into agent-a");
    }

    #[test]
    fn write_inter_cluster_relation_rejects_duplicate_triple() {
        let storage = create_test_storage();
        seed_entity_raw(&storage, "agg-a", "agent-h");
        seed_entity_raw(&storage, "agg-b", "agent-h");

        storage
            .write_inter_cluster_relation("agent-h", 2, "agg-a", "agg-b", "encompasses")
            .unwrap();

        // Same (src, tgt, type) — UNIQUE constraint fires.
        let result =
            storage.write_inter_cluster_relation("agent-h", 2, "agg-a", "agg-b", "encompasses");
        assert!(
            result.is_err(),
            "second write with same (src, tgt, type) must fail with UNIQUE"
        );

        // Same (src, tgt) with a different type — allowed.
        let _other = storage
            .write_inter_cluster_relation("agent-h", 2, "agg-a", "agg-b", "differs-from")
            .unwrap();
    }

    // ============================================================================
    // MEM-001 Part B-1 — search_entities_by_name_embedding returns confidence
    // ============================================================================

    #[test]
    fn search_entities_by_name_embedding_returns_entity_confidence() {
        let storage = create_test_storage();
        let agent = "agent-b1";

        // The default vec0 dimension in the test DB is 384 (see schema
        // initialisation). Insert one entity with a non-default
        // confidence (0.42) + its all-zero vec row at that dim. The
        // vector doesn't need to be meaningful — KNN against the same
        // all-zero query returns it as the closest (and only) hit.
        const DIM: usize = 384;
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('e1', ?1, 'Concept', 'Alice', 'alice', 'h-alice',
                             'current', 0.42, 1, 0, ?2, ?2)",
                    rusqlite::params![agent, now],
                )?;
                let vec_json = serde_json::to_string(&vec![0.0_f32; DIM]).unwrap();
                conn.execute(
                    "INSERT INTO kg_name_index(entity_id, name_embedding) VALUES ('e1', ?1)",
                    rusqlite::params![vec_json],
                )?;
                Ok(())
            })
            .unwrap();

        let query = vec![0.0_f32; DIM];

        let hits = storage
            .search_entities_by_name_embedding(&query, 5, agent)
            .expect("search ok");

        assert_eq!(hits.len(), 1, "expected one hit, got {hits:?}");
        let (_id, name, _etype, _dist, confidence) = &hits[0];
        assert_eq!(name, "Alice");
        assert!(
            (confidence - 0.42).abs() < 1e-6,
            "expected confidence 0.42, got {confidence}"
        );
    }

    // ========================================================================
    // MEM-001 Part B-2 — traverse_weighted_sync tests
    // ========================================================================

    fn insert_b2_entity(
        storage: &GraphStorage,
        id: &str,
        agent: &str,
        confidence: f64,
        epistemic_class: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, ?3, ?4, 1, 0, ?5, ?5)",
                    rusqlite::params![id, agent, epistemic_class, confidence, now],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)] // 8 args in a test helper is fine.
    fn insert_b2_relationship(
        storage: &GraphStorage,
        id: &str,
        agent: &str,
        src: &str,
        tgt: &str,
        rel_type: &str,
        confidence: f64,
        epistemic_class: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_relationships
                        (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 0, ?8, ?8)",
                    rusqlite::params![
                        id,
                        agent,
                        src,
                        tgt,
                        rel_type,
                        epistemic_class,
                        confidence,
                        now
                    ],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn traverse_weighted_returns_direct_neighbor() {
        let storage = create_test_storage();
        let agent = "agent-b2";

        insert_b2_entity(&storage, "seed", agent, 1.0, "current");
        insert_b2_entity(&storage, "neighbor", agent, 0.7, "current");
        insert_b2_relationship(
            &storage,
            "r1",
            agent,
            "seed",
            "neighbor",
            "RelatedTo",
            0.9,
            "current",
        );

        let rows = storage
            .traverse_weighted_sync("seed", agent, 2, 0.0, 10)
            .expect("ok");
        assert_eq!(rows.len(), 1, "expected one neighbor, got {rows:?}");
        let r = &rows[0];
        assert_eq!(r.entity_id, "neighbor");
        assert_eq!(r.hop, 1);
        assert!((r.entity_confidence - 0.7).abs() < 1e-6);
        assert!((r.edge_confidence_product - 0.9).abs() < 1e-6);
    }

    #[test]
    fn traverse_weighted_filters_low_confidence_edges() {
        let storage = create_test_storage();
        let agent = "agent-b2";

        insert_b2_entity(&storage, "seed", agent, 1.0, "current");
        insert_b2_entity(&storage, "near", agent, 0.8, "current");
        insert_b2_entity(&storage, "far", agent, 0.8, "current");
        // High-confidence edge to "near"; low-confidence edge to "far".
        insert_b2_relationship(
            &storage,
            "r-near",
            agent,
            "seed",
            "near",
            "RelatedTo",
            0.9,
            "current",
        );
        insert_b2_relationship(
            &storage, "r-far", agent, "seed", "far", "WorksFor", 0.05, "current",
        );

        let rows = storage
            .traverse_weighted_sync("seed", agent, 1, 0.5, 10)
            .expect("ok");
        let ids: Vec<&str> = rows.iter().map(|r| r.entity_id.as_str()).collect();
        assert_eq!(ids, vec!["near"], "low-confidence edge should be dropped");
    }

    #[test]
    fn traverse_weighted_two_hops_accumulates_edge_product() {
        let storage = create_test_storage();
        let agent = "agent-b2";

        insert_b2_entity(&storage, "seed", agent, 1.0, "current");
        insert_b2_entity(&storage, "mid", agent, 0.9, "current");
        insert_b2_entity(&storage, "leaf", agent, 0.6, "current");
        insert_b2_relationship(
            &storage,
            "r1",
            agent,
            "seed",
            "mid",
            "RelatedTo",
            0.8,
            "current",
        );
        insert_b2_relationship(
            &storage, "r2", agent, "mid", "leaf", "Contains", 0.5, "current",
        );

        let rows = storage
            .traverse_weighted_sync("seed", agent, 2, 0.0, 10)
            .expect("ok");
        let leaf = rows
            .iter()
            .find(|r| r.entity_id == "leaf")
            .expect("leaf should be reachable at hop 2");
        assert_eq!(leaf.hop, 2);
        assert!(
            (leaf.edge_confidence_product - (0.8 * 0.5)).abs() < 1e-6,
            "edge product should be 0.8 * 0.5 = 0.4, got {}",
            leaf.edge_confidence_product
        );
    }

    #[test]
    fn traverse_weighted_excludes_archival_entities_and_edges() {
        let storage = create_test_storage();
        let agent = "agent-b2";

        insert_b2_entity(&storage, "seed", agent, 1.0, "current");
        insert_b2_entity(&storage, "archived-target", agent, 0.7, "archival");
        insert_b2_entity(&storage, "current-target", agent, 0.7, "current");
        insert_b2_relationship(
            &storage,
            "r-arch",
            agent,
            "seed",
            "archived-target",
            "X",
            0.9,
            "current",
        );
        insert_b2_relationship(
            &storage,
            "r-archedge",
            agent,
            "seed",
            "current-target",
            "Y",
            0.9,
            "archival",
        );

        let rows = storage
            .traverse_weighted_sync("seed", agent, 1, 0.0, 10)
            .expect("ok");
        assert!(
            rows.is_empty(),
            "archival rows + archival edges both excluded: got {rows:?}"
        );
    }

    #[test]
    fn traverse_weighted_agent_scope_blocks_cross_agent() {
        let storage = create_test_storage();

        insert_b2_entity(&storage, "seed", "agent-a", 1.0, "current");
        insert_b2_entity(&storage, "other", "agent-b", 0.8, "current");
        insert_b2_relationship(
            &storage, "r-x", "agent-b", "seed", "other", "X", 0.9, "current",
        );

        let rows = storage
            .traverse_weighted_sync("seed", "agent-a", 1, 0.0, 10)
            .expect("ok");
        assert!(
            rows.is_empty(),
            "agent-b's edge shouldn't be visible from agent-a: {rows:?}"
        );
    }

    #[test]
    fn traverse_weighted_global_agent_visible_to_any_agent() {
        let storage = create_test_storage();

        insert_b2_entity(&storage, "seed", "agent-a", 1.0, "current");
        insert_b2_entity(&storage, "shared", "__global__", 0.8, "current");
        insert_b2_relationship(
            &storage,
            "r-g",
            "__global__",
            "seed",
            "shared",
            "Z",
            0.9,
            "current",
        );

        let rows = storage
            .traverse_weighted_sync("seed", "agent-a", 1, 0.0, 10)
            .expect("ok");
        let ids: Vec<&str> = rows.iter().map(|r| r.entity_id.as_str()).collect();
        assert_eq!(ids, vec!["shared"], "__global__ rows must be reachable");
    }
}
