//! Bootstrap zbot-owned governance definitions into Engram.

use std::sync::Arc;

use chrono::Utc;
use engram_domain::{
    Actor, ActorKind, AllowedUse, Concept, ConceptLabel, ConceptRelation, ConceptRelationKind,
    ConceptScheme, DeleteMode, Id, Ontology, OntologyClass, OntologyLanguage, OntologyProperty,
    OntologyPropertyKind, OntologyStatus, OntologyTermStatus, Policy, Provenance, Retention, Scope,
    Sensitivity, Visibility,
};
use engram_knowledge::{OntologyRepository, TaxonomyRepository};

use crate::{
    config::AdapterConfig,
    error::{AdapterError, AdapterResult},
    governance::{
        builtin_base_ontology, builtin_starter_skos_scheme, OntologyDefinition,
        OntologyPropertyDefinition, SkosConceptDefinition, SkosSchemeDefinition,
    },
};

/// Result summary for a governance bootstrap run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceBootstrapReport {
    pub ontology_id: String,
    pub taxonomy_scheme_id: String,
    pub class_count: usize,
    pub property_count: usize,
    pub concept_count: usize,
    pub relation_count: usize,
}

/// Bootstrap active built-in governance definitions into Engram.
pub async fn bootstrap_governance_definitions(
    config: &AdapterConfig,
    ontology_repo: Arc<dyn OntologyRepository>,
    taxonomy_repo: Arc<dyn TaxonomyRepository>,
) -> AdapterResult<GovernanceBootstrapReport> {
    let scope = governance_scope(config);
    let ontology = builtin_base_ontology();
    let taxonomy = builtin_starter_skos_scheme();
    let now = Utc::now();

    ontology_repo
        .put_ontology(to_engram_ontology(&ontology, &scope, now))
        .await
        .map_err(|_| bootstrap_error("put_ontology"))?;

    for class in &ontology.entity_classes {
        ontology_repo
            .put_class(OntologyClass {
                id: Id::from(format!("{}:class:{}", ontology.ontology_id, class.id)),
                ontology_id: Id::from(ontology.ontology_id.clone()),
                uri: format!(
                    "urn:zbot:ontology:{}:class:{}",
                    ontology.ontology_id, class.id
                ),
                label: class.label.clone(),
                description: None,
                parent_class_ids: Vec::new(),
                concept_refs: Vec::new(),
                status: OntologyTermStatus::Active,
                provenance: provenance(now),
                created_at: now,
                updated_at: Some(now),
                metadata: Some(metadata([("zbot_entity_type", class.id.as_str())])),
            })
            .await
            .map_err(|_| bootstrap_error("put_class"))?;
    }

    for property in &ontology.relationship_properties {
        ontology_repo
            .put_property(to_engram_property(&ontology, property, now))
            .await
            .map_err(|_| bootstrap_error("put_property"))?;
    }

    taxonomy_repo
        .put_concept_scheme(to_engram_scheme(&taxonomy, &scope, now))
        .await
        .map_err(|_| bootstrap_error("put_concept_scheme"))?;

    for concept in &taxonomy.concepts {
        taxonomy_repo
            .put_concept(to_engram_concept(&taxonomy, concept, now))
            .await
            .map_err(|_| bootstrap_error("put_concept"))?;
    }

    let mut relation_count = 0;
    for concept in &taxonomy.concepts {
        for target in &concept.broader {
            put_relation(
                taxonomy_repo.as_ref(),
                &taxonomy.scheme_id,
                &concept.id,
                ConceptRelationKind::Broader,
                target,
                now,
            )
            .await?;
            relation_count += 1;
        }
        for target in &concept.narrower {
            put_relation(
                taxonomy_repo.as_ref(),
                &taxonomy.scheme_id,
                &concept.id,
                ConceptRelationKind::Narrower,
                target,
                now,
            )
            .await?;
            relation_count += 1;
        }
        for target in &concept.related {
            put_relation(
                taxonomy_repo.as_ref(),
                &taxonomy.scheme_id,
                &concept.id,
                ConceptRelationKind::Related,
                target,
                now,
            )
            .await?;
            relation_count += 1;
        }
    }

    Ok(GovernanceBootstrapReport {
        ontology_id: ontology.ontology_id,
        taxonomy_scheme_id: taxonomy.scheme_id,
        class_count: ontology.entity_classes.len(),
        property_count: ontology.relationship_properties.len(),
        concept_count: taxonomy.concepts.len(),
        relation_count,
    })
}

fn to_engram_ontology(
    ontology: &OntologyDefinition,
    scope: &Scope,
    now: chrono::DateTime<Utc>,
) -> Ontology {
    Ontology {
        id: Id::from(ontology.ontology_id.clone()),
        uri: format!("urn:zbot:ontology:{}", ontology.ontology_id),
        name: ontology.label.clone(),
        scope: scope.clone(),
        language: OntologyLanguage::PropertyGraph,
        version: version_from_id(&ontology.ontology_id),
        status: OntologyStatus::Active,
        imports: Vec::new(),
        policy: policy(),
        provenance: provenance(now),
        created_at: now,
        updated_at: Some(now),
        metadata: None,
    }
}

fn to_engram_property(
    ontology: &OntologyDefinition,
    property: &OntologyPropertyDefinition,
    now: chrono::DateTime<Utc>,
) -> OntologyProperty {
    OntologyProperty {
        id: Id::from(format!("{}:property:{}", ontology.ontology_id, property.id)),
        ontology_id: Id::from(ontology.ontology_id.clone()),
        uri: format!(
            "urn:zbot:ontology:{}:property:{}",
            ontology.ontology_id, property.id
        ),
        label: property.label.clone(),
        kind: OntologyPropertyKind::Object,
        domain_class_id: property
            .domain
            .first()
            .map(|id| Id::from(format!("{}:class:{id}", ontology.ontology_id))),
        range_class_id: property
            .range
            .first()
            .map(|id| Id::from(format!("{}:class:{id}", ontology.ontology_id))),
        datatype: None,
        inverse_property_id: None,
        status: OntologyTermStatus::Active,
        provenance: provenance(now),
        created_at: now,
        updated_at: Some(now),
        metadata: Some(metadata([("zbot_relationship_type", property.id.as_str())])),
    }
}

fn to_engram_scheme(
    taxonomy: &SkosSchemeDefinition,
    scope: &Scope,
    now: chrono::DateTime<Utc>,
) -> ConceptScheme {
    ConceptScheme {
        id: Id::from(taxonomy.scheme_id.clone()),
        uri: format!("urn:zbot:taxonomy:{}", taxonomy.scheme_id),
        name: taxonomy.label.clone(),
        scope: scope.clone(),
        version: version_from_id(&taxonomy.scheme_id),
        provenance: provenance(now),
        policy: policy(),
        created_at: now,
        updated_at: Some(now),
    }
}

fn to_engram_concept(
    taxonomy: &SkosSchemeDefinition,
    concept: &SkosConceptDefinition,
    now: chrono::DateTime<Utc>,
) -> Concept {
    Concept {
        id: concept_id(&taxonomy.scheme_id, &concept.id),
        uri: format!(
            "urn:zbot:taxonomy:{}:concept:{}",
            taxonomy.scheme_id, concept.id
        ),
        scheme_id: Id::from(taxonomy.scheme_id.clone()),
        pref_label: ConceptLabel {
            value: concept.pref_label.clone(),
            language: Some("en".to_string()),
        },
        alt_labels: concept
            .alt_labels
            .iter()
            .map(|label| ConceptLabel {
                value: label.clone(),
                language: Some("en".to_string()),
            })
            .collect(),
        definition: None,
        notation: Some(concept.id.clone()),
        status: if concept.deprecated {
            engram_domain::ConceptStatus::Deprecated
        } else {
            engram_domain::ConceptStatus::Active
        },
        provenance: provenance(now),
        created_at: now,
        updated_at: Some(now),
    }
}

async fn put_relation(
    taxonomy_repo: &dyn TaxonomyRepository,
    scheme_id: &str,
    source_id: &str,
    kind: ConceptRelationKind,
    target_id: &str,
    now: chrono::DateTime<Utc>,
) -> AdapterResult<()> {
    let predicate = match kind {
        ConceptRelationKind::Broader => "broader",
        ConceptRelationKind::Narrower => "narrower",
        ConceptRelationKind::Related => "related",
    };
    taxonomy_repo
        .put_concept_relation(ConceptRelation {
            id: format!("{scheme_id}:rel:{source_id}:{predicate}:{target_id}"),
            scheme_id: Id::from(scheme_id),
            subject_id: concept_id(scheme_id, source_id),
            predicate: kind,
            object_id: concept_id(scheme_id, target_id),
            provenance: provenance(now),
            created_at: now,
        })
        .await
        .map_err(|_| bootstrap_error("put_concept_relation"))?;
    Ok(())
}

fn governance_scope(config: &AdapterConfig) -> Scope {
    Scope {
        tenant: config.tenant.clone(),
        workspace: Some("zbot-governance".to_string()),
        subject: None,
        session: None,
        environment: Some("runtime".to_string()),
    }
}

fn policy() -> Policy {
    Policy {
        visibility: Visibility::Workspace,
        retention: Retention::Durable,
        sensitivity: Some(Sensitivity::Low),
        allowed_uses: vec![AllowedUse::Retrieval, AllowedUse::Evaluation],
        expires_at: None,
        delete_mode: Some(DeleteMode::Tombstone),
    }
}

fn provenance(now: chrono::DateTime<Utc>) -> Provenance {
    Provenance {
        source: "zbot-governance-bootstrap".to_string(),
        actor: Actor {
            id: Id::from("zbot-governance"),
            kind: ActorKind::System,
            display_name: Some("Zbot Governance Bootstrap".to_string()),
            metadata: None,
        },
        observed_at: now,
        evidence: Vec::new(),
        derivations: Vec::new(),
        confidence: Some(1.0),
        method: Some("built_in_definition".to_string()),
    }
}

fn metadata<const N: usize>(pairs: [(&str, &str); N]) -> engram_domain::Metadata {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), serde_json::json!(value)))
        .collect()
}

fn concept_id(scheme_id: &str, concept_id: &str) -> Id {
    Id::from(format!("{scheme_id}:concept:{concept_id}"))
}

fn version_from_id(id: &str) -> String {
    id.rsplit_once(':')
        .map(|(_, version)| version.to_string())
        .unwrap_or_else(|| "v1".to_string())
}

fn bootstrap_error(component: &'static str) -> AdapterError {
    AdapterError::Bootstrap {
        component,
        reason: "governance definition bootstrap failed".to_string(),
    }
}
