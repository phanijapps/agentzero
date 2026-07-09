use std::sync::Arc;
use tempfile::tempdir;

use gateway_services::VaultPaths;
use zbot_stores_sqlite::vector_index::{SqliteVecIndex, VectorIndex};
use zbot_stores_sqlite::{KnowledgeDatabase, WardWikiRepository};

fn setup() -> (
    tempfile::TempDir,
    Arc<KnowledgeDatabase>,
    WardWikiRepository,
) {
    let tmp = tempdir().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    std::fs::create_dir_all(paths.conversations_db().parent().unwrap()).unwrap();
    let db = Arc::new(KnowledgeDatabase::new(paths).unwrap());
    let vec: Arc<dyn VectorIndex> = Arc::new(
        SqliteVecIndex::new(db.clone(), "wiki_articles_index", "article_id").expect("init vec"),
    );
    let repo = WardWikiRepository::new(db.clone(), vec);
    (tmp, db, repo)
}

fn normalized(v: Vec<f32>) -> Vec<f32> {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / n).collect()
}

#[test]
fn hybrid_matches_fts_when_keyword_present() {
    let (_tmp, db, repo) = setup();
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO ward_wiki_articles (id, ward_id, agent_id, title, content, tags, source_fact_ids, version, created_at, updated_at) \
             VALUES ('w1','wardA','root','AISStream Endpoint','wss://stream.aisstream.io/v0/stream returns 404 on v2.','[]','[]',1,'2026-04-15','2026-04-15')",
            [],
        )?;
        Ok(())
    }).unwrap();

    let hits = repo
        .search_hybrid("AISStream", Some("wardA"), None, 5)
        .unwrap();
    assert!(hits.iter().any(|h| h.article.id == "w1"));
}

#[test]
fn hybrid_matches_vector_when_query_and_title_are_semantically_similar() {
    let (_tmp, db, repo) = setup();
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO ward_wiki_articles (id, ward_id, agent_id, title, content, tags, source_fact_ids, version, created_at, updated_at) \
             VALUES ('w2','wardA','root','Hormuz Bounding Box','24 N to 27 N, 54 E to 58 E.','[]','[]',1,'2026-04-15','2026-04-15')",
            [],
        )?;
        Ok(())
    }).unwrap();

    let dim = repo.vec_index_for_tests().dim();
    let emb = normalized((0..dim).map(|i| (i as f32).sin()).collect());
    repo.vec_index_for_tests().upsert("w2", &emb).unwrap();

    let hits = repo
        .search_hybrid("strait monitoring region", Some("wardA"), Some(emb), 5)
        .unwrap();
    assert!(hits.iter().any(|h| h.article.id == "w2"));
}
