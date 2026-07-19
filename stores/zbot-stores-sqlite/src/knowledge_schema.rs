//! Schema v31 for `knowledge.db`.
//!
//! All long-term memory + graph + vector indexes live here.
//! Applied idempotently on daemon boot. No migrations — clean slate.

use rusqlite::Connection;

const SCHEMA_VERSION: i32 = 31;

/// v23 delta: full-text search over `ward_wiki_articles` with sync triggers.
const V23_WIKI_FTS_SQL: &str = include_str!("../migrations/v23_wiki_fts.sql");

/// v24 delta: one-time backfill that promotes facts in global-type categories
/// (domain / reference / book / research / user) from the default
/// `scope='agent'` to `scope='global'`, making them visible to every agent
/// via the scope-aware search filter.
const V24_GLOBAL_SCOPE_BACKFILL_SQL: &str =
    include_str!("../migrations/v24_global_scope_backfill.sql");

/// v25 delta: bi-temporal phase 1 — backfill `valid_from` on legacy
/// `memory_facts` rows so point-in-time queries can include pre-migration
/// facts in "now" results without special-casing NULL.
const V25_MEMORY_FACTS_VALID_FROM_BACKFILL_SQL: &str =
    include_str!("../migrations/v25_memory_facts_valid_from_backfill.sql");

/// v26 delta: bi-temporal phase 3 — align `kg_relationships` with the
/// symmetric `valid_from` / `valid_until` schema used by `memory_facts`
/// and `kg_entities`, backfilling from the legacy `valid_at` /
/// `invalidated_at` pair. The `ALTER TABLE ADD COLUMN` is handled by
/// `ensure_kg_relationships_bitemporal_columns` (PRAGMA-guarded for
/// idempotency); this SQL file performs the UPDATE backfill only.
const V26_KG_RELATIONSHIPS_BITEMPORAL_SQL: &str =
    include_str!("../migrations/v26_kg_relationships_bitemporal.sql");

/// v27 delta: add `kg_beliefs` table for the Belief Network (Phase B-1).
/// A belief is an aggregate over one or more memory_facts about a single
/// subject. Partition-scoped from day one using `partition_id` so the
/// future R-series rename of `ward_id` doesn't need to touch this table.
const V27_KG_BELIEFS_SQL: &str = include_str!("../migrations/v27_kg_beliefs.sql");

/// v28 delta: add `kg_belief_contradictions` table for the Belief Network
/// (Phase B-2). Stores pair-wise contradiction rows produced by the
/// `BeliefContradictionDetector`. `belief_a_id` is always the
/// lexicographically smaller of the two — canonical pair ordering keeps
/// `UNIQUE(belief_a_id, belief_b_id)` doing real work.
const V28_KG_BELIEF_CONTRADICTIONS_SQL: &str =
    include_str!("../migrations/v28_kg_belief_contradictions.sql");

/// v29 delta: add `stale INTEGER NOT NULL DEFAULT 0` column to
/// `kg_beliefs` to support B-3 confidence propagation. When a source
/// fact is invalidated and the belief has multiple sources, the belief
/// is marked stale (stale = 1); the next BeliefSynthesizer cycle picks
/// it up and re-synthesizes from the remaining sources. The migration
/// SQL only carries the partial index — the `ALTER TABLE ADD COLUMN`
/// is handled by `ensure_kg_beliefs_stale_column` (PRAGMA-guarded for
/// idempotency, same pattern as v26).
const V29_KG_BELIEFS_STALE_SQL: &str = include_str!("../migrations/v29_kg_beliefs_stale.sql");

/// v30 delta: add `embedding BLOB` column to `kg_beliefs` for B-4
/// recall integration. Belief recall uses in-memory cosine over a
/// bounded candidate set rather than a separate vec0 table — see the
/// migration SQL for the rationale. The `ALTER TABLE ADD COLUMN` is
/// handled by `ensure_kg_beliefs_embedding_column` (PRAGMA-guarded,
/// mirroring v29). The migration file itself is documentation-only
/// (no DDL beyond the inline schema and the guard helper).
const V30_KG_BELIEFS_EMBEDDING_SQL: &str =
    include_str!("../migrations/v30_kg_beliefs_embedding.sql");

/// v31 delta: hierarchical-memory columns on `kg_entities` and
/// `kg_relationships`. Lays the groundwork for HiRAG/LeanRAG-style
/// recursive aggregation of entities into higher-layer summary entities
/// plus explicit inter-cluster relations. The `ALTER TABLE ADD COLUMN`
/// operations are handled by the `ensure_*_hierarchy_columns` guards
/// below (PRAGMA-probe pattern, mirroring v29/v30). The migration SQL
/// file only carries the supporting indexes (`IF NOT EXISTS`). No
/// backfill — every existing row is a valid layer-0 base node/edge.
/// See `project_hierarchical_memory_plan.md` for the full design.
const V31_KG_HIERARCHY_COLUMNS_SQL: &str =
    include_str!("../migrations/v31_kg_hierarchy_columns.sql");

/// Initialize the knowledge database schema (v30).
///
/// Creates all tables and indexes if they don't exist. Records the
/// schema version in `schema_version` table. Safe to call on an
/// already-initialized DB — every delta is idempotent.
pub fn initialize_knowledge_database(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(SCHEMA_SQL)?;
    conn.execute_batch(V23_WIKI_FTS_SQL)?;
    conn.execute_batch(V24_GLOBAL_SCOPE_BACKFILL_SQL)?;
    conn.execute_batch(V25_MEMORY_FACTS_VALID_FROM_BACKFILL_SQL)?;
    add_skill_index_format_version_if_missing(conn)?;
    ensure_evidence_column(conn, "kg_entities")?;
    ensure_evidence_column(conn, "kg_relationships")?;
    ensure_kg_relationships_bitemporal_columns(conn)?;
    conn.execute_batch(V26_KG_RELATIONSHIPS_BITEMPORAL_SQL)?;
    conn.execute_batch(V27_KG_BELIEFS_SQL)?;
    conn.execute_batch(V28_KG_BELIEF_CONTRADICTIONS_SQL)?;
    ensure_kg_beliefs_stale_column(conn)?;
    conn.execute_batch(V29_KG_BELIEFS_STALE_SQL)?;
    ensure_kg_beliefs_embedding_column(conn)?;
    conn.execute_batch(V30_KG_BELIEFS_EMBEDDING_SQL)?;
    ensure_kg_entities_hierarchy_columns(conn)?;
    ensure_kg_relationships_hierarchy_columns(conn)?;
    conn.execute_batch(V31_KG_HIERARCHY_COLUMNS_SQL)?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
        rusqlite::params![SCHEMA_VERSION],
    )?;
    Ok(())
}

/// Add `format_version` to `skill_index_state` if the table predates it.
/// SQLite doesn't support `ADD COLUMN IF NOT EXISTS`, so we check the
/// table's column list first. The default `1` matches what existing
/// rows would have logically had, so the reindex diff naturally treats
/// them as stale (current code constant is `2`) and re-embeds once.
fn add_skill_index_format_version_if_missing(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(skill_index_state)")?;
    let column_exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == "format_version");
    if !column_exists {
        conn.execute_batch(
            "ALTER TABLE skill_index_state ADD COLUMN format_version INTEGER NOT NULL DEFAULT 1",
        )?;
    }
    Ok(())
}

/// Ensure the `evidence TEXT` column exists on the given KG table.
/// Idempotent — does nothing if the column already exists.
fn ensure_evidence_column(conn: &Connection, table: &str) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let has_evidence = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == "evidence");
    if !has_evidence {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN evidence TEXT"), [])?;
    }
    Ok(())
}

/// Ensure `kg_beliefs` carries the `stale` column introduced in schema
/// v29. SQLite errors on duplicate `ADD COLUMN`, so we PRAGMA-probe the
/// table before each ALTER. Fresh databases get the column via the
/// `CREATE TABLE` body and this function is a no-op. The companion
/// migration SQL file creates the partial index on `stale = 1`.
fn ensure_kg_beliefs_stale_column(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)")?;
    let has_stale = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == "stale");
    if !has_stale {
        conn.execute(
            "ALTER TABLE kg_beliefs ADD COLUMN stale INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

/// Ensure `kg_beliefs` carries the `embedding` BLOB column introduced
/// in schema v30. The column is nullable — beliefs synthesized without
/// an available embedding client store NULL and won't surface in
/// semantic recall (only via direct lookup). Fresh databases get the
/// column via the inline `CREATE TABLE` body; this function is the
/// idempotent ALTER guard for existing DBs.
fn ensure_kg_beliefs_embedding_column(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)")?;
    let has_embedding = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == "embedding");
    if !has_embedding {
        conn.execute("ALTER TABLE kg_beliefs ADD COLUMN embedding BLOB", [])?;
    }
    Ok(())
}

/// Ensure `kg_entities` carries the hierarchical-memory columns
/// `layer` and `parent_cluster_id` introduced in schema v31.
///
/// `layer` defaults to `0` (every base-extracted entity is layer 0).
/// `parent_cluster_id` is nullable and references `kg_entities.id` as a
/// soft foreign key (not enforced — SQLite-friendly + the parent may
/// itself be an aggregate entity in the same table). The future
/// HierarchyBuilder sleep worker sets these when it produces layer-N+1
/// aggregate entities from layer-N clusters.
///
/// Fresh databases get the columns via the inline `CREATE TABLE` body;
/// this function is the idempotent ALTER guard for existing DBs.
fn ensure_kg_entities_hierarchy_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(kg_entities)")?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();
    if !existing.iter().any(|name| name == "layer") {
        conn.execute(
            "ALTER TABLE kg_entities ADD COLUMN layer INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !existing.iter().any(|name| name == "parent_cluster_id") {
        conn.execute(
            "ALTER TABLE kg_entities ADD COLUMN parent_cluster_id TEXT",
            [],
        )?;
    }
    Ok(())
}

/// Ensure `kg_relationships` carries the hierarchical-memory columns
/// `layer` and `is_inter_cluster` introduced in schema v31.
///
/// `layer` defaults to `0` (every base-extracted edge is at layer 0,
/// matching its endpoints). `is_inter_cluster` defaults to `0` (false)
/// — only relations produced by the HierarchyBuilder between aggregate
/// entities at layer N+1 carry `is_inter_cluster = 1`. This is the
/// LeanRAG load-bearing distinction: same-cluster relations at higher
/// layers are derivative; inter-cluster relations are the new
/// information the LLM synthesised when the connectivity strength λ
/// between two clusters crossed the threshold τ.
///
/// Fresh databases get the columns via the inline `CREATE TABLE` body;
/// this function is the idempotent ALTER guard for existing DBs.
fn ensure_kg_relationships_hierarchy_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(kg_relationships)")?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();
    if !existing.iter().any(|name| name == "layer") {
        conn.execute(
            "ALTER TABLE kg_relationships ADD COLUMN layer INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !existing.iter().any(|name| name == "is_inter_cluster") {
        conn.execute(
            "ALTER TABLE kg_relationships ADD COLUMN is_inter_cluster INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

/// Ensure `kg_relationships` carries the bi-temporal `valid_from` and
/// `valid_until` columns introduced in schema v26.
///
/// SQLite errors on duplicate `ADD COLUMN`, so we PRAGMA-probe the table
/// before each ALTER. Fresh databases get the columns via the
/// `CREATE TABLE` body and this function is a no-op. The companion
/// migration SQL file backfills the new columns from the legacy
/// `valid_at` / `invalidated_at` pair after this function runs.
fn ensure_kg_relationships_bitemporal_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(kg_relationships)")?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();
    if !existing.iter().any(|name| name == "valid_from") {
        conn.execute(
            "ALTER TABLE kg_relationships ADD COLUMN valid_from TEXT",
            [],
        )?;
    }
    if !existing.iter().any(|name| name == "valid_until") {
        conn.execute(
            "ALTER TABLE kg_relationships ADD COLUMN valid_until TEXT",
            [],
        )?;
    }
    Ok(())
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

-- ========================================================================
-- Knowledge Graph — entities, relationships, aliases
-- ========================================================================

CREATE TABLE IF NOT EXISTS kg_entities (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_hash TEXT NOT NULL,
    properties TEXT,
    epistemic_class TEXT NOT NULL DEFAULT 'current',
    confidence REAL NOT NULL DEFAULT 0.8,
    mention_count INTEGER NOT NULL DEFAULT 1,
    access_count INTEGER NOT NULL DEFAULT 0,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    last_accessed_at TEXT,
    valid_from TEXT,
    valid_until TEXT,
    invalidated_by TEXT,
    compressed_into TEXT,
    source_episode_ids TEXT,
    evidence TEXT,
    -- v31: hierarchical memory. layer=0 for base-extracted entities;
    -- HierarchyBuilder sets layer>0 + parent_cluster_id for aggregates.
    layer INTEGER NOT NULL DEFAULT 0,
    parent_cluster_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_entities_normalized_hash
    ON kg_entities(agent_id, entity_type, normalized_hash);
CREATE INDEX IF NOT EXISTS idx_entities_agent_type
    ON kg_entities(agent_id, entity_type);
CREATE INDEX IF NOT EXISTS idx_entities_name ON kg_entities(name);
CREATE INDEX IF NOT EXISTS idx_entities_last_accessed ON kg_entities(last_accessed_at);
CREATE INDEX IF NOT EXISTS idx_entities_epistemic
    ON kg_entities(agent_id, epistemic_class);

CREATE TABLE IF NOT EXISTS kg_relationships (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    source_entity_id TEXT NOT NULL,
    target_entity_id TEXT NOT NULL,
    relationship_type TEXT NOT NULL,
    properties TEXT,
    epistemic_class TEXT NOT NULL DEFAULT 'current',
    confidence REAL NOT NULL DEFAULT 0.8,
    mention_count INTEGER NOT NULL DEFAULT 1,
    access_count INTEGER NOT NULL DEFAULT 0,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    last_accessed_at TEXT,
    valid_at TEXT,
    invalidated_at TEXT,
    valid_from TEXT,
    valid_until TEXT,
    invalidated_by TEXT,
    source_episode_ids TEXT,
    evidence TEXT,
    -- v31: hierarchical memory. layer=0 for base-extracted edges;
    -- is_inter_cluster=1 marks edges synthesised between aggregate
    -- entities at higher layers (LeanRAG inter-cluster relations).
    layer INTEGER NOT NULL DEFAULT 0,
    is_inter_cluster INTEGER NOT NULL DEFAULT 0,
    UNIQUE(source_entity_id, target_entity_id, relationship_type),
    FOREIGN KEY (source_entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE,
    FOREIGN KEY (target_entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_rels_source ON kg_relationships(source_entity_id);
CREATE INDEX IF NOT EXISTS idx_rels_target ON kg_relationships(target_entity_id);
CREATE INDEX IF NOT EXISTS idx_rels_agent ON kg_relationships(agent_id);
CREATE INDEX IF NOT EXISTS idx_rels_valid ON kg_relationships(valid_at);

CREATE TABLE IF NOT EXISTS kg_aliases (
    id TEXT PRIMARY KEY,
    entity_id TEXT NOT NULL,
    surface_form TEXT NOT NULL,
    normalized_form TEXT NOT NULL,
    source TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 1.0,
    first_seen_at TEXT NOT NULL,
    FOREIGN KEY (entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE,
    UNIQUE(normalized_form, entity_id)
);
CREATE INDEX IF NOT EXISTS idx_aliases_normalized ON kg_aliases(normalized_form);
CREATE INDEX IF NOT EXISTS idx_aliases_entity ON kg_aliases(entity_id);

CREATE TABLE IF NOT EXISTS kg_episodes (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    session_id TEXT,
    agent_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    retry_count INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    UNIQUE(content_hash, source_type)
);
CREATE INDEX IF NOT EXISTS idx_episodes_status ON kg_episodes(status);
CREATE INDEX IF NOT EXISTS idx_episodes_source_ref ON kg_episodes(source_ref);
CREATE INDEX IF NOT EXISTS idx_episodes_session ON kg_episodes(session_id);

CREATE TABLE IF NOT EXISTS kg_goals (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    ward_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    state TEXT NOT NULL DEFAULT 'active',
    parent_goal_id TEXT,
    slots TEXT,
    filled_slots TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (parent_goal_id) REFERENCES kg_goals(id)
);
CREATE INDEX IF NOT EXISTS idx_goals_agent_state ON kg_goals(agent_id, state);
CREATE INDEX IF NOT EXISTS idx_goals_ward ON kg_goals(ward_id);

CREATE TABLE IF NOT EXISTS kg_compactions (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    entity_id TEXT,
    relationship_id TEXT,
    merged_into TEXT,
    reason TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_compactions_run ON kg_compactions(run_id);

CREATE TABLE IF NOT EXISTS kg_causal_edges (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    cause_entity_id TEXT NOT NULL,
    effect_entity_id TEXT NOT NULL,
    relationship TEXT NOT NULL,
    confidence REAL DEFAULT 0.7,
    session_id TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (cause_entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE,
    FOREIGN KEY (effect_entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_causal_cause ON kg_causal_edges(cause_entity_id);
CREATE INDEX IF NOT EXISTS idx_causal_effect ON kg_causal_edges(effect_entity_id);

-- ========================================================================
-- Memory facts — no embedding column; embeddings live in memory_facts_index
-- ========================================================================

CREATE TABLE IF NOT EXISTS memory_facts (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    agent_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    category TEXT NOT NULL,
    key TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.8,
    mention_count INTEGER NOT NULL DEFAULT 1,
    source_summary TEXT,
    source_episode_id TEXT,
    source_ref TEXT,
    ward_id TEXT NOT NULL DEFAULT '__global__',
    epistemic_class TEXT NOT NULL DEFAULT 'current',
    contradicted_by TEXT,
    valid_from TEXT,
    valid_until TEXT,
    superseded_by TEXT,
    pinned INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    expires_at TEXT,
    UNIQUE(agent_id, scope, ward_id, key)
);
CREATE INDEX IF NOT EXISTS idx_facts_agent_scope ON memory_facts(agent_id, scope);
CREATE INDEX IF NOT EXISTS idx_facts_category ON memory_facts(agent_id, category);
CREATE INDEX IF NOT EXISTS idx_facts_ward ON memory_facts(ward_id);
CREATE INDEX IF NOT EXISTS idx_facts_epistemic ON memory_facts(epistemic_class);
CREATE VIRTUAL TABLE IF NOT EXISTS memory_facts_fts USING fts5(
    key, content, category, content=memory_facts
);

CREATE TABLE IF NOT EXISTS memory_facts_archive (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    agent_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    category TEXT NOT NULL,
    key TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL NOT NULL,
    ward_id TEXT NOT NULL,
    epistemic_class TEXT NOT NULL,
    archived_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_facts_archive_agent ON memory_facts_archive(agent_id);

CREATE TABLE IF NOT EXISTS ward_wiki_articles (
    id TEXT PRIMARY KEY,
    ward_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT,
    source_fact_ids TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(ward_id, title)
);
CREATE INDEX IF NOT EXISTS idx_wiki_ward ON ward_wiki_articles(ward_id);

CREATE TABLE IF NOT EXISTS procedures (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    ward_id TEXT NOT NULL DEFAULT '__global__',
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    trigger_pattern TEXT,
    steps TEXT NOT NULL,
    parameters TEXT,
    success_count INTEGER NOT NULL DEFAULT 1,
    failure_count INTEGER NOT NULL DEFAULT 0,
    avg_duration_ms INTEGER,
    avg_token_cost INTEGER,
    last_used TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_procedures_agent ON procedures(agent_id);
CREATE INDEX IF NOT EXISTS idx_procedures_ward ON procedures(ward_id);

CREATE TABLE IF NOT EXISTS session_episodes (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL UNIQUE,
    agent_id TEXT NOT NULL,
    ward_id TEXT,
    task_summary TEXT,
    outcome TEXT,
    strategy_used TEXT,
    key_learnings TEXT,
    token_cost INTEGER,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_episodes_agent ON session_episodes(agent_id);
CREATE INDEX IF NOT EXISTS idx_session_episodes_ward ON session_episodes(ward_id);
CREATE INDEX IF NOT EXISTS idx_session_episodes_outcome ON session_episodes(outcome);

CREATE TABLE IF NOT EXISTS embedding_cache (
    content_hash TEXT NOT NULL,
    model TEXT NOT NULL,
    embedding BLOB NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (content_hash, model)
);

-- Per-skill staleness tracker for the incremental reindex. One row per
-- visible skill (after vault-wins dedup). The reindexer compares this
-- table against the on-disk skill set at session start: new rows get
-- embedded, missing rows get deleted, mismatched (mtime, size, format)
-- tuples get re-embedded. When the DB is wiped, the table is empty →
-- every on-disk skill is treated as new and reindexed cleanly.
--
-- `format_version` records the embedding-content schema. Code bumps the
-- in-process constant when it changes how it builds the indexed text;
-- rows with an older version are forced to re-embed once.
CREATE TABLE IF NOT EXISTS skill_index_state (
    name              TEXT PRIMARY KEY,
    source_root       TEXT NOT NULL,        -- 'vault' | 'agent'
    file_path         TEXT NOT NULL,
    mtime_unix        INTEGER NOT NULL,
    size_bytes        INTEGER NOT NULL,
    last_indexed_unix INTEGER NOT NULL,
    format_version    INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS kg_episode_payloads (
    episode_id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (episode_id) REFERENCES kg_episodes(id) ON DELETE CASCADE
);

-- v27: Belief Network — aggregate of one or more facts about a subject.
-- See migrations/v27_kg_beliefs.sql for the canonical definition; this
-- inline copy keeps fresh-DB init self-contained.
--
-- v29: `stale` column added so a multi-source belief whose source fact
-- was invalidated can be queued for re-synthesis on the next sleep cycle.
--
-- v30: `embedding` BLOB column added for B-4 recall integration. Stores
-- a serialized f32 vector (little-endian bytes, same format as other
-- caches in this crate) so the in-memory cosine search in
-- `SqliteBeliefStore::search_beliefs` can match beliefs against query
-- embeddings. NULL means "no embedding generated" — belief is still
-- usable via direct lookup but won't surface in semantic recall.
CREATE TABLE IF NOT EXISTS kg_beliefs (
    id TEXT PRIMARY KEY,
    partition_id TEXT NOT NULL,
    subject TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL NOT NULL,
    valid_from TEXT,
    valid_until TEXT,
    source_fact_ids TEXT NOT NULL,
    synthesizer_version INTEGER NOT NULL DEFAULT 1,
    reasoning TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    superseded_by TEXT,
    stale INTEGER NOT NULL DEFAULT 0,
    embedding BLOB,
    UNIQUE(partition_id, subject, valid_from)
);
CREATE INDEX IF NOT EXISTS idx_beliefs_partition_subject ON kg_beliefs(partition_id, subject);
CREATE INDEX IF NOT EXISTS idx_beliefs_valid ON kg_beliefs(valid_from, valid_until);
-- NOTE: idx_beliefs_stale is intentionally NOT defined here.
-- The `stale` column was added by v29 via `ensure_kg_beliefs_stale_column`,
-- which runs AFTER this inline schema. On databases created before v29,
-- the column doesn't exist when this batch executes — creating the
-- partial index here would fail with "no such column: stale".
-- The index is created in `V29_KG_BELIEFS_STALE_SQL` (migration file),
-- which runs after the ensure-column helper. For fresh DBs the column
-- exists (declared above) but the index still waits for the v29 step,
-- which is idempotent via IF NOT EXISTS.

-- v28: Belief Network — pair-wise contradictions between two beliefs.
-- See migrations/v28_kg_belief_contradictions.sql for the canonical
-- definition; this inline copy keeps fresh-DB init self-contained.
CREATE TABLE IF NOT EXISTS kg_belief_contradictions (
    id TEXT PRIMARY KEY,
    belief_a_id TEXT NOT NULL,
    belief_b_id TEXT NOT NULL,
    contradiction_type TEXT NOT NULL,
    severity REAL NOT NULL,
    judge_reasoning TEXT,
    detected_at TEXT NOT NULL,
    resolved_at TEXT,
    resolution TEXT,
    FOREIGN KEY (belief_a_id) REFERENCES kg_beliefs(id) ON DELETE CASCADE,
    FOREIGN KEY (belief_b_id) REFERENCES kg_beliefs(id) ON DELETE CASCADE,
    UNIQUE(belief_a_id, belief_b_id)
);
CREATE INDEX IF NOT EXISTS idx_belief_contradictions_a ON kg_belief_contradictions(belief_a_id);
CREATE INDEX IF NOT EXISTS idx_belief_contradictions_b ON kg_belief_contradictions(belief_b_id);
CREATE INDEX IF NOT EXISTS idx_belief_contradictions_unresolved ON kg_belief_contradictions(detected_at) WHERE resolved_at IS NULL;
"#;

#[allow(dead_code)] // retained for reference/tests; runtime uses initialize_vec_tables_with_dim
const VEC0_SQL: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS kg_name_index USING vec0(
    entity_id TEXT PRIMARY KEY,
    name_embedding FLOAT[384]
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_facts_index USING vec0(
    fact_id TEXT PRIMARY KEY,
    embedding FLOAT[384]
);

CREATE VIRTUAL TABLE IF NOT EXISTS wiki_articles_index USING vec0(
    article_id TEXT PRIMARY KEY,
    embedding FLOAT[384]
);

CREATE VIRTUAL TABLE IF NOT EXISTS procedures_index USING vec0(
    procedure_id TEXT PRIMARY KEY,
    embedding FLOAT[384]
);

CREATE VIRTUAL TABLE IF NOT EXISTS session_episodes_index USING vec0(
    episode_id TEXT PRIMARY KEY,
    embedding FLOAT[384]
);
"#;

const TRIGGERS_SQL: &str = r#"
-- Clean up vec0 partner rows when base rows are deleted.

CREATE TRIGGER IF NOT EXISTS trg_entities_delete_vec
AFTER DELETE ON kg_entities
BEGIN
    DELETE FROM kg_name_index WHERE entity_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_facts_delete_vec
AFTER DELETE ON memory_facts
BEGIN
    DELETE FROM memory_facts_index WHERE fact_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_wiki_delete_vec
AFTER DELETE ON ward_wiki_articles
BEGIN
    DELETE FROM wiki_articles_index WHERE article_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_procedures_delete_vec
AFTER DELETE ON procedures
BEGIN
    DELETE FROM procedures_index WHERE procedure_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_episodes_delete_vec
AFTER DELETE ON session_episodes
BEGIN
    DELETE FROM session_episodes_index WHERE episode_id = OLD.id;
END;

-- Keep the memory_facts_fts contentless FTS5 index in sync with memory_facts.

CREATE TRIGGER IF NOT EXISTS memory_facts_ai
AFTER INSERT ON memory_facts
BEGIN
    INSERT INTO memory_facts_fts(rowid, key, content, category)
    VALUES (new.rowid, new.key, new.content, new.category);
END;

CREATE TRIGGER IF NOT EXISTS memory_facts_ad
AFTER DELETE ON memory_facts
BEGIN
    INSERT INTO memory_facts_fts(memory_facts_fts, rowid, key, content, category)
    VALUES ('delete', old.rowid, old.key, old.content, old.category);
END;

CREATE TRIGGER IF NOT EXISTS memory_facts_au
AFTER UPDATE ON memory_facts
BEGIN
    INSERT INTO memory_facts_fts(memory_facts_fts, rowid, key, content, category)
    VALUES ('delete', old.rowid, old.key, old.content, old.category);
    INSERT INTO memory_facts_fts(rowid, key, content, category)
    VALUES (new.rowid, new.key, new.content, new.category);
END;
"#;

/// Initialize vec0 virtual tables and cleanup triggers.
///
/// Call AFTER `load_sqlite_vec()` AND AFTER `initialize_knowledge_database()`.
/// Triggers reference both vec0 tables and base tables.
///
/// Uses the default embedding dimension of 384.
pub fn initialize_vec_tables(conn: &Connection) -> Result<(), rusqlite::Error> {
    initialize_vec_tables_with_dim(conn, 384)
}

/// The five vec0 virtual tables materialised by
/// [`initialize_vec_tables_with_dim`]. Kept in one place so the post-init
/// presence check and the reindex pipeline stay aligned with the DDL.
pub const REQUIRED_VEC_TABLES: &[&str] = &[
    "memory_facts_index",
    "kg_name_index",
    "session_episodes_index",
    "wiki_articles_index",
    "procedures_index",
];

/// Variant of [`initialize_vec_tables`] that parameterizes the embedding
/// dimension for the vec0 virtual tables.
///
/// Phase 1 of embedding-backend-selection: callers that know the active
/// embedding dimension (e.g. `EmbeddingService`) pass it here so fresh
/// installs honor the user's chosen backend dim. Existing installs still
/// use 384 via [`initialize_vec_tables`].
///
/// Note: `CREATE VIRTUAL TABLE IF NOT EXISTS` is a no-op when the table
/// already exists with a different dim — reindex must drop-and-recreate.
///
/// After running the CREATE batch we verify that all five expected vec0
/// tables materialised. If sqlite-vec failed to load on the connection,
/// `CREATE VIRTUAL TABLE ... USING vec0(...)` silently no-ops; returning an
/// error here lets callers (notably `KnowledgeDatabase::new`'s `.expect`
/// at daemon boot) fail loud with a descriptive message instead of leaving
/// `memory.recall` to blow up on the first query.
pub fn initialize_vec_tables_with_dim(
    conn: &Connection,
    dim: usize,
) -> Result<(), rusqlite::Error> {
    let sql = format!(
        r#"
CREATE VIRTUAL TABLE IF NOT EXISTS kg_name_index USING vec0(
    entity_id TEXT PRIMARY KEY,
    name_embedding FLOAT[{dim}]
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_facts_index USING vec0(
    fact_id TEXT PRIMARY KEY,
    embedding FLOAT[{dim}]
);

CREATE VIRTUAL TABLE IF NOT EXISTS wiki_articles_index USING vec0(
    article_id TEXT PRIMARY KEY,
    embedding FLOAT[{dim}]
);

CREATE VIRTUAL TABLE IF NOT EXISTS procedures_index USING vec0(
    procedure_id TEXT PRIMARY KEY,
    embedding FLOAT[{dim}]
);

CREATE VIRTUAL TABLE IF NOT EXISTS session_episodes_index USING vec0(
    episode_id TEXT PRIMARY KEY,
    embedding FLOAT[{dim}]
);
"#
    );
    conn.execute_batch(&sql)?;
    conn.execute_batch(TRIGGERS_SQL)?;
    verify_vec_tables_present(conn)?;
    Ok(())
}

/// Verify all [`REQUIRED_VEC_TABLES`] exist in `sqlite_master`.
///
/// Returns a descriptive error when the count is short — the usual cause
/// is that the sqlite-vec extension failed to load on this connection, so
/// the `CREATE VIRTUAL TABLE ... USING vec0(...)` statements became
/// silent no-ops.
fn verify_vec_tables_present(conn: &Connection) -> Result<(), rusqlite::Error> {
    let expected = REQUIRED_VEC_TABLES.len() as i64;
    let row_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
         ('memory_facts_index', 'kg_name_index', 'session_episodes_index', \
          'wiki_articles_index', 'procedures_index')",
        [],
        |r| r.get(0),
    )?;
    if row_count != expected {
        let message = format!(
            "vec0 table init incomplete: expected {expected} virtual tables, \
             found {row_count}. sqlite-vec extension likely failed to load — \
             check logs for sqlite_vec errors."
        );
        // Re-use ToSqlConversionFailure as a general error carrier —
        // rusqlite 0.32's `ModuleError` / `UserFunctionError` variants are
        // feature-gated behind `vtab` / `functions` which this crate does
        // not enable. Callers bubble this up via `KnowledgeDatabase::new`'s
        // `.expect()` at daemon boot so the operator sees the full context.
        return Err(rusqlite::Error::ToSqlConversionFailure(
            std::io::Error::other(message).into(),
        ));
    }
    Ok(())
}

/// Query `sqlite_master` for which [`REQUIRED_VEC_TABLES`] exist.
///
/// Returns `(present, missing)` as `Vec<String>` lists suitable for the
/// `/api/embeddings/health` endpoint. Unlike [`verify_vec_tables_present`]
/// this never errors — it returns empty-vectors on a DB error so the
/// health endpoint keeps responding.
#[must_use]
pub fn list_vec_table_presence(conn: &Connection) -> (Vec<String>, Vec<String>) {
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for &name in REQUIRED_VEC_TABLES {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                rusqlite::params![name],
                |_r| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            present.push(name.to_string());
        } else {
            missing.push(name.to_string());
        }
    }
    (present, missing)
}

/// Drop every live vec0 index and recreate it at `dim`.
///
/// Called by the boot-time dim reconciler in `AppState::new` when the
/// configured `EmbeddingService` dim disagrees with the `.embedding-state`
/// marker. Data loss is intentional — the reindex pipeline repopulates
/// from the source tables on the next sleep cycle. Until that repopulates,
/// recall returns empty results rather than blowing up with a dim mismatch.
///
/// # Errors
///
/// Returns an error if DROP or CREATE fails on the connection (e.g.
/// sqlite-vec failed to load).
pub fn drop_and_recreate_vec_tables_at_dim(
    conn: &Connection,
    dim: usize,
) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS memory_facts_index;
         DROP TABLE IF EXISTS kg_name_index;
         DROP TABLE IF EXISTS session_episodes_index;
         DROP TABLE IF EXISTS wiki_articles_index;
         DROP TABLE IF EXISTS procedures_index;",
    )?;
    initialize_vec_tables_with_dim(conn, dim)
}

/// Drop any orphan `*__new` reindex tables left behind by a crash. Idempotent.
///
/// # Errors
///
/// Returns an error if any of the `DROP TABLE` statements fail.
pub fn cleanup_orphan_reindex_tables(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS memory_facts_index__new;
         DROP TABLE IF EXISTS kg_name_index__new;
         DROP TABLE IF EXISTS session_episodes_index__new;
         DROP TABLE IF EXISTS wiki_articles_index__new;
         DROP TABLE IF EXISTS procedures_index__new;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_v22_non_vec_tables_initialize_on_fresh_in_memory_db() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(version, 31);

        // Regular tables.
        for table in [
            "kg_entities",
            "kg_relationships",
            "kg_aliases",
            "kg_episodes",
            "kg_goals",
            "kg_compactions",
            "memory_facts",
            "memory_facts_archive",
            "ward_wiki_articles",
            "procedures",
            "session_episodes",
            "embedding_cache",
            "kg_episode_payloads",
            "kg_beliefs",
            "kg_belief_contradictions",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |r| r.get(0),
                )
                .expect("query");
            assert_eq!(count, 1, "missing table: {table}");
        }

        // FTS5 virtual table for memory_facts.
        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='memory_facts_fts'",
                [],
                |r| r.get(0),
            )
            .expect("query fts");
        assert!(fts_count >= 1, "memory_facts_fts not created");

        // Structural assertion: the base tables carry NO embedding column.
        for table in [
            "memory_facts",
            "ward_wiki_articles",
            "procedures",
            "session_episodes",
        ] {
            let has_embedding: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM pragma_table_info('{}') WHERE name='embedding'",
                        table
                    ),
                    [],
                    |r| r.get(0),
                )
                .expect("pragma");
            assert_eq!(
                has_embedding, 0,
                "table {table} must not have embedding BLOB column"
            );
        }
    }

    use crate::sqlite_vec_loader::load_sqlite_vec;

    #[test]
    fn full_v22_schema_initializes_with_vec_tables_and_triggers() {
        let conn = Connection::open_in_memory().expect("open");
        load_sqlite_vec(&conn).expect("load sqlite-vec");

        initialize_knowledge_database(&conn).expect("init base schema");
        initialize_vec_tables(&conn).expect("init vec tables");

        // All 5 vec0 virtual tables exist.
        for vt in [
            "kg_name_index",
            "memory_facts_index",
            "wiki_articles_index",
            "procedures_index",
            "session_episodes_index",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name=?1",
                    rusqlite::params![vt],
                    |r| r.get(0),
                )
                .expect("query");
            assert!(count >= 1, "missing vec0 table: {vt}");
        }

        // All 5 triggers exist.
        for trg in [
            "trg_entities_delete_vec",
            "trg_facts_delete_vec",
            "trg_wiki_delete_vec",
            "trg_procedures_delete_vec",
            "trg_episodes_delete_vec",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name=?1",
                    rusqlite::params![trg],
                    |r| r.get(0),
                )
                .expect("query");
            assert_eq!(count, 1, "missing trigger: {trg}");
        }
    }

    #[test]
    fn delete_entity_cascades_to_kg_name_index_via_trigger() {
        let conn = Connection::open_in_memory().expect("open");
        load_sqlite_vec(&conn).expect("load");
        initialize_knowledge_database(&conn).expect("init");
        initialize_vec_tables(&conn).expect("init vec");

        // Insert entity + its vec row.
        conn.execute(
            "INSERT INTO kg_entities(id, agent_id, entity_type, name, normalized_name, normalized_hash, first_seen_at, last_seen_at)
             VALUES ('e1', 'root', 'person', 'Alice', 'alice', 'h1', datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert entity");

        let vec_json = serde_json::to_string(&vec![0.1_f32; 384]).unwrap();
        conn.execute(
            "INSERT INTO kg_name_index(entity_id, name_embedding) VALUES ('e1', ?1)",
            rusqlite::params![vec_json],
        )
        .expect("insert vec row");

        // Delete the entity — trigger must cascade to vec index.
        conn.execute("DELETE FROM kg_entities WHERE id = 'e1'", [])
            .expect("delete");

        let vec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_name_index WHERE entity_id = 'e1'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(vec_count, 0, "vec0 row should be cleaned up by trigger");
    }

    // -----------------------------------------------------------------
    // Fix 1: verify_vec_tables_present / initialize_vec_tables_with_dim
    // fails loud when sqlite-vec didn't load on the connection.
    // -----------------------------------------------------------------

    #[test]
    fn initialize_vec_tables_errors_when_sqlite_vec_not_loaded() {
        // Open a plain connection WITHOUT load_sqlite_vec — the CREATE VIRTUAL
        // TABLE ... USING vec0(...) statements become "no such module: vec0"
        // which is already caught by execute_batch; but belt-and-braces the
        // verify_vec_tables_present call also guarantees we never silently
        // return Ok with missing tables.
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init base schema");

        let err = initialize_vec_tables_with_dim(&conn, 384)
            .expect_err("expected failure when sqlite-vec is not loaded");
        let msg = err.to_string();
        assert!(
            msg.contains("vec0") || msg.contains("no such module"),
            "error message should mention vec0: {msg}"
        );
    }

    #[test]
    fn list_vec_table_presence_reports_all_five_when_initialized() {
        let conn = Connection::open_in_memory().expect("open");
        load_sqlite_vec(&conn).expect("load sqlite-vec");
        initialize_knowledge_database(&conn).expect("init base schema");
        initialize_vec_tables_with_dim(&conn, 384).expect("init vec tables");

        let (present, missing) = list_vec_table_presence(&conn);
        assert_eq!(present.len(), 5, "expected 5 present, got: {present:?}");
        assert!(missing.is_empty(), "unexpected missing: {missing:?}");
    }

    #[test]
    fn list_vec_table_presence_reports_missing_when_dropped() {
        let conn = Connection::open_in_memory().expect("open");
        load_sqlite_vec(&conn).expect("load sqlite-vec");
        initialize_knowledge_database(&conn).expect("init base schema");
        initialize_vec_tables_with_dim(&conn, 384).expect("init vec tables");

        // Manually drop one to simulate a partial state.
        conn.execute("DROP TABLE memory_facts_index", [])
            .expect("drop");

        let (present, missing) = list_vec_table_presence(&conn);
        assert_eq!(present.len(), 4);
        assert_eq!(missing, vec!["memory_facts_index".to_string()]);
    }

    #[test]
    fn kg_entities_and_relationships_have_evidence_column() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        for table in ["kg_entities", "kg_relationships"] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            assert!(
                cols.contains(&"evidence".to_string()),
                "{table} must have evidence column; got: {cols:?}"
            );
        }
    }

    // -----------------------------------------------------------------
    // Bi-temporal phase 3: v26 — kg_relationships symmetric columns
    // -----------------------------------------------------------------

    /// Read the symmetric `valid_from` / `valid_until` pair for a
    /// relationship row by id. Returns `(valid_from, valid_until)`.
    fn read_rel_bitemporal(conn: &Connection, id: &str) -> (Option<String>, Option<String>) {
        conn.query_row(
            "SELECT valid_from, valid_until FROM kg_relationships WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .expect("query rel bitemporal")
    }

    /// Seed a minimal pair of entities + a relationship row with the
    /// legacy `valid_at` / `invalidated_at` columns populated and the
    /// symmetric `valid_from` / `valid_until` left NULL — i.e. the
    /// pre-v26 shape.
    fn seed_legacy_rel(
        conn: &Connection,
        rel_id: &str,
        valid_at: Option<&str>,
        invalidated_at: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO kg_entities
                (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                 first_seen_at, last_seen_at)
             VALUES ('src-e', 'agent', 'Concept', 'src', 'src', 'h-src',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("seed src entity");
        conn.execute(
            "INSERT INTO kg_entities
                (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                 first_seen_at, last_seen_at)
             VALUES ('tgt-e', 'agent', 'Concept', 'tgt', 'tgt', 'h-tgt',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("seed tgt entity");
        conn.execute(
            "INSERT INTO kg_relationships
                (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                 first_seen_at, last_seen_at, valid_at, invalidated_at,
                 valid_from, valid_until)
             VALUES (?1, 'agent', 'src-e', 'tgt-e', 'relates_to',
                     datetime('now'), datetime('now'), ?2, ?3,
                     NULL, NULL)",
            rusqlite::params![rel_id, valid_at, invalidated_at],
        )
        .expect("seed rel");
    }

    /// v26 backfill copies the legacy `valid_at` / `invalidated_at`
    /// pair into the symmetric `valid_from` / `valid_until` columns
    /// when the symmetric columns are NULL.
    #[test]
    fn v26_backfill_copies_valid_at_and_invalidated_at_into_symmetric_pair() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        // Re-seed the legacy state after init clobbered any data:
        // initialize_knowledge_database also runs the v26 backfill, but
        // there are no rows yet so it's a no-op. We then seed legacy
        // rows and re-run the backfill SQL directly to simulate the
        // upgrade path on an existing database.
        seed_legacy_rel(
            &conn,
            "rel-legacy",
            Some("2026-01-01T00:00:00Z"),
            Some("2026-03-01T00:00:00Z"),
        );

        // Precondition: symmetric columns NULL.
        let (vf, vu) = read_rel_bitemporal(&conn, "rel-legacy");
        assert!(vf.is_none(), "precondition: valid_from should be NULL");
        assert!(vu.is_none(), "precondition: valid_until should be NULL");

        // Apply the v26 backfill SQL directly.
        conn.execute_batch(V26_KG_RELATIONSHIPS_BITEMPORAL_SQL)
            .expect("run v26 backfill");

        let (vf, vu) = read_rel_bitemporal(&conn, "rel-legacy");
        assert_eq!(
            vf.as_deref(),
            Some("2026-01-01T00:00:00Z"),
            "valid_from must be backfilled from valid_at"
        );
        assert_eq!(
            vu.as_deref(),
            Some("2026-03-01T00:00:00Z"),
            "valid_until must be backfilled from invalidated_at"
        );
    }

    /// The v26 ALTER + UPDATE pipeline is idempotent: re-running the
    /// migration on an already-migrated database is a no-op and does
    /// not corrupt previously backfilled values.
    #[test]
    fn v26_migration_is_idempotent_on_rerun() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        seed_legacy_rel(&conn, "rel-idem", Some("2026-02-02T00:00:00Z"), None);
        conn.execute_batch(V26_KG_RELATIONSHIPS_BITEMPORAL_SQL)
            .expect("first backfill");

        // Re-run the full migration pipeline (ALTER + UPDATE) — both
        // halves must remain no-ops on an already-migrated database.
        ensure_kg_relationships_bitemporal_columns(&conn).expect("rerun ALTER guard");
        conn.execute_batch(V26_KG_RELATIONSHIPS_BITEMPORAL_SQL)
            .expect("rerun backfill");

        let (vf, vu) = read_rel_bitemporal(&conn, "rel-idem");
        assert_eq!(
            vf.as_deref(),
            Some("2026-02-02T00:00:00Z"),
            "valid_from must survive a re-run"
        );
        assert!(
            vu.is_none(),
            "valid_until must stay NULL when invalidated_at was NULL"
        );
    }

    // -----------------------------------------------------------------
    // v27 — kg_beliefs migration idempotency
    // -----------------------------------------------------------------

    /// v28 belief-contradictions table creation is idempotent: re-running
    /// the migration SQL on an already-initialized database is a no-op and
    /// does not corrupt the table.
    #[test]
    fn v28_kg_belief_contradictions_migration_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='kg_belief_contradictions'",
                [],
                |r| r.get(0),
            )
            .expect("query kg_belief_contradictions");
        assert_eq!(
            count, 1,
            "kg_belief_contradictions must be created by initialize"
        );

        // Re-running the migration directly must not error.
        conn.execute_batch(V28_KG_BELIEF_CONTRADICTIONS_SQL)
            .expect("rerun v28 migration");

        // Re-running the full init path must also stay healthy.
        initialize_knowledge_database(&conn).expect("re-init is idempotent");

        let count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='kg_belief_contradictions'",
                [],
                |r| r.get(0),
            )
            .expect("recount kg_belief_contradictions");
        assert_eq!(count_after, 1, "no duplicate table after rerun");
    }

    /// v29 belief-staleness migration is idempotent: the `stale` column
    /// is added once, and re-running the full init path is a no-op that
    /// preserves the column. Mirrors the v26 PRAGMA-guarded ALTER pattern.
    /// Regression test for the daemon-startup crash on databases created
    /// before v29: pre-v29 databases have a `kg_beliefs` table without a
    /// `stale` column. If the inline `SCHEMA_SQL` tries to `CREATE INDEX
    /// ON kg_beliefs(stale)` BEFORE `ensure_kg_beliefs_stale_column` runs
    /// the ADD COLUMN guard, init crashes with `no such column: stale`.
    ///
    /// This test simulates a pre-v29 database (kg_beliefs without stale)
    /// and then runs `initialize_knowledge_database`. It must succeed —
    /// the ensure-column helper adds the column, then v29 SQL creates the
    /// index. The fix is the absence of the stale-index line from inline
    /// SCHEMA_SQL.
    #[test]
    fn initialize_succeeds_on_pre_v29_database_without_stale_column() {
        let conn = Connection::open_in_memory().expect("open");

        // Bootstrap minimum schema_version table + a pre-v29 kg_beliefs
        // shape (no `stale`, no `embedding`).
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             CREATE TABLE kg_beliefs (
                id TEXT PRIMARY KEY,
                partition_id TEXT NOT NULL,
                subject TEXT NOT NULL,
                content TEXT NOT NULL,
                confidence REAL NOT NULL,
                valid_from TEXT,
                valid_until TEXT,
                source_fact_ids TEXT NOT NULL,
                synthesizer_version INTEGER NOT NULL DEFAULT 1,
                reasoning TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                superseded_by TEXT,
                UNIQUE(partition_id, subject, valid_from)
             );",
        )
        .expect("seed pre-v29 schema");

        // Now run init — must NOT panic with `no such column: stale`.
        initialize_knowledge_database(&conn).expect(
            "init must succeed on a pre-v29 database; the inline schema must \
             not reference the `stale` column in any CREATE INDEX statement, \
             because the column is added later by ensure_kg_beliefs_stale_column",
        );

        // After init, stale + embedding must both exist on the existing table.
        let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(cols.contains(&"stale".to_string()), "stale column added");
        assert!(
            cols.contains(&"embedding".to_string()),
            "embedding column added"
        );

        // And the partial stale index must exist.
        let stale_index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_beliefs_stale'",
                [],
                |r| r.get(0),
            )
            .expect("query stale index");
        assert_eq!(
            stale_index_count, 1,
            "idx_beliefs_stale must be created by v29 migration"
        );
    }

    #[test]
    fn v29_kg_beliefs_stale_migration_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            cols.contains(&"stale".to_string()),
            "kg_beliefs must have stale column after init; got: {cols:?}"
        );

        // Re-running the full init path must not error and the column
        // must still be there exactly once.
        initialize_knowledge_database(&conn).expect("re-init is idempotent");
        ensure_kg_beliefs_stale_column(&conn).expect("rerun ALTER guard");
        conn.execute_batch(V29_KG_BELIEFS_STALE_SQL)
            .expect("rerun v29 migration SQL");

        let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)").unwrap();
        let stale_count = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .filter(|name| name == "stale")
            .count();
        assert_eq!(stale_count, 1, "stale column must be present exactly once");

        // The partial index must also be present (idempotent CREATE).
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_beliefs_stale'",
                [],
                |r| r.get(0),
            )
            .expect("query index");
        assert_eq!(index_count, 1, "idx_beliefs_stale index must exist");
    }

    /// v30 belief-embedding migration is idempotent: the `embedding`
    /// BLOB column is added once on first init and re-running the full
    /// init path is a no-op that preserves the column. Same PRAGMA-guarded
    /// ALTER pattern as v26 / v29.
    #[test]
    fn v30_kg_beliefs_embedding_migration_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        // Column must exist after fresh init.
        let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            cols.contains(&"embedding".to_string()),
            "kg_beliefs must have embedding column after init; got: {cols:?}"
        );

        // Re-running the full init path must not error and the column
        // must still be there exactly once.
        initialize_knowledge_database(&conn).expect("re-init is idempotent");
        ensure_kg_beliefs_embedding_column(&conn).expect("rerun ALTER guard");
        conn.execute_batch(V30_KG_BELIEFS_EMBEDDING_SQL)
            .expect("rerun v30 migration SQL");

        let mut stmt = conn.prepare("PRAGMA table_info(kg_beliefs)").unwrap();
        let embedding_count = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .filter(|name| name == "embedding")
            .count();
        assert_eq!(
            embedding_count, 1,
            "embedding column must be present exactly once"
        );
    }

    /// v27 belief table creation is idempotent: re-running the migration
    /// SQL on an already-initialized database is a no-op and does not
    /// corrupt the table.
    #[test]
    fn v27_kg_beliefs_migration_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        // Confirm the table exists after the first run.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='kg_beliefs'",
                [],
                |r| r.get(0),
            )
            .expect("query kg_beliefs");
        assert_eq!(count, 1, "kg_beliefs must be created by initialize");

        // Re-running the migration directly must not error.
        conn.execute_batch(V27_KG_BELIEFS_SQL)
            .expect("rerun v27 migration");

        // Re-running the full init path must also stay healthy.
        initialize_knowledge_database(&conn).expect("re-init is idempotent");

        let count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='kg_beliefs'",
                [],
                |r| r.get(0),
            )
            .expect("recount kg_beliefs");
        assert_eq!(count_after, 1, "no duplicate table after rerun");
    }

    /// Writers exercising the kg_relationships INSERT path populate
    /// `valid_from` on creation. We exercise the schema directly with
    /// the same column shape used by `store_relationship` in
    /// `kg::storage`, which is the canonical production writer.
    #[test]
    fn kg_relationships_insert_populates_valid_from() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        // Insert two entities so the FK constraint is satisfied.
        conn.execute(
            "INSERT INTO kg_entities
                (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                 first_seen_at, last_seen_at)
             VALUES ('e-src', 'agent', 'Concept', 'a', 'a', 'h-a',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("seed src entity");
        conn.execute(
            "INSERT INTO kg_entities
                (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                 first_seen_at, last_seen_at)
             VALUES ('e-tgt', 'agent', 'Concept', 'b', 'b', 'h-b',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("seed tgt entity");

        let first_seen = "2026-05-15T12:00:00+00:00";
        conn.execute(
            "INSERT INTO kg_relationships
                (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                 first_seen_at, last_seen_at, mention_count, valid_from)
             VALUES ('rel-new', 'agent', 'e-src', 'e-tgt', 'relates_to',
                     ?1, ?1, 1, ?1)",
            rusqlite::params![first_seen],
        )
        .expect("insert rel");

        let (vf, vu) = read_rel_bitemporal(&conn, "rel-new");
        assert_eq!(
            vf.as_deref(),
            Some(first_seen),
            "writers must populate valid_from on creation"
        );
        assert!(vu.is_none(), "valid_until must stay NULL on creation");
    }

    /// v31 hierarchy-columns migration is idempotent on a fresh DB:
    /// `kg_entities` gets `layer` + `parent_cluster_id`, `kg_relationships`
    /// gets `layer` + `is_inter_cluster`, and all four indexes are present.
    /// Re-running init does not duplicate any column.
    #[test]
    fn v31_kg_hierarchy_columns_migration_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        let entity_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(kg_entities)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            entity_cols.contains(&"layer".to_string()),
            "kg_entities must have layer column after init; got: {entity_cols:?}"
        );
        assert!(
            entity_cols.contains(&"parent_cluster_id".to_string()),
            "kg_entities must have parent_cluster_id column after init; got: {entity_cols:?}"
        );

        let rel_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(kg_relationships)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            rel_cols.contains(&"layer".to_string()),
            "kg_relationships must have layer column after init; got: {rel_cols:?}"
        );
        assert!(
            rel_cols.contains(&"is_inter_cluster".to_string()),
            "kg_relationships must have is_inter_cluster column after init; got: {rel_cols:?}"
        );

        // All four indexes must exist.
        for index_name in [
            "idx_kg_entities_layer",
            "idx_kg_entities_parent_cluster",
            "idx_kg_relationships_layer",
            "idx_kg_relationships_inter_cluster",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='index' AND name=?1",
                    rusqlite::params![index_name],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("query index {index_name}: {e}"));
            assert_eq!(count, 1, "{index_name} must exist after init");
        }

        // Re-running the full init path must not duplicate the columns.
        initialize_knowledge_database(&conn).expect("re-init is idempotent");
        ensure_kg_entities_hierarchy_columns(&conn).expect("rerun entity ALTER guard");
        ensure_kg_relationships_hierarchy_columns(&conn).expect("rerun rel ALTER guard");
        conn.execute_batch(V31_KG_HIERARCHY_COLUMNS_SQL)
            .expect("rerun v31 migration SQL");

        let layer_count = conn
            .prepare("PRAGMA table_info(kg_entities)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|name| name == "layer")
            .count();
        assert_eq!(
            layer_count, 1,
            "kg_entities.layer must be present exactly once after re-init"
        );
    }

    /// v31 defaults: existing rows inserted without specifying the new
    /// columns get `layer = 0`, `parent_cluster_id = NULL`,
    /// `is_inter_cluster = 0`. This is the "no behaviour change" guarantee
    /// — every existing entity is a valid layer-0 base node, every existing
    /// relationship is a valid layer-0 base edge.
    #[test]
    fn v31_defaults_match_no_op_semantics_for_existing_rows() {
        let conn = Connection::open_in_memory().expect("open");
        initialize_knowledge_database(&conn).expect("init");

        // Insert a minimal entity without setting the new columns.
        conn.execute(
            "INSERT INTO kg_entities
                (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                 first_seen_at, last_seen_at)
             VALUES ('e1', 'agent', 'Concept', 'thing', 'thing', 'h1',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert entity");

        let (layer, parent): (i64, Option<String>) = conn
            .query_row(
                "SELECT layer, parent_cluster_id FROM kg_entities WHERE id = 'e1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("read entity");
        assert_eq!(layer, 0, "layer must default to 0");
        assert!(parent.is_none(), "parent_cluster_id must default to NULL");

        // Insert a partner entity + a relationship without the new columns.
        conn.execute(
            "INSERT INTO kg_entities
                (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                 first_seen_at, last_seen_at)
             VALUES ('e2', 'agent', 'Concept', 'other', 'other', 'h2',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert partner entity");
        conn.execute(
            "INSERT INTO kg_relationships
                (id, agent_id, source_entity_id, target_entity_id, relationship_type,
                 first_seen_at, last_seen_at)
             VALUES ('r1', 'agent', 'e1', 'e2', 'relates_to',
                     datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert relationship");

        let (rel_layer, is_inter): (i64, i64) = conn
            .query_row(
                "SELECT layer, is_inter_cluster FROM kg_relationships WHERE id = 'r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("read relationship");
        assert_eq!(rel_layer, 0, "layer must default to 0");
        assert_eq!(is_inter, 0, "is_inter_cluster must default to 0");
    }

    /// Regression test for daemon-startup safety on a pre-v31 database.
    /// Simulates a kg_entities / kg_relationships shape without the v31
    /// hierarchy columns and verifies that `initialize_knowledge_database`
    /// adds them via the `ensure_*_hierarchy_columns` ALTER guards.
    ///
    /// Note: the v31 migration SQL only carries the supporting indexes
    /// — none of them reference the new columns in a way that fails
    /// before the ALTER (the partial indexes use the columns but the
    /// ALTER runs first), so this is mainly a structural assertion.
    #[test]
    fn initialize_succeeds_on_pre_v31_database_without_hierarchy_columns() {
        let conn = Connection::open_in_memory().expect("open");

        // Seed a pre-v31 shape of kg_entities + kg_relationships. The
        // seed must include every column referenced by existing
        // indexes (last_accessed_at, epistemic_class, etc.), because
        // the inline SCHEMA_SQL's `CREATE INDEX IF NOT EXISTS` runs
        // before our ALTER guards and would fail on a column it can't
        // find. The v31 columns (layer, parent_cluster_id,
        // is_inter_cluster) are intentionally omitted — that is what
        // we're testing.
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             CREATE TABLE kg_entities (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                normalized_hash TEXT NOT NULL,
                properties TEXT,
                epistemic_class TEXT NOT NULL DEFAULT 'current',
                confidence REAL NOT NULL DEFAULT 0.8,
                mention_count INTEGER NOT NULL DEFAULT 1,
                access_count INTEGER NOT NULL DEFAULT 0,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                last_accessed_at TEXT,
                valid_from TEXT,
                valid_until TEXT,
                invalidated_by TEXT,
                compressed_into TEXT,
                source_episode_ids TEXT,
                evidence TEXT
             );
             CREATE TABLE kg_relationships (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                source_entity_id TEXT NOT NULL,
                target_entity_id TEXT NOT NULL,
                relationship_type TEXT NOT NULL,
                properties TEXT,
                epistemic_class TEXT NOT NULL DEFAULT 'current',
                confidence REAL NOT NULL DEFAULT 0.8,
                mention_count INTEGER NOT NULL DEFAULT 1,
                access_count INTEGER NOT NULL DEFAULT 0,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                last_accessed_at TEXT,
                valid_at TEXT,
                invalidated_at TEXT,
                valid_from TEXT,
                valid_until TEXT,
                invalidated_by TEXT,
                source_episode_ids TEXT,
                evidence TEXT,
                UNIQUE(source_entity_id, target_entity_id, relationship_type)
             );",
        )
        .expect("seed pre-v31 schema");

        // Init must succeed and add the new columns.
        initialize_knowledge_database(&conn).expect("init must succeed on a pre-v31 database");

        let entity_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(kg_entities)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(entity_cols.contains(&"layer".to_string()));
        assert!(entity_cols.contains(&"parent_cluster_id".to_string()));

        let rel_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(kg_relationships)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(rel_cols.contains(&"layer".to_string()));
        assert!(rel_cols.contains(&"is_inter_cluster".to_string()));
    }
}
