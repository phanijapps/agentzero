use std::sync::Arc;
use tempfile::tempdir;

use gateway_services::VaultPaths;
use zbot_stores_sqlite::KnowledgeDatabase;

fn db() -> (tempfile::TempDir, Arc<KnowledgeDatabase>) {
    let tmp = tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
    let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
    (tmp, db)
}

#[test]
fn ward_wiki_articles_fts_exists_after_migration() {
    let (_tmp, db) = db();
    let exists: i64 = db
        .with_connection(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='ward_wiki_articles_fts'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(exists, 1);
}

#[test]
fn inserting_wiki_populates_fts_via_trigger() {
    let (_tmp, db) = db();
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO ward_wiki_articles (id, ward_id, agent_id, title, content, tags, source_fact_ids, version, created_at, updated_at) \
             VALUES ('w1','wardA','root','Hormuz Geofence','Latitude 24.0 to 27.5','[]','[]',1,'2026-04-15','2026-04-15')",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let hits: i64 = db
        .with_connection(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM ward_wiki_articles_fts WHERE ward_wiki_articles_fts MATCH 'Hormuz'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(hits, 1);
}
