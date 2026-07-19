use knowledge_graph::types::{Entity, EntityType};
use zbot_stores::types::ResolveOutcome;
use zbot_stores::KnowledgeGraphStore;

mod fixtures;

#[tokio::test]
async fn alias_round_trip_then_exact_resolve() {
    let (_tmp, store) = fixtures::sqlite_store().await;

    let e = Entity::new(
        "test-agent".to_string(),
        EntityType::Person,
        "Augusta Ada King".to_string(),
    );
    let id = store.upsert_entity("test-agent", e).await.expect("upsert");

    // Add an alias for the entity
    store
        .add_alias(&id, "Ada Lovelace")
        .await
        .expect("add_alias");

    // Exact-name resolve should now match via alias
    let outcome = store
        .resolve_entity("test-agent", &EntityType::Person, "Ada Lovelace", None)
        .await
        .expect("resolve");
    match outcome {
        ResolveOutcome::Match(matched) => assert_eq!(matched, id),
        ResolveOutcome::NoMatch => panic!("expected alias match"),
    }
}

#[tokio::test]
async fn resolve_unknown_entity_returns_no_match() {
    let (_tmp, store) = fixtures::sqlite_store().await;
    let outcome = store
        .resolve_entity("test-agent", &EntityType::Person, "Nobody Special", None)
        .await
        .expect("resolve");
    assert!(matches!(outcome, ResolveOutcome::NoMatch));
}
