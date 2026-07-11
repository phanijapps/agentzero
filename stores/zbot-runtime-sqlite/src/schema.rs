// ============================================================================
// DATABASE SCHEMA
// SQLite schema for sessions, agent executions, and messages
// ============================================================================

use rusqlite::{Connection, Result};

/// Current schema version
const SCHEMA_VERSION: i32 = 22;

/// Run migrations for existing databases.
///
/// Checks the current schema version and applies any needed migrations.
fn migrate_database(conn: &Connection) -> Result<()> {
    // Check if schema_version table exists
    let has_version: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
        [],
        |row| row.get::<_, i64>(0),
    )? > 0;

    if !has_version {
        return Ok(()); // Fresh database, no migration needed
    }

    let version: i32 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    // v8 → v9: Add routing fields to sessions
    if version < 9 {
        // Use try/ignore pattern since columns may already exist on fresh DB
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN thread_id TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN connector_id TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN respond_to TEXT", []);
    }

    // v9 → v10: Add bridge_outbox table
    if version < 10 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS bridge_outbox (
                id TEXT PRIMARY KEY,
                adapter_id TEXT NOT NULL,
                capability TEXT NOT NULL,
                payload TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                session_id TEXT,
                thread_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                sent_at TEXT,
                error TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                retry_after TEXT
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_outbox_adapter_status ON bridge_outbox(adapter_id, status)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_outbox_created ON bridge_outbox(created_at)",
            [],
        );
    }

    // v10 → v11: Add distillation_runs, session_episodes tables; add ward_id to memory_facts
    if version < 11 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS distillation_runs (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                facts_extracted INTEGER DEFAULT 0,
                entities_extracted INTEGER DEFAULT 0,
                relationships_extracted INTEGER DEFAULT 0,
                episode_created INTEGER DEFAULT 0,
                error TEXT,
                retry_count INTEGER DEFAULT 0,
                duration_ms INTEGER,
                created_at TEXT NOT NULL
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_distillation_runs_status ON distillation_runs(status)",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS session_episodes (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                ward_id TEXT NOT NULL DEFAULT '__global__',
                task_summary TEXT NOT NULL,
                outcome TEXT NOT NULL,
                strategy_used TEXT,
                key_learnings TEXT,
                token_cost INTEGER,
                embedding BLOB,
                created_at TEXT NOT NULL
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_episodes_agent ON session_episodes(agent_id)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_episodes_ward ON session_episodes(ward_id)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_episodes_outcome ON session_episodes(outcome)",
            [],
        );

        // Add ward_id column to memory_facts
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN ward_id TEXT NOT NULL DEFAULT '__global__'",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_facts_ward ON memory_facts(ward_id)",
            [],
        );
        // Drop the old unique constraint (it was an inline UNIQUE, which SQLite
        // implements as an auto-named index "sqlite_autoindex_memory_facts_1")
        let _ = conn.execute("DROP INDEX IF EXISTS sqlite_autoindex_memory_facts_1", []);
        // Create the new unique constraint including ward_id
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_memory_facts_agent_scope_ward_key ON memory_facts(agent_id, scope, ward_id, key)",
            [],
        );
    }

    // v11 → v12: Add contradicted_by column to memory_facts
    if version < 12 {
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN contradicted_by TEXT",
            [],
        );
    }

    // v12 → v13: Add recall_log, memory_facts_archive tables; add archived to sessions
    if version < 13 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS recall_log (
                session_id TEXT NOT NULL,
                fact_key TEXT NOT NULL,
                recalled_at TEXT NOT NULL,
                PRIMARY KEY (session_id, fact_key)
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recall_log_session ON recall_log(session_id)",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_facts_archive (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'agent',
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                ward_id TEXT NOT NULL DEFAULT '__global__',
                mention_count INTEGER NOT NULL DEFAULT 1,
                source_summary TEXT,
                embedding BLOB,
                contradicted_by TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                archived_at TEXT NOT NULL
            )",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
            [],
        );
    }

    // v13 → v14: Add pinned column to memory_facts
    if version < 14 {
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
            [],
        );
    }

    // v14 → v15: Add child_session_id to agent_executions for smart resume
    if version < 15 {
        let _ = conn.execute(
            "ALTER TABLE agent_executions ADD COLUMN child_session_id TEXT",
            [],
        );
    }

    // v15 → v16: Add artifacts table for agent-generated file tracking
    if version < 16 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                ward_id TEXT,
                execution_id TEXT,
                agent_id TEXT,
                file_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_type TEXT,
                file_size INTEGER,
                label TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id)",
            [],
        );
    }

    // v16 → v17: Add mode column to sessions for persistent execution mode
    if version < 17 {
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN mode TEXT", []);
    }

    // v17 → v18: Add temporal columns to memory_facts; add kg_causal_edges table
    if version < 18 {
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN valid_from TEXT", []);
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN valid_until TEXT", []);
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN superseded_by TEXT", []);
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_temporal ON memory_facts(valid_from, valid_until)",
            [],
        );

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_causal_edges (
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
            CREATE INDEX IF NOT EXISTS idx_causal_effect ON kg_causal_edges(effect_entity_id);",
        )?;
    }

    // v18 → v19: Add ward_wiki_articles table for compiled wiki knowledge per ward
    if version < 19 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ward_wiki_articles (
                id TEXT PRIMARY KEY,
                ward_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                tags TEXT,
                source_fact_ids TEXT,
                embedding BLOB,
                version INTEGER DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(ward_id, title)
            );
            CREATE INDEX IF NOT EXISTS idx_wiki_ward ON ward_wiki_articles(ward_id);",
        )?;
    }

    // v19 → v20: Add procedures table for learned procedure patterns
    if version < 20 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS procedures (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                ward_id TEXT DEFAULT '__global__',
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                trigger_pattern TEXT,
                steps TEXT NOT NULL,
                parameters TEXT,
                success_count INTEGER DEFAULT 1,
                failure_count INTEGER DEFAULT 0,
                avg_duration_ms INTEGER,
                avg_token_cost INTEGER,
                last_used TEXT,
                embedding BLOB,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_procedures_agent ON procedures(agent_id);
            CREATE INDEX IF NOT EXISTS idx_procedures_ward ON procedures(ward_id);",
        )?;
    }

    // v20 → v21: Add kg_episodes table and provenance columns on memory_facts
    if version < 21 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_episodes (
                id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                session_id TEXT,
                agent_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(content_hash, source_type)
            );
            CREATE INDEX IF NOT EXISTS idx_episodes_session ON kg_episodes(session_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_source ON kg_episodes(source_type, source_ref);",
        )?;

        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN epistemic_class TEXT DEFAULT 'current'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN source_episode_id TEXT",
            [],
        );
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN source_ref TEXT", []);
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_class ON memory_facts(agent_id, epistemic_class)",
            [],
        );
    }

    // v21 → v22: knowledge tables moved to knowledge.db (see knowledge_schema.rs)
    if version < 22 {
        // No-op: memory_facts, memory_facts_fts, memory_facts_archive, session_episodes,
        // kg_episodes, ward_wiki_articles, procedures, and embedding_cache have been
        // relocated to knowledge.db. Any pre-existing rows in conversations.db are
        // orphaned and will be ignored; the repository layer routes to KnowledgeDatabase.
    }

    Ok(())
}

/// Initialize the database with all tables
pub fn initialize_database(conn: &Connection) -> Result<()> {
    // Run migrations for existing databases before creating tables
    migrate_database(conn)?;

    // Enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON", [])?;

    // =========================================================================
    // SESSIONS
    // Top-level container for a user's work session
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL DEFAULT 'running',
            source TEXT NOT NULL DEFAULT 'web',
            root_agent_id TEXT NOT NULL,
            title TEXT,
            created_at TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            total_tokens_in INTEGER DEFAULT 0,
            total_tokens_out INTEGER DEFAULT 0,
            metadata TEXT,
            pending_delegations INTEGER DEFAULT 0,
            continuation_needed INTEGER DEFAULT 0,
            ward_id TEXT,
            parent_session_id TEXT,
            thread_id TEXT,
            connector_id TEXT,
            respond_to TEXT,
            archived INTEGER NOT NULL DEFAULT 0,
            mode TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_created ON sessions(created_at)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_root_agent ON sessions(root_agent_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id)",
        [],
    )?;

    // =========================================================================
    // AGENT EXECUTIONS
    // An agent's participation in a session (root or delegated subagent)
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS agent_executions (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            parent_execution_id TEXT,
            delegation_type TEXT NOT NULL DEFAULT 'root',
            task TEXT,
            status TEXT NOT NULL DEFAULT 'queued',
            started_at TEXT,
            completed_at TEXT,
            tokens_in INTEGER DEFAULT 0,
            tokens_out INTEGER DEFAULT 0,
            checkpoint TEXT,
            error TEXT,
            log_path TEXT,
            child_session_id TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (parent_execution_id) REFERENCES agent_executions(id) ON DELETE SET NULL,
            FOREIGN KEY (child_session_id) REFERENCES sessions(id) ON DELETE SET NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_session ON agent_executions(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_parent ON agent_executions(parent_execution_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_status ON agent_executions(status)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_agent ON agent_executions(agent_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_started ON agent_executions(started_at)",
        [],
    )?;

    // =========================================================================
    // MESSAGES
    // Individual messages in an agent's conversation
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            execution_id TEXT,
            session_id TEXT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            token_count INTEGER DEFAULT 0,
            tool_calls TEXT,
            tool_results TEXT,
            tool_call_id TEXT,
            FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_execution ON messages(execution_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_session_created ON messages(session_id, created_at)",
        [],
    )?;

    // =========================================================================
    // EXECUTION LOGS
    // Detailed logs for debugging and tracing agent execution
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS execution_logs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            conversation_id TEXT,
            agent_id TEXT NOT NULL,
            parent_session_id TEXT,
            timestamp TEXT NOT NULL,
            level TEXT NOT NULL,
            category TEXT NOT NULL,
            message TEXT NOT NULL,
            metadata TEXT,
            duration_ms INTEGER
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_session ON execution_logs(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON execution_logs(timestamp)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_agent ON execution_logs(agent_id)",
        [],
    )?;

    // =========================================================================
    // BRIDGE OUTBOX
    // Reliable delivery queue for outbound messages to bridge workers
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bridge_outbox (
            id TEXT PRIMARY KEY,
            adapter_id TEXT NOT NULL,
            capability TEXT NOT NULL,
            payload TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            session_id TEXT,
            thread_id TEXT,
            agent_id TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            sent_at TEXT,
            error TEXT,
            retry_count INTEGER NOT NULL DEFAULT 0,
            retry_after TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_outbox_adapter_status ON bridge_outbox(adapter_id, status)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_outbox_created ON bridge_outbox(created_at)",
        [],
    )?;

    // =========================================================================
    // DISTILLATION RUNS
    // Tracks distillation health per session
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS distillation_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            facts_extracted INTEGER DEFAULT 0,
            entities_extracted INTEGER DEFAULT 0,
            relationships_extracted INTEGER DEFAULT 0,
            episode_created INTEGER DEFAULT 0,
            error TEXT,
            retry_count INTEGER DEFAULT 0,
            duration_ms INTEGER,
            created_at TEXT NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_distillation_runs_status ON distillation_runs(status)",
        [],
    )?;

    // =========================================================================
    // RECALL LOG
    // Tracks which facts were recalled per session for predictive recall
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS recall_log (
            session_id TEXT NOT NULL,
            fact_key TEXT NOT NULL,
            recalled_at TEXT NOT NULL,
            PRIMARY KEY (session_id, fact_key)
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_recall_log_session ON recall_log(session_id)",
        [],
    )?;

    // =========================================================================
    // ARTIFACTS
    // File artifacts produced by agent executions
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS artifacts (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            ward_id TEXT,
            execution_id TEXT,
            agent_id TEXT,
            file_path TEXT NOT NULL,
            file_name TEXT NOT NULL,
            file_type TEXT,
            file_size INTEGER,
            label TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id)",
        [],
    )?;

    // kg_causal_edges moved to knowledge.db (see knowledge_schema.rs) in v22

    // =========================================================================
    // SCHEMA VERSION
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        )",
        [],
    )?;

    conn.execute(
        "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
        [SCHEMA_VERSION],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Helper: create a fresh in-memory database and initialize it.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        initialize_database(&conn).expect("initialize_database");
        conn
    }

    #[test]
    fn test_migration_creates_distillation_runs_table() {
        let conn = setup_db();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='distillation_runs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "distillation_runs table should exist");
    }

    #[test]
    fn test_schema_version_is_current() {
        let conn = setup_db();
        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 22, "schema version should be 22");
    }

    #[test]
    fn test_recall_log_table_exists() {
        let conn = setup_db();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='recall_log'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "recall_log table should exist");

        // Verify we can insert and query
        conn.execute(
            "INSERT INTO recall_log (session_id, fact_key, recalled_at) VALUES ('s1', 'k1', datetime('now'))",
            [],
        )
        .expect("insert into recall_log");

        let fact_key: String = conn
            .query_row(
                "SELECT fact_key FROM recall_log WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fact_key, "k1");
    }

    #[test]
    fn test_sessions_has_archived_column() {
        let conn = setup_db();
        // Insert a session and verify archived defaults to 0
        conn.execute(
            "INSERT INTO sessions (id, root_agent_id, created_at) VALUES ('s1', 'agent1', datetime('now'))",
            [],
        )
        .expect("insert session");

        let archived: i32 = conn
            .query_row("SELECT archived FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(archived, 0, "default archived should be 0");

        // Update to archived
        conn.execute("UPDATE sessions SET archived = 1 WHERE id = 's1'", [])
            .expect("update archived");

        let archived: i32 = conn
            .query_row("SELECT archived FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(archived, 1, "archived should be 1 after update");
    }
}
