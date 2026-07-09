use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use zbot_stores::extracted::ExtractedKnowledge;
use zbot_stores::KnowledgeGraphStore;

mod fixtures;

#[tokio::test]
async fn store_knowledge_atomic() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    let a = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Carol".to_string(),
    );
    let b = Entity::new(
        "test-agent".to_string(),
        EntityType::Organization,
        "Initech".to_string(),
    );
    let rel = Relationship::new(
        "test-agent".to_string(),
        a.id.clone(),
        b.id.clone(),
        RelationshipType::WorksFor,
    );

    let knowledge = ExtractedKnowledge {
        entities: vec![a, b],
        relationships: vec![rel],
    };

    let outcome = store
        .store_knowledge("test-agent", knowledge)
        .await
        .expect("store");

    assert_eq!(outcome.entities_inserted + outcome.entities_merged, 2);
    assert_eq!(outcome.relationships_inserted, 1);
}
