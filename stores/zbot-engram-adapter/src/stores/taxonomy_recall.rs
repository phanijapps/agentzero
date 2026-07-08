//! SKOS-style taxonomy expansion for recall.

use std::{collections::VecDeque, sync::Arc};

use async_trait::async_trait;
use engram_domain::{Concept, ConceptStatus, Id, Scope};
use engram_knowledge::TaxonomyRepository;
use zbot_stores_traits::{
    RecallTaxonomyExpander, RecallTaxonomyExpansion, RecallTaxonomyExpansionCandidate,
    RecallTaxonomyExpansionRequest,
};

use crate::{
    bootstrap::EngramProvider,
    config::{AdapterConfig, ProviderMode},
    error::{AdapterError, AdapterResult},
    governance::{builtin_starter_skos_scheme, GovernancePolicy, GovernanceScope},
};

/// Engram-backed taxonomy recall expander.
#[derive(Clone)]
pub struct EngramTaxonomyRecallExpander {
    taxonomy: Arc<dyn TaxonomyRepository>,
    tenant: String,
    governance: GovernancePolicy,
}

impl EngramTaxonomyRecallExpander {
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "taxonomy_recall",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        let provider = EngramProvider::open(config.clone())?;
        Self::from_provider(config, &provider)
    }

    pub fn from_provider(config: AdapterConfig, provider: &EngramProvider) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "taxonomy_recall",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        Ok(Self {
            taxonomy: provider.taxonomy()?,
            tenant: config.tenant.clone(),
            governance: config.governance.clone(),
        })
    }

    fn governance_scope(&self) -> Scope {
        Scope {
            tenant: self.tenant.clone(),
            workspace: Some("zbot-governance".to_string()),
            subject: None,
            session: None,
            environment: Some("runtime".to_string()),
        }
    }
}

#[async_trait]
impl RecallTaxonomyExpander for EngramTaxonomyRecallExpander {
    async fn expand_recall_query(
        &self,
        request: RecallTaxonomyExpansionRequest,
    ) -> Result<RecallTaxonomyExpansion, String> {
        if request.max_candidates == 0 {
            return Ok(RecallTaxonomyExpansion {
                expanded_query: request.query,
                candidates: Vec::new(),
            });
        }

        let ward_id = request
            .ward_id
            .clone()
            .unwrap_or_else(|| "__global__".to_string());
        let selected = self.governance.select(GovernanceScope {
            ward_id: Some(&ward_id),
            ..GovernanceScope::default()
        });
        if selected.taxonomy_scheme_ids.is_empty() {
            return Ok(RecallTaxonomyExpansion {
                expanded_query: request.query,
                candidates: Vec::new(),
            });
        }

        let mut candidates = Vec::new();
        for scheme_id in selected.taxonomy_scheme_ids {
            if candidates.len() >= request.max_candidates as usize {
                break;
            }
            let mut concepts = self
                .taxonomy
                .list_concepts(&Id::from(scheme_id.clone()), &self.governance_scope())
                .await
                .map_err(|error| error.to_string())?;
            concepts.retain(|concept| concept.status == ConceptStatus::Active);
            concepts.sort_by(|left, right| left.id.to_string().cmp(&right.id.to_string()));
            let mut scheme_candidates =
                expand_scheme(&request.query, &scheme_id, &concepts, &request);
            let remaining = request.max_candidates as usize - candidates.len();
            scheme_candidates.truncate(remaining);
            candidates.extend(scheme_candidates);
        }

        let expanded_query = expanded_query(&request.query, &candidates);
        Ok(RecallTaxonomyExpansion {
            expanded_query,
            candidates,
        })
    }
}

fn expand_scheme(
    query: &str,
    scheme_id: &str,
    concepts: &[Concept],
    request: &RecallTaxonomyExpansionRequest,
) -> Vec<RecallTaxonomyExpansionCandidate> {
    let query_lc = query.to_lowercase();
    let mut out = Vec::new();
    let mut queue = VecDeque::new();

    for concept in concepts {
        if out.len() >= request.max_candidates as usize {
            break;
        }
        let Some(matched_label) = matched_label(concept, &query_lc) else {
            continue;
        };
        push_candidate(
            &mut out,
            scheme_id,
            concept,
            matched_label,
            None,
            0,
            request.max_candidates,
        );
        queue.push_back((concept.id.to_string(), 0_u8));
    }

    if scheme_id != crate::governance::ZBOT_GENERAL_SCHEME_ID {
        return out;
    }

    let builtin = builtin_starter_skos_scheme();
    while let Some((concept_id, depth)) = queue.pop_front() {
        if depth >= request.max_depth || out.len() >= request.max_candidates as usize {
            continue;
        }
        let Some(source) = builtin
            .concepts
            .iter()
            .find(|concept| concept.id == compact_concept_id(&concept_id))
        else {
            continue;
        };
        let mut edges = Vec::new();
        edges.extend(source.broader.iter().map(|id| ("broader", id)));
        edges.extend(source.narrower.iter().map(|id| ("narrower", id)));
        edges.extend(source.related.iter().map(|id| ("related", id)));
        for (relation, target_id) in edges.into_iter().take(request.max_fan_out as usize) {
            if out.len() >= request.max_candidates as usize {
                break;
            }
            let Some(target) = concepts
                .iter()
                .find(|concept| compact_concept_id(concept.id.as_str()) == target_id.as_str())
            else {
                continue;
            };
            push_candidate(
                &mut out,
                scheme_id,
                target,
                source.pref_label.clone(),
                Some(relation),
                depth + 1,
                request.max_candidates,
            );
            queue.push_back((target.id.to_string(), depth + 1));
        }
    }

    out
}

fn matched_label(concept: &Concept, query_lc: &str) -> Option<String> {
    let pref = concept.pref_label.value.to_lowercase();
    if query_lc.contains(&pref) {
        return Some(concept.pref_label.value.clone());
    }
    concept
        .alt_labels
        .iter()
        .find(|label| query_lc.contains(&label.value.to_lowercase()))
        .map(|label| label.value.clone())
}

fn push_candidate(
    out: &mut Vec<RecallTaxonomyExpansionCandidate>,
    scheme_id: &str,
    concept: &Concept,
    matched_label: String,
    relation: Option<&str>,
    depth: u8,
    max_candidates: u16,
) {
    if out.len() >= max_candidates as usize {
        return;
    }
    let label = concept.pref_label.value.clone();
    let concept_id = concept.id.to_string();
    if out.iter().any(|candidate| {
        candidate.concept_id == concept_id && candidate.relation == relation.map(str::to_string)
    }) {
        return;
    }
    out.push(RecallTaxonomyExpansionCandidate {
        scheme_id: scheme_id.to_string(),
        concept_id,
        label,
        matched_label,
        relation: relation.map(str::to_string),
        depth,
    });
}

fn expanded_query(query: &str, candidates: &[RecallTaxonomyExpansionCandidate]) -> String {
    let mut parts = vec![query.to_string()];
    for candidate in candidates {
        if !parts
            .iter()
            .any(|part| part.eq_ignore_ascii_case(&candidate.label))
        {
            parts.push(candidate.label.clone());
        }
    }
    parts.join(" ")
}

fn compact_concept_id(id: &str) -> &str {
    id.rsplit(":concept:").next().unwrap_or(id)
}
