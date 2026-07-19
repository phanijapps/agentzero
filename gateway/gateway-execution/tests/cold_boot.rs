//! Cold-boot baseline: how long does it take to initialize a knowledge.db
//! pointing at an already-populated 10k-entity vault?
//!
//! Phase 5 acceptance: first successful query returns in < 10s.

use std::sync::Arc;
use std::time::Instant;

use tempfile::tempdir;

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
fn cold_boot_under_10s_with_10k_entities() {
    let tmp = tempdir().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();

    // Seed: create the DB once, write 10k entities, drop.
    {
        let db = Arc::new(KnowledgeDatabase::new(paths.clone()).expect("seed db"));
        let storage = GraphStorage::new(db.clone()).expect("seed storage");
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
        // drop db + storage explicitly when this block ends
    }

    // Cold boot: reopen and measure time to first successful query.
    let start = Instant::now();
    let db = Arc::new(KnowledgeDatabase::new(paths.clone()).expect("cold boot db"));
    db.with_connection(|conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM kg_entities", [], |r| r.get(0))?;
        assert_eq!(n, 10_000);
        Ok(())
    })
    .expect("cold-boot query");
    let elapsed = start.elapsed();

    eprintln!("Cold-boot @ 10k entities: {elapsed:?}");
    assert!(
        elapsed.as_secs() < 10,
        "cold boot must be under 10s, got {elapsed:?}"
    );
}
