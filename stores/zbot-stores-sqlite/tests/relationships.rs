use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use zbot_stores::KnowledgeGraphStore;

mod fixtures;

#[tokio::test]
async fn relationship_upsert_and_delete() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    let a = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Alice".to_string(),
    );
    let b = Entity::new(
        "test-agent".to_string(),
        EntityType::Organization,
        "ACME".to_string(),
    );
    let a_id = store
        .upsert_entity("test-agent", a.clone())
        .await
        .expect("a");
    let b_id = store
        .upsert_entity("test-agent", b.clone())
        .await
        .expect("b");

    let rel = Relationship::new(
        "test-agent".to_string(),
        a_id.0.clone(),
        b_id.0.clone(),
        RelationshipType::WorksFor,
    );
    let rel_id = store
        .upsert_relationship("test-agent", rel)
        .await
        .expect("upsert rel");

    store
        .delete_relationship(&rel_id)
        .await
        .expect("delete rel");
}
