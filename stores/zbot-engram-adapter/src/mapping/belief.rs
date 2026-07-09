//! Belief and contradiction mappings between AgentZero DTOs and Engram records.

use std::collections::BTreeMap;

use chrono::Utc;
use engram_domain::{
    Actor, ActorKind, AllowedUse, BeliefId as EngramBeliefId, BeliefSource, BeliefSourceTargetType,
    BeliefStatus, BeliefSubject, Contradiction, ContradictionId as EngramContradictionId,
    ContradictionKind, ContradictionResolution, ContradictionResolutionKind, ContradictionStatus,
    ContradictionTarget, ContradictionTargetType, DeleteMode, DerivationKind, DerivationRef,
    Metadata, Policy, Provenance, Retention, Visibility,
};
use serde_json::json;
use zbot_stores_domain::{
    Belief as ZbotBelief, BeliefContradiction as ZbotContradiction, ContradictionType, Resolution,
};

use crate::{
    error::{AdapterError, AdapterResult},
    scope::ScopeMapper,
};

const ZBOT_BELIEF_METADATA_KEY: &str = "zbotBelief";

/// Map a zbot belief into Engram's derived belief contract.
pub fn belief_to_belief_record(
    belief: &ZbotBelief,
    mapper: &ScopeMapper,
) -> AdapterResult<engram_domain::Belief> {
    let scope = mapper.partition_scope(&belief.partition_id)?;
    let mut sidecar = belief.clone();
    sidecar.embedding = None;
    let mut metadata = BTreeMap::new();
    metadata.insert(
        ZBOT_BELIEF_METADATA_KEY.to_string(),
        serde_json::to_value(sidecar).map_err(|error| AdapterError::Mapping {
            field: ZBOT_BELIEF_METADATA_KEY,
            reason: error.to_string(),
        })?,
    );
    metadata.insert("partitionId".to_string(), json!(belief.partition_id));
    metadata.insert(
        "synthesizerVersion".to_string(),
        json!(belief.synthesizer_version),
    );

    Ok(engram_domain::Belief {
        id: EngramBeliefId::from(belief.id.clone()),
        scope,
        subject: BeliefSubject {
            key: belief.subject.clone(),
            entity_ref: None,
            concept_ref: None,
            aliases: Vec::new(),
        },
        content: belief.content.clone(),
        status: status_for_belief(belief),
        confidence: belief.confidence as f32,
        sources: belief
            .source_fact_ids
            .iter()
            .map(|fact_id| BeliefSource {
                target_type: BeliefSourceTargetType::Memory,
                target_id: fact_id.clone(),
                authority_level: None,
                weight: None,
                confidence: None,
                valid_from: belief.valid_from,
                valid_until: belief.valid_until,
            })
            .collect(),
        valid_from: belief.valid_from,
        valid_until: belief.valid_until,
        superseded_by: belief.superseded_by.clone().map(EngramBeliefId::from),
        stale: Some(belief.stale),
        synthesizer: Some(DerivationRef {
            kind: DerivationKind::Consolidation,
            model: Some(format!(
                "zbot-belief-synthesizer-v{}",
                belief.synthesizer_version
            )),
            prompt_hash: None,
            input_refs: Vec::new(),
            created_at: belief.created_at,
        }),
        reasoning: belief.reasoning.clone(),
        embedding_refs: Vec::new(),
        policy: durable_workspace_policy(),
        provenance: provenance(
            "agentzero-belief-synthesizer",
            belief.valid_from.unwrap_or(belief.created_at),
            "agentzero.belief_adapter",
            Some(belief.confidence as f32),
        ),
        created_at: belief.created_at,
        updated_at: Some(belief.updated_at),
        metadata: Some(metadata),
    })
}

/// Reconstruct a zbot belief from an Engram belief record.
pub fn belief_record_to_belief(record: &engram_domain::Belief) -> AdapterResult<ZbotBelief> {
    if let Some(value) = record
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(ZBOT_BELIEF_METADATA_KEY))
    {
        let mut belief: ZbotBelief =
            serde_json::from_value(value.clone()).map_err(|error| AdapterError::Mapping {
                field: ZBOT_BELIEF_METADATA_KEY,
                reason: error.to_string(),
            })?;
        belief.content = record.content.clone();
        belief.confidence = f64::from(record.confidence);
        belief.valid_from = record.valid_from;
        belief.valid_until = record.valid_until;
        belief.superseded_by = record
            .superseded_by
            .as_ref()
            .map(|id| id.as_str().to_string());
        belief.stale = record.stale.unwrap_or(record.status == BeliefStatus::Stale);
        belief.reasoning = record.reasoning.clone();
        belief.updated_at = record.updated_at.unwrap_or(record.created_at);
        belief.embedding = None;
        return Ok(belief);
    }

    Ok(ZbotBelief {
        id: record.id.as_str().to_string(),
        partition_id: metadata_string(record.metadata.as_ref(), "partitionId")
            .or_else(|| record.scope.workspace.clone())
            .unwrap_or_else(|| "root".to_string()),
        subject: record.subject.key.clone(),
        content: record.content.clone(),
        confidence: f64::from(record.confidence),
        valid_from: record.valid_from,
        valid_until: record.valid_until,
        source_fact_ids: record
            .sources
            .iter()
            .filter(|source| source.target_type == BeliefSourceTargetType::Memory)
            .map(|source| source.target_id.clone())
            .collect(),
        synthesizer_version: metadata_i64(record.metadata.as_ref(), "synthesizerVersion")
            .unwrap_or(0) as i32,
        reasoning: record.reasoning.clone(),
        created_at: record.created_at,
        updated_at: record.updated_at.unwrap_or(record.created_at),
        superseded_by: record
            .superseded_by
            .as_ref()
            .map(|id| id.as_str().to_string()),
        stale: record.stale.unwrap_or(record.status == BeliefStatus::Stale),
        embedding: None,
    })
}

/// Map a zbot contradiction into Engram's reviewable contradiction contract.
pub fn contradiction_to_record(
    contradiction: &ZbotContradiction,
    partition_id: &str,
    mapper: &ScopeMapper,
) -> AdapterResult<Contradiction> {
    let scope = mapper.partition_scope(partition_id)?;
    let targets = vec![
        ContradictionTarget {
            target_type: ContradictionTargetType::Belief,
            target_id: contradiction.belief_a_id.clone(),
            role: Some("a".to_string()),
        },
        ContradictionTarget {
            target_type: ContradictionTargetType::Belief,
            target_id: contradiction.belief_b_id.clone(),
            role: Some("b".to_string()),
        },
    ];
    let resolution = contradiction.resolution.as_ref().map(|resolution| {
        contradiction_resolution_to_record(
            resolution,
            &contradiction.belief_a_id,
            &contradiction.belief_b_id,
            contradiction.resolved_at.unwrap_or_else(Utc::now),
        )
    });

    Ok(Contradiction {
        id: EngramContradictionId::from(contradiction.id.clone()),
        scope,
        kind: kind_for_contradiction(&contradiction.contradiction_type),
        targets,
        severity: contradiction.severity as f32,
        status: if resolution.is_some() {
            ContradictionStatus::Resolved
        } else {
            ContradictionStatus::Open
        },
        reasoning: contradiction.judge_reasoning.clone(),
        detected_by: None,
        resolution,
        provenance: provenance(
            "agentzero-belief-contradiction-detector",
            contradiction.detected_at,
            "agentzero.belief_contradiction_adapter",
            Some(contradiction.severity as f32),
        ),
        detected_at: contradiction.detected_at,
        updated_at: contradiction.resolved_at,
    })
}

/// Reconstruct a zbot contradiction from an Engram contradiction record.
pub fn contradiction_record_to_contradiction(
    record: &Contradiction,
) -> AdapterResult<ZbotContradiction> {
    let belief_targets = record
        .targets
        .iter()
        .filter(|target| target.target_type == ContradictionTargetType::Belief)
        .collect::<Vec<_>>();
    if belief_targets.len() < 2 {
        return Err(AdapterError::Mapping {
            field: "contradiction.targets",
            reason: "expected two belief targets".to_string(),
        });
    }

    Ok(ZbotContradiction {
        id: record.id.as_str().to_string(),
        belief_a_id: belief_targets[0].target_id.clone(),
        belief_b_id: belief_targets[1].target_id.clone(),
        contradiction_type: contradiction_type_for_kind(&record.kind),
        severity: f64::from(record.severity),
        judge_reasoning: record.reasoning.clone(),
        detected_at: record.detected_at,
        resolved_at: record
            .resolution
            .as_ref()
            .map(|resolution| resolution.resolved_at),
        resolution: record
            .resolution
            .as_ref()
            .map(|resolution| resolution_from_record(resolution, &belief_targets[0].target_id)),
    })
}

/// Map a zbot resolution into Engram's review resolution contract.
pub fn contradiction_resolution_to_record(
    resolution: &Resolution,
    belief_a_id: &str,
    belief_b_id: &str,
    resolved_at: chrono::DateTime<Utc>,
) -> ContradictionResolution {
    let (kind, winning_target_id) = match resolution {
        Resolution::AWon => (
            ContradictionResolutionKind::TargetWon,
            Some(belief_a_id.to_string()),
        ),
        Resolution::BWon => (
            ContradictionResolutionKind::TargetWon,
            Some(belief_b_id.to_string()),
        ),
        Resolution::Compatible => (ContradictionResolutionKind::Compatible, None),
        Resolution::Unresolved => (ContradictionResolutionKind::NeedsMoreEvidence, None),
    };

    ContradictionResolution {
        kind,
        winning_target_id,
        actor: Actor {
            id: "agentzero-operator".into(),
            kind: ActorKind::User,
            display_name: None,
            metadata: None,
        },
        reason: None,
        resolved_at,
    }
}

fn status_for_belief(belief: &ZbotBelief) -> BeliefStatus {
    if belief.superseded_by.is_some() {
        BeliefStatus::Superseded
    } else if belief.stale {
        BeliefStatus::Stale
    } else {
        BeliefStatus::Active
    }
}

fn kind_for_contradiction(kind: &ContradictionType) -> ContradictionKind {
    match kind {
        ContradictionType::Logical => ContradictionKind::Logical,
        ContradictionType::Tension => ContradictionKind::Tension,
        ContradictionType::Temporal => ContradictionKind::Temporal,
    }
}

fn contradiction_type_for_kind(kind: &ContradictionKind) -> ContradictionType {
    match kind {
        ContradictionKind::Logical => ContradictionType::Logical,
        ContradictionKind::Temporal => ContradictionType::Temporal,
        ContradictionKind::Tension | ContradictionKind::Duplicate | ContradictionKind::Policy => {
            ContradictionType::Tension
        }
    }
}

fn resolution_from_record(record: &ContradictionResolution, belief_a_id: &str) -> Resolution {
    match record.kind {
        ContradictionResolutionKind::TargetWon => {
            if record.winning_target_id.as_deref() == Some(belief_a_id) {
                Resolution::AWon
            } else {
                Resolution::BWon
            }
        }
        ContradictionResolutionKind::Compatible => Resolution::Compatible,
        ContradictionResolutionKind::NeedsMoreEvidence
        | ContradictionResolutionKind::ManualIgnore
        | ContradictionResolutionKind::Merged
        | ContradictionResolutionKind::Retracted => Resolution::Unresolved,
    }
}

fn provenance(
    actor_id: &str,
    observed_at: chrono::DateTime<Utc>,
    method: &str,
    confidence: Option<f32>,
) -> Provenance {
    Provenance {
        source: method.to_string(),
        actor: Actor {
            id: actor_id.into(),
            kind: ActorKind::Agent,
            display_name: None,
            metadata: None,
        },
        observed_at,
        evidence: Vec::new(),
        derivations: Vec::new(),
        confidence,
        method: Some(method.to_string()),
    }
}

fn durable_workspace_policy() -> Policy {
    Policy {
        visibility: Visibility::Workspace,
        retention: Retention::Durable,
        sensitivity: None,
        allowed_uses: vec![AllowedUse::Retrieval, AllowedUse::Consolidation],
        expires_at: None,
        delete_mode: Some(DeleteMode::Archive),
    }
}

fn metadata_string(metadata: Option<&Metadata>, key: &str) -> Option<String> {
    metadata?.get(key)?.as_str().map(ToOwned::to_owned)
}

fn metadata_i64(metadata: Option<&Metadata>, key: &str) -> Option<i64> {
    metadata?.get(key)?.as_i64()
}
