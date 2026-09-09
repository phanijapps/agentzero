//! `GET /api/wards/:ward_id/content` — ward content aggregator.
//!
//! Returns a single snapshot of everything that belongs to one ward: the four
//! content types (facts, wiki, procedures, episodes), counts per type, and a
//! derived summary sourced from the ward's `__index__` wiki article (if any).
//! Each item is stamped with a server-computed `age_bucket` (inline helper)
//! so the UI doesn't need to reimplement recency classification.
//!
//! Limits: facts, wiki and procedures are capped at 100 rows; episodes at 50.
//!
//! All four content types are read through trait stores
//! (`memory_store`, `episode_store`, `wiki_store`, `procedure_store`).
//! Requests that arrive before stores are wired return `503 Service
//! Unavailable`.

use super::ErrorResponse;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use zbot_stores_domain::{
    MemoryFact, Procedure, RouteHint, RouteSourceKind, SessionEpisode, WikiArticle,
};

/// Classify a timestamp into a human-meaningful recency bucket relative to `now`.
/// Returns one of: "today", "last_7_days", "historical".
fn age_bucket(now: DateTime<Utc>, created_at: DateTime<Utc>) -> &'static str {
    let age = now.signed_duration_since(created_at);
    if age < Duration::hours(24) {
        "today"
    } else if age < Duration::days(7) {
        "last_7_days"
    } else {
        "historical"
    }
}

const FACT_LIMIT: usize = 100;
const WIKI_LIMIT: usize = 100;
const PROCEDURE_LIMIT: usize = 100;
const EPISODE_LIMIT: usize = 50;

/// Response body for `GET /api/wards/:ward_id/content`.
#[derive(Debug, Serialize)]
pub struct WardContentResponse {
    pub ward_id: String,
    pub summary: WardSummary,
    pub facts: Vec<Value>,
    pub wiki: Vec<Value>,
    pub procedures: Vec<Value>,
    pub episodes: Vec<Value>,
    pub counts: Counts,
}

/// Derived summary for a ward — title/description/updated_at pulled from the
/// ward's `__index__` wiki article when present; falls back to the ward id.
#[derive(Debug, Serialize)]
pub struct WardSummary {
    pub title: String,
    pub description: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct Counts {
    pub facts: usize,
    pub wiki: usize,
    pub procedures: usize,
    pub episodes: usize,
}

pub type HandlerError = (StatusCode, Json<ErrorResponse>);

fn internal(context: &str, e: impl std::fmt::Display) -> HandlerError {
    tracing::error!("{}: {}", context, e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(format!("{}: {}", context, e))),
    )
}

/// Parse an RFC-3339 timestamp into UTC; returns `None` for blanks or malformed
/// input.
fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Attach `age_bucket` to a serialized item relative to `now`, using
/// `created_at` as the recency anchor when `anchor` is `None`.
fn stamp(mut value: Value, now: DateTime<Utc>, anchor: Option<&str>) -> Value {
    let bucket = anchor
        .and_then(parse_ts)
        .map(|ts| age_bucket(now, ts))
        .unwrap_or("historical");
    if let Value::Object(ref mut map) = value {
        map.insert("age_bucket".to_string(), Value::String(bucket.to_string()));
    }
    value
}

fn route_hint_value(hint: RouteHint) -> Value {
    serde_json::to_value(hint).unwrap_or(Value::Null)
}

fn first_non_empty_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn build_summary(ward_id: &str, wiki: &[WikiArticle]) -> WardSummary {
    if let Some(idx) = wiki.iter().find(|a| a.title == "__index__") {
        return WardSummary {
            title: ward_id.to_string(),
            description: first_non_empty_line(&idx.content),
            updated_at: Some(idx.updated_at.clone()),
        };
    }
    WardSummary {
        title: ward_id.to_string(),
        description: None,
        updated_at: None,
    }
}

/// Error helper: the relevant trait store isn't wired (e.g. partial test
/// state). Returns 503 so the UI can surface a clean error instead of a
/// panic.
fn store_unavailable(what: &str) -> HandlerError {
    ErrorResponse::service_disabled(&format!("{what} store unavailable"))
}

fn fact_to_value(fact: MemoryFact, now: DateTime<Utc>) -> Value {
    let updated = fact.updated_at.clone();
    let route_hint = RouteHint::new(fact.ward_id.clone(), RouteSourceKind::Fact)
        .with_memory_id(fact.id.clone())
        .with_session_id(fact.session_id.clone())
        .with_source_path(fact.source_ref.clone());
    let body = json!({
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
        "route_hint": route_hint_value(route_hint) });
    stamp(body, now, Some(&updated))
}

fn wiki_to_value(article: WikiArticle, now: DateTime<Utc>) -> Value {
    let updated = article.updated_at.clone();
    let route_hint = RouteHint::new(article.ward_id.clone(), RouteSourceKind::WikiArticle)
        .with_memory_id(article.id.clone());
    let body = json!({
        "id": article.id,
        "ward_id": article.ward_id,
        "agent_id": article.agent_id,
        "title": article.title,
        "content": article.content,
        "tags": article.tags,
        "version": article.version,
        "created_at": article.created_at,
        "updated_at": article.updated_at,
        "route_hint": route_hint_value(route_hint) });
    stamp(body, now, Some(&updated))
}

fn procedure_to_value(proc: Procedure, now: DateTime<Utc>) -> Value {
    // Prefer `last_used` as the recency anchor if present, else fall back to
    // `created_at`.
    let anchor = proc
        .last_used
        .clone()
        .unwrap_or_else(|| proc.created_at.clone());
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
    let body = json!({
        "id": proc.id,
        "agent_id": proc.agent_id,
        "ward_id": proc.ward_id,
        "name": proc.name,
        "description": proc.description,
        "trigger_pattern": proc.trigger_pattern,
        "success_count": proc.success_count,
        "failure_count": proc.failure_count,
        "avg_duration_ms": proc.avg_duration_ms,
        "last_used": proc.last_used,
        "created_at": proc.created_at,
        "updated_at": proc.updated_at,
        "route_hint": route_hint });
    stamp(body, now, Some(&anchor))
}

fn episode_to_value(ep: SessionEpisode, now: DateTime<Utc>) -> Value {
    let created = ep.created_at.clone();
    let route_hint = RouteHint::new(ep.ward_id.clone(), RouteSourceKind::Episode)
        .with_memory_id(ep.id.clone())
        .with_session_id(Some(ep.session_id.clone()));
    let body = json!({
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
        "route_hint": route_hint_value(route_hint) });
    stamp(body, now, Some(&created))
}

/// GET /api/wards/:ward_id/content — aggregate all ward-scoped content.
pub async fn get_ward_content(
    State(state): State<AppState>,
    Path(ward_id): Path<String>,
) -> Result<Json<WardContentResponse>, HandlerError> {
    let memory_store = state
        .memory_store
        .as_ref()
        .ok_or_else(|| store_unavailable("memory"))?;
    let episode_store = state
        .episode_store
        .as_ref()
        .ok_or_else(|| store_unavailable("episode"))?;
    let wiki_store = state
        .wiki_store
        .as_ref()
        .ok_or_else(|| store_unavailable("wiki"))?;
    let procedure_store = state
        .procedure_store
        .as_ref()
        .ok_or_else(|| store_unavailable("procedure"))?;

    // Memory facts: trait surface lacks a `list_by_ward` projection so we
    // pull a generous slice and filter in-handler. The cap is high enough
    // that any single ward's row count comfortably fits.
    const FACT_AGG_LIMIT: usize = 5000;
    let facts: Vec<MemoryFact> = memory_store
        .list_memory_facts(None, None, None, FACT_AGG_LIMIT, 0)
        .await
        .map_err(|e| internal("list facts by ward", e))?
        .into_iter()
        .filter(|v| v.get("ward_id").and_then(|w| w.as_str()) == Some(ward_id.as_str()))
        .filter_map(|v| serde_json::from_value::<MemoryFact>(v).ok())
        .filter(|fact| !matches!(fact.category.as_str(), "ctx" | "instruction" | "correction"))
        .take(FACT_LIMIT)
        .collect();

    let episode_values: Vec<Value> = episode_store
        .list_by_ward(&ward_id, EPISODE_LIMIT)
        .await
        .map_err(|e| internal("list episodes by ward", e))?;

    let wiki_articles: Vec<WikiArticle> = wiki_store
        .list_articles(&ward_id)
        .await
        .map_err(|e| internal("list wiki by ward", e))?
        .into_iter()
        .filter_map(|v| serde_json::from_value::<WikiArticle>(v).ok())
        .collect();

    let procedures: Vec<Procedure> = procedure_store
        .list_by_ward(&ward_id, PROCEDURE_LIMIT)
        .await
        .map_err(|e| internal("list procedures by ward", e))?
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Procedure>(v).ok())
        .collect();

    // Cap wiki at WIKI_LIMIT (list_articles has no LIMIT clause).
    let wiki_articles: Vec<WikiArticle> = wiki_articles.into_iter().take(WIKI_LIMIT).collect();

    let summary = build_summary(&ward_id, &wiki_articles);

    let now = Utc::now();

    let counts = Counts {
        facts: facts.len(),
        wiki: wiki_articles.len(),
        procedures: procedures.len(),
        episodes: episode_values.len(),
    };

    let facts_json: Vec<Value> = facts.into_iter().map(|f| fact_to_value(f, now)).collect();
    let wiki_json: Vec<Value> = wiki_articles
        .into_iter()
        .map(|a| wiki_to_value(a, now))
        .collect();
    let procedures_json: Vec<Value> = procedures
        .into_iter()
        .map(|p| procedure_to_value(p, now))
        .collect();
    // Episode values come from the trait as JSON; deserialize each into
    // SessionEpisode for the response decorator, and skip rows that fail
    // to decode.
    let episodes_json: Vec<Value> = episode_values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<SessionEpisode>(v).ok())
        .map(|e| episode_to_value(e, now))
        .collect();

    Ok(Json(WardContentResponse {
        ward_id,
        summary,
        facts: facts_json,
        wiki: wiki_json,
        procedures: procedures_json,
        episodes: episodes_json,
        counts,
    }))
}

/// Response item for `GET /api/wards` — one ward entry.
#[derive(Debug, Serialize)]
pub struct WardListItem {
    pub id: String,
    pub count: usize,
}

/// GET /api/wards — list distinct wards with fact counts.
///
/// Streams up to `WARD_AGG_LIMIT` facts via the trait surface and
/// aggregates ward_id-distinct counts in the handler — the trait does
/// not expose a distinct-projection method yet.
pub async fn list_wards(
    State(state): State<AppState>,
) -> Result<Json<Vec<WardListItem>>, HandlerError> {
    let memory_store = state
        .memory_store
        .as_ref()
        .ok_or_else(|| store_unavailable("memory"))?;

    const WARD_AGG_LIMIT: usize = 5000;
    let rows = memory_store
        .list_memory_facts(None, None, None, WARD_AGG_LIMIT, 0)
        .await
        .map_err(|e| internal("list wards", e))?;

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in rows {
        if matches!(
            row.get("category").and_then(|v| v.as_str()),
            Some("ctx" | "instruction" | "correction")
        ) {
            continue;
        }
        let ward = row
            .get("ward_id")
            .and_then(|v| v.as_str())
            .unwrap_or("__global__");
        *counts.entry(ward.to_string()).or_insert(0) += 1;
    }

    let mut items: Vec<WardListItem> = counts
        .into_iter()
        .map(|(id, count)| WardListItem { id, count })
        .collect();
    items.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));

    Ok(Json(items))
}

#[cfg(test)]
mod helpers_tests {
    use super::*;

    fn now_anchor() -> DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-04-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn age_bucket_today_within_24h() {
        let now = now_anchor();
        assert_eq!(age_bucket(now, now - Duration::hours(1)), "today");
        assert_eq!(age_bucket(now, now - Duration::hours(23)), "today");
    }

    #[test]
    fn age_bucket_last_7_days_between_24h_and_week() {
        let now = now_anchor();
        assert_eq!(age_bucket(now, now - Duration::hours(25)), "last_7_days");
        assert_eq!(age_bucket(now, now - Duration::days(6)), "last_7_days");
    }

    #[test]
    fn age_bucket_historical_beyond_7_days() {
        let now = now_anchor();
        assert_eq!(age_bucket(now, now - Duration::days(7)), "historical");
        assert_eq!(age_bucket(now, now - Duration::days(30)), "historical");
    }

    #[test]
    fn parse_ts_handles_valid_rfc3339() {
        assert!(parse_ts("2026-04-01T00:00:00Z").is_some());
    }

    #[test]
    fn parse_ts_returns_none_for_garbage() {
        assert!(parse_ts("not-a-date").is_none());
        assert!(parse_ts("").is_none());
    }

    #[test]
    fn stamp_attaches_age_bucket_with_explicit_anchor() {
        let stamped = stamp(
            json!({"id": "x"}),
            now_anchor(),
            Some("2026-04-15T11:30:00Z"),
        );
        assert_eq!(stamped["age_bucket"], "today");
    }

    #[test]
    fn stamp_with_unparseable_anchor_falls_back_to_historical() {
        let stamped = stamp(json!({"id": "x"}), now_anchor(), Some("nonsense"));
        assert_eq!(stamped["age_bucket"], "historical");
    }

    #[test]
    fn stamp_with_no_anchor_falls_back_to_historical() {
        let stamped = stamp(json!({"id": "x"}), now_anchor(), None);
        assert_eq!(stamped["age_bucket"], "historical");
    }

    #[test]
    fn first_non_empty_line_returns_first_with_content() {
        assert_eq!(
            first_non_empty_line("\n\n  hello world\nignored second"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn first_non_empty_line_returns_none_for_blank_doc() {
        assert!(first_non_empty_line("").is_none());
        assert!(first_non_empty_line("\n\n   \n").is_none());
    }

    fn make_wiki(title: &str, content: &str) -> WikiArticle {
        WikiArticle {
            id: format!("art-{title}"),
            ward_id: "lab".into(),
            agent_id: "root".into(),
            title: title.into(),
            content: content.into(),
            tags: None,
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: "2026-04-01T00:00:00Z".into(),
            updated_at: "2026-04-02T12:30:00Z".into(),
        }
    }

    #[test]
    fn build_summary_uses_index_article_when_present() {
        let articles = vec![
            make_wiki("hello", "first body"),
            make_wiki("__index__", "Project lab description\nsecond line ignored"),
        ];
        let s = build_summary("lab", &articles);
        assert_eq!(s.title, "lab");
        assert_eq!(s.description.as_deref(), Some("Project lab description"));
        assert_eq!(s.updated_at.as_deref(), Some("2026-04-02T12:30:00Z"));
    }

    #[test]
    fn build_summary_falls_back_when_no_index() {
        let articles = vec![make_wiki("hello", "no index here")];
        let s = build_summary("lab", &articles);
        assert_eq!(s.title, "lab");
        assert!(s.description.is_none());
        assert!(s.updated_at.is_none());
    }

    #[test]
    fn store_unavailable_returns_503_with_what_label() {
        let (status, body) = store_unavailable("memory");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.0.error.contains("memory"));
        assert!(body.0.error.contains("store unavailable"));
    }

    #[test]
    fn fact_to_value_includes_age_bucket_from_updated_at() {
        let now = now_anchor();
        let fact = MemoryFact {
            id: "f1".into(),
            session_id: None,
            agent_id: "root".into(),
            scope: "agent".into(),
            category: "preference".into(),
            key: "tone".into(),
            content: "concise".into(),
            confidence: 0.9,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: "lab".into(),
            contradicted_by: None,
            created_at: "2026-04-15T11:00:00Z".into(),
            updated_at: "2026-04-15T11:00:00Z".into(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("convention".into()),
            source_episode_id: None,
            source_ref: None,
        };
        let v = fact_to_value(fact, now);
        assert_eq!(v["age_bucket"], "today");
        assert_eq!(v["agent_id"], "root");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "fact");
        assert_eq!(v["route_hint"]["memory_id"], "f1");
    }

    #[test]
    fn wiki_to_value_includes_age_bucket() {
        let v = wiki_to_value(make_wiki("title", "body"), now_anchor());
        assert_eq!(v["age_bucket"], "historical");
        assert_eq!(v["title"], "title");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "wiki_article");
    }

    #[test]
    fn procedure_to_value_uses_last_used_when_present() {
        let now = now_anchor();
        let proc = Procedure {
            id: "p1".into(),
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
            last_used: Some("2026-04-15T11:00:00Z".into()),
            embedding: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-04-01T00:00:00Z".into(),
        };
        let v = procedure_to_value(proc, now);
        assert_eq!(v["age_bucket"], "today");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "procedure");
        assert_eq!(v["route_hint"]["memory_id"], "p1");
    }

    #[test]
    fn procedure_to_value_falls_back_to_created_at_when_no_last_used() {
        let now = now_anchor();
        let proc = Procedure {
            id: "p1".into(),
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
            last_used: None,
            embedding: None,
            created_at: "2026-04-15T11:30:00Z".into(),
            updated_at: "2026-04-15T11:30:00Z".into(),
        };
        let v = procedure_to_value(proc, now);
        assert_eq!(v["age_bucket"], "today");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "procedure");
    }

    #[test]
    fn episode_to_value_uses_created_at_for_age() {
        let now = now_anchor();
        let ep = SessionEpisode {
            id: "e1".into(),
            session_id: "s1".into(),
            agent_id: "root".into(),
            ward_id: "lab".into(),
            task_summary: "fixed bug".into(),
            outcome: "success".into(),
            strategy_used: None,
            key_learnings: None,
            token_cost: None,
            embedding: None,
            created_at: "2026-04-10T12:00:00Z".into(),
        };
        let v = episode_to_value(ep, now);
        assert_eq!(v["age_bucket"], "last_7_days");
        assert_eq!(v["task_summary"], "fixed bug");
        assert_eq!(v["route_hint"]["ward_id"], "lab");
        assert_eq!(v["route_hint"]["source_kind"], "episode");
        assert_eq!(v["route_hint"]["session_id"], "s1");
    }

    #[test]
    fn internal_helper_emits_500_with_context() {
        let (status, body) = internal("list facts", "db locked");
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.0.error.contains("list facts"));
        assert!(body.0.error.contains("db locked"));
    }
}
