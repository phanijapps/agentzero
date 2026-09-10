//! Memory fact mapping between AgentZero rows and Engram memory records.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use engram_domain::{
    Actor, ActorKind, AllowedUse, DeleteMode, MemoryContent, MemoryContentFormat, MemoryId,
    MemoryKind, MemoryRecord, MemoryStatus, Metadata, Policy, Provenance, Retention, Visibility,
};
use serde_json::{json, Value};
use zbot_stores_traits::MemoryFact;

use crate::{
    config::EmbeddingMode,
    error::{AdapterError, AdapterResult},
    governance::{select_and_persist_governance_metadata, GovernancePolicy, GovernanceScope},
    scope::ScopeMapper,
};

const ZBOT_FACT_METADATA_KEY: &str = "zbotMemoryFact";

/// Map a zbot `MemoryFact` into Engram's canonical `MemoryRecord`.
///
/// The Engram record receives first-class scope/provenance/policy fields while
/// the exact AgentZero DTO is preserved in metadata for gateway compatibility.
/// Embedding vectors are intentionally not embedded in the record metadata; the
/// store keeps those bytes in its sidecar.
pub fn memory_fact_to_record(
    fact: &MemoryFact,
    mapper: &ScopeMapper,
    embedding_mode: EmbeddingMode,
) -> AdapterResult<MemoryRecord> {
    memory_fact_to_record_with_governance(fact, mapper, embedding_mode, None)
}

/// Map a zbot `MemoryFact` and persist the selected governance classification
/// on the canonical Engram memory record.
pub fn memory_fact_to_record_with_governance(
    fact: &MemoryFact,
    mapper: &ScopeMapper,
    embedding_mode: EmbeddingMode,
    governance: Option<&GovernancePolicy>,
) -> AdapterResult<MemoryRecord> {
    let created_at = parse_timestamp("created_at", &fact.created_at)?;
    let updated_at = parse_timestamp("updated_at", &fact.updated_at)?;
    let observed_at = match fact.valid_from.as_deref() {
        Some(value) => parse_timestamp("valid_from", value)?,
        None => created_at,
    };
    let expires_at = optional_timestamp("expires_at", fact.expires_at.as_deref())?;
    let scope = mapper.memory_fact_scope(&fact.ward_id, fact.session_id.as_deref())?;

    let mut fact_metadata = fact.clone();
    fact_metadata.embedding = None;

    let mut metadata = BTreeMap::new();
    metadata.insert(
        ZBOT_FACT_METADATA_KEY.to_string(),
        serde_json::to_value(&fact_metadata).map_err(|error| AdapterError::Mapping {
            field: ZBOT_FACT_METADATA_KEY,
            reason: error.to_string(),
        })?,
    );
    metadata.insert("agentId".to_string(), json!(fact.agent_id));
    metadata.insert("scope".to_string(), json!(fact.scope));
    metadata.insert("category".to_string(), json!(fact.category));
    metadata.insert("key".to_string(), json!(fact.key));
    metadata.insert("wardId".to_string(), json!(fact.ward_id));
    metadata.insert("sessionId".to_string(), json!(fact.session_id));
    metadata.insert("sourceEpisodeId".to_string(), json!(fact.source_episode_id));
    metadata.insert("sourceRef".to_string(), json!(fact.source_ref));
    metadata.insert("importance".to_string(), Value::Null);
    metadata.insert(
        "embeddingMode".to_string(),
        json!(embedding_mode_wire_value(embedding_mode)),
    );
    apply_memory_governance(governance, fact, &mut metadata);

    Ok(MemoryRecord {
        id: MemoryId::from(fact.id.clone()),
        kind: memory_kind_for(&fact.category),
        content: MemoryContent {
            text: fact.content.clone(),
            summary: fact.source_summary.clone(),
            entities: Vec::new(),
            language: None,
            format: Some(MemoryContentFormat::Text),
            structured: None,
            hash: None,
        },
        scope,
        provenance: Provenance {
            source: fact
                .source_ref
                .clone()
                .unwrap_or_else(|| "agentzero.memory_fact".to_string()),
            actor: Actor {
                id: fact.agent_id.as_str().into(),
                kind: ActorKind::Agent,
                display_name: None,
                metadata: None,
            },
            observed_at,
            evidence: Vec::new(),
            derivations: Vec::new(),
            confidence: Some(fact.confidence as f32),
            method: Some("agentzero.memory_fact_adapter".to_string()),
        },
        policy: Policy {
            visibility: visibility_for(&fact.scope),
            retention: retention_for(&fact.scope, fact.session_id.as_deref()),
            sensitivity: None,
            allowed_uses: vec![
                AllowedUse::Retrieval,
                AllowedUse::Personalization,
                AllowedUse::Consolidation,
            ],
            expires_at,
            delete_mode: Some(DeleteMode::Archive),
        },
        status: if fact.valid_until.is_some() || fact.superseded_by.is_some() {
            MemoryStatus::Archived
        } else {
            MemoryStatus::Active
        },
        links: Vec::new(),
        assertions: Vec::new(),
        created_at,
        updated_at: Some(updated_at),
        metadata: Some(metadata),
    })
}

fn apply_memory_governance(
    governance: Option<&GovernancePolicy>,
    fact: &MemoryFact,
    metadata: &mut Metadata,
) {
    let Some(governance) = governance else {
        return;
    };
    let selection = select_and_persist_governance_metadata(
        governance,
        GovernanceScope {
            ward_id: Some(&fact.ward_id),
            session_id: fact.session_id.as_deref(),
            source_id: fact.source_ref.as_deref(),
            ..GovernanceScope::default()
        },
        metadata,
    );
    let concept_ids = selection
        .taxonomy_scheme_ids
        .iter()
        .map(|scheme_id| format!("{scheme_id}:concept:memory"))
        .collect::<Vec<_>>();
    if !concept_ids.is_empty() {
        metadata.insert(
            "governanceTaxonomyConceptIds".to_string(),
            json!(concept_ids),
        );
    }
}

/// Reconstruct a zbot `MemoryFact` from an Engram `MemoryRecord`.
pub fn memory_record_to_fact(record: &MemoryRecord) -> AdapterResult<MemoryFact> {
    if let Some(fact) = record
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(ZBOT_FACT_METADATA_KEY))
    {
        let mut fact: MemoryFact =
            serde_json::from_value(fact.clone()).map_err(|error| AdapterError::Mapping {
                field: ZBOT_FACT_METADATA_KEY,
                reason: error.to_string(),
            })?;
        fact.embedding = None;
        return Ok(fact);
    }

    let metadata = record.metadata.as_ref();
    Ok(MemoryFact {
        id: record.id.as_str().to_string(),
        session_id: record.scope.session.clone(),
        agent_id: metadata_string(metadata, "agentId")
            .unwrap_or_else(|| record.provenance.actor.id.as_str().to_string()),
        scope: metadata_string(metadata, "scope").unwrap_or_else(|| scope_name(record)),
        category: metadata_string(metadata, "category").unwrap_or_else(|| "fact".to_string()),
        key: metadata_string(metadata, "key").unwrap_or_else(|| record.id.as_str().to_string()),
        content: record.content.text.clone(),
        confidence: record
            .provenance
            .confidence
            .map(f64::from)
            .unwrap_or(1.0_f64),
        mention_count: metadata_i64(metadata, "mentionCount").unwrap_or(1) as i32,
        source_summary: record.content.summary.clone(),
        embedding: None,
        ward_id: metadata_string(metadata, "wardId")
            .or_else(|| record.scope.workspace.clone())
            .unwrap_or_else(|| "__global__".to_string()),
        contradicted_by: metadata_string(metadata, "contradictedBy"),
        created_at: timestamp_to_rfc3339(record.created_at),
        updated_at: record
            .updated_at
            .map(timestamp_to_rfc3339)
            .unwrap_or_else(|| timestamp_to_rfc3339(record.created_at)),
        expires_at: record.policy.expires_at.map(timestamp_to_rfc3339),
        valid_from: Some(timestamp_to_rfc3339(record.provenance.observed_at)),
        valid_until: metadata_string(metadata, "validUntil"),
        superseded_by: metadata_string(metadata, "supersededBy"),
        pinned: metadata_bool(metadata, "pinned").unwrap_or(false),
        epistemic_class: metadata_string(metadata, "epistemicClass"),
        source_episode_id: metadata_string(metadata, "sourceEpisodeId"),
        source_ref: metadata_string(metadata, "sourceRef"),
        last_accessed: metadata_string(metadata, "lastAccessed"),
    })
}

fn parse_timestamp(field: &'static str, value: &str) -> AdapterResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| AdapterError::Mapping {
            field,
            reason: error.to_string(),
        })
}

fn optional_timestamp(
    field: &'static str,
    value: Option<&str>,
) -> AdapterResult<Option<DateTime<Utc>>> {
    value.map(|value| parse_timestamp(field, value)).transpose()
}

fn timestamp_to_rfc3339(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn memory_kind_for(category: &str) -> MemoryKind {
    match category {
        "preference" => MemoryKind::Preference,
        "episode" => MemoryKind::Episode,
        "artifact" | "primitive" => MemoryKind::Artifact,
        "relationship" => MemoryKind::Relationship,
        "strategy" | "pattern" | "procedure" | "instruction" => MemoryKind::Procedure,
        "observation" | "ctx" => MemoryKind::Observation,
        _ => MemoryKind::Fact,
    }
}

fn visibility_for(scope: &str) -> Visibility {
    match scope {
        "agent" | "session" => Visibility::Private,
        _ => Visibility::Workspace,
    }
}

fn retention_for(scope: &str, session_id: Option<&str>) -> Retention {
    if scope == "session" || session_id.is_some() {
        Retention::Session
    } else {
        Retention::Durable
    }
}

fn embedding_mode_wire_value(mode: EmbeddingMode) -> &'static str {
    match mode {
        EmbeddingMode::PreserveBytes => "preserve_bytes",
        EmbeddingMode::EngramRefs => "engram_refs",
        EmbeddingMode::Disabled => "disabled",
    }
}

fn metadata_string(metadata: Option<&BTreeMap<String, Value>>, key: &str) -> Option<String> {
    metadata?.get(key)?.as_str().map(ToOwned::to_owned)
}

fn metadata_i64(metadata: Option<&BTreeMap<String, Value>>, key: &str) -> Option<i64> {
    metadata?.get(key)?.as_i64()
}

fn metadata_bool(metadata: Option<&BTreeMap<String, Value>>, key: &str) -> Option<bool> {
    metadata?.get(key)?.as_bool()
}

fn scope_name(record: &MemoryRecord) -> String {
    if record.scope.session.is_some() {
        "session".to_string()
    } else if record.scope.workspace.as_deref() == Some("__global__") {
        "global".to_string()
    } else {
        "agent".to_string()
    }
}
