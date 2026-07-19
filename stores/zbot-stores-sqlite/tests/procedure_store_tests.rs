//! Integration tests for `GatewayProcedureStore`.

use std::sync::Arc;

use gateway_services::paths::VaultPaths;
use tempfile::TempDir;
use zbot_stores_sqlite::{
    GatewayProcedureStore, KnowledgeDatabase, ProcedureRepository, SqliteVecIndex, VectorIndex,
};
use zbot_stores_traits::{PatternProcedureInsert, ProcedureStore};

/// Build a `GatewayProcedureStore` backed by an on-disk temp SQLite database.
/// The TempDir must outlive the store, so it is returned alongside.
fn test_procedure_store() -> (TempDir, GatewayProcedureStore) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
    let vec_index: Arc<dyn VectorIndex> = Arc::new(
        SqliteVecIndex::new(db.clone(), "procedures_index", "procedure_id")
            .expect("vec index init"),
    );
    let repo = Arc::new(ProcedureRepository::new(db, vec_index));
    let store = GatewayProcedureStore::new(repo);
    (tmp, store)
}

/// L2-normalize a vector. The `procedures_index` vec0 table expects
/// normalized 384-dim vectors when callers want meaningful similarity.
fn normalized(v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-9 {
        v
    } else {
        v.into_iter().map(|x| x / norm).collect()
    }
}

#[tokio::test]
async fn insert_pattern_procedure_persists_embedding() {
    let (_tmp, store) = test_procedure_store();

    // procedures_index is a 384-dim vec0 table; use a normalized 384-dim
    // vector so the upsert succeeds and similarity search returns 1.0.
    let mut emb = vec![0.0_f32; 384];
    emb[0] = 0.1;
    emb[1] = 0.2;
    emb[2] = 0.3;
    let emb = normalized(emb);

    let req = PatternProcedureInsert {
        agent_id: "root".into(),
        ward_id: Some("__global__".into()),
        name: "test_proc".into(),
        description: "test".into(),
        trigger_pattern: None,
        steps_json: "[]".into(),
        parameters_json: None,
        embedding: Some(emb.clone()),
        success_count: 2,
    };
    let id = store.insert_pattern_procedure(req).await.unwrap();

    let results = store
        .search_procedures_by_similarity(&emb, "root", None, 5)
        .await
        .unwrap();
    assert!(
        results.iter().any(|r| r["procedure"]["id"] == id),
        "expected inserted procedure {id} to be returned by similarity search; got {results:?}",
    );
}

#[tokio::test]
async fn get_procedure_by_name_returns_full_row() {
    let (_tmp, store) = test_procedure_store();
    let req = PatternProcedureInsert {
        agent_id: "root".into(),
        ward_id: Some("__global__".into()),
        name: "find_me".into(),
        description: "a procedure".into(),
        trigger_pattern: None,
        steps_json: r#"[{"action":"shell","args":{"cmd":"ls"},"binds":[]}]"#.into(),
        parameters_json: Some(r#"["dir"]"#.into()),
        embedding: None,
        success_count: 2,
    };
    store.insert_pattern_procedure(req).await.unwrap();
    let found = store
        .get_procedure_by_name("root", "find_me")
        .await
        .unwrap();
    let proc = found.expect("not found");
    assert_eq!(proc.name, "find_me");
    assert_eq!(proc.description, "a procedure");
    assert_eq!(proc.ward_id.as_deref(), Some("__global__"));
    assert!(proc.steps.contains("shell"));
    assert_eq!(proc.parameters.as_deref(), Some(r#"["dir"]"#));
}

#[tokio::test]
async fn get_procedure_by_name_returns_none_when_missing() {
    let (_tmp, store) = test_procedure_store();
    let found = store
        .get_procedure_by_name("root", "nonexistent")
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn insert_pattern_procedure_records_evidence_count() {
    let (_tmp, store) = test_procedure_store();
    // Mining produced 4 matching successful sessions — the writer should be
    // able to record that, not hardcode 1.
    let req = PatternProcedureInsert {
        agent_id: "root".into(),
        ward_id: Some("__global__".into()),
        name: "well_evidenced".into(),
        description: "Procedure mined from 4 sessions".into(),
        trigger_pattern: None,
        steps_json: r#"[{"action":"shell","args":{},"binds":[]}]"#.into(),
        parameters_json: None,
        embedding: None,
        success_count: 4,
    };
    store.insert_pattern_procedure(req).await.unwrap();
    let proc = store
        .get_procedure_by_name("root", "well_evidenced")
        .await
        .unwrap()
        .expect("not found");
    assert_eq!(proc.success_count, 4);
}

#[tokio::test]
async fn dedupe_procedures_by_name_keeps_highest_sc_per_name() {
    let (_tmp, store) = test_procedure_store();

    // Insert 4 rows total under 2 names:
    //   - "alpha"  3 copies (sc=1, sc=3, sc=2) → keep the sc=3 row
    //   - "beta"   1 copy   (sc=5)             → unchanged
    let rows = [
        ("proc-a1", "alpha", 1),
        ("proc-a2", "alpha", 3),
        ("proc-a3", "alpha", 2),
        ("proc-b1", "beta", 5),
    ];
    for (id, name, sc) in rows {
        let proc = zbot_stores_sqlite::Procedure {
            id: id.into(),
            agent_id: "root".into(),
            ward_id: Some("__global__".into()),
            name: name.into(),
            description: format!("desc {id}"),
            trigger_pattern: None,
            steps: "[]".into(),
            parameters: None,
            success_count: sc,
            failure_count: 0,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        let v = serde_json::to_value(&proc).unwrap();
        store.upsert_procedure(v, None).await.unwrap();
    }

    let deleted = store.dedupe_procedures_by_name().await.unwrap();
    assert_eq!(deleted, 2, "should delete the 2 lower-sc 'alpha' rows");

    // The kept alpha row is proc-a2 (sc=3).
    let alpha = store
        .get_procedure_by_name("root", "alpha")
        .await
        .unwrap()
        .expect("alpha survived");
    assert_eq!(alpha.id, "proc-a2");
    assert_eq!(alpha.success_count, 3);

    // beta unchanged.
    let beta = store
        .get_procedure_by_name("root", "beta")
        .await
        .unwrap()
        .expect("beta survived");
    assert_eq!(beta.id, "proc-b1");
    assert_eq!(beta.success_count, 5);

    // Idempotent — second call deletes nothing.
    let deleted2 = store.dedupe_procedures_by_name().await.unwrap();
    assert_eq!(deleted2, 0);
}

#[tokio::test]
async fn dedupe_procedures_by_name_noop_when_no_duplicates() {
    let (_tmp, store) = test_procedure_store();
    let proc = zbot_stores_sqlite::Procedure {
        id: "proc-solo".into(),
        agent_id: "root".into(),
        ward_id: Some("__global__".into()),
        name: "solo".into(),
        description: "only one".into(),
        trigger_pattern: None,
        steps: "[]".into(),
        parameters: None,
        success_count: 1,
        failure_count: 0,
        avg_duration_ms: None,
        avg_token_cost: None,
        last_used: None,
        embedding: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
    };
    let v = serde_json::to_value(&proc).unwrap();
    store.upsert_procedure(v, None).await.unwrap();
    let deleted = store.dedupe_procedures_by_name().await.unwrap();
    assert_eq!(deleted, 0);
    assert!(store
        .get_procedure_by_name("root", "solo")
        .await
        .unwrap()
        .is_some());
}

#[test]
fn pattern_procedure_insert_defaults_success_count_for_back_compat() {
    // Older serialized payloads omit `success_count` — they should default to
    // 2 (above the middleware's legacy-advisory floor) so existing callers
    // don't silently land below visibility.
    let json = r#"{
        "agent_id": "root",
        "ward_id": "__global__",
        "name": "legacy",
        "description": "no success_count in payload",
        "trigger_pattern": null,
        "steps_json": "[]",
        "parameters_json": null
    }"#;
    let parsed: PatternProcedureInsert = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.success_count, 2);
    assert_eq!(parsed.embedding, None);
}
