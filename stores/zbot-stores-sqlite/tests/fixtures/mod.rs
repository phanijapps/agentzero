use gateway_services::paths::VaultPaths;
use std::sync::Arc;
use tempfile::TempDir;
use zbot_stores_sqlite::kg::storage::GraphStorage;
use zbot_stores_sqlite::knowledge_db::KnowledgeDatabase;
use zbot_stores_sqlite::SqliteKgStore;

pub async fn sqlite_store() -> (TempDir, SqliteKgStore) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    let kdb = Arc::new(KnowledgeDatabase::new(paths).expect("kdb"));
    let storage = Arc::new(GraphStorage::new(kdb).expect("storage"));
    let store = SqliteKgStore::new(storage);
    (tmp, store)
}
