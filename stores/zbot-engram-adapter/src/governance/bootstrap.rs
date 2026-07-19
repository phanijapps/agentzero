//! Bootstrap zbot-owned governance definitions into Engram.

use std::{collections::BTreeSet, path::Path, sync::Arc};

use chrono::Utc;
use engram_domain::{
    Actor, ActorKind, AllowedUse, Concept, ConceptLabel, ConceptRelation, ConceptRelationKind,
    ConceptScheme, DeleteMode, Id, Ontology, OntologyClass, OntologyLanguage, OntologyProperty,
    OntologyPropertyKind, OntologyStatus, OntologyTermStatus, Policy, Provenance, Retention, Scope,
    Sensitivity, Visibility,
};
use engram_knowledge::{OntologyRepository, TaxonomyRepository};

use serde::Deserialize;

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
    /// All ontology definitions persisted during this bootstrap.
    pub ontology_ids: Vec<String>,
    /// All taxonomy scheme definitions persisted during this bootstrap.
    pub taxonomy_scheme_ids: Vec<String>,
    /// First ontology ID, retained for the existing additive health DTO.
    pub ontology_id: String,
    /// First taxonomy scheme ID, retained for the existing additive health DTO.
    pub taxonomy_scheme_id: String,
    pub class_count: usize,
    pub property_count: usize,
    pub concept_count: usize,
    pub relation_count: usize,
}

/// Definitions loaded once from the configured, confined local files.
///
/// The provider holds this exact snapshot for the lifetime of its stores. That
/// keeps bootstrap and query expansion deterministic even if an operator edits
/// a definition file while the daemon is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GovernanceDefinitions {
    pub(crate) ontologies: Vec<OntologyDefinition>,
    pub(crate) taxonomies: Vec<SkosSchemeDefinition>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DefinitionDocument<T> {
    kind: String,
    schema_version: u32,
    #[serde(flatten)]
    definition: T,
}

/// Bootstrap active built-in governance definitions into Engram.
pub(crate) async fn bootstrap_governance_definitions(
    config: &AdapterConfig,
    ontology_repo: Arc<dyn OntologyRepository>,
    taxonomy_repo: Arc<dyn TaxonomyRepository>,
) -> AdapterResult<(GovernanceDefinitions, GovernanceBootstrapReport)> {
    let definitions = load_governance_definitions(config)?;
    let scope = governance_scope(config);
    let now = Utc::now();

    let mut class_count = 0;
    let mut property_count = 0;
    for ontology in &definitions.ontologies {
        bootstrap_ontology(ontology_repo.as_ref(), ontology, &scope, now).await?;
        class_count += ontology.entity_classes.len();
        property_count += ontology.relationship_properties.len();
    }

    let mut concept_count = 0;
    let mut relation_count = 0;
    for taxonomy in &definitions.taxonomies {
        relation_count += bootstrap_taxonomy(taxonomy_repo.as_ref(), taxonomy, &scope, now).await?;
        concept_count += taxonomy.concepts.len();
    }

    let ontology_ids = definitions
        .ontologies
        .iter()
        .map(|ontology| ontology.ontology_id.clone())
        .collect::<Vec<_>>();
    let taxonomy_scheme_ids = definitions
        .taxonomies
        .iter()
        .map(|taxonomy| taxonomy.scheme_id.clone())
        .collect::<Vec<_>>();
    let report = GovernanceBootstrapReport {
        ontology_id: ontology_ids.first().cloned().unwrap_or_default(),
        taxonomy_scheme_id: taxonomy_scheme_ids.first().cloned().unwrap_or_default(),
        ontology_ids,
        taxonomy_scheme_ids,
        class_count,
        property_count,
        concept_count,
        relation_count,
    };

    Ok((definitions, report))
}

async fn bootstrap_ontology(
    ontology_repo: &dyn OntologyRepository,
    ontology: &OntologyDefinition,
    scope: &Scope,
    now: chrono::DateTime<Utc>,
) -> AdapterResult<()> {
    ontology_repo
        .put_ontology(to_engram_ontology(ontology, scope, now))
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
            .put_property(to_engram_property(ontology, property, now))
            .await
            .map_err(|_| bootstrap_error("put_property"))?;
    }

    Ok(())
}

async fn bootstrap_taxonomy(
    taxonomy_repo: &dyn TaxonomyRepository,
    taxonomy: &SkosSchemeDefinition,
    scope: &Scope,
    now: chrono::DateTime<Utc>,
) -> AdapterResult<usize> {
    taxonomy_repo
        .put_concept_scheme(to_engram_scheme(taxonomy, scope, now))
        .await
        .map_err(|_| bootstrap_error("put_concept_scheme"))?;

    for concept in &taxonomy.concepts {
        taxonomy_repo
            .put_concept(to_engram_concept(taxonomy, concept, now))
            .await
            .map_err(|_| bootstrap_error("put_concept"))?;
    }

    let mut relation_count = 0;
    for concept in &taxonomy.concepts {
        for target in &concept.broader {
            put_relation(
                taxonomy_repo,
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
                taxonomy_repo,
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
                taxonomy_repo,
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

    Ok(relation_count)
}

pub(crate) fn load_governance_definitions(
    config: &AdapterConfig,
) -> AdapterResult<GovernanceDefinitions> {
    let paths = config.resolve_governance_definition_paths()?;
    let ontologies = if paths.ontology_definition_paths.is_empty() {
        vec![builtin_base_ontology()]
    } else {
        load_definition_documents(
            &paths.ontology_definition_paths,
            "zbot.ontology",
            "governance_ontology_definition",
        )?
    };
    let taxonomies = if paths.taxonomy_definition_paths.is_empty() {
        vec![builtin_starter_skos_scheme()]
    } else {
        load_definition_documents(
            &paths.taxonomy_definition_paths,
            "zbot.skos_taxonomy",
            "governance_taxonomy_definition",
        )?
    };

    validate_definitions(config, &ontologies, &taxonomies)?;
    Ok(GovernanceDefinitions {
        ontologies,
        taxonomies,
    })
}

fn load_definition_documents<T>(
    paths: &[std::path::PathBuf],
    expected_kind: &str,
    component: &'static str,
) -> AdapterResult<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    paths
        .iter()
        .map(|path| load_definition_document(path, expected_kind, component))
        .collect()
}

fn load_definition_document<T>(
    path: &Path,
    expected_kind: &str,
    component: &'static str,
) -> AdapterResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let contents = std::fs::read_to_string(path).map_err(|_| AdapterError::Bootstrap {
        component,
        reason: "configured definition file cannot be read".to_string(),
    })?;
    let document = serde_json::from_str::<DefinitionDocument<T>>(&contents).map_err(|_| {
        AdapterError::Bootstrap {
            component,
            reason: "configured definition file is invalid".to_string(),
        }
    })?;
    if document.kind != expected_kind || document.schema_version != 1 {
        return Err(AdapterError::Bootstrap {
            component,
            reason: "configured definition kind or schema version is unsupported".to_string(),
        });
    }
    Ok(document.definition)
}

fn validate_definitions(
    config: &AdapterConfig,
    ontologies: &[OntologyDefinition],
    taxonomies: &[SkosSchemeDefinition],
) -> AdapterResult<()> {
    validate_ontology_definitions(ontologies)?;
    validate_taxonomy_definitions(taxonomies)?;

    let ontology_ids = ontologies
        .iter()
        .map(|definition| definition.ontology_id.as_str())
        .collect::<BTreeSet<_>>();
    let taxonomy_scheme_ids = taxonomies
        .iter()
        .map(|definition| definition.scheme_id.as_str())
        .collect::<BTreeSet<_>>();
    for selection in std::iter::once(&config.governance.default_selection).chain(
        config
            .governance
            .overlays
            .iter()
            .map(|overlay| &overlay.selection),
    ) {
        if selection
            .ontology_ids
            .iter()
            .any(|id| !ontology_ids.contains(id.as_str()))
        {
            return Err(AdapterError::Bootstrap {
                component: "governance_selection",
                reason: "selected ontology definition is unavailable".to_string(),
            });
        }
        if selection
            .taxonomy_scheme_ids
            .iter()
            .any(|id| !taxonomy_scheme_ids.contains(id.as_str()))
        {
            return Err(AdapterError::Bootstrap {
                component: "governance_selection",
                reason: "selected taxonomy definition is unavailable".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_ontology_definitions(definitions: &[OntologyDefinition]) -> AdapterResult<()> {
    let ids = definitions
        .iter()
        .map(|definition| definition.ontology_id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != definitions.len()
        || definitions.iter().any(|definition| {
            definition.ontology_id.trim().is_empty() || definition.label.trim().is_empty()
        })
    {
        return Err(AdapterError::Bootstrap {
            component: "governance_ontology_definition",
            reason: "configured ontology definitions are invalid".to_string(),
        });
    }

    for definition in definitions {
        let class_ids = definition
            .entity_classes
            .iter()
            .map(|class| class.id.as_str())
            .collect::<BTreeSet<_>>();
        let property_ids = definition
            .relationship_properties
            .iter()
            .map(|property| property.id.as_str())
            .collect::<BTreeSet<_>>();
        if class_ids.len() != definition.entity_classes.len()
            || property_ids.len() != definition.relationship_properties.len()
            || definition
                .entity_classes
                .iter()
                .any(|class| class.id.trim().is_empty() || class.label.trim().is_empty())
            || definition
                .relationship_properties
                .iter()
                .any(|property| property.id.trim().is_empty() || property.label.trim().is_empty())
        {
            return Err(AdapterError::Bootstrap {
                component: "governance_ontology_definition",
                reason: "configured ontology definitions are invalid".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_taxonomy_definitions(definitions: &[SkosSchemeDefinition]) -> AdapterResult<()> {
    let ids = definitions
        .iter()
        .map(|definition| definition.scheme_id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != definitions.len()
        || definitions.iter().any(|definition| {
            definition.scheme_id.trim().is_empty() || definition.label.trim().is_empty()
        })
    {
        return Err(AdapterError::Bootstrap {
            component: "governance_taxonomy_definition",
            reason: "configured taxonomy definitions are invalid".to_string(),
        });
    }
    for definition in definitions {
        crate::governance::validate_skos_scheme(definition).map_err(|_| {
            AdapterError::Bootstrap {
                component: "governance_taxonomy_definition",
                reason: "configured taxonomy definitions are invalid".to_string(),
            }
        })?;
    }
    Ok(())
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
