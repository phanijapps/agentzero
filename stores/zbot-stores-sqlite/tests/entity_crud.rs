use knowledge_graph::types::{Entity, EntityType};
use zbot_stores::KnowledgeGraphStore;

mod fixtures;

#[tokio::test]
async fn entity_round_trip() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    let e = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Ada Lovelace".to_string(),
    );
    let original_id = e.id.clone();

    // upsert_entity
    let id = store.upsert_entity("test-agent", e).await.expect("upsert");
    assert_eq!(id.as_ref(), original_id);

    // get_entity
    let fetched = store.get_entity(&id).await.expect("get");
    assert!(fetched.is_some(), "entity should exist after upsert");
    assert_eq!(fetched.unwrap().name, "Ada Lovelace");

    // bump_entity_mention
    store.bump_entity_mention(&id).await.expect("bump");
    let after_bump = store.get_entity(&id).await.expect("get").unwrap();
    assert!(
        after_bump.mention_count >= 2,
        "mention_count should increment"
    );

    // delete_entity
    store.delete_entity(&id).await.expect("delete");
    let after_delete = store.get_entity(&id).await.expect("get");
    assert!(after_delete.is_none(), "entity should be gone after delete");
}
