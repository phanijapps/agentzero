//! Tests for `POST /api/memory/search` unified hybrid search (Task 6 —
//! Memory Tab Command Deck).
//!
//! Seeds one item in each of the four content types (facts, wiki, procedures,
//! episodes) in a shared ward and asserts the unified handler fans out across
//! all four, returning per-type `hits` arrays and `latency_ms`.

mod common;

use common::{insert_episode, now_iso, setup, upsert_procedure, upsert_wiki_article};
use gateway::AppState;
use serde_json::{json, Value};
use zbot_stores_domain::{MemoryFact, Procedure, SessionEpisode, WikiArticle};

const TEST_WARD: &str = "maritime-vessel-tracking";

fn seed_all_four_types(state: &AppState) {
    let now = now_iso();

    let fact = MemoryFact {
        id: "fact-hormuz".to_string(),
        session_id: None,
        agent_id: "agent:root".to_string(),
        scope: "agent".to_string(),
        category: "pattern".to_string(),
        key: "mar.hormuz".to_string(),
        content: "Strait of Hormuz transit".to_string(),
        confidence: 0.9,
        mention_count: 1,
        source_summary: None,
        embedding: None,
        ward_id: TEST_WARD.to_string(),
        contradicted_by: None,
        created_at: now.clone(),
        updated_at: now.clone(),
        expires_at: None,
        valid_from: None,
        valid_until: None,
        superseded_by: None,
        pinned: false,
        epistemic_class: Some("current".to_string()),
        source_episode_id: None,
        source_ref: None,
        last_accessed: None,
    };
    futures::executor::block_on(
        state
            .memory_store
            .as_ref()
            .expect("memory_store")
            .upsert_typed_fact(fact.clone(), None),
    )
    .expect("upsert fact");

    let article = WikiArticle {
        id: "wiki-hormuz".to_string(),
        ward_id: TEST_WARD.to_string(),
        agent_id: "agent:root".to_string(),
        title: "Hormuz".to_string(),
        content: "Narrow strait between Oman and Iran.".to_string(),
        tags: None,
        source_fact_ids: None,
        embedding: None,
        version: 1,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_wiki_article(state, &article);

    let proc = Procedure {
        id: "proc-hormuz".to_string(),
        agent_id: "agent:root".to_string(),
        ward_id: Some(TEST_WARD.to_string()),
        name: "track-hormuz".to_string(),
        description: "Track vessels in Hormuz".to_string(),
        trigger_pattern: None,
        steps: "[]".to_string(),
        parameters: None,
        success_count: 0,
        failure_count: 0,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_procedure(state, &proc);

    let ep = SessionEpisode {
        id: "ep-hormuz".to_string(),
        session_id: "sess-h".to_string(),
        agent_id: "agent:root".to_string(),
        ward_id: TEST_WARD.to_string(),
        task_summary: "Monitored Hormuz traffic".to_string(),
        outcome: "success".to_string(),
        strategy_used: None,
        key_learnings: None,
        token_cost: None,
        embedding: None,
        created_at: now.clone(),
    };
    insert_episode(state, &ep);
}

fn assert_block(body: &Value, key: &str) {
    let block = &body[key];
    assert!(block.is_object(), "{key} should be an object, got: {block}");
    assert!(
        block["hits"].is_array(),
        "{key}.hits should be array, got: {block}"
    );
    assert!(
        block["latency_ms"].is_number(),
        "{key}.latency_ms should be number, got: {block}"
    );
}

#[tokio::test]
async fn searches_all_four_types_in_parallel() {
    let (server, _dir, state) = setup();
    seed_all_four_types(&state);

    let response = server
        .post("/api/memory/search")
        .json(&json!({
            "query": "hormuz",
            "mode": "hybrid",
            "types": ["facts", "wiki", "procedures", "episodes"],
            "ward_ids": [TEST_WARD],
            "limit": 10
        }))
        .await;

    response.assert_status_ok();
    let body: Value = response.json();

    assert_block(&body, "facts");
    assert_block(&body, "wiki");
    assert_block(&body, "procedures");
    assert_block(&body, "episodes");
}

#[tokio::test]
async fn mode_fts_skips_procedures_and_returns_empty() {
    let (server, _dir, state) = setup();
    seed_all_four_types(&state);

    let response = server
        .post("/api/memory/search")
        .json(&json!({
            "query": "hormuz",
            "mode": "fts",
            "types": ["facts", "wiki", "procedures", "episodes"],
            "ward_ids": [TEST_WARD],
            "limit": 10
        }))
        .await;

    response.assert_status_ok();
    let body: Value = response.json();

    assert_block(&body, "facts");
    assert_block(&body, "wiki");
    assert_block(&body, "procedures");
    assert_block(&body, "episodes");

    let procs = body["procedures"]["hits"].as_array().expect("procs arr");
    assert!(
        procs.is_empty(),
        "procedures must be empty in fts mode (no FTS index), got: {procs:?}"
    );

    // Facts and wiki should find the seeded "hormuz" content via FTS.
    let facts = body["facts"]["hits"].as_array().expect("facts arr");
    assert!(!facts.is_empty(), "facts should have hits via FTS");
    let wiki = body["wiki"]["hits"].as_array().expect("wiki arr");
    assert!(!wiki.is_empty(), "wiki should have hits via FTS");
}

#[tokio::test]
async fn facts_lane_filters_internal_reserved_categories() {
    let (server, _dir, state) = setup();
    let now = now_iso();

    for category in ["ctx", "instruction", "correction"] {
        let fact = MemoryFact {
            id: format!("fact-internal-{category}"),
            session_id: Some("sess-internal".to_string()),
            agent_id: "agent:root".to_string(),
            scope: "session".to_string(),
            category: category.to_string(),
            key: format!("{category}.private"),
            content: "reserved-private-sentinel must not be public".to_string(),
            confidence: 1.0,
            mention_count: 1,
            source_summary: None,
            embedding: None,
            ward_id: TEST_WARD.to_string(),
            contradicted_by: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: true,
            epistemic_class: Some("current".to_string()),
            source_episode_id: None,
            source_ref: None,
            last_accessed: None,
        };
        futures::executor::block_on(
            state
                .memory_store
                .as_ref()
                .expect("memory_store")
                .upsert_typed_fact(fact.clone(), None),
        )
        .expect("upsert internal fact");
    }

    let response = server
        .post("/api/memory/search")
        .json(&json!({
            "query": "reserved-private-sentinel",
            "mode": "fts",
            "types": ["facts"],
            "ward_ids": [TEST_WARD],
            "limit": 10
        }))
        .await;

    response.assert_status_ok();
    let body: Value = response.json();
    let facts = body["facts"]["hits"].as_array().expect("facts hits");
    assert!(
        facts.is_empty(),
        "reserved internal facts must not appear in unified public search: {facts:?}"
    );
}

#[tokio::test]
async fn invalid_mode_returns_bad_request() {
    let (server, _dir, state) = setup();
    seed_all_four_types(&state);

    let response = server
        .post("/api/memory/search")
        .json(&json!({
            "query": "hormuz",
            "mode": "surprise",
            "types": ["wiki", "episodes"],
            "ward_ids": [TEST_WARD],
            "limit": 10
        }))
        .await;

    response.assert_status_bad_request();
    let body: Value = response.json();
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|message| message.contains("unsupported memory search mode")),
        "unexpected error body: {body}"
    );
}
