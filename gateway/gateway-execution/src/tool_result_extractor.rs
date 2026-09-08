//! ToolResultExtractor — parses structured tool outputs and emits entity
//! extractions in real time (post-tool-result, pre-next-iteration).
//!
//! Unlike distillation (end-of-session, LLM-based), this runs during
//! execution with zero LLM cost using schema-aware parsers per tool.
//!
//! Each extraction produces an `Episode` with `source_type = tool_result`
//! and `source_ref = tool_call_id`, enabling drill-down from graph to
//! the exact tool invocation that produced it.

use crate::errors::ExecutionError;
use agent_tools::{EvidenceRecord, IngestionAccess};
use knowledge_graph::{Entity, EntityType};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use zbot_stores::{ExtractedKnowledge, KnowledgeGraphStore};
use zbot_stores_domain::EpisodeSource;
use zbot_stores_traits::KgEpisodeStore;

/// Extract entities from a tool result and persist them with episode provenance.
///
/// Dispatches to a tool-specific parser. Tool outputs that don't match any
/// known schema produce zero extractions (silent no-op).
///
/// Errors are logged at warn level and never propagate — extraction is
/// best-effort and must never block the execution loop.
pub struct ExtractAndPersistRequest<'a> {
    pub tool_name: &'a str,
    pub tool_call_id: &'a str,
    pub result_text: &'a str,
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub evidence_intake: Option<&'a dyn IngestionAccess>,
    pub episode_store: &'a dyn KgEpisodeStore,
    pub kg: &'a dyn KnowledgeGraphStore,
}

pub async fn extract_and_persist(request: ExtractAndPersistRequest<'_>) {
    let entities = extract_from_tool(request.tool_name, request.result_text);
    if entities.is_empty() {
        return;
    }

    if let Some(intake) = request.evidence_intake {
        let record = tool_result_evidence_record(
            request.tool_name,
            request.tool_call_id,
            request.session_id,
            request.agent_id,
        );
        if let Err(e) = intake.record_evidence(record).await {
            tracing::warn!(tool = %request.tool_name, error = %e, "Failed to record tool-result evidence intake");
            return;
        }
    }

    let episode_id = match ensure_episode(
        request.episode_store,
        request.tool_call_id,
        request.result_text,
        request.session_id,
        request.agent_id,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(tool = %request.tool_name, error = %e, "Failed to create tool-result episode");
            return;
        }
    };

    let stamped = entities
        .into_iter()
        .map(|e| stamp_provenance(e, &episode_id, request.tool_call_id))
        .collect::<Vec<_>>();

    let knowledge = ExtractedKnowledge {
        entities: stamped,
        relationships: Vec::new(),
    };
    if let Err(e) = request
        .kg
        .store_knowledge(request.agent_id, knowledge)
        .await
    {
        tracing::warn!(tool = %request.tool_name, error = %e, "Failed to persist tool-result entities");
    }
}

/// Dispatch table: route tool name to its specific extractor.
fn extract_from_tool(tool_name: &str, result_text: &str) -> Vec<Entity> {
    // Parse JSON envelope once if possible (tools typically return JSON strings)
    let parsed = serde_json::from_str::<Value>(result_text).ok();

    match tool_name {
        "web_fetch" | "web-fetch" | "webfetch" => extract_web_fetch(parsed.as_ref()),
        "shell" => extract_shell(parsed.as_ref()),
        "multimodal_analyze" | "multimodal" => extract_multimodal(parsed.as_ref()),
        _ => Vec::new(),
    }
}

/// Insert a property on an entity if the named key is present as a string.
fn insert_str_prop(entity: &mut Entity, obj: &Map<String, Value>, src_key: &str, dst_key: &str) {
    if let Some(v) = obj.get(src_key).and_then(|v| v.as_str()) {
        entity
            .properties
            .insert(dst_key.to_string(), Value::String(v.to_string()));
    }
}

/// Extract entities from a web_fetch result.
/// Known fields: url, title, description, content, publish_date.
fn extract_web_fetch(parsed: Option<&Value>) -> Vec<Entity> {
    let Some(obj) = parsed.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let Some(url) = obj.get("url").and_then(|v| v.as_str()) else {
        return Vec::new();
    };

    let mut document = Entity::new(
        "__global__".to_string(),
        EntityType::Document,
        url.to_string(),
    );
    insert_str_prop(&mut document, obj, "title", "title");
    insert_str_prop(&mut document, obj, "description", "description");
    // Accept publish_date or publishDate
    if let Some(date) = obj
        .get("publish_date")
        .or_else(|| obj.get("publishDate"))
        .and_then(|v| v.as_str())
    {
        document.properties.insert(
            "publication_date".to_string(),
            Value::String(date.to_string()),
        );
    }
    vec![document]
}

/// Extract from shell tool output: currently just captures file paths mentioned
/// in stdout (useful for search/find/ls outputs) as File entities.
fn extract_shell(parsed: Option<&Value>) -> Vec<Entity> {
    let Some(obj) = parsed.and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    // Only extract when the shell command succeeded
    let success = obj
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !success {
        return Vec::new();
    }

    let stdout = obj.get("stdout").and_then(|v| v.as_str()).unwrap_or("");

    let paths = extract_file_paths(stdout);
    paths
        .into_iter()
        .take(10) // cap to avoid search floods
        .map(|path| {
            let mut e = Entity::new("__global__".to_string(), EntityType::File, path.clone());
            e.properties.insert("path".to_string(), Value::String(path));
            e
        })
        .collect()
}

/// Extract from multimodal_analyze result — look for named entities in the
/// analysis output field.
fn extract_multimodal(parsed: Option<&Value>) -> Vec<Entity> {
    let Some(obj) = parsed.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let Some(arr) = obj.get("entities").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|item| {
            let o = item.as_object()?;
            let name = o.get("name").and_then(|v| v.as_str())?;
            let type_str = o.get("type").and_then(|v| v.as_str()).unwrap_or("concept");
            let mut entity = Entity::new(
                "__global__".to_string(),
                EntityType::from_str(type_str),
                name.to_string(),
            );
            for (k, v) in o {
                if k != "name" && k != "type" {
                    entity.properties.insert(k.clone(), v.clone());
                }
            }
            Some(entity)
        })
        .collect()
}

fn extract_file_paths(text: &str) -> Vec<String> {
    // Very conservative path matcher: tokens starting with / or ./ with
    // at least one more /. Skip URLs (contain ://).
    const TRIM_CHARS: [char; 7] = [',', ':', ';', ')', ']', '"', '\''];
    text.split_whitespace()
        .filter_map(|tok| {
            let t = tok.trim_end_matches(TRIM_CHARS);
            let is_path = (t.starts_with('/') || t.starts_with("./"))
                && t.matches('/').count() >= 1
                && !t.contains("://")
                && t.len() < 300;
            if is_path {
                Some(t.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn stamp_provenance(mut entity: Entity, episode_id: &str, tool_call_id: &str) -> Entity {
    entity.properties.insert(
        "_source_episode_id".to_string(),
        Value::String(episode_id.to_string()),
    );
    entity.properties.insert(
        "_source_ref".to_string(),
        Value::String(format!("tool_call:{}", tool_call_id)),
    );
    entity.properties.insert(
        "_epistemic_class".to_string(),
        Value::String("archival".to_string()),
    );
    entity
}

async fn ensure_episode(
    store: &dyn KgEpisodeStore,
    tool_call_id: &str,
    content: &str,
    session_id: &str,
    agent_id: &str,
) -> Result<String, ExecutionError> {
    let content_hash = hash_content(content);
    // Dedup: if we've seen this exact tool output before, reuse the episode.
    if let Ok(Some(existing)) = store
        .get_by_content_hash(EpisodeSource::ToolResult.as_str(), &content_hash)
        .await
    {
        if let Some(id) = existing.get("id").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
    }
    let id = store
        .upsert_pending(
            EpisodeSource::ToolResult.as_str(),
            tool_call_id,
            &content_hash,
            Some(session_id),
            agent_id,
        )
        .await?;
    // Tool-result extraction is synchronous w.r.t. the episode — once the
    // entities are about to be stored, the extraction is "done."
    store.mark_done(&id).await?;
    Ok(id)
}

fn hash_content(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    format!("{:x}", h.finalize())
}

fn tool_result_evidence_record(
    tool_name: &str,
    tool_call_id: &str,
    session_id: &str,
    agent_id: &str,
) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: format!("{agent_id}:tool_result:{tool_call_id}"),
        action: "tool_result_distillation".to_string(),
        source_id: tool_call_id.to_string(),
        source_type: format!("tool_result:{tool_name}"),
        session_id: (!session_id.is_empty()).then(|| session_id.to_string()),
        ward_id: None,
        agent_id: agent_id.to_string(),
        retention_policy: "durable".to_string(),
        ontology_labels: Vec::new(),
        taxonomy_labels: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_fetch_extracts_document_entity() {
        let result = r#"{"url": "https://example.com/article", "title": "Test Article", "description": "A test"}"#;
        let entities = extract_from_tool("web_fetch", result);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "https://example.com/article");
        assert!(matches!(entities[0].entity_type, EntityType::Document));
        assert_eq!(
            entities[0].properties.get("title").and_then(|v| v.as_str()),
            Some("Test Article")
        );
    }

    #[test]
    fn web_fetch_without_url_extracts_nothing() {
        let result = r#"{"title": "orphan"}"#;
        let entities = extract_from_tool("web_fetch", result);
        assert!(entities.is_empty());
    }

    #[test]
    fn shell_success_extracts_file_paths() {
        let result =
            r#"{"success": true, "exit_code": 0, "stdout": "/tmp/foo.rs\n./src/bar.rs\n"}"#;
        let entities = extract_from_tool("shell", result);
        assert_eq!(entities.len(), 2);
        assert!(entities.iter().any(|e| e.name == "/tmp/foo.rs"));
    }

    #[test]
    fn shell_failure_extracts_nothing() {
        let result = r#"{"success": false, "stdout": "/tmp/foo.rs"}"#;
        let entities = extract_from_tool("shell", result);
        assert!(entities.is_empty());
    }

    #[test]
    fn shell_skips_urls_in_stdout() {
        let result = r#"{"success": true, "stdout": "https://example.com/"}"#;
        let entities = extract_from_tool("shell", result);
        assert!(entities.is_empty());
    }

    #[test]
    fn multimodal_extracts_entities_array() {
        let result =
            r#"{"entities": [{"name": "Eiffel Tower", "type": "location", "city": "Paris"}]}"#;
        let entities = extract_from_tool("multimodal_analyze", result);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "Eiffel Tower");
        assert!(matches!(entities[0].entity_type, EntityType::Location));
        assert_eq!(
            entities[0].properties.get("city").and_then(|v| v.as_str()),
            Some("Paris")
        );
    }

    #[test]
    fn unknown_tool_returns_empty() {
        let entities = extract_from_tool("mystery_tool", r#"{"any": "thing"}"#);
        assert!(entities.is_empty());
    }

    #[test]
    fn non_json_returns_empty() {
        let entities = extract_from_tool("web_fetch", "not json at all");
        assert!(entities.is_empty());
    }

    #[test]
    fn stamp_provenance_adds_three_markers() {
        let e = Entity::new("root".to_string(), EntityType::Concept, "x".to_string());
        let stamped = stamp_provenance(e, "ep-1", "call-42");
        assert!(stamped.properties.contains_key("_source_episode_id"));
        assert!(stamped.properties.contains_key("_source_ref"));
        assert_eq!(
            stamped
                .properties
                .get("_epistemic_class")
                .and_then(|v| v.as_str()),
            Some("archival")
        );
    }

    #[test]
    fn tool_result_evidence_record_uses_tool_call_provenance() {
        let record = tool_result_evidence_record("shell", "call-42", "sess-1", "root");

        assert_eq!(record.evidence_id, "root:tool_result:call-42");
        assert_eq!(record.action, "tool_result_distillation");
        assert_eq!(record.source_id, "call-42");
        assert_eq!(record.source_type, "tool_result:shell");
        assert_eq!(record.session_id.as_deref(), Some("sess-1"));
        assert_eq!(record.retention_policy, "durable");
    }

    #[test]
    fn file_path_extractor_caps_at_ten() {
        let stdout = (0..25)
            .map(|i| format!("/tmp/file{}.txt", i))
            .collect::<Vec<_>>()
            .join(" ");
        let result = format!(r#"{{"success": true, "stdout": "{}"}}"#, stdout);
        let entities = extract_from_tool("shell", &result);
        assert!(entities.len() <= 10);
    }
}
