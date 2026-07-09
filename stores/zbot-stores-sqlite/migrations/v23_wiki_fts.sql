-- stores/zbot-stores-sqlite/migrations/v23_wiki_fts.sql
CREATE VIRTUAL TABLE IF NOT EXISTS ward_wiki_articles_fts USING fts5(
    title,
    content,
    content='ward_wiki_articles',
    content_rowid='rowid'
);

-- Backfill from existing rows.
INSERT INTO ward_wiki_articles_fts(rowid, title, content)
SELECT rowid, title, content FROM ward_wiki_articles
WHERE rowid NOT IN (SELECT rowid FROM ward_wiki_articles_fts);

-- Keep FTS in sync with source table.
CREATE TRIGGER IF NOT EXISTS ward_wiki_articles_fts_ai AFTER INSERT ON ward_wiki_articles BEGIN
    INSERT INTO ward_wiki_articles_fts(rowid, title, content)
    VALUES (new.rowid, new.title, new.content);
END;

CREATE TRIGGER IF NOT EXISTS ward_wiki_articles_fts_ad AFTER DELETE ON ward_wiki_articles BEGIN
    INSERT INTO ward_wiki_articles_fts(ward_wiki_articles_fts, rowid, title, content)
    VALUES ('delete', old.rowid, old.title, old.content);
END;

CREATE TRIGGER IF NOT EXISTS ward_wiki_articles_fts_au AFTER UPDATE ON ward_wiki_articles BEGIN
    INSERT INTO ward_wiki_articles_fts(ward_wiki_articles_fts, rowid, title, content)
    VALUES ('delete', old.rowid, old.title, old.content);
    INSERT INTO ward_wiki_articles_fts(rowid, title, content)
    VALUES (new.rowid, new.title, new.content);
END;
