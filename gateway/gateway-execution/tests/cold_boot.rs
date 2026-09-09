//! Cold-boot baseline: how long does it take to initialize a knowledge.db
//! pointing at an already-populated 10k-entity vault?
//!
//! Phase 5 acceptance: first successful query returns in < 10s.

use std::sync::Arc;
use std::time::Instant;

use tempfile::tempdir;

mod common;

use knowledge_graph::{Entity, EntityType};

fn normalized(v: Vec<f32>) -> Vec<f32> {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n < 1e-9 {
        v
    } else {
        v.into_iter().map(|x| x / n).collect()
    }
}

async fn kg_store_upsert(
    kg: &Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>,
    agent_id: &str,
    entity: Entity,
) {
    kg.upsert_entity(agent_id, entity)
        .await
        .expect("seed store");
}

fn make_embedding(seed: u64) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15);
    let v: Vec<f32> = (0..384)
        .map(|_| {
            s = s.wrapping_add(0xBF58476D1CE4E5B9);
            s ^= s >> 30;
            s = s.wrapping_mul(0x94D049BB133111EB);
            ((s & 0xFFFF) as f32 / 65535.0) - 0.5
        })
        .collect();
    normalized(v)
}

#[tokio::test]
async fn cold_boot_under_10s_with_10k_entities() {
    let tmp = tempdir().expect("tempdir");

    // Seed 10k entities through the production (engram) store.
    let (kg_store,) = {
        let (kg, _episodes) = common::engram_stores::kg_and_episode_stores(&tmp);
        let types = [
            EntityType::Person,
            EntityType::Organization,
            EntityType::Location,
            EntityType::Event,
            EntityType::Concept,
        ];
        for i in 0..10_000u64 {
            let t = types[(i as usize) % types.len()].clone();
            let mut e = Entity::new("root".to_string(), t, format!("Entity{i}"));
            e.id = format!("e{i}");
            e.name_embedding = Some(make_embedding(i));
            kg_store_upsert(&kg, "root", e).await;
        }
        let _ = types;
        (kg,)
    };
    drop(kg_store);

    // Cold boot: reopen the provider and measure time to first successful
    // trait query.
    let start = Instant::now();
    let (kg, _episodes) = common::engram_stores::kg_and_episode_stores(&tmp);
    let count = kg.count_all_entities().await.expect("cold-boot query");
    assert_eq!(count, 10_000);
    let elapsed = start.elapsed();

    eprintln!("Cold-boot @ 10k entities: {elapsed:?}");
    assert!(
        elapsed.as_secs() < 10,
        "cold boot must be under 10s, got {elapsed:?}"
    );
}
