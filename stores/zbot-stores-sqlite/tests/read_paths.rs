use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use zbot_stores::types::Direction;
use zbot_stores::KnowledgeGraphStore;

mod fixtures;

#[tokio::test]
async fn neighbors_returns_outgoing_edge() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    let a = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Dan".to_string(),
    );
    let b = Entity::new(
        "test-agent".to_string(),
        EntityType::Project,
        "Halite".to_string(),
    );
    let a_id = store.upsert_entity("test-agent", a.clone()).await.unwrap();
    let b_id = store.upsert_entity("test-agent", b.clone()).await.unwrap();

    let rel = Relationship::new(
        "test-agent".to_string(),
        a.id.clone(),
        b.id.clone(),
        RelationshipType::Created,
    );
    store.upsert_relationship("test-agent", rel).await.unwrap();

    let neighbors = store
        .get_neighbors(&a_id, Direction::Outgoing, 10)
        .await
        .expect("neighbors");
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].entity_id, b_id);
}

#[tokio::test]
async fn search_by_name_finds_entity() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    let e = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Erika Mustermann".to_string(),
    );
    store.upsert_entity("test-agent", e).await.unwrap();

    let hits = store
        .search_entities_by_name("test-agent", "Erika", 10)
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn traverse_returns_multi_hop_path() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    // Build a 3-entity chain: A → B → C
    let a = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Alice".to_string(),
    );
    let b = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Bob".to_string(),
    );
    let c = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Carol".to_string(),
    );
    let a_id = store.upsert_entity("test-agent", a.clone()).await.unwrap();
    let b_id = store.upsert_entity("test-agent", b.clone()).await.unwrap();
    let c_id = store.upsert_entity("test-agent", c.clone()).await.unwrap();

    let rel_ab = Relationship::new(
        "test-agent".to_string(),
        a.id.clone(),
        b.id.clone(),
        RelationshipType::WorksFor,
    );
    let rel_bc = Relationship::new(
        "test-agent".to_string(),
        b.id.clone(),
        c.id.clone(),
        RelationshipType::WorksFor,
    );
    store
        .upsert_relationship("test-agent", rel_ab)
        .await
        .unwrap();
    store
        .upsert_relationship("test-agent", rel_bc)
        .await
        .unwrap();

    // Traverse from A with max_hops=2 — should find B (1 hop) and C (2 hops).
    let hits = store.traverse(&a_id, 2, 10).await.expect("traverse");

    assert!(!hits.is_empty(), "traverse should return at least one hit");
    let ids: std::collections::HashSet<_> = hits.iter().map(|h| h.entity_id.clone()).collect();
    assert!(ids.contains(&b_id), "should reach B in 1 hop");
    assert!(ids.contains(&c_id), "should reach C in 2 hops");
}
