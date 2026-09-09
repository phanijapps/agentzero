//! Cross-impl conformance scenarios for `zbot-stores` traits.
//!
//! Each function takes a generic `&S` where `S: KnowledgeGraphStore` (or
//! `MemoryFactStore`, etc.) and runs an end-to-end behavioural check.
//! Impl crates call these from their integration tests; behavioural drift
//! between impls produces failing assertions.

pub mod parity;

use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
use zbot_stores::extracted::ExtractedKnowledge;
use zbot_stores::types::{Direction, EntityId, ResolveOutcome};
use zbot_stores::KnowledgeGraphStore;
use zbot_stores_domain::MemoryFact;

// =============================================================================
// Entity CRUD
// =============================================================================

pub async fn entity_round_trip<S: KnowledgeGraphStore>(store: &S) {
    let e = Entity::new(
        "conformance-agent".to_string(),
        EntityType::Person,
        "Conformance Subject".to_string(),
    );
    let original_id = e.id.clone();

    let id = store
        .upsert_entity("conformance-agent", e)
        .await
        .expect("upsert");
    assert_eq!(id.as_ref(), original_id);

    let fetched = store.get_entity(&id).await.expect("get");
    assert!(fetched.is_some(), "entity should exist after upsert");
    assert_eq!(fetched.unwrap().name, "Conformance Subject");

    store.delete_entity(&id).await.expect("delete");
    let after_delete = store.get_entity(&id).await.expect("get");
    assert!(after_delete.is_none(), "entity should be gone after delete");
}

pub async fn upsert_increments_mention_count<S: KnowledgeGraphStore>(store: &S) {
    let e = Entity::new("conf".into(), EntityType::Person, "Subject".into());
    let id = store.upsert_entity("conf", e.clone()).await.unwrap();
    store.upsert_entity("conf", e.clone()).await.unwrap();
    store.upsert_entity("conf", e).await.unwrap();
    let fetched = store.get_entity(&id).await.unwrap().expect("entity");
    assert!(
        fetched.mention_count >= 2,
        "expected mention_count to grow on repeated upsert, got {}",
        fetched.mention_count
    );
}

pub async fn bump_mention_increases_count<S: KnowledgeGraphStore>(store: &S) {
    let e = Entity::new("conf".into(), EntityType::Concept, "Bumpy".into());
    let id = store.upsert_entity("conf", e).await.unwrap();
    let before = store.get_entity(&id).await.unwrap().unwrap().mention_count;
    store.bump_entity_mention(&id).await.unwrap();
    let after = store.get_entity(&id).await.unwrap().unwrap().mention_count;
    assert!(after > before, "bump should increment");
}

// =============================================================================
// Alias / resolve
// =============================================================================

pub async fn resolve_exact_match<S: KnowledgeGraphStore>(store: &S) {
    let e = Entity::new("conf".into(), EntityType::Person, "Carol".into());
    let id = store.upsert_entity("conf", e).await.unwrap();
    let outcome = store
        .resolve_entity("conf", &EntityType::Person, "Carol", None)
        .await
        .unwrap();
    match outcome {
        ResolveOutcome::Match(found) => assert_eq!(found.as_ref(), id.as_ref()),
        ResolveOutcome::NoMatch => panic!("should match existing"),
    }
}

pub async fn resolve_via_alias<S: KnowledgeGraphStore>(store: &S) {
    let e = Entity::new("conf".into(), EntityType::Person, "Carol".into());
    let id = store.upsert_entity("conf", e).await.unwrap();
    store.add_alias(&id, "Carolyn").await.unwrap();
    let outcome = store
        .resolve_entity("conf", &EntityType::Person, "Carolyn", None)
        .await
        .unwrap();
    match outcome {
        ResolveOutcome::Match(found) => assert_eq!(found.as_ref(), id.as_ref()),
        ResolveOutcome::NoMatch => panic!("alias should resolve"),
    }
}

pub async fn resolve_no_match<S: KnowledgeGraphStore>(store: &S) {
    let outcome = store
        .resolve_entity("conf", &EntityType::Person, "DoesNotExist", None)
        .await
        .unwrap();
    assert!(matches!(outcome, ResolveOutcome::NoMatch));
}

// =============================================================================
// Relationships + bulk ingest
// =============================================================================

async fn alice_and_bob<S: KnowledgeGraphStore>(store: &S) -> (EntityId, EntityId) {
    let alice = Entity::new("conf".into(), EntityType::Person, "Alice".into());
    let bob = Entity::new("conf".into(), EntityType::Person, "Bob".into());
    let alice_id = store.upsert_entity("conf", alice).await.unwrap();
    let bob_id = store.upsert_entity("conf", bob).await.unwrap();
    (alice_id, bob_id)
}

pub async fn relationship_round_trip<S: KnowledgeGraphStore>(store: &S) {
    let (alice, bob) = alice_and_bob(store).await;
    let rel = Relationship::new(
        "conf".into(),
        alice.0.clone(),
        bob.0.clone(),
        RelationshipType::WorksFor,
    );
    let rid = store.upsert_relationship("conf", rel).await.unwrap();
    store.delete_relationship(&rid).await.unwrap();
}

pub async fn store_knowledge_writes_both<S: KnowledgeGraphStore>(store: &S) {
    let alice = Entity::new("conf".into(), EntityType::Person, "Alice".into());
    let bob = Entity::new("conf".into(), EntityType::Person, "Bob".into());
    let rel = Relationship::new(
        "conf".into(),
        alice.id.clone(),
        bob.id.clone(),
        RelationshipType::WorksFor,
    );
    let knowledge = ExtractedKnowledge {
        entities: vec![alice, bob],
        relationships: vec![rel],
    };
    store.store_knowledge("conf", knowledge).await.unwrap();
    let n_entities = store.count_all_entities().await.unwrap();
    let n_rels = store.count_all_relationships().await.unwrap();
    assert!(
        n_entities >= 2,
        "expected at least 2 entities, got {n_entities}"
    );
    assert!(
        n_rels >= 1,
        "expected at least 1 relationship, got {n_rels}"
    );
}

// =============================================================================
// Traversal
// =============================================================================

pub async fn neighbors_outgoing<S: KnowledgeGraphStore>(store: &S) {
    let (alice, bob) = alice_and_bob(store).await;
    let rel = Relationship::new(
        "conf".into(),
        alice.0.clone(),
        bob.0.clone(),
        RelationshipType::RelatedTo,
    );
    store.upsert_relationship("conf", rel).await.unwrap();

    let neighbors = store
        .get_neighbors(&alice, Direction::Outgoing, 10)
        .await
        .unwrap();
    assert!(
        neighbors
            .iter()
            .any(|n| n.entity_id.as_ref() == bob.as_ref()),
        "outgoing should include Bob"
    );
}

pub async fn neighbors_incoming<S: KnowledgeGraphStore>(store: &S) {
    let (alice, bob) = alice_and_bob(store).await;
    let rel = Relationship::new(
        "conf".into(),
        alice.0.clone(),
        bob.0.clone(),
        RelationshipType::RelatedTo,
    );
    store.upsert_relationship("conf", rel).await.unwrap();

    let neighbors = store
        .get_neighbors(&bob, Direction::Incoming, 10)
        .await
        .unwrap();
    assert!(
        neighbors
            .iter()
            .any(|n| n.entity_id.as_ref() == alice.as_ref()),
        "incoming to Bob should include Alice"
    );
}

pub async fn traverse_respects_max_hops<S: KnowledgeGraphStore>(store: &S) {
    let a = store
        .upsert_entity(
            "conf",
            Entity::new("conf".into(), EntityType::Concept, "A".into()),
        )
        .await
        .unwrap();
    let b = store
        .upsert_entity(
            "conf",
            Entity::new("conf".into(), EntityType::Concept, "B".into()),
        )
        .await
        .unwrap();
    let c = store
        .upsert_entity(
            "conf",
            Entity::new("conf".into(), EntityType::Concept, "C".into()),
        )
        .await
        .unwrap();
    store
        .upsert_relationship(
            "conf",
            Relationship::new(
                "conf".into(),
                a.0.clone(),
                b.0.clone(),
                RelationshipType::RelatedTo,
            ),
        )
        .await
        .unwrap();
    store
        .upsert_relationship(
            "conf",
            Relationship::new(
                "conf".into(),
                b.0.clone(),
                c.0.clone(),
                RelationshipType::RelatedTo,
            ),
        )
        .await
        .unwrap();

    let hits_1 = store.traverse(&a, 1, 100).await.unwrap();
    let hits_2 = store.traverse(&a, 2, 100).await.unwrap();
    assert!(
        hits_2.len() >= hits_1.len(),
        "deeper traversal should reach >= entities"
    );
}

// =============================================================================
// Search / FTS / KNN
// =============================================================================

pub async fn fts_finds_match<S: KnowledgeGraphStore>(store: &S) {
    store
        .upsert_entity(
            "conf",
            Entity::new("conf".into(), EntityType::Person, "Alice Walker".into()),
        )
        .await
        .unwrap();
    store
        .upsert_entity(
            "conf",
            Entity::new("conf".into(), EntityType::Person, "Bob Smith".into()),
        )
        .await
        .unwrap();
    let hits = store
        .search_entities_by_name("conf", "alice", 10)
        .await
        .unwrap();
    assert!(
        hits.iter().any(|e| e.name.contains("Alice")),
        "FTS should find Alice"
    );
}

// =============================================================================
// Reindex idempotency
// =============================================================================

pub async fn reindex_idempotent_when_dim_matches<S: KnowledgeGraphStore>(store: &S) {
    // First call establishes dim 1024; second with same dim should be a no-op.
    let _ = store.reindex_embeddings(1024).await.unwrap();
    let report = store.reindex_embeddings(1024).await.unwrap();
    assert!(
        report.tables_rebuilt.is_empty(),
        "matching dim should be no-op, got {:?}",
        report.tables_rebuilt
    );
}

// =============================================================================
// Stats / health
// =============================================================================

pub async fn stats_reflects_writes<S: KnowledgeGraphStore>(store: &S) {
    let before = store.count_all_entities().await.unwrap();
    store
        .upsert_entity(
            "conf",
            Entity::new("conf".into(), EntityType::Concept, "StatProbe".into()),
        )
        .await
        .unwrap();
    let after = store.count_all_entities().await.unwrap();
    assert_eq!(after, before + 1, "count should grow by 1 after one upsert");
}

pub async fn graph_stats_per_agent<S: KnowledgeGraphStore>(store: &S) {
    store
        .upsert_entity(
            "agent-x",
            Entity::new("agent-x".into(), EntityType::Concept, "X1".into()),
        )
        .await
        .unwrap();
    store
        .upsert_entity(
            "agent-y",
            Entity::new("agent-y".into(), EntityType::Concept, "Y1".into()),
        )
        .await
        .unwrap();
    let s_x = store.graph_stats("agent-x").await.unwrap();
    let s_y = store.graph_stats("agent-y").await.unwrap();
    assert!(s_x.entity_count >= 1);
    assert!(s_y.entity_count >= 1);
}

// =============================================================================
// Archival
// =============================================================================

pub async fn mark_archival_sets_class<S: KnowledgeGraphStore>(store: &S) {
    let e = Entity::new("conf".into(), EntityType::Concept, "Archivee".into());
    let id = store.upsert_entity("conf", e).await.unwrap();
    store
        .mark_entity_archival(&id, "conformance-test")
        .await
        .unwrap();
    // The contract is that mark_entity_archival succeeds; entity may still
    // exist but with epistemic_class='archival'. Backend-specific.
}

// =============================================================================
// Cross-agent isolation
// =============================================================================

pub async fn list_entities_respects_agent<S: KnowledgeGraphStore>(store: &S) {
    store
        .upsert_entity(
            "agent-iso-a",
            Entity::new("agent-iso-a".into(), EntityType::Concept, "OnlyA".into()),
        )
        .await
        .unwrap();
    store
        .upsert_entity(
            "agent-iso-b",
            Entity::new("agent-iso-b".into(), EntityType::Concept, "OnlyB".into()),
        )
        .await
        .unwrap();
    store
        .upsert_entity(
            "__global__",
            Entity::new(
                "__global__".into(),
                EntityType::Concept,
                "SharedAcrossAgents".into(),
            ),
        )
        .await
        .unwrap();

    let a_list = store
        .list_entities("agent-iso-a", None, 100, 0)
        .await
        .unwrap();
    assert!(
        a_list.iter().any(|e| e.name == "OnlyA"),
        "list should retain the requested agent's entity"
    );
    assert!(
        a_list.iter().any(|e| e.name == "SharedAcrossAgents"),
        "list should retain explicitly global entities"
    );
    assert!(
        a_list
            .iter()
            .all(|e| e.agent_id == "agent-iso-a" || e.agent_id == "__global__"),
        "list should include only the requested agent and explicitly global entities"
    );
}

// =============================================================================
// Memory store conformance
// =============================================================================

use zbot_stores_traits::MemoryFactStore;

pub async fn memory_save_and_count<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact("conf", "preference", "k1", "loves coffee", 0.9, None, None)
        .await
        .unwrap();
    let n = store.count_all_facts(Some("conf")).await.unwrap();
    assert!(n >= 1, "saved fact should be counted, got {n}");
}

pub async fn memory_recall_finds_match<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "conf",
            "preference",
            "k1",
            "Bob really likes espresso",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let result = store.recall_facts("conf", "espresso", 10).await.unwrap();
    let arr = result.as_array().expect("array");
    assert!(!arr.is_empty(), "recall should find match");
}

pub async fn memory_recall_respects_agent_isolation<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "agent-mem-a",
            "preference",
            "k1",
            "agent A note",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let _ = store
        .save_fact(
            "agent-mem-b",
            "preference",
            "k1",
            "agent B note",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let result = store.recall_facts("agent-mem-a", "note", 10).await.unwrap();
    let arr = result.as_array().expect("array");
    for item in arr {
        assert_eq!(
            item.get("agent_id").and_then(|v| v.as_str()),
            Some("agent-mem-a")
        );
    }
}

pub async fn memory_list_facts_filters_and_paginates<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "conf-a",
            "preference",
            "k1",
            "Alice likes coffee",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let _ = store
        .save_fact("conf-a", "skill", "k2", "Alice knows Rust", 0.8, None, None)
        .await
        .unwrap();
    let _ = store
        .save_fact(
            "conf-b",
            "preference",
            "k3",
            "Bob likes tea",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();

    let a_facts = store
        .list_memory_facts(Some("conf-a"), None, None, 100, 0)
        .await
        .unwrap();
    assert!(
        a_facts.len() >= 2,
        "agent filter should return >= 2 facts, got {}",
        a_facts.len()
    );

    let a_pref = store
        .list_memory_facts(Some("conf-a"), Some("preference"), None, 100, 0)
        .await
        .unwrap();
    assert!(
        a_pref.iter().all(|f| f["category"] == "preference"),
        "category filter should hold"
    );

    let all = store
        .list_memory_facts(None, None, None, 100, 0)
        .await
        .unwrap();
    assert!(
        all.len() >= 3,
        "unfiltered should return >= 3, got {}",
        all.len()
    );

    for row in &all {
        assert!(row.get("id").is_some(), "row should have id");
        assert!(row.get("agent_id").is_some(), "row should have agent_id");
        assert!(row.get("content").is_some(), "row should have content");
    }
}

pub async fn memory_get_by_id_round_trip<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "conf-gbi",
            "preference",
            "k1",
            "Milk no sugar",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();

    let listed = store
        .list_memory_facts(Some("conf-gbi"), None, None, 10, 0)
        .await
        .unwrap();
    assert!(!listed.is_empty(), "should have at least one fact");
    let id = listed[0]["id"].as_str().expect("id should be a string");

    let fetched = store
        .get_memory_fact_by_id(id)
        .await
        .expect("get_by_id should not error");
    assert!(fetched.is_some(), "fact should exist by id");
    let fact = fetched.unwrap();
    assert_eq!(fact["content"], "Milk no sugar");

    let missing = store
        .get_memory_fact_by_id("nonexistent-id-xyz")
        .await
        .expect("get_by_id for missing should not error");
    assert!(missing.is_none(), "missing id should return None");
}

pub async fn memory_delete_fact_removes_it<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "conf-del",
            "preference",
            "k1",
            "To be deleted",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();

    let listed = store
        .list_memory_facts(Some("conf-del"), None, None, 10, 0)
        .await
        .unwrap();
    assert!(!listed.is_empty(), "should have at least one fact");
    let id = listed[0]["id"].as_str().expect("id should be a string");

    let deleted = store
        .delete_memory_fact(id)
        .await
        .expect("delete should not error");
    assert!(deleted, "delete should return true for existing fact");

    let gone = store
        .get_memory_fact_by_id(id)
        .await
        .expect("get after delete should not error");
    assert!(gone.is_none(), "fact should be gone after delete");
}

pub async fn memory_archive_fact_hides_from_listing<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "conf-arch",
            "preference",
            "k1",
            "To be archived",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();

    let listed = store
        .list_memory_facts(Some("conf-arch"), None, None, 10, 0)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1, "should see one active fact");
    let id = listed[0]["id"].as_str().expect("id should be a string");

    let archived = store
        .archive_fact(id)
        .await
        .expect("archive should not error");
    assert!(archived, "archive should return true for existing fact");

    let after = store
        .list_memory_facts(Some("conf-arch"), None, None, 10, 0)
        .await
        .unwrap();
    assert!(
        after.is_empty(),
        "archived fact should not appear in listing"
    );

    let still_exists = store
        .get_memory_fact_by_id(id)
        .await
        .expect("get_by_id should not error");
    assert!(
        still_exists.is_some(),
        "archived fact should still be retrievable by id"
    );
}

pub async fn memory_supersede_fact_succeeds<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact("conf-sup", "preference", "k1", "Old fact", 0.9, None, None)
        .await
        .unwrap();

    let listed = store
        .list_memory_facts(Some("conf-sup"), None, None, 10, 0)
        .await
        .unwrap();
    let old_id = listed[0]["id"].as_str().expect("id should be a string");

    store
        .supersede_fact(old_id, "replacement-fact-id", chrono::Utc::now())
        .await
        .expect("supersede should not error");
}

pub async fn memory_upsert_typed_fact_round_trip<S: MemoryFactStore>(store: &S) {
    let fact_id = "conf-typed-001";
    let fact = MemoryFact {
        id: fact_id.to_string(),
        session_id: None,
        agent_id: "conf-typed".to_string(),
        scope: "session".to_string(),
        category: "preference".to_string(),
        key: "k1".to_string(),
        content: "Typed fact content".to_string(),
        confidence: 0.95,
        mention_count: 0,
        source_summary: None,
        embedding: None,
        ward_id: "__global__".to_string(),
        contradicted_by: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
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

    store
        .upsert_typed_fact(fact, None)
        .await
        .expect("upsert_typed_fact should not error");

    let fetched = store
        .get_memory_fact_by_id(fact_id)
        .await
        .expect("get_by_id should not error");
    assert!(fetched.is_some(), "typed fact should exist");
    let row = fetched.unwrap();
    assert_eq!(row["content"], "Typed fact content");
}

pub async fn memory_hybrid_search_finds_match<S: MemoryFactStore>(store: &S) {
    let _ = store
        .save_fact(
            "conf-hybrid",
            "preference",
            "k1",
            "Dark roast coffee beans",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let _ = store
        .save_fact(
            "conf-hybrid",
            "preference",
            "k2",
            "Herbal tea at night",
            0.8,
            None,
            None,
        )
        .await
        .unwrap();

    let results = store
        .search_memory_facts_hybrid(Some("conf-hybrid"), "coffee", "fts", 10, None, None, None)
        .await
        .expect("hybrid search should not error");
    assert!(
        !results.is_empty(),
        "hybrid search should find at least one match"
    );
    assert!(
        results
            .iter()
            .any(|r| r["content"].as_str().unwrap_or("").contains("coffee")),
        "results should contain the coffee fact"
    );
}

// =============================================================================
// Belief store conformance
// =============================================================================

use zbot_stores_traits::{Belief, BeliefStore};

pub async fn belief_upsert_get_round_trip<S: BeliefStore>(store: &S) {
    let now = chrono::Utc::now();
    let belief = Belief {
        id: "belief-conf-upsert-get".to_string(),
        partition_id: "partition-conf".to_string(),
        subject: "user.preference".to_string(),
        content: "User prefers concise responses".to_string(),
        confidence: 0.85,
        valid_from: Some(now),
        valid_until: None,
        source_fact_ids: vec!["fact-conf-1".to_string()],
        synthesizer_version: 1,
        reasoning: None,
        created_at: now,
        updated_at: now,
        superseded_by: None,
        stale: false,
        embedding: None,
    };

    store.upsert_belief(&belief).await.unwrap();

    let fetched = store
        .get_belief("partition-conf", "user.preference", None)
        .await
        .unwrap()
        .expect("belief should exist after upsert");

    assert_eq!(fetched.id, belief.id);
    assert_eq!(fetched.partition_id, belief.partition_id);
    assert_eq!(fetched.subject, belief.subject);
    assert_eq!(fetched.source_fact_ids, belief.source_fact_ids);
}
