use serde_json::json;
use zbot_engram_adapter::{AdapterConfig, EngramSidecarStores};
use zbot_stores_traits::{
    CompactionStore, DistillationStore, EmbeddingQueryIdentity, EpisodeStore, GoalStore,
    KgEpisodeStore, OutboxStore, PatternProcedureInsert, Procedure, ProcedureStore, RecallLogStore,
    SessionEpisode,
};

fn engram_config(root: &tempfile::TempDir) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.embedding_provider.dimensions = 2;
    config
}

fn query_identity() -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: "fastembed".to_string(),
        model: "BAAI/bge-small-en-v1.5".to_string(),
        dimensions: 2,
        prompt_profile: "query".to_string(),
        normalization: None,
    }
}

fn procedure(id: &str, name: &str) -> Procedure {
    Procedure {
        id: id.to_string(),
        agent_id: "agent-a".to_string(),
        ward_id: Some("ward-a".to_string()),
        name: name.to_string(),
        description: "Build safely".to_string(),
        trigger_pattern: Some("build".to_string()),
        steps: "[]".to_string(),
        parameters: None,
        success_count: 1,
        failure_count: 0,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: "2026-07-06T00:00:00Z".to_string(),
        updated_at: "2026-07-06T00:00:00Z".to_string(),
    }
}

fn episode(id: &str, outcome: &str, summary: &str) -> SessionEpisode {
    SessionEpisode {
        id: id.to_string(),
        session_id: format!("sess-{id}"),
        agent_id: "agent-a".to_string(),
        ward_id: "ward-a".to_string(),
        task_summary: summary.to_string(),
        outcome: outcome.to_string(),
        strategy_used: Some("spec first".to_string()),
        key_learnings: Some("tests caught it".to_string()),
        token_cost: Some(42),
        embedding: None,
        created_at: "2026-07-06T00:00:00Z".to_string(),
    }
}

#[tokio::test]
async fn procedure_sidecar_round_trips_searches_and_updates() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramSidecarStores::open(engram_config(&root)).expect("store");
    let identity = query_identity();

    ProcedureStore::upsert_procedure(
        &store,
        procedure("proc-1", "build-agent"),
        Some(vec![1.0, 0.0]),
    )
    .await
    .expect("upsert procedure");

    assert_eq!(
        ProcedureStore::list_by_ward(&store, "ward-a", 10)
            .await
            .expect("list procedures")
            .len(),
        1
    );
    assert_eq!(
        ProcedureStore::search_procedures_by_similarity_with_identity(
            &store,
            &[1.0, 0.0],
            Some(&identity),
            "agent-a",
            None,
            10
        )
        .await
        .expect("search procedures")
        .len(),
        1
    );
    let mut same_basename = identity.clone();
    same_basename.model = "other/bge-small-en-v1.5".to_string();
    assert!(
        ProcedureStore::search_procedures_by_similarity_with_identity(
            &store,
            &[1.0, 0.0],
            Some(&same_basename),
            "agent-a",
            None,
            10
        )
        .await
        .expect("same-basename procedure mismatch")
        .is_empty()
    );

    ProcedureStore::increment_success(&store, "proc-1", Some(10), Some(11))
        .await
        .expect("increment");
    let summary = ProcedureStore::get_procedure_summary_by_name(&store, "agent-a", "build-agent")
        .await
        .expect("summary")
        .expect("summary row");
    assert_eq!(summary.success_count, 2);
    assert_eq!(
        ProcedureStore::procedure_stats(&store)
            .await
            .expect("stats")
            .total,
        1
    );

    let inserted = ProcedureStore::insert_pattern_procedure(
        &store,
        PatternProcedureInsert {
            agent_id: "agent-a".to_string(),
            ward_id: Some("ward-a".to_string()),
            name: "mined".to_string(),
            description: "Mined pattern".to_string(),
            trigger_pattern: None,
            steps_json: "[]".to_string(),
            parameters_json: None,
            embedding: Some(vec![0.0, 1.0]),
            success_count: 2,
        },
    )
    .await
    .expect("insert pattern");
    assert!(inserted.starts_with("proc-"));
}

#[tokio::test]
async fn episode_sidecar_round_trips_searches_and_summarizes() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramSidecarStores::open(engram_config(&root)).expect("store");
    let identity = query_identity();

    EpisodeStore::insert_episode(
        &store,
        episode("1", "success", "Used Engram sidecars"),
        Some(vec![1.0, 0.0]),
    )
    .await
    .expect("insert episode");
    EpisodeStore::insert_episode(
        &store,
        episode("2", "failed", "Ignored tests"),
        Some(vec![0.0, 1.0]),
    )
    .await
    .expect("insert failed episode");

    assert_eq!(
        EpisodeStore::list_by_ward(&store, "ward-a", 10)
            .await
            .expect("list episodes")
            .len(),
        2
    );
    assert_eq!(
        EpisodeStore::search_episodes_by_similarity_with_identity(
            &store,
            "agent-a",
            &[1.0, 0.0],
            Some(&identity),
            0.9,
            10
        )
        .await
        .expect("search episodes")
        .len(),
        1
    );
    let mut same_basename = identity.clone();
    same_basename.model = "other/bge-small-en-v1.5".to_string();
    assert!(EpisodeStore::search_episodes_by_similarity_with_identity(
        &store,
        "agent-a",
        &[1.0, 0.0],
        Some(&same_basename),
        0.9,
        10
    )
    .await
    .expect("same-basename episode mismatch")
    .is_empty());
    assert_eq!(
        EpisodeStore::keyword_search_episodes(&store, "Engram", Some("ward-a"), 10)
            .await
            .expect("keyword")
            .len(),
        1
    );
    assert_eq!(
        EpisodeStore::fetch_recent_successful_by_ward(&store, "ward-a", 10)
            .await
            .expect("recent")
            .len(),
        1
    );
    assert_eq!(
        EpisodeStore::task_summaries_for_sessions(&store, &["sess-1".to_string()])
            .await
            .expect("summaries"),
        vec!["Used Engram sidecars".to_string()]
    );
}

#[tokio::test]
async fn kg_episode_sidecar_preserves_queue_lifecycle_and_payloads() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramSidecarStores::open(engram_config(&root)).expect("store");

    let id = KgEpisodeStore::upsert_pending(
        &store,
        "ward_file",
        "ward-a/src/main.rs#chunk-1",
        "hash-a",
        Some("sess-a"),
        "agent-a",
    )
    .await
    .expect("upsert pending");
    assert_eq!(
        KgEpisodeStore::upsert_pending(
            &store,
            "ward_file",
            "ward-a/src/main.rs#chunk-1",
            "hash-a",
            Some("sess-a"),
            "agent-a",
        )
        .await
        .expect("dedupe"),
        id
    );

    KgEpisodeStore::set_payload(&store, &id, "payload")
        .await
        .expect("payload");
    assert_eq!(
        KgEpisodeStore::get_payload(&store, &id)
            .await
            .expect("get payload")
            .as_deref(),
        Some("payload")
    );
    let claimed = KgEpisodeStore::claim_next_pending(&store)
        .await
        .expect("claim")
        .expect("claimed");
    assert_eq!(claimed["id"], id);
    assert_eq!(claimed["status"], "running");

    KgEpisodeStore::mark_failed(&store, &id, "bad extraction")
        .await
        .expect("failed");
    assert!(KgEpisodeStore::retry_if_eligible(&store, &id, 3)
        .await
        .expect("retry"));
    assert_eq!(
        KgEpisodeStore::count_pending_global(&store)
            .await
            .expect("pending"),
        1
    );
    let counts = KgEpisodeStore::status_counts_for_source(&store, "ward-a/src/main.rs")
        .await
        .expect("counts");
    assert_eq!(counts.pending, 1);
}

#[tokio::test]
async fn auxiliary_sidecars_cover_goals_recall_and_distillation() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramSidecarStores::open(engram_config(&root)).expect("store");

    let goal_id = GoalStore::create_goal(
        &store,
        json!({
            "agent_id": "agent-a",
            "state": "active",
            "title": "ship sidecars"
        }),
    )
    .await
    .expect("create goal");
    assert_eq!(
        GoalStore::list_active_goals(&store, "agent-a")
            .await
            .expect("active goals")
            .len(),
        1
    );
    GoalStore::update_goal_state(&store, &goal_id, "satisfied")
        .await
        .expect("goal state");
    assert!(GoalStore::list_active_goals(&store, "agent-a")
        .await
        .expect("active after update")
        .is_empty());

    RecallLogStore::log_recall(&store, "sess-a", "fact-a")
        .await
        .expect("recall");
    assert_eq!(
        RecallLogStore::get_keys_for_sessions(&store, &["sess-a".to_string()])
            .await
            .expect("recall keys"),
        vec!["fact-a".to_string()]
    );

    DistillationStore::record_distillation_pending(&store, "sess-a", "pending", None)
        .await
        .expect("pending distillation");
    DistillationStore::record_distillation_success(&store, "sess-a", 1, 2, 3, true, 44)
        .await
        .expect("success distillation");
    let run = DistillationStore::get_run_by_session(&store, "sess-a")
        .await
        .expect("distillation run")
        .expect("run");
    assert_eq!(run["status"], "success");
    assert_eq!(run["facts_extracted"], 1);
}

#[tokio::test]
async fn compaction_and_outbox_sidecars_record_lifecycle_state() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramSidecarStores::open(engram_config(&root)).expect("store");

    CompactionStore::record_merge(&store, "run-a", "entity-loser", "entity-winner", "same")
        .await
        .expect("merge");
    CompactionStore::record_prune(&store, "run-a", Some("entity-old"), None, "stale")
        .await
        .expect("prune");
    let summary = CompactionStore::latest_run_summary(&store)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(summary.run_id, "run-a");
    assert_eq!(summary.merges, 1);
    assert_eq!(summary.prunes, 1);

    let outbox_id = OutboxStore::insert_item(
        &store,
        "adapter-a",
        "capability-a",
        &json!({ "ok": true }),
        Some("sess-a"),
        None,
        Some("agent-a"),
    )
    .expect("insert outbox");
    OutboxStore::mark_inflight(&store, &outbox_id).expect("inflight");
    assert_eq!(
        OutboxStore::reset_inflight(&store, "adapter-a").expect("reset"),
        1
    );
    OutboxStore::mark_sent(&store, &outbox_id).expect("sent");
}
