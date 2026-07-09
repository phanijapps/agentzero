//! Fresh-boot integration test: both DBs come up, sqlite-vec is loaded,
//! vec0 tables work end-to-end.

use std::sync::Arc;
use tempfile::tempdir;

use gateway_services::VaultPaths;
use zbot_stores_sqlite::{DatabaseManager, KnowledgeDatabase};

#[test]
fn fresh_boot_creates_both_databases_with_vec0_working() {
    let tmp = tempdir().expect("tempdir");
    let paths: Arc<VaultPaths> = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));

    // KnowledgeDatabase::new creates the data dir itself, but DatabaseManager
    // does not always — create it defensively so both inits can find their parent.
    let data_dir = paths
        .conversations_db()
        .parent()
        .expect("conversations_db has parent")
        .to_path_buf();
    std::fs::create_dir_all(&data_dir).expect("mkdir data dir");

    // Boot both databases.
    let _conversations = DatabaseManager::new(paths.clone()).expect("conversations db initializes");
    let knowledge = KnowledgeDatabase::new(paths.clone()).expect("knowledge db initializes");

    // Both database files must exist on disk after initialization.
    assert!(
        paths.conversations_db().exists(),
        "conversations.db missing after init"
    );
    assert!(
        paths.knowledge_db().exists(),
        "knowledge.db missing after init"
    );

    // vec0 table is queryable and initially empty.
    knowledge
        .with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM kg_name_index", [], |r| r.get(0))?;
            assert_eq!(count, 0, "kg_name_index should be empty on fresh boot");
            Ok(())
        })
        .expect("query kg_name_index on fresh boot");

    // End-to-end: insert entity, insert vec row, verify count, delete entity,
    // verify trigger cascades the vec row deletion.
    knowledge
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO kg_entities(
                    id, agent_id, entity_type, name, normalized_name, normalized_hash,
                    first_seen_at, last_seen_at
                ) VALUES ('e1', 'root', 'person', 'Alice', 'alice', 'h1',
                          datetime('now'), datetime('now'))",
                [],
            )?;

            let embedding_json = serde_json::to_string(&vec![0.1_f32; 384]).unwrap();
            conn.execute(
                "INSERT INTO kg_name_index(entity_id, name_embedding) VALUES ('e1', ?1)",
                rusqlite::params![embedding_json],
            )?;

            let count_after_insert: i64 =
                conn.query_row("SELECT COUNT(*) FROM kg_name_index", [], |r| r.get(0))?;
            assert_eq!(count_after_insert, 1, "one vec row after insert");

            // Delete the entity — the DELETE trigger must cascade to kg_name_index.
            conn.execute("DELETE FROM kg_entities WHERE id = 'e1'", [])?;

            let count_after_delete: i64 =
                conn.query_row("SELECT COUNT(*) FROM kg_name_index", [], |r| r.get(0))?;
            assert_eq!(
                count_after_delete, 0,
                "DELETE trigger must cascade to kg_name_index"
            );
            Ok(())
        })
        .expect("end-to-end insert/query/delete on knowledge db");
}
