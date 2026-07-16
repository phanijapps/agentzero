//! Test-only ontology retrieval quality gate.
//!
//! This evaluator deliberately consumes frozen ranked IDs instead of invoking
//! production retrieval. It records whether a future ontology-aware candidate
//! has earned a separate implementation spec without smuggling ontology logic
//! into the live ranking path.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

const FIXTURE: &str = include_str!(
    "../../../../docs/specs/unified-recall-default/ontology-retrieval-evaluation.json"
);
const PRODUCTION_RECALL: &str = include_str!("mod.rs");
const K: usize = 5;
const REQUIRED_MEAN_NDCG_IMPROVEMENT: f64 = 0.05;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationFixture {
    fixture_version: u32,
    baseline_configuration_fingerprint: String,
    candidate_configuration_fingerprint: String,
    expected_decision: String,
    cases: Vec<EvaluationCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationCase {
    id: String,
    query: String,
    scope: EvaluationScope,
    candidate_universe: Vec<String>,
    relevance: BTreeMap<String, u8>,
    baseline_ranked_ids: Vec<String>,
    candidate_ranked_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EvaluationScope {
    tenant: String,
    ward: String,
    #[serde(default)]
    session: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationReport {
    fixture_version: u32,
    case_count: usize,
    baseline_configuration_fingerprint: String,
    candidate_configuration_fingerprint: String,
    baseline_mean_ndcg_at_5: f64,
    candidate_mean_ndcg_at_5: f64,
    mean_ndcg_delta: f64,
    top_1_regressions: usize,
    decision: &'static str,
    cases: Vec<CaseReport>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReport {
    id: String,
    baseline_ndcg_at_5: f64,
    candidate_ndcg_at_5: f64,
    baseline_top_1_grade: u8,
    candidate_top_1_grade: u8,
}

fn evaluate(fixture: &EvaluationFixture) -> Result<EvaluationReport, String> {
    if fixture.fixture_version != 1 || fixture.cases.len() < 12 {
        return Err("fixture version or case count is invalid".to_string());
    }
    if fixture.baseline_configuration_fingerprint.trim().is_empty()
        || fixture
            .candidate_configuration_fingerprint
            .trim()
            .is_empty()
    {
        return Err("configuration fingerprints are required".to_string());
    }

    let mut reports = Vec::with_capacity(fixture.cases.len());
    for case in &fixture.cases {
        validate_case(case)?;
        reports.push(CaseReport {
            id: case.id.clone(),
            baseline_ndcg_at_5: ndcg_at_k(&case.baseline_ranked_ids, &case.relevance, K),
            candidate_ndcg_at_5: ndcg_at_k(&case.candidate_ranked_ids, &case.relevance, K),
            baseline_top_1_grade: top_1_grade(&case.baseline_ranked_ids, &case.relevance),
            candidate_top_1_grade: top_1_grade(&case.candidate_ranked_ids, &case.relevance),
        });
    }

    let case_count = reports.len() as f64;
    let baseline_mean_ndcg_at_5 = reports
        .iter()
        .map(|case| case.baseline_ndcg_at_5)
        .sum::<f64>()
        / case_count;
    let candidate_mean_ndcg_at_5 = reports
        .iter()
        .map(|case| case.candidate_ndcg_at_5)
        .sum::<f64>()
        / case_count;
    let mean_ndcg_delta = candidate_mean_ndcg_at_5 - baseline_mean_ndcg_at_5;
    let top_1_regressions = reports
        .iter()
        .filter(|case| case.candidate_top_1_grade < case.baseline_top_1_grade)
        .count();
    let decision = if mean_ndcg_delta >= REQUIRED_MEAN_NDCG_IMPROVEMENT && top_1_regressions == 0 {
        "go"
    } else {
        "no_go"
    };

    Ok(EvaluationReport {
        fixture_version: fixture.fixture_version,
        case_count: reports.len(),
        baseline_configuration_fingerprint: fixture.baseline_configuration_fingerprint.clone(),
        candidate_configuration_fingerprint: fixture.candidate_configuration_fingerprint.clone(),
        baseline_mean_ndcg_at_5,
        candidate_mean_ndcg_at_5,
        mean_ndcg_delta,
        top_1_regressions,
        decision,
        cases: reports,
    })
}

fn validate_case(case: &EvaluationCase) -> Result<(), String> {
    if case.id.trim().is_empty()
        || case.query.trim().is_empty()
        || case.scope.tenant.trim().is_empty()
        || case.scope.ward.trim().is_empty()
    {
        return Err("case identity or scope is invalid".to_string());
    }
    if case
        .scope
        .session
        .as_deref()
        .is_some_and(|session| session.trim().is_empty())
    {
        return Err("case session scope is invalid".to_string());
    }
    let universe = case.candidate_universe.iter().collect::<BTreeSet<_>>();
    if universe.len() != case.candidate_universe.len()
        || case.relevance.len() != universe.len()
        || case.relevance.keys().any(|id| !universe.contains(id))
        || case.relevance.values().any(|grade| *grade > 2)
    {
        return Err("case candidate universe or relevance grades are invalid".to_string());
    }
    for ranking in [&case.baseline_ranked_ids, &case.candidate_ranked_ids] {
        let ids = ranking.iter().collect::<BTreeSet<_>>();
        if ranking.len() != ids.len() || ranking.iter().any(|id| !universe.contains(id)) {
            return Err("case ranked IDs are outside the frozen universe".to_string());
        }
    }
    Ok(())
}

fn ndcg_at_k(ranked_ids: &[String], relevance: &BTreeMap<String, u8>, k: usize) -> f64 {
    let dcg = ranked_ids
        .iter()
        .take(k)
        .enumerate()
        .map(|(index, id)| discounted_gain(*relevance.get(id).unwrap_or(&0), index))
        .sum::<f64>();
    let mut ideal = relevance.values().copied().collect::<Vec<_>>();
    ideal.sort_unstable_by(|left, right| right.cmp(left));
    let idcg = ideal
        .into_iter()
        .take(k)
        .enumerate()
        .map(|(index, grade)| discounted_gain(grade, index))
        .sum::<f64>();
    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

fn discounted_gain(grade: u8, zero_indexed_rank: usize) -> f64 {
    (2_f64.powi(i32::from(grade)) - 1.0) / ((zero_indexed_rank + 2) as f64).log2()
}

fn top_1_grade(ranked_ids: &[String], relevance: &BTreeMap<String, u8>) -> u8 {
    ranked_ids
        .first()
        .and_then(|id| relevance.get(id))
        .copied()
        .unwrap_or(0)
}

#[test]
fn ontology_retrieval_evaluation_records_a_no_go_without_production_ranking() {
    let fixture = serde_json::from_str::<EvaluationFixture>(FIXTURE).expect("fixture");
    let report = evaluate(&fixture).expect("evaluation");
    let json = serde_json::to_value(&report).expect("report JSON");

    assert_eq!(report.case_count, 12);
    assert_eq!(report.decision, fixture.expected_decision);
    assert_eq!(report.decision, "no_go");
    assert!(report.mean_ndcg_delta < REQUIRED_MEAN_NDCG_IMPROVEMENT);
    assert_eq!(report.top_1_regressions, 0);
    assert_eq!(json["decision"], "no_go");
    assert_eq!(json["cases"].as_array().map(Vec::len), Some(12));
    assert!(json.get("baselineConfigurationFingerprint").is_some());
    assert!(json.get("candidateConfigurationFingerprint").is_some());

    // The quality gate does not authorize a production ontology retrieval
    // implementation. Keep the live unified-recall source free of ontology
    // rewriting, filtering, and ranking until a separate approved spec exists.
    let live_recall_source =
        PRODUCTION_RECALL.replace("#[cfg(test)]\nmod ontology_retrieval_evaluation;", "");
    assert!(!live_recall_source.contains("ontology"));
}
