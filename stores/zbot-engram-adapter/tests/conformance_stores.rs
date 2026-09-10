//! Belief + belief-contradiction conformance for the engram adapter,
//! plus the sidecar trait families that had no conformance coverage:
//! episodes, wiki, kg-episodes, compaction, goals.

use zbot_engram_adapter::{
    AdapterConfig, EngramBeliefStore, EngramProvider, EngramSidecarStores, EngramWikiStore,
};
use zbot_stores_conformance as conf;

fn config(root: &tempfile::TempDir) -> AdapterConfig {
    AdapterConfig::engram_for_data_root(root.path(), "engram.db")
}

fn belief_store(root: &tempfile::TempDir) -> EngramBeliefStore {
    let config = config(root);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    EngramBeliefStore::from_provider(config, &provider).expect("belief store opens")
}

fn sidecars(root: &tempfile::TempDir) -> EngramSidecarStores {
    let config = config(root);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    EngramSidecarStores::from_provider(config, &provider).expect("sidecar stores open")
}

fn wiki_store(root: &tempfile::TempDir) -> EngramWikiStore {
    let config = config(root);
    let provider = EngramProvider::open(config.clone()).expect("provider opens");
    EngramWikiStore::from_provider(config, &provider).expect("wiki store opens")
}

#[tokio::test]
async fn belief_upsert_get_round_trip() {
    let root = tempfile::tempdir().expect("root");
    conf::belief_upsert_get_round_trip(&belief_store(&root)).await;
}

#[tokio::test]
async fn episode_insert_and_recent_fetch() {
    let root = tempfile::tempdir().expect("root");
    conf::episode_insert_and_recent_fetch(&sidecars(&root)).await;
}

#[tokio::test]
async fn wiki_article_round_trip() {
    let root = tempfile::tempdir().expect("root");
    conf::wiki_article_round_trip(&wiki_store(&root)).await;
}

#[tokio::test]
async fn kg_episode_queue_lifecycle() {
    let root = tempfile::tempdir().expect("root");
    conf::kg_episode_queue_lifecycle(&sidecars(&root)).await;
}

#[tokio::test]
async fn compaction_recording_round_trip() {
    let root = tempfile::tempdir().expect("root");
    conf::compaction_recording_round_trip(&sidecars(&root)).await;
}

#[tokio::test]
async fn goal_round_trip() {
    let root = tempfile::tempdir().expect("root");
    conf::goal_round_trip(&sidecars(&root)).await;
}
