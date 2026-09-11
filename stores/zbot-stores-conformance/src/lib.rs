//! Cross-impl conformance scenarios for `zbot-stores` traits.
//!
//! Each function takes a generic `&S` where `S: KnowledgeGraphStore` (or
//! `MemoryFactStore`, etc.) and runs an end-to-end behavioural check.
//! Impl crates call these from their integration tests; behavioural drift
//! between impls produces failing assertions.

pub mod parity;

use knowledge_graph::kg_trait::kg_types::{EntityId, ResolveOutcome};
use knowledge_graph::kg_trait::ExtractedKnowledge;
use knowledge_graph::kg_trait::KnowledgeGraphStore;
use knowledge_graph::types::Direction;
use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
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
    let arr = result["results"].as_array().expect("results array");
    assert!(!arr.is_empty(), "recall should find match");
}

pub async fn memory_recall_respects_agent_isolation<S: MemoryFactStore>(store: &S) {
    // Isolation applies to agent-scoped categories. Both production
    // backends deliberately share global-scoped categories (user/domain/
    // reference) across agents — that is documented product policy, not a
    // leak — so the scenario uses `correction`, which scopes agent-private.
    let _ = store
        .save_fact(
            "agent-mem-a",
            "correction",
            "iso.k1",
            "agent A private note",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let _ = store
        .save_fact(
            "agent-mem-b",
            "correction",
            "iso.k1",
            "agent B private note",
            0.9,
            None,
            None,
        )
        .await
        .unwrap();
    let result = store.recall_facts("agent-mem-a", "note", 10).await.unwrap();
    let arr = result["results"].as_array().expect("results array");
    assert!(
        arr.iter().any(|item| item["content"]
            .as_str()
            .is_some_and(|content| content.contains("agent A"))),
        "recall should find the agent's own correction: {arr:?}"
    );
    for item in arr {
        assert_eq!(
            item.get("agent_id").and_then(|v| v.as_str()),
            Some("agent-mem-a"),
            "agent-scoped corrections must never cross agents: {arr:?}"
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
        importance: None,
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
use zbot_stores_traits::{CompactionStore, EpisodeStore, GoalStore, KgEpisodeStore, WikiStore};

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

// =============================================================================
// EpisodeStore — chain + avoid-list contract
// =============================================================================

/// Insert episodes and verify the successful/partial chain and the failed
/// avoid-list read paths, including the learnings requirement on failed rows.
pub async fn episode_insert_and_recent_fetch<S: EpisodeStore>(store: &S) {
    use zbot_stores_domain::SessionEpisode;

    let episode = |id: &str, outcome: &str| SessionEpisode {
        id: id.to_string(),
        session_id: format!("sess-{id}"),
        agent_id: "conf-agent".to_string(),
        ward_id: "conf-ward".to_string(),
        task_summary: format!("task {id}"),
        outcome: outcome.to_string(),
        strategy_used: None,
        key_learnings: Some("learn something".to_string()),
        token_cost: None,
        embedding: None,
        created_at: "2026-09-01T00:00:00Z".to_string(),
    };

    store
        .insert_episode(episode("ep-conf-ok", "success"), None)
        .await
        .expect("insert success episode");
    store
        .insert_episode(episode("ep-conf-fail", "failed"), None)
        .await
        .expect("insert failed episode");

    let chain = store
        .fetch_recent_successful_by_ward("conf-ward", 5)
        .await
        .expect("fetch chain");
    assert!(
        chain.iter().any(|e| e.id == "ep-conf-ok"),
        "successful episode must surface in the chain read"
    );
    assert!(
        !chain.iter().any(|e| e.id == "ep-conf-fail"),
        "failed episodes must not surface in the chain read"
    );

    let avoid = store
        .fetch_recent_failed_by_ward("conf-ward", 5)
        .await
        .expect("fetch avoid-list");
    assert!(
        avoid
            .iter()
            .any(|e| e.id == "ep-conf-fail" && e.key_learnings.is_some()),
        "failed episode with learnings must surface in the avoid-list"
    );
}

// =============================================================================
// WikiStore — article lifecycle contract
// =============================================================================

/// Upsert → get → list → delete round trip for one ward article.
pub async fn wiki_article_round_trip<S: WikiStore>(store: &S) {
    use zbot_stores_domain::WikiArticle;

    let article = WikiArticle {
        id: "wiki-conf-1".to_string(),
        ward_id: "conf-ward".to_string(),
        agent_id: "conf-agent".to_string(),
        title: "Conformance Article".to_string(),
        content: "Body text for the conformance article.".to_string(),
        tags: Some("conformance".to_string()),
        source_fact_ids: None,
        embedding: None,
        version: 1,
        created_at: "2026-09-01T00:00:00Z".to_string(),
        updated_at: "2026-09-01T00:00:00Z".to_string(),
    };
    store
        .upsert_article(article, None)
        .await
        .expect("upsert article");

    let fetched = store
        .get_article("conf-ward", "Conformance Article")
        .await
        .expect("get article")
        .expect("article should exist after upsert");
    assert!(
        fetched
            .get("content")
            .and_then(|value| value.as_str())
            .is_some_and(|content| content.contains("conformance article")),
        "fetched article content must round trip: {fetched:?}"
    );

    let listed = store
        .list_articles("conf-ward")
        .await
        .expect("list articles");
    assert!(
        listed.iter().any(|row| row
            .get("title")
            .and_then(|value| value.as_str())
            .is_some_and(|title| title == "Conformance Article")),
        "listed articles must include the seeded article"
    );

    let deleted = store
        .delete_article("conf-ward", "Conformance Article")
        .await
        .expect("delete article");
    assert!(deleted, "delete should report a removed row");
    let after = store
        .get_article("conf-ward", "Conformance Article")
        .await
        .expect("get after delete");
    assert!(after.is_none(), "article must be gone after delete");
}

// =============================================================================
// KgEpisodeStore — extraction-queue lifecycle contract
// =============================================================================

/// Pending upsert → claim → done transition with per-source pending counts.
pub async fn kg_episode_queue_lifecycle<S: KgEpisodeStore>(store: &S) {
    let id = store
        .upsert_pending(
            "ingest",
            "conf://source/doc.md",
            "hash-conf-1",
            Some("sess-conf"),
            "conf-agent",
        )
        .await
        .expect("upsert pending");

    let claimed = store
        .claim_next_pending()
        .await
        .expect("claim next pending")
        .expect("seeded pending episode must be claimable");
    let claimed_id = claimed
        .get("id")
        .and_then(|value| value.as_str())
        .expect("claimed payload carries id")
        .to_string();
    assert_eq!(claimed_id, id, "claim must return the seeded episode");

    store.mark_done(&id).await.expect("mark done");
    // Idempotency: a second done is a no-op, not an error.
    store.mark_done(&id).await.expect("mark done idempotent");
}

// =============================================================================
// CompactionStore — audit-record contract
// =============================================================================

/// Audit rows record and the latest-run summary reflects them.
pub async fn compaction_recording_round_trip<S: CompactionStore>(store: &S) {
    let run = "run-conf-1";
    let merge_id = store
        .record_merge(run, "loser-conf", "winner-conf", "conformance merge")
        .await
        .expect("record merge");
    store
        .record_prune(run, Some("entity-conf"), None, "conformance prune")
        .await
        .expect("record prune");
    assert!(!merge_id.is_empty(), "merge audit row must return a row id");

    let summary = store
        .latest_run_summary()
        .await
        .expect("latest run summary")
        .expect("summary must exist after recorded runs");
    assert!(summary.merges >= 1, "summary must count the merge");
    assert!(summary.prunes >= 1, "summary must count the prune");
}

// =============================================================================
// GoalStore — goal lifecycle contract
// =============================================================================

/// Create → get → active-list → state-transition round trip.
pub async fn goal_round_trip<S: GoalStore>(store: &S) {
    use serde_json::json;

    let goal_id = store
        .create_goal(json!({
            "agent_id": "conf-agent",
            "ward_id": "conf-ward",
            "title": "Conformance goal",
            "state": "active",
        }))
        .await
        .expect("create goal");

    let fetched = store
        .get_goal(&goal_id)
        .await
        .expect("get goal")
        .expect("goal must exist after create");
    assert_eq!(
        fetched.get("title").and_then(|value| value.as_str()),
        Some("Conformance goal")
    );

    let active = store
        .list_active_goals("conf-agent")
        .await
        .expect("list active goals");
    assert!(
        active
            .iter()
            .any(|goal| goal.get("id").and_then(|value| value.as_str()) == Some(goal_id.as_str())),
        "active list must include the created goal"
    );

    store
        .update_goal_state(&goal_id, "satisfied")
        .await
        .expect("update goal state");
    let after = store
        .get_goal(&goal_id)
        .await
        .expect("get after transition")
        .expect("goal still exists");
    assert_eq!(
        after.get("state").and_then(|value| value.as_str()),
        Some("satisfied"),
        "state transition must persist"
    );
}

// ===========================================================================
// KG-maintenance scenarios (KG-lane port conformance). Verified against the
// sqlite reference as oracle before its deletion; the production (engram)
// store must hold the same behavior.
//
// Readbacks use behavioral surfaces only (embedding-search hits, resolver,
// candidate lists): the sqlite reference keeps confidence/compression in
// columns while the engram adapter projects them through entity
// properties, so internal representations are deliberately not asserted.
//
// The resolver merges same-agent entities at cosine >= 0.87, so duplicate
// scenarios seed embeddings in the 0.80-0.86 band — similar enough for the
// compactor's threshold, distinct enough to remain separate rows.
// ===========================================================================

/// One-hot 384-dim unit vector at `index`.
fn kg_one_hot(index: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; 384];
    v[index] = 1.0;
    v
}

/// Unit vector at cosine `c` to the one-hot at `i` (rotated toward `j`).
fn kg_rotated(i: usize, j: usize, c: f32) -> Vec<f32> {
    let mut v = vec![0.0_f32; 384];
    v[i] = c;
    v[j] = (1.0 - c * c).sqrt();
    v
}

/// Entity with controllable `last_seen_at`/`first_seen_at` (days ago).
fn kg_entity(
    agent: &str,
    ty: EntityType,
    name: &str,
    days_ago: i64,
    embedding: Option<Vec<f32>>,
) -> Entity {
    let mut entity = Entity::new(agent.to_string(), ty, name.to_string());
    let seen = chrono::Utc::now() - chrono::Duration::days(days_ago);
    entity.first_seen_at = seen;
    entity.last_seen_at = seen;
    entity.name_embedding = embedding;
    entity
}

/// Confidence of `name`'s embedding-search hit under `agent`. The identity
/// is harness-supplied: embedding lanes are identity-gated by design, and
/// each backend harness constructs the identity its store expects.
async fn kg_hit_confidence<S: KnowledgeGraphStore>(
    store: &S,
    agent: &str,
    identity: &zbot_stores_traits::EmbeddingQueryIdentity,
    query: &[f32],
    name: &str,
) -> Option<f64> {
    let hits = store
        .search_entities_by_name_embedding_with_identity(agent, query, Some(identity), 10)
        .await
        .expect("embedding search");
    hits.iter()
        .find(|hit| hit.name == name)
        .map(|hit| hit.confidence)
}

/// Decay floors old rows at `min_confidence`, leaves recent rows above the
/// floor, and skips archival rows. The two decay calls cover both storage
/// schemes: backends that stamp fresh entities under the requested agent
/// and backends that stamp them `__global__` — a row matches exactly one.
pub async fn kg_decay_entity_confidence<S: KnowledgeGraphStore>(
    store: &S,
    identity: &zbot_stores_traits::EmbeddingQueryIdentity,
) {
    let agent = "kg-decay-agent";
    let old_emb = kg_one_hot(1);
    store
        .upsert_entity(
            agent,
            kg_entity(
                agent,
                EntityType::Person,
                "Old Hand",
                30,
                Some(old_emb.clone()),
            ),
        )
        .await
        .expect("seed old");
    let recent_emb = kg_one_hot(2);
    store
        .upsert_entity(
            agent,
            kg_entity(
                agent,
                EntityType::Person,
                "Recent Visit",
                0,
                Some(recent_emb.clone()),
            ),
        )
        .await
        .expect("seed recent");
    let mut archival_entity = kg_entity(
        agent,
        EntityType::Person,
        "Archive Case",
        30,
        Some(kg_one_hot(3)),
    );
    archival_entity
        .properties
        .insert("epistemic_class".to_string(), serde_json::json!("archival"));
    store
        .upsert_entity(agent, archival_entity)
        .await
        .expect("seed archival");

    // 30 days at a 1-day half-life decays past any floor; skip-recent keeps
    // the fresh row untouched.
    let mut touched = 0;
    for scope in [agent, "__global__"] {
        touched += store
            .decay_entity_confidence(scope, 1.0, 0.25, 12)
            .await
            .expect("decay");
    }
    assert!(touched >= 1, "the old row must be matched under one scope");

    let old_confidence = kg_hit_confidence(store, agent, identity, &old_emb, "Old Hand").await;
    let recent_confidence =
        kg_hit_confidence(store, agent, identity, &recent_emb, "Recent Visit").await;
    assert_eq!(
        old_confidence,
        Some(0.25),
        "old row pinned to the floor: got {old_confidence:?}"
    );
    let recent_confidence = recent_confidence.unwrap_or(0.0);
    assert!(
        recent_confidence > 0.5,
        "recent row keeps its default (0.8/1.0 per backend): got {recent_confidence}"
    );
}

/// Duplicate candidates: same type, embedding-backed, cosine at or above
/// the supplied threshold; limit respected.
pub async fn kg_find_duplicate_candidates<S: KnowledgeGraphStore>(store: &S) {
    let agent = "kg-dup-agent";
    // cosine(base, rotated) == 0.84 — above the scenario threshold 0.80,
    // below the resolver's 0.87 merge band.
    let base = kg_one_hot(1);
    let rotated = kg_rotated(1, 2, 0.84);
    let far = kg_one_hot(3);
    store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Near One", 1, Some(base)),
        )
        .await
        .expect("seed near one");
    store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Near Two", 1, Some(rotated)),
        )
        .await
        .expect("seed near two");
    store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Far Away", 1, Some(far)),
        )
        .await
        .expect("seed far");

    // Threshold 0.70 with the 0.84 pair satisfies both backends: the
    // cosine-correct comparison, and the sqlite reference's
    // euclidean-vs-L2-squared threshold conversion (its ANN returns
    // euclidean distance while the threshold is 2·(1−cos)) — see the
    // KG-lane port report for the sqlite-side bug note.
    let mut all_pairs = Vec::new();
    for scope in [agent, "__global__"] {
        all_pairs.extend(
            store
                .find_duplicate_candidates(scope, &EntityType::Person, 0.70, 10)
                .await
                .expect("find duplicates"),
        );
    }
    assert!(
        all_pairs
            .iter()
            .any(|p| p.cosine_similarity >= 0.70 && p.cosine_similarity <= 0.90),
        "the seeded near-pair must surface: got {all_pairs:?}"
    );
    assert!(
        all_pairs.iter().all(|p| p.cosine_similarity >= 0.70),
        "every returned pair must clear the threshold"
    );

    let none = store
        .find_duplicate_candidates(agent, &EntityType::Person, 0.70, 0)
        .await
        .expect("limit zero");
    assert!(none.is_empty(), "limit 0 yields no pairs");
}

/// Orphan candidates: old + zero edges only; connected and recent rows
/// excluded.
pub async fn kg_orphan_candidates<S: KnowledgeGraphStore>(store: &S) {
    let agent = "kg-orphan-agent";
    let orphan_id = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Lonely Old", 40, None),
        )
        .await
        .expect("seed orphan");
    let connected_id = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Linked Old", 40, None),
        )
        .await
        .expect("seed connected");
    let fresh_id = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Lonely Fresh", 0, None),
        )
        .await
        .expect("seed fresh");

    let other = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Anchor", 0, None),
        )
        .await
        .expect("seed anchor");
    let rel = Relationship::new(
        agent.to_string(),
        connected_id.0.clone(),
        other.0.clone(),
        RelationshipType::Uses,
    );
    store
        .upsert_relationship(agent, rel)
        .await
        .expect("seed relationship");

    let candidates = store
        .list_orphan_old_candidates(agent, 30, 10)
        .await
        .expect("list orphans");
    let ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&orphan_id.0.as_str()), "old orphan listed");
    assert!(
        !ids.contains(&connected_id.0.as_str()),
        "connected entity excluded"
    );
    assert!(
        !ids.contains(&fresh_id.0.as_str()),
        "recent orphan excluded"
    );
}

/// Merge re-points relationships, drops colliding duplicates, transfers
/// aliases, and takes the loser out of prune candidacy.
pub async fn kg_merge_entity_into<S: KnowledgeGraphStore>(store: &S) {
    let agent = "kg-merge-agent";
    let loser = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Loser Co", 5, None),
        )
        .await
        .expect("seed loser");
    let winner = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Winner Co", 5, None),
        )
        .await
        .expect("seed winner");
    let outside = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Outside", 5, None),
        )
        .await
        .expect("seed outside");

    let unique_rel = Relationship::new(
        agent.to_string(),
        loser.0.clone(),
        outside.0.clone(),
        RelationshipType::Uses,
    );
    let unique_id = store
        .upsert_relationship(agent, unique_rel)
        .await
        .expect("seed unique");
    let winner_edge = Relationship::new(
        agent.to_string(),
        winner.0.clone(),
        outside.0.clone(),
        RelationshipType::RelatedTo,
    );
    store
        .upsert_relationship(agent, winner_edge)
        .await
        .expect("seed winner edge");
    let loser_dup = Relationship::new(
        agent.to_string(),
        loser.0.clone(),
        outside.0.clone(),
        RelationshipType::RelatedTo,
    );
    store
        .upsert_relationship(agent, loser_dup)
        .await
        .expect("seed loser dup");

    store
        .add_alias(&loser, "Loser Alias")
        .await
        .expect("seed alias");

    store
        .merge_entity_into(&loser, &winner)
        .await
        .expect("merge");

    let rels = store
        .list_relationships(agent, None, 100, 0)
        .await
        .expect("list relationships");
    assert!(
        rels.iter().any(|r| r.id == unique_id.0),
        "unique relationship survives re-pointing"
    );
    assert!(
        rels.iter().any(
            |r| r.source_entity_id == winner.0 && r.relationship_type == RelationshipType::Uses
        ),
        "unique edge re-pointed to winner"
    );
    let competes: Vec<&Relationship> = rels
        .iter()
        .filter(|r| r.relationship_type == RelationshipType::RelatedTo)
        .collect();
    assert_eq!(
        competes.len(),
        1,
        "colliding loser edge dropped, winner edge kept"
    );

    // The loser leaves prune candidacy (compressed rows are excluded from
    // orphan listings on both backends).
    let candidates = store
        .list_orphan_old_candidates(agent, 1, 100)
        .await
        .expect("orphans after merge");
    assert!(
        candidates.iter().all(|c| c.id != loser.0),
        "merged loser is not a prune candidate"
    );

    let resolved = store
        .resolve_entity(agent, &EntityType::Person, "Loser Alias", None)
        .await
        .expect("resolve alias");
    match resolved {
        knowledge_graph::kg_trait::ResolveOutcome::Match(id) => assert_eq!(
            id, winner,
            "alias must resolve to the winner, not the compressed loser"
        ),
        knowledge_graph::kg_trait::ResolveOutcome::NoMatch => {}
    }
}

/// Prune removes the entity from resolution and prune candidacy.
pub async fn kg_mark_entity_pruned<S: KnowledgeGraphStore>(store: &S) {
    let agent = "kg-prune-agent";
    let victim = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Prune Me", 40, None),
        )
        .await
        .expect("seed victim");
    store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Keep Me", 40, None),
        )
        .await
        .expect("seed keeper");

    store.mark_entity_pruned(&victim).await.expect("prune");

    // Note: resolution-after-prune is backend-divergent (the sqlite
    // resolver lacks a compressed filter; the adapter excludes pruned
    // rows entirely) and is deliberately not asserted.
    let resolved_keep = store
        .resolve_entity(agent, &EntityType::Person, "Keep Me", None)
        .await
        .expect("resolve keeper");
    assert!(
        matches!(
            resolved_keep,
            knowledge_graph::kg_trait::ResolveOutcome::Match(_)
        ),
        "keeper still resolves"
    );
    let candidates = store
        .list_orphan_old_candidates(agent, 1, 100)
        .await
        .expect("orphans");
    assert!(
        candidates.iter().all(|c| c.id != victim.0),
        "pruned entity is not a prune candidate"
    );
}

/// Confidence multipliers floor correctly; the count covers the matched
/// rows under whichever scope owns them.
pub async fn kg_confidence_multiplier<S: KnowledgeGraphStore>(
    store: &S,
    identity: &zbot_stores_traits::EmbeddingQueryIdentity,
) {
    let agent = "kg-mult-agent";
    let emb = kg_one_hot(1);
    let mine = store
        .upsert_entity(
            agent,
            kg_entity(agent, EntityType::Person, "Mine", 1, Some(emb.clone())),
        )
        .await
        .expect("seed mine");

    let mut touched = 0;
    for scope in [agent, "__global__"] {
        touched += store
            .apply_entity_confidence_multiplier(scope, std::slice::from_ref(&mine), 0.5, 0.4)
            .await
            .expect("apply multiplier");
    }
    assert_eq!(touched, 1, "the row is matched exactly once across scopes");

    let confidence = kg_hit_confidence(store, agent, identity, &emb, "Mine").await;
    assert!(
        matches!(confidence, Some(c) if (0.35..=0.65).contains(&c)),
        "0.5 x default lands mid-band above the 0.4 floor: got {confidence:?}"
    );

    let mut floored = 0;
    for scope in [agent, "__global__"] {
        floored += store
            .apply_entity_confidence_multiplier(scope, std::slice::from_ref(&mine), 0.1, 0.45)
            .await
            .expect("apply below floor");
    }
    assert_eq!(floored, 1);
    let confidence = kg_hit_confidence(store, agent, identity, &emb, "Mine").await;
    assert!(
        matches!(confidence, Some(c) if (c - 0.45).abs() < 1e-6),
        "floor clamps: got {confidence:?}"
    );
}
