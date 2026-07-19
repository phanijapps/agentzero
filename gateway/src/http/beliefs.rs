//! # Belief Network HTTP Endpoints (Phase B-5)
//!
//! Read/write surface for the Belief Network UI. Beliefs and
//! contradictions live in `kg_beliefs` and `kg_belief_contradictions`;
//! these handlers route through `state.belief_store` and
//! `state.belief_contradiction_store` so the persistence layer stays
//! abstract.
//!
//! ## Gating
//!
//! The Belief Network is opt-in via
//! `execution.memory.beliefNetwork.enabled`. When the flag is off the
//! store handles are `None` and these endpoints return `503 Service
//! Unavailable` with a helpful message — *not* `404`, because the route
//! exists; the feature is just dormant.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use zbot_stores_traits::{Belief, BeliefContradiction, ContradictionType, Resolution};

// ============================================================================
// REQUEST / RESPONSE TYPES
// ============================================================================

/// Wire shape for `Belief` — flattens the bi-temporal interval and skips
/// the raw `embedding` bytes (UI never needs them).
#[derive(Debug, Serialize)]
pub struct BeliefResponse {
    pub id: String,
    pub partition_id: String,
    pub subject: String,
    pub content: String,
    pub confidence: f64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub source_fact_ids: Vec<String>,
    pub synthesizer_version: i32,
    pub reasoning: Option<String>,
    pub stale: bool,
    pub created_at: String,
    pub updated_at: String,
    pub superseded_by: Option<String>,
}

impl From<Belief> for BeliefResponse {
    fn from(b: Belief) -> Self {
        Self {
            id: b.id,
            partition_id: b.partition_id,
            subject: b.subject,
            content: b.content,
            confidence: b.confidence,
            valid_from: b.valid_from.map(|t| t.to_rfc3339()),
            valid_until: b.valid_until.map(|t| t.to_rfc3339()),
            source_fact_ids: b.source_fact_ids,
            synthesizer_version: b.synthesizer_version,
            reasoning: b.reasoning,
            stale: b.stale,
            created_at: b.created_at.to_rfc3339(),
            updated_at: b.updated_at.to_rfc3339(),
            superseded_by: b.superseded_by,
        }
    }
}

/// Wire shape for `BeliefContradiction` — serializes the enums to the
/// kebab-case strings the UI matches against.
#[derive(Debug, Serialize)]
pub struct BeliefContradictionResponse {
    pub id: String,
    pub belief_a_id: String,
    pub belief_b_id: String,
    pub contradiction_type: &'static str,
    pub severity: f64,
    pub judge_reasoning: Option<String>,
    pub detected_at: String,
    pub resolved_at: Option<String>,
    pub resolution: Option<&'static str>,
}

fn contradiction_type_str(t: &ContradictionType) -> &'static str {
    match t {
        ContradictionType::Logical => "logical",
        ContradictionType::Tension => "tension",
        ContradictionType::Temporal => "temporal",
    }
}

fn resolution_str(r: &Resolution) -> &'static str {
    match r {
        Resolution::AWon => "a_won",
        Resolution::BWon => "b_won",
        Resolution::Compatible => "compatible",
        Resolution::Unresolved => "unresolved",
    }
}

impl From<BeliefContradiction> for BeliefContradictionResponse {
    fn from(c: BeliefContradiction) -> Self {
        Self {
            id: c.id,
            belief_a_id: c.belief_a_id,
            belief_b_id: c.belief_b_id,
            contradiction_type: contradiction_type_str(&c.contradiction_type),
            severity: c.severity,
            judge_reasoning: c.judge_reasoning,
            detected_at: c.detected_at.to_rfc3339(),
            resolved_at: c.resolved_at.map(|t| t.to_rfc3339()),
            resolution: c.resolution.as_ref().map(resolution_str),
        }
    }
}

/// Lightweight summary of a source fact for the detail view — id +
/// content + category + confidence is all the UI needs to render a row.
#[derive(Debug, Serialize)]
pub struct SourceFactSummary {
    pub id: String,
    pub content: String,
    pub category: String,
    pub confidence: f64,
}

/// Full belief detail: belief itself + resolved source facts + any
/// contradictions involving this belief.
#[derive(Debug, Serialize)]
pub struct BeliefDetailResponse {
    pub belief: BeliefResponse,
    pub source_facts: Vec<SourceFactSummary>,
    pub contradictions: Vec<BeliefContradictionResponse>,
}

#[derive(Debug, Serialize)]
pub struct BeliefListResponse {
    pub beliefs: Vec<BeliefResponse>,
}

#[derive(Debug, Serialize)]
pub struct ContradictionListResponse {
    pub contradictions: Vec<BeliefContradictionResponse>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct ListBeliefsQuery {
    #[serde(default = "default_list_limit")]
    pub limit: usize,
    /// Honored client-side — `BeliefStore::list_beliefs` doesn't offer a
    /// native offset, so we slice the returned `limit + offset` rows.
    #[serde(default)]
    pub offset: usize,
}

fn default_list_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
pub struct ListContradictionsQuery {
    #[serde(default = "default_contradiction_limit")]
    pub limit: usize,
}

fn default_contradiction_limit() -> usize {
    20
}

#[derive(Debug, Deserialize)]
pub struct ResolveContradictionRequest {
    pub resolution: String,
}

const BELIEF_DISABLED_MSG: &str =
    "Belief Network is not enabled (set execution.memory.beliefNetwork.enabled = true in settings)";

// ============================================================================
// HANDLERS
// ============================================================================

/// `GET /api/beliefs/:agent_id` — list beliefs in the agent's partition.
///
/// Partition_id is the same string the `belief` agent tool uses — for now
/// the gateway treats `agent_id` as the partition_id (mirroring how the
/// rest of the memory tab maps "agent" to "ward/partition"). Returns
/// 503 when the Belief Network is disabled.
pub async fn list_beliefs(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<ListBeliefsQuery>,
) -> Result<Json<BeliefListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = require_belief_store(&state)?;

    // BeliefStore::list_beliefs is limit-only; we fetch limit+offset and
    // slice client-side. Belief counts are bounded by design (single-digit
    // hundreds per partition), so this is cheap.
    let total_needed = query.limit.saturating_add(query.offset);
    let mut beliefs = store
        .list_beliefs(&agent_id, total_needed)
        .await
        .map_err(internal)?;

    if query.offset >= beliefs.len() {
        return Ok(Json(BeliefListResponse { beliefs: vec![] }));
    }
    let beliefs: Vec<BeliefResponse> = beliefs
        .drain(query.offset..)
        .take(query.limit)
        .map(BeliefResponse::from)
        .collect();

    Ok(Json(BeliefListResponse { beliefs }))
}

/// `GET /api/beliefs/:agent_id/:belief_id` — detail view with resolved
/// source facts and any contradictions involving this belief.
pub async fn get_belief_detail(
    State(state): State<AppState>,
    Path((agent_id, belief_id)): Path<(String, String)>,
) -> Result<Json<BeliefDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = require_belief_store(&state)?;

    let belief = store
        .get_belief_by_id(&belief_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Belief not found"))?;

    if belief.partition_id != agent_id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Belief does not belong to this agent".to_string(),
            }),
        ));
    }

    // Resolve source facts via the existing MemoryFactStore. Facts that
    // fail to load are silently skipped — the UI just won't see them.
    let source_facts = resolve_source_facts(&state, &belief.source_fact_ids).await;

    // Pull contradictions involving this belief. Best-effort: if the
    // contradiction store is missing the detail view still renders.
    let contradictions = match &state.belief_contradiction_store {
        Some(cs) => cs
            .for_belief(&belief.id)
            .await
            .map_err(internal)?
            .into_iter()
            .map(BeliefContradictionResponse::from)
            .collect(),
        None => Vec::new(),
    };

    Ok(Json(BeliefDetailResponse {
        belief: BeliefResponse::from(belief),
        source_facts,
        contradictions,
    }))
}

/// `GET /api/beliefs/:agent_id/:belief_id/contradictions` — list every
/// contradiction (resolved or not) involving this belief.
pub async fn list_belief_contradictions(
    State(state): State<AppState>,
    Path((_agent_id, belief_id)): Path<(String, String)>,
) -> Result<Json<ContradictionListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = require_contradiction_store(&state)?;
    let rows = store.for_belief(&belief_id).await.map_err(internal)?;
    Ok(Json(ContradictionListResponse {
        contradictions: rows.into_iter().map(Into::into).collect(),
    }))
}

/// `GET /api/contradictions/:agent_id` — recent contradictions in the
/// partition, newest first.
pub async fn list_recent_contradictions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<ListContradictionsQuery>,
) -> Result<Json<ContradictionListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = require_contradiction_store(&state)?;
    let rows = store
        .list_recent(&agent_id, query.limit)
        .await
        .map_err(internal)?;
    Ok(Json(ContradictionListResponse {
        contradictions: rows.into_iter().map(Into::into).collect(),
    }))
}

/// `POST /api/contradictions/:contradiction_id/resolve` — mark a
/// contradiction resolved with `a_won`, `b_won`, or `compatible`. The
/// store stamps `resolved_at` to "now".
pub async fn resolve_contradiction(
    State(state): State<AppState>,
    Path(contradiction_id): Path<String>,
    Json(body): Json<ResolveContradictionRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = require_contradiction_store(&state)?;
    let resolution = parse_resolution(&body.resolution)?;
    store
        .resolve(&contradiction_id, resolution)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// HELPERS
// ============================================================================

fn require_belief_store(
    state: &AppState,
) -> Result<&std::sync::Arc<dyn zbot_stores_traits::BeliefStore>, (StatusCode, Json<ErrorResponse>)>
{
    state.belief_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: BELIEF_DISABLED_MSG.to_string(),
            }),
        )
    })
}

fn require_contradiction_store(
    state: &AppState,
) -> Result<
    &std::sync::Arc<dyn zbot_stores_traits::BeliefContradictionStore>,
    (StatusCode, Json<ErrorResponse>),
> {
    state.belief_contradiction_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: BELIEF_DISABLED_MSG.to_string(),
            }),
        )
    })
}

fn internal(e: String) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!("belief endpoint error: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: e }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

fn parse_resolution(s: &str) -> Result<Resolution, (StatusCode, Json<ErrorResponse>)> {
    match s {
        "a_won" => Ok(Resolution::AWon),
        "b_won" => Ok(Resolution::BWon),
        "compatible" => Ok(Resolution::Compatible),
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Invalid resolution '{}' (expected: a_won | b_won | compatible)",
                    other
                ),
            }),
        )),
    }
}

/// Load source-fact summaries for a belief's `source_fact_ids`. Each
/// fact is fetched independently; failures are swallowed so the detail
/// view still renders for callers when one fact has been deleted.
async fn resolve_source_facts(state: &AppState, fact_ids: &[String]) -> Vec<SourceFactSummary> {
    let Some(memory_store) = state.memory_store.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(fact_ids.len());
    for fact_id in fact_ids {
        if let Ok(Some(value)) = memory_store.get_memory_fact_by_id(fact_id).await {
            if let Some(summary) = summarize_fact(fact_id, &value) {
                out.push(summary);
            }
        }
    }
    out
}

fn summarize_fact(fact_id: &str, value: &serde_json::Value) -> Option<SourceFactSummary> {
    let content = value.get("content").and_then(|v| v.as_str())?.to_string();
    let category = value
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let confidence = value
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    Some(SourceFactSummary {
        id: fact_id.to_string(),
        content,
        category,
        confidence,
    })
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_state() -> (TempDir, AppState) {
        let dir = TempDir::new().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        let state = AppState::minimal(dir.path().to_path_buf());
        (dir, state)
    }

    /// With no `belief_store` wired (the `minimal()` AppState path that
    /// the entire HTTP test suite uses), every belief endpoint MUST
    /// return 503 with a clear message — not 404, not 500.
    #[tokio::test]
    async fn list_beliefs_returns_503_when_disabled() {
        let (_dir, state) = make_state();
        let res = list_beliefs(
            State(state),
            Path("root".to_string()),
            Query(ListBeliefsQuery {
                limit: 50,
                offset: 0,
            }),
        )
        .await;
        let (status, body) = res.expect_err("must be Err when disabled");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.0.error.contains("Belief Network is not enabled"));
    }

    #[tokio::test]
    async fn get_belief_detail_returns_503_when_disabled() {
        let (_dir, state) = make_state();
        let res = get_belief_detail(
            State(state),
            Path(("root".to_string(), "belief-1".to_string())),
        )
        .await;
        let (status, _) = res.expect_err("must be Err when disabled");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn list_recent_contradictions_returns_503_when_disabled() {
        let (_dir, state) = make_state();
        let res = list_recent_contradictions(
            State(state),
            Path("root".to_string()),
            Query(ListContradictionsQuery { limit: 20 }),
        )
        .await;
        let (status, _) = res.expect_err("must be Err when disabled");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn list_belief_contradictions_returns_503_when_disabled() {
        let (_dir, state) = make_state();
        let res = list_belief_contradictions(
            State(state),
            Path(("root".to_string(), "belief-1".to_string())),
        )
        .await;
        let (status, _) = res.expect_err("must be Err when disabled");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn resolve_contradiction_returns_503_when_disabled() {
        let (_dir, state) = make_state();
        let res = resolve_contradiction(
            State(state),
            Path("contradiction-1".to_string()),
            Json(ResolveContradictionRequest {
                resolution: "a_won".to_string(),
            }),
        )
        .await;
        let (status, _) = res.expect_err("must be Err when disabled");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn parse_resolution_recognizes_all_valid_values() {
        assert!(matches!(parse_resolution("a_won"), Ok(Resolution::AWon)));
        assert!(matches!(parse_resolution("b_won"), Ok(Resolution::BWon)));
        assert!(matches!(
            parse_resolution("compatible"),
            Ok(Resolution::Compatible)
        ));
    }

    #[test]
    fn parse_resolution_rejects_unknown_value() {
        let err = parse_resolution("unresolved").unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        let err2 = parse_resolution("garbage").unwrap_err();
        assert_eq!(err2.0, StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // With a live BeliefStore wired into AppState, the happy paths must
    // serialize the domain types into the wire shape correctly. We bolt
    // an in-memory stub store directly onto the state so the test path
    // doesn't depend on the SQLite belief schema.
    // -----------------------------------------------------------------------
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use std::sync::Arc;
    use zbot_stores_traits::{BeliefStore, ScoredBelief};

    struct StubBeliefStore {
        beliefs: Vec<Belief>,
    }

    #[async_trait]
    impl BeliefStore for StubBeliefStore {
        async fn get_belief(
            &self,
            _partition_id: &str,
            _subject: &str,
            _as_of: Option<DateTime<Utc>>,
        ) -> Result<Option<Belief>, String> {
            Ok(None)
        }
        async fn list_beliefs(
            &self,
            partition_id: &str,
            limit: usize,
        ) -> Result<Vec<Belief>, String> {
            Ok(self
                .beliefs
                .iter()
                .filter(|b| b.partition_id == partition_id)
                .take(limit)
                .cloned()
                .collect())
        }
        async fn upsert_belief(&self, _b: &Belief) -> Result<(), String> {
            Ok(())
        }
        async fn supersede_belief(
            &self,
            _old: &str,
            _new: &str,
            _t: DateTime<Utc>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn mark_stale(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
        async fn retract_belief(&self, _id: &str, _t: DateTime<Utc>) -> Result<(), String> {
            Ok(())
        }
        async fn beliefs_referencing_fact(&self, _f: &str) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        async fn get_belief_by_id(&self, id: &str) -> Result<Option<Belief>, String> {
            Ok(self.beliefs.iter().find(|b| b.id == id).cloned())
        }
        async fn list_stale(&self, _p: &str, _l: usize) -> Result<Vec<Belief>, String> {
            Ok(vec![])
        }
        async fn clear_stale(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
        async fn search_beliefs(
            &self,
            _p: &str,
            _q: &[f32],
            _l: usize,
        ) -> Result<Vec<ScoredBelief>, String> {
            Ok(vec![])
        }
    }

    fn sample_belief(id: &str, partition: &str, subject: &str) -> Belief {
        let now = Utc::now();
        Belief {
            id: id.to_string(),
            partition_id: partition.to_string(),
            subject: subject.to_string(),
            content: format!("content for {subject}"),
            confidence: 0.85,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec!["fact-1".to_string()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        }
    }

    fn state_with_stub_beliefs(beliefs: Vec<Belief>) -> (TempDir, AppState) {
        let (dir, mut state) = make_state();
        let store: Arc<dyn BeliefStore> = Arc::new(StubBeliefStore { beliefs });
        state.belief_store = Some(store);
        (dir, state)
    }

    #[tokio::test]
    async fn list_beliefs_returns_beliefs_in_partition() {
        let (_dir, state) = state_with_stub_beliefs(vec![
            sample_belief("b1", "root", "user.location"),
            sample_belief("b2", "root", "user.employment"),
            sample_belief("b3", "other", "user.location"),
        ]);
        let res = list_beliefs(
            State(state),
            Path("root".to_string()),
            Query(ListBeliefsQuery {
                limit: 10,
                offset: 0,
            }),
        )
        .await
        .expect("ok");
        assert_eq!(res.0.beliefs.len(), 2);
        assert_eq!(res.0.beliefs[0].id, "b1");
        assert_eq!(res.0.beliefs[1].id, "b2");
    }

    #[tokio::test]
    async fn list_beliefs_honors_offset_and_limit() {
        let (_dir, state) = state_with_stub_beliefs(vec![
            sample_belief("b1", "root", "s1"),
            sample_belief("b2", "root", "s2"),
            sample_belief("b3", "root", "s3"),
            sample_belief("b4", "root", "s4"),
        ]);
        let res = list_beliefs(
            State(state),
            Path("root".to_string()),
            Query(ListBeliefsQuery {
                limit: 2,
                offset: 1,
            }),
        )
        .await
        .expect("ok");
        assert_eq!(res.0.beliefs.len(), 2);
        assert_eq!(res.0.beliefs[0].id, "b2");
        assert_eq!(res.0.beliefs[1].id, "b3");
    }

    #[tokio::test]
    async fn get_belief_detail_404_when_missing() {
        let (_dir, state) = state_with_stub_beliefs(vec![]);
        let res =
            get_belief_detail(State(state), Path(("root".to_string(), "nope".to_string()))).await;
        let (status, _) = res.expect_err("err");
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_belief_detail_403_when_partition_mismatch() {
        let (_dir, state) = state_with_stub_beliefs(vec![sample_belief("b1", "root", "s1")]);
        let res =
            get_belief_detail(State(state), Path(("other".to_string(), "b1".to_string()))).await;
        let (status, _) = res.expect_err("err");
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}
