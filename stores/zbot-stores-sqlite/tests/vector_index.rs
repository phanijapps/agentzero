//! VectorIndex integration tests against a real knowledge.db + sqlite-vec.

use std::sync::Arc;
use tempfile::tempdir;

use gateway_services::VaultPaths;
use zbot_stores_sqlite::vector_index::{SqliteVecIndex, VectorIndex};
use zbot_stores_sqlite::KnowledgeDatabase;

fn setup() -> (tempfile::TempDir, Arc<KnowledgeDatabase>) {
    let tmp = tempdir().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
    let db = Arc::new(KnowledgeDatabase::new(paths.clone()).expect("init knowledge db"));
    (tmp, db)
}

fn normalized(v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-9 {
        v
    } else {
        v.into_iter().map(|x| x / norm).collect()
    }
}

#[test]
fn upsert_and_query_nearest_returns_self() {
    let (_tmp, db) = setup();
    let idx =
        SqliteVecIndex::new(db.clone(), "kg_name_index", "entity_id").expect("vec index init");

    let v = normalized((0..384).map(|i| i as f32).collect());
    idx.upsert("e1", &v).expect("upsert");

    let results = idx.query_nearest(&v, 1).expect("query");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "e1");
    assert!(
        results[0].1 < 1e-3,
        "nearest self distance should be ~0, got {}",
        results[0].1
    );
}

#[test]
fn delete_removes_entry() {
    let (_tmp, db) = setup();
    let idx =
        SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id").expect("vec index init");

    let v = normalized((0..384).map(|i| i as f32).collect());
    idx.upsert("f1", &v).expect("upsert");
    idx.delete("f1").expect("delete");

    let results = idx.query_nearest(&v, 5).expect("query");
    assert!(results.iter().all(|(id, _)| id != "f1"));
}

#[test]
fn upsert_same_id_replaces_not_duplicates() {
    let (_tmp, db) = setup();
    let idx = SqliteVecIndex::new(db.clone(), "procedures_index", "procedure_id")
        .expect("vec index init");

    let v1 = normalized(vec![1.0; 384]);
    let v2 = normalized((0..384).map(|i| i as f32).collect());
    idx.upsert("p1", &v1).expect("first");
    idx.upsert("p1", &v2).expect("replace");

    let count_total = db
        .with_connection(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM procedures_index WHERE procedure_id = ?1",
                rusqlite::params!["p1"],
                |r| r.get(0),
            )?;
            Ok(n)
        })
        .expect("count");
    assert_eq!(count_total, 1, "upsert must replace, not duplicate");
}

#[test]
fn dim_mismatch_errors() {
    let (_tmp, db) = setup();
    let idx = SqliteVecIndex::new(db.clone(), "wiki_articles_index", "article_id")
        .expect("vec index init");

    let wrong = vec![0.5_f32; 100];
    assert!(idx.upsert("w1", &wrong).is_err());
    assert!(idx.query_nearest(&wrong, 5).is_err());
}

#[test]
fn self_heals_when_table_is_recreated_at_new_dim() {
    // Simulates the reindex pipeline: after a backend switch, the vec0 table
    // is dropped + recreated at a different dim. The cached index must
    // pick up the new dim on the next query instead of returning a stale
    // "dim mismatch" error.
    let (_tmp, db) = setup();
    let idx =
        SqliteVecIndex::new(db.clone(), "memory_facts_index", "fact_id").expect("vec index init");
    assert_eq!(idx.dim(), 384);

    // Drop and recreate at a different dimension (mirrors the reindex path).
    db.with_connection(|conn| {
        conn.execute("DROP TABLE memory_facts_index", [])?;
        conn.execute(
            "CREATE VIRTUAL TABLE memory_facts_index USING vec0(fact_id TEXT PRIMARY KEY, embedding FLOAT[1024])",
            [],
        )?;
        Ok(())
    })
    .expect("recreate table");

    // A 1024-d upsert should now succeed (self-heal rereads the DDL).
    let v = normalized(vec![0.25_f32; 1024]);
    idx.upsert("m1", &v).expect("self-heal upsert");
    assert_eq!(idx.dim(), 1024, "dim must be refreshed from the new DDL");

    // A 384-d call against the refreshed index is now a real mismatch.
    let old = normalized(vec![0.25_f32; 384]);
    assert!(idx.upsert("m2", &old).is_err());
}
