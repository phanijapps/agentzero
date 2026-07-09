//! Resolver p95 latency benchmark. Seeds 1000 entities, then measures
//! 100 fresh candidate resolutions. Fails if p95 ≥ 20 ms.
//!
//! Run with `--release` — debug builds are 5-10× slower and the budget
//! assumes release-mode SQLite + vec0.

use std::sync::Arc;
use std::time::Instant;

use gateway_services::VaultPaths;
use knowledge_graph::{Entity, EntityType, ExtractedKnowledge};
use zbot_stores_sqlite::kg::storage::GraphStorage;
use zbot_stores_sqlite::KnowledgeDatabase;

fn normalized(v: Vec<f32>) -> Vec<f32> {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n < 1e-9 {
        v
    } else {
        v.into_iter().map(|x| x / n).collect()
    }
}

/// Deterministic pseudo-random 384-d L2-normalized embedding from a seed.
/// Splitmix64-style so runs are reproducible for perf regression tracking.
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

#[test]
fn resolver_p95_under_20ms_at_1000_entities() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
    let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
    let storage = GraphStorage::new(db.clone()).expect("storage");

    let types = [
        EntityType::Person,
        EntityType::Organization,
        EntityType::Location,
        EntityType::Event,
        EntityType::Concept,
    ];

    // Seed 1000 entities with deterministic embeddings.
    for i in 0..1000u64 {
        let t = types[(i as usize) % types.len()].clone();
        let mut e = Entity::new("root".to_string(), t, format!("Entity{i}"));
        e.id = format!("e{i}");
        e.name_embedding = Some(make_embedding(i));
        storage
            .store_knowledge(
                "root",
                ExtractedKnowledge {
                    entities: vec![e],
                    relationships: vec![],
                },
            )
            .expect("seed store");
    }

    // Measure 100 fresh resolutions.
    let mut durations = Vec::with_capacity(100);
    for i in 1000..1100u64 {
        let t = types[(i as usize) % types.len()].clone();
        let mut e = Entity::new("root".to_string(), t, format!("Candidate{i}"));
        e.id = format!("cand{i}");
        e.name_embedding = Some(make_embedding(i + 7919));

        let start = Instant::now();
        storage
            .store_knowledge(
                "root",
                ExtractedKnowledge {
                    entities: vec![e],
                    relationships: vec![],
                },
            )
            .expect("resolve + store");
        durations.push(start.elapsed());
    }

    durations.sort();
    let p50 = durations[durations.len() / 2];
    let p95 = durations[(durations.len() * 95) / 100];
    let p99 = durations[durations.len() - 1];
    eprintln!("Resolver benchmark — p50={p50:?} p95={p95:?} p99={p99:?}");

    assert!(
        p95.as_millis() < 20,
        "resolver p95 must be < 20ms, got {p95:?}"
    );
}
