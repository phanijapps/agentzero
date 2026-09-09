//! End-to-end recall tests.
//!
//! The sqlite fact-store identity-fail-closed test that used to live here
//! died with the retired sqlite fact store — the engram adapter covers the
//! behavior (`memory_sidecars.rs`: mismatched identity → no hits).

use std::sync::Arc;

use tempfile::tempdir;

use agent_primitives::vault_paths::VaultPaths;
use gateway_execution::recall::{ItemKind, MemoryRecall};
use gateway_services::RecallConfig;
use zbot_stores_sqlite::{
    EpisodeRepository, KnowledgeDatabase, SessionEpisode, SqliteVecIndex, VectorIndex,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_unified_injects_previous_episodes_for_ward() {
    let tmp = tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
    let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));

    let ep_vec: Arc<dyn VectorIndex> = Arc::new(
        SqliteVecIndex::new(db.clone(), "session_episodes_index", "episode_id")
            .expect("vec index init"),
    );
    let episode_repo = Arc::new(EpisodeRepository::new(db.clone(), ep_vec));

    let ward = "finance";
    let ep = SessionEpisode {
        id: "ep-prev-1".to_string(),
        session_id: "sess-prev-1".to_string(),
        agent_id: "root".to_string(),
        ward_id: ward.to_string(),
        task_summary: "reviewed Q3 earnings".to_string(),
        outcome: "success".to_string(),
        strategy_used: None,
        key_learnings: Some("prefer the 10-Q over press releases".to_string()),
        token_cost: None,
        embedding: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    episode_repo.insert(&ep).unwrap();

    // No memory store wired — the facts lane is optional; this test proves
    // the episode chain injects regardless.
    let config = Arc::new(RecallConfig::default());
    let episode_store: Arc<dyn zbot_stores_traits::EpisodeStore> = Arc::new(
        zbot_stores_sqlite::GatewayEpisodeStore::new(episode_repo.clone()),
    );
    let mut recall = MemoryRecall::new(None, config);
    recall.set_episode_store(episode_store);

    let items = recall
        .recall_unified("root", "earnings", Some(ward), &[], 10)
        .await
        .expect("recall_unified succeeds");

    assert!(
        items.iter().any(|i| i.kind == ItemKind::Episode),
        "expected at least one episode item for ward={ward}, got {items:?}"
    );
}
