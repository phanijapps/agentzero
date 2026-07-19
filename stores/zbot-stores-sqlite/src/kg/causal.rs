//! # Causal Edge Store
//!
//! CRUD operations for causal edges in the knowledge graph.
//! Causal edges represent cause-effect relationships between entities.

use crate::KnowledgeDatabase;
use knowledge_graph::error::{GraphError, GraphResult};
use rusqlite::params;
use std::sync::Arc;

/// A causal relationship between two knowledge graph entities.
#[derive(Debug, Clone)]
pub struct CausalEdge {
    pub id: String,
    pub agent_id: String,
    pub cause_entity_id: String,
    pub effect_entity_id: String,
    pub relationship: String,
    pub confidence: f64,
    pub session_id: Option<String>,
    pub created_at: String,
}

/// CRUD operations for causal edges in the knowledge graph.
pub struct CausalEdgeStore {
    db: Arc<KnowledgeDatabase>,
}

impl CausalEdgeStore {
    /// Create a new causal edge store backed by the shared `KnowledgeDatabase` pool.
    pub fn new(db: Arc<KnowledgeDatabase>) -> Self {
        Self { db }
    }

    /// Store a causal edge. Skips if duplicate (same primary key).
    pub async fn store_edge(&self, edge: &CausalEdge) -> GraphResult<()> {
        let id = edge.id.clone();
        let agent_id = edge.agent_id.clone();
        let cause_entity_id = edge.cause_entity_id.clone();
        let effect_entity_id = edge.effect_entity_id.clone();
        let relationship = edge.relationship.clone();
        let confidence = edge.confidence;
        let session_id = edge.session_id.clone();
        let created_at = edge.created_at.clone();

        self.db
            .with_connection(|conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO kg_causal_edges \
                     (id, agent_id, cause_entity_id, effect_entity_id, relationship, confidence, session_id, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        id,
                        agent_id,
                        cause_entity_id,
                        effect_entity_id,
                        relationship,
                        confidence,
                        session_id,
                        created_at,
                    ],
                )?;
                Ok(())
            })
            .map_err(GraphError::Other)
    }

    /// Get causal edges where the given entity is the cause.
    pub async fn get_effects(&self, entity_id: &str) -> GraphResult<Vec<CausalEdge>> {
        let entity_id = entity_id.to_owned();
        self.db
            .with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_id, cause_entity_id, effect_entity_id, relationship, confidence, session_id, created_at \
                     FROM kg_causal_edges WHERE cause_entity_id = ?1",
                )?;

                let edges = stmt
                    .query_map(params![entity_id], |row| {
                        Ok(CausalEdge {
                            id: row.get(0)?,
                            agent_id: row.get(1)?,
                            cause_entity_id: row.get(2)?,
                            effect_entity_id: row.get(3)?,
                            relationship: row.get(4)?,
                            confidence: row.get(5)?,
                            session_id: row.get(6)?,
                            created_at: row.get(7)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(edges)
            })
            .map_err(GraphError::Other)
    }

    /// Get causal edges where the given entity is the effect.
    pub async fn get_causes(&self, entity_id: &str) -> GraphResult<Vec<CausalEdge>> {
        let entity_id = entity_id.to_owned();
        self.db
            .with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_id, cause_entity_id, effect_entity_id, relationship, confidence, session_id, created_at \
                     FROM kg_causal_edges WHERE effect_entity_id = ?1",
                )?;

                let edges = stmt
                    .query_map(params![entity_id], |row| {
                        Ok(CausalEdge {
                            id: row.get(0)?,
                            agent_id: row.get(1)?,
                            cause_entity_id: row.get(2)?,
                            effect_entity_id: row.get(3)?,
                            relationship: row.get(4)?,
                            confidence: row.get(5)?,
                            session_id: row.get(6)?,
                            created_at: row.get(7)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(edges)
            })
            .map_err(GraphError::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_services::VaultPaths;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup_test_db() -> (Arc<KnowledgeDatabase>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());

        // Seed the two entities required by the FK constraints on kg_causal_edges.
        // Use named columns since the full schema has 19 fields (most with defaults).
        db.with_connection(|conn| {
            conn.execute_batch(
                "INSERT INTO kg_entities (id, agent_id, entity_type, name, normalized_name, normalized_hash, first_seen_at, last_seen_at, mention_count)
                   VALUES ('e1', 'root', 'pattern', 'rate_limiting', 'rate_limiting', 'h1', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1);
                 INSERT INTO kg_entities (id, agent_id, entity_type, name, normalized_name, normalized_hash, first_seen_at, last_seen_at, mention_count)
                   VALUES ('e2', 'root', 'outcome', 'api_ban', 'api_ban', 'h2', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1);",
            )
        })
        .unwrap();

        (db, tmp)
    }

    fn make_edge(id: &str, session_id: Option<&str>) -> CausalEdge {
        CausalEdge {
            id: id.to_string(),
            agent_id: "root".to_string(),
            cause_entity_id: "e1".to_string(),
            effect_entity_id: "e2".to_string(),
            relationship: "prevents".to_string(),
            confidence: 0.9,
            session_id: session_id.map(String::from),
            created_at: "2026-04-11T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn test_store_and_get_effects() {
        let (db, _tmp) = setup_test_db();
        let store = CausalEdgeStore::new(db);

        let edge = make_edge("ce1", Some("sess-1"));
        store.store_edge(&edge).await.unwrap();

        let effects = store.get_effects("e1").await.unwrap();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].relationship, "prevents");
        assert_eq!(effects[0].effect_entity_id, "e2");
    }

    #[tokio::test]
    async fn test_get_causes() {
        let (db, _tmp) = setup_test_db();
        let store = CausalEdgeStore::new(db);

        let edge = make_edge("ce1", None);
        store.store_edge(&edge).await.unwrap();

        let causes = store.get_causes("e2").await.unwrap();
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0].cause_entity_id, "e1");
    }

    #[tokio::test]
    async fn test_duplicate_edge_ignored() {
        let (db, _tmp) = setup_test_db();
        let store = CausalEdgeStore::new(db);

        let edge = make_edge("ce1", None);
        store.store_edge(&edge).await.unwrap();
        // Duplicate insert should not fail
        store.store_edge(&edge).await.unwrap();

        let effects = store.get_effects("e1").await.unwrap();
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn test_no_edges_returns_empty() {
        let (db, _tmp) = setup_test_db();
        let store = CausalEdgeStore::new(db);

        let effects = store.get_effects("nonexistent").await.unwrap();
        assert!(effects.is_empty());
    }
}
