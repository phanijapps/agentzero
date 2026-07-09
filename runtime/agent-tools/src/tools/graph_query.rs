// ============================================================================
// GRAPH QUERY TOOL
// Search, explore neighbors, and get contextual subgraphs from the knowledge graph.
// ============================================================================

// Public API types — consumed by downstream crates (e.g., pi-mono) that wire
// a concrete GraphStorageAccess into the tool. No internal caller yet.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext};

// ============================================================================
// DATA TYPES
// ============================================================================

/// Rich entity payload — carries everything an agent needs to reason about an
/// entity without a follow-up query: the canonical id, the full free-form
/// `properties` blob (aliases, evidence, location refs, roles, dates — the
/// domain-specific content the ingest tool placed here), and temporal metadata.
///
/// The JSON serialization is what reaches the LLM. Fields map one-to-one onto
/// the `kg_entities` columns the storage layer already hydrates into
/// `knowledge_graph::Entity`.
#[derive(Debug, Clone, Serialize)]
pub struct EntityInfo {
    /// Canonical stable id — the dedup key. Same id across sources collapses
    /// to one node, so reading this tells the agent it's looking at THE
    /// Steve Jobs, not a second one.
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub mention_count: i64,
    /// Free-form property map — aliases, roles, descriptions, chunk-file
    /// pointers, evidence, dates, anything the ingest payload carried.
    pub properties: Value,
    /// ISO-8601 timestamps.
    pub first_seen_at: String,
    pub last_seen_at: String,
}

/// A neighbor entity with the edge connecting it. Carries the same rich
/// entity payload as [`EntityInfo`], plus the edge's own free-form properties
/// (evidence chunks per relationship, confidence, development timeline, etc.).
#[derive(Debug, Clone, Serialize)]
pub struct NeighborInfo {
    pub entity: EntityInfo,
    pub relationship_type: String,
    /// "outgoing" or "incoming"
    pub direction: String,
    /// Edge-level property map — evidence, confidence, direction, etc.
    pub rel_properties: Value,
    pub rel_first_seen_at: String,
    pub rel_last_seen_at: String,
}

// ============================================================================
// GRAPH STORAGE TRAIT
// ============================================================================

/// Abstraction over the knowledge-graph storage layer.
///
/// Implementors provide the actual DB queries; the tool formats the results.
#[async_trait]
pub trait GraphStorageAccess: Send + Sync + 'static {
    /// Find entities whose name matches `query` (LIKE / substring search).
    async fn search_entities_by_name(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: usize,
    ) -> std::result::Result<Vec<EntityInfo>, String>;

    /// Find entities whose name matches `query`, ranked by the requested view.
    ///
    /// `view` is one of `"semantic"`, `"temporal"`, `"entity"`, `"hybrid"`.
    /// Unknown values fall back to `"semantic"`.
    ///
    /// Default implementation delegates to [`search_entities_by_name`], so
    /// implementors only need to override for MAGMA-style views.
    async fn search_entities_with_view(
        &self,
        query: &str,
        entity_type: Option<&str>,
        _view: &str,
        limit: usize,
    ) -> std::result::Result<Vec<EntityInfo>, String> {
        self.search_entities_by_name(query, entity_type, limit)
            .await
    }

    /// Get entities connected to `entity_name`.
    /// `direction` is one of "outgoing", "incoming", or "both".
    async fn get_entity_neighbors(
        &self,
        entity_name: &str,
        direction: &str,
        limit: usize,
    ) -> std::result::Result<Vec<NeighborInfo>, String>;

    /// Look up a single entity by exact name.
    async fn get_entity_by_name(
        &self,
        name: &str,
    ) -> std::result::Result<Option<EntityInfo>, String>;
}

// ============================================================================
// TOOL
// ============================================================================

/// Tool that lets agents query the knowledge graph.
///
/// Supports three actions:
/// - `search` — find entities by name
/// - `neighbors` — explore connections from a given entity
/// - `context` — get entities and relationships relevant to a topic
pub struct GraphQueryTool {
    storage: Arc<dyn GraphStorageAccess>,
}

impl GraphQueryTool {
    pub fn new(storage: Arc<dyn GraphStorageAccess>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl Tool for GraphQueryTool {
    fn name(&self) -> &str {
        "graph_query"
    }

    fn description(&self) -> &str {
        "Query the knowledge graph: search entities, explore neighbors, or get contextual subgraphs."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "neighbors", "context"],
                    "description": "The query action to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search query string (for 'search' and 'context' actions)"
                },
                "entity_name": {
                    "type": "string",
                    "description": "Entity name (for 'neighbors' action)"
                },
                "entity_type": {
                    "type": "string",
                    "description": "Filter by entity type (for 'search' action)"
                },
                "direction": {
                    "type": "string",
                    "enum": ["outgoing", "incoming", "both"],
                    "default": "both",
                    "description": "Relationship direction (for 'neighbors' action)"
                },
                "depth": {
                    "type": "integer",
                    "default": 1,
                    "description": "Traversal depth 1-2 (for 'neighbors' action)"
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "description": "Maximum number of results"
                },
                "view": {
                    "type": "string",
                    "enum": ["semantic", "temporal", "entity", "hybrid"],
                    "description": "Query view: semantic (by similarity/mentions, default), temporal (most recent first), entity (most connected first), hybrid (reranked combination)"
                }
            },
            "required": ["action"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'action' parameter".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        match action {
            "search" => self.handle_search(&args, limit).await,
            "neighbors" => self.handle_neighbors(&args, limit).await,
            "context" => self.handle_context(&args, limit).await,
            other => Err(AgentError::Tool(format!(
                "Unknown action '{}'. Use 'search', 'neighbors', or 'context'.",
                other
            ))),
        }
    }
}

// ============================================================================
// ACTION HANDLERS
// ============================================================================

impl GraphQueryTool {
    async fn handle_search(&self, args: &Value, limit: usize) -> Result<Value> {
        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            AgentError::Tool("Missing 'query' parameter for search action".to_string())
        })?;

        let entity_type = args.get("entity_type").and_then(|v| v.as_str());
        let view = args
            .get("view")
            .and_then(|v| v.as_str())
            .unwrap_or("semantic");

        let entities = self
            .storage
            .search_entities_with_view(query, entity_type, view, limit)
            .await
            .map_err(|e| AgentError::Tool(format!("Graph search failed: {e}")))?;

        if entities.is_empty() {
            return Ok(json!({
                "summary": format!("No entities found matching \"{}\".", query),
                "count": 0,
                "entities": []
            }));
        }

        let mut md = format!("## Entities matching \"{}\"\n", query);
        for e in &entities {
            let _ = writeln!(
                md,
                "- **{}** [{}] ({}) — mentions={}",
                e.name, e.id, e.entity_type, e.mention_count
            );
        }

        // Structured payload carries the rich fields (id, properties, timestamps)
        // so the agent doesn't need a follow-up query to read aliases, chunk
        // pointers, evidence, etc. `summary` is the human-glance one-liner.
        Ok(json!({
            "summary": md,
            "count": entities.len(),
            "entities": entities,
        }))
    }

    async fn handle_neighbors(&self, args: &Value, limit: usize) -> Result<Value> {
        let entity_name = args
            .get("entity_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::Tool("Missing 'entity_name' parameter for neighbors action".to_string())
            })?;

        let direction = args
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("both");

        let depth = args
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .min(2) as usize;

        // Verify the entity exists first.
        let entity = self
            .storage
            .get_entity_by_name(entity_name)
            .await
            .map_err(|e| AgentError::Tool(format!("Graph lookup failed: {e}")))?;

        let Some(entity) = entity else {
            return Ok(json!({
                "summary": format!("Entity \"{}\" not found in the knowledge graph.", entity_name),
                "entity": null,
                "neighbors": [],
                "count": 0
            }));
        };

        let mut md = format!(
            "## Neighbors of \"{}\" [{}] ({})\n",
            entity.name, entity.id, entity.entity_type
        );

        // Depth-1 neighbors
        let neighbors = self
            .storage
            .get_entity_neighbors(entity_name, direction, limit)
            .await
            .map_err(|e| AgentError::Tool(format!("Graph neighbors failed: {e}")))?;

        if neighbors.is_empty() {
            let _ = writeln!(md, "No relationships found.");
            return Ok(json!({
                "summary": md,
                "entity": entity,
                "neighbors": [],
                "count": 0
            }));
        }

        Self::append_neighbors(&mut md, &neighbors, 1);

        // Depth-2: expand each depth-1 neighbor
        if depth >= 2 {
            self.expand_depth2(&mut md, &neighbors, direction, limit)
                .await?;
        }

        let count = neighbors.len();
        // Structured payload: the focal entity + every neighbor with full
        // properties, relationship properties, and timestamps. Agent can read
        // chunk_file pointers, evidence arrays, aliases directly.
        Ok(json!({
            "summary": md,
            "entity": entity,
            "neighbors": neighbors,
            "count": count
        }))
    }

    fn append_neighbors(md: &mut String, neighbors: &[NeighborInfo], depth: usize) {
        let prefix = if depth == 1 { "" } else { "  " };
        for n in neighbors {
            let arrow = if n.direction == "outgoing" {
                "-->"
            } else {
                "<--"
            };
            let _ = writeln!(
                md,
                "{}- {} {} **{}** ({}) via `{}`",
                prefix,
                n.entity.name,
                arrow,
                n.entity.name,
                n.entity.entity_type,
                n.relationship_type
            );
        }
    }

    async fn expand_depth2(
        &self,
        md: &mut String,
        neighbors: &[NeighborInfo],
        direction: &str,
        limit: usize,
    ) -> Result<()> {
        let _ = writeln!(md, "\n### Depth 2");
        for n in neighbors.iter().take(5) {
            let depth2 = self
                .storage
                .get_entity_neighbors(&n.entity.name, direction, limit.min(10))
                .await
                .map_err(|e| AgentError::Tool(format!("Depth-2 expansion failed: {e}")))?;
            if !depth2.is_empty() {
                let _ = writeln!(md, "#### From \"{}\"", n.entity.name);
                Self::append_neighbors(md, &depth2, 2);
            }
        }
        Ok(())
    }

    async fn handle_context(&self, args: &Value, limit: usize) -> Result<Value> {
        let topic = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            AgentError::Tool("Missing 'query' parameter for context action".to_string())
        })?;

        let view = args
            .get("view")
            .and_then(|v| v.as_str())
            .unwrap_or("semantic");

        // Search for entities matching the topic
        let entities = self
            .storage
            .search_entities_with_view(topic, None, view, limit)
            .await
            .map_err(|e| AgentError::Tool(format!("Context search failed: {e}")))?;

        if entities.is_empty() {
            return Ok(json!({
                "summary": format!("No context found for topic \"{}\".", topic),
                "count": 0,
                "entities": [],
                "relationships": []
            }));
        }

        let mut md = format!("## Context for \"{}\"\n\n### Entities\n", topic);
        for e in &entities {
            let _ = writeln!(
                md,
                "- **{}** [{}] ({}) — mentions={}",
                e.name, e.id, e.entity_type, e.mention_count
            );
        }

        // Gather relationships for found entities (limit expansion) and also
        // accumulate them structurally so the agent has chunk-file pointers,
        // evidence, etc., not just a markdown blurb.
        let _ = writeln!(md, "\n### Relationships");
        let mut rel_rows: Vec<NeighborInfo> = Vec::new();
        for e in entities.iter().take(5) {
            let neighbors = self
                .storage
                .get_entity_neighbors(&e.name, "both", 10)
                .await
                .map_err(|err| AgentError::Tool(format!("Context neighbors failed: {err}")))?;
            for n in &neighbors {
                let arrow = if n.direction == "outgoing" {
                    "-->"
                } else {
                    "<--"
                };
                let _ = writeln!(
                    md,
                    "- {} {} {} via `{}`",
                    e.name, arrow, n.entity.name, n.relationship_type
                );
            }
            rel_rows.extend(neighbors);
        }

        if rel_rows.is_empty() {
            let _ = writeln!(md, "No relationships found.");
        }

        Ok(json!({
            "summary": md,
            "count": entities.len(),
            "entities": entities,
            "relationships": rel_rows,
        }))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::{CallbackContext, Content, EventActions, ReadonlyContext, ToolContext};

    // --- Mock graph storage ---

    struct MockGraphStorage {
        entities: Vec<EntityInfo>,
        neighbors: Vec<NeighborInfo>,
    }

    impl MockGraphStorage {
        fn sample() -> Self {
            // Sample data carries realistic rich `properties` so tests verify
            // the tool preserves ingest-written content (aliases, chunk file
            // pointers, evidence) through the full pipeline.
            let now = "2026-04-16T12:00:00Z".to_string();
            let yfinance = EntityInfo {
                id: "module:yfinance".into(),
                name: "yfinance".into(),
                entity_type: "module".into(),
                mention_count: 5,
                properties: serde_json::json!({
                    "aliases": ["yf"],
                    "description": "Python library for stock data",
                }),
                first_seen_at: now.clone(),
                last_seen_at: now.clone(),
            };
            let rate_limiting = EntityInfo {
                id: "concept:rate-limiting".into(),
                name: "rate_limiting".into(),
                entity_type: "concept".into(),
                mention_count: 3,
                properties: serde_json::json!({}),
                first_seen_at: now.clone(),
                last_seen_at: now.clone(),
            };
            let pandas = EntityInfo {
                id: "module:pandas".into(),
                name: "pandas".into(),
                entity_type: "module".into(),
                mention_count: 8,
                properties: serde_json::json!({"aliases": ["pd"]}),
                first_seen_at: now.clone(),
                last_seen_at: now.clone(),
            };
            let entities = vec![yfinance.clone(), rate_limiting.clone(), pandas.clone()];
            let neighbors = vec![
                NeighborInfo {
                    entity: rate_limiting,
                    relationship_type: "requires".into(),
                    direction: "outgoing".into(),
                    rel_properties: serde_json::json!({"confidence": 0.9}),
                    rel_first_seen_at: now.clone(),
                    rel_last_seen_at: now.clone(),
                },
                NeighborInfo {
                    entity: pandas,
                    relationship_type: "depends_on".into(),
                    direction: "outgoing".into(),
                    rel_properties: serde_json::json!({"evidence": [{"chunk_file": "chunks/ch-01.json", "line": 42}]}),
                    rel_first_seen_at: now.clone(),
                    rel_last_seen_at: now.clone(),
                },
            ];
            Self {
                entities,
                neighbors,
            }
        }

        fn empty() -> Self {
            Self {
                entities: vec![],
                neighbors: vec![],
            }
        }
    }

    #[async_trait]
    impl GraphStorageAccess for MockGraphStorage {
        async fn search_entities_by_name(
            &self,
            query: &str,
            entity_type: Option<&str>,
            limit: usize,
        ) -> std::result::Result<Vec<EntityInfo>, String> {
            let query_lower = query.to_lowercase();
            Ok(self
                .entities
                .iter()
                .filter(|e| e.name.to_lowercase().contains(&query_lower))
                .filter(|e| entity_type.map(|t| e.entity_type == t).unwrap_or(true))
                .take(limit)
                .cloned()
                .collect())
        }

        async fn get_entity_neighbors(
            &self,
            _entity_name: &str,
            direction: &str,
            limit: usize,
        ) -> std::result::Result<Vec<NeighborInfo>, String> {
            Ok(self
                .neighbors
                .iter()
                .filter(|n| direction == "both" || n.direction == direction)
                .take(limit)
                .cloned()
                .collect())
        }

        async fn get_entity_by_name(
            &self,
            name: &str,
        ) -> std::result::Result<Option<EntityInfo>, String> {
            Ok(self.entities.iter().find(|e| e.name == name).cloned())
        }
    }

    // --- Mock tool context ---

    struct MockToolContext;

    impl ReadonlyContext for MockToolContext {
        fn invocation_id(&self) -> &str {
            "test"
        }
        fn agent_name(&self) -> &str {
            "test-agent"
        }
        fn user_id(&self) -> &str {
            "test"
        }
        fn app_name(&self) -> &str {
            "test"
        }
        fn session_id(&self) -> &str {
            "test-session"
        }
        fn branch(&self) -> &str {
            "test"
        }
        fn user_content(&self) -> &Content {
            use std::sync::LazyLock;
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for MockToolContext {
        fn get_state(&self, _key: &str) -> Option<Value> {
            None
        }
        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for MockToolContext {
        fn function_call_id(&self) -> String {
            "test-call".to_string()
        }
        fn actions(&self) -> EventActions {
            EventActions::default()
        }
        fn set_actions(&self, _actions: EventActions) {}
    }

    fn make_ctx() -> Arc<dyn ToolContext> {
        Arc::new(MockToolContext)
    }

    // --- Tests ---

    #[tokio::test]
    async fn search_finds_entities() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(make_ctx(), json!({"action": "search", "query": "yfinance"}))
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("yfinance"));
        assert!(text.contains("module"));
        assert_eq!(result["count"], 1);
    }

    #[tokio::test]
    async fn search_no_matches() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::empty()));
        let result = tool
            .execute(
                make_ctx(),
                json!({"action": "search", "query": "nonexistent"}),
            )
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("No entities found"));
    }

    #[tokio::test]
    async fn search_filters_by_type() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(
                make_ctx(),
                json!({"action": "search", "query": "a", "entity_type": "concept"}),
            )
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("rate_limiting"));
        assert!(!text.contains("yfinance"));
    }

    #[tokio::test]
    async fn neighbors_shows_connections() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(
                make_ctx(),
                json!({"action": "neighbors", "entity_name": "yfinance"}),
            )
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("rate_limiting"));
        assert!(text.contains("requires"));
        assert!(text.contains("-->"));
    }

    #[tokio::test]
    async fn neighbors_entity_not_found() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(
                make_ctx(),
                json!({"action": "neighbors", "entity_name": "nonexistent"}),
            )
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("not found"));
    }

    #[tokio::test]
    async fn context_returns_entities_and_relationships() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(make_ctx(), json!({"action": "context", "query": "finance"}))
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("Context for"));
        assert!(text.contains("yfinance"));
        assert!(text.contains("Relationships"));
    }

    #[tokio::test]
    async fn context_no_matches() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::empty()));
        let result = tool
            .execute(make_ctx(), json!({"action": "context", "query": "nothing"}))
            .await
            .unwrap();
        let text = result["summary"].as_str().unwrap();
        assert!(text.contains("No context found"));
    }

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(make_ctx(), json!({"action": "bad_action"}))
            .await;
        assert!(result.is_err());
    }

    /// The point of the richness fix: ingest writes `properties` with
    /// chunk-file pointers, aliases, evidence; the agent reads all of that
    /// back through graph_query without a follow-up call. This test proves
    /// the round-trip for both entity and neighbor/relationship properties.
    #[tokio::test]
    async fn search_payload_carries_rich_properties() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(make_ctx(), json!({"action": "search", "query": "yfinance"}))
            .await
            .unwrap();
        let entities = result["entities"].as_array().expect("entities array");
        assert_eq!(entities.len(), 1);
        let e = &entities[0];
        assert_eq!(e["id"], "module:yfinance");
        assert_eq!(e["name"], "yfinance");
        assert_eq!(e["properties"]["aliases"][0], "yf");
        assert_eq!(
            e["properties"]["description"],
            "Python library for stock data"
        );
        assert!(e["first_seen_at"].is_string());
    }

    #[tokio::test]
    async fn neighbors_payload_carries_edge_evidence() {
        let tool = GraphQueryTool::new(Arc::new(MockGraphStorage::sample()));
        let result = tool
            .execute(
                make_ctx(),
                json!({"action": "neighbors", "entity_name": "yfinance"}),
            )
            .await
            .unwrap();
        let ns = result["neighbors"].as_array().expect("neighbors array");
        // The sample mock returns both neighbors for any entity lookup.
        assert_eq!(ns.len(), 2);
        let pandas_edge = ns
            .iter()
            .find(|n| n["entity"]["id"] == "module:pandas")
            .expect("pandas neighbor present");
        assert_eq!(pandas_edge["relationship_type"], "depends_on");
        assert_eq!(
            pandas_edge["rel_properties"]["evidence"][0]["chunk_file"],
            "chunks/ch-01.json"
        );
        assert_eq!(pandas_edge["rel_properties"]["evidence"][0]["line"], 42);
        assert!(pandas_edge["rel_first_seen_at"].is_string());
    }
}
