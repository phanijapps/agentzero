use knowledge_graph::types::{Entity, EntityType};
use zbot_stores::KnowledgeGraphStore;

mod fixtures;

#[tokio::test]
async fn stats_reflects_inserts() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    let before = store.stats().await.expect("stats");
    assert_eq!(before.entity_count, 0);

    let e = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Faye".to_string(),
    );
    store.upsert_entity("test-agent", e).await.unwrap();

    let after = store.stats().await.expect("stats");
    assert_eq!(after.entity_count, 1);
}
