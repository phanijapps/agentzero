//! Two-pass LLM extractor: pass 1 entities, pass 2 relationships conditioned
//! on the entity list. Concrete LLM-backed impl lands in Tasks 5 and 6.
//! `NoopExtractor` provides a test-friendly no-op for queue-level tests.

use crate::ingest::json_shape::parse_llm_json;
use agent_runtime::llm::{ChatMessage, LlmClient};
use async_trait::async_trait;
use gateway_services::ProviderService;
use knowledge_graph::{Entity, EntityType};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Deserialize)]
struct EntitiesEnvelope {
    entities: Vec<EntityItem>,
}

#[derive(Debug, Deserialize)]
struct EntityItem {
    name: Option<String>,
    #[serde(rename = "type")]
    type_str: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    #[allow(dead_code)]
    aliases: Option<Vec<String>>,
}

/// Maximum length for the summary property. LLMs sometimes emit long
/// descriptions; truncate to keep graph rows compact.
const SUMMARY_MAX_LEN: usize = 200;

#[derive(Debug, Deserialize)]
struct RelationshipsEnvelope {
    relationships: Vec<RelationshipItem>,
}

#[derive(Debug, Deserialize)]
struct RelationshipItem {
    source: Option<String>,
    target: Option<String>,
    #[serde(rename = "type")]
    type_str: Option<String>,
    evidence: Option<String>,
    weight: Option<f64>,
}

/// A parsed relationship candidate from the LLM, including optional
/// evidence + weight. Source/target names still need resolution to entity ids.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipTriple {
    pub source: String,
    pub target: String,
    pub type_str: String,
    pub evidence: Option<String>,
    pub weight: Option<f64>,
}

/// Processes one episode — runs extraction + writes to graph.
/// Errors propagate to the worker which marks the episode failed.
///
/// Phase B2: takes the trait-routed `kg_store` instead of the concrete
/// `Arc<GraphStorage>` so writes work on both SQLite (and any future alternate backend).
/// Episode metadata is passed by id only (the extractor needs only the
/// id for provenance — agent_id lives on the extractor itself).
#[async_trait]
pub trait Extractor: Send + Sync {
    async fn process(
        &self,
        episode_id: &str,
        chunk_text: &str,
        kg_store: &Arc<dyn zbot_stores::KnowledgeGraphStore>,
    ) -> Result<(), String>;
}

/// Test-only extractor: records each episode id and always succeeds.
pub struct NoopExtractor {
    pub seen: Mutex<Vec<String>>,
}

impl NoopExtractor {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Default for NoopExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Extractor for NoopExtractor {
    async fn process(
        &self,
        episode_id: &str,
        _chunk_text: &str,
        _kg_store: &Arc<dyn zbot_stores::KnowledgeGraphStore>,
    ) -> Result<(), String> {
        self.seen.lock().await.push(episode_id.to_string());
        Ok(())
    }
}

/// LLM-backed two-pass extractor. Pass 1: entities. Pass 2: relationships
/// (conditioned on the entity list).
pub struct LlmExtractor {
    provider_service: Arc<ProviderService>,
    agent_id: String,
}

impl LlmExtractor {
    pub fn new(provider_service: Arc<ProviderService>, agent_id: String) -> Self {
        Self {
            provider_service,
            agent_id,
        }
    }

    /// Build an LLM client from the current default provider. Mirrors
    /// `SessionDistiller::build_llm_client` so ingestion picks up provider
    /// changes without needing a restart.
    fn build_client(&self) -> Result<Arc<dyn LlmClient>, String> {
        let providers = self
            .provider_service
            .list()
            .map_err(|e| format!("list providers: {e}"))?;
        if providers.is_empty() {
            return Err("No LLM providers configured".to_string());
        }
        let provider = providers
            .iter()
            .find(|p| p.is_default)
            .or_else(|| providers.first())
            .ok_or_else(|| "No suitable provider".to_string())?;

        let model = provider.default_model().to_string();
        let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());
        let config = agent_runtime::llm::LlmConfig::new(
            provider.base_url.clone(),
            provider.api_key.clone(),
            model,
            provider_id,
        )
        .with_temperature(0.2)
        .with_max_tokens(4096);

        let client = agent_runtime::llm::openai::OpenAiClient::new(config)
            .map_err(|e| format!("build client: {e}"))?;
        Ok(Arc::new(client) as Arc<dyn LlmClient>)
    }

    async fn extract_entities(
        &self,
        client: &Arc<dyn LlmClient>,
        chunk_text: &str,
    ) -> Result<Vec<Entity>, String> {
        let system = "You extract named entities from text. \
            Return ONLY valid JSON matching the schema. \
            Do not wrap in code fences. Do not add commentary.";

        let user = format!(
            "Extract entities from this text. Output JSON: \
            {{\"entities\": [{{\"name\": string, \"type\": string, \"summary\": string, \"aliases\": [string], \"description\": string}}]}}\n\n\
            Valid types: person, organization, location, event, document, concept, tool, project, file, time_period, role, artifact, ward.\n\n\
            Field semantics:\n\
            - `summary` — REQUIRED one-sentence ground-truth description of what this entity IS (not what it does in this text). Max 200 chars. \
            For \"Microsoft Corporation\" → \"American multinational technology company headquartered in Redmond, Washington\". \
            For \"Chapter 3\" → \"Third chapter of {{book_name}}\". For a file entity → \"The {{format}} file at {{path}}\".\n\
            - `description` — free-form context about how the entity appeared in THIS text (optional).\n\n\
            TEXT:\n{chunk_text}"
        );

        let messages = vec![
            ChatMessage::system(system.to_string()),
            ChatMessage::user(user),
        ];

        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("llm entity pass failed: {e}"))?;

        parse_entities_response(&response.content, &self.agent_id)
    }

    async fn extract_relationships(
        &self,
        client: &Arc<dyn LlmClient>,
        chunk_text: &str,
        entity_names: &[String],
    ) -> Result<Vec<RelationshipTriple>, String> {
        if entity_names.len() < 2 {
            return Ok(Vec::new());
        }
        let system = "You extract relationships between entities. \
            Return ONLY valid JSON. Do not add commentary. \
            Every source and target MUST exactly match a name from the provided list. \
            For each relationship include a short `evidence` quote (verbatim when possible) \
            from the text supporting it, and a `weight` between 0.0 and 1.0 \
            (1.0 = explicitly stated, 0.5 = inferred).";
        let user = format!(
            "Given these entities: {}\n\n\
            Extract relationships between them from this text. \
            Output JSON: {{\"relationships\": [{{\"source\": string, \"target\": string, \"type\": string, \"evidence\": string, \"weight\": number}}]}}\n\n\
            TEXT:\n{chunk_text}",
            entity_names.join(", ")
        );
        let messages = vec![
            ChatMessage::system(system.to_string()),
            ChatMessage::user(user),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("llm rel pass failed: {e}"))?;

        parse_relationships_response(&response.content, entity_names)
    }
}

#[async_trait]
impl Extractor for LlmExtractor {
    async fn process(
        &self,
        episode_id: &str,
        chunk_text: &str,
        kg_store: &Arc<dyn zbot_stores::KnowledgeGraphStore>,
    ) -> Result<(), String> {
        if chunk_text.trim().is_empty() {
            return Ok(());
        }

        let client = self.build_client()?;

        let mut entities = self.extract_entities(&client, chunk_text).await?;
        if entities.is_empty() {
            return Ok(());
        }
        let entity_names: Vec<String> = entities.iter().map(|e| e.name.clone()).collect();

        let rel_tuples = self
            .extract_relationships(&client, chunk_text, &entity_names)
            .await?;

        // Build Relationship structs using the candidate entity ids.
        // store_knowledge remaps them to canonical ids via resolver merges.
        let mut candidate_rels = Vec::new();
        for triple in rel_tuples {
            let src_id = entities
                .iter()
                .find(|e| e.name == triple.source)
                .map(|e| e.id.clone());
            let tgt_id = entities
                .iter()
                .find(|e| e.name == triple.target)
                .map(|e| e.id.clone());
            let (Some(src_id), Some(tgt_id)) = (src_id, tgt_id) else {
                continue;
            };
            let mut rel = knowledge_graph::Relationship::new(
                self.agent_id.clone(),
                src_id,
                tgt_id,
                knowledge_graph::RelationshipType::from_str(&triple.type_str),
            );
            if let Some(ev) = triple.evidence {
                let trimmed = ev.trim();
                if !trimmed.is_empty() {
                    rel.properties.insert(
                        "evidence".to_string(),
                        serde_json::Value::String(trimmed.to_string()),
                    );
                }
            }
            if let Some(w) = triple.weight {
                rel.properties
                    .insert("weight".to_string(), serde_json::json!(w));
            }
            rel.properties.insert(
                "extracted_by".to_string(),
                serde_json::Value::String("ingest-llm-extractor".to_string()),
            );
            rel.properties.insert(
                "source_episode_id".to_string(),
                serde_json::Value::String(episode_id.to_string()),
            );
            candidate_rels.push(rel);
        }

        // Tag every entity with provenance.
        for e in &mut entities {
            e.properties.insert(
                "_source_episode_id".to_string(),
                serde_json::Value::String(episode_id.to_string()),
            );
        }

        // Phase B2: write through the trait surface so SurrealDB
        // is honored. The trait wants `zbot_stores::ExtractedKnowledge`;
        // the local `knowledge_graph::ExtractedKnowledge` converts via
        // the `From` impl in zbot-stores.
        let extracted = zbot_stores::ExtractedKnowledge {
            entities,
            relationships: candidate_rels,
        };
        kg_store
            .store_knowledge(&self.agent_id, extracted)
            .await
            .map_err(|e| format!("store_knowledge: {e}"))?;
        Ok(())
    }
}

fn parse_relationships_response(
    content: &str,
    known_entities: &[String],
) -> Result<Vec<RelationshipTriple>, String> {
    let env: RelationshipsEnvelope = parse_llm_json(content)?;
    let known: std::collections::HashSet<&str> =
        known_entities.iter().map(|s| s.as_str()).collect();
    let mut out = Vec::new();
    for item in env.relationships {
        let src = item.source.as_deref().map(str::trim).unwrap_or("");
        let tgt = item.target.as_deref().map(str::trim).unwrap_or("");
        let ty = item.type_str.as_deref().map(str::trim).unwrap_or("");
        if src.is_empty() || tgt.is_empty() || ty.is_empty() {
            continue;
        }
        if !known.contains(src) || !known.contains(tgt) {
            continue; // drop hallucinated references to entities not in pass-1
        }
        let evidence = item
            .evidence
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let weight = item.weight.map(|w| w.clamp(0.0, 1.0));
        out.push(RelationshipTriple {
            source: src.to_string(),
            target: tgt.to_string(),
            type_str: ty.to_string(),
            evidence,
            weight,
        });
    }
    Ok(out)
}

/// Truncate `s` to at most `max` characters (not bytes). Preserves Unicode boundaries.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Parse the LLM's JSON response into typed Entity values.
/// Strips optional code-fence wrapping. Silently skips malformed items.
fn parse_entities_response(content: &str, agent_id: &str) -> Result<Vec<Entity>, String> {
    let env: EntitiesEnvelope = parse_llm_json(content)?;
    let mut out = Vec::new();
    for item in env.entities {
        let Some(name) = item
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let type_str = item.type_str.as_deref().unwrap_or("concept");
        let ty = EntityType::from_str(type_str);
        let mut entity = Entity::new(agent_id.to_string(), ty, name.to_string());

        let description = item
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(ref desc) = description {
            entity.properties.insert(
                "description".to_string(),
                serde_json::Value::String(desc.clone()),
            );
        }

        // Summary is universally required. Prefer the LLM value; fall back
        // to description; otherwise synthesize from type+name and flag it.
        let summary = item
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let (summary_text, synthesized) = match summary {
            Some(s) => (s, false),
            None => match description.clone() {
                Some(d) => (d, false),
                None => (format!("{type_str} {name} (auto-summary)"), true),
            },
        };
        let summary_text = truncate_chars(&summary_text, SUMMARY_MAX_LEN);
        entity.properties.insert(
            "summary".to_string(),
            serde_json::Value::String(summary_text),
        );
        if synthesized {
            entity.properties.insert(
                "summary_synthesized".to_string(),
                serde_json::Value::Bool(true),
            );
        }

        out.push(entity);
    }
    Ok(out)
}

#[cfg(test)]
mod tests_extractor {
    use super::*;

    #[test]
    fn parses_entities_from_clean_json() {
        let json = r#"{"entities": [
            {"name": "Alice", "type": "person", "aliases": [], "description": "A character"},
            {"name": "Wonderland", "type": "location", "aliases": ["Land of Wonder"]}
        ]}"#;
        let entities = parse_entities_response(json, "root").unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].name, "Alice");
        assert!(matches!(entities[0].entity_type, EntityType::Person));
    }

    #[test]
    fn strips_code_fence_wrapping() {
        let json = "```json\n{\"entities\": [{\"name\": \"X\", \"type\": \"concept\"}]}\n```";
        let entities = parse_entities_response(json, "root").unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "X");
    }

    #[test]
    fn skips_empty_names() {
        let json =
            r#"{"entities": [{"name": "", "type": "person"}, {"name": "Ok", "type": "person"}]}"#;
        let entities = parse_entities_response(json, "root").unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "Ok");
    }

    #[test]
    fn missing_entities_array_errors() {
        let json = r#"{"nope": []}"#;
        assert!(parse_entities_response(json, "root").is_err());
    }

    #[test]
    fn parse_rejects_malformed_json() {
        let result = parse_entities_response("not valid json at all", "root");
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_entities_array_ok() {
        let result =
            parse_entities_response(r#"{"entities": []}"#, "root").expect("valid but empty");
        assert!(result.is_empty());
    }

    #[test]
    fn parses_relationships_and_drops_hallucinated_refs() {
        let json = r#"{"relationships": [
            {"source": "Alice", "target": "Wonderland", "type": "located_in"},
            {"source": "Ghost", "target": "Alice", "type": "haunts"}
        ]}"#;
        let rels =
            parse_relationships_response(json, &["Alice".into(), "Wonderland".into()]).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source, "Alice");
        assert_eq!(rels[0].type_str, "located_in");
        assert!(rels[0].evidence.is_none());
        assert!(rels[0].weight.is_none());
    }

    #[test]
    fn parses_relationships_with_evidence_and_weight() {
        let json = r#"{"relationships": [
            {"source": "Alice", "target": "Wonderland", "type": "located_in",
             "evidence": "Alice fell down the rabbit hole into Wonderland.", "weight": 0.95}
        ]}"#;
        let rels =
            parse_relationships_response(json, &["Alice".into(), "Wonderland".into()]).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(
            rels[0].evidence.as_deref(),
            Some("Alice fell down the rabbit hole into Wonderland.")
        );
        assert_eq!(rels[0].weight, Some(0.95));
    }

    #[test]
    fn clamps_out_of_range_weight() {
        let json = r#"{"relationships": [
            {"source": "A", "target": "B", "type": "uses", "weight": 2.5},
            {"source": "A", "target": "B", "type": "uses", "weight": -0.3}
        ]}"#;
        let rels = parse_relationships_response(json, &["A".into(), "B".into()]).unwrap();
        assert_eq!(rels[0].weight, Some(1.0));
        assert_eq!(rels[1].weight, Some(0.0));
    }

    #[test]
    fn blank_evidence_becomes_none() {
        let json = r#"{"relationships": [
            {"source": "A", "target": "B", "type": "uses", "evidence": "   "}
        ]}"#;
        let rels = parse_relationships_response(json, &["A".into(), "B".into()]).unwrap();
        assert!(rels[0].evidence.is_none());
    }

    #[test]
    fn summary_lands_in_properties_when_provided() {
        let json = r#"{"entities": [
            {"name": "Microsoft", "type": "organization",
             "summary": "American multinational technology company headquartered in Redmond, Washington",
             "description": "Mentioned as the employer of the user"}
        ]}"#;
        let entities = parse_entities_response(json, "root").unwrap();
        assert_eq!(entities.len(), 1);
        let summary = entities[0]
            .properties
            .get("summary")
            .and_then(|v| v.as_str())
            .expect("summary must be set");
        assert!(summary.contains("Redmond"));
        // Not synthesized, so flag should be absent.
        assert!(!entities[0].properties.contains_key("summary_synthesized"));
    }

    #[test]
    fn summary_falls_back_to_description_when_omitted() {
        let json = r#"{"entities": [
            {"name": "Chapter 3", "type": "document",
             "description": "Third chapter of the style guide"}
        ]}"#;
        let entities = parse_entities_response(json, "root").unwrap();
        assert_eq!(entities.len(), 1);
        let summary = entities[0]
            .properties
            .get("summary")
            .and_then(|v| v.as_str())
            .expect("summary must be synthesized from description");
        assert_eq!(summary, "Third chapter of the style guide");
        assert!(!entities[0].properties.contains_key("summary_synthesized"));
    }

    #[test]
    fn summary_synthesized_from_type_and_name_when_missing() {
        let json = r#"{"entities": [{"name": "Widget", "type": "tool"}]}"#;
        let entities = parse_entities_response(json, "root").unwrap();
        assert_eq!(entities.len(), 1);
        let summary = entities[0]
            .properties
            .get("summary")
            .and_then(|v| v.as_str())
            .expect("summary always present");
        assert!(!summary.is_empty());
        assert!(summary.contains("Widget"));
        assert!(summary.contains("tool"));
        assert_eq!(
            entities[0].properties.get("summary_synthesized"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn summary_truncated_to_max_length() {
        let long = "x".repeat(400);
        let json =
            format!(r#"{{"entities": [{{"name": "E", "type": "concept", "summary": "{long}"}}]}}"#);
        let entities = parse_entities_response(&json, "root").unwrap();
        let summary = entities[0]
            .properties
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(summary.chars().count(), SUMMARY_MAX_LEN);
    }

    #[test]
    fn blank_summary_triggers_fallback() {
        let json = r#"{"entities": [{"name": "E", "type": "concept", "summary": "   "}]}"#;
        let entities = parse_entities_response(json, "root").unwrap();
        let summary = entities[0]
            .properties
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(!summary.trim().is_empty());
        assert_eq!(
            entities[0].properties.get("summary_synthesized"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn empty_entity_list_returns_no_relationships_call() {
        // Parse directly; the guard in extract_relationships (network path) is
        // tested implicitly. Just confirm parser handles empty known list.
        let json = r#"{"relationships": [{"source": "A", "target": "B", "type": "x"}]}"#;
        let rels = parse_relationships_response(json, &[]).unwrap();
        assert!(rels.is_empty());
    }
}
