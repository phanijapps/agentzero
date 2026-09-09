use chrono::{TimeZone, Utc};
use serde_json::json;
use zbot_engram_adapter::{AdapterConfig, EngramMemoryFactStore};
use zbot_stores_traits::{
    EmbeddingQueryIdentity, MemoryFactStore, SkillIndexRow, StrategyFactInsert,
};

fn engram_config(root: &tempfile::TempDir) -> AdapterConfig {
    AdapterConfig::engram_for_data_root(root.path(), "engram")
}

fn embedding_identity(dimensions: u32) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: "fastembed".to_string(),
        model: "BAAI/bge-small-en-v1.5".to_string(),
        dimensions,
        prompt_profile: "query".to_string(),
        normalization: None,
    }
}

#[tokio::test]
async fn ctx_facts_round_trip_and_upsert_by_key() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .save_ctx_fact(
            "sess-1",
            "ward-a",
            "ctx.sess-1.intent",
            "first intent",
            "root",
            true,
        )
        .await
        .expect("save ctx");
    store
        .save_ctx_fact(
            "sess-1",
            "ward-a",
            "ctx.sess-1.intent",
            "second intent",
            "subagent:exec-1",
            false,
        )
        .await
        .expect("upsert ctx");

    let value = store
        .get_ctx_fact("ward-a", "ctx.sess-1.intent")
        .await
        .expect("get ctx")
        .expect("ctx value");

    assert_eq!(value["found"], true);
    assert_eq!(value["key"], "ctx.sess-1.intent");
    assert_eq!(value["content"], "second intent");
    assert_eq!(value["owner"], "subagent:exec-1");
    assert_eq!(value["session_id"], "sess-1");
    assert_eq!(value["pinned"], false);
    assert!(store
        .get_ctx_fact("ward-a", "ctx.sess-1.missing")
        .await
        .expect("missing ctx")
        .is_none());
}

#[tokio::test]
async fn primitives_list_as_json_and_typed_facts() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .upsert_primitive(
            "ward-a",
            "primitive.src.main.build_agent",
            "fn build_agent(config: Config) -> Agent",
            "Constructs an agent.",
        )
        .await
        .expect("upsert primitive");

    let value = store
        .list_primitives("ward-a")
        .await
        .expect("list primitives");
    assert_eq!(
        value,
        json!({
            "primitives": [{
                "key": "primitive.src.main.build_agent",
                "signature": "fn build_agent(config: Config) -> Agent",
                "summary": "Constructs an agent."
            }]
        })
    );

    let facts = store
        .list_primitives_for_ward("ward-a")
        .await
        .expect("typed primitives");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].category, "primitive");
    assert_eq!(facts[0].scope, "global");
    assert_eq!(facts[0].agent_id, "__ward__");
}

#[tokio::test]
async fn recent_state_handoffs_are_session_scoped_and_newest_first() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .save_ctx_fact(
            "sess-1",
            "ward-a",
            "ctx.sess-1.state.exec-1",
            "old handoff",
            "subagent:exec-1",
            true,
        )
        .await
        .expect("old handoff");
    store
        .save_ctx_fact(
            "sess-1",
            "ward-a",
            "ctx.sess-1.state.exec-2",
            "new handoff",
            "subagent:exec-2",
            true,
        )
        .await
        .expect("new handoff");
    store
        .save_ctx_fact(
            "sess-2",
            "ward-a",
            "ctx.sess-2.state.exec-1",
            "other session",
            "subagent:exec-1",
            true,
        )
        .await
        .expect("other handoff");

    let rows = store
        .list_recent_state_handoffs("sess-1", 1)
        .await
        .expect("handoffs");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "ctx.sess-1.state.exec-2");
    assert_eq!(rows[0].content, "new handoff");
}

#[tokio::test]
async fn skill_index_rows_persist_and_delete() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .upsert_skill_index(SkillIndexRow {
            name: "skill-a".to_string(),
            source_root: "vault".to_string(),
            file_path: "/tmp/skill-a/SKILL.md".to_string(),
            mtime_unix: 10,
            size_bytes: 20,
            last_indexed_unix: 30,
            format_version: 2,
        })
        .await
        .expect("upsert skill");

    let rows = store.list_skill_index().await.expect("list skill");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "skill-a");
    assert_eq!(rows[0].format_version, 2);

    assert!(store
        .delete_skill_index("skill-a")
        .await
        .expect("delete skill"));
    assert!(store
        .list_skill_index()
        .await
        .expect("list after delete")
        .is_empty());
}

#[tokio::test]
async fn embedding_cache_and_delete_by_key_use_sidecar_tables() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramMemoryFactStore::open(engram_config(&root)).expect("store");

    store
        .cache_embedding("sha256:abc", "model-a", &[0.1, 0.2, 0.3])
        .await
        .expect("cache embedding");
    assert_eq!(
        store
            .get_cached_embedding("sha256:abc", "model-a")
            .await
            .expect("cached embedding"),
        Some(vec![0.1, 0.2, 0.3])
    );

    store
        .save_fact(
            "agent-a",
            "skill",
            "skill.removed",
            "Removed skill content",
            0.7,
            None,
            None,
        )
        .await
        .expect("save skill fact");
    assert_eq!(
        store
            .delete_facts_by_key("skill", "skill.removed")
            .await
            .expect("delete by key"),
        1
    );
    assert_eq!(store.count_all_facts(None).await.expect("count"), 0);
}

#[tokio::test]
async fn strategy_fact_synthesis_uses_memory_sidecar() {
    let root = tempfile::tempdir().expect("root");
    let mut config = engram_config(&root);
    config.embedding_provider.dimensions = 3;
    let store = EngramMemoryFactStore::open(config).expect("store");

    let id = store
        .insert_strategy_fact(StrategyFactInsert {
            agent_id: "agent-a".to_string(),
            key: "strategy.use_specs".to_string(),
            content: "Write the contract before changing code.".to_string(),
            confidence: 0.91,
            source_summary: Some("synthesis".to_string()),
            embedding: Some(vec![1.0, 0.0, 0.0]),
            source_episode_id: Some("episode-a".to_string()),
        })
        .await
        .expect("insert strategy");

    assert!(store
        .find_strategy_fact_by_similarity("agent-a", &[1.0, 0.0, 0.0], 0.95, 10)
        .await
        .expect("find strategy")
        .is_none());

    let identity = embedding_identity(3);
    let found = store
        .find_strategy_fact_by_similarity_with_identity(
            "agent-a",
            &[1.0, 0.0, 0.0],
            Some(&identity),
            0.95,
            10,
        )
        .await
        .expect("find strategy")
        .expect("strategy match");
    assert_eq!(found.fact_id, id);
    assert_eq!(found.source_episode_id.as_deref(), Some("episode-a"));

    let mut mismatched = identity.clone();
    mismatched.model = "other/bge-small-en-v1.5".to_string();
    assert!(store
        .find_strategy_fact_by_similarity_with_identity(
            "agent-a",
            &[1.0, 0.0, 0.0],
            Some(&mismatched),
            0.95,
            10,
        )
        .await
        .expect("mismatched identity")
        .is_none());

    store
        .bump_strategy_fact_episodes(&id, "episode-a,episode-b", "2026-07-06T12:00:00Z")
        .await
        .expect("bump strategy");
    let fact = store
        .get_memory_fact_by_id(&id)
        .await
        .expect("get strategy")
        .expect("strategy fact");
    assert_eq!(fact["mention_count"], 2);
    assert_eq!(fact["source_episode_id"], "episode-a,episode-b");
    assert_eq!(fact["updated_at"], "2026-07-06T12:00:00Z");

    let mut contradicted = serde_json::from_value::<zbot_stores_traits::MemoryFact>(fact)
        .expect("strategy fact shape");
    contradicted.contradicted_by = Some("fact-other".to_string());
    contradicted.updated_at = "2026-07-07T00:00:00Z".to_string();
    store
        .upsert_typed_fact(contradicted, None)
        .await
        .expect("upsert contradicted");

    let episodes = store
        .list_contradicted_fact_episode_ids(
            "agent-a",
            Utc.with_ymd_and_hms(2026, 7, 6, 23, 0, 0).unwrap(),
        )
        .await
        .expect("contradicted episodes");
    assert_eq!(episodes, vec!["episode-a,episode-b".to_string()]);
}
