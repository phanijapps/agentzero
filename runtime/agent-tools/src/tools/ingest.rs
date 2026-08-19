// ============================================================================
// INGEST TOOL
// Bulk-structured graph writes + text ingest in a single polymorphic tool.
// ============================================================================

// Public API types — consumed by downstream (gateway) that wires a concrete
// IngestionAccess into the tool. No in-crate caller yet.
#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext};

// ---------------------------------------------------------------------------
// Public shapes
// ---------------------------------------------------------------------------

/// Entity shape accepted by the structured path. `type` uses the governed
/// built-in vocabulary; `properties` may carry free-form application metadata,
/// while scope and governance controls remain host-managed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredEntity {
    /// Stable slug — the dedup key. Reusing the same id across sources MERGES
    /// properties into one node (evidence arrays concatenate).
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    #[serde(default)]
    pub properties: serde_json::Map<String, Value>,
}

/// Generic relationship shape. `from` and `to` reference entity ids — either
/// ids listed in the same payload's `entities` array or existing graph ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredRelationship {
    #[serde(rename = "type")]
    pub rel_type: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub properties: serde_json::Map<String, Value>,
}

/// Counts returned from a structured ingest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredCounts {
    pub entities_upserted: usize,
    pub relationships_upserted: usize,
}

/// Internal evidence-intake record for durable memory/knowledge writes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub action: String,
    pub source_id: String,
    pub source_type: String,
    pub session_id: Option<String>,
    /// Active ward supplied by the trusted execution context when available.
    /// The adapter uses it to select the configured governance overlay.
    #[serde(default)]
    pub ward_id: Option<String>,
    pub agent_id: String,
    pub retention_policy: String,
    #[serde(default)]
    pub ontology_labels: Vec<String>,
    #[serde(default)]
    pub taxonomy_labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// Backend abstraction
// ---------------------------------------------------------------------------

/// Abstraction over the gateway's ingestion pipeline. Implementations bridge
/// to the episode repository (for text) and the graph storage (for structured).
#[async_trait]
pub trait IngestionAccess: Send + Sync + 'static {
    /// Record one durable evidence-intake boundary before semantic writes.
    async fn record_evidence(&self, _record: EvidenceRecord) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Chunk `text`, create one episode per chunk, and notify workers.
    /// Returns `(source_id, episode_count)`.
    async fn enqueue(
        &self,
        source_id: &str,
        source_type: &str,
        text: &str,
        session_id: Option<&str>,
        agent_id: &str,
    ) -> std::result::Result<(String, usize), String>;

    /// Bulk-upsert structured entities and relationships. No LLM extraction.
    /// The agent is responsible for the shape; backend merges into existing
    /// rows by id (entities) and by (source, target, type) (relationships).
    async fn ingest_structured(
        &self,
        agent_id: &str,
        ward_id: Option<String>,
        entities: Vec<StructuredEntity>,
        relationships: Vec<StructuredRelationship>,
    ) -> std::result::Result<StructuredCounts, String>;
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

/// Polymorphic ingest tool: text (LLM extracts asynchronously), structured
/// (direct bulk write), or both in the same call.
pub struct IngestTool {
    access: Arc<dyn IngestionAccess>,
}

impl IngestTool {
    pub fn new(access: Arc<dyn IngestionAccess>) -> Self {
        Self { access }
    }
}

#[async_trait]
impl Tool for IngestTool {
    fn name(&self) -> &str {
        "ingest"
    }

    fn description(&self) -> &str {
        "Write to the knowledge graph. Single call can mix three modes:\n\
         \n\
         - `text`: prose; a background LLM extracts entities/relationships asynchronously.\n\
         - `entities[]`: typed nodes bulk-upserted synchronously (no LLM).\n\
         - `relationships[]`: typed edges bulk-upserted synchronously.\n\
         \n\
         Prefer structured over text when you already have entity-shaped data \
         (from a tool result, a file, or your own analysis). Use text only for raw prose.\n\
         \n\
         Entity = {id, name, type, properties?}. Use stable slug ids: \
         '<type>:<kebab-name>' e.g. 'person:steve-jobs', 'organization:apple-inc', \
         'stock:aapl'. Same id across sources MERGES properties into one node \
         (keys union; arrays inside properties concatenate without duplicates — \
         so `evidence` accumulates across ingests). Entity types must use the \
         built-in vocabulary: person, organization, location, concept, tool, project, \
         file, event, time_period, document, role, artifact, or ward.\n\
         \n\
         Relationship = {type, from, to, properties?}. `from`/`to` reference \
         entity ids from this payload or already in the graph. Relationship types \
         must use the built-in vocabulary documented in the parameter schema. \
         Same (from,to,type) triple \
         across ingests merges properties the same way entities do.\n\
         \n\
         Example:\n\
         {\"entities\":[{\"id\":\"person:steve-jobs\",\"name\":\"Steve Jobs\",\"type\":\"person\"},\
         {\"id\":\"organization:apple\",\"name\":\"Apple Inc.\",\"type\":\"organization\",\
         \"properties\":{\"founded\":\"1976\"}}],\
         \"relationships\":[{\"type\":\"founder_of\",\"from\":\"person:steve-jobs\",\
         \"to\":\"organization:apple\",\"properties\":{\"evidence\":[{\"chunk\":\"bio/ch-05.md\",\"line\":123}],\
         \"notes\":\"source-confirmed\"}}]}\n\
         \n\
         Returns counts of entities/relationships upserted and text chunks enqueued."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "source_id": {
                    "type": "string",
                    "description": "Provenance tag for the text path (e.g., 'book-jobs-bio', 'earnings-aapl-2024q1'). Used only when `text` is supplied."
                },
                "source_type": {
                    "type": "string",
                    "description": "Free-form category for the text path: 'book', 'paper', 'earnings_call', 'article', 'transcript'. Defaults to 'document'.",
                    "default": "document"
                },
                "text": {
                    "type": "string",
                    "description": "Raw prose to enqueue for async LLM extraction. Leave empty when you already have structured entities/relationships — prefer those."
                },
                "entities": {
                    "type": "array",
                    "description": "Typed graph nodes. Written synchronously on return. Each entity MERGES into an existing row with the same `id` (properties key-union; arrays inside properties concatenate).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id":   {
                                "type": "string",
                                "description": "Stable slug. Convention: '<type>:<kebab-name>'. Examples: 'person:steve-jobs', 'organization:apple-inc', 'stock:aapl', 'concept:quantum-entanglement'. Reuse the SAME id across sources so the same real-world entity collapses to one node."
                            },
                            "name": {
                                "type": "string",
                                "description": "Human-readable surface form: 'Steve Jobs', 'Apple Inc.', 'AAPL'. Variants get recorded as aliases."
                            },
                            "type": {
                                "type": "string",
                                "description": "Built-in category: person, organization, location, concept, tool, project, file, event, time_period, document, role, artifact, or ward. Other values are rejected."
                            },
                            "properties": {
                                "type": "object",
                                "description": "Application metadata such as aliases, description, evidence, chapter, founded, ticker, or doi. Scope and governance controls (including ward_id, ontology/taxonomy selectors, epistemic/hierarchy fields, and confidence) are host-managed and ignored when supplied here."
                            }
                        },
                        "required": ["id", "name", "type"]
                    }
                },
                "relationships": {
                    "type": "array",
                    "description": "Typed edges. Written synchronously. Merges on the (from, to, type) triple — same triple across sources concatenates evidence.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "description": "Built-in predicate: works_for, located_in, related_to, created, uses, part_of, mentions, before, after, during, concurrent_with, succeeded_by, preceded_by, president_of, founder_of, member_of, author_of, held_role, employed_by, held_at, born_in, died_in, caused, enabled, prevented, triggered_by, contains, instance_of, or subtype_of. Other values are rejected."
                            },
                            "from": {
                                "type": "string",
                                "description": "Entity id of the source. Resolves against entities in THIS payload first, then against the existing graph."
                            },
                            "to": {
                                "type": "string",
                                "description": "Entity id of the target. Same resolution as `from`."
                            },
                            "properties": {
                                "type": "object",
                                "description": "Application metadata such as evidence, direction, date_range, or notes. Scope and governance controls (including ward_id, ontology/taxonomy selectors, epistemic/hierarchy fields, and confidence) are host-managed and ignored when supplied here."
                            }
                        },
                        "required": ["type", "from", "to"]
                    }
                },
                "retention_policy": {
                    "type": "string",
                    "description": "Durable evidence retention policy selected by the host. Defaults to 'durable'.",
                    "default": "durable"
                },
                "ontology_labels": {
                    "type": "array",
                    "description": "Optional zbot-selected dynamic ontology labels to attach to this evidence intake.",
                    "items": {"type": "string"}
                },
                "taxonomy_labels": {
                    "type": "array",
                    "description": "Optional zbot-selected SKOS/taxonomy labels to attach to this evidence intake.",
                    "items": {"type": "string"}
                }
            }
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let agent_id = ctx.agent_name().to_string();

        let entities: Vec<StructuredEntity> = args
            .get("entities")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let relationships: Vec<StructuredRelationship> = args
            .get("relationships")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");

        if entities.is_empty() && relationships.is_empty() && text.is_empty() {
            return Err(AgentError::Tool(
                "ingest requires at least one of: entities, relationships, or text".into(),
            ));
        }

        let source_id = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .unwrap_or("agent-ingest");
        let source_type = args
            .get("source_type")
            .and_then(|v| v.as_str())
            .unwrap_or("document");
        let session_id = ctx.session_id().to_string();
        let session_id_opt = if session_id.is_empty() {
            None
        } else {
            Some(session_id.as_str())
        };
        let ward_id = ctx.get_state("ward_id").and_then(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|ward_id| !ward_id.is_empty())
                .map(str::to_string)
        });
        let ontology_labels = string_array_arg(&args, "ontology_labels");
        let taxonomy_labels = string_array_arg(&args, "taxonomy_labels");
        let retention_policy = args
            .get("retention_policy")
            .and_then(|v| v.as_str())
            .unwrap_or("durable")
            .to_string();

        let evidence = EvidenceRecord {
            evidence_id: format!("{}:{}", agent_id, source_id),
            action: "ingest".to_string(),
            source_id: source_id.to_string(),
            source_type: source_type.to_string(),
            session_id: session_id_opt.map(str::to_string),
            ward_id,
            agent_id: agent_id.clone(),
            retention_policy,
            ontology_labels,
            taxonomy_labels,
        };
        self.access
            .record_evidence(evidence.clone())
            .await
            .map_err(AgentError::Tool)?;

        // Structured path — synchronous bulk write, runs first so relationships
        // whose `from`/`to` reference entities in the same call see them.
        let counts = if !entities.is_empty() || !relationships.is_empty() {
            self.access
                .ingest_structured(&agent_id, evidence.ward_id.clone(), entities, relationships)
                .await
                .map_err(AgentError::Tool)?
        } else {
            StructuredCounts {
                entities_upserted: 0,
                relationships_upserted: 0,
            }
        };

        // Text path — async background extraction (unchanged behavior).
        let (resolved_source, chunk_count) = if !text.is_empty() {
            self.access
                .enqueue(source_id, source_type, text, session_id_opt, &agent_id)
                .await
                .map_err(AgentError::Tool)?
        } else {
            (String::new(), 0)
        };

        Ok(json!({
            "entities_upserted": counts.entities_upserted,
            "relationships_upserted": counts.relationships_upserted,
            "text_chunks_enqueued": chunk_count,
            "source_id": resolved_source,
            "evidence": evidence,
            "status": "ok",
        }))
    }
}

fn string_array_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::{
        CallbackContext, EventActions, ReadonlyContext, ToolContext, types::Content,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockIngestion {
        records: Mutex<Vec<EvidenceRecord>>,
        structured_wards: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl IngestionAccess for MockIngestion {
        async fn record_evidence(&self, record: EvidenceRecord) -> std::result::Result<(), String> {
            self.records.lock().unwrap().push(record);
            Ok(())
        }

        async fn enqueue(
            &self,
            source_id: &str,
            _source_type: &str,
            _text: &str,
            _session_id: Option<&str>,
            _agent_id: &str,
        ) -> std::result::Result<(String, usize), String> {
            Ok((source_id.to_string(), 1))
        }

        async fn ingest_structured(
            &self,
            _agent_id: &str,
            ward_id: Option<String>,
            entities: Vec<StructuredEntity>,
            relationships: Vec<StructuredRelationship>,
        ) -> std::result::Result<StructuredCounts, String> {
            self.structured_wards.lock().unwrap().push(ward_id);
            Ok(StructuredCounts {
                entities_upserted: entities.len(),
                relationships_upserted: relationships.len(),
            })
        }
    }

    struct MockContext;

    impl ReadonlyContext for MockContext {
        fn invocation_id(&self) -> &str {
            "invoke"
        }
        fn agent_name(&self) -> &str {
            "root"
        }
        fn user_id(&self) -> &str {
            "user"
        }
        fn app_name(&self) -> &str {
            "zbot"
        }
        fn session_id(&self) -> &str {
            "sess-1"
        }
        fn branch(&self) -> &str {
            "main"
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

    impl CallbackContext for MockContext {
        fn get_state(&self, key: &str) -> Option<Value> {
            (key == "ward_id").then(|| json!("trusted-ward"))
        }

        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for MockContext {
        fn function_call_id(&self) -> String {
            "call-1".to_string()
        }

        fn actions(&self) -> EventActions {
            EventActions::default()
        }

        fn set_actions(&self, _actions: EventActions) {}
    }

    #[tokio::test]
    async fn ingest_records_evidence_with_ontology_and_taxonomy_policy() {
        let access = Arc::new(MockIngestion::default());
        let tool = IngestTool::new(access.clone());

        let result = tool
            .execute(
                Arc::new(MockContext),
                json!({
                    "source_id": "sec-10k",
                    "source_type": "filing",
                    "text": "Apple reports revenue.",
                    "retention_policy": "durable",
                    "ontology_labels": ["company", "financial_statement"],
                    "taxonomy_labels": ["skos:finance"]
                }),
            )
            .await
            .expect("ingest ok");

        assert_eq!(result["status"], "ok");
        let records = access.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source_id, "sec-10k");
        assert_eq!(records[0].session_id.as_deref(), Some("sess-1"));
        assert_eq!(
            records[0].ontology_labels,
            vec!["company", "financial_statement"]
        );
        assert_eq!(records[0].taxonomy_labels, vec!["skos:finance"]);
    }

    #[tokio::test]
    async fn empty_ingest_does_not_record_evidence() {
        let access = Arc::new(MockIngestion::default());
        let tool = IngestTool::new(access.clone());

        let err = tool
            .execute(Arc::new(MockContext), json!({}))
            .await
            .expect_err("empty ingest must fail");

        assert!(format!("{err}").contains("requires at least one"));
        assert!(access.records.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn structured_ingest_forwards_ward_from_trusted_tool_context() {
        let access = Arc::new(MockIngestion::default());
        let tool = IngestTool::new(access.clone());

        let result = tool
            .execute(
                Arc::new(MockContext),
                json!({
                    "entities": [{
                        "id": "concept:trusted",
                        "name": "Trusted",
                        "type": "concept",
                        "properties": {"ward_id": "attacker-ward"}
                    }]
                }),
            )
            .await
            .expect("structured ingest ok");

        assert_eq!(result["evidence"]["ward_id"], json!("trusted-ward"));
        assert_eq!(
            access.structured_wards.lock().unwrap().as_slice(),
            &[Some("trusted-ward".to_string())]
        );
    }
}
