//! `POST /api/memory/search` — unified hybrid search across memory types.
//!
//! Fans out a single query to up to four content types (facts, wiki,
//! procedures, episodes) in parallel and returns one block per type with
//! `hits` and a `latency_ms` timer. Modes:
//! - `hybrid` (default): FTS + vec with RRF where available; vec-only for
//!   procedures (no FTS table); vec + LIKE fallback for episodes
//! - `fts`: never embeds the query; procedures return empty; episodes use a
//!   LIKE fallback over `task_summary`/`key_learnings`
//! - `semantic`: embedding required (returns 400 if unavailable); vec-only
//!   across all selected types
//!
//! `limit` applies
//! per-type, not globally.
//!
//! All four search paths route through the trait stores
//! (`memory_store`, `wiki_store`, `procedure_store`, `episode_store`)
//! on `AppState`. Returns `503 Service Unavailable` when a store isn't
//! wired (stripped-down test fixtures).

use crate::state::AppState;
use agent_runtime::llm::embedding::EmbeddingClient;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;
use zbot_stores_domain::{RouteHint, RouteSourceKind, SessionEpisode, WikiHit};
use zbot_stores_traits::EmbeddingQueryIdentity;

fn route_hint_value(hint: RouteHint) -> Value {
    serde_json::to_value(hint).unwrap_or(Value::Null)
}

/// Request body for unified search.
#[derive(Debug, Deserialize)]
pub struct SearchBody {
    pub query: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_types")]
    pub types: Vec<String>,
    #[serde(default)]
    pub ward_ids: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Optional agent scope. When present, memory-fact queries are restricted
    /// to facts the given agent can see (its private `scope='agent'` facts
    /// plus all `scope='global'` facts). When absent, no agent filter is
    /// applied — useful for admin/debug views of the full pool.
    #[serde(default)]
    pub agent_id: Option<String>,
}

fn default_mode() -> String {
    "hybrid".into()
}

fn default_types() -> Vec<String> {
    vec![
        "facts".into(),
        "wiki".into(),
        "procedures".into(),
        "episodes".into(),
    ]
}

fn default_limit() -> usize {
    10
}

fn validate_mode(mode: &str) -> Result<&str, HandlerError> {
    match mode {
        "hybrid" | "fts" | "semantic" => Ok(mode),
        other => Err(err(
            StatusCode::BAD_REQUEST,
            format!("unsupported memory search mode: {other}"),
        )),
    }
}

/// One type's block in the response.
#[derive(Debug, Serialize, Default)]
pub struct TypeBlock {
    pub hits: Vec<Value>,
    pub latency_ms: u64,
}

/// Unified response with one block per content type.
#[derive(Debug, Serialize, Default)]
pub struct UnifiedResponse {
    pub facts: TypeBlock,
    pub wiki: TypeBlock,
    pub procedures: TypeBlock,
    pub episodes: TypeBlock,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

pub type HandlerError = (StatusCode, Json<ErrorBody>);

fn err(status: StatusCode, msg: impl Into<String>) -> HandlerError {
    (status, Json(ErrorBody { error: msg.into() }))
}

/// Error helper: returns 503 when a trait store isn't wired (e.g.
/// stripped-down test fixtures).
fn store_unavailable() -> HandlerError {
    err(StatusCode::SERVICE_UNAVAILABLE, "store unavailable")
}

fn wiki_hit_to_value(hit: WikiHit) -> Value {
    let snippet: String = hit.article.content.chars().take(240).collect();
    let route_hint = RouteHint::new(hit.article.ward_id.clone(), RouteSourceKind::WikiArticle)
        .with_memory_id(hit.article.id.clone());
    json!({
        "id": hit.article.id,
        "ward_id": hit.article.ward_id,
        "title": hit.article.title,
        "snippet": snippet,
        "updated_at": hit.article.updated_at,
        "score": hit.score,
        "match_source": hit.match_source,
        "route_hint": route_hint_value(route_hint) })
}

fn procedure_to_value(proc: zbot_stores_domain::Procedure, score: f64) -> Value {
    let route_hint = proc
        .ward_id
        .as_ref()
        .map(|ward| {
            route_hint_value(
                RouteHint::new(ward.clone(), RouteSourceKind::Procedure)
                    .with_memory_id(proc.id.clone()),
            )
        })
        .unwrap_or(Value::Null);
    json!({
        "id": proc.id,
        "agent_id": proc.agent_id,
        "ward_id": proc.ward_id,
        "name": proc.name,
        "description": proc.description,
        "success_count": proc.success_count,
        "failure_count": proc.failure_count,
        "last_used": proc.last_used,
        "created_at": proc.created_at,
        "updated_at": proc.updated_at,
        "score": score,
        "match_source": "vec",
        "route_hint": route_hint })
}

fn episode_to_value(ep: SessionEpisode, score: Option<f64>, source: &str) -> Value {
    let route_hint = RouteHint::new(ep.ward_id.clone(), RouteSourceKind::Episode)
        .with_memory_id(ep.id.clone())
        .with_session_id(Some(ep.session_id.clone()));
    let mut v = json!({
        "id": ep.id,
        "session_id": ep.session_id,
        "agent_id": ep.agent_id,
        "ward_id": ep.ward_id,
        "task_summary": ep.task_summary,
        "outcome": ep.outcome,
        "strategy_used": ep.strategy_used,
        "key_learnings": ep.key_learnings,
        "token_cost": ep.token_cost,
        "created_at": ep.created_at,
        "match_source": source,
        "route_hint": route_hint_value(route_hint) });
    if let (Value::Object(ref mut m), Some(s)) = (&mut v, score) {
        m.insert("score".into(), json!(s));
    }
    v
}

fn fact_to_value(fact: zbot_stores_domain::MemoryFact, source: &str, score: Option<f64>) -> Value {
    let public_source = public_match_source(source);
    let route_hint = RouteHint::new(fact.ward_id.clone(), RouteSourceKind::Fact)
        .with_memory_id(fact.id.clone())
        .with_session_id(fact.session_id.clone())
        .with_source_path(fact.source_ref.clone());
    let mut v = json!({
        "id": fact.id,
        "session_id": fact.session_id,
        "agent_id": fact.agent_id,
        "scope": fact.scope,
        "category": fact.category,
        "key": fact.key,
        "content": fact.content,
        "confidence": fact.confidence,
        "mention_count": fact.mention_count,
        "ward_id": fact.ward_id,
        "created_at": fact.created_at,
        "updated_at": fact.updated_at,
        "pinned": fact.pinned,
        "epistemic_class": fact.epistemic_class,
        "match_source": public_source,
        "route_hint": route_hint_value(route_hint) });
    if let (Value::Object(ref mut m), Some(s)) = (&mut v, score) {
        m.insert("score".into(), json!(s));
    }
    v
}

fn public_match_source(source: &str) -> &str {
    match source {
        "exact_degraded" => "fts",
        other => other,
    }
}

/// Fallback agent id used when scoping procedure / episode searches in v1
/// and no `agent_id` was supplied on the request. Procedure/episode repos
/// still require `&str`; the unified-search API will pass the caller's agent
/// when provided and fall back to `"root"` (the production root agent id)
/// otherwise. Memory-fact queries no longer rely on this — they accept
/// `Option<&str>` directly.
const FALLBACK_AGENT: &str = "root";

/// POST /api/memory/search — unified hybrid/fts/semantic search.
pub async fn memory_search(
    State(state): State<AppState>,
    Json(req): Json<SearchBody>,
) -> Result<Json<UnifiedResponse>, HandlerError> {
    let memory_store = state
        .memory_store()
        .as_ref()
        .ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "memory store unavailable"))?
        .clone();
    let wiki_store = state
        .wiki_store()
        .as_ref()
        .ok_or_else(store_unavailable)?
        .clone();
    let proc_store = state
        .procedure_store()
        .as_ref()
        .ok_or_else(store_unavailable)?
        .clone();
    let episode_store = state
        .episode_store()
        .as_ref()
        .ok_or_else(store_unavailable)?
        .clone();

    let ward: Option<String> = req.ward_ids.first().cloned();
    let ward_ref: Option<&str> = ward.as_deref();

    // Optional caller-scoped agent. `None` → no agent/scope gate (admin/debug).
    let agent: Option<String> = req.agent_id.clone();
    let embedding_client = state.embedding_service().client();
    let mode = validate_mode(req.mode.as_str())?;

    // Single embedding attempt, mode-dependent.
    let embedding: Option<Vec<f32>> = match mode {
        "fts" => None,
        "semantic" => {
            let v = embedding_client
                .embed(&[req.query.as_str()])
                .await
                .map_err(|e| {
                    tracing::debug!("unified semantic search: embedding unavailable ({e})");
                    err(StatusCode::BAD_REQUEST, "embedding backend unavailable")
                })?;
            Some(v.into_iter().next().unwrap_or_default())
        }
        _ => match embedding_client.embed(&[req.query.as_str()]).await {
            Ok(v) => v.into_iter().next(),
            Err(e) => {
                tracing::debug!("unified search: embedding unavailable ({e}); FTS-only");
                None
            }
        },
    };
    let query_identity = embedding
        .as_ref()
        .map(|_| embedding_query_identity(embedding_client.as_ref()));

    let want_facts = req.types.iter().any(|t| t == "facts");
    let want_wiki = req.types.iter().any(|t| t == "wiki");
    let want_proc = req.types.iter().any(|t| t == "procedures");
    let want_eps = req.types.iter().any(|t| t == "episodes");

    let query = req.query.clone();
    let mode = mode.to_string();
    let limit = req.limit;
    let ward_owned = ward.clone();
    let agent_owned = agent.clone();

    let facts_fut = {
        let memory_store = memory_store.clone();
        let query = query.clone();
        let mode = mode.clone();
        let emb = embedding.clone();
        let query_identity = query_identity.clone();
        let ward = ward_owned.clone();
        let agent = agent_owned.clone();
        async move {
            if !want_facts {
                return TypeBlock::default();
            }
            let t0 = Instant::now();
            let hits = run_facts(
                memory_store.as_ref(),
                &query,
                &mode,
                emb.as_deref(),
                query_identity.as_ref(),
                agent.as_deref(),
                ward.as_deref(),
                limit,
            )
            .await;
            TypeBlock {
                hits,
                latency_ms: t0.elapsed().as_millis() as u64,
            }
        }
    };

    let wiki_fut = {
        let wiki_store = wiki_store.clone();
        let query = query.clone();
        let mode = mode.clone();
        let emb = embedding.clone();
        let query_identity = query_identity.clone();
        let ward = ward_owned.clone();
        async move {
            if !want_wiki {
                return TypeBlock::default();
            }
            let t0 = Instant::now();
            if mode == "hybrid" && emb.is_none() {
                return TypeBlock {
                    hits: Vec::new(),
                    latency_ms: t0.elapsed().as_millis() as u64,
                };
            }
            // In semantic-only mode, pass embedding but use a synthetic query
            // the FTS arm won't match (search_hybrid still runs FTS with it —
            // acceptable: no harm since RRF will favor vec hits).
            let pass_query = if mode == "semantic" {
                ""
            } else {
                query.as_str()
            };
            let hits = wiki_store
                .search_wiki_hybrid_typed_with_identity(
                    ward.as_deref(),
                    pass_query,
                    limit,
                    emb.as_deref(),
                    query_identity.as_ref(),
                )
                .await
                .unwrap_or_default()
                .into_iter()
                .map(wiki_hit_to_value)
                .collect();
            TypeBlock {
                hits,
                latency_ms: t0.elapsed().as_millis() as u64,
            }
        }
    };

    let proc_fut = {
        let proc_store = proc_store.clone();
        let mode = mode.clone();
        let emb = embedding.clone();
        let query_identity = query_identity.clone();
        let ward = ward_owned.clone();
        let agent = agent_owned.clone();
        async move {
            if !want_proc {
                return TypeBlock::default();
            }
            let t0 = Instant::now();
            let scope_agent = agent.as_deref().unwrap_or(FALLBACK_AGENT);
            let hits: Vec<Value> = match (mode.as_str(), emb.as_ref()) {
                // No FTS table for procedures: fts mode returns empty.
                ("fts", _) => Vec::new(),
                (_, Some(emb)) => proc_store
                    .search_procedures_by_similarity_typed_with_identity(
                        emb,
                        query_identity.as_ref(),
                        scope_agent,
                        ward.as_deref(),
                        limit,
                    )
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(p, s)| procedure_to_value(p, s))
                    .collect(),
                (_, None) => Vec::new(),
            };
            TypeBlock {
                hits,
                latency_ms: t0.elapsed().as_millis() as u64,
            }
        }
    };

    let eps_fut = {
        let episode_store = episode_store.clone();
        let query = query.clone();
        let mode = mode.clone();
        let emb = embedding.clone();
        let query_identity = query_identity.clone();
        let ward = ward_owned.clone();
        let agent = agent_owned.clone();
        async move {
            if !want_eps {
                return TypeBlock::default();
            }
            let t0 = Instant::now();
            let scope_agent = agent.as_deref().unwrap_or(FALLBACK_AGENT);
            let hits: Vec<Value> = match (mode.as_str(), emb.as_ref()) {
                // Pure FTS path → LIKE fallback (no FTS5 partner table for episodes).
                ("fts", _) => episode_store
                    .keyword_search_episodes(&query, ward.as_deref(), limit)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|ep| episode_to_value(ep, None, "fts"))
                    .collect(),
                (_, Some(emb)) => episode_store
                    .search_episodes_by_similarity_typed_with_identity(
                        scope_agent,
                        emb,
                        query_identity.as_ref(),
                        0.0,
                        limit,
                    )
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    // Ward-scope filter in Rust since the trait method does
                    // not filter by ward.
                    .filter(|(ep, _)| ward.as_deref().is_none_or(|w| ep.ward_id == w))
                    .map(|(ep, s)| episode_to_value(ep, Some(s), "vec"))
                    .collect(),
                ("hybrid", None) => Vec::new(),
                (_, None) => episode_store
                    .keyword_search_episodes(&query, ward.as_deref(), limit)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|ep| episode_to_value(ep, None, "fts"))
                    .collect(),
            };
            TypeBlock {
                hits,
                latency_ms: t0.elapsed().as_millis() as u64,
            }
        }
    };

    let _ = ward_ref; // silence unused if branches skip
    let (facts, wiki, procedures, episodes) = tokio::join!(facts_fut, wiki_fut, proc_fut, eps_fut);

    Ok(Json(UnifiedResponse {
        facts,
        wiki,
        procedures,
        episodes,
    }))
}

/// Route a fact search through the memory store according to mode. Returns
/// JSON-ready hits.
///
/// `agent_id` is threaded straight into the trait method — `Some(a)` yields
/// scope-aware results (agent's private + global), `None` returns the
/// unfiltered pool (admin/debug).
#[allow(clippy::too_many_arguments)]
async fn run_facts(
    memory_store: &dyn zbot_stores_traits::MemoryFactStore,
    query: &str,
    mode: &str,
    embedding: Option<&[f32]>,
    query_identity: Option<&EmbeddingQueryIdentity>,
    agent_id: Option<&str>,
    ward: Option<&str>,
    limit: usize,
) -> Vec<Value> {
    if mode == "semantic" && embedding.is_none() {
        return Vec::new();
    }
    memory_store
        .search_memory_facts_hybrid_typed_with_identity(
            agent_id,
            query,
            mode,
            limit,
            ward,
            embedding,
            query_identity,
            None,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|(fact, _, _)| is_public_fact(fact))
        .map(|(fact, score, src)| fact_to_value(fact, &src, Some(score)))
        .collect()
}

fn is_public_fact(fact: &zbot_stores_domain::MemoryFact) -> bool {
    !matches!(fact.category.as_str(), "ctx" | "instruction" | "correction")
}

fn embedding_query_identity(client: &dyn EmbeddingClient) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: client.provider_type(),
        model: client.model_name(),
        dimensions: client.dimensions() as u32,
        prompt_profile: client.prompt_profile(),
        normalization: client.normalization(),
    }
}

#[cfg(test)]
mod helpers_tests {
    use super::*;
    use zbot_stores_domain::{MemoryFact, Procedure, SessionEpisode, WikiArticle, WikiHit};
    use zbot_stores_traits::StoreResult;

    fn fact() -> MemoryFact {
        MemoryFact {
            id: "fact-1".into(),
            session_id: Some("sess-1".into()),
            agent_id: "root".into(),
            scope: "agent".into(),
            category: "preference".into(),
            key: "tone".into(),
            content: "be concise".into(),
            confidence: 0.9,
            mention_count: 3,
            source_summary: None,
            embedding: None,
            ward_id: "lab".into(),
            contradicted_by: None,
            created_at: "2026-04-01T00:00:00Z".into(),
            updated_at: "2026-04-02T00:00:00Z".into(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("convention".into()),
            source_episode_id: None,
            source_ref: None,
            last_accessed: None,
        }
    }

    fn wiki() -> WikiArticle {
        WikiArticle {
            id: "wiki-1".into(),
            ward_id: "lab".into(),
            agent_id: "root".into(),
            title: "Index".into(),
            content: "lorem ipsum dolor sit amet".repeat(20),
            tags: None,
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: "2026-04-01T00:00:00Z".into(),
            updated_at: "2026-04-02T00:00:00Z".into(),
        }
    }

    fn proc() -> Procedure {
        Procedure {
            id: "proc-1".into(),
            agent_id: "root".into(),
            ward_id: Some("lab".into()),
            name: "build".into(),
            description: "build the project".into(),
            trigger_pattern: None,
            steps: "[]".into(),
            parameters: None,
            success_count: 5,
            failure_count: 1,
            avg_duration_ms: Some(120),
            avg_token_cost: None,
            last_used: Some("2026-04-02T00:00:00Z".into()),
            embedding: None,
            created_at: "2026-04-01T00:00:00Z".into(),
            updated_at: "2026-04-02T00:00:00Z".into(),
        }
    }

    fn episode() -> SessionEpisode {
        SessionEpisode {
            id: "ep-1".into(),
            session_id: "sess-1".into(),
            agent_id: "root".into(),
            ward_id: "lab".into(),
            task_summary: "fixed bug".into(),
            outcome: "success".into(),
            strategy_used: Some("split-and-trace".into()),
            key_learnings: Some("logs are gold".into()),
            token_cost: Some(2048),
            embedding: None,
            created_at: "2026-04-02T00:00:00Z".into(),
        }
    }

    #[test]
    fn defaults_match_expected_values() {
        assert_eq!(default_mode(), "hybrid");
        assert_eq!(default_limit(), 10);
        let types = default_types();
        assert_eq!(types.len(), 4);
        assert!(types.contains(&"facts".to_string()));
        assert!(types.contains(&"wiki".to_string()));
        assert!(types.contains(&"procedures".to_string()));
        assert!(types.contains(&"episodes".to_string()));
    }

    #[test]
    fn err_helper_emits_status_and_body() {
        let (status, body) = err(StatusCode::BAD_REQUEST, "boom");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.0.error, "boom");
    }

    #[test]
    fn store_unavailable_returns_503() {
        let (status, body) = store_unavailable();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0.error, "store unavailable");
    }

    #[test]
    fn wiki_hit_to_value_truncates_snippet_to_240_chars() {
        let hit = WikiHit {
            article: wiki(),
            score: 0.42,
            match_source: "fts".into(),
        };
        let v = wiki_hit_to_value(hit);
        let snippet = v["snippet"].as_str().unwrap();
        assert!(snippet.chars().count() <= 240);
        assert_eq!(v["score"], 0.42);
        assert_eq!(v["match_source"], "fts");
        assert_eq!(v["title"], "Index");
        assert_eq!(v["ward_id"], "lab");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "wiki_article");
        assert_eq!(v["route_hint"]["memory_id"], "wiki-1");
    }

    #[test]
    fn procedure_to_value_emits_match_source_vec() {
        let v = procedure_to_value(proc(), 0.7);
        assert_eq!(v["score"], 0.7);
        assert_eq!(v["match_source"], "vec");
        assert_eq!(v["name"], "build");
        assert_eq!(v["success_count"], 5);
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "procedure");
        assert_eq!(v["route_hint"]["memory_id"], "proc-1");
    }

    #[test]
    fn episode_to_value_with_score_includes_score_field() {
        let v = episode_to_value(episode(), Some(0.5), "vec");
        assert_eq!(v["match_source"], "vec");
        assert_eq!(v["score"], 0.5);
        assert_eq!(v["task_summary"], "fixed bug");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "episode");
        assert_eq!(v["route_hint"]["session_id"], "sess-1");
    }

    #[test]
    fn episode_to_value_without_score_omits_score_field() {
        let v = episode_to_value(episode(), None, "fts");
        assert_eq!(v["match_source"], "fts");
        assert!(v.get("score").is_none() || v.get("score") == Some(&Value::Null));
    }

    #[test]
    fn fact_to_value_with_score_includes_score() {
        let v = fact_to_value(fact(), "fts", Some(0.91));
        assert_eq!(v["score"], 0.91);
        assert_eq!(v["match_source"], "fts");
        assert_eq!(v["agent_id"], "root");
        assert_eq!(v["pinned"], false);
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "fact");
        assert_eq!(v["route_hint"]["memory_id"], "fact-1");
    }

    #[test]
    fn fact_to_value_without_score_omits_score() {
        let v = fact_to_value(fact(), "vec", None);
        assert_eq!(v["match_source"], "vec");
        assert!(v.get("score").is_none() || v.get("score") == Some(&Value::Null));
    }

    /// Stub store — every other method on `MemoryFactStore` has a default
    /// impl; we only need to implement the two non-default methods.
    struct StubStore;
    #[async_trait::async_trait]
    impl zbot_stores_traits::MemoryFactStore for StubStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> StoreResult<Value> {
            unreachable!()
        }
        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: usize,
        ) -> StoreResult<Value> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn run_facts_in_semantic_mode_without_embedding_returns_empty() {
        let store = StubStore;
        let out = run_facts(&store, "anything", "semantic", None, None, None, None, 10).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn run_facts_in_fts_mode_with_default_store_returns_empty() {
        let store = StubStore;
        let out = run_facts(
            &store,
            "build",
            "fts",
            None,
            None,
            Some("root"),
            Some("lab"),
            10,
        )
        .await;
        assert!(out.is_empty());
    }
}
